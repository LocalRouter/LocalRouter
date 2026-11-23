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
/// item has its own lifecycle events). A chat-completions stream's text
/// becomes a "message" output_item with `response.output_text.delta`
/// events; `close_message` ends it, so a stream that holds several
/// assistant messages (MCP via LLM text before and after gateway tool
/// calls) yields one message item each. Tool calls become their own
/// `function_call` output_item with argument deltas.
#[derive(Debug)]
pub struct ResponsesEmitter {
    pub response_id: String,
    pub model: String,
    pub created_at: i64,
    /// Set after opening a translated stream or observing a native event.
    started: bool,
    /// Responses clients use this required field to order every event.
    next_sequence_number: u64,
    /// Set while an assistant `message` output_item is open.
    message_item_added: bool,
    /// Text of the open message — replayed at the end to populate the
    /// cached `ResponseObject` stored for `previous_response_id` lookup.
    accumulated_text: String,
    annotations: Vec<Value>,
    /// Messages opened so far (the open one included); numbers their ids.
    message_count: usize,
    /// Messages already closed: `(output_index, item)`.
    closed_messages: Vec<(i64, Value)>,
    /// Text of every message, closed ones included.
    all_text: String,
    /// The finish reason `finish` was given, for `incomplete_details`.
    finish_reason: Option<String>,
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
            next_sequence_number: 0,
            message_item_added: false,
            accumulated_text: String::new(),
            annotations: Vec::new(),
            message_count: 0,
            closed_messages: Vec::new(),
            all_text: String::new(),
            finish_reason: None,
            next_output_index: 0,
            message_output_index: None,
            tool_slots: Vec::new(),
        }
    }

    /// Assistant text across the turn, every message included.
    pub fn text(&self) -> &str {
        &self.all_text
    }

    /// Response status for a chat-completions finish reason. A turn that
    /// ends in tool calls is `completed` (its `function_call` items are
    /// what the client answers); only a cut-off turn is `incomplete`.
    pub fn status_for(finish_reason: Option<&str>) -> &'static str {
        match finish_reason {
            Some("length") | Some("content_filter") => "incomplete",
            _ => "completed",
        }
    }

    fn incomplete_details(finish_reason: Option<&str>) -> Option<Value> {
        match finish_reason {
            Some("length") => Some(json!({"reason": "max_output_tokens"})),
            Some("content_filter") => Some(json!({"reason": "content_filter"})),
            _ => None,
        }
    }

    fn message_id(&self) -> String {
        match self.message_count {
            0 | 1 => format!("msg_{}", self.response_id),
            n => format!("msg_{}_{}", self.response_id, n - 1),
        }
    }

    fn message_item(&self, status: &str) -> Value {
        json!({
            "type": "message",
            "id": self.message_id(),
            "role": "assistant",
            "status": status,
            "content": [
                {"type": "output_text", "text": self.accumulated_text, "annotations": self.annotations}
            ],
        })
    }

    /// Frames for a piece of assistant text, opening a message item first
    /// when none is open (unsequenced).
    fn text_frames(&mut self, content: String) -> Vec<ResponsesSseFrame> {
        let mut frames = Vec::new();
        if content.is_empty() {
            return frames;
        }
        if !self.message_item_added {
            let idx = self.next_output_index;
            self.next_output_index += 1;
            self.message_output_index = Some(idx);
            self.message_item_added = true;
            self.message_count += 1;
            frames.push(ResponsesSseFrame {
                event: "response.output_item.added".into(),
                data: json!({
                    "type": "response.output_item.added",
                    "output_index": idx,
                    "item": {
                        "type": "message",
                        "id": self.message_id(),
                        "role": "assistant",
                        "status": "in_progress",
                        "content": [],
                    },
                }),
            });
            frames.push(ResponsesSseFrame {
                event: "response.content_part.added".into(),
                data: json!({
                    "type": "response.content_part.added",
                    "item_id": self.message_id(),
                    "output_index": idx,
                    "content_index": 0,
                    "part": {"type": "output_text", "text": "", "annotations": [], "logprobs": []},
                }),
            });
        }
        self.accumulated_text.push_str(&content);
        self.all_text.push_str(&content);
        frames.push(ResponsesSseFrame {
            event: "response.output_text.delta".into(),
            data: json!({
                "type": "response.output_text.delta",
                "item_id": self.message_id(),
                "output_index": self.message_output_index,
                "content_index": 0,
                "delta": content,
                "logprobs": [],
            }),
        });
        frames
    }

    /// `output_text.done`, `content_part.done` and `output_item.done` for
    /// the open message, if any, which is then closed (unsequenced).
    fn close_message_frames(&mut self) -> Vec<ResponsesSseFrame> {
        let Some(idx) = self.message_output_index.take() else {
            return Vec::new();
        };
        let item = self.message_item("completed");
        let item_id = self.message_id();
        self.closed_messages.push((idx, item.clone()));
        self.message_item_added = false;
        let text = std::mem::take(&mut self.accumulated_text);
        let annotations = std::mem::take(&mut self.annotations);
        vec![
            ResponsesSseFrame {
                event: "response.output_text.done".into(),
                data: json!({
                    "type": "response.output_text.done",
                    "item_id": item_id,
                    "output_index": idx,
                    "content_index": 0,
                    "text": text,
                    "logprobs": [],
                }),
            },
            ResponsesSseFrame {
                event: "response.content_part.done".into(),
                data: json!({
                    "type": "response.content_part.done",
                    "item_id": item_id,
                    "output_index": idx,
                    "content_index": 0,
                    "part": {"type": "output_text", "text": text, "annotations": annotations, "logprobs": []},
                }),
            },
            ResponsesSseFrame {
                event: "response.output_item.done".into(),
                data: json!({
                    "type": "response.output_item.done",
                    "output_index": idx,
                    "item": item,
                }),
            },
        ]
    }

    /// End the open assistant message: later text starts a new message
    /// item.
    pub fn close_message(&mut self) -> Vec<ResponsesSseFrame> {
        let mut frames = Vec::new();
        if !self.started {
            frames.extend(self.start());
        }
        let closing = self.close_message_frames();
        frames.extend(self.sequence_frames(closing));
        frames
    }

    /// A complete assistant message of its own (opened, filled, closed).
    pub fn add_message(&mut self, text: &str) -> Vec<ResponsesSseFrame> {
        let mut frames = self.close_message();
        let mut message = self.text_frames(text.to_string());
        message.extend(self.close_message_frames());
        frames.extend(self.sequence_frames(message));
        frames
    }

    fn sequence_frames(&mut self, mut frames: Vec<ResponsesSseFrame>) -> Vec<ResponsesSseFrame> {
        for frame in &mut frames {
            frame.data["sequence_number"] = json!(self.next_sequence_number);
            self.next_sequence_number += 1;
        }
        frames
    }

    /// Native events retain their upstream sequence numbers. A locally
    /// generated failure must continue after the last forwarded event.
    pub fn observe_native_envelope(&mut self, envelope: &Value) {
        self.started = true;
        if let Some(sequence_number) = envelope["sequence_number"].as_u64() {
            self.next_sequence_number = sequence_number.saturating_add(1);
        }
    }

    /// Emit the opening `response.created` and `response.in_progress`
    /// frames.
    pub fn start(&mut self) -> Vec<ResponsesSseFrame> {
        if self.started {
            return vec![];
        }
        self.started = true;
        let response = json!({
            "id": self.response_id,
            "object": "response",
            "created_at": self.created_at,
            "status": "in_progress",
            "model": self.model,
            "output": [],
        });
        let frames = ["response.created", "response.in_progress"]
            .into_iter()
            .map(|event| ResponsesSseFrame {
                event: event.into(),
                data: json!({"type": event, "response": response}),
            })
            .collect();
        self.sequence_frames(frames)
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

        if let Some(annotations) = &choice.delta.annotations {
            for annotation in crate::server_tools::response_annotations(annotations) {
                if self.annotations.len() < 128 && !self.annotations.contains(&annotation) {
                    self.annotations.push(annotation);
                }
            }
        }
        if let Some(content) = choice.delta.content {
            frames.extend(self.text_frames(content));
        }

        if let Some(tool_calls) = choice.delta.tool_calls {
            for tc in tool_calls {
                // Output items follow one another: the message a call
                // follows ends before the call's item is added.
                if self.message_item_added && !self.tool_slot_added(tc.index) {
                    frames.extend(self.close_message_frames());
                }
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
                                "status": "in_progress",
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

        self.sequence_frames(frames)
    }

    fn tool_slot_added(&self, index: u32) -> bool {
        self.tool_slots
            .get(index as usize)
            .is_some_and(|slot| slot.added_emitted)
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

    /// Emit the terminal `response.completed` (or `response.incomplete`)
    /// frame after all chunks have been folded in. Call once.
    pub fn finish(&mut self, finish_reason: Option<&str>) -> Vec<ResponsesSseFrame> {
        let mut opening = self.start();
        self.finish_reason = finish_reason.map(str::to_string);
        let mut frames = Vec::new();

        // Close any message item still open.
        frames.extend(self.close_message_frames());
        for slot in &self.tool_slots {
            if !slot.added_emitted {
                continue;
            }
            frames.push(ResponsesSseFrame {
                event: "response.function_call_arguments.done".into(),
                data: json!({
                    "type": "response.function_call_arguments.done",
                    "item_id": slot.item_id,
                    "output_index": slot.output_index,
                    "name": slot.name,
                    "arguments": slot.arguments,
                }),
            });
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
                        "status": "completed",
                    },
                }),
            });
        }

        let status = Self::status_for(finish_reason);

        frames.push(ResponsesSseFrame {
            event: format!("response.{}", status),
            data: json!({
                "type": format!("response.{}", status),
                "response": self.final_response_object(status),
            }),
        });
        opening.extend(self.sequence_frames(frames));
        opening
    }

    /// Emit a terminal failure with the same response envelope as success.
    pub fn fail(&mut self, error: Value) -> Vec<ResponsesSseFrame> {
        let mut frames = self.start();
        let mut response = self.final_response_object("failed");
        response["error"] = error;
        frames.extend(self.sequence_frames(vec![ResponsesSseFrame {
            event: "response.failed".into(),
            data: json!({"type": "response.failed", "response": response}),
        }]));
        frames
    }

    /// Build the `ResponseObject` payload that goes inside the
    /// `response.completed` frame and gets cached for
    /// `previous_response_id` lookups.
    pub fn final_response_object(&self, status: &str) -> Value {
        let mut output: Vec<(i64, Value)> = self.closed_messages.clone();
        let message_status = if status == "failed" {
            "incomplete"
        } else {
            "completed"
        };
        if let (true, Some(idx)) = (self.message_item_added, self.message_output_index) {
            output.push((idx, self.message_item(message_status)));
        }
        for slot in &self.tool_slots {
            if !slot.added_emitted {
                continue;
            }
            output.push((
                slot.output_index,
                json!({
                    "type": "function_call",
                    "id": slot.item_id,
                    "call_id": slot.call_id,
                    "name": slot.name,
                    "arguments": slot.arguments,
                    "status": message_status,
                }),
            ));
        }
        output.sort_by_key(|(idx, _)| *idx);
        let mut response = json!({
            "id": self.response_id,
            "object": "response",
            "created_at": self.created_at,
            "status": status,
            "model": self.model,
            "output": output.into_iter().map(|(_, item)| item).collect::<Vec<_>>(),
        });
        if status == "incomplete" {
            if let Some(details) = Self::incomplete_details(self.finish_reason.as_deref()) {
                response["incomplete_details"] = details;
            }
        }
        response
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
    let status = ResponsesEmitter::status_for(finish_reason.as_deref());
    // A cut-off response's message is cut off too.
    let message_status = if status == "incomplete" {
        "incomplete"
    } else {
        "completed"
    };

    let mut output: Vec<Value> = Vec::new();

    if let Some(msg) = message {
        let text = msg.content.as_str().into_owned();
        if !text.is_empty() {
            output.push(json!({
                "type": "message",
                "id": format!("msg_{}", response_id),
                "role": "assistant",
                "status": message_status,
                "content": [
                    {"type": "output_text", "text": text, "annotations": crate::server_tools::response_annotations(msg.annotations.as_deref().unwrap_or_default())}
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
                    "status": "completed",
                }));
            }
        }
    }

    let mut response = json!({
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
    });
    if let Some(details) = ResponsesEmitter::incomplete_details(finish_reason.as_deref()) {
        response["incomplete_details"] = details;
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkChoice, ChunkDelta, CompletionChunk};

    fn assert_sequences(frames: &[ResponsesSseFrame], first: u64) {
        #[derive(serde::Deserialize)]
        struct EventEnvelope {
            r#type: String,
            sequence_number: u64,
        }

        for (offset, frame) in frames.iter().enumerate() {
            let event: EventEnvelope = serde_json::from_value(frame.data.clone()).unwrap();
            assert_eq!(event.r#type, frame.event);
            assert_eq!(event.sequence_number, first + offset as u64);
        }
    }

    fn text_chunk(content: &str) -> CompletionChunk {
        CompletionChunk {
            id: "chatcmpl-1".into(),
            object: "chat.completion.chunk".into(),
            created: 0,
            model: "gpt-4o".into(),
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    annotations: None,
                    role: None,
                    content: Some(content.into()),
                    tool_calls: None,
                    reasoning_content: None,
                },
                finish_reason: None,
            }],
            extensions: None,
            usage: None,
            provider: None,
        }
    }

    #[test]
    fn closed_messages_are_separate_items_in_output_order() {
        let mut e = ResponsesEmitter::new("resp_1".into(), "gpt-4o".into(), 0);
        let mut frames = e.on_chunk(text_chunk("Let me look."));
        frames.extend(e.close_message());
        frames.extend(e.on_chunk(text_chunk("Found it.")));
        frames.extend(e.finish(Some("stop")));
        assert_sequences(&frames, 0);
        let added: Vec<_> = frames
            .iter()
            .filter(|f| f.event == "response.output_item.added")
            .map(|f| (f.data["output_index"].clone(), f.data["item"]["id"].clone()))
            .collect();
        assert_eq!(
            added,
            [
                (json!(0), json!("msg_resp_1")),
                (json!(1), json!("msg_resp_1_1"))
            ]
        );
        let done = frames
            .iter()
            .filter(|f| f.event == "response.output_item.done")
            .count();
        assert_eq!(done, 2);
        let output = &frames.last().unwrap().data["response"]["output"];
        assert_eq!(output[0]["content"][0]["text"], "Let me look.");
        assert_eq!(output[1]["content"][0]["text"], "Found it.");
        assert_eq!(e.text(), "Let me look.Found it.");

        // A message of its own, then the stream's text: two items again.
        let mut e = ResponsesEmitter::new("resp_2".into(), "gpt-4o".into(), 0);
        let mut frames = e.add_message("Earlier");
        frames.extend(e.on_chunk(text_chunk("Final")));
        frames.extend(e.finish(None));
        assert_sequences(&frames, 0);
        let output = &frames.last().unwrap().data["response"]["output"];
        assert_eq!(output.as_array().unwrap().len(), 2);
        assert_eq!(output[0]["content"][0]["text"], "Earlier");
    }

    #[test]
    fn tool_calls_complete_the_response_and_cut_off_output_is_incomplete() {
        assert_eq!(
            ResponsesEmitter::status_for(Some("tool_calls")),
            "completed"
        );
        assert_eq!(ResponsesEmitter::status_for(Some("stop")), "completed");
        assert_eq!(ResponsesEmitter::status_for(None), "completed");
        assert_eq!(ResponsesEmitter::status_for(Some("length")), "incomplete");
        let mut e = ResponsesEmitter::new("resp_1".into(), "gpt-4o".into(), 0);
        let _ = e.on_chunk(text_chunk("partial"));
        let frames = e.finish(Some("length"));
        let last = frames.last().unwrap();
        assert_eq!(last.event, "response.incomplete");
        assert_eq!(
            last.data["response"]["incomplete_details"],
            json!({"reason": "max_output_tokens"})
        );
    }

    #[test]
    fn emits_created_item_added_deltas_item_done_completed() {
        let mut e = ResponsesEmitter::new("resp_1".into(), "gpt-4o".into(), 100);
        let mut frames: Vec<ResponsesSseFrame> = Vec::new();
        frames.extend(e.on_chunk(text_chunk("Hel")));
        frames.extend(e.on_chunk(text_chunk("lo")));
        frames.extend(e.finish(Some("stop")));

        let names: Vec<String> = frames.iter().map(|f| f.event.clone()).collect();
        // OpenAI's sequence for a message.
        assert_eq!(
            names,
            vec![
                "response.created",
                "response.in_progress",
                "response.output_item.added",
                "response.content_part.added",
                "response.output_text.delta",
                "response.output_text.delta",
                "response.output_text.done",
                "response.content_part.done",
                "response.output_item.done",
                "response.completed",
            ]
        );
        let item_id = &frames[2].data["item"]["id"];
        for frame in &frames[3..8] {
            assert_eq!(&frame.data["item_id"], item_id, "{}", frame.event);
            assert_eq!(frame.data["output_index"], 0, "{}", frame.event);
            assert_eq!(frame.data["content_index"], 0, "{}", frame.event);
        }
        assert_eq!(
            frames[3].data["part"],
            json!({"type": "output_text", "text": "", "annotations": [], "logprobs": []})
        );
        assert_eq!(frames[6].data["text"], "Hello");
        assert_eq!(frames[7].data["part"]["text"], "Hello");
        assert_eq!(frames[1].data["response"]["status"], "in_progress");
        assert_sequences(&frames, 0);
        // Required fields used by strict Responses consumers (including
        // Goose 1.57) must survive JSON decoding, not just match event names.
        #[derive(serde::Deserialize)]
        struct TextDelta {
            item_id: String,
            output_index: u32,
            content_index: u32,
            delta: String,
            logprobs: Vec<Value>,
        }
        let mut decoded_text = String::new();
        for frame in frames
            .iter()
            .filter(|f| f.event == "response.output_text.delta")
        {
            let delta: TextDelta = serde_json::from_value(frame.data.clone()).unwrap();
            assert_eq!(delta.item_id, frames[2].data["item"]["id"]);
            assert_eq!(delta.output_index, 0);
            assert_eq!(delta.content_index, 0);
            assert!(delta.logprobs.is_empty());
            decoded_text.push_str(&delta.delta);
        }
        assert_eq!(decoded_text, "Hello");
        assert_eq!(
            frames.last().unwrap().data["response"]["output"][0]["content"][0]["text"],
            decoded_text
        );
        assert_eq!(e.text(), "Hello");
    }

    #[test]
    fn tool_call_frames_are_sequenced_across_chunks_and_completion() {
        let mut e = ResponsesEmitter::new("resp_tool".into(), "model".into(), 100);
        let mut frames = e.start();
        assert!(e.start().is_empty());
        for (id, name, arguments) in [
            (Some("call_1"), Some("weather"), "{\"city\":"),
            (None, None, "\"Toronto\"}"),
        ] {
            let mut chunk = text_chunk("");
            chunk.choices[0].delta.tool_calls = Some(vec![crate::ToolCallDelta {
                index: 0,
                id: id.map(str::to_owned),
                tool_type: Some("function".into()),
                function: Some(crate::FunctionCallDelta {
                    name: name.map(str::to_owned),
                    arguments: Some(arguments.into()),
                }),
            }]);
            frames.extend(e.on_chunk(chunk));
        }
        frames.extend(e.finish(Some("tool_calls")));
        assert_sequences(&frames, 0);
        let added = &frames[2].data["item"];
        let arguments: String = frames
            .iter()
            .filter(|f| f.event == "response.function_call_arguments.delta")
            .map(|f| {
                assert_eq!(f.data["item_id"], added["id"]);
                assert_eq!(f.data["output_index"], 0);
                f.data["delta"].as_str().unwrap()
            })
            .collect();
        assert_eq!(arguments, r#"{"city":"Toronto"}"#);
        assert_eq!(
            frames.last().unwrap().data["response"]["output"][0]["arguments"],
            arguments
        );
        assert_eq!(added["status"], "in_progress");
        let done = frames
            .iter()
            .find(|f| f.event == "response.output_item.done")
            .unwrap();
        assert_eq!(done.data["item"]["status"], "completed");
        assert_eq!(
            frames.last().unwrap().data["response"]["output"][0]["status"],
            "completed"
        );
    }

    #[test]
    fn empty_stream_and_failure_emit_complete_sequenced_envelopes() {
        let mut empty = ResponsesEmitter::new("resp_empty".into(), "model".into(), 100);
        let frames = empty.finish(Some("stop"));
        assert_sequences(&frames, 0);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].event, "response.created");
        assert_eq!(frames[1].event, "response.in_progress");
        assert_eq!(frames[2].event, "response.completed");

        let mut failed = ResponsesEmitter::new("resp_failed".into(), "model".into(), 100);
        let error = json!({"code":"server_error", "message":"upstream disconnected"});
        let mut frames = failed.on_chunk(text_chunk("partial"));
        frames.extend(failed.fail(error.clone()));
        assert_sequences(&frames, 0);
        let failure = &frames.last().unwrap().data["response"];
        assert_eq!(failure["id"], "resp_failed");
        assert_eq!(failure["object"], "response");
        assert_eq!(failure["model"], "model");
        assert_eq!(failure["status"], "failed");
        assert_eq!(failure["error"], error);
        assert_eq!(failure["output"][0]["status"], "incomplete");
        assert_eq!(failure["output"][0]["content"][0]["text"], "partial");

        let mut early_failure = ResponsesEmitter::new("resp_early".into(), "model".into(), 100);
        let frames = early_failure.fail(error);
        assert_sequences(&frames, 0);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].event, "response.created");
        assert_eq!(frames[1].event, "response.in_progress");
        assert_eq!(frames[2].event, "response.failed");
    }

    #[test]
    fn failure_after_native_event_continues_upstream_sequence_without_local_opening() {
        let mut e = ResponsesEmitter::new("resp_native".into(), "model".into(), 100);
        let envelope = json!({"type":"response.created", "sequence_number":17,
            "response":{"id":"upstream", "output":[]}});
        e.observe_native_envelope(&envelope);
        let frames = e.fail(json!({"code":"server_error", "message":"disconnected"}));
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event, "response.failed");
        assert_sequences(&frames, 18);
        assert_eq!(envelope["sequence_number"], 17);
    }

    /// Text, then a function call, in one stream: as native upstreams send
    /// it, the message item is finished before the call's item is added, and
    /// the call's arguments are announced complete before its item is done.
    #[test]
    fn message_ends_before_a_function_call_item_and_arguments_are_finished() {
        let mut e = ResponsesEmitter::new("resp_mix".into(), "model".into(), 100);
        let mut frames = e.on_chunk(text_chunk("Checking."));
        for (id, arguments) in [(Some("call_1"), "{\"a\":"), (None, "1}")] {
            let mut chunk = text_chunk("");
            chunk.choices[0].delta.content = None;
            chunk.choices[0].delta.tool_calls = Some(vec![crate::ToolCallDelta {
                index: 0,
                id: id.map(str::to_owned),
                tool_type: Some("function".into()),
                function: Some(crate::FunctionCallDelta {
                    name: id.map(|_| "lookup".to_owned()),
                    arguments: Some(arguments.into()),
                }),
            }]);
            frames.extend(e.on_chunk(chunk));
        }
        frames.extend(e.finish(Some("tool_calls")));
        assert_sequences(&frames, 0);
        let names: Vec<&str> = frames.iter().map(|f| f.event.as_str()).collect();
        assert_eq!(
            names,
            [
                "response.created",
                "response.in_progress",
                "response.output_item.added",
                "response.content_part.added",
                "response.output_text.delta",
                "response.output_text.done",
                "response.content_part.done",
                "response.output_item.done",
                "response.output_item.added",
                "response.function_call_arguments.delta",
                "response.function_call_arguments.delta",
                "response.function_call_arguments.done",
                "response.output_item.done",
                "response.completed",
            ]
        );
        assert_eq!(frames[7].data["output_index"], 0);
        assert_eq!(frames[8].data["output_index"], 1);
        let done = &frames[11].data;
        assert_eq!(done["item_id"], frames[8].data["item"]["id"]);
        assert_eq!(done["output_index"], 1);
        assert_eq!(done["arguments"], r#"{"a":1}"#);
        assert_eq!(done["name"], "lookup");
        let output = &frames.last().unwrap().data["response"]["output"];
        assert_eq!(output[0]["content"][0]["text"], "Checking.");
        assert_eq!(output[1]["type"], "function_call");
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
                    annotations: None,
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
        assert_eq!(v["output"][0]["status"], "completed");
        assert_eq!(v["usage"]["total_tokens"], 3);

        // Items carry their status, as OpenAI's do: a cut-off message is
        // incomplete, a function call complete.
        let mut cut = resp.clone();
        cut.choices[0].finish_reason = Some("length".into());
        let v = completion_to_response_object(&cut, "resp_2", 100);
        assert_eq!(v["status"], "incomplete");
        assert_eq!(v["output"][0]["status"], "incomplete");
        let mut call = resp.clone();
        call.choices[0].message.tool_calls = Some(vec![crate::ToolCall {
            id: "call_1".into(),
            tool_type: "function".into(),
            function: crate::FunctionCall {
                name: "f".into(),
                arguments: "{}".into(),
            },
        }]);
        let v = completion_to_response_object(&call, "resp_3", 100);
        assert_eq!(v["output"][1]["type"], "function_call");
        assert_eq!(v["output"][1]["status"], "completed");
    }
}
