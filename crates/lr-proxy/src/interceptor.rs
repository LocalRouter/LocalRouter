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
    /// The client's routing strategy id (for metrics attribution).
    pub strategy_id: String,
    /// Wall-clock latency of the exchange in milliseconds, once known.
    pub latency_ms: Option<u64>,
    /// Upstream host (e.g. `api.anthropic.com`).
    pub host: String,
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

/// Resolves the USD cost of a call from its model + token usage. Implemented by
/// the app against the model catalog; kept as a trait so `lr-proxy` doesn't
/// depend on the catalog/provider crates.
pub trait PricingResolver: Send + Sync {
    fn cost_usd(&self, model: &str, usage: TokenUsage) -> Option<f64>;
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

    /// Called with the decrypted (and, for SSE, reconstructed) response at end
    /// of stream. Used to record the exchange (monitor + metrics).
    async fn on_response(&self, _exchange: &ObservedExchange) {}
}
