//! MCP Sampling Support
//!
//! Enables backend MCP servers to request LLM completions through the gateway.
//! The gateway routes these requests to configured LLM providers.

#![allow(dead_code)]

use crate::protocol::{SamplingContent, SamplingMessage, SamplingRequest, SamplingResponse};
use lr_providers::{ChatMessage, ChatMessageContent, CompletionRequest, CompletionResponse};
use lr_types::{AppError, AppResult};

/// Convert MCP sampling request to provider completion request
pub fn convert_sampling_to_chat_request(
    sampling_req: SamplingRequest,
) -> AppResult<CompletionRequest> {
    // Convert messages
    let mut messages: Vec<ChatMessage> = sampling_req
        .messages
        .into_iter()
        .map(convert_sampling_message_to_chat)
        .collect();

    // If system_prompt is provided, prepend as system message
    if let Some(system_prompt) = sampling_req.system_prompt {
        messages.insert(
            0,
            ChatMessage {
                role: "system".to_string(),
                content: ChatMessageContent::Text(system_prompt),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
        );
    }

    Ok(CompletionRequest {
        model: "".to_string(), // Will be set by provider selection
        messages,
        temperature: sampling_req.temperature,
        max_tokens: sampling_req.max_tokens,
        stream: false,
        top_p: None,
        frequency_penalty: None,
        presence_penalty: None,
        stop: sampling_req.stop_sequences,
        top_k: None,
        repetition_penalty: None,
        seed: None,
        logprobs: None,
        top_logprobs: None,
        response_format: None,
        tools: None,
        tool_choice: None,
        extensions: None,
    })
}

/// Convert MCP sampling message to OpenAI chat message
fn convert_sampling_message_to_chat(msg: SamplingMessage) -> ChatMessage {
    let content = match msg.content {
        SamplingContent::Text(text) => ChatMessageContent::Text(text),
        SamplingContent::Structured(value) => {
            // Try to extract text from structured content
            if let Some(text) = value.get("text").and_then(|v| v.as_str()) {
                ChatMessageContent::Text(text.to_string())
            } else {
                // Fall back to JSON representation
                ChatMessageContent::Text(value.to_string())
            }
        }
    };

    ChatMessage {
        role: msg.role,
        content,
        tool_calls: None,
        tool_call_id: None,
        name: None,
    }
}

/// Convert OpenAI completion response to MCP sampling response
pub fn convert_chat_to_sampling_response(
    chat_resp: CompletionResponse,
) -> AppResult<SamplingResponse> {
    // Get first choice
    let choice = chat_resp
        .choices
        .first()
        .ok_or_else(|| AppError::Internal("No choices in completion response".into()))?;

    // Extract content
    let content = match &choice.message.content {
        ChatMessageContent::Text(text) => SamplingContent::Text(text.clone()),
        ChatMessageContent::Parts(_) => {
            // Convert parts to structured content
            SamplingContent::Structured(serde_json::to_value(&choice.message.content)?)
        }
    };

    // Map finish reason
    let stop_reason = choice
        .finish_reason
        .as_ref()
        .map(|r| match r.as_str() {
            "stop" => "end_turn",
            "length" => "max_tokens",
            "content_filter" => "end_turn",
            "tool_calls" => "end_turn",
            _ => "end_turn",
        })
        .unwrap_or("end_turn")
        .to_string();

    Ok(SamplingResponse {
        model: chat_resp.model,
        stop_reason,
        role: "assistant".to_string(),
        content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_convert_sampling_request_simple() {
        let sampling_req = SamplingRequest {
            messages: vec![SamplingMessage {
                role: "user".to_string(),
                content: SamplingContent::Text("Hello, world!".to_string()),
            }],
            model_preferences: None,
            system_prompt: None,
            max_tokens: Some(100),
            temperature: Some(0.7),
            stop_sequences: None,
            metadata: None,
        };

        let chat_req = convert_sampling_to_chat_request(sampling_req).unwrap();

        assert_eq!(chat_req.messages.len(), 1);
        assert_eq!(chat_req.messages[0].role, "user");
        assert_eq!(chat_req.max_tokens, Some(100));
        assert_eq!(chat_req.temperature, Some(0.7));
    }

    #[test]
    fn test_convert_sampling_request_with_system_prompt() {
        let sampling_req = SamplingRequest {
            messages: vec![SamplingMessage {
                role: "user".to_string(),
                content: SamplingContent::Text("Hello".to_string()),
            }],
            model_preferences: None,
            system_prompt: Some("You are a helpful assistant".to_string()),
            max_tokens: None,
            temperature: None,
            stop_sequences: None,
            metadata: None,
        };

        let chat_req = convert_sampling_to_chat_request(sampling_req).unwrap();

        assert_eq!(chat_req.messages.len(), 2);
        assert_eq!(chat_req.messages[0].role, "system");
        assert_eq!(chat_req.messages[1].role, "user");
    }

    #[test]
    fn test_convert_sampling_message_structured_content() {
        let msg = SamplingMessage {
            role: "user".to_string(),
            content: SamplingContent::Structured(json!({
                "type": "text",
                "text": "Hello from structured content"
            })),
        };

        let chat_msg = convert_sampling_message_to_chat(msg);

        match chat_msg.content {
            ChatMessageContent::Text(text) => {
                assert_eq!(text, "Hello from structured content");
            }
            _ => panic!("Expected text content"),
        }
    }
}
