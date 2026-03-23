use crate::types::{EventStatus, MonitorEvent, MonitorEventData, MonitorEventSummary};

/// Generate a one-line summary string from event data for list display.
/// For combined events, the summary changes based on status (pending vs complete/error).
pub fn generate_summary(event: &MonitorEvent) -> String {
    match &event.data {
        // LLM Call (combined)
        MonitorEventData::LlmCall {
            endpoint,
            model,
            message_count,
            stream,
            provider,
            total_tokens,
            status_code,
            error,
            ..
        } => match event.status {
            EventStatus::Pending => {
                let stream_label = if *stream { " (stream)" } else { "" };
                format!(
                    "{} → {} ({} msgs{})",
                    endpoint, model, message_count, stream_label
                )
            }
            EventStatus::Complete => {
                let prov = provider.as_deref().unwrap_or("?");
                let tokens = total_tokens.unwrap_or(0);
                format!("{}/{} — {} tokens", prov, model, tokens)
            }
            EventStatus::Error => {
                let prov = provider.as_deref().unwrap_or("?");
                let code = status_code.unwrap_or(0);
                let err = error.as_deref().unwrap_or("unknown error");
                format!("{}/{} — HTTP {} {}", prov, model, code, truncate(err, 40))
            }
        },

        // MCP Tool Call
        MonitorEventData::McpToolCall {
            tool_name,
            success,
            error,
            ..
        } => match event.status {
            EventStatus::Pending => {
                format!("tools/call → {}", tool_name)
            }
            EventStatus::Complete => {
                let status = if success.unwrap_or(true) {
                    "OK"
                } else {
                    "Error"
                };
                format!("tools/call {} ({})", tool_name, status)
            }
            EventStatus::Error => {
                let err = error.as_deref().unwrap_or("failed");
                format!("tools/call {} — {}", tool_name, truncate(err, 40))
            }
        },

        // MCP Resource Read
        MonitorEventData::McpResourceRead {
            uri,
            success,
            error,
            ..
        } => match event.status {
            EventStatus::Pending => {
                format!("resources/read → {}", truncate(uri, 60))
            }
            EventStatus::Complete => {
                let status = if success.unwrap_or(true) {
                    "OK"
                } else {
                    "Error"
                };
                format!("resources/read {} ({})", truncate(uri, 50), status)
            }
            EventStatus::Error => {
                let err = error.as_deref().unwrap_or("failed");
                format!(
                    "resources/read {} — {}",
                    truncate(uri, 40),
                    truncate(err, 30)
                )
            }
        },

        // MCP Prompt Get
        MonitorEventData::McpPromptGet {
            prompt_name,
            success,
            error,
            ..
        } => match event.status {
            EventStatus::Pending => {
                format!("prompts/get → {}", prompt_name)
            }
            EventStatus::Complete => {
                let status = if success.unwrap_or(true) {
                    "OK"
                } else {
                    "Error"
                };
                format!("prompts/get {} ({})", prompt_name, status)
            }
            EventStatus::Error => {
                let err = error.as_deref().unwrap_or("failed");
                format!("prompts/get {} — {}", prompt_name, truncate(err, 40))
            }
        },

        // MCP Elicitation
        MonitorEventData::McpElicitation {
            message, action, ..
        } => match event.status {
            EventStatus::Pending => {
                format!("elicitation → {}", truncate(message, 60))
            }
            _ => {
                let act = action.as_deref().unwrap_or("?");
                format!("elicitation ← {}", act)
            }
        },

        // MCP Sampling
        MonitorEventData::McpSampling {
            message_count,
            model_hint,
            action,
            ..
        } => match event.status {
            EventStatus::Pending => {
                let model = model_hint.as_deref().unwrap_or("any");
                format!("sampling → {} msgs ({})", message_count, model)
            }
            _ => {
                let act = action.as_deref().unwrap_or("?");
                format!("sampling ← {}", act)
            }
        },

        // Guardrail Scan (input)
        MonitorEventData::GuardrailScan {
            direction,
            result,
            flagged_categories,
            ..
        } => match event.status {
            EventStatus::Pending => {
                format!("guardrail scan ({})", direction)
            }
            _ => {
                let res = result.as_deref().unwrap_or("?");
                if let Some(cats) = flagged_categories {
                    if !cats.is_empty() {
                        let cat_names: Vec<&str> =
                            cats.iter().map(|c| c.category.as_str()).collect();
                        return format!("guardrail: {} [{}]", res, cat_names.join(", "));
                    }
                }
                format!("guardrail: {}", res)
            }
        },

        // Guardrail Response Scan (output)
        MonitorEventData::GuardrailResponseScan {
            direction,
            result,
            flagged_categories,
            ..
        } => match event.status {
            EventStatus::Pending => {
                format!("response guardrail scan ({})", direction)
            }
            _ => {
                let res = result.as_deref().unwrap_or("?");
                if let Some(cats) = flagged_categories {
                    if !cats.is_empty() {
                        let cat_names: Vec<&str> =
                            cats.iter().map(|c| c.category.as_str()).collect();
                        return format!("response guardrail: {} [{}]", res, cat_names.join(", "));
                    }
                }
                format!("response guardrail: {}", res)
            }
        },

        // Secret Scan
        MonitorEventData::SecretScan {
            findings_count,
            action_taken,
            ..
        } => match event.status {
            EventStatus::Pending => "secret scan check".to_string(),
            _ => {
                let count = findings_count.unwrap_or(0);
                let action = action_taken.as_deref().unwrap_or("?");
                format!("secret scan: {} findings ({})", count, action)
            }
        },

        // RouteLLM Classify
        MonitorEventData::RouteLlmClassify {
            original_model,
            selected_tier,
            win_rate,
            ..
        } => match event.status {
            EventStatus::Pending => {
                format!("classify: {}", original_model)
            }
            _ => {
                let tier = selected_tier.as_deref().unwrap_or("?");
                let rate = win_rate.unwrap_or(0.0);
                format!("classified: {} (win_rate={:.2})", tier, rate)
            }
        },

        // ---- Standalone events (unchanged) ----
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

        MonitorEventData::RateLimitEvent {
            reason,
            status_code,
            ..
        } => {
            format!("HTTP {} — {}", status_code, reason)
        }

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

        MonitorEventData::McpServerEvent {
            server_id,
            action,
            message,
            ..
        } => {
            format!("{}: {} — {}", server_id, action, truncate(message, 50))
        }

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

        MonitorEventData::InternalError {
            error_type,
            status_code,
            message,
        } => {
            format!(
                "HTTP {} {}: {}",
                status_code,
                error_type,
                truncate(message, 50)
            )
        }

        MonitorEventData::ModerationEvent {
            reason,
            status_code,
            ..
        } => {
            format!("HTTP {} — {}", status_code, reason)
        }

        MonitorEventData::ConnectionError {
            transport,
            action,
            message,
        } => {
            format!("{} {} — {}", transport, action, truncate(message, 50))
        }

        MonitorEventData::PromptCompression {
            reduction_percent, ..
        } => {
            format!("compression: {:.1}% reduction", reduction_percent)
        }
        MonitorEventData::MemoryCompaction {
            session_id,
            model,
            transcript_bytes,
            summary_bytes,
            compression_ratio,
            error,
            ..
        } => match event.status {
            EventStatus::Pending => {
                format!("compacting session {} via {}", session_id, model)
            }
            EventStatus::Complete => {
                let summary_b = summary_bytes.unwrap_or(0);
                let ratio = compression_ratio.unwrap_or(0.0);
                format!(
                    "compacted {}: {}B \u{2192} {}B ({:.0}% reduction)",
                    session_id, transcript_bytes, summary_b, ratio
                )
            }
            EventStatus::Error => {
                let err = error.as_deref().unwrap_or("unknown error");
                format!(
                    "compaction failed for {}: {}",
                    session_id,
                    truncate(err, 40)
                )
            }
        },
        MonitorEventData::FirewallDecision {
            firewall_type,
            item_name,
            action,
            ..
        } => {
            format!("{} firewall: {} ({})", firewall_type, item_name, action)
        }
        MonitorEventData::SseConnection { action, session_id } => {
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
        session_id: event.session_id.clone(),
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
