//! HTTPS inspection proxy for LocalRouter.
//!
//! Lets tools that honor `HTTPS_PROXY` (e.g. Claude Code) route their LLM
//! traffic through LocalRouter, which terminates TLS with a trusted root CA,
//! **passively inspects** the request/response for the monitor, and forwards
//! the bytes unchanged to the real upstream. The client's own credentials flow
//! straight through — LocalRouter neither re-issues nor stores them.
//!
//! Only allow-listed LLM API hosts are intercepted; everything else (auth,
//! telemetry, arbitrary HTTPS) is blind-tunneled without decryption.
//!
//! ## Module map
//! - [`cert`] — root CA + on-demand leaf certificate minting.
//! - [`anthropic`] — parse the Anthropic Messages wire format for monitoring.
//! - [`interceptor`] — the observe/rewrite seam ([`interceptor::ProxyInterceptor`]).
//! - [`passive`] — the v1 inspect-only interceptor that records to the monitor.
//!
//! The live MITM data-path (CONNECT handling, TLS terminate/re-originate,
//! streaming tap) and the [`ProxyManager`] lifecycle build on these pieces.

pub mod active;
pub mod anthropic;
pub mod cert;
pub mod error;
pub mod interceptor;
pub mod manager;
pub mod passive;
pub mod resolver;
pub mod tap;
pub mod tls;
pub mod transport;

pub use error::ProxyError;
pub use manager::ProxyManager;

/// Hosts the proxy will MITM (decrypt + inspect). Everything else is tunneled
/// blindly. Kept deliberately narrow: only LLM API endpoints belong here, never
/// auth/identity hosts such as `claude.ai`.
pub const MITM_HOST_ALLOWLIST: &[&str] = &["api.anthropic.com"];

/// Whether `host` (no port) should be intercepted rather than blind-tunneled.
///
/// Matches the allow-list exactly or as a dotted suffix (so
/// `foo.api.anthropic.com` also matches `api.anthropic.com`).
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
}
