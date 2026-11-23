//! Which upstreams must receive a transformed request body with a
//! `Content-Length` instead of chunked framing.
//!
//! A streamed, rewritten request body has no known length while it uploads,
//! so HTTP/1.1 frames it as chunked. Whether every authenticated upstream
//! accepts that is not proven. For a host that requires a length, the
//! transport holds the transformed body (in RAM, then encrypted on disk) and
//! sends it with a `Content-Length` once it ended. A host requires a length
//! when the configuration says so, or once it refused a chunked request: an
//! upstream cannot be sent the same streamed body twice (it was never kept),
//! so the refused request fails and every later one to that host is held.

use std::collections::HashSet;

/// Hosts that need a `Content-Length`, configured and learned.
#[derive(Debug, Default)]
pub struct LengthRequiredHosts {
    configured: parking_lot::RwLock<HashSet<String>>,
    learned: parking_lot::RwLock<HashSet<String>>,
}

fn normalize(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

impl LengthRequiredHosts {
    pub fn new(configured: impl IntoIterator<Item = String>) -> Self {
        let hosts = Self::default();
        hosts.set_configured(configured);
        hosts
    }

    /// Replace the configured hosts (the learned ones stay).
    pub fn set_configured(&self, hosts: impl IntoIterator<Item = String>) {
        *self.configured.write() = hosts
            .into_iter()
            .map(|h| normalize(&h))
            .filter(|h| !h.is_empty())
            .collect();
    }

    /// Whether a transformed body for `host` must be sent with a length.
    pub fn requires(&self, host: &str) -> bool {
        let host = normalize(host);
        self.configured.read().contains(&host) || self.learned.read().contains(&host)
    }

    /// Remember that `host` refused a chunked request body. Returns true
    /// when this is new.
    pub fn learn(&self, host: &str) -> bool {
        let host = normalize(host);
        let new = self.learned.write().insert(host.clone());
        if new {
            tracing::warn!(
                "proxy: {host} refused a chunked request body; later transformed requests to it are sent with a Content-Length"
            );
        }
        new
    }

    /// Hosts learned during this process.
    pub fn learned(&self) -> Vec<String> {
        let mut hosts: Vec<_> = self.learned.read().iter().cloned().collect();
        hosts.sort();
        hosts
    }
}

/// Whether an upstream answer to a chunked request says that it needs a
/// length: `411 Length Required`, or a `400` whose structured JSON error
/// carries the `length_required` code or type. Free text is never matched.
pub fn refuses_chunked(status: u16, body: &[u8]) -> bool {
    if status == 411 {
        return true;
    }
    if status != 400 || body.is_empty() || body.len() > 64 * 1024 {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return false;
    };
    let error = value.get("error").unwrap_or(&value);
    ["code", "type"]
        .iter()
        .any(|field| error.get(*field).and_then(|v| v.as_str()) == Some("length_required"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_and_learned_hosts_require_a_length() {
        let hosts = LengthRequiredHosts::new(["API.Example.com.".to_string(), " ".into()]);
        assert!(hosts.requires("api.example.com"));
        assert!(!hosts.requires("other.example.com"));
        assert!(hosts.learn("Other.Example.com"));
        assert!(!hosts.learn("other.example.com"));
        assert!(hosts.requires("other.example.com"));
        hosts.set_configured(Vec::new());
        assert!(!hosts.requires("api.example.com"));
        assert!(hosts.requires("other.example.com"), "learned hosts stay");
        assert_eq!(hosts.learned(), vec!["other.example.com".to_string()]);
    }

    #[test]
    fn only_structured_length_errors_count() {
        assert!(refuses_chunked(411, b""));
        assert!(refuses_chunked(
            400,
            br#"{"error":{"code":"length_required","message":"x"}}"#
        ));
        assert!(refuses_chunked(400, br#"{"type":"length_required"}"#));
        // Free text mentioning the length is not a structured signal.
        assert!(!refuses_chunked(
            400,
            br#"{"error":{"message":"Content-Length required"}}"#
        ));
        assert!(!refuses_chunked(400, b"length_required"));
        assert!(!refuses_chunked(
            401,
            br#"{"error":{"code":"length_required"}}"#
        ));
        assert!(!refuses_chunked(200, b""));
    }
}
