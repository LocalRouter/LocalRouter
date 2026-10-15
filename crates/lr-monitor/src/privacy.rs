//! Payload redaction keeps the event's typed lifecycle and usage metadata.
use crate::types::{MonitorEvent, MonitorEventData};
use serde_json::Value;
const HIDDEN: &str = "[payload not retained]";

pub(crate) fn redact(event: &mut MonitorEvent) {
    event.payload_redacted = true;
    event.client_name = None;
    event.session_id = None;
    let Ok(mut value) = serde_json::to_value(&event.data) else {
        return;
    };
    if let Some(object) = value.as_object_mut() {
        for (key, value) in object {
            // Closed metadata allowlist. Bodies, previews, findings, schemas,
            // raw traffic, routing explanations and arbitrary errors are omitted.
            let keep = matches!(
                key.as_str(),
                "type"
                    | "endpoint"
                    | "model"
                    | "requested_model"
                    | "provider"
                    | "source"
                    | "protocol"
                    | "client_api"
                    | "upstream_api"
                    | "stream"
                    | "streamed"
                    | "message_count"
                    | "has_tools"
                    | "tool_count"
                    | "status_code"
                    | "input_tokens"
                    | "output_tokens"
                    | "total_tokens"
                    | "reasoning_tokens"
                    | "cached_input_tokens"
                    | "cost_usd"
                    | "latency_ms"
                    | "duration_ms"
                    | "duplicate_hop"
                    | "success"
                    | "rules_count"
                    | "findings_count"
                    | "threshold"
                    | "win_rate"
                    | "original_tokens"
                    | "compressed_tokens"
                    | "reduction_percent"
                    | "transcript_bytes"
                    | "summary_bytes"
                    | "compression_ratio"
                    | "max_tokens"
                    | "retry_after_secs"
                    | "transport"
                    | "server_id"
                    | "direction"
                    | "mode"
                    | "port"
                    | "bytes_sent"
                    | "bytes_received"
            );
            if keep && !value.is_object() && !value.is_array() {
                continue;
            }
            *value = match value {
                Value::String(_) => Value::String(HIDDEN.into()),
                Value::Array(_) => Value::Array(vec![]),
                Value::Bool(_) => Value::Bool(false),
                Value::Number(_) => Value::from(0),
                _ => Value::Null,
            };
        }
    }
    event.data =
        serde_json::from_value(value).unwrap_or_else(|_| MonitorEventData::InternalError {
            error_type: "capture_policy".into(),
            message: HIDDEN.into(),
            status_code: 0,
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EventStatus, MonitorEventStore, MonitorEventType};
    #[test]
    fn security_and_mcp_payloads_are_removed_before_create_and_update_emission() {
        let store = MonitorEventStore::new(20);
        store.set_payload_policy(|_| true);
        let emissions = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let captured = emissions.clone();
        store.set_emitter(move |_, json| captured.lock().push(json));
        for (kind, data) in [
            (
                MonitorEventType::SecretScan,
                MonitorEventData::SecretScan {
                    text_preview: "sensitive-payload".into(),
                    rules_count: 7,
                    findings_count: Some(1),
                    findings: Some(serde_json::json!({"secret":"sensitive-payload"})),
                    action_taken: None,
                    latency_ms: Some(10),
                },
            ),
            (
                MonitorEventType::GuardrailScan,
                MonitorEventData::GuardrailScan {
                    direction: "request".into(),
                    text_preview: "sensitive-payload".into(),
                    models_used: vec!["sensitive-payload".into()],
                    result: None,
                    flagged_categories: None,
                    action_taken: None,
                    latency_ms: None,
                },
            ),
            (
                MonitorEventType::McpToolCall,
                MonitorEventData::McpToolCall {
                    tool_name: "sensitive-payload".into(),
                    server_id: "server".into(),
                    server_name: None,
                    arguments: serde_json::json!({"secret":"sensitive-payload"}),
                    firewall_action: None,
                    latency_ms: None,
                    success: None,
                    response_preview: Some("sensitive-payload".into()),
                    error: Some("sensitive-payload".into()),
                },
            ),
        ] {
            let id = store.push(
                kind,
                Some("client".into()),
                Some("sensitive-payload".into()),
                Some("sensitive-payload".into()),
                data,
                EventStatus::Pending,
                None,
            );
            store.update(&id, |event| event.status = EventStatus::Complete);
            let event = store.get(&id).unwrap();
            assert!(!matches!(
                event.data,
                MonitorEventData::InternalError { .. }
            ));
            assert!(!serde_json::to_string(&event)
                .unwrap()
                .contains("sensitive-payload"));
        }
        assert_eq!(emissions.lock().len(), 6);
        assert!(emissions
            .lock()
            .iter()
            .all(|json| !json.contains("sensitive-payload")));
    }
}
