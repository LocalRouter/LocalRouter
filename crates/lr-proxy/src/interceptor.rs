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
    /// Block the request; return this synthesized response to the client and
    /// never contact the upstream (firewall deny).
    Reject {
        status: u16,
        content_type: String,
        body: Vec<u8>,
    },
}

impl RequestAction {
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

/// A decrypted HTTP exchange handed to the interceptor for observation.
///
/// Bodies are raw, size-capped byte copies captured by the transport (which
/// stays protocol-agnostic); the interceptor decides how to parse them. Large
/// or streaming payloads are truncated at the cap, never buffered unbounded.
#[derive(Debug, Clone, Default)]
pub struct ObservedExchange {
    /// The client this exchange belongs to.
    pub client_id: String,
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
    /// Raw request body bytes (capped), if any.
    pub request_body: Option<Vec<u8>>,
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

    /// Called with the decrypted request before it is forwarded. The firewall
    /// evaluates here and may forward, rewrite, or reject. Awaited by the
    /// transport (so an "ask" rule can pause for user approval).
    async fn on_request(&self, _exchange: &ObservedExchange) -> RequestAction {
        RequestAction::Forward
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
