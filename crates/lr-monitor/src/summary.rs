use crate::types::{MonitorEvent, MonitorEventData, MonitorEventSummary};

/// Generate a one-line summary string from event data for list display.
pub fn generate_summary(event: &MonitorEvent) -> String {
    match &event.data {
        // LLM
        MonitorEventData::LlmRequest {
            endpoint,
            model,
            message_count,
            stream,
            ..
        } => {
            let stream_label = if *stream { " (stream)" } else { "" };
            format!(
                "{} → {} ({} msgs{})",
                endpoint, model, message_count, stream_label
            )
        }
        MonitorEventData::LlmRequestTransformed {
            model,
            message_count,
            tool_count,
            transformations_applied,
            ..
        } => {
            let transforms = transformations_applied.join(", ");
            format!(
                "{} ({} msgs, {} tools) [{}]",
                model, message_count, tool_count, transforms
            )
        }
        MonitorEventData::LlmResponse {
            provider,
            model,
            total_tokens,
            ..
        } => {
            format!("{}/{} — {} tokens", provider, model, total_tokens)
        }
        MonitorEventData::LlmError {
            provider,
            model,
            status_code,
            ..
        } => {
            format!("{}/{} — HTTP {}", provider, model, status_code)
        }

        // MCP
        MonitorEventData::McpToolCall { tool_name, .. } => {
            format!("tools/call → {}", tool_name)
        }
        MonitorEventData::McpToolResponse {
            tool_name, success, ..
        } => {
            let status = if *success { "OK" } else { "Error" };
            format!("tools/call ← {} ({})", tool_name, status)
        }
        MonitorEventData::McpResourceRead { uri, .. } => {
            format!("resources/read → {}", truncate(uri, 60))
        }
        MonitorEventData::McpResourceResponse { uri, success, .. } => {
            let status = if *success { "OK" } else { "Error" };
            format!("resources/read ← {} ({})", truncate(uri, 50), status)
        }
        MonitorEventData::McpPromptGet { prompt_name, .. } => {
            format!("prompts/get → {}", prompt_name)
        }
        MonitorEventData::McpPromptResponse {
            prompt_name,
            success,
            ..
        } => {
            let status = if *success { "OK" } else { "Error" };
            format!("prompts/get ← {} ({})", prompt_name, status)
        }
        MonitorEventData::McpElicitationRequest { message, .. } => {
            format!("elicitation → {}", truncate(message, 60))
        }
        MonitorEventData::McpElicitationResponse { action, .. } => {
            format!("elicitation ← {}", action)
        }
        MonitorEventData::McpSamplingRequest {
            message_count,
            model_hint,
            ..
        } => {
            let model = model_hint.as_deref().unwrap_or("any");
            format!("sampling → {} msgs ({})", message_count, model)
        }
        MonitorEventData::McpSamplingResponse { action, .. } => {
            format!("sampling ← {}", action)
        }

        // Security
        MonitorEventData::GuardrailRequest { direction, .. } => {
            format!("guardrail scan ({})", direction)
        }
        MonitorEventData::GuardrailResponse {
            result,
            flagged_categories,
            ..
        } => {
            if flagged_categories.is_empty() {
                format!("guardrail: {}", result)
            } else {
                let cats: Vec<&str> = flagged_categories
                    .iter()
                    .map(|c| c.category.as_str())
                    .collect();
                format!("guardrail: {} [{}]", result, cats.join(", "))
            }
        }
        MonitorEventData::SecretScanRequest { .. } => "secret scan check".to_string(),
        MonitorEventData::SecretScanResponse {
            findings_count,
            action_taken,
            ..
        } => {
            format!("secret scan: {} findings ({})", findings_count, action_taken)
        }

        // Routing
        MonitorEventData::RouteLlmRequest {
            original_model, ..
        } => {
            format!("classify: {}", original_model)
        }
        MonitorEventData::RouteLlmResponse {
            selected_tier,
            win_rate,
            ..
        } => {
            format!("classified: {} (win_rate={:.2})", selected_tier, win_rate)
        }
        MonitorEventData::RoutingDecision {
            routing_type,
            original_model,
            final_model,
            ..
        } => {
            if original_model == final_model {
                format!("{}: {}", routing_type, final_model)
            } else {
                format!("{}: {} → {}", routing_type, original_model, final_model)
            }
        }

        // Auth & Access Control
        MonitorEventData::AuthError {
            error_type,
            endpoint,
            status_code,
            ..
        } => {
            format!("HTTP {} {} — {}", status_code, endpoint, error_type)
        }
        MonitorEventData::AccessDenied {
            reason,
            endpoint,
            status_code,
            ..
        } => {
            format!("HTTP {} {} — {}", status_code, endpoint, reason)
        }

        // Rate Limiting
        MonitorEventData::RateLimitEvent {
            reason,
            status_code,
            ..
        } => {
            format!("HTTP {} — {}", status_code, reason)
        }

        // Validation
        MonitorEventData::ValidationError {
            endpoint,
            field,
            message,
            ..
        } => {
            if let Some(f) = field {
                format!("{} — {} ({})", endpoint, message, f)
            } else {
                format!("{} — {}", endpoint, message)
            }
        }

        // MCP Server Health
        MonitorEventData::McpServerEvent {
            server_id,
            action,
            message,
            ..
        } => {
            format!("{}: {} — {}", server_id, action, truncate(message, 50))
        }

        // OAuth
        MonitorEventData::OAuthEvent {
            action,
            client_id_hint,
            status_code,
            ..
        } => {
            if let Some(cid) = client_id_hint {
                format!("HTTP {} {} ({})", status_code, action, truncate(cid, 16))
            } else {
                format!("HTTP {} {}", status_code, action)
            }
        }

        // Internal
        MonitorEventData::InternalError {
            error_type,
            status_code,
            message,
        } => {
            format!("HTTP {} {}: {}", status_code, error_type, truncate(message, 50))
        }

        // Moderation
        MonitorEventData::ModerationEvent {
            reason,
            status_code,
            ..
        } => {
            format!("HTTP {} — {}", status_code, reason)
        }

        // Connection
        MonitorEventData::ConnectionError {
            transport,
            action,
            message,
        } => {
            format!("{} {} — {}", transport, action, truncate(message, 50))
        }

        // Other
        MonitorEventData::PromptCompression {
            reduction_percent, ..
        } => {
            format!("compression: {:.1}% reduction", reduction_percent)
        }
        MonitorEventData::FirewallDecision {
            firewall_type,
            item_name,
            action,
            ..
        } => {
            format!("{} firewall: {} ({})", firewall_type, item_name, action)
        }
        MonitorEventData::SseConnection {
            action,
            session_id,
        } => {
            format!("SSE {}: {}", action, truncate(session_id, 16))
        }
    }
}

/// Create a summary from a full event.
pub fn to_summary(event: &MonitorEvent) -> MonitorEventSummary {
    MonitorEventSummary {
        id: event.id.clone(),
        sequence: event.sequence,
        timestamp: event.timestamp,
        event_type: event.event_type,
        client_id: event.client_id.clone(),
        client_name: event.client_name.clone(),
        status: event.status,
        duration_ms: event.duration_ms,
        summary: generate_summary(event),
    }
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}
