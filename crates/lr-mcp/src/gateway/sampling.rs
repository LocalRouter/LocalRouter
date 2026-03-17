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

    // --- Tool call preservation tests ---
    // These tests document that the MCP sampling conversion currently strips
    // tool_calls and tool_call_id from messages. This is a known limitation:
    // the MCP SamplingMessage type doesn't have fields for tool_calls/tool_call_id,
    // so multi-turn conversations with tool use lose this information when converted.

    #[test]
    fn test_convert_sampling_message_sets_tool_calls_to_none() {
        let msg = SamplingMessage {
            role: "assistant".to_string(),
            content: SamplingContent::Text("I'll search for that.".to_string()),
        };

        let chat_msg = convert_sampling_message_to_chat(msg);

        assert_eq!(chat_msg.role, "assistant");
        // BUG: tool_calls are always None even for assistant messages that should have them.
        // The MCP SamplingMessage type has no field for tool_calls, so they cannot be preserved.
        assert!(
            chat_msg.tool_calls.is_none(),
            "tool_calls should be None (current limitation: MCP SamplingMessage has no tool_calls field)"
        );
    }

    #[test]
    fn test_convert_sampling_message_sets_tool_call_id_to_none() {
        let msg = SamplingMessage {
            role: "tool".to_string(),
            content: SamplingContent::Text("Tool result content".to_string()),
        };

        let chat_msg = convert_sampling_message_to_chat(msg);

        assert_eq!(chat_msg.role, "tool");
        // BUG: tool_call_id is always None even for tool role messages that require it.
        // Without tool_call_id, the LLM provider cannot match tool results to tool calls.
        assert!(
            chat_msg.tool_call_id.is_none(),
            "tool_call_id should be None (current limitation: MCP SamplingMessage has no tool_call_id field)"
        );
        assert!(
            chat_msg.name.is_none(),
            "name should be None (current limitation: MCP SamplingMessage has no name field)"
        );
    }

    #[test]
    fn test_convert_sampling_request_strips_tool_calls_from_multi_turn_conversation() {
        // Simulates a multi-turn conversation where an external client (e.g., opencode)
        // sends message history that originally included tool calls.
        // The MCP sampling protocol cannot represent tool_calls, so they are lost.
        let sampling_req = SamplingRequest {
            messages: vec![
                SamplingMessage {
                    role: "user".to_string(),
                    content: SamplingContent::Text("Find GuardRails implementation".to_string()),
                },
                // This was originally an assistant message with tool_calls, but the MCP
                // SamplingMessage type can only carry content, not tool_calls.
                SamplingMessage {
                    role: "assistant".to_string(),
                    content: SamplingContent::Text(String::new()),
                },
                // This was originally a tool role message with tool_call_id, but again
                // the MCP SamplingMessage type cannot carry tool_call_id.
                SamplingMessage {
                    role: "tool".to_string(),
                    content: SamplingContent::Text("Found 697 matches...".to_string()),
                },
            ],
            model_preferences: None,
            system_prompt: Some("You are a helpful assistant".to_string()),
            max_tokens: Some(32000),
            temperature: None,
            stop_sequences: None,
            metadata: None,
        };

        let chat_req = convert_sampling_to_chat_request(sampling_req).unwrap();

        // System prompt + 3 conversation messages
        assert_eq!(chat_req.messages.len(), 4);
        assert_eq!(chat_req.messages[0].role, "system");
        assert_eq!(chat_req.messages[1].role, "user");
        assert_eq!(chat_req.messages[2].role, "assistant");
        assert_eq!(chat_req.messages[3].role, "tool");

        // The assistant message has no tool_calls — this breaks the OpenAI protocol
        // because a tool result message follows without a preceding tool call.
        assert!(
            chat_req.messages[2].tool_calls.is_none(),
            "BUG: assistant message should have tool_calls but MCP sampling strips them"
        );

        // The tool message has no tool_call_id — providers will reject this
        assert!(
            chat_req.messages[3].tool_call_id.is_none(),
            "BUG: tool message should have tool_call_id but MCP sampling strips it"
        );
    }

    #[test]
    fn test_convert_sampling_request_does_not_forward_tools_definition() {
        // The CompletionRequest produced by sampling conversion never includes
        // tools definitions, even though the downstream LLM may need them for
        // multi-turn tool calling.
        let sampling_req = SamplingRequest {
            messages: vec![SamplingMessage {
                role: "user".to_string(),
                content: SamplingContent::Text("Hello".to_string()),
            }],
            model_preferences: None,
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            stop_sequences: None,
            metadata: None,
        };

        let chat_req = convert_sampling_to_chat_request(sampling_req).unwrap();

        assert!(
            chat_req.tools.is_none(),
            "tools should be None — sampling conversion never passes tool definitions"
        );
        assert!(
            chat_req.tool_choice.is_none(),
            "tool_choice should be None — sampling conversion never passes tool_choice"
        );
    }

    #[test]
    fn test_convert_response_discards_tool_calls_from_llm() {
        use lr_providers::{CompletionChoice, CompletionResponse, FunctionCall, TokenUsage, ToolCall};

        // When the LLM responds with tool_calls (finish_reason: "tool_calls"),
        // the sampling conversion silently discards them and maps the finish
        // reason to "end_turn".
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

        // The tool_calls are silently discarded — the response only contains empty text
        match &sampling_resp.content {
            SamplingContent::Text(text) => {
                assert_eq!(text, "", "Tool call response content should be empty text");
            }
            _ => panic!("Expected text content"),
        }

        // finish_reason "tool_calls" is mapped to "end_turn" — the caller has no way
        // to know that the LLM wanted to make a tool call
        assert_eq!(
            sampling_resp.stop_reason, "end_turn",
            "BUG: finish_reason 'tool_calls' is silently mapped to 'end_turn', losing the signal that the LLM wanted to call a tool"
        );

        assert_eq!(sampling_resp.role, "assistant");
        assert_eq!(sampling_resp.model, "gpt-4");
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
                assert_eq!(text, "The GuardRails implementation is in crates/lr-guardrails/");
            }
            _ => panic!("Expected text content"),
        }
        assert_eq!(sampling_resp.stop_reason, "end_turn");
        assert_eq!(sampling_resp.model, "claude-sonnet-4-20250514");
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
            convert_chat_to_sampling_response(make_resp("stop")).unwrap().stop_reason,
            "end_turn"
        );
        assert_eq!(
            convert_chat_to_sampling_response(make_resp("length")).unwrap().stop_reason,
            "max_tokens"
        );
        assert_eq!(
            convert_chat_to_sampling_response(make_resp("content_filter")).unwrap().stop_reason,
            "end_turn"
        );
        // tool_calls finish_reason is lost — mapped to end_turn
        assert_eq!(
            convert_chat_to_sampling_response(make_resp("tool_calls")).unwrap().stop_reason,
            "end_turn",
            "tool_calls finish_reason is silently mapped to end_turn"
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

        // Parts content should be converted to Structured (JSON)
        match &sampling_resp.content {
            SamplingContent::Structured(value) => {
                assert!(value.is_array() || value.is_object(), "Should be structured JSON");
            }
            _ => panic!("Expected structured content for Parts input"),
        }
    }
}
