//! Small content-only previews for the Monitor table. Never ship full payloads
//! in list responses or live summary notifications.
use serde_json::Value;

use crate::types::MonitorEventData;

const MAX_PREVIEW_CHARS: usize = 1024;

fn single_line(text: &str) -> String {
    let mut chars = text.trim().chars().peekable();
    let mut result = String::new();
    let mut whitespace = false;
    for _ in 0..MAX_PREVIEW_CHARS {
        let Some(c) = chars.next() else { break };
        if c.is_whitespace() {
            whitespace = !result.is_empty();
        } else {
            if whitespace {
                result.push(' ');
                whitespace = false;
            }
            result.push(c);
        }
    }
    if chars.peek().is_some() {
        result.push_str("...");
    }
    result
}

/// Text blocks from OpenAI, Anthropic, Responses, and MCP. Non-text blocks are
/// described without copying image/audio bytes into a summary.
pub(crate) fn content(value: &Value) -> String {
    match value {
        Value::String(s) => single_line(s),
        Value::Array(items) => single_line(
            &items
                .iter()
                .map(content)
                .filter(|s| !s.is_empty())
                .take(16)
                .collect::<Vec<_>>()
                .join(" "),
        ),
        Value::Object(_) => {
            // Check the block kind before generic text/content fields: reasoning
            // and signed thinking must never become a table preview.
            match value.get("type").and_then(Value::as_str) {
                Some("thinking" | "redacted_thinking" | "reasoning") => return String::new(),
                Some("function_call_output" | "custom_tool_call_output") => {
                    return format!(
                        "[tool result] {}",
                        value.get("output").map(content).unwrap_or_default()
                    );
                }
                Some("tool_result") => {
                    return format!(
                        "[tool result] {}",
                        value.get("content").map(content).unwrap_or_default()
                    );
                }
                Some("tool_use" | "function_call" | "custom_tool_call") => {
                    return tool_call(value);
                }
                Some("image" | "image_url" | "input_image") => return "[image]".into(),
                Some("input_audio" | "audio") => return "[audio]".into(),
                Some("document" | "file" | "input_file") => return "[document]".into(),
                _ => {}
            }
            if let Some(text) = value.get("text").and_then(Value::as_str) {
                return single_line(text);
            }
            if let Some(body) = value.get("content") {
                let text = content(body);
                if !text.is_empty() {
                    return text;
                }
            }
            if let Some(calls) = value.get("tool_calls").and_then(Value::as_array) {
                return single_line(
                    &calls
                        .iter()
                        .take(16)
                        .map(|call| tool_call(call.get("function").unwrap_or(call)))
                        .collect::<Vec<_>>()
                        .join("; "),
                );
            }
            value
                .get("refusal")
                .and_then(Value::as_str)
                .map(single_line)
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

fn tool_call(value: &Value) -> String {
    single_line(&format!(
        "{}({})",
        value["name"].as_str().unwrap_or("tool"),
        value
            .get("input")
            .or_else(|| value.get("arguments"))
            .map(display_value)
            .unwrap_or_default()
    ))
}

fn display_value(value: &Value) -> String {
    value
        .as_str()
        .map(single_line)
        .unwrap_or_else(|| single_line(&value.to_string()))
}

pub(crate) fn request(body: &Value) -> String {
    if body["_truncated"] == true {
        return body["_monitor_preview"]["question"]
            .as_str()
            .map(single_line)
            .unwrap_or_default();
    }
    if let Some(questions) = body.get("questions").and_then(Value::as_object) {
        return single_line(
            &questions
                .iter()
                .map(|(id, q)| q["instructions"].as_str().unwrap_or(id))
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    if let Some(messages) = body
        .get("messages")
        .or_else(|| body.get("input"))
        .and_then(Value::as_array)
    {
        if let Some(message) = messages.iter().rev().find(|m| m["role"] == "user") {
            return content(message);
        }
        // Incremental tool turns may omit the original user message. Show
        // the actual tool input without relabelling a system/assistant prompt.
        if let Some(message) = messages.iter().rev().find(|m| {
            m["role"] == "tool"
                || matches!(
                    m["type"].as_str(),
                    Some("function_call_output" | "custom_tool_call_output")
                )
        }) {
            let text = content(message);
            return if message["role"] == "tool" {
                format!("[tool result] {text}")
            } else {
                text
            };
        }
        // Never show a system prompt as the user's question.
        if body.get("messages").is_some() || messages.iter().any(|m| m.get("role").is_some()) {
            return String::new();
        }
    }
    body.get("prompt")
        .or_else(|| body.get("input"))
        .map(content)
        .unwrap_or_default()
}

pub(crate) fn response(body: &Value) -> String {
    if body["_truncated"] == true {
        return body["_monitor_preview"]["answer"]
            .as_str()
            .map(single_line)
            .unwrap_or_default();
    }
    if let Some(answers) = body.get("answers").and_then(Value::as_object) {
        return single_line(
            &answers
                .iter()
                .map(|(id, a)| {
                    let value = a
                        .get("choice")
                        .or_else(|| a.get("score"))
                        .or_else(|| a.get("noul"))
                        .unwrap_or(a);
                    format!("{id}: {}", display_value(value))
                })
                .collect::<Vec<_>>()
                .join("; "),
        );
    }
    if let Some(choice) = body
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
    {
        let text = content(&choice["message"]);
        if !text.is_empty() {
            return text;
        }
        return content(&choice["text"]);
    }
    for key in ["output_text", "output", "content", "text", "contents"] {
        if let Some(value) = body.get(key) {
            let text = content(value);
            if !text.is_empty() {
                return text;
            }
        }
    }
    body.get("error")
        .filter(|e| !e.is_null())
        .map(|e| e.get("message").filter(|m| !m.is_null()).unwrap_or(e))
        .map(display_value)
        .unwrap_or_default()
}

fn preview_response(body: Option<&Value>, preview: Option<&str>, error: Option<&str>) -> String {
    if let Some(error) = error {
        return single_line(error);
    }
    let text = body.map(response).unwrap_or_default();
    if !text.is_empty() {
        return text;
    }
    preview
        .map(|s| {
            let parsed = serde_json::from_str::<Value>(s)
                .ok()
                .map(|v| response(&v))
                .unwrap_or_default();
            if parsed.is_empty() {
                single_line(s)
            } else {
                parsed
            }
        })
        .unwrap_or_default()
}

pub(crate) fn question_and_answer(data: &MonitorEventData) -> (String, String) {
    let (question, answer) = match data {
        MonitorEventData::LlmCall {
            request_body,
            response_body,
            content_preview,
            error,
            ..
        } => (
            request(request_body),
            preview_response(
                response_body.as_ref(),
                content_preview.as_deref(),
                error.as_deref(),
            ),
        ),
        MonitorEventData::MemoryCompaction {
            request_body,
            response_body,
            content_preview,
            error,
            ..
        } => (
            request_body.as_ref().map(request).unwrap_or_default(),
            preview_response(
                response_body.as_ref(),
                content_preview.as_deref(),
                error.as_deref(),
            ),
        ),
        MonitorEventData::McpToolCall {
            tool_name,
            arguments,
            response_preview,
            error,
            ..
        } => (
            format!("{tool_name}({})", display_value(arguments)),
            preview_response(None, response_preview.as_deref(), error.as_deref()),
        ),
        MonitorEventData::McpPromptGet {
            prompt_name,
            arguments,
            content_preview,
            error,
            ..
        } => (
            format!("{prompt_name}({})", display_value(arguments)),
            preview_response(None, content_preview.as_deref(), error.as_deref()),
        ),
        MonitorEventData::McpResourceRead {
            uri,
            content_preview,
            error,
            ..
        } => (
            uri.clone(),
            preview_response(None, content_preview.as_deref(), error.as_deref()),
        ),
        MonitorEventData::McpElicitation {
            message,
            content,
            action,
            ..
        } => (
            message.clone(),
            content
                .as_ref()
                .map(display_value)
                .or_else(|| action.clone())
                .unwrap_or_default(),
        ),
        MonitorEventData::McpPushEvent {
            event_name,
            delivered_payload,
            original_payload,
            decision,
            ..
        } => (
            event_name.clone(),
            delivered_payload
                .as_ref()
                .or(original_payload.as_ref())
                .map(display_value)
                .unwrap_or_else(|| decision.clone()),
        ),
        MonitorEventData::McpSampling {
            content_preview,
            action,
            ..
        } => (
            String::new(),
            content_preview
                .as_ref()
                .or(action.as_ref())
                .cloned()
                .unwrap_or_default(),
        ),
        MonitorEventData::GuardrailScan {
            text_preview,
            result,
            ..
        }
        | MonitorEventData::GuardrailResponseScan {
            text_preview,
            result,
            ..
        } => (text_preview.clone(), result.clone().unwrap_or_default()),
        MonitorEventData::SecretScan {
            text_preview,
            action_taken,
            ..
        } => (
            text_preview.clone(),
            action_taken.clone().unwrap_or_default(),
        ),
        MonitorEventData::AuthError { message, .. }
        | MonitorEventData::AccessDenied { message, .. }
        | MonitorEventData::RateLimitEvent { message, .. }
        | MonitorEventData::ValidationError { message, .. }
        | MonitorEventData::McpServerEvent { message, .. }
        | MonitorEventData::OAuthEvent { message, .. }
        | MonitorEventData::InternalError { message, .. }
        | MonitorEventData::ModerationEvent { message, .. }
        | MonitorEventData::ConnectionError { message, .. } => (String::new(), message.clone()),
        MonitorEventData::FirewallDecision {
            item_name, action, ..
        } => (item_name.clone(), action.clone()),
        MonitorEventData::JsonRepair {
            original, repaired, ..
        } => (
            original.clone().unwrap_or_default(),
            repaired.clone().unwrap_or_default(),
        ),
        MonitorEventData::RouteLlmClassify { .. }
        | MonitorEventData::RoutingDecision { .. }
        | MonitorEventData::PromptCompression { .. }
        | MonitorEventData::SseConnection { .. } => (String::new(), String::new()),
        // Passthrough intentionally captures no question/answer content.
        MonitorEventData::ProxyPassthrough { error, .. } => {
            (String::new(), error.clone().unwrap_or_default())
        }
    };
    (single_line(&question), single_line(&answer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn latest_user_question_excludes_system_and_history() {
        assert_eq!(
            request(&json!({"messages": [
                {"role":"system", "content":"instructions"},
                {"role":"user", "content":"old question"},
                {"role":"assistant", "content":"old answer"},
                {"role":"user", "content":[{"type":"text", "text":"New\n\tquestion?"}, {"type":"image_url", "image_url":{"url":"data:secret"}}]}
            ]})),
            "New question? [image]"
        );
        assert_eq!(
            request(&json!({"messages":[{"role":"system","content":"instructions"}]})),
            ""
        );
    }

    #[test]
    fn handles_responses_completions_system_one_and_mcp() {
        assert_eq!(
            request(
                &json!({"input":[{"role":"user","content":[{"type":"input_text","text":"Hello"}]}]})
            ),
            "Hello"
        );
        assert_eq!(request(&json!({"prompt":"Hello\nworld"})), "Hello world");
        assert_eq!(
            request(&json!({"questions":{"team":{"instructions":"Which team?"}}})),
            "Which team?"
        );
        assert_eq!(
            response(&json!({"answers":{"team":{"choice":"billing"}}})),
            "team: billing"
        );
        assert_eq!(
            response(
                &json!({"output":[{"type":"message","content":[{"type":"output_text","text":"Answer"}]}]})
            ),
            "Answer"
        );
        assert_eq!(
            preview_response(
                None,
                Some(r#"{"content":[{"type":"text","text":"File contents"}]}"#),
                None
            ),
            "File contents"
        );
    }

    #[test]
    fn full_response_precedes_preview_and_errors_precede_both() {
        let body = json!({"choices":[{"message":{"content":"Complete answer"}}]});
        assert_eq!(
            preview_response(Some(&body), Some("Complete..."), None),
            "Complete answer"
        );
        assert_eq!(
            preview_response(Some(&body), Some("Complete..."), Some("Failed\nretry")),
            "Failed retry"
        );
        assert_eq!(preview_response(None, None, None), "");
        assert_eq!(
            response(
                &json!({"content":[{"type":"thinking","thinking":"private reasoning"},{"type":"text","text":"Actual answer"}]})
            ),
            "Actual answer"
        );
        assert_eq!(
            response(
                &json!({"choices":[{"message":{"tool_calls":[{"function":{"name":"read_file","arguments":"{}"}}]}}]})
            ),
            "read_file({})"
        );
    }

    #[test]
    fn incremental_tool_turns_and_custom_tools_have_previews() {
        for kind in ["function_call_output", "custom_tool_call_output"] {
            assert_eq!(
                request(
                    &json!({"input":[{"type":kind,"call_id":"call-1","output":"File contents"}]})
                ),
                "[tool result] File contents"
            );
        }
        assert_eq!(
            request(&json!({"messages":[{"role":"tool","content":"Command succeeded"}]})),
            "[tool result] Command succeeded"
        );
        assert_eq!(
            request(
                &json!({"messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","content":[{"type":"text","text":"File contents"}]}]}]})
            ),
            "[tool result] File contents"
        );
        assert_eq!(
            response(
                &json!({"output":[{"type":"custom_tool_call","name":"apply_patch","input":"patch text"}]})
            ),
            "apply_patch(patch text)"
        );
        assert_eq!(
            response(
                &json!({"choices":[{"message":{"content":null,"tool_calls":[{"function":{"name":"read_file","arguments":"{}"}}]}}]})
            ),
            "read_file({})"
        );
    }

    #[test]
    fn refusals_and_documents_are_described_without_reasoning_or_binary_data() {
        assert_eq!(
            response(&json!({"choices":[{"message":{"content":null,"refusal":"Cannot comply"}}]})),
            "Cannot comply"
        );
        assert_eq!(
            request(
                &json!({"input":[{"role":"user","content":[{"type":"input_file","file_data":"data:secret"}]}]})
            ),
            "[document]"
        );
        for kind in ["thinking", "redacted_thinking", "reasoning"] {
            assert_eq!(
                content(&json!({"type":kind,"text":"private","content":"private"})),
                ""
            );
        }
        assert_eq!(
            request(
                &json!({"messages":[{"role":"system","content":"instructions"},{"role":"assistant","content":"Earlier answer"}]})
            ),
            ""
        );
    }

    #[test]
    fn previews_are_bounded_single_line_and_unicode_safe() {
        let preview = single_line(&"🦀".repeat(2000));
        assert_eq!(preview.chars().count(), MAX_PREVIEW_CHARS + 3);
        assert!(preview.ends_with("..."));
        assert_eq!(single_line("  hi\n\r\t world  "), "hi world");
    }
    #[test]
    fn null_error_is_not_an_answer_and_does_not_hide_a_content_preview() {
        let body = serde_json::json!({"output":[], "error":null});
        assert_eq!(response(&body), "");
        assert_eq!(
            preview_response(Some(&body), Some("Actual answer"), None),
            "Actual answer"
        );
        assert_eq!(
            response(&serde_json::json!({"error":{"message":"Failed"}})),
            "Failed"
        );
    }
}
