use std::sync::Arc;
use std::time::Instant;

use crate::store::MonitorEventStore;
use crate::types::{EventStatus, MonitorEventData, MonitorEventType};

/// RAII guard that ensures a combined monitor event is completed.
///
/// If the guard is dropped without being explicitly defused (via `defuse()`),
/// it automatically marks the event as `Error` with a descriptive message.
/// This prevents events from staying in `Pending` state forever when
/// early returns or errors skip the explicit completion call.
///
/// Call `set_early_error()` before returning an error to capture the actual
/// HTTP status code and error message on the monitor event. If not called,
/// the guard falls back to a generic "Request failed" message on drop.
pub struct MonitorEventGuard {
    store: Arc<MonitorEventStore>,
    event_id: String,
    event_type: MonitorEventType,
    created_at: Instant,
    completed: bool,
    /// Captured error context from early returns (status_code, error_message).
    /// Set via `set_early_error()` before the guard drops.
    early_error: Option<(u16, String)>,
}

impl MonitorEventGuard {
    /// Create a new guard for a pending monitor event.
    pub fn new(
        store: Arc<MonitorEventStore>,
        event_id: String,
        event_type: MonitorEventType,
    ) -> Self {
        Self {
            store,
            event_id,
            event_type,
            created_at: Instant::now(),
            completed: false,
            early_error: None,
        }
    }

    /// Get the event ID.
    pub fn event_id(&self) -> &str {
        &self.event_id
    }

    /// Capture error context from an early return, so the guard's `Drop` can
    /// record the actual HTTP status code and error message instead of a generic fallback.
    pub fn set_early_error(&mut self, status_code: u16, error: impl Into<String>) {
        self.early_error = Some((status_code, error.into()));
    }

    /// Defuse the guard, returning the event ID.
    ///
    /// After calling this, the guard will NOT auto-complete the event on drop.
    /// Use this when handing off the event ID to a spawned task that manages
    /// its own completion (e.g., streaming response handlers).
    pub fn defuse(mut self) -> String {
        self.completed = true;
        std::mem::take(&mut self.event_id)
    }
}

impl Drop for MonitorEventGuard {
    fn drop(&mut self) {
        if self.completed {
            return;
        }

        let duration_ms = self.created_at.elapsed().as_millis() as u64;
        let event_type_label = self.event_type.label();

        // Use captured error context if available, otherwise fall back to generic message
        let (err_status, err_msg) = self
            .early_error
            .take()
            .unwrap_or_else(|| (0, format!("{} failed", event_type_label)));

        self.store.update(&self.event_id, |event| {
            event.status = EventStatus::Error;
            event.duration_ms = Some(duration_ms);

            // Set error message (and status_code for LlmCall) based on event type
            match &mut event.data {
                MonitorEventData::LlmCall {
                    error, status_code, ..
                } => {
                    if err_status > 0 {
                        *status_code = Some(err_status);
                    }
                    *error = Some(err_msg.clone());
                }
                MonitorEventData::McpToolCall { error, .. } => {
                    *error = Some(err_msg.clone());
                }
                MonitorEventData::McpResourceRead { error, .. } => {
                    *error = Some(err_msg.clone());
                }
                MonitorEventData::McpPromptGet { error, .. } => {
                    *error = Some(err_msg.clone());
                }
                _ => {}
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> Arc<MonitorEventStore> {
        Arc::new(MonitorEventStore::new(100))
    }

    fn push_pending_llm_call(store: &MonitorEventStore) -> String {
        store.push(
            MonitorEventType::LlmCall,
            Some("client-1".to_string()),
            Some("Test Client".to_string()),
            None,
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
            },
            EventStatus::Pending,
            None,
        )
    }

    #[test]
    fn test_guard_drop_marks_event_as_error() {
        let store = make_store();
        let event_id = push_pending_llm_call(&store);

        // Verify initial state is Pending
        let event = store.get(&event_id).unwrap();
        assert_eq!(event.status, EventStatus::Pending);

        // Create guard and drop it without defusing
        {
            let _guard =
                MonitorEventGuard::new(store.clone(), event_id.clone(), MonitorEventType::LlmCall);
        } // guard dropped here

        // Event should now be Error
        let event = store.get(&event_id).unwrap();
        assert_eq!(event.status, EventStatus::Error);
        assert!(event.duration_ms.is_some());

        // Check error message was set (generic fallback when no early_error captured)
        if let MonitorEventData::LlmCall { error, .. } = &event.data {
            assert!(error.is_some());
            assert!(error.as_ref().unwrap().contains("failed"));
        } else {
            panic!("Expected LlmCall data");
        }
    }

    #[test]
    fn test_guard_drop_with_early_error_captures_context() {
        let store = make_store();
        let event_id = push_pending_llm_call(&store);

        // Create guard, set early error context, then drop
        {
            let mut guard =
                MonitorEventGuard::new(store.clone(), event_id.clone(), MonitorEventType::LlmCall);
            guard.set_early_error(404, "Model not found: gpt-5");
        } // guard dropped here

        // Event should be Error with captured context
        let event = store.get(&event_id).unwrap();
        assert_eq!(event.status, EventStatus::Error);
        assert!(event.duration_ms.is_some());

        if let MonitorEventData::LlmCall {
            error, status_code, ..
        } = &event.data
        {
            assert_eq!(*status_code, Some(404));
            assert_eq!(error.as_deref(), Some("Model not found: gpt-5"));
        } else {
            panic!("Expected LlmCall data");
        }
    }

    #[test]
    fn test_guard_defuse_prevents_auto_error() {
        let store = make_store();
        let event_id = push_pending_llm_call(&store);

        // Create guard and defuse it
        let guard =
            MonitorEventGuard::new(store.clone(), event_id.clone(), MonitorEventType::LlmCall);
        let returned_id = guard.defuse();

        assert_eq!(returned_id, event_id);

        // Event should still be Pending (not auto-errored)
        let event = store.get(&event_id).unwrap();
        assert_eq!(event.status, EventStatus::Pending);
    }
}
