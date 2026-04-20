//! Inbound-side translator: our chat-completions `CompletionChunk`s →
//! Responses API SSE events.
//!
//! Used by the `/v1/responses` server route (phase 2) so we can expose
//! a Responses-API front door backed by any chat-completions provider.
//! The reverse direction lives in `stream.rs`.

use serde_json::{json, Value};

use crate::{CompletionChoice, CompletionChunk, CompletionResponse};

/// One SSE frame to send to the client: `(event_name, json_payload)`.
#[derive(Debug, Clone)]
pub struct ResponsesSseFrame {
    pub event: String,
    pub data: Value,
}

/// State kept across a streaming turn while translating
/// `CompletionChunk`s into Responses API SSE frames.
///
/// The Responses API is richer than Chat Completions (every output
/// item has its own lifecycle events). We collapse a chat-completions
/// stream into a single "message" output_item and fold its deltas
/// into `response.output_text.delta`. Tool calls become their own
/// `function_call` output_item with argument deltas.
#[derive(Debug)]
pub struct ResponsesEmitter {
    pub response_id: String,
    pub model: String,
    pub created_at: i64,
    /// Set after `response.created` is emitted so we don't re-emit it.
    started: bool,
    /// Set after the assistant `message` output_item has been added.
    message_item_added: bool,
    /// Accumulated assistant text across deltas — replayed at the end
    /// to populate the cached `ResponseObject` stored for
    /// `previous_response_id` lookup.
    accumulated_text: String,
    /// Running `output_index` counter for added items.
    next_output_index: i64,
    /// Index within `output[]` assigned to the assistant message.
    message_output_index: Option<i64>,
    /// Tool-call tracking: maps the chat-completions `tool_calls[i]`
    /// index to an `output_index` and accumulated args.
    tool_slots: Vec<ToolSlot>,
}

#[derive(Debug, Default, Clone)]
struct ToolSlot {
    /// The Responses-API `item.id` / `call_id` we fabricated or echoed.
    item_id: String,
    call_id: String,
    name: String,
    arguments: String,
    output_index: i64,
    added_emitted: bool,
}

impl ResponsesEmitter {
    pub fn new(response_id: String, model: String, created_at: i64) -> Self {
        Self {
            response_id,
            model,
            created_at,
            started: false,
            message_item_added: false,
            accumulated_text: String::new(),
            next_output_index: 0,
            message_output_index: None,
            tool_slots: Vec::new(),
        }
    }

    /// Accumulated assistant text across the turn. Used by the route
    /// handler to cache the final `ResponseObject`.
    pub fn text(&self) -> &str {
        &self.accumulated_text
    }

    /// Emit the opening `response.created` frame.
    pub fn start(&mut self) -> Vec<ResponsesSseFrame> {
        if self.started {
            return vec![];
        }
        self.started = true;
        vec![ResponsesSseFrame {
            event: "response.created".into(),
            data: json!({
                "type": "response.created",
                "response": {
                    "id": self.response_id,
                    "object": "response",
                    "created_at": self.created_at,
                    "status": "in_progress",
                    "model": self.model,
                    "output": [],
                },
            }),
        }]
    }

    /// Feed one chat-completions chunk; emit zero-or-more SSE frames.
    pub fn on_chunk(&mut self, chunk: CompletionChunk) -> Vec<ResponsesSseFrame> {
        let mut frames = Vec::new();
        if !self.started {
            frames.extend(self.start());
        }
        for choice in chunk.choices {
            frames.extend(self.on_choice(choice));
        }
        frames
    }

    fn on_choice(&mut self, choice: crate::ChunkChoice) -> Vec<ResponsesSseFrame> {
        let mut frames = Vec::new();

        if let Some(content) = choice.delta.content {
            if !content.is_empty() {
                if !self.message_item_added {
                    let idx = self.next_output_index;
                    self.next_output_index += 1;
                    self.message_output_index = Some(idx);
                    self.message_item_added = true;
                    frames.push(ResponsesSseFrame {
                        event: "response.output_item.added".into(),
                        data: json!({
                            "type": "response.output_item.added",
                            "output_index": idx,
                            "item": {
                                "type": "message",
                                "id": format!("msg_{}", self.response_id),
                                "role": "assistant",
                                "content": [],
                            },
                        }),
                    });
                }
                self.accumulated_text.push_str(&content);
                frames.push(ResponsesSseFrame {
                    event: "response.output_text.delta".into(),
                    data: json!({
                        "type": "response.output_text.delta",
                        "output_index": self.message_output_index,
                        "delta": content,
                    }),
                });
            }
        }

        if let Some(tool_calls) = choice.delta.tool_calls {
            for tc in tool_calls {
                let slot = self.ensure_tool_slot(&tc);
                if !slot.added_emitted {
                    slot.added_emitted = true;
                    frames.push(ResponsesSseFrame {
                        event: "response.output_item.added".into(),
                        data: json!({
                            "type": "response.output_item.added",
                            "output_index": slot.output_index,
                            "item": {
                                "type": "function_call",
                                "id": slot.item_id,
                                "call_id": slot.call_id,
                                "name": slot.name,
                                "arguments": "",
                            },
                        }),
                    });
                }
                if let Some(func) = tc.function {
                    if let Some(name) = func.name {
                        if !name.is_empty() && slot.name != name {
                            slot.name = name;
                        }
                    }
                    if let Some(args) = func.arguments {
                        if !args.is_empty() {
                            slot.arguments.push_str(&args);
                            frames.push(ResponsesSseFrame {
                                event: "response.function_call_arguments.delta".into(),
                                data: json!({
                                    "type": "response.function_call_arguments.delta",
                                    "output_index": slot.output_index,
                                    "item_id": slot.item_id,
                                    "delta": args,
                                }),
                            });
                        }
                    }
                }
            }
        }

        frames
    }

    fn ensure_tool_slot(&mut self, tc: &crate::ToolCallDelta) -> &mut ToolSlot {
        let needed = (tc.index as usize).saturating_add(1);
        while self.tool_slots.len() < needed {
            self.tool_slots.push(ToolSlot::default());
        }
        let slot = &mut self.tool_slots[tc.index as usize];
        if slot.item_id.is_empty() {
            slot.item_id = format!("fc_{}_{}", self.response_id, tc.index);
        }
        if slot.call_id.is_empty() {
            slot.call_id = tc
                .id
                .clone()
                .unwrap_or_else(|| format!("call_{}_{}", self.response_id, tc.index));
        }
        if slot.output_index == 0 && !slot.added_emitted {
            slot.output_index = self.next_output_index;
            self.next_output_index += 1;
        }
        if let Some(func) = &tc.function {
            if let Some(name) = &func.name {
                if !name.is_empty() && slot.name.is_empty() {
                    slot.name = name.clone();
                }
            }
        }
        slot
    }

    /// Emit the terminal `response.completed` (or `response.failed`)
    /// frame after all chunks have been folded in. Call once.
    pub fn finish(&mut self, finish_reason: Option<&str>) -> Vec<ResponsesSseFrame> {
        let mut frames = Vec::new();

        // Close any message item we opened.
        if let Some(idx) = self.message_output_index {
            frames.push(ResponsesSseFrame {
                event: "response.output_item.done".into(),
                data: json!({
                    "type": "response.output_item.done",
                    "output_index": idx,
                    "item": {
                        "type": "message",
                        "id": format!("msg_{}", self.response_id),
                        "role": "assistant",
                        "content": [
                            {"type": "output_text", "text": self.accumulated_text.clone()}
                        ],
                    },
                }),
            });
        }
        for slot in &self.tool_slots {
            if !slot.added_emitted {
                continue;
            }
            frames.push(ResponsesSseFrame {
                event: "response.output_item.done".into(),
                data: json!({
                    "type": "response.output_item.done",
                    "output_index": slot.output_index,
                    "item": {
                        "type": "function_call",
                        "id": slot.item_id,
                        "call_id": slot.call_id,
                        "name": slot.name,
                        "arguments": slot.arguments,
                    },
                }),
            });
        }

        let status = if finish_reason == Some("tool_calls") {
            "incomplete"
        } else {
            "completed"
        };

        frames.push(ResponsesSseFrame {
            event: format!("response.{}", status),
            data: json!({
                "type": format!("response.{}", status),
                "response": self.final_response_object(status),
            }),
        });
        frames
    }

    /// Build the `ResponseObject` payload that goes inside the
    /// `response.completed` frame and gets cached for
    /// `previous_response_id` lookups.
    pub fn final_response_object(&self, status: &str) -> Value {
        let mut output: Vec<Value> = Vec::new();
        if self.message_item_added {
            output.push(json!({
                "type": "message",
                "id": format!("msg_{}", self.response_id),
                "role": "assistant",
                "content": [
                    {"type": "output_text", "text": self.accumulated_text}
                ],
            }));
        }
        for slot in &self.tool_slots {
            if !slot.added_emitted {
                continue;
            }
            output.push(json!({
                "type": "function_call",
                "id": slot.item_id,
                "call_id": slot.call_id,
                "name": slot.name,
                "arguments": slot.arguments,
            }));
        }
        json!({
            "id": self.response_id,
            "object": "response",
            "created_at": self.created_at,
            "status": status,
            "model": self.model,
            "output": output,
        })
    }
}

// ============================================================================
// Non-streaming: CompletionResponse → ResponseObject JSON
// ============================================================================

/// Translate a non-streaming `CompletionResponse` (chat-completions
/// shape) into a `ResponseObject` JSON value suitable for serving on
/// `POST /v1/responses` when `stream=false`.
pub fn completion_to_response_object(
    resp: &CompletionResponse,
    response_id: &str,
    created_at: i64,
) -> Value {
    let choice = resp.choices.first();
    let message = choice.map(|c: &CompletionChoice| &c.message);
    let finish_reason = choice.and_then(|c| c.finish_reason.clone());

    let mut output: Vec<Value> = Vec::new();

    if let Some(msg) = message {
        let text = msg.content.as_str().into_owned();
        if !text.is_empty() {
            output.push(json!({
                "type": "message",
                "id": format!("msg_{}", response_id),
                "role": "assistant",
                "content": [
                    {"type": "output_text", "text": text}
                ],
            }));
        }
        if let Some(calls) = &msg.tool_calls {
            for (idx, tc) in calls.iter().enumerate() {
                output.push(json!({
                    "type": "function_call",
                    "id": format!("fc_{}_{}", response_id, idx),
                    "call_id": tc.id,
                    "name": tc.function.name,
                    "arguments": tc.function.arguments,
                }));
            }
        }
    }

    let status = if finish_reason.as_deref() == Some("tool_calls") {
        "incomplete"
    } else {
        "completed"
    };

    json!({
        "id": response_id,
        "object": "response",
        "created_at": created_at,
        "status": status,
        "model": resp.model,
        "output": output,
        "usage": {
            "input_tokens": resp.usage.prompt_tokens,
            "output_tokens": resp.usage.completion_tokens,
            "total_tokens": resp.usage.total_tokens,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkChoice, ChunkDelta, CompletionChunk};

    fn text_chunk(content: &str) -> CompletionChunk {
        CompletionChunk {
            id: "chatcmpl-1".into(),
            object: "chat.completion.chunk".into(),
            created: 0,
            model: "gpt-4o".into(),
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    role: None,
                    content: Some(content.into()),
                    tool_calls: None,
                    reasoning_content: None,
                },
                finish_reason: None,
            }],
            extensions: None,
        }
    }

    #[test]
    fn emits_created_item_added_deltas_item_done_completed() {
        let mut e = ResponsesEmitter::new("resp_1".into(), "gpt-4o".into(), 100);
        let mut frames: Vec<ResponsesSseFrame> = Vec::new();
        frames.extend(e.on_chunk(text_chunk("Hel")));
        frames.extend(e.on_chunk(text_chunk("lo")));
        frames.extend(e.finish(Some("stop")));

        let names: Vec<String> = frames.iter().map(|f| f.event.clone()).collect();
        assert_eq!(
            names,
            vec![
                "response.created",
                "response.output_item.added",
                "response.output_text.delta",
                "response.output_text.delta",
                "response.output_item.done",
                "response.completed",
            ]
        );
        assert_eq!(e.text(), "Hello");
    }

    #[test]
    fn final_response_object_from_non_stream_completion() {
        use crate::{ChatMessage, ChatMessageContent, CompletionChoice, TokenUsage};
        let resp = CompletionResponse {
            id: "cmpl_1".into(),
            object: "chat.completion".into(),
            created: 10,
            model: "gpt-4o".into(),
            provider: "OpenAI".into(),
            choices: vec![CompletionChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".into(),
                    content: ChatMessageContent::Text("Hi".into()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                    reasoning_content: None,
                },
                finish_reason: Some("stop".into()),
                logprobs: None,
            }],
            usage: TokenUsage {
                prompt_tokens: 2,
                completion_tokens: 1,
                total_tokens: 3,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            system_fingerprint: None,
            service_tier: None,
            extensions: None,
            routellm_win_rate: None,
            request_usage_entries: None,
        };
        let v = completion_to_response_object(&resp, "resp_1", 100);
        assert_eq!(v["id"], "resp_1");
        assert_eq!(v["status"], "completed");
        assert_eq!(v["output"][0]["content"][0]["text"], "Hi");
        assert_eq!(v["usage"]["total_tokens"], 3);
    }
}
