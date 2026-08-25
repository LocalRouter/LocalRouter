//! HTTPS inspection proxy for LocalRouter.
//!
//! Lets tools that honor `HTTPS_PROXY` (e.g. Claude Code, Codex) route their
//! LLM traffic through LocalRouter, which terminates TLS with a trusted root
//! CA, **passively inspects** the request/response for the monitor, and
//! forwards the bytes unchanged to the real upstream. The client's own
//! credentials flow straight through — LocalRouter neither re-issues nor
//! stores them. Both HTTP (plain + SSE) and WebSocket transports are
//! inspected; see [`websocket`].
//!
//! Only allow-listed LLM API hosts are intercepted; everything else (auth,
//! telemetry, arbitrary HTTPS) is blind-tunneled without decryption.
//!
//! `HTTPS_PROXY` is process-wide, so a tool pointed here also sends its git,
//! package-manager and update traffic through the proxy. All of it — including
//! absolute-form plain HTTP — is forwarded verbatim, and recorded as a
//! *passthrough* monitor event carrying the destination and byte volume only,
//! so accidental egress is visible without ever capturing its content.
//!
//! ## Module map
//! - [`cert`] — root CA + on-demand leaf certificate minting.
//! - [`wire`] — wire-format dispatch + shared SSE event parsing.
//! - [`anthropic`] / [`openai`] — per-format request/response parsing.
//! - [`interceptor`] — the observe/rewrite seam ([`interceptor::ProxyInterceptor`]).
//! - [`passive`] — the v1 inspect-only interceptor that records to the monitor.
//! - [`websocket`] — frame codec + message-aware relay for upgraded
//!   connections (Codex's Responses-over-websocket transport).
//! - [`reverse`] — the *reverse* proxy: bind a local provider's original port
//!   and forward to the relocated provider, teeing traffic to the monitor.
//!
//! The live MITM data-path (CONNECT handling, TLS terminate/re-originate,
//! streaming tap) and the [`ProxyManager`] lifecycle build on these pieces.

pub mod active;
pub mod anthropic;
pub mod cert;
pub mod error;
pub mod interceptor;
pub mod manager;
pub mod ollama;
pub mod openai;
pub mod passive;
pub mod resolver;
pub mod reverse;
pub mod tap;
pub mod tls;
pub mod transport;
pub mod websocket;
pub mod wire;

pub use error::ProxyError;
pub use manager::ProxyManager;

/// Hosts the proxy will MITM (decrypt + inspect). Everything else is tunneled
/// blindly. Kept deliberately narrow: only LLM API endpoints belong here, never
/// auth/identity hosts such as `claude.ai`.
pub const MITM_HOST_ALLOWLIST: &[&str] = &["api.anthropic.com", "api.openai.com", "chatgpt.com"];

/// Whether `host` (no port) should be intercepted rather than blind-tunneled.
///
/// Matches the allow-list exactly or as a dotted suffix (so
/// `foo.api.anthropic.com` also matches `api.anthropic.com`).
/// Read the cross-hop trace an earlier LocalRouter hop may have stamped on
/// `headers`, and replace it with the trace for the next hop (or a fresh one).
/// Returns the outbound trace. With `enabled == false` the headers are left
/// untouched and `None` is returned, so the request is neither recognized as
/// a duplicate nor marked for downstream hops.
pub fn stamp_trace(
    headers: &mut hyper::HeaderMap,
    enabled: bool,
) -> Option<lr_types::RequestTrace> {
    if !enabled {
        return None;
    }
    let inbound = headers
        .get(lr_types::TRACE_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(lr_types::RequestTrace::parse);
    let outbound = lr_types::RequestTrace::outbound_for(inbound.as_ref());
    if let Ok(value) = hyper::header::HeaderValue::from_str(&outbound.header_value()) {
        headers.insert(
            hyper::header::HeaderName::from_static(lr_types::TRACE_HEADER),
            value,
        );
    }
    Some(outbound)
}

pub fn should_mitm_host(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    MITM_HOST_ALLOWLIST
        .iter()
        .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlists_only_llm_api_hosts() {
        assert!(should_mitm_host("api.anthropic.com"));
        assert!(should_mitm_host("API.Anthropic.com")); // case-insensitive
        assert!(should_mitm_host("edge.api.anthropic.com")); // subdomain
                                                             // Auth / unrelated hosts must NOT be intercepted.
        assert!(!should_mitm_host("claude.ai"));
        assert!(!should_mitm_host("statsig.anthropic.com"));
        assert!(!should_mitm_host("example.com"));
        // Guard against naive substring matching.
        assert!(!should_mitm_host("api.anthropic.com.evil.com"));
    }

    #[test]
    fn stamp_trace_marks_fresh_and_duplicate_requests() {
        use lr_types::{RequestTrace, TRACE_HEADER};
        let mut h = hyper::HeaderMap::new();
        let t = stamp_trace(&mut h, true).unwrap();
        assert_eq!(t.hop, 1);
        assert_eq!(
            h.get(TRACE_HEADER).unwrap().to_str().unwrap(),
            t.header_value()
        );

        let mut h = hyper::HeaderMap::new();
        h.insert(TRACE_HEADER, "abc;hop=1".parse().unwrap());
        let t = stamp_trace(&mut h, true).unwrap();
        assert_eq!((t.trace_id.as_str(), t.hop), ("abc", 2));
        assert!(t.is_duplicate());
        assert_eq!(h.get(TRACE_HEADER).unwrap(), "abc;hop=2");
        assert_eq!(RequestTrace::parse("abc;hop=2").unwrap(), t);

        // Disabled: header untouched, nothing recognized.
        let mut h = hyper::HeaderMap::new();
        h.insert(TRACE_HEADER, "abc;hop=1".parse().unwrap());
        assert!(stamp_trace(&mut h, false).is_none());
        assert_eq!(h.get(TRACE_HEADER).unwrap(), "abc;hop=1");
        let mut h = hyper::HeaderMap::new();
        assert!(stamp_trace(&mut h, false).is_none());
        assert!(h.get(TRACE_HEADER).is_none());
    }
}
