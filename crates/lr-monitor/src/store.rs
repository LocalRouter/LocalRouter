use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::Utc;
use parking_lot::RwLock;
use uuid::Uuid;

use crate::summary::to_summary;
use crate::types::*;

/// Callback type for emitting Tauri events.
pub type EventEmitter = Arc<dyn Fn(&str, String) + Send + Sync>;

/// In-memory ring buffer for monitor events.
pub struct MonitorEventStore {
    events: RwLock<VecDeque<MonitorEvent>>,
    max_capacity: RwLock<usize>,
    next_sequence: AtomicU64,
    emitter: RwLock<Option<EventEmitter>>,
}

impl MonitorEventStore {
    pub fn new(max_capacity: usize) -> Self {
        Self {
            events: RwLock::new(VecDeque::with_capacity(max_capacity.min(2048))),
            max_capacity: RwLock::new(max_capacity),
            next_sequence: AtomicU64::new(1),
            emitter: RwLock::new(None),
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
    #[allow(clippy::too_many_arguments)]
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

        {
            let cap = *self.max_capacity.read();
            let mut events = self.events.write();
            while events.len() >= cap {
                events.pop_front();
            }
            events.push_back(event);
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
        if let Some(event) = events.iter_mut().rev().find(|e| e.id == id) {
            updater(event);
            let updated_summary = to_summary(event);
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
            .iter()
            .rev() // newest first
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
        events.iter().rev().find(|e| e.id == id).cloned()
    }

    /// Clear all events.
    pub fn clear(&self) {
        self.events.write().clear();
    }

    /// Update the maximum capacity. If the new capacity is smaller,
    /// excess old events are evicted immediately.
    pub fn set_max_capacity(&self, cap: usize) {
        let cap = cap.max(1); // minimum 1
        *self.max_capacity.write() = cap;
        let mut events = self.events.write();
        while events.len() > cap {
            events.pop_front();
        }
    }

    /// Get current store statistics.
    pub fn stats(&self) -> MonitorStats {
        let events = self.events.read();
        let mut by_type = std::collections::HashMap::new();
        for event in events.iter() {
            *by_type.entry(event.event_type).or_insert(0) += 1;
        }
        MonitorStats {
            total_events: events.len(),
            max_capacity: *self.max_capacity.read(),
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
            if !summary.to_lowercase().contains(&search_lower) {
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
