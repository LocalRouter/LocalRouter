//! Gateway-wide bookkeeping that outlives a single session lock:
//!
//! - [`SessionHolds`]: open streams (legacy SSE / Streamable HTTP GET,
//!   `events/stream`, `subscriptions/listen`) keep their session alive.
//! - [`CacheEpochs`]: list-cache invalidation that never waits on (or
//!   skips) a busy session.
//! - [`RetiredCapabilities`]: a session that expired in the background
//!   leaves its client's capabilities for a later session under its key.

use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::types::ClientCapabilities;

#[derive(Default)]
struct HoldState {
    count: usize,
    released_at: Option<Instant>,
}

/// Open streams per session key. A held session never expires; its idle
/// time restarts when the last hold is released.
#[derive(Default)]
pub(crate) struct SessionHolds {
    holds: DashMap<String, HoldState>,
}

/// Keeps a session alive while it lives (see [`SessionHolds`]).
#[must_use = "the session is kept alive only while the hold lives"]
pub struct SessionHold {
    holds: Arc<SessionHolds>,
    key: String,
}

impl std::fmt::Debug for SessionHold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHold")
            .field("key", &self.key)
            .finish()
    }
}

impl SessionHolds {
    pub(crate) fn hold(self: &Arc<Self>, key: &str) -> SessionHold {
        self.holds.entry(key.to_string()).or_default().count += 1;
        SessionHold {
            holds: self.clone(),
            key: key.to_string(),
        }
    }

    /// Whether streams keep `key` alive: one is open, or the last one
    /// closed less than `ttl` ago.
    pub(crate) fn keeps_alive(&self, key: &str, ttl: Duration) -> bool {
        self.holds.get(key).is_some_and(|state| {
            state.count > 0 || state.released_at.is_some_and(|at| at.elapsed() <= ttl)
        })
    }

    /// Drop released entries whose session is gone.
    pub(crate) fn prune(&self, live: impl Fn(&str) -> bool) {
        self.holds.retain(|key, state| state.count > 0 || live(key));
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.holds.len()
    }
}

impl Drop for SessionHold {
    fn drop(&mut self) {
        if let Some(mut state) = self.holds.holds.get_mut(&self.key) {
            state.count = state.count.saturating_sub(1);
            if state.count == 0 {
                state.released_at = Some(Instant::now());
            }
        }
    }
}

/// Which cached lists an invalidation affects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheEpoch {
    pub tools: u64,
    pub resources: u64,
    pub prompts: u64,
}

/// Invalidation counters per client. Invalidating bumps a counter without
/// touching any session; each request compares its session's last seen
/// counters (under the session write lock it already holds) and drops the
/// caches that went stale, before any list is served.
#[derive(Default)]
pub(crate) struct CacheEpochs {
    epochs: DashMap<String, CacheEpoch>,
}

impl CacheEpochs {
    pub(crate) fn bump(&self, client_id: &str, tools: bool, resources: bool, prompts: bool) {
        let mut epoch = self.epochs.entry(client_id.to_string()).or_default();
        epoch.tools += u64::from(tools);
        epoch.resources += u64::from(resources);
        epoch.prompts += u64::from(prompts);
    }

    pub(crate) fn current(&self, client_id: &str) -> CacheEpoch {
        self.epochs
            .get(client_id)
            .map(|e| *e.value())
            .unwrap_or_default()
    }
}

/// A client's capabilities left by a session that expired in the
/// background, for the session next created under the same key.
pub(crate) struct RetiredCapabilities {
    entries: DashMap<String, (ClientCapabilities, Option<serde_json::Value>, Instant)>,
    keep: Duration,
}

impl Default for RetiredCapabilities {
    fn default() -> Self {
        Self {
            entries: DashMap::new(),
            keep: Duration::from_secs(24 * 60 * 60),
        }
    }
}

impl RetiredCapabilities {
    pub(crate) fn retire(
        &self,
        key: &str,
        capabilities: Option<ClientCapabilities>,
        raw: Option<serde_json::Value>,
    ) {
        if let Some(capabilities) = capabilities {
            self.entries
                .insert(key.to_string(), (capabilities, raw, Instant::now()));
        }
    }

    pub(crate) fn take(
        &self,
        key: &str,
    ) -> Option<(ClientCapabilities, Option<serde_json::Value>)> {
        self.entries
            .remove(key)
            .filter(|(_, (_, _, at))| at.elapsed() <= self.keep)
            .map(|(_, (caps, raw, _))| (caps, raw))
    }

    pub(crate) fn forget(&self, key: &str) {
        self.entries.remove(key);
    }

    pub(crate) fn prune(&self) {
        let keep = self.keep;
        self.entries.retain(|_, (_, _, at)| at.elapsed() <= keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_keeps_its_session_alive_and_restarts_idle_time_on_release() {
        let holds = Arc::new(SessionHolds::default());
        assert!(!holds.keeps_alive("s", Duration::ZERO));
        let first = holds.hold("s");
        let second = holds.hold("s");
        assert!(holds.keeps_alive("s", Duration::ZERO));
        drop(first);
        assert!(
            holds.keeps_alive("s", Duration::ZERO),
            "one stream still open"
        );
        drop(second);
        assert!(
            holds.keeps_alive("s", Duration::from_secs(60)),
            "idle time restarts"
        );
        std::thread::sleep(Duration::from_millis(5));
        assert!(!holds.keeps_alive("s", Duration::from_millis(1)));
        assert!(!holds.keeps_alive("other", Duration::from_secs(60)));
        holds.prune(|_| false);
        assert_eq!(holds.len(), 0, "released holds of gone sessions are pruned");
    }

    #[test]
    fn prune_keeps_open_holds() {
        let holds = Arc::new(SessionHolds::default());
        let _open = holds.hold("s");
        holds.prune(|_| false);
        assert!(holds.keeps_alive("s", Duration::ZERO));
    }

    #[test]
    fn epochs_count_per_client_and_list() {
        let epochs = CacheEpochs::default();
        epochs.bump("a", true, false, false);
        epochs.bump("a", true, true, true);
        assert_eq!(
            epochs.current("a"),
            CacheEpoch {
                tools: 2,
                resources: 1,
                prompts: 1
            }
        );
        assert_eq!(epochs.current("b"), CacheEpoch::default());
    }

    #[test]
    fn retired_capabilities_are_taken_once() {
        let retired = RetiredCapabilities::default();
        retired.retire("k", None, None);
        assert!(retired.take("k").is_none(), "nothing to keep");
        retired.retire("k", Some(ClientCapabilities::default()), None);
        assert!(retired.take("k").is_some());
        assert!(retired.take("k").is_none());
    }
}
