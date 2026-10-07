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
fn content(value: &Value) -> String {
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
            if let Some(text) = value.get("text").and_then(Value::as_str) {
                return single_line(text);
            }
            if let Some(body) = value.get("content") {
                return content(body);
            }
            match value.get("type").and_then(Value::as_str) {
                Some("image" | "image_url" | "input_image") => "[image]".into(),
                Some("input_audio" | "audio") => "[audio]".into(),
                Some("tool_use" | "function_call") => format!(
                    "{}({})",
                    value["name"].as_str().unwrap_or("tool"),
                    value
                        .get("input")
                        .or_else(|| value.get("arguments"))
                        .map(display_value)
                        .unwrap_or_default()
                ),
                Some("thinking" | "redacted_thinking" | "reasoning") => String::new(),
                _ => String::new(),
            }
        }
        _ => String::new(),
    }
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
        let text = content(&choice["message"]["content"]);
        if !text.is_empty() {
            return text;
        }
        if let Some(calls) = choice["message"]["tool_calls"].as_array() {
            return single_line(
                &calls
                    .iter()
                    .map(|call| {
                        format!(
                            "{}({})",
                            call["function"]["name"].as_str().unwrap_or("tool"),
                            display_value(&call["function"]["arguments"])
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; "),
            );
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
