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
//!
//! Learned hosts are kept in a small state file (when one is given) so a
//! restart does not fail the first transformed request to such a host again.
//! The file holds at most [`MAX_LEARNED`] hosts, each forgotten
//! [`LEARNED_TTL`] after it was learned (the upstream may have changed). It
//! holds host names only, never request content. Configured hosts are
//! replaced whenever the configuration changes ([`LengthRequiredHosts::set_configured`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Most learned hosts remembered; the oldest are forgotten first.
pub const MAX_LEARNED: usize = 256;
/// How long a learned host is remembered.
pub const LEARNED_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Hosts that need a `Content-Length`, configured and learned.
#[derive(Debug, Default)]
pub struct LengthRequiredHosts {
    configured: parking_lot::RwLock<HashSet<String>>,
    /// Learned host → when it was learned (seconds since the Unix epoch).
    learned: parking_lot::RwLock<HashMap<String, u64>>,
    /// Where learned hosts persist.
    state: Option<PathBuf>,
}

/// The state file's content.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct StateFile {
    #[serde(default)]
    learned: BTreeMap<String, u64>,
}

fn normalize(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Learned too long ago, or (a clock that went back) in the future.
fn expired(learned_at: u64, now: u64) -> bool {
    now.saturating_sub(learned_at) >= LEARNED_TTL.as_secs()
        || learned_at > now.saturating_add(24 * 60 * 60)
}

/// Keep the newest [`MAX_LEARNED`] unexpired hosts.
fn prune(learned: &mut HashMap<String, u64>, now: u64) {
    learned.retain(|host, at| !host.is_empty() && !expired(*at, now));
    if learned.len() > MAX_LEARNED {
        let mut by_age: Vec<(u64, String)> =
            learned.iter().map(|(h, at)| (*at, h.clone())).collect();
        by_age.sort();
        let excess = learned.len() - MAX_LEARNED;
        for (_, host) in by_age.into_iter().take(excess) {
            learned.remove(&host);
        }
    }
}

impl LengthRequiredHosts {
    pub fn new(configured: impl IntoIterator<Item = String>) -> Self {
        let hosts = Self::default();
        hosts.set_configured(configured);
        hosts
    }

    /// Hosts whose learned part is kept in `state` (read now, written when a
    /// host is learned). An unreadable or invalid file is ignored.
    pub fn persistent(configured: impl IntoIterator<Item = String>, state: PathBuf) -> Self {
        let mut hosts = Self::new(configured);
        let mut learned: HashMap<String, u64> = std::fs::read(&state)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<StateFile>(&bytes).ok())
            .map(|file| {
                file.learned
                    .into_iter()
                    .map(|(host, at)| (normalize(&host), at))
                    .collect()
            })
            .unwrap_or_default();
        prune(&mut learned, now());
        *hosts.learned.write() = learned;
        hosts.state = Some(state);
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
        self.requires_at(host, now())
    }

    fn requires_at(&self, host: &str, now: u64) -> bool {
        let host = normalize(host);
        self.configured.read().contains(&host)
            || self
                .learned
                .read()
                .get(&host)
                .is_some_and(|at| !expired(*at, now))
    }

    /// Remember that `host` refused a chunked request body. Returns true
    /// when this is new (or renews an expired entry).
    pub fn learn(&self, host: &str) -> bool {
        self.learn_at(host, now())
    }

    fn learn_at(&self, host: &str, now: u64) -> bool {
        let host = normalize(host);
        if host.is_empty() {
            return false;
        }
        let snapshot = {
            let mut learned = self.learned.write();
            if learned.get(&host).is_some_and(|at| !expired(*at, now)) {
                return false;
            }
            learned.insert(host.clone(), now);
            prune(&mut learned, now);
            learned.clone()
        };
        tracing::warn!(
            "proxy: {host} refused a chunked request body; later transformed requests to it are sent with a Content-Length"
        );
        self.save(snapshot);
        true
    }

    /// Write the learned hosts to the state file (a temporary file renamed
    /// over it). A failure is logged; the hosts stay learned for this
    /// process.
    fn save(&self, learned: HashMap<String, u64>) {
        let Some(path) = &self.state else {
            return;
        };
        let file = StateFile {
            learned: learned.into_iter().collect(),
        };
        let written = (|| -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let bytes = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, bytes)?;
            std::fs::rename(&tmp, path)
        })();
        if let Err(error) = written {
            tracing::warn!(
                "proxy: could not save learned length-required hosts to {}: {error}",
                path.display()
            );
        }
    }

    /// Hosts learned and still remembered.
    pub fn learned(&self) -> Vec<String> {
        let now = now();
        let mut hosts: Vec<_> = self
            .learned
            .read()
            .iter()
            .filter(|(_, at)| !expired(**at, now))
            .map(|(host, _)| host.clone())
            .collect();
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
    fn learned_hosts_persist_bounded_and_expire() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proxy").join("length_required_hosts.json");
        let hosts = LengthRequiredHosts::persistent(Vec::new(), path.clone());
        assert!(hosts.learned().is_empty(), "no file yet");
        assert!(hosts.learn("API.Example.com."));
        assert!(!hosts.learn("api.example.com"));
        // A new process reads it back; configured hosts are not stored.
        let restarted =
            LengthRequiredHosts::persistent(["cfg.example.com".to_string()], path.clone());
        assert!(restarted.requires("api.example.com"));
        assert!(restarted.requires("cfg.example.com"));
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("api.example.com"));
        assert!(!saved.contains("cfg.example.com"));
        assert!(!path.with_extension("json.tmp").exists());

        // Expiry: a host learned longer ago than the TTL no longer counts.
        let now = now();
        let old = now - LEARNED_TTL.as_secs() - 1;
        restarted
            .learned
            .write()
            .insert("old.example.com".into(), old);
        assert!(!restarted.requires_at("old.example.com", now));
        assert!(restarted.requires_at("old.example.com", old + 10));
        // Learning it again renews it.
        assert!(restarted.learn_at("old.example.com", now));
        assert!(restarted.requires("old.example.com"));
        // Expired entries are dropped when the file is read.
        std::fs::write(
            &path,
            format!(r#"{{"learned":{{"stale.example.com":{old},"fresh.example.com":{now}}}}}"#),
        )
        .unwrap();
        let reloaded = LengthRequiredHosts::persistent(Vec::new(), path.clone());
        assert_eq!(reloaded.learned(), vec!["fresh.example.com".to_string()]);

        // Bounded: the oldest are forgotten first.
        for i in 0..(MAX_LEARNED + 10) {
            reloaded.learn_at(&format!("h{i}.example.com"), now + 1 + i as u64);
        }
        let file: StateFile = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(file.learned.len(), MAX_LEARNED);
        assert!(!file.learned.contains_key("fresh.example.com"));
        assert!(file
            .learned
            .contains_key(&format!("h{}.example.com", MAX_LEARNED + 9)));

        // A corrupt file is ignored.
        std::fs::write(&path, b"not json").unwrap();
        assert!(LengthRequiredHosts::persistent(Vec::new(), path)
            .learned()
            .is_empty());
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
