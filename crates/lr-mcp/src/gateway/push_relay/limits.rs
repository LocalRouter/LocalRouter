//! Push Relay's internal limits.
//!
//! None of these are user configuration. They bound memory and runaway
//! loops, and are deliberately generous so they never ration normal use.
//! [`PushRelayLimits::default`] is the one place their values are defined.
//! Tests shrink them through `PushRelay::set_limits` to reach a bound
//! quickly.

use std::time::Duration;

const MIB: u64 = 1024 * 1024;

/// How long `initialize` / `server/discover` waits for a visible server that
/// has no current capability snapshot.
pub const FIRST_DISCOVERY_WAIT: Duration = Duration::from_secs(10);

/// Every Push Relay bound. Durations are whole seconds (`_secs`) or
/// milliseconds (`_ms`); use the accessor methods for a [`Duration`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushRelayLimits {
    /// Occurrences held per client for polling and replay.
    pub journal_max_events: usize,
    /// Same, by total size of event data per client (first limit wins).
    pub journal_max_bytes: u64,
    /// Age after which journal entries are dropped.
    pub journal_retention_secs: u64,
    /// Largest accepted `data` of one occurrence; larger events are dropped.
    pub max_event_bytes: u64,
    /// Held (Ask) events per client awaiting a decision. A held event waits
    /// for its decision indefinitely; this only stops the queue growing
    /// without bound. Further Ask events are denied with a Monitor record.
    pub ask_hold_cap: usize,
    /// Lower clamp on an upstream's `nextPollMs`.
    pub upstream_poll_min_interval_ms: u64,
    /// Upper clamp on an upstream's `nextPollMs`.
    pub upstream_poll_max_interval_ms: u64,
    /// First delay before reconnecting a dropped upstream `events/stream`.
    pub upstream_reconnect_backoff_min_ms: u64,
    /// Largest reconnect delay (the backoff is jittered between the bounds).
    pub upstream_reconnect_backoff_max_ms: u64,
    /// An upstream stream with no heartbeat for this long is treated as dead.
    pub upstream_heartbeat_timeout_secs: u64,
    /// Interval of `notifications/events/heartbeat` sent to clients.
    pub downstream_heartbeat_secs: u64,
    /// Concurrent `events/stream` requests per client session.
    pub max_streams_per_session: usize,
    /// Ceiling on events per `events/poll` page.
    pub poll_max_events: usize,
    /// Page size of `events/poll` when the caller gives no limit.
    pub poll_default_events: usize,
    /// Ceiling on `EventPoll`'s `wait_seconds`.
    pub poll_max_wait_secs: u64,
    /// Lifetime of one `/events/watch` connection.
    pub watch_max_lifetime_secs: u64,
    /// Rendered `content` per channel message; longer content is truncated.
    pub channel_max_content_bytes: usize,
    /// Channel messages per minute per session before coalescing.
    pub channel_rate_per_minute: u32,
    /// Timeout of one webhook delivery attempt.
    pub webhook_timeout_secs: u64,
    /// Delay before each retry of a failed webhook delivery. The first attempt
    /// plus one retry per entry is the total number of attempts; after the
    /// last the subscription is suspended until refreshed.
    pub webhook_retry_delays_secs: Vec<u64>,
    /// Active webhook subscriptions per client.
    pub webhook_max_subscriptions: usize,
    /// Webhook subscription TTL when the client asks for none.
    pub webhook_lease_default_secs: u64,
    /// Shortest webhook subscription TTL accepted.
    pub webhook_lease_min_secs: u64,
    /// Longest webhook subscription TTL accepted.
    pub webhook_lease_max_secs: u64,
}

impl Default for PushRelayLimits {
    fn default() -> Self {
        Self {
            journal_max_events: 10_000,
            journal_max_bytes: 64 * MIB,
            journal_retention_secs: 24 * 60 * 60,
            max_event_bytes: MIB,
            ask_hold_cap: 1_000,
            upstream_poll_min_interval_ms: 1_000,
            upstream_poll_max_interval_ms: 5 * 60 * 1_000,
            upstream_reconnect_backoff_min_ms: 1_000,
            upstream_reconnect_backoff_max_ms: 60_000,
            upstream_heartbeat_timeout_secs: 90,
            downstream_heartbeat_secs: 30,
            max_streams_per_session: 64,
            poll_max_events: 500,
            poll_default_events: 50,
            poll_max_wait_secs: 50,
            watch_max_lifetime_secs: 24 * 60 * 60,
            channel_max_content_bytes: 16 * 1024,
            channel_rate_per_minute: 60,
            webhook_timeout_secs: 10,
            webhook_retry_delays_secs: vec![5, 30, 120, 600, 1_800],
            webhook_max_subscriptions: 100,
            webhook_lease_default_secs: 60 * 60,
            webhook_lease_min_secs: 60,
            webhook_lease_max_secs: 24 * 60 * 60,
        }
    }
}

impl PushRelayLimits {
    pub fn journal_retention(&self) -> Duration {
        Duration::from_secs(self.journal_retention_secs)
    }

    pub fn upstream_reconnect_backoff_min(&self) -> Duration {
        Duration::from_millis(self.upstream_reconnect_backoff_min_ms)
    }

    pub fn upstream_reconnect_backoff_max(&self) -> Duration {
        Duration::from_millis(self.upstream_reconnect_backoff_max_ms)
    }

    pub fn upstream_heartbeat_timeout(&self) -> Duration {
        Duration::from_secs(self.upstream_heartbeat_timeout_secs)
    }

    pub fn downstream_heartbeat(&self) -> Duration {
        Duration::from_secs(self.downstream_heartbeat_secs)
    }

    pub fn poll_max_wait(&self) -> Duration {
        Duration::from_secs(self.poll_max_wait_secs)
    }

    pub fn watch_max_lifetime(&self) -> Duration {
        Duration::from_secs(self.watch_max_lifetime_secs)
    }

    pub fn webhook_timeout(&self) -> Duration {
        Duration::from_secs(self.webhook_timeout_secs)
    }

    /// Total delivery attempts per webhook event (first try plus retries).
    pub fn webhook_attempts(&self) -> usize {
        self.webhook_retry_delays_secs.len() + 1
    }

    /// Clamp a requested webhook lease to the bounds.
    pub fn webhook_lease(&self, requested_secs: Option<u64>) -> Duration {
        let secs = requested_secs
            .unwrap_or(self.webhook_lease_default_secs)
            .clamp(self.webhook_lease_min_secs, self.webhook_lease_max_secs);
        Duration::from_secs(secs)
    }

    /// Clamp an upstream's `nextPollMs` to the bounds.
    pub fn clamp_upstream_poll_interval(&self, next_poll_ms: u64) -> Duration {
        Duration::from_millis(next_poll_ms.clamp(
            self.upstream_poll_min_interval_ms,
            self.upstream_poll_max_interval_ms,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_clamp() {
        let limits = PushRelayLimits::default();
        assert_eq!(limits.webhook_lease(None), Duration::from_secs(3600));
        assert_eq!(limits.webhook_lease(Some(1)), Duration::from_secs(60));
        assert_eq!(
            limits.webhook_lease(Some(10_000_000)),
            Duration::from_secs(86_400)
        );
        assert_eq!(
            limits.clamp_upstream_poll_interval(10),
            Duration::from_secs(1)
        );
        assert_eq!(
            limits.clamp_upstream_poll_interval(u64::MAX),
            Duration::from_secs(300)
        );
        assert_eq!(limits.webhook_attempts(), 6);
        assert_eq!(limits.ask_hold_cap, 1_000);
    }
}
