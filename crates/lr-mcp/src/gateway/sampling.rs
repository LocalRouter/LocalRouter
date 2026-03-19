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
        tools: sampling_req.tools,
        tool_choice: sampling_req.tool_choice,
        extensions: None,
        pre_computed_routing: None,
        n: None,
        logit_bias: None,
        parallel_tool_calls: None,
        service_tier: None,
        store: None,
        metadata: None,
        modalities: None,
        audio: None,
        prediction: None,
        reasoning_effort: None,
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
        tool_calls: msg.tool_calls,
        tool_call_id: msg.tool_call_id,
        name: msg.name,
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
            "tool_calls" => "tool_calls",
            _ => "end_turn",
        })
        .unwrap_or("end_turn")
        .to_string();

    let tool_calls = choice.message.tool_calls.clone();

    Ok(SamplingResponse {
        model: chat_resp.model,
        stop_reason,
        role: "assistant".to_string(),
        content,
        tool_calls,
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
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            model_preferences: None,
            system_prompt: None,
            max_tokens: Some(100),
            temperature: Some(0.7),
            stop_sequences: None,
            metadata: None,
            tools: None,
            tool_choice: None,
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
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            model_preferences: None,
            system_prompt: Some("You are a helpful assistant".to_string()),
            max_tokens: None,
            temperature: None,
            stop_sequences: None,
            metadata: None,
            tools: None,
            tool_choice: None,
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
            tool_calls: None,
            tool_call_id: None,
            name: None,
        };

        let chat_msg = convert_sampling_message_to_chat(msg);

        match chat_msg.content {
            ChatMessageContent::Text(text) => {
                assert_eq!(text, "Hello from structured content");
            }
            _ => panic!("Expected text content"),
        }
    }

    // --- Tool call preservation tests ---

    #[test]
    fn test_convert_sampling_message_preserves_tool_calls() {
        use lr_providers::{FunctionCall, ToolCall};

        let msg = SamplingMessage {
            role: "assistant".to_string(),
            content: SamplingContent::Text(String::new()),
            tool_calls: Some(vec![ToolCall {
                id: "call_abc123".to_string(),
                tool_type: "function".to_string(),
                function: FunctionCall {
                    name: "search_code".to_string(),
                    arguments: r#"{"query": "guardrails"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let chat_msg = convert_sampling_message_to_chat(msg);

        assert_eq!(chat_msg.role, "assistant");
        assert!(chat_msg.tool_calls.is_some());
        let tc = &chat_msg.tool_calls.unwrap()[0];
        assert_eq!(tc.id, "call_abc123");
        assert_eq!(tc.function.name, "search_code");
    }

    #[test]
    fn test_convert_sampling_message_preserves_tool_call_id() {
        let msg = SamplingMessage {
            role: "tool".to_string(),
            content: SamplingContent::Text("Tool result content".to_string()),
            tool_calls: None,
            tool_call_id: Some("call_abc123".to_string()),
            name: Some("search_code".to_string()),
        };

        let chat_msg = convert_sampling_message_to_chat(msg);

        assert_eq!(chat_msg.role, "tool");
        assert_eq!(chat_msg.tool_call_id.as_deref(), Some("call_abc123"));
        assert_eq!(chat_msg.name.as_deref(), Some("search_code"));
    }

    #[test]
    fn test_convert_sampling_message_none_tool_fields_pass_through() {
        let msg = SamplingMessage {
            role: "user".to_string(),
            content: SamplingContent::Text("Hello".to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        };

        let chat_msg = convert_sampling_message_to_chat(msg);
        assert!(chat_msg.tool_calls.is_none());
        assert!(chat_msg.tool_call_id.is_none());
        assert!(chat_msg.name.is_none());
    }

    #[test]
    fn test_multi_turn_tool_use_conversation_roundtrip() {
        use lr_providers::{FunctionCall, FunctionDefinition, Tool, ToolCall, ToolChoice};

        let sampling_req = SamplingRequest {
            messages: vec![
                SamplingMessage {
                    role: "user".to_string(),
                    content: SamplingContent::Text("Find GuardRails implementation".to_string()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
                SamplingMessage {
                    role: "assistant".to_string(),
                    content: SamplingContent::Text(String::new()),
                    tool_calls: Some(vec![ToolCall {
                        id: "call_abc".to_string(),
                        tool_type: "function".to_string(),
                        function: FunctionCall {
                            name: "search_code".to_string(),
                            arguments: r#"{"query":"guardrails"}"#.to_string(),
                        },
                    }]),
                    tool_call_id: None,
                    name: None,
                },
                SamplingMessage {
                    role: "tool".to_string(),
                    content: SamplingContent::Text("Found 697 matches...".to_string()),
                    tool_calls: None,
                    tool_call_id: Some("call_abc".to_string()),
                    name: Some("search_code".to_string()),
                },
            ],
            model_preferences: None,
            system_prompt: Some("You are a helpful assistant".to_string()),
            max_tokens: Some(32000),
            temperature: None,
            stop_sequences: None,
            metadata: None,
            tools: Some(vec![Tool {
                tool_type: "function".to_string(),
                function: FunctionDefinition {
                    name: "search_code".to_string(),
                    description: Some("Search for code".to_string()),
                    parameters: serde_json::json!({"type": "object"}),
                },
            }]),
            tool_choice: Some(ToolChoice::Auto("auto".to_string())),
        };

        let chat_req = convert_sampling_to_chat_request(sampling_req).unwrap();

        // System prompt + 3 conversation messages
        assert_eq!(chat_req.messages.len(), 4);
        assert_eq!(chat_req.messages[0].role, "system");
        assert_eq!(chat_req.messages[1].role, "user");
        assert_eq!(chat_req.messages[2].role, "assistant");
        assert_eq!(chat_req.messages[3].role, "tool");

        // Assistant message preserves tool_calls
        assert!(chat_req.messages[2].tool_calls.is_some());
        assert_eq!(
            chat_req.messages[2].tool_calls.as_ref().unwrap()[0].id,
            "call_abc"
        );

        // Tool message preserves tool_call_id and name
        assert_eq!(
            chat_req.messages[3].tool_call_id.as_deref(),
            Some("call_abc")
        );
        assert_eq!(chat_req.messages[3].name.as_deref(), Some("search_code"));

        // Tools definition is forwarded
        assert!(chat_req.tools.is_some());
        assert_eq!(chat_req.tools.unwrap()[0].function.name, "search_code");

        // Tool choice is forwarded
        assert!(chat_req.tool_choice.is_some());
    }

    #[test]
    fn test_convert_sampling_request_forwards_tools() {
        use lr_providers::{FunctionDefinition, Tool, ToolChoice};

        let sampling_req = SamplingRequest {
            messages: vec![SamplingMessage {
                role: "user".to_string(),
                content: SamplingContent::Text("Hello".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            model_preferences: None,
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            stop_sequences: None,
            metadata: None,
            tools: Some(vec![Tool {
                tool_type: "function".to_string(),
                function: FunctionDefinition {
                    name: "search".to_string(),
                    description: Some("Search for code".to_string()),
                    parameters: serde_json::json!({"type": "object"}),
                },
            }]),
            tool_choice: Some(ToolChoice::Auto("auto".to_string())),
        };

        let chat_req = convert_sampling_to_chat_request(sampling_req).unwrap();

        assert!(chat_req.tools.is_some());
        assert_eq!(chat_req.tools.unwrap()[0].function.name, "search");
        assert!(chat_req.tool_choice.is_some());
    }

    #[test]
    fn test_convert_sampling_request_no_tools_passes_none() {
        let sampling_req = SamplingRequest {
            messages: vec![SamplingMessage {
                role: "user".to_string(),
                content: SamplingContent::Text("Hello".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            model_preferences: None,
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            stop_sequences: None,
            metadata: None,
            tools: None,
            tool_choice: None,
        };

        let chat_req = convert_sampling_to_chat_request(sampling_req).unwrap();

        assert!(chat_req.tools.is_none());
        assert!(chat_req.tool_choice.is_none());
    }

    #[test]
    fn test_convert_response_preserves_tool_calls() {
        use lr_providers::{
            CompletionChoice, CompletionResponse, FunctionCall, TokenUsage, ToolCall,
        };

        let chat_resp = CompletionResponse {
            id: "chatcmpl-123".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "gpt-4".to_string(),
            provider: "openai".to_string(),
            choices: vec![CompletionChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content: ChatMessageContent::Text(String::new()),
                    tool_calls: Some(vec![ToolCall {
                        id: "call_abc123".to_string(),
                        tool_type: "function".to_string(),
                        function: FunctionCall {
                            name: "search_code".to_string(),
                            arguments: r#"{"query": "guardrails"}"#.to_string(),
                        },
                    }]),
                    tool_call_id: None,
                    name: None,
                },
                finish_reason: Some("tool_calls".to_string()),
                logprobs: None,
            }],
            usage: TokenUsage {
                prompt_tokens: 100,
                completion_tokens: 20,
                total_tokens: 120,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            system_fingerprint: None,
            service_tier: None,
            extensions: None,
            routellm_win_rate: None,
            request_usage_entries: None,
        };

        let sampling_resp = convert_chat_to_sampling_response(chat_resp).unwrap();

        assert_eq!(sampling_resp.stop_reason, "tool_calls");
        assert_eq!(sampling_resp.role, "assistant");
        assert_eq!(sampling_resp.model, "gpt-4");

        assert!(sampling_resp.tool_calls.is_some());
        let tc = &sampling_resp.tool_calls.unwrap()[0];
        assert_eq!(tc.id, "call_abc123");
        assert_eq!(tc.function.name, "search_code");
    }

    #[test]
    fn test_convert_response_preserves_text_content() {
        use lr_providers::{CompletionChoice, CompletionResponse, TokenUsage};

        let chat_resp = CompletionResponse {
            id: "chatcmpl-456".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "claude-sonnet-4-20250514".to_string(),
            provider: "anthropic".to_string(),
            choices: vec![CompletionChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content: ChatMessageContent::Text(
                        "The GuardRails implementation is in crates/lr-guardrails/".to_string(),
                    ),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
                finish_reason: Some("stop".to_string()),
                logprobs: None,
            }],
            usage: TokenUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            system_fingerprint: None,
            service_tier: None,
            extensions: None,
            routellm_win_rate: None,
            request_usage_entries: None,
        };

        let sampling_resp = convert_chat_to_sampling_response(chat_resp).unwrap();

        match &sampling_resp.content {
            SamplingContent::Text(text) => {
                assert_eq!(
                    text,
                    "The GuardRails implementation is in crates/lr-guardrails/"
                );
            }
            _ => panic!("Expected text content"),
        }
        assert_eq!(sampling_resp.stop_reason, "end_turn");
        assert_eq!(sampling_resp.model, "claude-sonnet-4-20250514");
        assert!(sampling_resp.tool_calls.is_none());
    }

    #[test]
    fn test_convert_response_maps_finish_reasons_correctly() {
        use lr_providers::{CompletionChoice, CompletionResponse, TokenUsage};

        let make_resp = |finish_reason: &str| -> CompletionResponse {
            CompletionResponse {
                id: "test".to_string(),
                object: "chat.completion".to_string(),
                created: 0,
                model: "test-model".to_string(),
                provider: "test".to_string(),
                choices: vec![CompletionChoice {
                    index: 0,
                    message: ChatMessage {
                        role: "assistant".to_string(),
                        content: ChatMessageContent::Text("ok".to_string()),
                        tool_calls: None,
                        tool_call_id: None,
                        name: None,
                    },
                    finish_reason: Some(finish_reason.to_string()),
                    logprobs: None,
                }],
                usage: TokenUsage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                    prompt_tokens_details: None,
                    completion_tokens_details: None,
                },
                system_fingerprint: None,
                service_tier: None,
                extensions: None,
                routellm_win_rate: None,
                request_usage_entries: None,
            }
        };

        assert_eq!(
            convert_chat_to_sampling_response(make_resp("stop"))
                .unwrap()
                .stop_reason,
            "end_turn"
        );
        assert_eq!(
            convert_chat_to_sampling_response(make_resp("length"))
                .unwrap()
                .stop_reason,
            "max_tokens"
        );
        assert_eq!(
            convert_chat_to_sampling_response(make_resp("content_filter"))
                .unwrap()
                .stop_reason,
            "end_turn"
        );
        assert_eq!(
            convert_chat_to_sampling_response(make_resp("tool_calls"))
                .unwrap()
                .stop_reason,
            "tool_calls"
        );
    }

    #[test]
    fn test_convert_response_empty_choices_returns_error() {
        use lr_providers::{CompletionResponse, TokenUsage};

        let chat_resp = CompletionResponse {
            id: "test".to_string(),
            object: "chat.completion".to_string(),
            created: 0,
            model: "test-model".to_string(),
            provider: "test".to_string(),
            choices: vec![],
            usage: TokenUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            system_fingerprint: None,
            service_tier: None,
            extensions: None,
            routellm_win_rate: None,
            request_usage_entries: None,
        };

        let result = convert_chat_to_sampling_response(chat_resp);
        assert!(result.is_err(), "Should error when no choices in response");
    }

    #[test]
    fn test_convert_response_multipart_content_becomes_structured() {
        use lr_providers::{CompletionChoice, CompletionResponse, ContentPart, TokenUsage};

        let chat_resp = CompletionResponse {
            id: "test".to_string(),
            object: "chat.completion".to_string(),
            created: 0,
            model: "test-model".to_string(),
            provider: "test".to_string(),
            choices: vec![CompletionChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content: ChatMessageContent::Parts(vec![ContentPart::Text {
                        text: "Hello".to_string(),
                    }]),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
                finish_reason: Some("stop".to_string()),
                logprobs: None,
            }],
            usage: TokenUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            system_fingerprint: None,
            service_tier: None,
            extensions: None,
            routellm_win_rate: None,
            request_usage_entries: None,
        };

        let sampling_resp = convert_chat_to_sampling_response(chat_resp).unwrap();

        match &sampling_resp.content {
            SamplingContent::Structured(value) => {
                assert!(
                    value.is_array() || value.is_object(),
                    "Should be structured JSON"
                );
            }
            _ => panic!("Expected structured content for Parts input"),
        }
    }
}
