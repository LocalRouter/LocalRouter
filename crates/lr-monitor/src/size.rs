//! Approximate heap-size accounting for monitor events.
//!
//! The store keeps whole request and response bodies so a call stays
//! inspectable after it finished. A single proxied coding-agent exchange can
//! carry several megabytes of JSON, so a count-only capacity says nothing about
//! how much memory the store is holding — 1000 events is a few megabytes of
//! MCP traffic or well over a gigabyte of long-context LLM calls.
//!
//! These estimates are what the byte budget in [`crate::store`] evicts against.
//! They only have to track the real footprint closely enough to keep the store
//! inside its budget, so they count the parts that actually scale — string
//! payloads and JSON trees — and approximate the fixed per-node overhead.

use serde_json::Value;

use crate::types::*;

/// Size of a `serde_json::Value` slot, paid per array element and map entry.
const VALUE_SIZE: usize = std::mem::size_of::<Value>();

/// Size of a `String` header, paid per owned string regardless of its content.
const STRING_SIZE: usize = std::mem::size_of::<String>();

/// Approximate per-entry cost of `serde_json`'s object map: the key `String`
/// header plus the map's own node bookkeeping.
const MAP_ENTRY_OVERHEAD: usize = STRING_SIZE + 32;

/// Fixed cost of an event that carries no payload: the `MonitorEvent` itself
/// plus the deque slot and id/timestamp allocations it always has.
const EVENT_BASE: usize = std::mem::size_of::<MonitorEvent>() + 128;

/// Approximate heap bytes held by one stored event.
pub fn event_size(event: &MonitorEvent) -> usize {
    EVENT_BASE
        + opt_str(&event.client_id)
        + opt_str(&event.client_name)
        + opt_str(&event.session_id)
        + data_size(&event.data)
}

/// Approximate heap bytes held by a JSON tree.
pub fn json_size(value: &Value) -> usize {
    match value {
        // Inline in the `Value` slot already counted by the parent.
        Value::Null | Value::Bool(_) | Value::Number(_) => 0,
        Value::String(s) => s.len(),
        Value::Array(items) => {
            items.len() * VALUE_SIZE + items.iter().map(json_size).sum::<usize>()
        }
        Value::Object(map) => map
            .iter()
            .map(|(key, val)| MAP_ENTRY_OVERHEAD + key.len() + VALUE_SIZE + json_size(val))
            .sum(),
    }
}

fn str_size(s: &str) -> usize {
    STRING_SIZE + s.len()
}

fn opt_str(s: &Option<String>) -> usize {
    s.as_deref().map_or(0, str_size)
}

fn opt_json(v: &Option<Value>) -> usize {
    v.as_ref().map_or(0, json_size)
}

fn strs(v: &[String]) -> usize {
    v.iter().map(|s| str_size(s)).sum()
}

fn opt_strs(v: &Option<Vec<String>>) -> usize {
    v.as_deref().map_or(0, strs)
}

/// Approximate heap bytes held by an event's payload.
///
/// The match is exhaustive over the variants so a new event type has to be
/// weighed here rather than silently costing nothing; within a variant only the
/// fields that can grow are summed, the scalars are already in [`EVENT_BASE`].
fn data_size(data: &MonitorEventData) -> usize {
    match data {
        MonitorEventData::LlmCall {
            endpoint,
            model,
            request_body,
            transformed_body,
            transformations_applied,
            provider,
            finish_reason,
            content_preview,
            response_body,
            raw_request,
            raw_response,
            error,
            routing_info,
            trace_id,
            ..
        } => {
            str_size(endpoint)
                + str_size(model)
                + json_size(request_body)
                + opt_json(transformed_body)
                + opt_strs(transformations_applied)
                + opt_str(provider)
                + opt_str(finish_reason)
                + opt_str(content_preview)
                + opt_json(response_body)
                + opt_str(raw_request)
                + opt_str(raw_response)
                + opt_str(error)
                + routing_info.as_ref().map_or(0, routing_info_size)
                + opt_str(trace_id)
        }
        MonitorEventData::McpToolCall {
            tool_name,
            server_id,
            server_name,
            arguments,
            firewall_action,
            response_preview,
            error,
            ..
        } => {
            str_size(tool_name)
                + str_size(server_id)
                + opt_str(server_name)
                + json_size(arguments)
                + opt_str(firewall_action)
                + opt_str(response_preview)
                + opt_str(error)
        }
        MonitorEventData::McpResourceRead {
            uri,
            server_id,
            server_name,
            content_preview,
            error,
            ..
        } => {
            str_size(uri)
                + str_size(server_id)
                + opt_str(server_name)
                + opt_str(content_preview)
                + opt_str(error)
        }
        MonitorEventData::McpPromptGet {
            prompt_name,
            server_id,
            server_name,
            arguments,
            content_preview,
            error,
            ..
        } => {
            str_size(prompt_name)
                + str_size(server_id)
                + opt_str(server_name)
                + json_size(arguments)
                + opt_str(content_preview)
                + opt_str(error)
        }
        MonitorEventData::McpElicitation {
            server_id,
            server_name,
            message,
            schema,
            action,
            content,
            ..
        } => {
            str_size(server_id)
                + opt_str(server_name)
                + str_size(message)
                + json_size(schema)
                + opt_str(action)
                + opt_json(content)
        }
        MonitorEventData::McpSampling {
            server_id,
            server_name,
            model_hint,
            action,
            model_used,
            content_preview,
            ..
        } => {
            str_size(server_id)
                + opt_str(server_name)
                + opt_str(model_hint)
                + opt_str(action)
                + opt_str(model_used)
                + opt_str(content_preview)
        }
        MonitorEventData::GuardrailScan {
            direction,
            text_preview,
            models_used,
            result,
            flagged_categories,
            action_taken,
            ..
        }
        | MonitorEventData::GuardrailResponseScan {
            direction,
            text_preview,
            models_used,
            result,
            flagged_categories,
            action_taken,
            ..
        } => {
            str_size(direction)
                + str_size(text_preview)
                + strs(models_used)
                + opt_str(result)
                + flagged_categories.as_deref().map_or(0, |cats| {
                    cats.iter()
                        .map(|c| str_size(&c.category) + str_size(&c.action) + 16)
                        .sum()
                })
                + opt_str(action_taken)
        }
        MonitorEventData::SecretScan {
            text_preview,
            findings,
            action_taken,
            ..
        } => str_size(text_preview) + opt_json(findings) + opt_str(action_taken),
        MonitorEventData::RouteLlmClassify {
            original_model,
            selected_tier,
            routed_model,
            ..
        } => str_size(original_model) + opt_str(selected_tier) + opt_str(routed_model),
        MonitorEventData::RoutingDecision {
            routing_type,
            original_model,
            final_model,
            candidate_models,
            firewall_action,
        } => {
            str_size(routing_type)
                + str_size(original_model)
                + str_size(final_model)
                + opt_strs(candidate_models)
                + opt_str(firewall_action)
        }
        MonitorEventData::AuthError {
            error_type,
            endpoint,
            message,
            ..
        } => str_size(error_type) + str_size(endpoint) + str_size(message),
        MonitorEventData::AccessDenied {
            reason,
            endpoint,
            message,
            ..
        }
        | MonitorEventData::RateLimitEvent {
            reason,
            endpoint,
            message,
            ..
        } => str_size(reason) + str_size(endpoint) + str_size(message),
        MonitorEventData::ValidationError {
            endpoint,
            field,
            message,
            ..
        } => str_size(endpoint) + opt_str(field) + str_size(message),
        MonitorEventData::McpServerEvent {
            server_id,
            server_name,
            action,
            message,
        } => str_size(server_id) + opt_str(server_name) + str_size(action) + str_size(message),
        MonitorEventData::OAuthEvent {
            action,
            client_id_hint,
            message,
            ..
        } => str_size(action) + opt_str(client_id_hint) + str_size(message),
        MonitorEventData::InternalError {
            error_type,
            message,
            ..
        } => str_size(error_type) + str_size(message),
        MonitorEventData::ModerationEvent {
            reason, message, ..
        } => str_size(reason) + str_size(message),
        MonitorEventData::ConnectionError {
            transport,
            action,
            message,
        } => str_size(transport) + str_size(action) + str_size(message),
        MonitorEventData::PromptCompression { method, .. } => str_size(method),
        MonitorEventData::MemoryCompaction {
            session_id,
            model,
            transcript_path,
            request_body,
            summary_path,
            finish_reason,
            response_body,
            content_preview,
            error,
            ..
        } => {
            str_size(session_id)
                + str_size(model)
                + opt_str(transcript_path)
                + opt_json(request_body)
                + opt_str(summary_path)
                + opt_str(finish_reason)
                + opt_json(response_body)
                + opt_str(content_preview)
                + opt_str(error)
        }
        MonitorEventData::FirewallDecision {
            firewall_type,
            item_name,
            action,
            duration,
        } => str_size(firewall_type) + str_size(item_name) + str_size(action) + opt_str(duration),
        MonitorEventData::SseConnection { session_id, action } => {
            str_size(session_id) + str_size(action)
        }
        MonitorEventData::ProxyPassthrough {
            host,
            method,
            path,
            note,
            error,
            ..
        } => str_size(host) + opt_str(method) + opt_str(path) + str_size(note) + opt_str(error),
    }
}

fn routing_info_size(info: &AutoRoutingInfo) -> usize {
    std::mem::size_of::<AutoRoutingInfo>()
        + opt_str(&info.routellm_tier)
        + strs(&info.candidate_models)
        + info
            .attempts
            .iter()
            .map(|a| {
                std::mem::size_of::<RoutingAttempt>()
                    + str_size(&a.provider)
                    + str_size(&a.model)
                    + str_size(&a.outcome)
                    + opt_str(&a.error)
            })
            .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn llm_event(request_body: Value) -> MonitorEvent {
        MonitorEvent {
            id: "mon-1".to_string(),
            sequence: 1,
            timestamp: chrono::Utc::now(),
            event_type: MonitorEventType::LlmCall,
            client_id: None,
            client_name: None,
            session_id: None,
            data: MonitorEventData::LlmCall {
                endpoint: "/v1/chat/completions".to_string(),
                model: "gpt-4".to_string(),
                stream: false,
                message_count: 1,
                has_tools: false,
                tool_count: 0,
                request_body,
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
            status: EventStatus::Pending,
            duration_ms: None,
        }
    }

    #[test]
    fn json_size_counts_string_payloads() {
        let small = json_size(&serde_json::json!({"a": "x"}));
        let large = json_size(&serde_json::json!({"a": "x".repeat(10_000)}));
        assert_eq!(large, small + 9_999);
    }

    #[test]
    fn json_size_ignores_scalars() {
        assert_eq!(json_size(&Value::Null), 0);
        assert_eq!(json_size(&serde_json::json!(42)), 0);
        assert_eq!(json_size(&serde_json::json!(true)), 0);
    }

    #[test]
    fn json_size_recurses_into_nested_structures() {
        let nested = serde_json::json!({
            "messages": [
                {"role": "user", "content": "y".repeat(5_000)},
                {"role": "assistant", "content": "z".repeat(5_000)},
            ]
        });
        assert!(json_size(&nested) >= 10_000);
    }

    #[test]
    fn event_size_tracks_body_growth() {
        let empty = event_size(&llm_event(Value::Null));
        let big = event_size(&llm_event(
            serde_json::json!({"prompt": "p".repeat(100_000)}),
        ));
        assert!(empty >= EVENT_BASE);
        assert!(big >= empty + 100_000);
    }

    #[test]
    fn event_size_counts_raw_capture_strings() {
        let mut event = llm_event(Value::Null);
        let base = event_size(&event);
        if let MonitorEventData::LlmCall {
            raw_request,
            raw_response,
            ..
        } = &mut event.data
        {
            *raw_request = Some("q".repeat(64 * 1024));
            *raw_response = Some("r".repeat(64 * 1024));
        }
        assert!(event_size(&event) >= base + 128 * 1024);
    }

    #[test]
    fn event_size_is_small_for_scalar_only_events() {
        let event = MonitorEvent {
            id: "mon-2".to_string(),
            sequence: 2,
            timestamp: chrono::Utc::now(),
            event_type: MonitorEventType::SseConnection,
            client_id: None,
            client_name: None,
            session_id: None,
            data: MonitorEventData::SseConnection {
                session_id: "sess".to_string(),
                action: "opened".to_string(),
            },
            status: EventStatus::Complete,
            duration_ms: None,
        };
        assert!(event_size(&event) < 4096);
    }
}
