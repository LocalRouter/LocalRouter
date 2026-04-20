//! Translation layer: `CompletionRequest` (chat-completions shape) →
//! `ResponsesApiRequest` (Responses API shape).
//!
//! Mapping rules, adapted from Codex's Rust client (`codex-rs/core/src/
//! client.rs`) and OpenClaw's JS client
//! (`openresponses-http-*.js`):
//!
//! - `system`/`developer` role messages → hoisted into the top-level
//!   `instructions` field, newline-joined.
//! - `user`/`assistant`/`tool` role messages → preserved in order as
//!   typed `ResponseItem`s.
//! - `assistant.tool_calls[]` → emit a `ResponseItem::FunctionCall`
//!   for each call **before** any subsequent `tool` message so the
//!   server sees the round-trip in the right order.
//! - Image parts (`image_url`) → `ContentItem::InputImage`. Text
//!   parts → `InputText`.
//! - `reasoning_effort: "low" | "medium" | "high"` → `Reasoning {
//!   effort: Some(...) }`. When set we also request
//!   `include: ["usage"]` so token counts come back on the stream.
//! - `response_format: JsonSchema` → `text.format = JsonSchema`.
//! - `tool_choice` hardcoded `"auto"` (Codex default; ChatGPT-backend
//!   ignores other values).

use super::types::{
    ContentItem, Reasoning, ResponseItem, ResponsesApiRequest, TextControls, TextFormat,
};
use crate::{ChatMessageContent, CompletionRequest, ContentPart, ResponseFormat};
use serde_json::json;

/// Translate a chat-completions request into a Responses API request.
///
/// `store` controls server-side retention of the response; callers
/// typically pass `false` when speaking to upstream-but-not-our-own
/// `/responses` (avoid lingering conversations in someone else's
/// store), `true` when the caller wants `previous_response_id`
/// continuation against OpenAI's retention.
pub fn translate_completion_request(req: &CompletionRequest, store: bool) -> ResponsesApiRequest {
    let mut instructions_parts: Vec<String> = Vec::new();
    let mut input: Vec<ResponseItem> = Vec::with_capacity(req.messages.len());

    for msg in &req.messages {
        match msg.role.as_str() {
            "system" | "developer" => {
                // Flatten into top-level `instructions` field. We keep
                // the concatenation order the client provided.
                let text = msg.content.as_str();
                if !text.is_empty() {
                    instructions_parts.push(text.into_owned());
                }
            }
            "assistant" => {
                // Assistant messages may carry both text content and
                // tool calls. Emit the text first (if any), then one
                // FunctionCall item per tool call. The server replays
                // this whole history when computing its response.
                let content = chat_content_to_items(&msg.content, /*is_output=*/ true);
                if !content.is_empty() {
                    input.push(ResponseItem::Message {
                        role: "assistant".into(),
                        content,
                    });
                }
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        input.push(ResponseItem::FunctionCall {
                            call_id: tc.id.clone(),
                            name: tc.function.name.clone(),
                            arguments: tc.function.arguments.clone(),
                        });
                    }
                }
            }
            "tool" => {
                // `tool` role messages map to FunctionCallOutput, the
                // response payload for a previously-issued call.
                let call_id = msg
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| msg.name.clone().unwrap_or_default());
                input.push(ResponseItem::FunctionCallOutput {
                    call_id,
                    output: msg.content.as_str().into_owned(),
                });
            }
            _ => {
                // "user" and any unknown role fall through as a plain
                // user message — preserving content is safer than
                // dropping it.
                let content = chat_content_to_items(&msg.content, /*is_output=*/ false);
                if !content.is_empty() {
                    input.push(ResponseItem::Message {
                        role: msg.role.clone(),
                        content,
                    });
                }
            }
        }
    }

    let (reasoning, include) = match req.reasoning_effort.as_deref() {
        Some(effort) if !effort.is_empty() => (
            Some(Reasoning {
                effort: Some(effort.to_string()),
                summary: None,
            }),
            vec!["usage".to_string()],
        ),
        _ => (None, vec![]),
    };

    let text = req.response_format.as_ref().and_then(|f| match f {
        ResponseFormat::JsonSchema { schema, .. } => Some(TextControls {
            verbosity: None,
            format: Some(TextFormat {
                format_type: "json_schema".into(),
                strict: true,
                schema: schema.clone(),
                name: String::new(),
            }),
        }),
        ResponseFormat::JsonObject { .. } => None, // /responses doesn't have a non-schema JSON mode
    });

    let tools = req
        .tools
        .as_ref()
        .map(|ts| ts.iter().map(tool_to_value).collect::<Vec<_>>())
        .unwrap_or_default();

    ResponsesApiRequest {
        model: req.model.clone(),
        instructions: instructions_parts.join("\n"),
        input,
        tools,
        tool_choice: "auto".into(),
        parallel_tool_calls: req.parallel_tool_calls.unwrap_or(true),
        reasoning,
        store,
        stream: req.stream,
        include,
        service_tier: req.service_tier.clone(),
        prompt_cache_key: None,
        text,
        previous_response_id: None,
    }
}

/// Convert our `ChatMessageContent` (text or multimodal parts) into
/// Responses-API `ContentItem`s. `is_output = true` for assistant
/// messages replayed in history — they must use `output_text`, not
/// `input_text`; anything else uses `input_*`.
fn chat_content_to_items(content: &ChatMessageContent, is_output: bool) -> Vec<ContentItem> {
    match content {
        ChatMessageContent::Text(t) => {
            if t.is_empty() {
                vec![]
            } else if is_output {
                vec![ContentItem::OutputText { text: t.clone() }]
            } else {
                vec![ContentItem::InputText { text: t.clone() }]
            }
        }
        ChatMessageContent::Parts(parts) => parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text } => {
                    if text.is_empty() {
                        None
                    } else if is_output {
                        Some(ContentItem::OutputText { text: text.clone() })
                    } else {
                        Some(ContentItem::InputText { text: text.clone() })
                    }
                }
                ContentPart::ImageUrl { image_url } => Some(ContentItem::InputImage {
                    image_url: image_url.url.clone(),
                    detail: image_url.detail.clone(),
                }),
            })
            .collect(),
    }
}

/// Convert our internal `Tool` into the Responses API's tool wire
/// format (`{ "type": "function", "name": ..., "parameters": ... }`).
fn tool_to_value(tool: &crate::Tool) -> serde_json::Value {
    json!({
        "type": tool.tool_type,
        "name": tool.function.name,
        "description": tool.function.description,
        "parameters": tool.function.parameters,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatMessage, ChatMessageContent, CompletionRequest, FunctionCall, Tool, ToolCall};

    fn plain(req: CompletionRequest) -> ResponsesApiRequest {
        translate_completion_request(&req, false)
    }

    /// Build an empty-field `CompletionRequest` for tests.
    /// `CompletionRequest` has ~25 fields without a `Default` impl, so
    /// we spell out the full construction once.
    fn base_request(messages: Vec<ChatMessage>) -> CompletionRequest {
        CompletionRequest {
            model: "gpt-4o".into(),
            messages,
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            extensions: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            logprobs: None,
            top_logprobs: None,
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
            pre_computed_routing: None,
        }
    }

    fn msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content: ChatMessageContent::Text(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }
    }

    #[test]
    fn hoists_system_into_instructions() {
        let req = base_request(vec![msg("system", "Be helpful."), msg("user", "hi")]);
        let out = plain(req);
        assert_eq!(out.instructions, "Be helpful.");
        assert_eq!(out.input.len(), 1);
        assert!(matches!(&out.input[0], ResponseItem::Message { role, .. } if role == "user"));
        assert_eq!(out.tool_choice, "auto");
    }

    #[test]
    fn tool_call_round_trip_preserves_order() {
        let mut messages = vec![msg("user", "weather?")];
        messages.push(ChatMessage {
            role: "assistant".into(),
            content: ChatMessageContent::Text(String::new()),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".into(),
                tool_type: "function".into(),
                function: FunctionCall {
                    name: "get_weather".into(),
                    arguments: r#"{"city":"sf"}"#.into(),
                },
            }]),
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        });
        messages.push(ChatMessage {
            role: "tool".into(),
            content: ChatMessageContent::Text("72F".into()),
            tool_calls: None,
            tool_call_id: Some("call_1".into()),
            name: None,
            reasoning_content: None,
        });

        let out = plain(base_request(messages));
        // Order: user message, (no assistant text because content is empty),
        // FunctionCall, FunctionCallOutput.
        assert!(
            matches!(&out.input[0], ResponseItem::Message { role, .. } if role == "user"),
            "first item should be the user message"
        );
        assert!(matches!(
            &out.input[1],
            ResponseItem::FunctionCall { call_id, .. } if call_id == "call_1"
        ));
        assert!(matches!(
            &out.input[2],
            ResponseItem::FunctionCallOutput { call_id, .. } if call_id == "call_1"
        ));
    }

    #[test]
    fn reasoning_effort_sets_include_usage() {
        let mut req = base_request(vec![msg("user", "hi")]);
        req.reasoning_effort = Some("high".into());
        let out = plain(req);
        assert_eq!(
            out.reasoning.as_ref().and_then(|r| r.effort.as_deref()),
            Some("high")
        );
        assert_eq!(out.include, vec!["usage"]);
    }

    #[test]
    fn json_schema_response_format_translates() {
        let schema = serde_json::json!({"type":"object","properties":{"x":{"type":"number"}}});
        let mut req = base_request(vec![msg("user", "n")]);
        req.response_format = Some(ResponseFormat::JsonSchema {
            format_type: "json_schema".into(),
            schema: schema.clone(),
        });
        let out = plain(req);
        let fmt = out.text.unwrap().format.unwrap();
        assert_eq!(fmt.format_type, "json_schema");
        assert!(fmt.strict);
        assert_eq!(fmt.schema, schema);
    }

    #[test]
    fn tools_passed_through_as_function_wire_format() {
        let mut req = base_request(vec![msg("user", "hi")]);
        req.tools = Some(vec![Tool {
            tool_type: "function".into(),
            function: crate::FunctionDefinition {
                name: "search".into(),
                description: Some("search the web".into()),
                parameters: serde_json::json!({"type":"object"}),
            },
        }]);
        let out = plain(req);
        assert_eq!(out.tools.len(), 1);
        assert_eq!(out.tools[0]["type"], "function");
        assert_eq!(out.tools[0]["name"], "search");
    }
}
