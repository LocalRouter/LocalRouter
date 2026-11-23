//! The interception abstraction that decouples the MITM transport from what we
//! do with the decrypted traffic.
//!
//! Passive mode (today) only *observes* — it records requests/responses to the
//! monitor and forwards them unchanged. Active mode (future) will return
//! `Replace(..)` from the request/response hooks to rewrite model selection,
//! apply JSON optimization, enforce allow-lists, etc. The transport layer never
//! needs to change between the two — only the interceptor implementation does.

use async_trait::async_trait;

/// Identity + policy for the client that opened a proxied tunnel.
#[derive(Debug, Clone, Default)]
pub struct ClientCtx {
    /// Resolved LocalRouter client id (from proxy auth).
    pub client_id: String,
    /// The client's routing strategy id (for metrics attribution).
    pub strategy_id: String,
    /// Whether this client is allowed to use the proxy at all.
    pub proxy_enabled: bool,
}

/// What to do with a `CONNECT host:port` tunnel request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectDecision {
    /// Terminate TLS and inspect (MITM) this host.
    Mitm,
    /// Forward bytes blindly without decrypting (e.g. auth/telemetry hosts).
    Tunnel,
    /// Refuse the tunnel (e.g. unauthenticated client).
    Reject(&'static str),
}

/// What to do with a parsed request head + body on an intercepted connection.
///
/// Passive mode always returns `Forward`. The `Replace` variant is the seam for
/// active rewriting; it is intentionally opaque here so the transport crate owns
/// the concrete request type.
/// What the transport should do with an intercepted request.
pub enum RequestAction {
    /// Forward the original request unchanged.
    Forward,
    /// Forward a rewritten request body instead (e.g. model rewrite / transform).
    Replace(Vec<u8>),
    /// Masked wire request and its request-owned restoration capability.
    Protect {
        body: Vec<u8>,
        vault: std::sync::Arc<lr_secret_scanner::placeholders::Vault>,
    },
    /// Block the request; return this synthesized response to the client and
    /// never contact the upstream (firewall deny).
    Reject {
        status: u16,
        content_type: String,
        body: Vec<u8>,
    },
}

impl RequestAction {
    pub fn secret_scan_denied() -> Self {
        Self::Reject { status:403, content_type:"application/json".into(), body:serde_json::json!({"type":"error","error":{"type":"secret_scan_denied_by_user","code":"secret_scan_denied_by_user","message":"Request denied by your Secret Scan decision"}}).to_string().into_bytes() }
    }
    /// A JSON error `Reject` in the OpenAI/Anthropic-ish error envelope.
    pub fn reject_json(status: u16, message: &str) -> Self {
        let body = serde_json::json!({
            "type": "error",
            "error": { "type": "localrouter_firewall", "message": message }
        });
        RequestAction::Reject {
            status,
            content_type: "application/json".to_string(),
            body: serde_json::to_vec(&body).unwrap_or_default(),
        }
    }
}

/// The head of a decrypted request, available before any body byte is read.
///
/// [`ProxyInterceptor::body_policy`] decides from this alone whether the body
/// has to be buffered for inspection, can be forwarded as it arrives, or is
/// refused outright.
#[derive(Debug, Clone, Copy)]
pub struct RequestHead<'a> {
    /// The client this request belongs to.
    pub client_id: &'a str,
    /// The client's routing strategy id.
    pub strategy_id: &'a str,
    /// Upstream host; the wrapped provider's name on the reverse proxy.
    pub host: &'a str,
    /// Upstream port, `0` on the reverse proxy.
    pub port: u16,
    /// Request method (e.g. `POST`).
    pub method: &'a str,
    /// Request path including the query string.
    pub path: &'a str,
    /// Request headers as they will be forwarded.
    pub headers: &'a http::HeaderMap,
    /// Cross-hop trace stamped on the forwarded request.
    pub trace: Option<&'a lr_types::RequestTrace>,
    /// Provider name when the host does not identify it (reverse proxy).
    pub provider_override: Option<&'a str>,
    /// Which proxy received the request.
    pub source: ExchangeSource,
}

impl RequestHead<'_> {
    /// Whether an earlier LocalRouter hop already handled this request.
    pub fn is_duplicate_hop(&self) -> bool {
        self.trace.is_some_and(|t| t.is_duplicate())
    }

    /// Catalog/metrics provider name, as [`crate::active::FirewallRequest`]
    /// carries it.
    pub fn provider(&self) -> String {
        self.provider_override
            .map(str::to_owned)
            .unwrap_or_else(|| crate::wire::provider_for_host(self.host).to_string())
    }

    /// The content-decoding failure that is already certain from the headers:
    /// a `Content-Encoding` the proxy cannot decode on a path it inspects.
    /// Corrupt or oversized bodies are only known once the body is read.
    pub fn content_encoding_error(&self) -> Option<crate::request_body::DecodeError> {
        crate::wire::detect(self.path)?;
        let mut count = 0usize;
        for value in self.headers.get_all(http::header::CONTENT_ENCODING) {
            let Ok(value) = value.to_str() else {
                return Some(crate::request_body::DecodeError::Unsupported);
            };
            for encoding in value.split(',') {
                let encoding = encoding.trim().to_ascii_lowercase();
                count += 1;
                if count > MAX_CONTENT_ENCODINGS
                    || !matches!(
                        encoding.as_str(),
                        "identity" | "zstd" | "gzip" | "deflate" | "br"
                    )
                {
                    return Some(crate::request_body::DecodeError::Unsupported);
                }
            }
        }
        None
    }
}

/// Encodings a request may stack; mirrors the decoder in [`crate::request_body`].
const MAX_CONTENT_ENCODINGS: usize = 4;

/// Default lag between detection and release for a streamed request body.
pub const DEFAULT_STREAM_LAG: usize = 16 * 1024 * 1024;

/// What a streamed request body is subjected to while it is forwarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamMasking {
    /// Nothing is detected or masked; the original bytes are forwarded.
    Off,
    /// Secrets are detected and reported; the original bytes are forwarded.
    Notify,
    /// Secrets are detected and masked before the bytes are released.
    Redact,
    /// Secrets are detected; each new one stops the release at its unit
    /// until the user answered (a denial ends the upload), and the bytes
    /// are masked as decided.
    Ask,
}

/// How an inspected request body is streamed to the upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamPolicy {
    pub masking: StreamMasking,
    /// Nothing is released until the request's `model` is known and permitted.
    pub needs_model: bool,
    /// A consumer needs the whole document, so nothing is released before the
    /// body ends.
    pub whole_document: bool,
    /// Bytes the release cursor stays behind the detection cursor.
    pub lag: usize,
    /// A vault from earlier turns exists and only restores references.
    pub restore_only: bool,
    /// Strict mode: nothing is released before the whole body was inspected
    /// and every question answered (held bytes spill to disk), whatever its
    /// size. Masking modes only; original bytes forwarded for Off/Notify
    /// are not held.
    pub strict: bool,
}

/// How the transport handles a request body, decided from the head alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyPolicy {
    /// Nothing inspects this body: forward it as it arrives, with its length
    /// and encoding intact. The transport still calls
    /// [`ProxyInterceptor::on_request`], with `request_body: None` and
    /// `request_capture: Some(..)`; only `Forward` and `Reject` are honored
    /// there, a rewrite fails the request closed.
    PassThrough,
    /// Refuse the request with this status and message without contacting the
    /// upstream.
    Reject(u16, String),
    /// Inspect and forward the body incrementally (MITM HTTP only; the
    /// reverse proxy buffers such requests). A body of at most `lag` bytes is
    /// still read whole and judged by [`ProxyInterceptor::on_request`]; a
    /// larger one is offered to [`ProxyInterceptor::open_stream`].
    Stream(StreamPolicy),
    /// Read the whole body before deciding anything.
    Buffered,
}

/// How a body above the stream lag is handled, decided by
/// [`ProxyInterceptor::open_stream`] once its metadata is known.
pub enum StreamOpen {
    /// Forward it through this session as it arrives.
    Stream {
        session: Box<dyn crate::request_stream::StreamSession>,
        /// Restores references in the response, as for `RequestAction::Protect`.
        vault: Option<std::sync::Arc<lr_secret_scanner::placeholders::Vault>>,
        /// The session never changes the body: forward the client's original
        /// bytes (with its encoding and length) as they arrive and only
        /// inspect a decoded copy.
        tee: bool,
    },
    /// Read it whole and judge it with `on_request`, as a buffered request.
    Buffer,
    /// Refuse it without contacting the upstream.
    Reject(RequestAction),
}

/// What the monitor knows about a request body that was streamed.
#[derive(Debug, Clone, Default)]
pub struct StreamedRequest {
    /// Request metadata from the sniffer.
    pub meta: crate::wire::RequestMeta,
    pub tool_count: usize,
    /// Explicit conversation id the body carries in its metadata fields.
    pub conversation_id: Option<String>,
    /// Correlation inputs, once the whole body was forwarded.
    pub correlation: Option<lr_monitor::CorrelationParts>,
}

/// A bounded view of a request body that was forwarded without being buffered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoundedCapture {
    /// The first bytes of the body, at most the transport's capture cap.
    /// Always empty on paths that are not LLM calls.
    pub head: Vec<u8>,
    /// Bytes of the body read so far; its full length once `complete`.
    pub total_len: u64,
    /// `head` holds less than `total_len` bytes.
    pub truncated: bool,
    /// The body was read to its end. False while it is still uploading, and
    /// when the client or the upstream ended the exchange early.
    pub complete: bool,
    /// The body has a non-identity `Content-Encoding`, so `head` is not text.
    pub encoded: bool,
}

impl BoundedCapture {
    /// The whole body, when the capture holds all of it in a parseable form.
    pub fn inspectable(&self) -> Option<&[u8]> {
        (self.complete && !self.truncated && !self.encoded && !self.head.is_empty())
            .then_some(self.head.as_slice())
    }
}

/// A decrypted HTTP exchange handed to the interceptor for observation.
///
/// The request body is the whole decoded body when the transport buffered it
/// for inspection, so it can be as large as the transport's request limit. A
/// request forwarded without buffering has no `request_body`; it carries a
/// bounded [`request_capture`](Self::request_capture) instead. The response
/// body is a copy truncated at the transport's capture cap.
#[derive(Debug, Clone, Default)]
pub struct ObservedExchange {
    /// The client this exchange belongs to.
    pub client_id: String,
    /// Per-request identity, shared with scans and approval decisions. Each
    /// websocket cycle gets its own ID, independently of the connection trace.
    pub session_id: Option<String>,
    /// Explicit authenticated conversation affinity, independent of monitor IDs.
    pub secret_session: Option<String>,
    /// Monitor event id of the pending event opened at request time, so the
    /// response half updates it (pending → complete) instead of pushing a new one.
    pub event_id: Option<String>,
    /// The client's routing strategy id (for metrics attribution).
    pub strategy_id: String,
    /// Wall-clock latency of the exchange in milliseconds, once known.
    pub latency_ms: Option<u64>,
    /// Upstream host (e.g. `api.anthropic.com`).
    pub host: String,
    /// Upstream port. `0` when the capture site doesn't have one (the reverse
    /// proxy, which identifies its upstream by provider instead).
    pub port: u16,
    /// Request method (e.g. `POST`).
    pub method: String,
    /// Request path (e.g. `/v1/messages`).
    pub path: String,
    /// The whole decoded request body, when the transport buffered it.
    pub request_body: Option<Vec<u8>>,
    /// Bounded capture of a request body that was not buffered. `Some` on a
    /// request handed to [`ProxyInterceptor::on_request`] marks it as
    /// pass-through, before any byte was read.
    pub request_capture: Option<BoundedCapture>,
    /// Set when the body was streamed: `request_capture` then holds the head
    /// of the forwarded (masked) bytes, and this the sniffed metadata.
    pub request_stream: Option<StreamedRequest>,
    /// Safe content-decoding failure; active policy must reject it rather than
    /// evaluating missing JSON. Passive forwarding retains the original bytes.
    pub request_body_error: Option<crate::request_body::DecodeError>,
    /// Response status code, once the response head is available.
    pub status: Option<u16>,
    /// Raw response body bytes (capped), if any. For SSE this is the raw event
    /// stream; see [`response_is_sse`](Self::response_is_sse).
    pub response_body: Option<Vec<u8>>,
    /// Whether the response was an SSE stream (`text/event-stream`).
    pub response_is_sse: bool,
    /// Whether the response was newline-delimited JSON (Ollama's native
    /// streaming encoding, seen by the reverse proxy).
    pub response_is_ndjson: bool,
    /// Provider name for metrics/pricing attribution, when the host doesn't
    /// identify it. The reverse proxy sets this from the wrapped provider
    /// instance; the MITM proxy leaves it unset and infers from the host.
    pub provider_override: Option<String>,
    /// Monitor event source, so reverse-proxied calls are distinguishable from
    /// MITM-proxied ones in the UI.
    pub source: ExchangeSource,
    /// Transport-level failure (e.g. the upstream could not be reached), shown
    /// on the monitor event.
    pub error: Option<String>,
    /// Cross-hop trace stamped on the forwarded request. `hop > 1` means an
    /// earlier LocalRouter hop already handled this request: it is passed
    /// through (no firewall) and not counted in metrics.
    pub trace: Option<lr_types::RequestTrace>,
    /// Websocket handshake traces identify a connection, not an individual
    /// request. Retain them for hop accounting, but never use them to group turns.
    pub connection_trace: bool,
    /// Usage account the request bills to (Claude subscription, OpenAI API,
    /// …), classified from how it authenticated. Feeds the usage ledger.
    pub usage_account: Option<lr_usage::AccountRef>,
    /// The rate-limit admission the firewall reserved for this exchange.
    /// Shared by every clone of the exchange; settled when its usage is
    /// recorded, released when the last clone is dropped.
    pub admission: ExchangeAdmission,
    /// LocalRouter answered the request itself (a firewall refusal, an
    /// unreadable body) without contacting the upstream. Recorded in the
    /// monitor, but not as a request in the metrics and the usage ledger.
    pub refused_locally: bool,
}

/// Usage recorded for an exchange, handed to its admission when the
/// metrics row has been written.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RecordedUsage {
    /// The upstream answered and reported what it billed (a complete
    /// exchange, or a successful response interrupted after it reported
    /// usage).
    pub billed: bool,
    /// The whole prompt, cache reads and writes included.
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

/// What a firewall reserved for one exchange (rate-limit shares). Dropping
/// it releases the reservation.
pub trait AdmissionGuard: Send {
    /// The exchange's usage has been recorded: account for it, then release.
    fn settle(self: Box<Self>, usage: &RecordedUsage);
}

/// The admission slot of one exchange. Clones share the slot, so a guard the
/// firewall puts there through a [`crate::active::FirewallRequest`] stays with
/// the exchange until it is settled or every clone is gone.
#[derive(Clone, Default)]
pub struct ExchangeAdmission(std::sync::Arc<parking_lot::Mutex<Option<Box<dyn AdmissionGuard>>>>);

impl std::fmt::Debug for ExchangeAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ExchangeAdmission")
            .field(&self.is_held())
            .finish()
    }
}

impl ExchangeAdmission {
    /// Keep `guard` for this exchange, releasing whatever was held before.
    pub fn hold(&self, guard: Box<dyn AdmissionGuard>) {
        let previous = self.0.lock().replace(guard);
        drop(previous);
    }

    /// Whether a reservation is held.
    pub fn is_held(&self) -> bool {
        self.0.lock().is_some()
    }

    /// Release the reservation without accounting for any usage.
    pub fn release(&self) {
        let held = self.0.lock().take();
        drop(held);
    }

    /// Account for the recorded usage and release the reservation.
    pub fn settle(&self, usage: &RecordedUsage) {
        // Outside the slot's lock: settling takes rate-limiter locks.
        let held = self.0.lock().take();
        if let Some(guard) = held {
            guard.settle(usage);
        }
    }
}

impl ObservedExchange {
    /// Whether an earlier LocalRouter hop already handled this request.
    pub fn is_duplicate_hop(&self) -> bool {
        self.trace.as_ref().is_some_and(|t| t.is_duplicate())
    }
}

/// Where an observed exchange was captured.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExchangeSource {
    /// The HTTPS inspection (MITM) proxy.
    #[default]
    Proxy,
    /// A reverse-proxy listener wrapping a local provider's port.
    ReverseProxy,
}

/// A connection that passed through the proxy **without** being treated as an
/// LLM call, handed to the interceptor purely so the user can see it happened.
///
/// `HTTPS_PROXY` is process-wide: pointing a tool at LocalRouter also routes
/// that tool's git, package-manager, telemetry and update traffic here. Such
/// traffic is forwarded verbatim; this type carries only *where* it went and
/// how many bytes moved — never any request or response content.
#[derive(Debug, Clone, Default)]
pub struct PassthroughExchange {
    /// The authenticated client whose proxy settings carried this connection.
    pub client_id: String,
    /// How it was forwarded.
    pub mode: lr_monitor::PassthroughMode,
    /// Destination host.
    pub host: String,
    /// Destination port.
    pub port: u16,
    /// Request method, only when the request line was visible in cleartext.
    pub method: Option<String>,
    /// Request path **with the query string stripped**, only when visible.
    pub path: Option<String>,
    /// Upstream status code, when the response head was visible.
    pub status: Option<u16>,
    /// Bytes forwarded client → upstream.
    pub bytes_sent: u64,
    /// Bytes forwarded upstream → client.
    pub bytes_received: u64,
    /// Wall-clock duration once the connection closes.
    pub latency_ms: Option<u64>,
    /// Transport failure, if the passthrough could not be completed.
    pub error: Option<String>,
}

impl PassthroughExchange {
    /// A blind `CONNECT` tunnel to a host that is not inspected.
    pub fn tunnel(client_id: &str, host: &str, port: u16) -> Self {
        Self {
            client_id: client_id.to_string(),
            mode: lr_monitor::PassthroughMode::Tunnel,
            host: host.to_string(),
            port,
            ..Default::default()
        }
    }

    /// Drop the query string from a path, so no parameter values are ever
    /// recorded for passthrough traffic.
    pub fn strip_query(path: &str) -> String {
        path.split('?').next().unwrap_or(path).to_string()
    }
}

/// Token usage for a single call, for cost computation.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenUsage {
    /// Prompt tokens neither read from nor written to the cache (the
    /// Anthropic `input_tokens` sense): `cache_read` and `cache_write` are
    /// separate, not part of it.
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub reasoning: u64,
}

/// Resolves the USD cost of a call from its provider + model + token usage.
/// Implemented by the app against the model catalog; kept as a trait so
/// `lr-proxy` doesn't depend on the catalog/provider crates.
pub trait PricingResolver: Send + Sync {
    fn cost_usd(&self, provider: &str, model: &str, usage: TokenUsage) -> Option<f64>;
}

/// Resolves a client's display name from its id, so monitor events recorded by
/// the proxy show the same client name as the gateway path instead of a raw
/// UUID. Implemented by the app against the client manager; kept as a trait so
/// `lr-proxy` doesn't depend on `lr-clients`.
pub trait ClientNameResolver: Send + Sync {
    fn name_for(&self, client_id: &str) -> Option<String>;
}

/// Hooks the transport calls at each stage of an intercepted connection.
///
/// All methods have observe-only default behavior so a passive implementation
/// only needs to override what it cares about.
#[async_trait]
pub trait ProxyInterceptor: Send + Sync {
    /// Decide MITM vs blind tunnel vs reject for a new `CONNECT`.
    fn on_connect(&self, host: &str, client: &ClientCtx) -> ConnectDecision;

    /// Decide from the request head how the body is handled, before any of
    /// it is read. The default buffers every body, which is always safe.
    /// Return [`BodyPolicy::PassThrough`] only for a request whose body
    /// [`on_request`](Self::on_request) would neither read nor change.
    /// Not consulted for the websocket upgrade itself; the relay asks it about
    /// every client message of an LLM websocket as for a `POST` of that
    /// message to the connection's path (with no headers), and uses only a
    /// [`BodyPolicy::Stream`] answer (anything else holds the message whole).
    fn body_policy(&self, _head: &RequestHead<'_>) -> BodyPolicy {
        BodyPolicy::Buffered
    }

    /// Called with the decrypted request before it is forwarded. The firewall
    /// evaluates here and may forward, rewrite, or reject. Awaited by the
    /// transport (so an "ask" rule can pause for user approval).
    async fn on_request(&self, _exchange: &ObservedExchange) -> RequestAction {
        RequestAction::Forward
    }

    /// Decide how a [`BodyPolicy::Stream`] body that outgrew its lag is
    /// handled. `exchange` has no body; its `request_capture` holds the
    /// first decoded bytes (at most [`crate::stream_driver::PREVIEW_LIMIT`],
    /// `total_len` the decoded bytes received so far) for a read-only
    /// preview. `metadata` is what the sniffer has seen so far (its `model`
    /// is known). The client keeps uploading while this is awaited; its
    /// bytes are held. The default reads it whole.
    async fn open_stream(
        &self,
        _exchange: &ObservedExchange,
        _metadata: &lr_secret_scanner::request_stream::RequestMetadata,
        _policy: &StreamPolicy,
    ) -> StreamOpen {
        StreamOpen::Buffer
    }

    /// Called by the transport once the request has been accepted for forwarding
    /// (firewall allowed it), before the upstream response arrives. Opens a
    /// pending monitor event and returns its id so [`on_response`](Self::on_response)
    /// can complete it (pending → complete). Observe-only default records nothing.
    fn begin(&self, _exchange: &ObservedExchange) -> Option<String> {
        None
    }

    /// Called with the decrypted (and, for SSE, reconstructed) response at end
    /// of stream. Completes the pending event opened by [`begin`](Self::begin)
    /// (via `exchange.event_id`), or records the exchange in one push when there
    /// is none (e.g. a firewall reject).
    async fn on_response(&self, _exchange: &ObservedExchange) {}

    /// Called when a non-LLM connection starts being forwarded untouched
    /// (a blind tunnel, or a plain-HTTP proxy request). Returns a monitor event
    /// id so [`end_passthrough`](Self::end_passthrough) can complete it.
    fn begin_passthrough(&self, _exchange: &PassthroughExchange) -> Option<String> {
        None
    }

    /// Called when a forwarded non-LLM connection closes, with byte counts and
    /// duration. Completes the event opened by
    /// [`begin_passthrough`](Self::begin_passthrough), or records a combined one
    /// when there is none.
    fn end_passthrough(&self, _event_id: Option<String>, _exchange: &PassthroughExchange) {}
}
