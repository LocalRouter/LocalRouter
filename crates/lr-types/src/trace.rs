//! Cross-hop request trace.
//!
//! Every LocalRouter component that forwards an LLM request (the gateway, the
//! HTTPS inspection proxy, the reverse proxy) stamps the outgoing request with
//! [`TRACE_HEADER`]. A downstream LocalRouter hop that sees the header knows
//! the request has already been handled once and passes it through: it still
//! observes and logs the exchange, but performs no active rewriting or
//! enforcement and does not count it in stats a second time.
//!
//! Wire format: `X-LocalRouter-Trace: <trace_id>;hop=<n>`.

use serde::{Deserialize, Serialize};

/// Header name (lowercase, as HTTP header names are case-insensitive).
pub const TRACE_HEADER: &str = "x-localrouter-trace";

/// Identity of one logical request as it travels through LocalRouter hops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestTrace {
    /// Stable id shared by every hop of the same logical request.
    pub trace_id: String,
    /// 1 for the first LocalRouter hop, incremented by each forwarding hop.
    pub hop: u32,
}

impl RequestTrace {
    /// A fresh trace for a request LocalRouter is seeing for the first time.
    pub fn new() -> Self {
        Self {
            trace_id: uuid::Uuid::new_v4().to_string(),
            hop: 1,
        }
    }

    /// Parse a header value. Returns `None` for anything malformed so a bogus
    /// header is simply treated as absent.
    pub fn parse(header: &str) -> Option<Self> {
        let mut parts = header.split(';').map(str::trim);
        let trace_id = parts.next().filter(|s| !s.is_empty())?;
        if !trace_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            || trace_id.len() > 128
        {
            return None;
        }
        let mut hop = None;
        for part in parts {
            if let Some(v) = part.strip_prefix("hop=") {
                hop = Some(v.parse::<u32>().ok()?);
            }
        }
        Some(Self {
            trace_id: trace_id.to_string(),
            hop: hop.filter(|h| *h >= 1)?,
        })
    }

    /// The trace to stamp on the request when forwarding it onward.
    pub fn next_hop(&self) -> Self {
        Self {
            trace_id: self.trace_id.clone(),
            hop: self.hop.saturating_add(1),
        }
    }

    /// Serialized header value.
    pub fn header_value(&self) -> String {
        format!("{};hop={}", self.trace_id, self.hop)
    }

    /// Whether an earlier LocalRouter hop already handled this request.
    pub fn is_duplicate(&self) -> bool {
        self.hop > 1
    }

    /// Given the trace found on an inbound request (if any), the trace to use
    /// for the outbound request.
    pub fn outbound_for(inbound: Option<&RequestTrace>) -> Self {
        inbound.map(RequestTrace::next_hop).unwrap_or_default()
    }
}

impl Default for RequestTrace {
    fn default() -> Self {
        Self::new()
    }
}

tokio::task_local! {
    /// The trace of the request the current task is handling, as it will be
    /// stamped on any upstream request this task sends. Set once per inbound
    /// request by the server / proxy entry points; read by the HTTP client
    /// middleware (to add the header) and by the accounting layers (to skip
    /// counting a duplicate hop). `None` means dedupe is disabled.
    pub static OUTBOUND_TRACE: Option<RequestTrace>;
}

/// Run `fut` with `trace` as the current request trace.
pub async fn with_outbound_trace<F: std::future::Future>(
    trace: Option<RequestTrace>,
    fut: F,
) -> F::Output {
    OUTBOUND_TRACE.scope(trace, fut).await
}

/// The current request trace, if one is in scope.
pub fn current_outbound_trace() -> Option<RequestTrace> {
    OUTBOUND_TRACE.try_with(|t| t.clone()).ok().flatten()
}

/// Whether the request the current task is handling was already handled by
/// an earlier LocalRouter hop — in which case it must be passed through
/// unmodified and must not be counted in stats again.
pub fn is_duplicate_hop() -> bool {
    current_outbound_trace().is_some_and(|t| t.is_duplicate())
}

/// `tokio::spawn` that carries the current request trace into the new task
/// (task-locals are not inherited by spawned tasks otherwise).
pub fn spawn_traced<F>(fut: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(OUTBOUND_TRACE.scope(current_outbound_trace(), fut))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let t = RequestTrace::new();
        assert_eq!(t.hop, 1);
        assert!(!t.is_duplicate());
        let parsed = RequestTrace::parse(&t.header_value()).unwrap();
        assert_eq!(parsed, t);
    }

    #[test]
    fn next_hop_keeps_id() {
        let t = RequestTrace::parse("abc-123;hop=1").unwrap();
        let n = t.next_hop();
        assert_eq!(n.trace_id, "abc-123");
        assert_eq!(n.hop, 2);
        assert!(n.is_duplicate());
        assert_eq!(n.header_value(), "abc-123;hop=2");
    }

    #[test]
    fn parse_tolerates_whitespace() {
        let t = RequestTrace::parse(" abc ; hop=3 ").unwrap();
        assert_eq!(t.hop, 3);
    }

    #[test]
    fn malformed_is_none() {
        assert!(RequestTrace::parse("").is_none());
        assert!(RequestTrace::parse("abc").is_none());
        assert!(RequestTrace::parse("abc;hop=0").is_none());
        assert!(RequestTrace::parse("abc;hop=x").is_none());
        assert!(RequestTrace::parse(";hop=1").is_none());
        assert!(RequestTrace::parse("a b;hop=1").is_none());
        assert!(RequestTrace::parse("é;hop=1").is_none());
    }

    #[tokio::test]
    async fn task_local_scope_and_spawn() {
        assert!(current_outbound_trace().is_none());
        assert!(!is_duplicate_hop());
        let t = RequestTrace::parse("abc;hop=2").unwrap();
        with_outbound_trace(Some(t.clone()), async {
            assert_eq!(current_outbound_trace(), Some(t.clone()));
            assert!(is_duplicate_hop());
            // Plain spawn loses the scope; spawn_traced keeps it.
            assert!(!tokio::spawn(async { is_duplicate_hop() }).await.unwrap());
            assert!(spawn_traced(async { is_duplicate_hop() }).await.unwrap());
        })
        .await;
        assert!(!is_duplicate_hop());
    }

    #[test]
    fn outbound_for_inbound() {
        assert_eq!(RequestTrace::outbound_for(None).hop, 1);
        let inbound = RequestTrace::parse("x;hop=2").unwrap();
        let out = RequestTrace::outbound_for(Some(&inbound));
        assert_eq!((out.trace_id.as_str(), out.hop), ("x", 3));
    }
}
