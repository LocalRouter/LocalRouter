//! Non-streaming `/responses` → `CompletionResponse` translator.
//!
//! Flattens the Responses API's `output: Vec<OutputItem>` into a
//! single chat-completions-style choice. Text content from all
//! `message`-kind items is concatenated in order; `function_call`
//! items populate `choices[0].message.tool_calls`.

use super::types::{ContentItem, OutputItem, ResponseObject};
use crate::{
    ChatMessage, ChatMessageContent, CompletionChoice, CompletionResponse, FunctionCall,
    TokenUsage, ToolCall,
};

/// Convert a non-streaming `/responses` response into our
/// `CompletionResponse`. `provider_name` is the instance name used
/// for the `provider` field on the response.
pub fn response_to_completion(r: ResponseObject, provider_name: &str) -> CompletionResponse {
    let mut text_buf = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for item in r.output {
        match item {
            OutputItem::Message { content, .. } => {
                for c in content {
                    if let ContentItem::OutputText { text } = c {
                        text_buf.push_str(&text);
                    }
                }
            }
            OutputItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } => tool_calls.push(ToolCall {
                id: call_id,
                tool_type: "function".into(),
                function: FunctionCall { name, arguments },
            }),
            OutputItem::Other => {}
        }
    }

    let finish_reason = if !tool_calls.is_empty() {
        Some("tool_calls".to_string())
    } else {
        Some("stop".to_string())
    };

    let message = ChatMessage {
        role: "assistant".into(),
        content: ChatMessageContent::Text(text_buf),
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        tool_call_id: None,
        name: None,
        reasoning_content: None,
    };

    let usage = match r.usage {
        Some(u) => TokenUsage {
            prompt_tokens: u.input_tokens,
            completion_tokens: u.output_tokens,
            total_tokens: if u.total_tokens > 0 {
                u.total_tokens
            } else {
                u.input_tokens + u.output_tokens
            },
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        None => TokenUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
    };

    CompletionResponse {
        id: r.id,
        object: "chat.completion".into(),
        created: r.created_at.unwrap_or(0),
        model: r.model.unwrap_or_default(),
        provider: provider_name.to_string(),
        choices: vec![CompletionChoice {
            index: 0,
            message,
            finish_reason,
            logprobs: None,
        }],
        usage,
        system_fingerprint: None,
        service_tier: None,
        extensions: None,
        routellm_win_rate: None,
        request_usage_entries: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai_responses::types::{OutputTokensDetails, ResponsesUsage};

    #[test]
    fn flattens_message_output_into_text() {
        let r = ResponseObject {
            id: "resp_1".into(),
            object: Some("response".into()),
            created_at: Some(123),
            model: Some("gpt-4o".into()),
            status: Some("completed".into()),
            output: vec![OutputItem::Message {
                id: None,
                role: Some("assistant".into()),
                content: vec![
                    ContentItem::OutputText {
                        text: "hello".into(),
                    },
                    ContentItem::OutputText {
                        text: " world".into(),
                    },
                ],
            }],
            usage: Some(ResponsesUsage {
                input_tokens: 3,
                output_tokens: 2,
                total_tokens: 5,
                output_tokens_details: None,
            }),
        };
        let out = response_to_completion(r, "OpenAI");
        assert_eq!(out.id, "resp_1");
        assert_eq!(out.provider, "OpenAI");
        let choice = &out.choices[0];
        assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
        assert_eq!(choice.message.content.as_str(), "hello world");
        assert!(choice.message.tool_calls.is_none());
        assert_eq!(out.usage.prompt_tokens, 3);
        assert_eq!(out.usage.completion_tokens, 2);
    }

    #[test]
    fn function_call_output_becomes_tool_call() {
        let r = ResponseObject {
            id: "resp_2".into(),
            object: None,
            created_at: None,
            model: None,
            status: None,
            output: vec![OutputItem::FunctionCall {
                id: None,
                call_id: "call_1".into(),
                name: "get_weather".into(),
                arguments: r#"{"city":"sf"}"#.into(),
            }],
            usage: None,
        };
        let out = response_to_completion(r, "OpenAI");
        let msg = &out.choices[0].message;
        let tc = msg.tool_calls.as_ref().unwrap();
        assert_eq!(tc.len(), 1);
        assert_eq!(tc[0].id, "call_1");
        assert_eq!(tc[0].function.name, "get_weather");
        assert_eq!(out.choices[0].finish_reason.as_deref(), Some("tool_calls"));
    }

    #[test]
    fn unknown_output_variants_are_ignored() {
        let r = ResponseObject {
            id: "resp_3".into(),
            object: None,
            created_at: None,
            model: None,
            status: None,
            output: vec![
                OutputItem::Other,
                OutputItem::Message {
                    id: None,
                    role: None,
                    content: vec![ContentItem::OutputText { text: "ok".into() }],
                },
            ],
            usage: None,
        };
        let out = response_to_completion(r, "OpenAI");
        assert_eq!(out.choices[0].message.content.as_str(), "ok");
    }

    #[test]
    fn computes_total_tokens_when_missing() {
        let r = ResponseObject {
            id: "resp_4".into(),
            object: None,
            created_at: None,
            model: None,
            status: None,
            output: vec![],
            usage: Some(ResponsesUsage {
                input_tokens: 4,
                output_tokens: 6,
                total_tokens: 0, // missing / zero in wire
                output_tokens_details: Some(OutputTokensDetails {
                    reasoning_tokens: 1,
                }),
            }),
        };
        let out = response_to_completion(r, "OpenAI");
        assert_eq!(out.usage.total_tokens, 10);
    }
}
