//! The asynchronous half of a streamed request body: reading the client's
//! upload until the firewall can judge it ([`Intake`]), then forwarding it
//! through the [`RequestPipeline`] ([`StreamStart::spawn`]).
//!
//! # Before the stream opens
//! The raw body is kept (in RAM, then encrypted on disk) so that a body that
//! cannot be streamed still reaches the buffered path exactly as before.
//! Nothing else is done with a body of at most the lag: an identity body is
//! only counted, an encoded one only decoded to learn its size. Once the
//! decoded size exceeds the lag, the raw bytes are decoded again into a
//! pipeline that lexes and holds them, and from then on every new byte goes
//! to both. The stream opens when the pipeline knows the request's `model`
//! and layout and the firewall agrees; until then the client keeps
//! uploading and its bytes are held.
//!
//! # After it opened
//! The upstream connection is only made once there is something to send
//! (the transport awaits [`StreamHandle::ready`]), so a request refused
//! before its first released byte (a model-gate or Secret Scan denial, a
//! strict-mode body, a body held for a `Content-Length`) never reaches the
//! upstream at all. Afterwards an abort fails the upstream body, so hyper
//! closes the connection without completing the request, and the client's
//! remaining upload is drained (bounded in size and time) so that the client
//! can read the answer instead of seeing a reset.
//!
//! Three outputs:
//! - **Chunked**: the pipeline's output, sent as it is released.
//! - **Sized**: the pipeline's output held whole and sent with a
//!   `Content-Length` (hosts that require one, see [`crate::framing`]).
//! - **Tee** (Off/Notify): the client's original bytes with its framing and
//!   encoding, forwarded as they arrive while a decoded copy is inspected.
//!   Every byte is lexed before it is forwarded and the last byte is held
//!   back until the body was validated, so a body that turns out invalid
//!   (or to carry a duplicate key) is never delivered complete.

use std::ops::ControlFlow;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use http_body::{Body, Frame};
use http_body_util::BodyExt;
use lr_secret_scanner::request_stream::RequestMetadata;

use crate::request_body::StreamDecoder;
use crate::request_stream::{
    Answer, Question, RequestPipeline, StreamAbort, StreamSession, StreamSummary, MAX_HOLD, STEP,
};
use crate::spill::{SpillBuffer, SpillConfig, SpillError, CHUNK};

/// Most bytes read and discarded from a client after its upload was ended,
/// so it can read the answer.
pub(crate) const DRAIN_LIMIT: u64 = MAX_HOLD as u64;
/// Longest time spent draining such a client.
pub(crate) const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
/// Decoded bytes shown read-only in an approval popup for a body too large
/// to edit there.
pub const PREVIEW_LIMIT: usize = 1024 * 1024;

pub(crate) type BoxError = Box<dyn std::error::Error + Send + Sync>;
/// A client request body of any type.
pub(crate) type ClientBody = http_body_util::combinators::BoxBody<Bytes, BoxError>;

/// Reads a client's body frame by frame and remembers its end.
pub(crate) struct ClientReader {
    body: ClientBody,
    eof: bool,
}

impl ClientReader {
    pub(crate) fn new<B>(body: B) -> Self
    where
        B: Body<Data = Bytes> + Send + Sync + 'static,
        B::Error: Into<BoxError>,
    {
        let eof = body.is_end_stream();
        Self {
            body: body.map_err(Into::into).boxed(),
            eof,
        }
    }

    pub(crate) fn eof(&self) -> bool {
        self.eof
    }

    /// The next data frame, or `None` at the end of the body.
    pub(crate) async fn frame(&mut self) -> Result<Option<Bytes>, String> {
        while !self.eof {
            match self.body.frame().await {
                None => self.eof = true,
                Some(Err(error)) => return Err(error.to_string()),
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        return Ok(Some(data));
                    }
                }
            }
        }
        Ok(None)
    }

    /// At least `limit` bytes, or what is left of the body.
    pub(crate) async fn batch(&mut self, limit: usize) -> Result<Vec<u8>, String> {
        let mut data = Vec::new();
        while data.len() < limit {
            match self.frame().await? {
                Some(chunk) => data.extend_from_slice(&chunk),
                None => break,
            }
        }
        Ok(data)
    }

    /// Read and discard the rest, up to `limit` bytes or `timeout`. Returns
    /// the bytes read and whether the body ended.
    pub(crate) async fn drain(&mut self, limit: u64, timeout: Duration) -> (u64, bool) {
        let mut total = 0u64;
        let drained = tokio::time::timeout(timeout, async {
            while total <= limit {
                match self.frame().await {
                    Ok(Some(chunk)) => total += chunk.len() as u64,
                    Ok(None) | Err(_) => break,
                }
            }
        })
        .await;
        let _ = drained;
        (total, self.eof)
    }
}

/// Why reading a body before it opened failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IntakeError {
    /// Above every limit that applies to it.
    TooLarge,
    /// The client's body or the spill storage failed.
    Failed(String),
}

impl IntakeError {
    fn spill(error: SpillError) -> Self {
        match error {
            SpillError::RequestCap(_) => Self::TooLarge,
            other => Self::Failed(other.message()),
        }
    }
}

enum Phase {
    /// At most the lag so far: only counted (identity) or decoded to be
    /// counted (encoded).
    Counting(Option<StreamDecoder>),
    /// Beyond the lag: decoded into a pipeline that lexes and holds.
    Lexing(StreamDecoder, Box<RequestPipeline>),
    /// The body cannot be streamed; only the raw bytes are kept.
    Buffered,
}

/// A streamable body read up to the point where it either ended or the
/// firewall can decide about it.
pub(crate) struct Intake {
    raw: SpillBuffer,
    headers: http::HeaderMap,
    lag: usize,
    /// Largest raw body the buffered path accepts.
    buffered_cap: u64,
    spill: Option<Arc<SpillConfig>>,
    phase: Phase,
}

impl Intake {
    /// `streamable` is false when the format or encoding rules streaming
    /// out from the start; the raw bytes are then all that is kept.
    pub(crate) fn new(
        headers: &http::HeaderMap,
        lag: usize,
        spill: Option<Arc<SpillConfig>>,
        buffered_cap: usize,
        streamable: bool,
    ) -> Self {
        let mut encoding = http::HeaderMap::new();
        for value in headers.get_all(http::header::CONTENT_ENCODING) {
            encoding.append(http::header::CONTENT_ENCODING, value.clone());
        }
        let phase = match StreamDecoder::new(&encoding) {
            Ok(decoder) if streamable => {
                Phase::Counting((!decoder.is_identity()).then_some(decoder))
            }
            _ => Phase::Buffered,
        };
        let raw = match &spill {
            Some(config) => SpillBuffer::new(config.clone()),
            None => SpillBuffer::ram_only(buffered_cap as u64),
        };
        Self {
            raw,
            headers: encoding,
            lag,
            buffered_cap: buffered_cap as u64,
            spill,
            phase,
        }
    }

    /// Raw bytes received so far.
    pub(crate) fn received(&self) -> u64 {
        self.raw.end()
    }

    /// Whether the body may still be streamed.
    pub(crate) fn streamable(&self) -> bool {
        !matches!(self.phase, Phase::Buffered)
    }

    /// Whether the firewall can now be asked to open the stream.
    pub(crate) fn ready_to_open(&self) -> bool {
        matches!(&self.phase, Phase::Lexing(_, pipeline) if pipeline.ready_to_open())
    }

    pub(crate) fn metadata(&self) -> Option<&RequestMetadata> {
        match &self.phase {
            Phase::Lexing(_, pipeline) => Some(pipeline.metadata()),
            _ => None,
        }
    }

    /// The first decoded bytes, for a read-only preview.
    pub(crate) fn preview(&mut self, limit: usize) -> Option<(Vec<u8>, u64)> {
        match &mut self.phase {
            Phase::Lexing(_, pipeline) => {
                let total = pipeline.received();
                pipeline.head(limit).map(|head| (head, total))
            }
            _ => None,
        }
    }

    /// The first raw bytes, for the monitor.
    pub(crate) fn raw_head(&mut self, limit: usize) -> Vec<u8> {
        let to = self.raw.end().min(limit as u64);
        self.raw.read(0, to).unwrap_or_default()
    }

    /// Take more raw bytes.
    pub(crate) fn push(&mut self, data: &[u8]) -> Result<(), IntakeError> {
        self.raw.append(data).map_err(IntakeError::spill)?;
        let lag = self.lag as u64;
        match &mut self.phase {
            Phase::Counting(counter) => {
                let decoded = match counter {
                    None => Some(self.raw.end()),
                    Some(decoder) => decoder
                        .push(data, |_| ControlFlow::Continue(()))
                        .ok()
                        .map(|()| decoder.decoded_bytes()),
                };
                match decoded {
                    None => self.phase = Phase::Buffered,
                    Some(decoded) if decoded > lag => self.start_lexing()?,
                    Some(_) => {}
                }
            }
            Phase::Lexing(decoder, pipeline) => {
                let mut sink = Vec::new();
                let decoded = decoder.push(data, |piece| {
                    let _ = pipeline.push(piece, None, &mut sink);
                    ControlFlow::Continue(())
                });
                if decoded.is_err() || pipeline.unstreamable().is_some() {
                    self.phase = Phase::Buffered;
                }
            }
            Phase::Buffered => {}
        }
        self.check_buffered_cap()
    }

    fn check_buffered_cap(&self) -> Result<(), IntakeError> {
        if matches!(self.phase, Phase::Buffered) && self.raw.end() > self.buffered_cap {
            return Err(IntakeError::TooLarge);
        }
        Ok(())
    }

    /// The body outgrew the lag: decode what arrived so far into a pipeline.
    fn start_lexing(&mut self) -> Result<(), IntakeError> {
        let Ok(mut decoder) = StreamDecoder::new(&self.headers) else {
            self.phase = Phase::Buffered;
            return Ok(());
        };
        let hold = match &self.spill {
            Some(config) => SpillBuffer::new(config.clone()),
            None => SpillBuffer::ram_only(MAX_HOLD as u64),
        };
        let mut pipeline = RequestPipeline::with_hold(self.lag, hold);
        let end = self.raw.end();
        let mut offset = 0;
        let mut sink = Vec::new();
        while offset < end {
            let to = (offset + 4 * CHUNK as u64).min(end);
            let mut piece = self.raw.read(offset, to).map_err(IntakeError::spill)?;
            let decoded = decoder.push(&piece, |decoded| {
                let _ = pipeline.push(decoded, None, &mut sink);
                ControlFlow::Continue(())
            });
            zeroize::Zeroize::zeroize(&mut piece);
            if decoded.is_err() || pipeline.unstreamable().is_some() {
                self.phase = Phase::Buffered;
                return Ok(());
            }
            offset = to;
        }
        self.phase = Phase::Lexing(decoder, Box::new(pipeline));
        Ok(())
    }

    /// The firewall wants the whole body: stop streaming.
    pub(crate) fn give_up(&mut self) -> Result<(), IntakeError> {
        self.phase = Phase::Buffered;
        self.check_buffered_cap()
    }

    /// The whole raw body, for the buffered path.
    pub(crate) fn into_buffered(mut self) -> Result<Bytes, IntakeError> {
        if self.raw.end() > self.buffered_cap {
            return Err(IntakeError::TooLarge);
        }
        let end = self.raw.end();
        self.raw
            .read(0, end)
            .map(Bytes::from)
            .map_err(IntakeError::spill)
    }

    /// Hand the decoder, the pipeline (holding everything decoded so far)
    /// and the raw bytes to the driver.
    pub(crate) fn into_stream(self) -> Option<(StreamDecoder, RequestPipeline, SpillBuffer)> {
        match self.phase {
            Phase::Lexing(decoder, pipeline) => Some((decoder, *pipeline, self.raw)),
            _ => None,
        }
    }
}

/// How a streamed body is framed and what it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Output {
    /// The pipeline's output, chunked as it is released.
    Chunked,
    /// The pipeline's output, held until the body ended and sent with a
    /// `Content-Length`.
    Sized,
    /// The client's original bytes. `length` is the client's
    /// `Content-Length`, kept upstream; `hold` holds them until the end to
    /// send them with a length the client did not declare.
    Tee { length: Option<u64>, hold: bool },
}

/// The upstream half of a streamed request body.
pub(crate) enum Chunk {
    Data(Bytes),
    End,
}

/// Sends a streamed body to the upstream. Dropping it without
/// [`end`](Self::end) fails the body, so the upstream connection is reset
/// rather than sent a complete request.
pub(crate) struct UpstreamSender(tokio::sync::mpsc::Sender<Chunk>);

impl UpstreamSender {
    /// Queue forwarded bytes; `false` once the upstream stopped reading.
    pub(crate) async fn send(&self, data: Bytes) -> bool {
        data.is_empty() || self.0.send(Chunk::Data(data)).await.is_ok()
    }

    /// End the body normally.
    pub(crate) async fn end(self) {
        let _ = self.0.send(Chunk::End).await;
    }
}

/// The request body hyper sends upstream for a streamed request. Without a
/// known length HTTP/1.1 frames it as chunked; with one (set before the
/// request is sent) hyper sends a `Content-Length` and checks it. It ends
/// only on an explicit end, and fails (which makes hyper close the
/// connection without completing the request) when the sender is dropped.
pub(crate) struct UpstreamBody {
    rx: tokio::sync::mpsc::Receiver<Chunk>,
    done: bool,
    length: Arc<OnceLock<u64>>,
}

pub(crate) fn upstream_channel() -> (UpstreamSender, UpstreamBody) {
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    (
        UpstreamSender(tx),
        UpstreamBody {
            rx,
            done: false,
            length: Arc::new(OnceLock::new()),
        },
    )
}

impl Body for UpstreamBody {
    type Data = Bytes;
    type Error = BoxError;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        if self.done {
            return Poll::Ready(None);
        }
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(Chunk::Data(data))) => Poll::Ready(Some(Ok(Frame::data(data)))),
            Poll::Ready(Some(Chunk::End)) => {
                self.done = true;
                Poll::Ready(None)
            }
            Poll::Ready(None) => {
                self.done = true;
                Poll::Ready(Some(Err("request body aborted".into())))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.done
    }

    fn size_hint(&self) -> http_body::SizeHint {
        match self.length.get() {
            Some(length) => http_body::SizeHint::with_exact(*length),
            None => http_body::SizeHint::default(),
        }
    }
}

#[derive(Default)]
struct ProgressState {
    capture: crate::interceptor::BoundedCapture,
    abort: Option<StreamAbort>,
    summary: Option<crate::interceptor::StreamedRequest>,
    /// The upstream was sent the body without chunked framing.
    sized: bool,
}

/// What a streamed upload has forwarded so far, shared with the transport
/// for the monitor and for the status of an aborted upload.
#[derive(Clone, Default)]
pub(crate) struct StreamProgress(Arc<parking_lot::Mutex<ProgressState>>);

impl StreamProgress {
    /// The head of the forwarded bytes, for the monitor.
    pub(crate) fn snapshot(&self) -> crate::interceptor::BoundedCapture {
        self.0.lock().capture.clone()
    }

    /// Why the upload was aborted, if it was.
    pub(crate) fn abort(&self) -> Option<StreamAbort> {
        self.0.lock().abort.clone()
    }

    /// The monitor's view of the body, once it was forwarded completely.
    pub(crate) fn summary(&self) -> Option<crate::interceptor::StreamedRequest> {
        self.0.lock().summary.clone()
    }

    /// Whether the body went upstream with a known length.
    pub(crate) fn sized(&self) -> bool {
        self.0.lock().sized
    }

    fn record(&self, forwarded: &[u8], cap: usize) {
        let mut state = self.0.lock();
        let capture = &mut state.capture;
        let room = cap.saturating_sub(capture.head.len());
        capture
            .head
            .extend_from_slice(&forwarded[..room.min(forwarded.len())]);
        capture.total_len += forwarded.len() as u64;
        capture.truncated = capture.total_len > capture.head.len() as u64;
    }
}

/// The monitor's view of a streamed body from its sniffed metadata.
pub(crate) fn streamed_request(
    format: crate::wire::WireFormat,
    metadata: &RequestMetadata,
    correlation: Option<lr_monitor::CorrelationParts>,
) -> crate::interceptor::StreamedRequest {
    crate::interceptor::StreamedRequest {
        meta: crate::wire::request_meta_from_sniffed(format, metadata).unwrap_or_default(),
        tool_count: usize::try_from(metadata.tools_len()).unwrap_or(usize::MAX),
        conversation_id: crate::wire::conversation_id_from_sniffed(metadata),
        correlation,
    }
}

/// A request body that is about to stream: the client body still being
/// read, the decoder and pipeline holding what arrived so far, and the
/// firewall's session.
pub(crate) struct StreamStart {
    pub(crate) reader: ClientReader,
    pub(crate) decoder: StreamDecoder,
    pub(crate) pipeline: RequestPipeline,
    /// Every raw byte received so far (forwarded first by a tee).
    pub(crate) raw: SpillBuffer,
    pub(crate) session: Box<dyn StreamSession>,
    pub(crate) format: crate::wire::WireFormat,
    /// Bytes of forwarded content kept for the monitor.
    pub(crate) capture_cap: usize,
    /// Restores references in the response.
    pub(crate) vault: Option<Arc<lr_secret_scanner::placeholders::Vault>>,
    pub(crate) output: Output,
    /// Release nothing before the end of the body.
    pub(crate) strict: bool,
    /// The forwarded body is content-encoded (a tee of an encoded body).
    pub(crate) encoded: bool,
    pub(crate) spill: Option<Arc<SpillConfig>>,
}

/// What the transport gets for a streamed body.
pub(crate) struct StreamHandle {
    pub(crate) body: UpstreamBody,
    pub(crate) progress: StreamProgress,
    /// Resolves once there is something to send (connect upstream then), or
    /// with the reason the upload ended before anything was sent.
    pub(crate) ready: tokio::sync::oneshot::Receiver<Result<(), StreamAbort>>,
}

struct Work {
    decoder: Option<StreamDecoder>,
    pipeline: RequestPipeline,
    session: Box<dyn StreamSession>,
    /// Tee: raw bytes not yet forwarded, and the offset forwarded so far.
    raw: Option<SpillBuffer>,
    sent: u64,
    /// Output held until the end (sized outputs).
    hold: Option<SpillBuffer>,
}

impl Work {
    fn decode(
        decoder: &mut StreamDecoder,
        pipeline: &mut RequestPipeline,
        session: Option<&mut dyn StreamSession>,
        data: &[u8],
        out: &mut Vec<u8>,
    ) -> Result<(), StreamAbort> {
        let mut failed = None;
        let mut check = |pushed: Result<(), StreamAbort>| match pushed {
            Ok(()) => ControlFlow::Continue(()),
            Err(abort) => {
                failed = Some(abort);
                ControlFlow::Break(())
            }
        };
        let decoded = match session {
            Some(session) => decoder.push(data, |piece| {
                check(pipeline.push(piece, Some(&mut *session), out))
            }),
            None => decoder.push(data, |piece| check(pipeline.push(piece, None, out))),
        };
        if let Some(abort) = failed {
            return Err(abort);
        }
        decoded.map_err(|error| StreamAbort::new(error.status(), error.message()))
    }

    /// Hold client bytes without inspecting or releasing (while a question
    /// is open).
    fn hold_only(&mut self, data: &[u8]) -> Result<(), StreamAbort> {
        let decoder = self.decoder.as_mut().expect("decoder until the end");
        let mut out = Vec::new();
        Self::decode(decoder, &mut self.pipeline, None, data, &mut out)?;
        if let Some(raw) = &mut self.raw {
            raw.append(data).map_err(StreamAbort::spill)?;
        }
        Ok(())
    }

    /// Process one batch of client bytes (or none, to release what an
    /// answer reopened); at the end of the body, release the rest. Returns
    /// the bytes to forward now and, once everything was forwarded, the
    /// summary. At the end of the body with a question open, nothing is
    /// finished: the driver asks, then steps again.
    fn step(
        &mut self,
        data: &[u8],
        eof: bool,
    ) -> Result<(Vec<u8>, Option<StreamSummary>), StreamAbort> {
        let mut out = Vec::new();
        let decoder = self.decoder.as_mut().expect("decoder until the end");
        Self::decode(
            decoder,
            &mut self.pipeline,
            Some(self.session.as_mut()),
            data,
            &mut out,
        )?;
        if data.is_empty() {
            self.pipeline
                .push(&[], Some(self.session.as_mut()), &mut out)?;
        }
        let mut summary = None;
        if eof && !self.session.pending() {
            if let Some(decoder) = self.decoder.take() {
                decoder
                    .finish()
                    .map_err(|error| StreamAbort::new(error.status(), error.message()))?;
            }
            summary = Some(self.pipeline.finish(self.session.as_mut(), &mut out)?);
        }
        if let Some(raw) = &mut self.raw {
            // A tee forwards the original bytes, now that the pipeline has
            // lexed them; the last byte waits for the validated end.
            raw.append(data).map_err(StreamAbort::spill)?;
            let until = if summary.is_some() {
                raw.end()
            } else {
                raw.end().saturating_sub(1).max(self.sent)
            };
            out = raw.read(self.sent, until).map_err(StreamAbort::spill)?;
            self.sent = until;
            raw.discard_before(until);
        }
        if let Some(hold) = &mut self.hold {
            hold.append(&out).map_err(StreamAbort::spill)?;
            zeroize::Zeroize::zeroize(&mut out);
            out.clear();
        }
        Ok((out, summary))
    }

    /// The next piece of a held output.
    fn held_piece(&mut self, from: u64) -> Result<Vec<u8>, StreamAbort> {
        let hold = self.hold.as_mut().expect("held output");
        let to = (from + STEP as u64).min(hold.end());
        let piece = hold.read(from, to).map_err(StreamAbort::spill)?;
        hold.discard_before(to);
        Ok(piece)
    }
}

impl StreamStart {
    /// Start forwarding: returns the upstream body, the progress handle and
    /// the signal to connect upstream.
    pub(crate) fn spawn(self) -> StreamHandle {
        let (tx, body) = upstream_channel();
        let progress = StreamProgress::default();
        progress.0.lock().capture.encoded = self.encoded;
        let (ready_tx, ready) = tokio::sync::oneshot::channel();
        let length = body.length.clone();
        tokio::spawn(drive(self, tx, progress.clone(), ready_tx, length));
        StreamHandle {
            body,
            progress,
            ready,
        }
    }
}

/// Run one blocking step off the executor.
async fn blocking<T: Send + 'static>(
    work: &mut Option<Work>,
    f: impl FnOnce(&mut Work) -> Result<T, StreamAbort> + Send + 'static,
) -> Result<T, StreamAbort> {
    let mut taken = work.take().expect("work between steps");
    let (returned, result) = tokio::task::spawn_blocking(move || {
        let result = f(&mut taken);
        (taken, result)
    })
    .await
    .map_err(|_| StreamAbort::new(500, "Request inspection failed"))?;
    *work = Some(returned);
    result
}

/// Await an answer while holding whatever the client keeps uploading.
async fn wait_answer(
    question: Question,
    reader: &mut ClientReader,
    work: &mut Work,
) -> Result<Answer, StreamAbort> {
    let mut answer = question.0;
    loop {
        if reader.eof() {
            return Ok(answer.await);
        }
        tokio::select! {
            answer = &mut answer => return Ok(answer),
            frame = reader.frame() => match frame {
                Ok(Some(data)) => work.hold_only(&data)?,
                Ok(None) => {}
                Err(error) => {
                    return Err(StreamAbort::new(400, format!("client request body: {error}")))
                }
            },
        }
    }
}

async fn drive(
    start: StreamStart,
    tx: UpstreamSender,
    progress: StreamProgress,
    ready: tokio::sync::oneshot::Sender<Result<(), StreamAbort>>,
    length: Arc<OnceLock<u64>>,
) {
    let StreamStart {
        mut reader,
        decoder,
        mut pipeline,
        raw,
        session,
        format,
        capture_cap,
        vault: _,
        output,
        strict,
        encoded: _,
        spill,
    } = start;
    let mut ready = Some(ready);
    let tee = matches!(output, Output::Tee { .. });
    if tee {
        pipeline.inspect_only();
    } else if strict {
        pipeline.hold_until_end();
    }
    pipeline.open();
    let held_output = matches!(output, Output::Sized | Output::Tee { hold: true, .. });
    let hold = held_output.then(|| match &spill {
        Some(config) => SpillBuffer::new(config.clone()),
        None => SpillBuffer::ram_only(MAX_HOLD as u64),
    });
    if let Output::Tee {
        length: client_length,
        hold: false,
    } = output
    {
        // The original bytes are available at once.
        if let Some(client_length) = client_length {
            let _ = length.set(client_length);
            progress.0.lock().sized = true;
        }
        if let Some(ready) = ready.take() {
            let _ = ready.send(Ok(()));
        }
    }
    let mut work = Some(Work {
        decoder: Some(decoder),
        pipeline,
        session,
        raw: tee.then_some(raw),
        sent: 0,
        hold,
    });
    let result: Result<StreamSummary, StreamAbort> = async {
        // The first step releases what arrived before the stream opened.
        let mut release = true;
        loop {
            // Answer every open question first; the upload is held meanwhile.
            loop {
                let current = work.as_mut().expect("work between steps");
                let Some(question) = current.session.question() else {
                    break;
                };
                let answer = wait_answer(question, &mut reader, current).await?;
                let registered = current.session.answer(answer)?;
                current
                    .pipeline
                    .registered(&registered, current.session.as_mut());
                release = true;
            }
            let data = if release {
                Vec::new()
            } else {
                reader.batch(STEP).await.map_err(|error| {
                    StreamAbort::new(400, format!("client request body: {error}"))
                })?
            };
            release = false;
            let eof = reader.eof();
            let (out, summary) = blocking(&mut work, move |w| w.step(&data, eof)).await?;
            if !out.is_empty() {
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Ok(()));
                }
                progress.record(&out, capture_cap);
                if !tx.send(Bytes::from(out)).await {
                    return Err(StreamAbort::new(
                        502,
                        "The upstream stopped reading the request",
                    ));
                }
            }
            if let Some(summary) = summary {
                if held_output {
                    let total = work
                        .as_ref()
                        .and_then(|w| w.hold.as_ref())
                        .map_or(0, SpillBuffer::end);
                    let _ = length.set(total);
                    progress.0.lock().sized = true;
                    if let Some(ready) = ready.take() {
                        let _ = ready.send(Ok(()));
                    }
                    let mut from = 0;
                    while from < total {
                        let piece = blocking(&mut work, move |w| w.held_piece(from)).await?;
                        from += piece.len() as u64;
                        progress.record(&piece, capture_cap);
                        if !tx.send(Bytes::from(piece)).await {
                            return Err(StreamAbort::new(
                                502,
                                "The upstream stopped reading the request",
                            ));
                        }
                    }
                }
                return Ok(summary);
            }
            if eof {
                // A question opened at the very end: ask, then finish.
                release = true;
            }
        }
    }
    .await;
    match result {
        Ok(summary) => {
            if let Some(ready) = ready.take() {
                let _ = ready.send(Ok(()));
            }
            {
                let mut state = progress.0.lock();
                state.capture.complete = true;
                state.summary = Some(streamed_request(
                    format,
                    &summary.metadata,
                    Some(summary.correlation),
                ));
            }
            tx.end().await;
        }
        Err(abort) => {
            tracing::warn!("proxy: streamed request aborted: {}", abort.message);
            progress.0.lock().abort = Some(abort.clone());
            // Dropping the sender fails the upstream body: hyper closes the
            // connection without completing the request.
            drop(tx);
            drop(work);
            if let Some(ready) = ready.take() {
                let _ = ready.send(Err(abort));
            }
            // Let the client finish its upload so it can read the answer.
            reader.drain(DRAIN_LIMIT, DRAIN_TIMEOUT).await;
        }
    }
}
