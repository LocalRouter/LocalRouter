//! Reverse proxy: impersonate a local LLM provider's original port.
//!
//! Where the MITM proxy in [`crate::transport`] intercepts *outbound* traffic a
//! client was already sending elsewhere, the reverse proxy takes over the
//! *inbound* address a local provider used to own. The provider is relocated to
//! a different port; LocalRouter binds the original one and forwards every
//! request through verbatim, teeing a bounded copy for the monitor.
//!
//! Consequences of that shape, and why it looks different from `transport.rs`:
//! - **No authentication.** The listener *is* the client's identity — apps
//!   pointed at `localhost:11434` send no credentials and must keep working
//!   untouched. One listener therefore belongs to exactly one client.
//! - **Plain HTTP, no TLS.** Local providers speak `http://` on loopback.
//! - **Every path is forwarded**, not just LLM endpoints: native `/api/*`,
//!   OpenAI-shaped `/v1/*`, health probes, model pulls. Anything the wrapped
//!   provider serves keeps working, monitored or not.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{HeaderName, HeaderValue};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, TcpStream};

use crate::error::ProxyError;
use crate::interceptor::{BodyPolicy, RequestAction, RequestHead, StreamOpen, StreamPolicy};
use crate::tap::{RequestCaptureHandle, RequestTap};

/// Cap on the bytes captured per request/response for monitoring (1 MiB).
const MAX_CAPTURE: usize = 1024 * 1024;
/// Cap on a buffered request body (16 MiB) — matches the gateway's body limit.
/// Only bodies that are inspected are buffered; the rest are forwarded as
/// they arrive, whatever their size.
const MAX_REQUEST_BODY: usize = 16 * 1024 * 1024;

/// Headers that describe a single hop and must never be forwarded.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// One completed request/response through a reverse-proxy listener.
#[derive(Debug, Clone, Default)]
pub struct ReverseExchange {
    /// Client that owns this listener.
    pub client_id: String,
    pub session_id: Option<String>,
    pub secret_session: Option<String>,
    /// Strategy attached to that client (for metrics attribution).
    pub strategy_id: String,
    /// Provider instance name being wrapped, when known.
    pub provider_instance: Option<String>,
    /// Upstream base the request was forwarded to (`http://127.0.0.1:11435`).
    pub upstream: String,
    pub method: String,
    /// Path with query, e.g. `/api/chat`.
    pub path: String,
    pub status: Option<u16>,
    pub request_body: Option<Vec<u8>>,
    /// Bounded capture of a request body forwarded without buffering.
    pub request_capture: Option<crate::interceptor::BoundedCapture>,
    /// Set when the body was streamed: `request_capture` then holds the head
    /// of the forwarded (masked) bytes, and this the sniffed metadata.
    pub request_stream: Option<crate::interceptor::StreamedRequest>,
    pub request_body_error: Option<crate::request_body::DecodeError>,
    /// Response head (capped); see `response_tail` for a long response.
    pub response_body: Option<Vec<u8>>,
    /// The end of a response too long to capture whole.
    pub response_tail: Option<crate::interceptor::ResponseTail>,
    /// Response was `text/event-stream` (OpenAI-style streaming).
    pub response_is_sse: bool,
    /// Response was newline-delimited JSON (Ollama-style streaming).
    pub response_is_ndjson: bool,
    pub started_at: Option<DateTime<Utc>>,
    pub latency_ms: Option<u64>,
    /// Set when the upstream could not be reached at all.
    pub error: Option<String>,
    /// Monitor event opened by [`ReverseRecorder::begin`], completed on end.
    pub event_id: Option<String>,
    /// Cross-hop trace stamped on the forwarded request; `hop > 1` marks a
    /// request an earlier LocalRouter hop already handled (not counted).
    pub trace: Option<lr_types::RequestTrace>,
    /// The firewall's rate-limit reservation, as
    /// [`crate::interceptor::ObservedExchange::admission`].
    pub admission: crate::interceptor::ExchangeAdmission,
    /// As [`crate::interceptor::ObservedExchange::refused_locally`].
    pub refused_locally: bool,
}

/// Sink for observed exchanges. Implemented by the app layer (monitor +
/// metrics); kept as a trait so this crate stays free of app dependencies.
#[async_trait::async_trait]
pub trait ReverseRecorder: Send + Sync {
    /// Decide from the request head how the body is handled, with the same
    /// contract as [`crate::interceptor::ProxyInterceptor::body_policy`].
    /// The default buffers every body.
    fn body_policy(&self, _head: &RequestHead<'_>) -> BodyPolicy {
        BodyPolicy::Buffered
    }
    async fn on_request(&self, _exchange: &ReverseExchange) -> crate::interceptor::RequestAction {
        crate::interceptor::RequestAction::Forward
    }
    /// Decide about a body [`body_policy`](Self::body_policy) let stream,
    /// once it outgrew the lag, with the same contract as
    /// [`crate::interceptor::ProxyInterceptor::open_stream`]. The default
    /// reads it whole.
    async fn open_stream(
        &self,
        _exchange: &ReverseExchange,
        _metadata: &lr_secret_scanner::request_stream::RequestMetadata,
        _policy: &StreamPolicy,
    ) -> StreamOpen {
        StreamOpen::Buffer
    }
    /// Open a pending monitor event for an in-flight request. Returning `None`
    /// (the default) simply means no pending event is shown.
    fn begin(&self, _exchange: &ReverseExchange) -> Option<String> {
        None
    }

    /// Record a finished (or failed) exchange.
    async fn record(&self, exchange: ReverseExchange);
}

/// A no-op recorder, useful for tests and for running without monitoring.
pub struct NoopRecorder;

#[async_trait::async_trait]
impl ReverseRecorder for NoopRecorder {
    fn body_policy(&self, _head: &RequestHead<'_>) -> BodyPolicy {
        BodyPolicy::PassThrough
    }
    async fn record(&self, _exchange: ReverseExchange) {}
}

/// Identity carried by a listener and stamped onto every exchange.
#[derive(Debug, Clone, Default)]
pub struct ReverseClient {
    pub client_id: String,
    pub strategy_id: String,
    pub provider_instance: Option<String>,
}

/// A reverse-proxy listener's configuration and collaborators.
pub struct ReverseProxy {
    /// `scheme://host:port` of the relocated provider, no trailing slash.
    upstream: String,
    upstream_host: String,
    upstream_port: u16,
    client: ReverseClient,
    recorder: Arc<dyn ReverseRecorder>,
    /// Live "duplicate request detection" flag shared with the app.
    dedupe_enabled: Arc<std::sync::atomic::AtomicBool>,
    /// Where a streamed body's held bytes spill, encrypted, beyond RAM.
    /// Without it they are held in RAM up to the hold cap.
    spill: Option<Arc<crate::spill::SpillConfig>>,
    /// Whether the provider needs a `Content-Length` for a transformed body
    /// (learned when it refuses a chunked one).
    length_required: Arc<crate::framing::LengthRequiredHosts>,
}

impl ReverseProxy {
    /// Build a reverse proxy forwarding to `upstream_url` (plain `http://`).
    pub fn new(
        upstream_url: &str,
        client: ReverseClient,
        recorder: Arc<dyn ReverseRecorder>,
    ) -> Result<Self, ProxyError> {
        let (host, port) = parse_http_upstream(upstream_url)?;
        Ok(Self {
            upstream: format!("http://{host}:{port}"),
            upstream_host: host,
            upstream_port: port,
            client,
            recorder,
            dedupe_enabled: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            spill: None,
            length_required: Default::default(),
        })
    }

    /// Share the app's live "duplicate request detection" flag (defaults to
    /// enabled).
    pub fn with_dedupe_flag(mut self, flag: Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.dedupe_enabled = flag;
        self
    }

    /// Spill held bytes of streamed request bodies to encrypted files.
    pub fn with_spill(mut self, spill: Option<Arc<crate::spill::SpillConfig>>) -> Self {
        self.spill = spill;
        self
    }

    /// Share which upstreams need a `Content-Length` for a transformed body.
    pub fn with_length_required(mut self, hosts: Arc<crate::framing::LengthRequiredHosts>) -> Self {
        self.length_required = hosts;
        self
    }

    /// The normalized upstream base URL.
    pub fn upstream(&self) -> &str {
        &self.upstream
    }

    /// Bind a listener on `host:port` (port 0 = OS-assigned).
    pub async fn bind(host: &str, port: u16) -> Result<TcpListener, ProxyError> {
        Ok(TcpListener::bind((host, port)).await?)
    }

    /// Accept until `shutdown` resolves; one task per connection.
    pub async fn serve(
        self: Arc<Self>,
        listener: TcpListener,
        shutdown: impl std::future::Future<Output = ()> + Send,
    ) {
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = &mut shutdown => {
                    tracing::info!(
                        client_id = %self.client.client_id,
                        "reverse proxy accept loop shutting down"
                    );
                    break;
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _peer)) => {
                            let this = self.clone();
                            tokio::spawn(async move { this.serve_conn(stream).await });
                        }
                        Err(e) => tracing::warn!("reverse proxy accept error: {e}"),
                    }
                }
            }
        }
    }

    async fn serve_conn(self: Arc<Self>, stream: TcpStream) {
        // Nagle off: streamed tokens should reach the app as they arrive.
        let _ = stream.set_nodelay(true);
        let service = service_fn(move |req: Request<Incoming>| {
            let this = self.clone();
            async move { Ok::<_, Infallible>(this.forward(req).await) }
        });
        if let Err(e) = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await
        {
            // Clients disconnecting mid-stream is normal; log at debug.
            tracing::debug!("reverse proxy connection ended: {e}");
        }
    }

    /// Forward one request to the relocated provider, teeing the response.
    async fn forward(&self, req: Request<Incoming>) -> Response<BoxedBody> {
        let started = Instant::now();
        let started_at = Utc::now();
        let method = req.method().to_string();
        let path = req
            .uri()
            .path_and_query()
            .map(|p| p.as_str().to_string())
            .unwrap_or_else(|| "/".to_string());

        let (mut parts, body) = req.into_parts();
        // The client's declared length (Content-Length), before any is read.
        let client_length = http_body::Body::size_hint(&body).exact();

        // Rewrite for the new hop: strip per-hop headers, point Host at the
        // relocated provider, and ask for an uncompressed response so the
        // captured copy is parseable. hyper derives the framing from the
        // forwarded body, so a pass-through body keeps its Content-Length
        // through its exact size hint.
        for name in HOP_BY_HOP {
            parts.headers.remove(*name);
        }
        parts.headers.insert(
            hyper::header::ACCEPT_ENCODING,
            hyper::header::HeaderValue::from_static("identity"),
        );
        parts.headers.remove(hyper::header::CONTENT_LENGTH);
        if let Ok(host) = HeaderValue::from_str(&self.authority()) {
            parts.headers.insert(hyper::header::HOST, host);
        }
        // The upstream connection is per-request, so send an origin-form URI.
        parts.uri = match path.parse() {
            Ok(uri) => uri,
            Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid request path"),
        };
        // Cross-hop trace: recognize a request an earlier LocalRouter hop
        // already handled, and mark the forwarded copy for the next hop.
        let trace = crate::stamp_trace(
            &mut parts.headers,
            self.dedupe_enabled
                .load(std::sync::atomic::Ordering::Relaxed),
        );

        // Decide from the head alone whether the body must be buffered.
        let policy = self.recorder.body_policy(&RequestHead {
            client_id: &self.client.client_id,
            strategy_id: &self.client.strategy_id,
            host: self
                .client
                .provider_instance
                .as_deref()
                .unwrap_or(&self.upstream),
            port: 0,
            method: &method,
            path: &path,
            headers: &parts.headers,
            trace: trace.as_ref(),
            provider_override: self.client.provider_instance.as_deref(),
            source: crate::interceptor::ExchangeSource::ReverseProxy,
        });
        // Content is only ever captured for LLM calls.
        let capture_cap = if crate::wire::detect(&path).is_some() {
            crate::passive::RAW_CAP
        } else {
            0
        };
        let encoded = crate::tap::content_encoded(&parts.headers);

        let mut base = ReverseExchange {
            client_id: self.client.client_id.clone(),
            session_id: Some(uuid::Uuid::new_v4().to_string()),
            secret_session: [
                "x-session-id",
                "x-session-affinity",
                "session_id",
                "session-id",
            ]
            .iter()
            .find_map(|name| parts.headers.get(*name))
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
            strategy_id: self.client.strategy_id.clone(),
            provider_instance: self.client.provider_instance.clone(),
            upstream: self.upstream.clone(),
            method,
            path: path.clone(),
            started_at: Some(started_at),
            trace,
            ..Default::default()
        };

        let mut req_bytes = Bytes::new();
        let mut request_tap = None;
        let mut capture: Option<RequestCaptureHandle> = None;
        let mut streamed: Option<Box<crate::request_stream::StreamStart>> = None;
        match policy {
            BodyPolicy::Reject(status, message) => {
                // A client that waits for `100 Continue` has sent no body
                // yet. Any other is mid-upload: its body is read and
                // discarded so it can read the refusal.
                if !expects_continue(&parts.headers) {
                    base.request_capture = Some(
                        crate::tap::drain(body, capture_cap, MAX_REQUEST_BODY as u64, encoded)
                            .await,
                    );
                }
                return self
                    .reject(base, RequestAction::reject_json(status, &message))
                    .await;
            }
            BodyPolicy::PassThrough => {
                // If the provider fails or answers before the upload ended,
                // the rest is drained so the client can read the answer.
                let (tap, handle) = RequestTap::new(body, capture_cap, encoded);
                let tap = tap.drain_when_abandoned();
                base.request_capture = Some(handle.snapshot());
                request_tap = Some(tap);
                capture = Some(handle);
            }
            // A body above the stream lag is forwarded while it uploads, if
            // the firewall agrees once its model is known; anything smaller,
            // or anything the pipeline cannot stream, is buffered as below.
            BodyPolicy::Stream(stream_policy) => {
                match self
                    .prepare_stream(
                        body,
                        &parts.headers,
                        stream_policy,
                        &base,
                        capture_cap,
                        client_length,
                    )
                    .await
                {
                    Prepared::Buffered(bytes) => {
                        req_bytes = bytes;
                        inspect_buffered(&mut base, &parts.headers, req_bytes.clone()).await;
                    }
                    Prepared::Failed { too_large, error } => {
                        return self.body_read_failed(base, started, too_large, error).await
                    }
                    Prepared::Rejected { action, capture } => {
                        base.request_capture = Some(capture);
                        return self.reject(base, action).await;
                    }
                    Prepared::Stream(start) => streamed = Some(start),
                }
            }
            BodyPolicy::Buffered => {
                req_bytes = match http_body_util::Limited::new(body, MAX_REQUEST_BODY)
                    .collect()
                    .await
                {
                    Ok(c) => c.to_bytes(),
                    Err(e) => {
                        let too_large = e.is::<http_body_util::LengthLimitError>();
                        return self
                            .body_read_failed(base, started, too_large, e.to_string())
                            .await;
                    }
                };
                inspect_buffered(&mut base, &parts.headers, req_bytes.clone()).await;
            }
        }

        // A streamed body was judged by `open_stream`; its session protects it.
        let action = if streamed.is_some() {
            RequestAction::Forward
        } else {
            self.recorder.on_request(&base).await
        };
        let mut vault = None;
        let mut progress = None;
        let mut stream_ready = None;
        let forward_body: UpstreamBody = if let Some(start) = streamed.take() {
            // A tee forwards the client's bytes with their encoding and
            // length. Otherwise the forwarded body is decoded and possibly
            // rewritten: hyper frames it as chunked, or with the length of
            // the held output for a provider that requires one.
            if !matches!(start.output, crate::stream_driver::Output::Tee { .. }) {
                crate::request_body::rewritten(&mut parts.headers);
            }
            vault = start.vault.clone();
            base.request_stream = Some(crate::request_stream::streamed_request(
                start.format,
                start.pipeline.metadata(),
                None,
            ));
            base.request_capture = Some(Default::default());
            let handle = (*start).spawn();
            progress = Some(handle.progress);
            stream_ready = Some(handle.ready);
            handle.body.boxed_unsync()
        } else {
            match action {
                RequestAction::Forward => match request_tap.take() {
                    Some(tap) => tap.boxed_unsync(),
                    None => buffered_body(req_bytes),
                },
                // A pass-through body was never read, so there is nothing a
                // rewrite could have been derived from. Refuse rather than
                // forward bytes the recorder meant to replace.
                RequestAction::Replace(_) | RequestAction::Protect { .. } if capture.is_some() => {
                    tracing::error!("reverse proxy: recorder rewrote a pass-through request");
                    return self
                        .reject(
                            base,
                            RequestAction::reject_json(
                                StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                                "Request policy changed while the request was being forwarded",
                            ),
                        )
                        .await;
                }
                RequestAction::Replace(body) => {
                    crate::request_body::rewritten(&mut parts.headers);
                    base.request_body = Some(body.clone());
                    buffered_body(Bytes::from(body))
                }
                RequestAction::Protect {
                    body,
                    vault: capability,
                } => {
                    crate::request_body::rewritten(&mut parts.headers);
                    base.request_body = Some(body.clone());
                    vault = Some(capability);
                    buffered_body(Bytes::from(body))
                }
                rejected @ RequestAction::Reject { .. } => {
                    return self.reject(base, rejected).await
                }
            }
        };
        base.event_id = self.recorder.begin(&base);
        let request_progress = Progress {
            tap: capture.clone(),
            stream: progress,
        };
        let pending_recorder = self.recorder.clone();
        let mut pending_base = base.clone();
        let pending_progress = request_progress.clone();
        let mut pending = crate::tap::CancellationGuard::new(move || {
            if pending_base.event_id.is_none() {
                return;
            }
            pending_progress.apply(&mut pending_base);
            pending_base.status = Some(StatusCode::BAD_GATEWAY.as_u16());
            pending_base.error = Some("Proxy request cancelled before response delivery".into());
            pending_base.latency_ms = Some(started.elapsed().as_millis() as u64);
            tokio::spawn(async move {
                pending_recorder.record(pending_base).await;
            });
        });

        // A streamed body connects to the provider only once it has
        // something to send: a body refused before that never reaches it.
        if let Some(ready) = stream_ready {
            let refused = match ready.await {
                Ok(Ok(())) => None,
                Ok(Err(abort)) => Some(abort),
                Err(_) => Some(crate::request_stream::StreamAbort::new(
                    500,
                    "Request inspection ended unexpectedly",
                )),
            };
            if let Some(abort) = refused {
                pending.disarm();
                return self.aborted(base, &request_progress, started, abort);
            }
        }

        let stream =
            match TcpStream::connect((self.upstream_host.as_str(), self.upstream_port)).await {
                Ok(s) => s,
                Err(e) => {
                    pending.disarm();
                    request_progress.apply(&mut base);
                    return self.fail(
                        base,
                        started,
                        format!("cannot reach {}: {e}", self.upstream),
                    );
                }
            };
        let _ = stream.set_nodelay(true);

        let (mut sender, conn) =
            match hyper::client::conn::http1::handshake(TokioIo::new(stream)).await {
                Ok(pair) => pair,
                Err(e) => {
                    pending.disarm();
                    request_progress.apply(&mut base);
                    return self.fail(base, started, format!("upstream handshake: {e}"));
                }
            };
        tokio::spawn(async move {
            let _ = conn.await;
        });

        // A pass-through or streamed body is read from the client only as
        // fast as the provider accepts it, and the response may arrive
        // before it ends.
        let up_req = Request::from_parts(parts, forward_body);
        let resp = match sender.send_request(up_req).await {
            Ok(r) => r,
            Err(e) => {
                pending.disarm();
                // A streamed upload LocalRouter ended itself names its reason.
                if let Some(abort) = request_progress.stream.as_ref().and_then(|p| p.abort()) {
                    return self.aborted(base, &request_progress, started, abort);
                }
                // A pass-through upload that the client abandoned surfaces
                // here as a failed upstream request; name the real cause.
                let error = match capture.as_ref().and_then(RequestCaptureHandle::error) {
                    Some(client) => format!("client request body: {client}"),
                    None => format!("upstream request: {e}"),
                };
                request_progress.apply(&mut base);
                return self.fail(base, started, error);
            }
        };

        // A streamed body sent chunked that the provider refused for lacking
        // a length: later ones are held and sent with one.
        let chunked_stream = request_progress.stream.as_ref().is_some_and(|p| !p.sized());
        let status = resp.status().as_u16();
        if chunked_stream && crate::framing::refuses_chunked(status, &[]) {
            self.length_required.learn(&self.authority());
        }
        let learn_length = (chunked_stream && status == 400)
            .then(|| (self.length_required.clone(), self.authority()));

        let (mut rparts, rbody) = resp.into_parts();
        let content_type = rparts
            .headers
            .get(hyper::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        base.status = Some(rparts.status.as_u16());
        base.response_is_sse = content_type.contains("text/event-stream");
        base.response_is_ndjson = content_type.contains("application/x-ndjson")
            || content_type.contains("application/jsonlines");
        for name in HOP_BY_HOP {
            rparts.headers.remove(*name);
        }
        // Mark the hop so a curious user can tell the traffic went through
        // LocalRouter — the only header we add.
        rparts.headers.insert(
            HeaderName::from_static("x-localrouter-reverse-proxy"),
            HeaderValue::from_static("1"),
        );

        let recorder = self.recorder.clone();
        let sse = base.response_is_sse;
        let ndjson = base.response_is_ndjson;
        let mut recorded = base;
        let on_end: Box<dyn FnOnce(crate::tap::Capture) + Send> = Box::new(move |mut capture| {
            recorded.response_tail = capture.take_tail();
            let bytes = capture.bytes;
            if let Some((hosts, host)) = learn_length {
                if crate::framing::refuses_chunked(400, &bytes) {
                    hosts.learn(&host);
                }
            }
            // What was forwarded by now; the upload may still be running
            // when the provider answered early.
            request_progress.apply(&mut recorded);
            recorded.error = capture.error;
            recorded.response_body = (!bytes.is_empty()).then_some(bytes);
            recorded.latency_ms = Some(started.elapsed().as_millis() as u64);
            tokio::spawn(async move {
                recorder.record(recorded).await;
            });
        });

        let response = if let Some(vault) = vault {
            match crate::protected_response::restore_recorded(
                rparts, rbody, vault, sse, ndjson, on_end,
            )
            .await
            {
                Ok(response) => response,
                Err(_) => error_response(
                    StatusCode::BAD_GATEWAY,
                    "Protected response restoration failed",
                ),
            }
        } else {
            // Only an LLM response's end matters (its terminal event
            // carries the usage); model pulls and other responses keep the
            // head alone.
            let tapped = if capture_cap > 0 {
                crate::tap::TappedBody::with_tail(rbody, MAX_CAPTURE, on_end)
            } else {
                crate::tap::TappedBody::new(rbody, MAX_CAPTURE, on_end)
            };
            Response::from_parts(rparts, BodyExt::boxed_unsync(tapped))
        };
        pending.disarm();
        response
    }

    fn authority(&self) -> String {
        format!("{}:{}", self.upstream_host, self.upstream_port)
    }

    /// Read a streamable body, as the MITM transport does: until it outgrows
    /// the lag (or when it cannot be streamed) it is held whole, raw, and
    /// nothing is forwarded before the recorder opens the stream. While the
    /// recorder decides (which may wait for an approval popup) the client
    /// keeps uploading and its bytes are held, spilling to disk beyond RAM.
    async fn prepare_stream(
        &self,
        body: Incoming,
        headers: &hyper::HeaderMap,
        policy: StreamPolicy,
        base: &ReverseExchange,
        capture_cap: usize,
        client_length: Option<u64>,
    ) -> Prepared {
        use crate::stream_driver::{ClientReader, Intake, Output, PREVIEW_LIMIT};
        let format = crate::wire::detect(&base.path);
        // Undecodable encodings and unknown formats are judged on the
        // buffered path, exactly as before.
        let streamable =
            format.is_some() && crate::request_body::StreamDecoder::new(headers).is_ok();
        let mut intake = Some(Intake::new(
            headers,
            policy.lag,
            self.spill.clone(),
            MAX_REQUEST_BODY,
            streamable,
        ));
        let mut reader = ClientReader::new(body);
        loop {
            let data = match reader.batch(crate::request_stream::STEP).await {
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
        let mut offer = base.clone();
        if let Some((head, received)) = intake.preview(PREVIEW_LIMIT) {
            offer.request_capture = Some(crate::interceptor::BoundedCapture {
                truncated: received > head.len() as u64,
                head,
                total_len: received,
                complete: false,
                encoded: false,
            });
        }
        // Keep reading (and holding) while the recorder decides.
        let decision = {
            let decide = self.recorder.open_stream(&offer, &metadata, &policy);
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
        drop(offer);
        // Bytes that arrived meanwhile may have shown the body cannot be
        // streamed after all: nothing was sent, so it takes the buffered path.
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
                    let data = match reader.batch(crate::request_stream::STEP).await {
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
                let head = intake.raw_head(capture_cap);
                let received = intake.received();
                drop(intake);
                let (rest, complete) = reader
                    .drain(
                        (MAX_REQUEST_BODY as u64).saturating_sub(received),
                        crate::stream_driver::DRAIN_TIMEOUT,
                    )
                    .await;
                let total_len = received + rest;
                let capture = crate::interceptor::BoundedCapture {
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
                let length_required = self.length_required.requires(&self.authority());
                let output = if tee {
                    Output::Tee {
                        length: client_length,
                        hold: client_length.is_none() && length_required,
                    }
                } else if length_required {
                    Output::Sized
                } else {
                    Output::Chunked
                };
                Prepared::Stream(Box::new(crate::request_stream::StreamStart {
                    reader,
                    decoder,
                    pipeline,
                    raw,
                    session,
                    format: format.expect("a streamable body has a format"),
                    capture_cap,
                    vault,
                    output,
                    strict: policy.strict,
                    encoded: tee && crate::tap::content_encoded(headers),
                    spill: self.spill.clone(),
                }))
            }
        }
    }

    /// Record a streamed upload LocalRouter ended, and answer the caller
    /// with the reason (the provider never saw a complete body).
    fn aborted(
        &self,
        mut ex: ReverseExchange,
        progress: &Progress,
        started: Instant,
        abort: crate::request_stream::StreamAbort,
    ) -> Response<BoxedBody> {
        let status = StatusCode::from_u16(abort.status).unwrap_or(StatusCode::BAD_GATEWAY);
        tracing::warn!(
            client_id = %ex.client_id,
            "reverse proxy {}: {}",
            status.as_u16(),
            abort.message
        );
        progress.apply(&mut ex);
        ex.status = Some(status.as_u16());
        ex.error = Some(abort.message.clone());
        ex.latency_ms = Some(started.elapsed().as_millis() as u64);
        let recorder = self.recorder.clone();
        tokio::spawn(async move {
            recorder.record(ex).await;
        });
        match abort.json {
            Some(json) => Response::builder()
                .status(status)
                .header(hyper::header::CONTENT_TYPE, "application/json")
                .body(
                    Full::new(Bytes::from(json))
                        .map_err(|e: Infallible| match e {})
                        .boxed_unsync(),
                )
                .expect("abort response"),
            None => error_response(status, &abort.message),
        }
    }

    /// Answer a request whose body could not be read for inspection, and
    /// record the failure.
    async fn body_read_failed(
        &self,
        mut ex: ReverseExchange,
        started: Instant,
        too_large: bool,
        error: String,
    ) -> Response<BoxedBody> {
        let (status, error) = if too_large {
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "request body exceeds the reverse proxy's {} MiB limit",
                    MAX_REQUEST_BODY / (1024 * 1024)
                ),
            )
        } else {
            (
                StatusCode::BAD_REQUEST,
                format!("failed reading request body: {error}"),
            )
        };
        tracing::warn!(client_id = %ex.client_id, "reverse proxy {}: {error}", status.as_u16());
        ex.status = Some(status.as_u16());
        ex.error = Some(error.clone());
        ex.latency_ms = Some(started.elapsed().as_millis() as u64);
        self.recorder.record(ex).await;
        error_response(status, &error)
    }

    /// Record a refused request and answer the caller without contacting the
    /// provider.
    async fn reject(
        &self,
        mut base: ReverseExchange,
        action: RequestAction,
    ) -> Response<BoxedBody> {
        let RequestAction::Reject {
            status,
            content_type,
            body,
        } = action
        else {
            unreachable!("reject is only called with a Reject action");
        };
        base.status = Some(status);
        base.response_body = Some(body.clone());
        base.refused_locally = true;
        self.recorder.record(base).await;
        Response::builder()
            .status(status)
            .header(hyper::header::CONTENT_TYPE, content_type)
            .body(
                Full::new(Bytes::from(body))
                    .map_err(|e: Infallible| match e {})
                    .boxed_unsync(),
            )
            .expect("firewall response")
    }

    /// Record a failed attempt and answer the caller with a diagnosable 502.
    /// The message names the upstream, because the overwhelmingly likely cause
    /// is that the wrapped provider was never relocated (or is not running).
    fn fail(&self, mut ex: ReverseExchange, started: Instant, msg: String) -> Response<BoxedBody> {
        tracing::warn!(client_id = %ex.client_id, "reverse proxy: {msg}");
        ex.status = Some(StatusCode::BAD_GATEWAY.as_u16());
        ex.latency_ms = Some(started.elapsed().as_millis() as u64);
        ex.error = Some(msg.clone());
        let recorder = self.recorder.clone();
        tokio::spawn(async move {
            recorder.record(ex).await;
        });
        error_response(StatusCode::BAD_GATEWAY, &msg)
    }
}

type BoxedBody = http_body_util::combinators::UnsyncBoxBody<Bytes, std::io::Error>;
/// The request body sent to the provider: buffered bytes, or the client's
/// body forwarded as it arrives.
type UpstreamBody =
    http_body_util::combinators::UnsyncBoxBody<Bytes, Box<dyn std::error::Error + Send + Sync>>;

/// The outcome of reading a [`BodyPolicy::Stream`] body up to the point
/// where it either ended or outgrew its lag.
enum Prepared {
    /// The whole body, for the buffered path.
    Buffered(Bytes),
    Failed {
        too_large: bool,
        error: String,
    },
    Rejected {
        action: RequestAction,
        capture: crate::interceptor::BoundedCapture,
    },
    Stream(Box<crate::request_stream::StreamStart>),
}

impl From<crate::stream_driver::IntakeError> for Prepared {
    fn from(error: crate::stream_driver::IntakeError) -> Self {
        use crate::stream_driver::IntakeError;
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

/// Push a batch into the intake off the executor (it may decode, lex and
/// spill).
async fn intake_step(
    intake: &mut Option<crate::stream_driver::Intake>,
    data: Vec<u8>,
) -> Result<(), crate::stream_driver::IntakeError> {
    let mut taken = intake.take().expect("intake between steps");
    let (returned, result) = tokio::task::spawn_blocking(move || {
        let result = taken.push(&data);
        (taken, result)
    })
    .await
    .map_err(|_| crate::stream_driver::IntakeError::Failed("request intake failed".into()))?;
    *intake = Some(returned);
    result
}

/// Decode a buffered body for the recorder.
async fn inspect_buffered(base: &mut ReverseExchange, headers: &hyper::HeaderMap, bytes: Bytes) {
    let inspected = crate::request_body::inspect(&base.path, headers, bytes).await;
    base.request_body = inspected
        .as_ref()
        .ok()
        .filter(|b| !b.is_empty())
        .map(|b| b.to_vec());
    base.request_body_error = inspected.err();
}

/// What the proxy can report about a request body that was not buffered: a
/// pass-through tap, or a streamed upload.
#[derive(Clone, Default)]
struct Progress {
    tap: Option<RequestCaptureHandle>,
    stream: Option<crate::request_stream::StreamProgress>,
}

impl Progress {
    fn apply(&self, ex: &mut ReverseExchange) {
        if let Some(tap) = &self.tap {
            ex.request_capture = Some(tap.snapshot());
        }
        if let Some(stream) = &self.stream {
            ex.request_capture = Some(stream.snapshot());
            if let Some(summary) = stream.summary() {
                ex.request_stream = Some(summary);
            }
        }
    }
}

/// A fully buffered request body for the provider.
fn buffered_body(bytes: Bytes) -> UpstreamBody {
    Full::new(bytes)
        .map_err(|e: Infallible| match e {})
        .boxed_unsync()
}

/// Whether the client holds its body back until it is told to continue.
fn expects_continue(headers: &hyper::HeaderMap) -> bool {
    headers
        .get(hyper::header::EXPECT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("100-continue"))
}

/// Split `http://host:port` into its parts, rejecting anything else.
///
/// Public so callers that need the same host/port view of a configured upstream
/// (status probes, setup UI) parse it exactly the way the data path does,
/// rather than re-deriving it and disagreeing on edge cases like a trailing
/// `/v1` path.
pub fn parse_http_upstream(url: &str) -> Result<(String, u16), ProxyError> {
    let url = url.trim().trim_end_matches('/');
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        ProxyError::Config(format!(
            "reverse-proxy upstream must be a plain http:// URL (got '{url}')"
        ))
    })?;
    // Ignore any path component — forwarding always uses the request's own path.
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty() {
        return Err(ProxyError::Config(
            "reverse-proxy upstream has no host".into(),
        ));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => {
            let port: u16 = p.parse().map_err(|_| {
                ProxyError::Config(format!("invalid reverse-proxy upstream port '{p}'"))
            })?;
            (h.to_string(), port)
        }
        None => (authority.to_string(), 80),
    };
    if host.is_empty() || port == 0 {
        return Err(ProxyError::Config(format!(
            "invalid reverse-proxy upstream '{url}'"
        )));
    }
    Ok((host, port))
}

fn error_response(status: StatusCode, msg: &str) -> Response<BoxedBody> {
    let body = serde_json::json!({
        "error": {
            "message": format!("LocalRouter reverse proxy: {msg}"),
            "type": "reverse_proxy_error",
        }
    })
    .to_string();
    Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .body(
            Full::new(Bytes::from(body))
                .map_err(|e: Infallible| match e {})
                .boxed_unsync(),
        )
        .expect("static response builds")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_http_upstreams() {
        assert_eq!(
            parse_http_upstream("http://127.0.0.1:11435").unwrap(),
            ("127.0.0.1".to_string(), 11435)
        );
        assert_eq!(
            parse_http_upstream("http://localhost:1235/v1/").unwrap(),
            ("localhost".to_string(), 1235)
        );
        assert_eq!(
            parse_http_upstream("http://example.test").unwrap(),
            ("example.test".to_string(), 80)
        );
    }

    #[test]
    fn rejects_non_http_upstreams() {
        assert!(parse_http_upstream("https://127.0.0.1:11435").is_err());
        assert!(parse_http_upstream("127.0.0.1:11435").is_err());
        assert!(parse_http_upstream("http://").is_err());
        assert!(parse_http_upstream("http://host:abc").is_err());
        assert!(parse_http_upstream("http://host:0").is_err());
    }
}
