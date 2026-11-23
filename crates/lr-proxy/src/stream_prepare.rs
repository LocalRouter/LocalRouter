//! Reading a [`BodyPolicy::Stream`](crate::interceptor::BodyPolicy::Stream)
//! request body up to the point where it either ended or the interceptor
//! decided about it, shared by the MITM transport and the reverse proxy.
//!
//! Until the body outgrows the lag (or when it cannot be streamed) it is held
//! whole, raw; nothing is forwarded before the interceptor opens the stream.
//! While the interceptor decides (which may wait for an approval popup) the
//! client keeps uploading and its bytes are held, spilling to disk beyond
//! RAM. The two proxies differ only in what they pass in: how the
//! interceptor is asked ([`prepare_stream`]'s `decide`), the spill
//! configuration, the buffered limit and which upstream key the
//! length-required registry is asked about.

use std::future::Future;
use std::sync::Arc;

use bytes::Bytes;
use http_body::Body;
use lr_secret_scanner::request_stream::RequestMetadata;

use crate::interceptor::{BoundedCapture, RequestAction, StreamOpen, StreamPolicy};
use crate::request_stream::{StreamStart, STEP};
use crate::stream_driver::{
    BoxError, ClientReader, Intake, IntakeError, Output, DRAIN_TIMEOUT, PREVIEW_LIMIT,
};

/// The outcome of reading a streamable body.
pub(crate) enum Prepared {
    /// The whole body, for the buffered path.
    Buffered(Bytes),
    Failed {
        too_large: bool,
        error: String,
    },
    Rejected {
        action: RequestAction,
        capture: BoundedCapture,
    },
    Stream(Box<StreamStart>),
}

impl From<IntakeError> for Prepared {
    fn from(error: IntakeError) -> Self {
        match error {
            IntakeError::TooLarge => Prepared::Failed {
                too_large: true,
                error: String::new(),
            },
            IntakeError::Failed(error) => Prepared::Failed {
                too_large: false,
                error,
            },
        }
    }
}

/// What differs between the proxies that read a streamable body.
pub(crate) struct StreamTarget<'a> {
    /// The request path (selects the wire format).
    pub path: &'a str,
    /// Where held bytes spill, encrypted, beyond RAM.
    pub spill: Option<Arc<crate::spill::SpillConfig>>,
    /// Largest raw body the buffered path accepts.
    pub buffered_cap: usize,
    /// Bytes of forwarded content kept for the monitor.
    pub capture_cap: usize,
    /// The client's declared `Content-Length`.
    pub client_length: Option<u64>,
    /// Whether the upstream needs a `Content-Length` for a transformed body,
    /// and the key it is known by there.
    pub length_required: &'a crate::framing::LengthRequiredHosts,
    pub upstream_key: &'a str,
}

/// Push a batch into the intake off the executor (it may decode, lex and
/// spill).
pub(crate) async fn intake_step(
    intake: &mut Option<Intake>,
    data: Vec<u8>,
) -> Result<(), IntakeError> {
    let mut taken = intake.take().expect("intake between steps");
    let (returned, result) = tokio::task::spawn_blocking(move || {
        let result = taken.push(&data);
        (taken, result)
    })
    .await
    .map_err(|_| IntakeError::Failed("request intake failed".into()))?;
    *intake = Some(returned);
    result
}

/// Read a streamable body. `decide` asks the interceptor about it once it
/// outgrew the lag, with a read-only preview of its first decoded bytes and
/// its sniffed metadata.
pub(crate) async fn prepare_stream<B, D, F>(
    body: B,
    headers: &http::HeaderMap,
    policy: &StreamPolicy,
    target: StreamTarget<'_>,
    decide: D,
) -> Prepared
where
    B: Body<Data = Bytes> + Send + Sync + 'static,
    B::Error: Into<BoxError>,
    D: FnOnce(Option<BoundedCapture>, RequestMetadata) -> F,
    F: Future<Output = StreamOpen>,
{
    let format = crate::wire::detect(target.path);
    // Undecodable encodings and unknown formats are judged on the buffered
    // path, exactly as before.
    let streamable = format.is_some() && crate::request_body::StreamDecoder::new(headers).is_ok();
    let mut intake = Some(Intake::new(
        headers,
        policy.lag,
        target.spill.clone(),
        target.buffered_cap,
        streamable,
    ));
    let mut reader = ClientReader::new(body);
    loop {
        let data = match reader.batch(STEP).await {
            Ok(data) => data,
            Err(error) => {
                return Prepared::Failed {
                    too_large: false,
                    error,
                }
            }
        };
        if let Err(error) = intake_step(&mut intake, data).await {
            return error.into();
        }
        let current = intake.as_ref().expect("intake between steps");
        if reader.eof() || current.ready_to_open() {
            break;
        }
    }
    let mut intake = intake.expect("intake after reading");
    if !intake.ready_to_open() {
        return intake
            .into_buffered()
            .map_or_else(Prepared::from, Prepared::Buffered);
    }
    let metadata = intake.metadata().cloned().unwrap_or_default();
    let preview = intake
        .preview(PREVIEW_LIMIT)
        .map(|(head, received)| BoundedCapture {
            truncated: received > head.len() as u64,
            head,
            total_len: received,
            complete: false,
            encoded: false,
        });
    // Keep reading (and holding) while the interceptor decides.
    let decision = {
        let decide = decide(preview, metadata);
        tokio::pin!(decide);
        loop {
            if reader.eof() {
                break (&mut decide).await;
            }
            tokio::select! {
                decision = &mut decide => break decision,
                frame = reader.frame() => match frame {
                    Ok(Some(data)) => {
                        if let Err(error) = intake.push(&data) {
                            return error.into();
                        }
                    }
                    Ok(None) => {}
                    Err(error) => return Prepared::Failed { too_large: false, error },
                },
            }
        }
    };
    // Bytes that arrived meanwhile may have shown the body cannot be
    // streamed after all (a repeated key, invalid JSON): nothing was sent,
    // so it takes the buffered path.
    let decision = match decision {
        StreamOpen::Stream { .. } if !intake.streamable() => StreamOpen::Buffer,
        decision => decision,
    };
    match decision {
        StreamOpen::Buffer => {
            if let Err(error) = intake.give_up() {
                return error.into();
            }
            let mut intake = Some(intake);
            while !reader.eof() {
                let data = match reader.batch(STEP).await {
                    Ok(data) => data,
                    Err(error) => {
                        return Prepared::Failed {
                            too_large: false,
                            error,
                        }
                    }
                };
                if let Err(error) = intake_step(&mut intake, data).await {
                    return error.into();
                }
            }
            intake
                .expect("intake after reading")
                .into_buffered()
                .map_or_else(Prepared::from, Prepared::Buffered)
        }
        StreamOpen::Reject(action) => {
            // Read and discard the rest, so the client can read the refusal.
            let head = intake.raw_head(target.capture_cap);
            let received = intake.received();
            drop(intake);
            let (rest, complete) = reader
                .drain(
                    (target.buffered_cap as u64).saturating_sub(received),
                    DRAIN_TIMEOUT,
                )
                .await;
            let total_len = received + rest;
            let capture = BoundedCapture {
                truncated: total_len > head.len() as u64,
                head,
                total_len,
                complete,
                encoded: crate::tap::content_encoded(headers),
            };
            Prepared::Rejected { action, capture }
        }
        StreamOpen::Stream {
            session,
            vault,
            tee,
        } => {
            let Some((decoder, pipeline, raw)) = intake.into_stream() else {
                return Prepared::Failed {
                    too_large: false,
                    error: "request stream state lost".into(),
                };
            };
            let length_required = target.length_required.requires(target.upstream_key);
            let output = if tee {
                Output::Tee {
                    length: target.client_length,
                    hold: target.client_length.is_none() && length_required,
                }
            } else if length_required {
                Output::Sized
            } else {
                Output::Chunked
            };
            Prepared::Stream(Box::new(StreamStart {
                reader,
                decoder,
                pipeline,
                raw,
                session,
                format: format.expect("a streamable body has a format"),
                capture_cap: target.capture_cap,
                vault,
                output,
                strict: policy.strict,
                encoded: tee && crate::tap::content_encoded(headers),
                // A transformed body goes out as the client encoded it when
                // that was zstd alone; anything else is forwarded decoded.
                zstd: !tee && crate::request_body::is_zstd_only(headers),
                spill: target.spill,
            }))
        }
    }
}

/// Set the forwarded headers for a streamed body: a tee keeps the client's
/// encoding and length (a held tee is sent with a length the client may not
/// have declared, so an explicit `Transfer-Encoding: chunked` would override
/// it); a transformed body loses its representation headers and is framed
/// by hyper (chunked, or with the length of a held output), and gets
/// `Content-Encoding: zstd` back when it is re-encoded.
pub(crate) fn stream_headers(headers: &mut http::HeaderMap, start: &StreamStart) {
    if matches!(start.output, Output::Tee { .. }) {
        headers.remove(http::header::TRANSFER_ENCODING);
        return;
    }
    crate::request_body::rewritten(headers);
    if start.zstd {
        headers.insert(
            http::header::CONTENT_ENCODING,
            http::HeaderValue::from_static("zstd"),
        );
    }
}
