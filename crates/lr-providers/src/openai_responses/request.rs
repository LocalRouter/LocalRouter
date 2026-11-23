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
//! - `tool_choice`: the client's choice (`"auto"`, `"none"`,
//!   `"required"`, or a specific function as `{"type": "function",
//!   "name": ...}`), `"auto"` when unset.
//! - Function tools always carry `strict`: the client's value, else
//!   `false`. The Responses API treats a missing `strict` as `true`, which
//!   would make every property required, while a Chat Completions tool
//!   without `strict` is non-strict (Codex also sends `strict: false`).

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
    let mut reasoning = InputReasoning::from_request(req);
    // Calls to tools the request declares as `custom` replay as custom tool
    // items (free-form `input`), everything else as function calls.
    let custom_tools: std::collections::HashSet<&str> = req
        .tools
        .iter()
        .flatten()
        .filter(|t| t.tool_type == "custom")
        .map(|t| t.function.name.as_str())
        .collect();
    let mut custom_call_ids: std::collections::HashSet<String> = Default::default();

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
                    input.extend(reasoning.before_message(&msg.content.as_text()));
                    input.push(ResponseItem::Message {
                        role: "assistant".into(),
                        content,
                    });
                }
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        input.extend(reasoning.before_call(&tc.id));
                        if custom_tools.contains(tc.function.name.as_str()) {
                            custom_call_ids.insert(tc.id.clone());
                            input.push(ResponseItem::CustomToolCall {
                                call_id: tc.id.clone(),
                                name: tc.function.name.clone(),
                                input: tc.function.arguments.clone(),
                            });
                        } else {
                            input.push(ResponseItem::FunctionCall {
                                call_id: tc.id.clone(),
                                name: tc.function.name.clone(),
                                arguments: tc.function.arguments.clone(),
                            });
                        }
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
                let output = msg.content.as_str().into_owned();
                input.push(if custom_call_ids.contains(&call_id) {
                    ResponseItem::CustomToolCallOutput { call_id, output }
                } else {
                    ResponseItem::FunctionCallOutput { call_id, output }
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

    // The ChatGPT backend (`chatgpt.com/backend-api/codex/responses`)
    // rejects requests with empty `instructions` as
    // `400 {"detail":"Instructions are required"}`. Most clients
    // hitting `/v1/chat/completions` against ChatGPT Plus don't send
    // a system message, so we'd silently 400 their requests. Fall
    // back to a minimal generic prompt when the caller didn't supply
    // one — matches Codex CLI's behavior of always sending a non-
    // empty system prompt.
    let instructions = if instructions_parts.is_empty() {
        DEFAULT_INSTRUCTIONS.to_string()
    } else {
        instructions_parts.join("\n")
    };

    ResponsesApiRequest {
        model: req.model.clone(),
        instructions,
        input,
        tools,
        tool_choice: tool_choice_to_value(req.tool_choice.as_ref()),
        parallel_tool_calls: req.parallel_tool_calls.unwrap_or(true),
        reasoning,
        store,
        stream: req.stream,
        include,
        service_tier: req.service_tier.clone(),
        prompt_cache_key: crate::session::cache_key(req),
        text,
        previous_response_id: None,
    }
}

/// Request extension carrying the `reasoning` items of a Responses API
/// `input`, which the chat-completions shape has no place for. Each entry
/// is `{"call_id": ...}` or `{"assistant_text": ...}` (the output that
/// followed the reasoning) plus the original `item`. Only a provider whose
/// upstream API is the Responses API receives them (`retain_for_upstream`);
/// for any other provider the router drops them.
pub const INPUT_REASONING_EXTENSION: &str = "localrouter.responses.reasoning_items";

/// Drop the carried `reasoning` input items unless the provider speaks the
/// Responses API (structured: the provider's `upstream_api()`).
pub fn retain_for_upstream(request: &mut CompletionRequest, upstream: lr_types::LlmApi) {
    if upstream == lr_types::LlmApi::Responses {
        return;
    }
    if let Some(extensions) = request.extensions.as_mut() {
        extensions.remove(INPUT_REASONING_EXTENSION);
        if extensions.is_empty() {
            request.extensions = None;
        }
    }
}

/// Reasoning items to replay, by the output they preceded.
#[derive(Default)]
struct InputReasoning {
    by_call: std::collections::HashMap<String, Vec<ResponseItem>>,
    by_text: Vec<(String, Vec<ResponseItem>)>,
}

impl InputReasoning {
    fn from_request(req: &CompletionRequest) -> Self {
        let mut out = Self::default();
        let Some(entries) = req
            .extensions
            .as_ref()
            .and_then(|e| e.get(INPUT_REASONING_EXTENSION))
            .and_then(|v| v.as_array())
        else {
            return out;
        };
        for entry in entries {
            let Some(item) = entry.get("item").and_then(reasoning_item) else {
                continue;
            };
            if let Some(call_id) = entry.get("call_id").and_then(|v| v.as_str()) {
                out.by_call
                    .entry(call_id.to_string())
                    .or_default()
                    .push(item);
            } else if let Some(text) = entry.get("assistant_text").and_then(|v| v.as_str()) {
                match out.by_text.iter_mut().find(|(t, _)| t == text) {
                    Some((_, items)) => items.push(item),
                    None => out.by_text.push((text.to_string(), vec![item])),
                }
            }
        }
        out
    }

    fn before_call(&mut self, call_id: &str) -> Vec<ResponseItem> {
        self.by_call.remove(call_id).unwrap_or_default()
    }

    fn before_message(&mut self, text: &str) -> Vec<ResponseItem> {
        match self.by_text.iter().position(|(t, _)| t == text) {
            Some(i) => self.by_text.remove(i).1,
            None => Vec::new(),
        }
    }
}

/// A Responses API `reasoning` input item, keeping its opaque fields.
fn reasoning_item(value: &serde_json::Value) -> Option<ResponseItem> {
    if value.get("type").and_then(|t| t.as_str()) != Some("reasoning") {
        return None;
    }
    Some(ResponseItem::Reasoning {
        summary: value
            .get("summary")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
        content: value.get("content").and_then(|v| v.as_array()).cloned(),
        encrypted_content: value
            .get("encrypted_content")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    })
}

/// Fallback used when the chat-completions request had no system /
/// developer message and the upstream demands non-empty instructions
/// (today: ChatGPT Plus's `/responses` endpoint). Kept minimal so it
/// doesn't bias the model away from the user's actual request.
pub(crate) const DEFAULT_INSTRUCTIONS: &str = "You are a helpful assistant.";

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
                ContentPart::File { file } => Some(ContentItem::InputFile {
                    file_data: file.file_data.clone(),
                    filename: file.filename.clone(),
                }),
                ContentPart::ImageUrl { image_url } => Some(ContentItem::InputImage {
                    image_url: image_url.url.clone(),
                    detail: image_url.detail.clone(),
                }),
            })
            .collect(),
    }
}

/// Map a Chat Completions `tool_choice` to the Responses API's form: mode
/// strings pass through, a specific function becomes `{"type":
/// "function", "name": ...}` (no nested `function` object).
fn tool_choice_to_value(choice: Option<&crate::ToolChoice>) -> serde_json::Value {
    match choice {
        None => json!("auto"),
        Some(crate::ToolChoice::Auto(mode)) => json!(mode),
        Some(crate::ToolChoice::Specific {
            tool_type,
            function,
        }) => json!({"type": tool_type, "name": function.name}),
    }
}

/// Convert our internal `Tool` into the Responses API's tool wire
/// format (`{ "type": "function", "name": ..., "parameters": ...,
/// "strict": ... }`).
fn tool_to_value(tool: &crate::Tool) -> serde_json::Value {
    if tool.tool_type == "custom" {
        // Custom tools take free-form input; their optional grammar
        // travels in `parameters` (see the `/v1/responses` route).
        let mut value = json!({
            "type": "custom",
            "name": tool.function.name,
            "description": tool.function.description,
        });
        if !tool.function.parameters.is_null() {
            value["format"] = tool.function.parameters.clone();
        }
        return value;
    }
    json!({
        "type": tool.tool_type,
        "name": tool.function.name,
        "description": tool.function.description,
        "parameters": tool.function.parameters,
        "strict": tool.function.strict.unwrap_or(false),
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
            native_messages: None,
            server_tools: None,
            documents: None,
        }
    }

    fn msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            annotations: None,
            role: role.into(),
            content: ChatMessageContent::Text(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }
    }

    #[test]
    fn custom_tool_calls_and_reasoning_replay_as_their_own_items() {
        let mut req = base_request(vec![
            msg("user", "patch"),
            ChatMessage {
                tool_calls: Some(vec![ToolCall {
                    id: "c1".into(),
                    tool_type: "function".into(),
                    function: FunctionCall {
                        name: "apply_patch".into(),
                        arguments: "*** Begin Patch".into(),
                    },
                }]),
                ..msg("assistant", "")
            },
            ChatMessage {
                tool_call_id: Some("c1".into()),
                ..msg("tool", "Done")
            },
            msg("assistant", "Patched"),
        ]);
        req.tools = Some(vec![Tool {
            tool_type: "custom".into(),
            function: crate::FunctionDefinition {
                name: "apply_patch".into(),
                description: Some("Apply a patch".into()),
                parameters: serde_json::json!({"type": "grammar"}),
                strict: None,
            },
        }]);
        req.extensions = Some(
            [(
                INPUT_REASONING_EXTENSION.to_string(),
                serde_json::json!([
                    {"call_id": "c1", "item": {"type": "reasoning", "summary": [], "encrypted_content": "r1"}},
                    {"assistant_text": "Patched", "item": {"type": "reasoning", "summary": [], "encrypted_content": "r2"}},
                ]),
            )]
            .into_iter()
            .collect(),
        );
        let out = plain(req.clone());
        let items: Vec<serde_json::Value> = out
            .input
            .iter()
            .map(|item| serde_json::to_value(item).unwrap())
            .collect();
        let kinds: Vec<&str> = items.iter().map(|i| i["type"].as_str().unwrap()).collect();
        assert_eq!(
            kinds,
            [
                "message",
                "reasoning",
                "custom_tool_call",
                "custom_tool_call_output",
                "reasoning",
                "message"
            ]
        );
        assert_eq!(items[1]["encrypted_content"], "r1");
        assert_eq!(items[2]["input"], "*** Begin Patch");
        assert_eq!(items[3]["output"], "Done");
        assert_eq!(out.tools[0]["type"], "custom");
        assert_eq!(
            out.tools[0]["format"],
            serde_json::json!({"type": "grammar"})
        );
        assert!(out.tools[0].get("parameters").is_none());

        // A chat-completions upstream gets no reasoning items; a Responses
        // upstream keeps them.
        let mut chat = req.clone();
        retain_for_upstream(&mut chat, lr_types::LlmApi::ChatCompletions);
        assert!(chat.extensions.is_none());
        let mut responses = req;
        retain_for_upstream(&mut responses, lr_types::LlmApi::Responses);
        assert!(responses.extensions.is_some());
    }

    #[test]
    fn hoists_system_into_instructions() {
        let req = base_request(vec![msg("system", "Be helpful."), msg("user", "hi")]);
        let out = plain(req);
        assert_eq!(out.instructions, "Be helpful.");
        assert_eq!(out.input.len(), 1);
        assert!(matches!(&out.input[0], ResponseItem::Message { role, .. } if role == "user"));
        assert_eq!(out.tool_choice, serde_json::json!("auto"));
    }

    #[test]
    fn tool_call_round_trip_preserves_order() {
        let mut messages = vec![msg("user", "weather?")];
        messages.push(ChatMessage {
            annotations: None,
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
            annotations: None,
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
    fn empty_instructions_falls_back_to_default() {
        // ChatGPT backend rejects empty `instructions` as
        // `400 Instructions are required`. Verify the translator
        // supplies a non-empty default when the caller had no
        // system / developer message.
        let req = base_request(vec![msg("user", "hi")]);
        let out = plain(req);
        assert_eq!(out.instructions, DEFAULT_INSTRUCTIONS);
        assert!(!out.instructions.is_empty());
    }

    #[test]
    fn explicit_system_overrides_default_instructions() {
        // Defensive: when the caller does provide a system message,
        // it must take precedence over the fallback — otherwise we'd
        // be silently overriding user intent.
        let req = base_request(vec![
            msg("system", "Always reply in haiku."),
            msg("user", "weather?"),
        ]);
        let out = plain(req);
        assert_eq!(out.instructions, "Always reply in haiku.");
        assert_ne!(out.instructions, DEFAULT_INSTRUCTIONS);
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
                strict: None,
            },
        }]);
        let out = plain(req);
        assert_eq!(out.tools.len(), 1);
        assert_eq!(out.tools[0]["type"], "function");
        assert_eq!(out.tools[0]["name"], "search");
    }

    /// The Responses API treats a missing `strict` as true (every property
    /// required). A Chat Completions tool without `strict` is non-strict,
    /// so the translation must say `false`, and keep an explicit value.
    #[test]
    fn tool_strict_is_always_explicit() {
        let tool = |strict: Option<bool>| Tool {
            tool_type: "function".into(),
            function: crate::FunctionDefinition {
                name: "record".into(),
                description: None,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"a": {"type": "string"}, "b": {"type": "string"}},
                    "required": ["a"]
                }),
                strict,
            },
        };
        let mut req = base_request(vec![msg("user", "hi")]);
        req.tools = Some(vec![tool(None), tool(Some(true)), tool(Some(false))]);
        let out = plain(req);
        assert_eq!(out.tools[0]["strict"], false);
        assert_eq!(out.tools[1]["strict"], true);
        assert_eq!(out.tools[2]["strict"], false);
        // The schema is untouched: optional fields stay optional.
        assert_eq!(
            out.tools[0]["parameters"]["required"],
            serde_json::json!(["a"])
        );
    }

    #[test]
    fn tool_choice_follows_the_client() {
        let with = |choice: Option<crate::ToolChoice>| {
            let mut req = base_request(vec![msg("user", "hi")]);
            req.tool_choice = choice;
            plain(req).tool_choice
        };
        assert_eq!(with(None), serde_json::json!("auto"));
        for mode in ["auto", "none", "required"] {
            assert_eq!(
                with(Some(crate::ToolChoice::Auto(mode.into()))),
                serde_json::json!(mode)
            );
        }
        assert_eq!(
            with(Some(crate::ToolChoice::Specific {
                tool_type: "function".into(),
                function: crate::FunctionName {
                    name: "record".into()
                },
            })),
            serde_json::json!({"type": "function", "name": "record"})
        );
    }

    /// A client's `strict` survives parsing and is omitted when unset.
    #[test]
    fn function_definition_strict_round_trip() {
        let with: crate::FunctionDefinition = serde_json::from_value(serde_json::json!({
            "name": "f", "parameters": {"type": "object"}, "strict": true
        }))
        .unwrap();
        assert_eq!(with.strict, Some(true));
        let without: crate::FunctionDefinition = serde_json::from_value(serde_json::json!({
            "name": "f", "parameters": {"type": "object"}
        }))
        .unwrap();
        assert_eq!(without.strict, None);
        assert!(serde_json::to_value(&without)
            .unwrap()
            .get("strict")
            .is_none());
    }
}
