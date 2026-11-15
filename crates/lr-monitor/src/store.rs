use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use uuid::Uuid;

use crate::size::event_size;
use crate::summary::to_summary;
use crate::types::*;

/// Callback type for emitting Tauri events.
pub type EventEmitter = Arc<dyn Fn(&str, String) + Send + Sync>;

/// Callback told the number of requests in progress whenever it changes.
pub type PayloadPolicy = Arc<dyn Fn(Option<&str>) -> bool + Send + Sync>;

pub type PendingListener = Arc<dyn Fn(usize) + Send + Sync>;

/// Default ceiling on the memory the store may hold.
///
/// Events keep whole request and response bodies, so the event count alone does
/// not bound memory: a long-context coding-agent exchange is megabytes, and a
/// few hundred of them run to gigabytes. Over this budget the store first trims
/// the payloads of its oldest events (see [`crate::trim`]) and only evicts
/// events when that is not enough. The event count is a hard limit.
pub const DEFAULT_MAX_BYTES: usize = 128 * 1024 * 1024;

/// Events smaller than this are not worth trimming.
const MIN_TRIM_BYTES: usize = 8 * 1024;

/// List text computed before an event's payload was trimmed, so the list keeps
/// showing the full previews.
struct TrimmedText {
    summary: String,
    question: String,
    answer: String,
}

/// One stored event plus the approximate heap cost charged for it, kept
/// alongside so eviction never has to re-walk a body it is about to drop.
struct Stored {
    event: MonitorEvent,
    size: usize,
    correlation: Option<crate::conversation::Correlation>,
    secret_context: Option<Arc<dyn lr_types::secret_context::SecretContext>>,
    /// Trimming was tried (successfully or not); not retried until the
    /// payload changes.
    trim_attempted: bool,
    trimmed_text: Option<TrimmedText>,
}

impl Stored {
    fn new(
        event: MonitorEvent,
        size: usize,
        secret_context: Option<Arc<dyn lr_types::secret_context::SecretContext>>,
    ) -> Self {
        Self {
            event,
            size,
            correlation: None,
            secret_context,
            trim_attempted: false,
            trimmed_text: None,
        }
    }

    fn charged_size(&self) -> usize {
        event_size(&self.event) + self.correlation.as_ref().map_or(0, |c| c.size())
    }

    fn summary(&self) -> MonitorEventSummary {
        let mut summary = to_summary(&self.event);
        if let Some(text) = &self.trimmed_text {
            summary.summary.clone_from(&text.summary);
            summary.question.clone_from(&text.question);
            summary.answer.clone_from(&text.answer);
        }
        summary
    }
}

/// The buffer and its running totals, under one lock so they cannot drift.
struct Events {
    queue: VecDeque<Stored>,
    total_bytes: usize,
    /// Stored events that are requests still in progress
    /// (see [`is_pending_request`]).
    pending_requests: usize,
}

/// A client request (LLM call or MCP request) that is still running.
///
/// Internal steps (routing, scans, …) are not requests, and a duplicate hop —
/// a request an earlier LocalRouter hop already handled — is not counted
/// twice. Mirrors the dashboard's `isRequest`.
pub fn is_pending_request(event: &MonitorEvent) -> bool {
    event.status == EventStatus::Pending && is_client_request(event)
}

/// A client request (LLM call or MCP request), in progress or finished.
///
/// Internal steps (routing, scans, …) and background events are not client
/// requests, and a duplicate hop is not counted twice.
pub fn is_client_request(event: &MonitorEvent) -> bool {
    match &event.data {
        MonitorEventData::LlmCall { duplicate_hop, .. } => {
            !duplicate_hop.is_some_and(|hop| hop >= 2)
        }
        _ => matches!(
            event.event_type,
            MonitorEventType::McpToolCall
                | MonitorEventType::McpResourceRead
                | MonitorEventType::McpPromptGet
                | MonitorEventType::McpSampling
                | MonitorEventType::McpElicitation
        ),
    }
}

/// In-memory ring buffer for monitor events, bounded by both event count and
/// approximate memory footprint.
pub struct MonitorEventStore {
    events: RwLock<Events>,
    max_capacity: AtomicUsize,
    max_bytes: AtomicUsize,
    /// Unix milliseconds of the last client request start or finish, or of
    /// the store's creation when none has been seen.
    last_request_ms: AtomicI64,
    next_sequence: AtomicU64,
    payload_policy: RwLock<Option<PayloadPolicy>>,
    emitter: RwLock<Option<EventEmitter>>,
    pending_listener: RwLock<Option<PendingListener>>,
}

impl MonitorEventStore {
    /// Create a store bounded by `max_capacity` events and [`DEFAULT_MAX_BYTES`].
    pub fn new(max_capacity: usize) -> Self {
        Self::with_limits(max_capacity, DEFAULT_MAX_BYTES)
    }

    /// Create a store bounded by both an event count and a memory budget.
    pub fn with_limits(max_capacity: usize, max_bytes: usize) -> Self {
        Self {
            events: RwLock::new(Events {
                queue: VecDeque::with_capacity(max_capacity.min(2048)),
                total_bytes: 0,
                pending_requests: 0,
            }),
            max_capacity: AtomicUsize::new(max_capacity.max(1)),
            max_bytes: AtomicUsize::new(max_bytes.max(1)),
            last_request_ms: AtomicI64::new(Utc::now().timestamp_millis()),
            next_sequence: AtomicU64::new(1),
            payload_policy: RwLock::new(None),
            emitter: RwLock::new(None),
            pending_listener: RwLock::new(None),
        }
    }

    /// Bring the buffer within both limits.
    ///
    /// Over the event count, the oldest events are dropped. Over the memory
    /// budget, the payloads of the oldest finished events are trimmed first so
    /// they stay listed; events are dropped only if trimming is not enough.
    ///
    /// The newest event is always kept whole, even when it alone exceeds the
    /// budget: shortening or evicting it would make a single oversized exchange
    /// impossible to inspect, which is exactly the traffic a user opens the
    /// monitor to look at.
    fn evict(&self, events: &mut Events) {
        let max_capacity = self.max_capacity.load(Ordering::Relaxed);
        let max_bytes = self.max_bytes.load(Ordering::Relaxed);

        while events.queue.len() > max_capacity {
            Self::pop_oldest(events);
        }
        if events.total_bytes > max_bytes {
            Self::trim_oldest(events, max_bytes);
        }
        while events.total_bytes > max_bytes && events.queue.len() > 1 {
            Self::pop_oldest(events);
        }
    }

    fn pop_oldest(events: &mut Events) {
        if let Some(evicted) = events.queue.pop_front() {
            events.total_bytes = events.total_bytes.saturating_sub(evicted.size);
            if is_pending_request(&evicted.event) {
                events.pending_requests = events.pending_requests.saturating_sub(1);
            }
        }
    }

    /// Trim payloads oldest-first until the buffer fits `max_bytes`.
    ///
    /// In-progress events are skipped (their response is still arriving), as
    /// are secret-scan records, which later redaction restores read back.
    fn trim_oldest(events: &mut Events, max_bytes: usize) {
        let newest = events.queue.len().saturating_sub(1);
        for index in 0..newest {
            if events.total_bytes <= max_bytes {
                break;
            }
            let stored = &mut events.queue[index];
            if stored.trim_attempted
                || stored.size < MIN_TRIM_BYTES
                || stored.event.status == EventStatus::Pending
                || stored.event.payload_redacted
                || stored.event.event_type == MonitorEventType::SecretScan
            {
                continue;
            }
            stored.trim_attempted = true;
            let before = to_summary(&stored.event);
            if !crate::trim::trim_payload(&mut stored.event) {
                continue;
            }
            stored.trimmed_text = Some(TrimmedText {
                summary: before.summary,
                question: before.question,
                answer: before.answer,
            });
            let old_size = stored.size;
            stored.size = stored.charged_size();
            events.total_bytes = events.total_bytes + stored.size - old_size;
        }
    }

    /// Record client-request activity for [`Self::last_request_at`].
    fn note_activity(&self, event: &MonitorEvent) {
        if is_client_request(event) {
            self.last_request_ms
                .fetch_max(Utc::now().timestamp_millis(), Ordering::Relaxed);
        }
    }

    /// When a client request (LLM call or MCP request) last started or
    /// finished; the store's creation time if none has been seen.
    pub fn last_request_at(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(self.last_request_ms.load(Ordering::Relaxed))
            .unwrap_or_else(Utc::now)
    }

    /// Applies to new events only. Existing captures are untouched; callers
    /// must expose a separate, explicit user cleanup action for old history.
    pub fn set_payload_policy<F: Fn(Option<&str>) -> bool + Send + Sync + 'static>(
        &self,
        policy: F,
    ) {
        *self.payload_policy.write() = Some(Arc::new(policy));
    }
    fn new_event_payload_policy(&self, event: &mut MonitorEvent) {
        let policy = self.payload_policy.read().clone();
        if policy.is_some_and(|policy| policy(event.client_id.as_deref())) {
            crate::privacy::redact(event);
        }
    }

    /// Set the event emitter callback (typically wired to Tauri's emit).
    pub fn set_emitter<F: Fn(&str, String) + Send + Sync + 'static>(&self, emitter: F) {
        *self.emitter.write() = Some(Arc::new(emitter));
    }

    /// Set the callback told the number of requests in progress whenever it
    /// changes. It runs under the store's lock, so successive counts arrive
    /// in order; it must be quick and must not call back into the store.
    pub fn set_pending_listener<F: Fn(usize) + Send + Sync + 'static>(&self, listener: F) {
        let events = self.events.read();
        let listener: PendingListener = Arc::new(listener);
        listener(events.pending_requests);
        *self.pending_listener.write() = Some(listener);
    }

    /// Requests (LLM calls and MCP requests) still in progress.
    pub fn pending_requests(&self) -> usize {
        self.events.read().pending_requests
    }

    /// Tell the listener the in-progress count if it moved from `before`.
    /// Called with the events lock held.
    fn notify_pending(&self, events: &Events, before: usize) {
        if events.pending_requests == before {
            return;
        }
        if let Some(listener) = self.pending_listener.read().as_ref() {
            listener(events.pending_requests);
        }
    }

    /// Push a new event into the store. Returns the assigned event ID.
    ///
    /// The caller provides the event data; this method assigns the ID, sequence,
    /// and timestamp. If the store is at capacity, the oldest event is evicted.
    pub fn push(
        &self,
        event_type: MonitorEventType,
        client_id: Option<String>,
        client_name: Option<String>,
        session_id: Option<String>,
        data: MonitorEventData,
        status: EventStatus,
        duration_ms: Option<u64>,
    ) -> String {
        let id = format!("mon-{}", Uuid::new_v4());
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);

        let mut event = MonitorEvent {
            conversation: None,
            payload_redacted: false,
            payload_trimmed: false,
            id: id.clone(),
            sequence,
            timestamp: Utc::now(),
            event_type,
            client_id,
            client_name,
            session_id,
            data,
            status,
            duration_ms,
        };

        let secret_context =
            lr_types::secret_context::current().and_then(|context| context.monitor_context());
        crate::privacy::mask_secrets(&mut event, secret_context.as_deref());
        self.new_event_payload_policy(&mut event);
        let summary;

        {
            let mut events = self.events.write();
            let before = events.pending_requests;
            if !event.payload_redacted && event.session_id.is_some() {
                event.conversation = events.queue.iter().rev().find_map(|s| {
                    (s.event.client_id == event.client_id && s.event.session_id == event.session_id)
                        .then(|| s.event.conversation.clone())
                        .flatten()
                });
            }
            summary = to_summary(&event);
            let size = event_size(&event);
            if is_pending_request(&event) {
                events.pending_requests += 1;
            }
            events.total_bytes += size;
            self.note_activity(&event);
            events
                .queue
                .push_back(Stored::new(event, size, secret_context));
            self.evict(&mut events);
            self.notify_pending(&events, before);
        }

        // Callbacks may reconfigure the store; release the emitter lock first.
        let emitter = self.emitter.read().clone();
        if let Some(emitter) = emitter {
            if let Ok(payload) = serde_json::to_string(&summary) {
                emitter("monitor-event-created", payload);
            }
        }

        id
    }

    /// Correlate before request truncation, while honoring the capture policy.
    /// State lives with retained events and disappears on eviction/clear.
    pub fn track_conversation(&self, id: &str, body: &serde_json::Value) {
        // Avoid hashing protected captures, and keep long-context hashing outside
        // the store write lock so capture/list/update operations remain responsive.
        {
            let events = self.events.read();
            let Some(stored) = events.queue.iter().find(|s| s.event.id == id) else {
                return;
            };
            if stored.event.payload_redacted
                || stored.event.client_id.is_none()
                || stored.correlation.is_some()
            {
                return;
            }
        }
        let Some(correlation) = crate::conversation::Correlation::from_body(body) else {
            return;
        };
        let mut events = self.events.write();
        let Some(index) = events.queue.iter().position(|s| s.event.id == id) else {
            return;
        };
        if events.queue[index].event.payload_redacted || events.queue[index].correlation.is_some() {
            return;
        }
        let client = events.queue[index].event.client_id.clone();
        // Anonymous traffic cannot safely share an identity across callers.
        if client.is_none() {
            return;
        }
        let mut candidates = std::collections::BTreeMap::new();
        let mut longest_prefix = 0;
        for previous in events.queue.iter().take(index).rev() {
            if previous.event.client_id != client {
                continue;
            }
            let (Some(prior), Some(conversation)) =
                (&previous.correlation, &previous.event.conversation)
            else {
                continue;
            };
            if correlation.references_response(&previous.event.data) || correlation.follows(prior) {
                let length = if correlation.is_response_chain() {
                    usize::MAX
                } else {
                    prior.history_len()
                };
                if !correlation.is_explicit() {
                    if length < longest_prefix {
                        continue;
                    }
                    if length > longest_prefix {
                        candidates.clear();
                        longest_prefix = length;
                    }
                }
                let mut candidate = conversation.clone();
                if correlation.references_response(&previous.event.data)
                    && candidate.source != ConversationSource::Explicit
                {
                    candidate.source = ConversationSource::ResponseChain;
                }
                candidates.insert(candidate.id.clone(), candidate);
                // Explicit identities use the latest retained epoch (reset starts one).
                if correlation.is_explicit() {
                    break;
                }
            }
        }
        let mut conversation = if candidates.len() == 1 {
            candidates.into_values().next().unwrap()
        } else {
            MonitorConversation {
                id: format!("conv-{}", Uuid::new_v4()),
                title: correlation.title.clone(),
                source: if correlation.is_explicit() {
                    ConversationSource::Explicit
                } else if correlation.is_response_chain() {
                    ConversationSource::ResponseChain
                } else {
                    ConversationSource::MessageHistory
                },
            }
        };
        if conversation.title.is_empty() {
            conversation.title.clone_from(&correlation.title);
        }
        let session = events.queue[index].event.session_id.clone();
        events.queue[index].correlation = Some(correlation);
        let mut summaries = Vec::new();
        let mut added = 0;
        let mut removed = 0;
        for stored in &mut events.queue {
            if stored.event.id != id
                && !(session.is_some()
                    && stored.event.session_id == session
                    && stored.event.client_id == client)
            {
                continue;
            }
            if stored.event.payload_redacted {
                continue;
            }
            stored.event.conversation = Some(conversation.clone());
            crate::privacy::mask_secrets(&mut stored.event, stored.secret_context.as_deref());
            removed += stored.size;
            stored.size = stored.charged_size();
            added += stored.size;
            summaries.push(stored.summary());
        }
        events.total_bytes = events.total_bytes + added - removed;
        let before = events.pending_requests;
        self.evict(&mut events);
        self.notify_pending(&events, before);
        drop(events);
        let emitter = self.emitter.read().clone();
        if let Some(emitter) = emitter {
            for summary in summaries {
                if let Ok(payload) = serde_json::to_string(&summary) {
                    emitter("monitor-event-updated", payload);
                }
            }
        }
    }

    /// Reuse the detection record for an actual redaction in the same request.
    pub fn redaction_for_request(
        &self,
        client: &str,
        session: &str,
        after_sequence: u64,
    ) -> Option<MonitorEvent> {
        self.events.read().queue.iter().rev().find_map(|stored| {
            let event = &stored.event;
            if event.sequence <= after_sequence
                || event.client_id.as_deref() != Some(client)
                || event.session_id.as_deref() != Some(session)
            {
                return None;
            }
            match &event.data {
                MonitorEventData::SecretScan {
                    action_taken,
                    findings,
                    ..
                } if action_taken.as_deref() != Some("restore")
                    && (action_taken.as_deref() == Some("redact")
                        || findings
                            .as_ref()
                            .and_then(|v| v.as_array())
                            .is_some_and(|f| f.iter().any(|f| f["redacted"] == true))) =>
                {
                    Some(event.clone())
                }
                _ => None,
            }
        })
    }

    /// Update an existing event (e.g., combined event completion).
    /// Returns true if the event was found and updated.
    pub fn update<F>(&self, id: &str, updater: F) -> bool
    where
        F: FnOnce(&mut MonitorEvent),
    {
        let mut events = self.events.write();
        let before = events.pending_requests;
        if let Some(stored) = events.queue.iter_mut().rev().find(|s| s.event.id == id) {
            let was_pending = is_pending_request(&stored.event);
            let was_redacted = stored.event.payload_redacted;
            updater(&mut stored.event);
            crate::privacy::mask_secrets(&mut stored.event, stored.secret_context.as_deref());
            // An event created without capture stays protected even if its
            // client later selects full capture. Old full captures are unchanged.
            if was_redacted {
                crate::privacy::redact(&mut stored.event);
            }
            let is_pending = is_pending_request(&stored.event);
            // New payload may arrive after a trim; the list previews follow
            // the new data, and the event may be trimmed again.
            stored.trimmed_text = None;
            stored.trim_attempted = false;
            if is_pending != was_pending {
                self.note_activity(&stored.event);
            }
            let updated_summary = to_summary(&stored.event);

            // The response half is usually where the bulk of an LLM event
            // arrives, so re-charge the entry and re-check the budget.
            let old_size = stored.size;
            let new_size = stored.charged_size();
            stored.size = new_size;
            events.total_bytes = events.total_bytes + new_size - old_size;
            match (was_pending, is_pending) {
                (true, false) => {
                    events.pending_requests = events.pending_requests.saturating_sub(1)
                }
                (false, true) => events.pending_requests += 1,
                _ => {}
            }
            self.evict(&mut events);
            self.notify_pending(&events, before);
            drop(events);

            let emitter = self.emitter.read().clone();
            if let Some(emitter) = emitter {
                if let Ok(payload) = serde_json::to_string(&updated_summary) {
                    emitter("monitor-event-updated", payload);
                }
            }
            true
        } else {
            false
        }
    }

    /// Get paginated event summaries, newest first.
    /// Optional filter narrows results before pagination.
    pub fn list(
        &self,
        offset: usize,
        limit: usize,
        filter: Option<&MonitorEventFilter>,
    ) -> MonitorEventListResponse {
        let events = self.events.read();

        let filtered: Vec<&Stored> = events
            .queue
            .iter()
            .rev() // newest first
            .filter(|s| match_filter(&s.event, filter))
            .collect();

        let total = filtered.len();
        let page: Vec<MonitorEventSummary> = filtered
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(Stored::summary)
            .collect();

        MonitorEventListResponse {
            events: page,
            total,
        }
    }

    /// Get full event detail by ID.
    pub fn get(&self, id: &str) -> Option<MonitorEvent> {
        let events = self.events.read();
        events
            .queue
            .iter()
            .rev()
            .find(|s| s.event.id == id)
            .map(|s| s.event.clone())
    }

    /// Clear all events.
    pub fn clear(&self) {
        let mut events = self.events.write();
        let before = events.pending_requests;
        events.queue.clear();
        events.total_bytes = 0;
        events.pending_requests = 0;
        self.notify_pending(&events, before);
    }

    /// Update the maximum event count. If the new capacity is smaller,
    /// excess old events are evicted immediately.
    pub fn set_max_capacity(&self, cap: usize) {
        self.max_capacity.store(cap.max(1), Ordering::Relaxed);
        let mut events = self.events.write();
        let before = events.pending_requests;
        self.evict(&mut events);
        self.notify_pending(&events, before);
    }

    /// Update the memory budget. If the new budget is smaller, old events are
    /// evicted immediately until the buffer fits.
    pub fn set_max_bytes(&self, max_bytes: usize) {
        self.max_bytes.store(max_bytes.max(1), Ordering::Relaxed);
        let mut events = self.events.write();
        let before = events.pending_requests;
        self.evict(&mut events);
        self.notify_pending(&events, before);
    }

    /// Get current store statistics.
    pub fn stats(&self) -> MonitorStats {
        let events = self.events.read();
        let mut by_type = std::collections::HashMap::new();
        for stored in events.queue.iter() {
            *by_type.entry(stored.event.event_type).or_insert(0) += 1;
        }
        MonitorStats {
            total_events: events.queue.len(),
            max_capacity: self.max_capacity.load(Ordering::Relaxed),
            total_bytes: events.total_bytes,
            max_bytes: self.max_bytes.load(Ordering::Relaxed),
            events_by_type: by_type,
        }
    }
}

/// Check if an event matches the given filter.
fn match_filter(event: &MonitorEvent, filter: Option<&MonitorEventFilter>) -> bool {
    let Some(filter) = filter else {
        return true;
    };

    if let Some(types) = &filter.event_types {
        if !types.contains(&event.event_type) {
            return false;
        }
    }

    if let Some(client_id) = &filter.client_id {
        if event.client_id.as_deref() != Some(client_id.as_str()) {
            return false;
        }
    }

    if let Some(status) = &filter.status {
        if event.status != *status {
            return false;
        }
    }

    if let Some(session_id) = &filter.session_id {
        if event.session_id.as_deref() != Some(session_id.as_str()) {
            return false;
        }
    }

    if let Some(search) = &filter.search {
        if !search.is_empty() {
            let summary = crate::summary::to_summary(event);
            let search_lower = search.to_lowercase();
            let trace_match = matches!(
                &event.data,
                MonitorEventData::LlmCall { trace_id: Some(t), .. } if t.to_lowercase().contains(&search_lower)
            );
            if ![&summary.summary, &summary.question, &summary.answer]
                .iter()
                .any(|text| text.to_lowercase().contains(&search_lower))
                && !trace_match
            {
                return false;
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store(cap: usize) -> MonitorEventStore {
        MonitorEventStore::new(cap)
    }

    fn push_llm_call(store: &MonitorEventStore, model: &str) -> String {
        store.push(
            MonitorEventType::LlmCall,
            Some("client-1".to_string()),
            Some("Test Client".to_string()),
            Some(format!("sess-{}", Uuid::new_v4())),
            MonitorEventData::LlmCall {
                client_api: None,
                upstream_api: None,
                requested_model: None,
                cached_input_tokens: None,
                endpoint: "/v1/chat/completions".to_string(),
                model: model.to_string(),
                stream: false,
                message_count: 3,
                has_tools: false,
                tool_count: 0,
                request_body: serde_json::json!({"model": model}),
                source: LlmCallSource::Api,
                protocol: LlmProtocol::Openai,
                raw_request: None,
                raw_response: None,
                transformed_body: None,
                transformations_applied: None,
                provider: None,
                status_code: None,
                input_tokens: None,
                output_tokens: None,
                total_tokens: None,
                reasoning_tokens: None,
                cost_usd: None,
                latency_ms: None,
                finish_reason: None,
                content_preview: None,
                streamed: None,
                response_body: None,
                error: None,
                routing_info: None,
                trace_id: None,
                duplicate_hop: None,
            },
            EventStatus::Pending,
            None,
        )
    }

    #[test]
    fn capture_policy_is_new_only_client_scoped_and_sticky_through_completion() {
        let store = make_store(100);
        let old = push_llm_call(&store, "old-model");
        store.set_payload_policy(|client| client == Some("client-1"));
        let protected = push_llm_call(&store, "new-model");
        let other = store.push(
            MonitorEventType::InternalError,
            Some("client-2".into()),
            None,
            None,
            MonitorEventData::InternalError {
                error_type: "example".into(),
                message: "other-client-detail".into(),
                status_code: 500,
            },
            EventStatus::Error,
            None,
        );
        let original = store.get(&old).unwrap();
        assert!(!original.payload_redacted);
        assert!(original.session_id.is_some());
        assert!(!store.get(&other).unwrap().payload_redacted);
        assert_eq!(store.pending_requests(), 2);
        store.set_payload_policy(|_| false);
        store.update(&protected, |event| {
            event.status = EventStatus::Complete;
            if let MonitorEventData::LlmCall {
                response_body,
                content_preview,
                output_tokens,
                cost_usd,
                ..
            } = &mut event.data
            {
                *response_body = Some(serde_json::json!({"secret":"late-payload"}));
                *content_preview = Some("late-payload".into());
                *output_tokens = Some(25);
                *cost_usd = Some(0.1);
            } else {
                panic!("typed event lifecycle was lost");
            }
        });
        let event = store.get(&protected).unwrap();
        assert!(event.payload_redacted);
        assert!(event.session_id.is_none());
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("late-payload"));
        if let MonitorEventData::LlmCall {
            output_tokens,
            cost_usd,
            ..
        } = event.data
        {
            assert_eq!(output_tokens, Some(25));
            assert_eq!(cost_usd, Some(0.1));
        } else {
            panic!("typed lifecycle was lost");
        }
        assert_eq!(store.pending_requests(), 1);
        assert!(!store.get(&old).unwrap().payload_redacted);
    }
    fn push_push_event(store: &MonitorEventStore, session_id: Option<String>) -> String {
        store.push(
            MonitorEventType::McpPushEvent,
            Some("client-1".to_string()),
            Some("Test Client".to_string()),
            session_id,
            MonitorEventData::McpPushEvent {
                server_id: "vc".into(),
                server_name: Some("vocalcord".into()),
                event_name: "vocalcord__message.received".into(),
                event_id: format!("vocalcord__{}", Uuid::new_v4()),
                ingress_mode: "push".into(),
                egress_modes: vec!["tools".into()],
                decision: "allowed".into(),
                original_payload: Some(serde_json::json!({"text": "hi"})),
                delivered_payload: Some(serde_json::json!({"text": "hi"})),
                held_ms: None,
                error: None,
            },
            EventStatus::Complete,
            None,
        )
    }

    #[test]
    fn push_events_group_with_the_reading_sessions_request() {
        let store = make_store(20);
        let body = serde_json::json!({"messages":[{"role":"user","content":"Watch my inbox"}]});
        let llm = push_llm_call(&store, "test");
        store.track_conversation(&llm, &body);
        let conversation = store.get(&llm).unwrap().conversation.unwrap();
        let request = store.get(&llm).unwrap().session_id;

        // Delivered to the session recorded under that request id: same group
        let delivered = push_push_event(&store, request.clone());
        assert_eq!(
            store.get(&delivered).unwrap().conversation.unwrap().id,
            conversation.id
        );
        // No reading session (webhook, /events/watch, several sessions):
        // standalone, even though the client has a conversation
        let standalone = push_push_event(&store, None);
        assert!(store.get(&standalone).unwrap().conversation.is_none());

        // Recorded before the request was correlated: the correlation reaches it
        let early_request = Some(format!("sess-{}", Uuid::new_v4()));
        let early = push_push_event(&store, early_request.clone());
        assert!(store.get(&early).unwrap().conversation.is_none());
        let later = push_llm_call(&store, "test");
        store.update(&later, |e| e.session_id.clone_from(&early_request));
        store.track_conversation(&later, &body);
        let later_conversation = store.get(&later).unwrap().conversation.unwrap();
        assert_eq!(
            store.get(&early).unwrap().conversation.unwrap().id,
            later_conversation.id
        );
    }

    #[test]
    fn conversation_capture_is_scoped_bounded_and_propagates_to_request_events() {
        let store = make_store(20);
        let body = serde_json::json!({"extensions":{"localrouter.session":{"id":"private-id"}},"messages":[{"role":"user","content":"First question"}]});
        let first = push_llm_call(&store, "test");
        store.track_conversation(&first, &body);
        let metadata = store.get(&first).unwrap().conversation.unwrap();
        assert_eq!(metadata.title, "First question");
        assert!(!metadata.id.contains("private-id"));
        let second = push_llm_call(&store, "test");
        store.track_conversation(&second, &body);
        assert_eq!(
            store.get(&second).unwrap().conversation.unwrap().id,
            metadata.id
        );
        let parent = store.get(&first).unwrap();
        let child = store.push(
            MonitorEventType::InternalError,
            parent.client_id,
            None,
            parent.session_id,
            MonitorEventData::InternalError {
                error_type: "test".into(),
                message: "test".into(),
                status_code: 500,
            },
            EventStatus::Error,
            None,
        );
        assert_eq!(
            store.get(&child).unwrap().conversation.unwrap().id,
            metadata.id
        );
        let other = push_llm_call(&store, "test");
        store.update(&other, |e| e.client_id = Some("other-client".into()));
        store.track_conversation(&other, &body);
        assert_ne!(
            store.get(&other).unwrap().conversation.unwrap().id,
            metadata.id
        );
        let reset = push_llm_call(&store, "test");
        store.track_conversation(&reset, &serde_json::json!({"extensions":{"localrouter.session":{"id":"private-id","reset":true}}}));
        let reset_id = store.get(&reset).unwrap().conversation.unwrap().id;
        assert_ne!(reset_id, metadata.id);
        let next = push_llm_call(&store, "test");
        store.track_conversation(&next, &body);
        assert_eq!(store.get(&next).unwrap().conversation.unwrap().id, reset_id);
        store.clear();
        assert_eq!(store.stats().total_bytes, 0);
        let fresh = push_llm_call(&store, "test");
        store.track_conversation(&fresh, &body);
        assert_ne!(
            store.get(&fresh).unwrap().conversation.unwrap().id,
            reset_id
        );
    }

    #[test]
    fn history_correlation_rejects_ambiguous_continuations_and_protects_redacted_capture() {
        let store = make_store(20);
        let first_body =
            serde_json::json!({"messages":[{"role":"user","content":"same question"}]});
        let a = push_llm_call(&store, "test");
        let b = push_llm_call(&store, "test");
        store.track_conversation(&a, &first_body);
        store.track_conversation(&b, &first_body);
        let a_id = store.get(&a).unwrap().conversation.unwrap().id;
        let b_id = store.get(&b).unwrap().conversation.unwrap().id;
        assert_ne!(a_id, b_id);
        let c = push_llm_call(&store, "test");
        store.track_conversation(&c, &serde_json::json!({"messages":[{"role":"user","content":"same question"},{"role":"assistant","content":"answer"},{"role":"user","content":"next"}]}));
        let c_id = store.get(&c).unwrap().conversation.unwrap().id;
        assert_ne!(c_id, a_id);
        assert_ne!(c_id, b_id);
        let d = push_llm_call(&store, "test");
        store.track_conversation(&d, &serde_json::json!({"messages":[{"role":"user","content":"same question"},{"role":"assistant","content":"answer"},{"role":"user","content":"next"},{"role":"assistant","content":"next answer"},{"role":"user","content":"third"}]}));
        assert_eq!(store.get(&d).unwrap().conversation.unwrap().id, c_id);
        store.set_payload_policy(|_| true);
        let hidden = push_llm_call(&store, "test");
        let before = store.stats().total_bytes;
        store.track_conversation(&hidden, &first_body);
        assert!(store.get(&hidden).unwrap().conversation.is_none());
        assert_eq!(store.stats().total_bytes, before);
    }

    #[test]
    fn response_chains_keep_the_first_title_and_emit_without_locks() {
        let store = Arc::new(make_store(20));
        let first = push_llm_call(&store, "test");
        store.track_conversation(
            &first,
            &serde_json::json!({"input":"First response question"}),
        );
        store.update(&first, |e| {
            if let MonitorEventData::LlmCall { response_body, .. } = &mut e.data {
                *response_body = Some(serde_json::json!({"id":"resp-1"}));
            }
        });
        let first_conversation = store.get(&first).unwrap().conversation.unwrap();
        let second = push_llm_call(&store, "test");
        let weak = Arc::downgrade(&store);
        store.set_emitter(move |name, payload| {
            assert_eq!(name, "monitor-event-updated");
            assert!(payload.contains("First response question"));
            // Callbacks may reconfigure emitters and query the event store.
            let store = weak.upgrade().unwrap();
            store.set_emitter(|_, _| {});
            let _ = store.stats();
        });
        store.track_conversation(
            &second,
            &serde_json::json!({"input":"Next question", "previous_response_id":"resp-1"}),
        );
        assert_eq!(
            store.get(&second).unwrap().conversation.unwrap().id,
            first_conversation.id
        );
        assert!(store.stats().total_bytes > 0);
        // Eviction discards identity/fingerprints; no hidden unbounded cache survives.
        store.set_max_capacity(1);
        let fresh = push_llm_call(&store, "test");
        store.track_conversation(
            &fresh,
            &serde_json::json!({"input":"Third question", "previous_response_id":"resp-1"}),
        );
        assert_ne!(
            store.get(&fresh).unwrap().conversation.unwrap().id,
            first_conversation.id
        );
    }

    #[test]
    fn test_push_and_get() {
        let store = make_store(100);
        let id = push_llm_call(&store, "gpt-4");
        let event = store.get(&id).unwrap();
        assert_eq!(event.id, id);
        assert_eq!(event.event_type, MonitorEventType::LlmCall);
    }

    #[test]
    fn event_emitter_can_replace_itself_on_create_and_update() {
        let store = Arc::new(make_store(100));
        let calls = Arc::new(AtomicUsize::new(0));
        let install_callback = || {
            let weak_store = Arc::downgrade(&store);
            let calls = calls.clone();
            store.set_emitter(move |_, _| {
                let store = weak_store.upgrade().unwrap();
                // Fail without hanging if callbacks regress to running under the lock.
                assert!(store.emitter.try_write().is_some());
                store.set_emitter(|_, _| {});
                calls.fetch_add(1, Ordering::Relaxed);
            });
        };

        install_callback();
        let id = push_llm_call(&store, "synthetic-model");
        install_callback();
        assert!(store.update(&id, |event| event.status = EventStatus::Complete));
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_fifo_eviction() {
        let store = make_store(3);
        let id1 = push_llm_call(&store, "model-1");
        let _id2 = push_llm_call(&store, "model-2");
        let _id3 = push_llm_call(&store, "model-3");

        // Store is at capacity, push one more
        let _id4 = push_llm_call(&store, "model-4");

        // First event should be evicted
        assert!(store.get(&id1).is_none());
        assert_eq!(store.stats().total_events, 3);
    }

    #[test]
    fn test_list_newest_first() {
        let store = make_store(100);
        push_llm_call(&store, "model-a");
        push_llm_call(&store, "model-b");
        push_llm_call(&store, "model-c");

        let result = store.list(0, 10, None);
        assert_eq!(result.total, 3);
        assert_eq!(result.events.len(), 3);
        // newest first: sequence descending
        assert!(result.events[0].sequence > result.events[1].sequence);
        assert!(result.events[1].sequence > result.events[2].sequence);
    }

    #[test]
    fn test_list_pagination() {
        let store = make_store(100);
        for i in 0..10 {
            push_llm_call(&store, &format!("model-{}", i));
        }

        let page1 = store.list(0, 3, None);
        assert_eq!(page1.total, 10);
        assert_eq!(page1.events.len(), 3);

        let page2 = store.list(3, 3, None);
        assert_eq!(page2.events.len(), 3);
        // No overlap
        assert_ne!(page1.events[2].id, page2.events[0].id);
    }

    #[test]
    fn test_filter_by_type() {
        let store = make_store(100);
        push_llm_call(&store, "gpt-4");
        store.push(
            MonitorEventType::McpToolCall,
            Some("client-1".to_string()),
            None,
            None,
            MonitorEventData::McpToolCall {
                tool_name: "search".to_string(),
                server_id: "srv-1".to_string(),
                server_name: None,
                arguments: serde_json::json!({}),
                firewall_action: None,
                latency_ms: None,
                success: None,
                response_preview: None,
                error: None,
            },
            EventStatus::Pending,
            None,
        );

        let filter = MonitorEventFilter {
            event_types: Some(vec![MonitorEventType::McpToolCall]),
            ..Default::default()
        };
        let result = store.list(0, 10, Some(&filter));
        assert_eq!(result.total, 1);
        assert_eq!(result.events[0].event_type, MonitorEventType::McpToolCall);
    }

    #[test]
    fn test_update_llm_call_completion() {
        let store = make_store(100);
        let id = push_llm_call(&store, "gpt-4");

        // Verify initial state
        let event = store.get(&id).unwrap();
        assert_eq!(event.status, EventStatus::Pending);

        // Complete the event with response data
        let updated = store.update(&id, |e| {
            e.status = EventStatus::Complete;
            e.duration_ms = Some(1500);
            if let MonitorEventData::LlmCall {
                provider,
                status_code,
                output_tokens,
                total_tokens,
                content_preview,
                streamed,
                ..
            } = &mut e.data
            {
                *provider = Some("openai".to_string());
                *status_code = Some(200);
                *output_tokens = Some(150);
                *total_tokens = Some(200);
                *content_preview = Some("Hello, how can I help?".to_string());
                *streamed = Some(false);
            }
        });
        assert!(updated);

        let event = store.get(&id).unwrap();
        assert_eq!(event.status, EventStatus::Complete);
        assert_eq!(event.duration_ms, Some(1500));
    }

    #[test]
    fn test_filter_by_session() {
        let store = make_store(100);
        let session = "sess-abc".to_string();

        // Push events in the same session
        store.push(
            MonitorEventType::LlmCall,
            Some("client-1".to_string()),
            None,
            Some(session.clone()),
            MonitorEventData::LlmCall {
                client_api: None,
                upstream_api: None,
                requested_model: None,
                cached_input_tokens: None,
                endpoint: "/v1/chat/completions".to_string(),
                model: "gpt-4".to_string(),
                stream: false,
                message_count: 1,
                has_tools: false,
                tool_count: 0,
                request_body: serde_json::json!({}),
                source: LlmCallSource::Api,
                protocol: LlmProtocol::Openai,
                raw_request: None,
                raw_response: None,
                transformed_body: None,
                transformations_applied: None,
                provider: None,
                status_code: None,
                input_tokens: None,
                output_tokens: None,
                total_tokens: None,
                reasoning_tokens: None,
                cost_usd: None,
                latency_ms: None,
                finish_reason: None,
                content_preview: None,
                streamed: None,
                response_body: None,
                error: None,
                routing_info: None,
                trace_id: None,
                duplicate_hop: None,
            },
            EventStatus::Pending,
            None,
        );
        store.push(
            MonitorEventType::SecretScan,
            Some("client-1".to_string()),
            None,
            Some(session.clone()),
            MonitorEventData::SecretScan {
                text_preview: "test".to_string(),
                rules_count: 5,
                findings_count: None,
                findings: None,
                action_taken: None,
                latency_ms: None,
                error: None,
            },
            EventStatus::Pending,
            None,
        );
        // Different session
        push_llm_call(&store, "other-model");

        let filter = MonitorEventFilter {
            session_id: Some(session),
            ..Default::default()
        };
        let result = store.list(0, 10, Some(&filter));
        assert_eq!(result.total, 2);
    }

    #[test]
    fn test_clear() {
        let store = make_store(100);
        push_llm_call(&store, "gpt-4");
        push_llm_call(&store, "gpt-3.5");
        store.clear();
        assert_eq!(store.stats().total_events, 0);
    }

    #[test]
    fn test_set_max_capacity_shrink() {
        let store = make_store(10);
        for i in 0..10 {
            push_llm_call(&store, &format!("model-{}", i));
        }
        assert_eq!(store.stats().total_events, 10);

        store.set_max_capacity(5);
        assert_eq!(store.stats().total_events, 5);
        assert_eq!(store.stats().max_capacity, 5);
    }

    /// Push an LLM call whose request body is roughly `body_bytes` of payload.
    fn push_bulky(store: &MonitorEventStore, body_bytes: usize) -> String {
        store.push(
            MonitorEventType::LlmCall,
            None,
            None,
            None,
            MonitorEventData::LlmCall {
                client_api: None,
                upstream_api: None,
                requested_model: None,
                cached_input_tokens: None,
                endpoint: "/v1/chat/completions".to_string(),
                model: "gpt-4".to_string(),
                stream: false,
                message_count: 1,
                has_tools: false,
                tool_count: 0,
                request_body: serde_json::json!({"prompt": "x".repeat(body_bytes)}),
                source: LlmCallSource::Api,
                protocol: LlmProtocol::Openai,
                raw_request: None,
                raw_response: None,
                transformed_body: None,
                transformations_applied: None,
                provider: None,
                status_code: None,
                input_tokens: None,
                output_tokens: None,
                total_tokens: None,
                reasoning_tokens: None,
                cost_usd: None,
                latency_ms: None,
                finish_reason: None,
                content_preview: None,
                streamed: None,
                response_body: None,
                error: None,
                routing_info: None,
                trace_id: None,
                duplicate_hop: None,
            },
            EventStatus::Pending,
            None,
        )
    }

    #[test]
    fn test_byte_budget_evicts_before_capacity_is_reached() {
        // Room for 100 events by count, but only a few by size.
        let store = MonitorEventStore::with_limits(100, 50_000);
        for _ in 0..20 {
            push_bulky(&store, 10_000);
        }

        let stats = store.stats();
        assert!(stats.total_events < 20, "byte budget should have evicted");
        assert!(stats.total_bytes <= stats.max_bytes);
        assert_eq!(stats.max_bytes, 50_000);
    }

    fn push_finished_bulky(store: &MonitorEventStore, body_bytes: usize) -> String {
        let id = push_bulky(store, body_bytes);
        store.update(&id, |e| {
            e.status = EventStatus::Complete;
            if let MonitorEventData::LlmCall { response_body, .. } = &mut e.data {
                *response_body = Some(serde_json::json!({"text": "y".repeat(body_bytes)}));
            }
        });
        id
    }

    #[test]
    fn test_byte_budget_trims_finished_events_instead_of_dropping_them() {
        // Room for 100 events by count; their whole bodies would only fit a few.
        let store = MonitorEventStore::with_limits(100, 200_000);
        let ids: Vec<String> = (0..40)
            .map(|_| push_finished_bulky(&store, 20_000))
            .collect();

        let stats = store.stats();
        assert_eq!(stats.total_events, 40, "no event should be dropped");
        assert!(stats.total_bytes <= stats.max_bytes);

        let oldest = store.get(&ids[0]).unwrap();
        assert!(oldest.payload_trimmed);
        let MonitorEventData::LlmCall { request_body, .. } = &oldest.data else {
            panic!("expected an LLM call");
        };
        assert!(request_body["prompt"].as_str().unwrap().len() < 1_000);

        // The newest event is kept whole.
        let newest = store.get(ids.last().unwrap()).unwrap();
        assert!(!newest.payload_trimmed);

        // List previews were captured before the trim.
        let listed = store.list(0, 100, None);
        let summary = listed.events.iter().find(|e| e.id == ids[0]).unwrap();
        assert!(summary.payload_trimmed);
        let untrimmed = to_summary(&newest);
        assert_eq!(summary.question.len(), untrimmed.question.len());
    }

    #[test]
    fn test_count_limit_still_drops_trimmed_events() {
        let store = MonitorEventStore::with_limits(5, 100_000);
        let first = push_finished_bulky(&store, 20_000);
        for _ in 0..5 {
            push_finished_bulky(&store, 20_000);
        }
        assert_eq!(store.stats().total_events, 5);
        assert!(store.get(&first).is_none());
    }

    #[test]
    fn test_in_progress_events_are_not_trimmed() {
        let store = MonitorEventStore::with_limits(100, 30_000);
        let pending = push_bulky(&store, 20_000);
        let finished = push_finished_bulky(&store, 20_000);
        // The pending event cannot be trimmed, so it is the one evicted.
        assert!(store.get(&pending).is_none());
        assert!(store.get(&finished).is_some());
    }

    #[test]
    fn test_update_after_trim_refreshes_previews() {
        let store = MonitorEventStore::with_limits(100, 60_000);
        let old = push_finished_bulky(&store, 20_000);
        push_finished_bulky(&store, 20_000);
        assert!(store.get(&old).unwrap().payload_trimmed);

        store.update(&old, |e| {
            if let MonitorEventData::LlmCall {
                content_preview, ..
            } = &mut e.data
            {
                *content_preview = Some("updated".into());
            }
        });
        let summary = store.list(0, 10, None);
        assert!(summary.events.iter().any(|e| e.id == old));
    }

    #[test]
    fn test_last_request_at_tracks_client_requests_only() {
        let store = make_store(10);
        let created = store.last_request_at();
        std::thread::sleep(std::time::Duration::from_millis(5));
        store.push(
            MonitorEventType::McpServerEvent,
            None,
            None,
            None,
            MonitorEventData::McpServerEvent {
                server_id: "s".into(),
                server_name: None,
                action: "health_changed".into(),
                message: "ok".into(),
            },
            EventStatus::Complete,
            None,
        );
        assert_eq!(store.last_request_at(), created);

        std::thread::sleep(std::time::Duration::from_millis(5));
        let id = push_bulky(&store, 10);
        let started = store.last_request_at();
        assert!(started > created);

        std::thread::sleep(std::time::Duration::from_millis(5));
        store.update(&id, |e| e.status = EventStatus::Complete);
        assert!(store.last_request_at() > started);
    }

    #[test]
    fn test_byte_budget_keeps_the_newest_oversized_event() {
        let store = MonitorEventStore::with_limits(100, 1_000);
        let id = push_bulky(&store, 100_000);

        // The event blows the whole budget on its own, but dropping it would
        // hide exactly the traffic worth inspecting.
        assert_eq!(store.stats().total_events, 1);
        assert!(store.get(&id).is_some());
    }

    #[test]
    fn test_total_bytes_tracks_clear_and_eviction() {
        let store = MonitorEventStore::with_limits(100, 10 * 1024 * 1024);
        push_bulky(&store, 10_000);
        let after_one = store.stats().total_bytes;
        assert!(after_one >= 10_000);

        push_bulky(&store, 10_000);
        assert!(store.stats().total_bytes >= after_one * 2);

        store.clear();
        assert_eq!(store.stats().total_bytes, 0);
    }

    #[test]
    fn test_update_recharges_the_budget() {
        let store = MonitorEventStore::with_limits(100, 10 * 1024 * 1024);
        let id = push_bulky(&store, 100);
        let before = store.stats().total_bytes;

        store.update(&id, |e| {
            if let MonitorEventData::LlmCall { response_body, .. } = &mut e.data {
                *response_body = Some(serde_json::json!({"text": "y".repeat(50_000)}));
            }
        });

        assert!(store.stats().total_bytes >= before + 50_000);
    }

    #[test]
    fn test_update_growth_can_trigger_eviction() {
        let store = MonitorEventStore::with_limits(100, 40_000);
        let old = push_bulky(&store, 100);
        let newest = push_bulky(&store, 100);

        // Completing the newest event pushes the buffer over budget; the older
        // event is the one that goes.
        store.update(&newest, |e| {
            if let MonitorEventData::LlmCall { response_body, .. } = &mut e.data {
                *response_body = Some(serde_json::json!({"text": "y".repeat(60_000)}));
            }
        });

        assert!(store.get(&old).is_none());
        assert!(store.get(&newest).is_some());
    }

    #[test]
    fn test_set_max_bytes_shrink_evicts_immediately() {
        let store = MonitorEventStore::with_limits(100, 10 * 1024 * 1024);
        for _ in 0..10 {
            push_bulky(&store, 10_000);
        }
        assert_eq!(store.stats().total_events, 10);

        store.set_max_bytes(30_000);
        let stats = store.stats();
        assert!(stats.total_events < 10);
        assert!(stats.total_bytes <= 30_000);
    }

    #[test]
    fn test_stats_by_type() {
        let store = make_store(100);
        push_llm_call(&store, "gpt-4");
        push_llm_call(&store, "gpt-4");
        store.push(
            MonitorEventType::McpToolCall,
            None,
            None,
            None,
            MonitorEventData::McpToolCall {
                tool_name: "test".to_string(),
                server_id: "srv".to_string(),
                server_name: None,
                arguments: serde_json::json!({}),
                firewall_action: None,
                latency_ms: None,
                success: None,
                response_preview: None,
                error: None,
            },
            EventStatus::Pending,
            None,
        );

        let stats = store.stats();
        assert_eq!(stats.total_events, 3);
        assert_eq!(
            stats.events_by_type.get(&MonitorEventType::LlmCall),
            Some(&2)
        );
        assert_eq!(
            stats.events_by_type.get(&MonitorEventType::McpToolCall),
            Some(&1)
        );
    }

    fn push_mcp_call(store: &MonitorEventStore, status: EventStatus) -> String {
        store.push(
            MonitorEventType::McpToolCall,
            None,
            None,
            None,
            MonitorEventData::McpToolCall {
                tool_name: "search".to_string(),
                server_id: "srv-1".to_string(),
                server_name: None,
                arguments: serde_json::json!({}),
                firewall_action: None,
                latency_ms: None,
                success: None,
                response_preview: None,
                error: None,
            },
            status,
            None,
        )
    }

    /// The in-progress count follows pushes, completions, evictions and
    /// clears, ignores internal steps and duplicate hops, and reaches the
    /// listener in order.
    #[test]
    fn test_pending_request_count_and_listener() {
        let store = make_store(3);
        let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let sink = seen.clone();
        store.set_pending_listener(move |n| sink.lock().push(n));

        let llm = push_llm_call(&store, "gpt-4");
        let mcp = push_mcp_call(&store, EventStatus::Pending);
        push_mcp_call(&store, EventStatus::Complete);
        assert_eq!(store.pending_requests(), 2);

        // A duplicate hop is not a request of its own.
        store.update(&llm, |event| {
            if let MonitorEventData::LlmCall { duplicate_hop, .. } = &mut event.data {
                *duplicate_hop = Some(2);
            }
        });
        assert_eq!(store.pending_requests(), 1);
        store.update(&llm, |event| {
            if let MonitorEventData::LlmCall { duplicate_hop, .. } = &mut event.data {
                *duplicate_hop = None;
            }
        });
        store.update(&mcp, |event| event.status = EventStatus::Error);
        assert_eq!(store.pending_requests(), 1);

        // Evicting a running request drops it from the count.
        push_mcp_call(&store, EventStatus::Complete);
        push_mcp_call(&store, EventStatus::Complete);
        assert_eq!(store.pending_requests(), 0);

        push_mcp_call(&store, EventStatus::Pending);
        store.clear();
        assert_eq!(store.pending_requests(), 0);

        assert_eq!(*seen.lock(), vec![0, 1, 2, 1, 2, 1, 0, 1, 0]);
    }
}
