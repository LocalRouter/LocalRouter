use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use chrono::Utc;
use parking_lot::RwLock;
use uuid::Uuid;

use crate::size::event_size;
use crate::summary::to_summary;
use crate::types::*;

/// Callback type for emitting Tauri events.
pub type EventEmitter = Arc<dyn Fn(&str, String) + Send + Sync>;

/// Default ceiling on the memory the store may hold.
///
/// Events keep whole request and response bodies, so the event count alone does
/// not bound memory: a long-context coding-agent exchange is megabytes, and a
/// few hundred of them run to gigabytes. Whichever limit is reached first —
/// this budget or the event count — evicts the oldest events.
pub const DEFAULT_MAX_BYTES: usize = 128 * 1024 * 1024;

/// One stored event plus the approximate heap cost charged for it, kept
/// alongside so eviction never has to re-walk a body it is about to drop.
struct Stored {
    event: MonitorEvent,
    size: usize,
}

/// The buffer and its running byte total, under one lock so they cannot drift.
struct Events {
    queue: VecDeque<Stored>,
    total_bytes: usize,
}

/// In-memory ring buffer for monitor events, bounded by both event count and
/// approximate memory footprint.
pub struct MonitorEventStore {
    events: RwLock<Events>,
    max_capacity: AtomicUsize,
    max_bytes: AtomicUsize,
    next_sequence: AtomicU64,
    emitter: RwLock<Option<EventEmitter>>,
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
            }),
            max_capacity: AtomicUsize::new(max_capacity.max(1)),
            max_bytes: AtomicUsize::new(max_bytes.max(1)),
            next_sequence: AtomicU64::new(1),
            emitter: RwLock::new(None),
        }
    }

    /// Drop the oldest events until the buffer fits both limits.
    ///
    /// The newest event is always kept, even when it alone exceeds the budget:
    /// evicting it would make a single oversized exchange impossible to inspect,
    /// which is exactly the traffic a user opens the monitor to look at.
    fn evict(&self, events: &mut Events) {
        let max_capacity = self.max_capacity.load(Ordering::Relaxed);
        let max_bytes = self.max_bytes.load(Ordering::Relaxed);

        while events.queue.len() > max_capacity
            || (events.total_bytes > max_bytes && events.queue.len() > 1)
        {
            match events.queue.pop_front() {
                Some(evicted) => {
                    events.total_bytes = events.total_bytes.saturating_sub(evicted.size)
                }
                None => break,
            }
        }
    }

    /// Set the event emitter callback (typically wired to Tauri's emit).
    pub fn set_emitter<F: Fn(&str, String) + Send + Sync + 'static>(&self, emitter: F) {
        *self.emitter.write() = Some(Arc::new(emitter));
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

        let event = MonitorEvent {
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

        let summary = to_summary(&event);
        let size = event_size(&event);

        {
            let mut events = self.events.write();
            events.total_bytes += size;
            events.queue.push_back(Stored { event, size });
            self.evict(&mut events);
        }

        // Emit lightweight notification
        if let Some(emitter) = self.emitter.read().as_ref() {
            if let Ok(payload) = serde_json::to_string(&summary) {
                emitter("monitor-event-created", payload);
            }
        }

        id
    }

    /// Update an existing event (e.g., combined event completion).
    /// Returns true if the event was found and updated.
    pub fn update<F>(&self, id: &str, updater: F) -> bool
    where
        F: FnOnce(&mut MonitorEvent),
    {
        let mut events = self.events.write();
        if let Some(stored) = events.queue.iter_mut().rev().find(|s| s.event.id == id) {
            updater(&mut stored.event);
            let updated_summary = to_summary(&stored.event);

            // The response half is usually where the bulk of an LLM event
            // arrives, so re-charge the entry and re-check the budget.
            let old_size = stored.size;
            let new_size = event_size(&stored.event);
            stored.size = new_size;
            events.total_bytes = events.total_bytes + new_size - old_size;
            self.evict(&mut events);
            drop(events);

            if let Some(emitter) = self.emitter.read().as_ref() {
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

        let filtered: Vec<&MonitorEvent> = events
            .queue
            .iter()
            .rev() // newest first
            .map(|s| &s.event)
            .filter(|e| match_filter(e, filter))
            .collect();

        let total = filtered.len();
        let page: Vec<MonitorEventSummary> = filtered
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(to_summary)
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
        events.queue.clear();
        events.total_bytes = 0;
    }

    /// Update the maximum event count. If the new capacity is smaller,
    /// excess old events are evicted immediately.
    pub fn set_max_capacity(&self, cap: usize) {
        self.max_capacity.store(cap.max(1), Ordering::Relaxed);
        let mut events = self.events.write();
        self.evict(&mut events);
    }

    /// Update the memory budget. If the new budget is smaller, old events are
    /// evicted immediately until the buffer fits.
    pub fn set_max_bytes(&self, max_bytes: usize) {
        self.max_bytes.store(max_bytes.max(1), Ordering::Relaxed);
        let mut events = self.events.write();
        self.evict(&mut events);
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
        if !types.is_empty() && !types.contains(&event.event_type) {
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
            let summary = crate::summary::generate_summary(event);
            let search_lower = search.to_lowercase();
            let trace_match = matches!(
                &event.data,
                MonitorEventData::LlmCall { trace_id: Some(t), .. } if t.to_lowercase().contains(&search_lower)
            );
            if !summary.to_lowercase().contains(&search_lower) && !trace_match {
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
    fn test_push_and_get() {
        let store = make_store(100);
        let id = push_llm_call(&store, "gpt-4");
        let event = store.get(&id).unwrap();
        assert_eq!(event.id, id);
        assert_eq!(event.event_type, MonitorEventType::LlmCall);
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
}
