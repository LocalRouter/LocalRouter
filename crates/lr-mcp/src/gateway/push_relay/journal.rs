//! Normalised occurrences and the per-client journal they are delivered to.
//!
//! Every occurrence a client may read (after permissions and Ask) is
//! appended to that client's journal with a sequence number. Every delivery
//! mode reads from it from its own position: `events/poll`, `events/stream`,
//! the fallback tools, `/events/watch`, Claude Channels and webhooks. The
//! journal is bounded by count, total size and age; a reader whose position
//! fell behind what was dropped is told `truncated`.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use tokio::sync::watch;

/// How an occurrence reached LocalRouter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IngressMode {
    /// Upstream `events/stream`.
    Push,
    /// Upstream `events/poll`.
    Poll,
    /// Upstream `notifications/claude/channel`.
    Channel,
    /// The built-in test source (`send_push_test_event`), not an upstream.
    Test,
}

impl IngressMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::Poll => "poll",
            Self::Channel => "channel",
            Self::Test => "test",
        }
    }
}

/// One event an upstream emitted, normalised.
#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    /// Namespaced like the name: `<server>__<upstream event id>`.
    pub event_id: String,
    /// Server-qualified name, `<server>__<upstream name>`.
    pub name: String,
    pub server_id: String,
    /// The server's slug (the prefix of `name`).
    pub server_slug: String,
    /// The upstream's own event name.
    pub upstream_name: String,
    pub timestamp: DateTime<Utc>,
    pub data: Value,
    pub upstream_cursor: Option<String>,
    pub ingress: IngressMode,
    /// Canonical arguments of the upstream subscription it came from.
    pub args_key: String,
    /// The downstream session whose own upstream connection produced it (a
    /// stdio subscription or a channel message on that session's process);
    /// `None` for a relay-owned connection shared by every consumer.
    pub origin_session: Option<String>,
}

impl Occurrence {
    /// Size of `data` on the wire.
    pub fn data_bytes(data: &Value) -> usize {
        serde_json::to_vec(data).map(|v| v.len()).unwrap_or(0)
    }
}

/// An occurrence as one client receives it.
#[derive(Debug, Clone)]
pub struct JournalEntry {
    pub seq: u64,
    pub occurrence: Arc<Occurrence>,
    /// Delivered data: the original, or the user's edit.
    pub data: Value,
    bytes: u64,
    appended_at: Instant,
}

impl JournalEntry {
    /// An entry for an occurrence that is not (yet) in any journal, to
    /// render it for an approval.
    pub fn preview(occurrence: Arc<Occurrence>) -> Self {
        Self {
            seq: 0,
            data: occurrence.data.clone(),
            occurrence,
            bytes: 0,
            appended_at: Instant::now(),
        }
    }

    /// The SEP-3415 occurrence object.
    pub fn wire(&self) -> Value {
        json!({
            "eventId": self.occurrence.event_id,
            "name": self.occurrence.name,
            "timestamp": self.occurrence.timestamp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "data": self.data,
        })
    }
}

/// Which entries a reader wants.
#[derive(Debug, Clone, Default)]
pub struct ReadScope {
    /// Server-qualified names; `None` reads every name.
    pub names: Option<HashSet<String>>,
    /// Canonical subscription arguments; `None` reads any. Channel
    /// occurrences carry no arguments and match every scope.
    pub args_key: Option<String>,
}

impl ReadScope {
    pub fn matches(&self, entry: &JournalEntry) -> bool {
        let occurrence = &entry.occurrence;
        self.names
            .as_ref()
            .is_none_or(|names| names.contains(&occurrence.name))
            && (occurrence.ingress == IngressMode::Channel
                || self
                    .args_key
                    .as_ref()
                    .is_none_or(|key| key == &occurrence.args_key))
    }
}

/// The result of one read.
#[derive(Debug, Clone)]
pub struct ReadBatch {
    pub entries: Vec<JournalEntry>,
    /// Position after this read (a cursor for the next one).
    pub position: u64,
    /// Where the read started after any truncation.
    pub start: u64,
    /// Entries the reader had not seen were dropped.
    pub truncated: bool,
    /// More matching entries are waiting past `position`.
    pub has_more: bool,
}

/// Bounds of one journal.
#[derive(Debug, Clone, Copy)]
pub struct JournalLimits {
    pub max_events: usize,
    pub max_bytes: u64,
    pub retention: Duration,
}

impl JournalLimits {
    pub fn from_limits(limits: &super::limits::PushRelayLimits) -> Self {
        Self {
            max_events: limits.journal_max_events.max(1),
            max_bytes: limits.journal_max_bytes.max(1),
            retention: limits.journal_retention(),
        }
    }
}

/// One client's journal.
pub struct ClientJournal {
    entries: VecDeque<JournalEntry>,
    next_seq: u64,
    bytes: u64,
    /// Highest sequence number dropped (0 = none).
    dropped_through: u64,
    changed: watch::Sender<u64>,
}

impl Default for ClientJournal {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientJournal {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            next_seq: 1,
            bytes: 0,
            dropped_through: 0,
            changed: watch::channel(0).0,
        }
    }

    /// The current position: a reader starting "now" sees only later entries.
    pub fn now(&self) -> u64 {
        self.next_seq - 1
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn total_bytes(&self) -> u64 {
        self.bytes
    }

    /// Wakes whenever an entry is appended.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// Append an occurrence with the data to deliver. Returns its sequence.
    pub fn append(
        &mut self,
        occurrence: Arc<Occurrence>,
        data: Value,
        limits: &JournalLimits,
    ) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        let bytes = Occurrence::data_bytes(&data) as u64;
        self.bytes += bytes;
        self.entries.push_back(JournalEntry {
            seq,
            occurrence,
            data,
            bytes,
            appended_at: Instant::now(),
        });
        self.prune(limits);
        self.changed.send_replace(seq);
        seq
    }

    /// Drop entries beyond the count, size and age bounds.
    pub fn prune(&mut self, limits: &JournalLimits) {
        let now = Instant::now();
        while let Some(front) = self.entries.front() {
            let expired = now.duration_since(front.appended_at) > limits.retention;
            if !(expired || self.entries.len() > limits.max_events || self.bytes > limits.max_bytes)
            {
                break;
            }
            let dropped = self.entries.pop_front().expect("front exists");
            self.bytes -= dropped.bytes;
            self.dropped_through = dropped.seq;
        }
    }

    /// Read up to `limit` matching entries after position `after`. The
    /// position advances over entries that do not match, too.
    pub fn read(&self, after: u64, scope: &ReadScope, limit: usize) -> ReadBatch {
        let truncated = after < self.dropped_through;
        let start = after.max(self.dropped_through).min(self.now());
        let mut entries = Vec::new();
        let mut position = start;
        let mut has_more = false;
        for entry in self.entries.iter().filter(|e| e.seq > start) {
            if !scope.matches(entry) {
                position = entry.seq;
                continue;
            }
            if entries.len() >= limit.max(1) {
                has_more = true;
                break;
            }
            position = entry.seq;
            entries.push(entry.clone());
        }
        if !has_more {
            position = position.max(self.now());
        }
        ReadBatch {
            entries,
            position,
            start,
            truncated,
            has_more,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn occurrence(name: &str, args_key: &str, data: Value) -> Arc<Occurrence> {
        Arc::new(Occurrence {
            event_id: format!("srv__{}", uuid::Uuid::new_v4()),
            name: name.to_string(),
            server_id: "srv-id".into(),
            server_slug: "srv".into(),
            upstream_name: name.trim_start_matches("srv__").into(),
            timestamp: Utc::now(),
            data,
            upstream_cursor: None,
            ingress: IngressMode::Push,
            args_key: args_key.to_string(),
            origin_session: None,
        })
    }

    fn limits(max_events: usize, max_bytes: u64) -> JournalLimits {
        JournalLimits {
            max_events,
            max_bytes,
            retention: Duration::from_secs(3600),
        }
    }

    fn scope(names: &[&str]) -> ReadScope {
        ReadScope {
            names: Some(names.iter().map(|s| s.to_string()).collect()),
            args_key: Some("{}".into()),
        }
    }

    #[test]
    fn reads_from_position_and_skips_other_names() {
        let mut journal = ClientJournal::new();
        let l = limits(100, 1 << 20);
        assert_eq!(journal.now(), 0);
        journal.append(occurrence("srv__a", "{}", json!(1)), json!(1), &l);
        journal.append(occurrence("srv__b", "{}", json!(2)), json!(2), &l);
        journal.append(occurrence("srv__a", "{}", json!(3)), json!(3), &l);
        let batch = journal.read(0, &scope(&["srv__a"]), 10);
        assert_eq!(batch.entries.len(), 2);
        assert_eq!(batch.position, 3);
        assert!(!batch.truncated && !batch.has_more);
        let batch = journal.read(batch.position, &scope(&["srv__a"]), 10);
        assert!(batch.entries.is_empty());
        assert_eq!(batch.position, 3);
        // Other arguments are another subscription
        let mut other = scope(&["srv__a"]);
        other.args_key = Some(r#"{"x":1}"#.into());
        assert!(journal.read(0, &other, 10).entries.is_empty());
    }

    #[test]
    fn pages_with_has_more() {
        let mut journal = ClientJournal::new();
        let l = limits(100, 1 << 20);
        for i in 0..5 {
            journal.append(occurrence("srv__a", "{}", json!(i)), json!(i), &l);
        }
        let first = journal.read(0, &scope(&["srv__a"]), 2);
        assert_eq!(first.entries.len(), 2);
        assert!(first.has_more);
        assert_eq!(first.position, 2);
        let rest = journal.read(first.position, &scope(&["srv__a"]), 10);
        assert_eq!(rest.entries.len(), 3);
        assert!(!rest.has_more);
    }

    #[test]
    fn count_bound_drops_oldest_and_reports_truncation() {
        let mut journal = ClientJournal::new();
        let l = limits(3, 1 << 20);
        for i in 0..5 {
            journal.append(occurrence("srv__a", "{}", json!(i)), json!(i), &l);
        }
        assert_eq!(journal.len(), 3);
        let batch = journal.read(0, &scope(&["srv__a"]), 10);
        assert!(batch.truncated);
        assert_eq!(batch.start, 2);
        assert_eq!(
            batch
                .entries
                .iter()
                .map(|e| e.data.clone())
                .collect::<Vec<_>>(),
            vec![json!(2), json!(3), json!(4)]
        );
        // A reader that had seen everything dropped is not truncated
        assert!(!journal.read(2, &scope(&["srv__a"]), 10).truncated);
    }

    #[test]
    fn byte_bound_drops_oldest() {
        let mut journal = ClientJournal::new();
        let big = json!("x".repeat(100));
        let size = Occurrence::data_bytes(&big) as u64;
        let l = limits(100, size * 2);
        for _ in 0..3 {
            journal.append(occurrence("srv__a", "{}", big.clone()), big.clone(), &l);
        }
        assert_eq!(journal.len(), 2);
        assert_eq!(journal.total_bytes(), size * 2);
    }

    #[test]
    fn retention_drops_old_entries() {
        let mut journal = ClientJournal::new();
        let mut l = limits(100, 1 << 20);
        journal.append(occurrence("srv__a", "{}", json!(1)), json!(1), &l);
        l.retention = Duration::ZERO;
        std::thread::sleep(Duration::from_millis(5));
        journal.prune(&l);
        assert!(journal.is_empty());
        assert!(journal.read(0, &scope(&["srv__a"]), 10).truncated);
    }

    #[test]
    fn channel_occurrences_match_any_arguments() {
        let mut journal = ClientJournal::new();
        let l = limits(100, 1 << 20);
        let mut channel = (*occurrence("srv__channel.message", "", json!({}))).clone();
        channel.ingress = IngressMode::Channel;
        journal.append(Arc::new(channel), json!({}), &l);
        let mut s = scope(&["srv__channel.message"]);
        s.args_key = Some(r#"{"x":1}"#.into());
        assert_eq!(journal.read(0, &s, 10).entries.len(), 1);
    }

    #[test]
    fn wire_form_carries_delivered_data() {
        let mut journal = ClientJournal::new();
        let l = limits(100, 1 << 20);
        let occ = occurrence("srv__a", "{}", json!({"orig": true}));
        journal.append(occ.clone(), json!({"edited": true}), &l);
        let wire = journal.read(0, &ReadScope::default(), 10).entries[0].wire();
        assert_eq!(wire["eventId"], json!(occ.event_id));
        assert_eq!(wire["name"], json!("srv__a"));
        assert_eq!(wire["data"], json!({"edited": true}));
        assert!(wire["timestamp"].as_str().unwrap().ends_with('Z'));
    }
}
