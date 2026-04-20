//! Streaming adapter: Responses-API SSE → chat-completions-style
//! `CompletionChunk`s.
//!
//! The Responses API streams typed events (`response.output_text.delta`,
//! `response.function_call_arguments.delta`, `response.completed`, …).
//! We fold those into the same `CompletionChunk` shape our chat
//! completions path already produces so the rest of the app can
//! consume either stream identically.
//!
//! The loop mirrors `openai.rs`'s line-buffered SSE reader
//! (`data: { ... }` frames, blank-line separated), but we branch on
//! the JSON `"type"` discriminator rather than the `event:` header
//! since the payload carries the event type redundantly.

use std::collections::HashMap;
use std::pin::Pin;

use futures::stream::{Stream, StreamExt};
use serde::Deserialize;
use tracing::warn;

use super::types::{ContentItem, OutputItem, ResponsesSseEnvelope, ResponsesUsage};
use crate::{ChunkChoice, ChunkDelta, CompletionChunk, FunctionCallDelta, ToolCallDelta};
use lr_types::{AppError, AppResult};

/// Transform a raw `/responses` SSE byte stream into our
/// `CompletionChunk` stream. `provider_id` and `model` are stamped on
/// every emitted chunk so downstream consumers don't have to
/// reconstruct them.
pub fn responses_to_completion_chunks<S, B>(
    bytes_stream: S,
    provider_id: String,
    model_fallback: String,
) -> Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>
where
    S: Stream<Item = Result<B, reqwest::Error>> + Send + 'static,
    B: AsRef<[u8]>,
{
    let mut state = StreamState::new(provider_id, model_fallback);
    let mut buffer = String::new();

    let stream = bytes_stream.flat_map(move |chunk_result: Result<B, reqwest::Error>| {
        let events: Vec<AppResult<CompletionChunk>> = match chunk_result {
            Ok(bytes) => {
                buffer.push_str(&String::from_utf8_lossy(bytes.as_ref()));
                drain_frames(&mut buffer, &mut state)
            }
            Err(e) => {
                vec![Err(AppError::Provider(
                    crate::http_client::format_stream_error(&e),
                ))]
            }
        };
        futures::stream::iter(events)
    });

    Box::pin(stream)
}

/// Drain complete SSE frames from `buf`, decoding each and forwarding
/// the state machine. Partial frames remain in `buf` for the next
/// byte chunk.
fn drain_frames(buf: &mut String, state: &mut StreamState) -> Vec<AppResult<CompletionChunk>> {
    let mut emitted: Vec<AppResult<CompletionChunk>> = Vec::new();

    // A single "frame" is one or more header lines (`event:` / `data:`)
    // terminated by a blank line. We scan for `\n\n` boundaries and
    // process each complete frame in turn.
    while let Some(boundary) = find_frame_boundary(buf) {
        let frame = buf[..boundary].to_string();
        // Skip past the terminator (either `\n\n` or `\r\n\r\n`).
        let term_len = if buf[boundary..].starts_with("\r\n\r\n") {
            4
        } else {
            2
        };
        buf.replace_range(..boundary + term_len, "");

        // Concatenate all `data:` lines in the frame (SSE spec allows
        // multi-line data payloads, joined with `\n`).
        let mut data_payload = String::new();
        for line in frame.lines() {
            if let Some(rest) = line.strip_prefix("data:") {
                if !data_payload.is_empty() {
                    data_payload.push('\n');
                }
                data_payload.push_str(rest.trim_start());
            }
        }
        if data_payload.is_empty() {
            continue;
        }

        match serde_json::from_str::<ResponsesSseEnvelope>(&data_payload) {
            Ok(env) => {
                for out in state.on_event(env, &data_payload) {
                    emitted.push(out);
                }
            }
            Err(e) => warn!(
                "Failed to parse /responses SSE frame ({}): {}",
                e, data_payload
            ),
        }
    }

    emitted
}

/// Locate the byte offset of the next `\n\n` (or `\r\n\r\n`) frame
/// boundary in `buf`, if present.
fn find_frame_boundary(buf: &str) -> Option<usize> {
    // Try `\r\n\r\n` first so we don't greedily split on a lone `\n`
    // that's actually part of a CRLF pair.
    if let Some(i) = buf.find("\r\n\r\n") {
        return Some(i);
    }
    buf.find("\n\n")
}

// ============================================================================
// Event-to-chunk state machine
// ============================================================================

struct StreamState {
    provider_id: String,
    model: String,
    response_id: String,
    created: i64,
    /// Maps a Responses-API `item_id` → the `tool_calls[]` index we
    /// assigned in the emitted chat-completions delta stream.
    tool_call_slots: HashMap<String, u32>,
    next_tool_index: u32,
    opening_emitted: bool,
}

impl StreamState {
    fn new(provider_id: String, model_fallback: String) -> Self {
        Self {
            provider_id,
            model: model_fallback,
            response_id: String::new(),
            created: 0,
            tool_call_slots: HashMap::new(),
            next_tool_index: 0,
            opening_emitted: false,
        }
    }

    fn base_chunk(&self) -> CompletionChunk {
        CompletionChunk {
            id: if self.response_id.is_empty() {
                format!("chatcmpl-{}", self.provider_id)
            } else {
                self.response_id.clone()
            },
            object: "chat.completion.chunk".into(),
            created: self.created,
            model: self.model.clone(),
            choices: vec![],
            extensions: None,
        }
    }

    fn on_event(
        &mut self,
        env: ResponsesSseEnvelope,
        raw: &str,
    ) -> Vec<AppResult<CompletionChunk>> {
        // Map event by `type` discriminator. The long match below is
        // intentional — every branch matches one Responses-API event
        // we care about, all fallthrough to `ignore_other` so we
        // skip unknown variants without ever dropping the stream.
        match env.event_type.as_str() {
            "response.created" => self.on_created(raw),
            "response.output_item.added" => self.on_output_item_added(env),
            "response.output_text.delta" => self.on_output_text_delta(env),
            "response.function_call_arguments.delta" | "response.custom_tool_call_input.delta" => {
                self.on_tool_call_args_delta(env)
            }
            "response.output_item.done" => self.on_output_item_done(env),
            "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => {
                self.on_reasoning_delta(env)
            }
            "response.completed" => self.on_completed(raw),
            "response.failed" | "response.incomplete" => self.on_failed(env),
            // `response.created`'s sibling events (`in_progress`,
            // `content_part.added/done`, etc.) are informational — we
            // skip them rather than noisily emit empty chunks.
            _ => vec![],
        }
    }

    fn on_created(&mut self, raw: &str) -> Vec<AppResult<CompletionChunk>> {
        // Extract `response.id`, `response.model`, `response.created_at`
        // if present. We tolerate missing fields; if any is absent we
        // fall back to the constructor defaults.
        if let Ok(CreatedPayload { response }) = serde_json::from_str::<CreatedPayload>(raw) {
            if !response.id.is_empty() {
                self.response_id = response.id;
            }
            if let Some(m) = response.model {
                self.model = m;
            }
            if let Some(c) = response.created_at {
                self.created = c;
            }
        }

        // Emit the opening chunk with role=assistant so downstream
        // consumers see the same shape as OpenAI's chat-completions
        // first frame.
        self.opening_emitted = true;
        vec![Ok(CompletionChunk {
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    role: Some("assistant".into()),
                    content: None,
                    tool_calls: None,
                    reasoning_content: None,
                },
                finish_reason: None,
            }],
            ..self.base_chunk()
        })]
    }

    fn on_output_item_added(
        &mut self,
        env: ResponsesSseEnvelope,
    ) -> Vec<AppResult<CompletionChunk>> {
        // When a new function_call item appears, pre-assign it an
        // index in our tool_calls[] array so subsequent argument
        // deltas can reference the same slot consistently.
        let Some(item) = env.item else {
            return vec![];
        };
        let item_obj = match item {
            serde_json::Value::Object(m) => m,
            _ => return vec![],
        };
        let kind = item_obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if kind != "function_call" && kind != "custom_tool_call" {
            return vec![];
        }
        let item_id = env
            .item_id
            .or_else(|| {
                item_obj
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let call_id = item_obj
            .get("call_id")
            .and_then(|v| v.as_str())
            .unwrap_or(&item_id)
            .to_string();
        let name = item_obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let index = *self.tool_call_slots.entry(item_id).or_insert_with(|| {
            let i = self.next_tool_index;
            self.next_tool_index += 1;
            i
        });

        vec![Ok(CompletionChunk {
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    role: None,
                    content: None,
                    tool_calls: Some(vec![ToolCallDelta {
                        index,
                        id: Some(call_id),
                        tool_type: Some("function".into()),
                        function: Some(FunctionCallDelta {
                            name: Some(name),
                            arguments: None,
                        }),
                    }]),
                    reasoning_content: None,
                },
                finish_reason: None,
            }],
            ..self.base_chunk()
        })]
    }

    fn on_output_text_delta(
        &mut self,
        env: ResponsesSseEnvelope,
    ) -> Vec<AppResult<CompletionChunk>> {
        let Some(delta) = env.delta else {
            return vec![];
        };
        if delta.is_empty() {
            return vec![];
        }
        vec![Ok(CompletionChunk {
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    role: None,
                    content: Some(delta),
                    tool_calls: None,
                    reasoning_content: None,
                },
                finish_reason: None,
            }],
            ..self.base_chunk()
        })]
    }

    fn on_tool_call_args_delta(
        &mut self,
        env: ResponsesSseEnvelope,
    ) -> Vec<AppResult<CompletionChunk>> {
        let Some(delta) = env.delta else {
            return vec![];
        };
        let Some(item_id) = env.item_id else {
            return vec![];
        };
        let index = *self.tool_call_slots.entry(item_id).or_insert_with(|| {
            let i = self.next_tool_index;
            self.next_tool_index += 1;
            i
        });
        vec![Ok(CompletionChunk {
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    role: None,
                    content: None,
                    tool_calls: Some(vec![ToolCallDelta {
                        index,
                        id: None,
                        tool_type: None,
                        function: Some(FunctionCallDelta {
                            name: None,
                            arguments: Some(delta),
                        }),
                    }]),
                    reasoning_content: None,
                },
                finish_reason: None,
            }],
            ..self.base_chunk()
        })]
    }

    fn on_output_item_done(
        &mut self,
        _env: ResponsesSseEnvelope,
    ) -> Vec<AppResult<CompletionChunk>> {
        // Nothing to emit — function_call completion is already
        // signaled by `response.completed`'s finish_reason derivation.
        vec![]
    }

    fn on_reasoning_delta(&mut self, env: ResponsesSseEnvelope) -> Vec<AppResult<CompletionChunk>> {
        let Some(delta) = env.delta else {
            return vec![];
        };
        vec![Ok(CompletionChunk {
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    role: None,
                    content: None,
                    tool_calls: None,
                    reasoning_content: Some(delta),
                },
                finish_reason: None,
            }],
            ..self.base_chunk()
        })]
    }

    fn on_completed(&mut self, raw: &str) -> Vec<AppResult<CompletionChunk>> {
        // Determine finish_reason by peeking at the final `response.output`:
        // if any item is a function_call, this turn stopped to invoke
        // tools. Otherwise it's a plain "stop".
        let finish_reason = match serde_json::from_str::<CompletedPayload>(raw) {
            Ok(p) => {
                let has_tool = p
                    .response
                    .output
                    .iter()
                    .any(|item| matches!(item, OutputItem::FunctionCall { .. }));
                if has_tool {
                    "tool_calls".to_string()
                } else {
                    "stop".to_string()
                }
            }
            Err(_) => "stop".to_string(),
        };

        let extensions = serde_json::from_str::<CompletedPayload>(raw)
            .ok()
            .and_then(|p| p.response.usage)
            .map(|u| {
                let mut map = std::collections::HashMap::new();
                map.insert(
                    "usage".to_string(),
                    serde_json::json!({
                        "prompt_tokens": u.input_tokens,
                        "completion_tokens": u.output_tokens,
                        "total_tokens": if u.total_tokens > 0 {
                            u.total_tokens
                        } else {
                            u.input_tokens + u.output_tokens
                        },
                    }),
                );
                map
            });

        vec![Ok(CompletionChunk {
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    role: None,
                    content: None,
                    tool_calls: None,
                    reasoning_content: None,
                },
                finish_reason: Some(finish_reason),
            }],
            extensions,
            ..self.base_chunk()
        })]
    }

    fn on_failed(&mut self, env: ResponsesSseEnvelope) -> Vec<AppResult<CompletionChunk>> {
        let msg = env
            .error
            .as_ref()
            .and_then(|e| e.message.clone())
            .unwrap_or_else(|| "Responses API stream failed".to_string());
        let code = env
            .error
            .as_ref()
            .and_then(|e| e.code.clone())
            .unwrap_or_default();
        if code == "context_length_exceeded" || msg.contains("maximum context length") {
            vec![Err(AppError::ContextLengthExceeded {
                max: None,
                requested: None,
            })]
        } else {
            vec![Err(AppError::Provider(msg))]
        }
    }
}

// Payloads for event-specific decoding that needs more than the
// envelope. Defined inline (not in types.rs) because they're only
// meaningful during streaming.

#[derive(Debug, Deserialize)]
struct CreatedPayload {
    response: CreatedResponse,
}

#[derive(Debug, Deserialize)]
struct CreatedResponse {
    #[serde(default)]
    id: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    created_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CompletedPayload {
    response: CompletedResponse,
}

#[derive(Debug, Deserialize)]
struct CompletedResponse {
    #[serde(default)]
    output: Vec<OutputItem>,
    #[serde(default)]
    usage: Option<ResponsesUsage>,
}

// The `ContentItem` import is re-exported so the fixture tests can
// construct message-with-output_text items without pulling from
// `super::types` directly.
#[allow(unused_imports)]
use ContentItem as _;

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    fn collect_chunks(raw_sse: &'static str) -> Vec<AppResult<CompletionChunk>> {
        // Chunk the input into 8-byte pieces so the line buffer actually
        // has to stitch partial frames — mirrors real network behavior.
        let bytes_stream = stream::iter(
            raw_sse
                .as_bytes()
                .chunks(8)
                .map(|c| Ok::<_, reqwest::Error>(c.to_vec()))
                .collect::<Vec<_>>(),
        );
        let stream = responses_to_completion_chunks(bytes_stream, "openai".into(), "gpt-4o".into());
        // Block on collecting. tokio runtime is available via lr-providers dev deps.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut out = Vec::new();
            let mut s = stream;
            while let Some(ev) = s.next().await {
                out.push(ev);
            }
            out
        })
    }

    #[test]
    fn simple_text_turn_emits_role_then_deltas_then_finish() {
        let sse = "event: response.created\n\
data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\",\"model\":\"gpt-4o\",\"created_at\":100}}\n\
\n\
event: response.output_text.delta\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\
\n\
event: response.output_text.delta\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\
\n\
event: response.completed\n\
data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"output\":[{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}],\"usage\":{\"input_tokens\":2,\"output_tokens\":1,\"total_tokens\":3}}}\n\
\n\
";
        let chunks: Vec<_> = collect_chunks(sse)
            .into_iter()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(chunks.len(), 4, "role + 2 deltas + finish");
        assert_eq!(
            chunks[0].choices[0].delta.role.as_deref(),
            Some("assistant")
        );
        assert_eq!(chunks[1].choices[0].delta.content.as_deref(), Some("Hel"));
        assert_eq!(chunks[2].choices[0].delta.content.as_deref(), Some("lo"));
        assert_eq!(chunks[3].choices[0].finish_reason.as_deref(), Some("stop"));
        assert_eq!(chunks[3].id, "resp_1");
    }

    #[test]
    fn tool_call_turn_emits_function_call_deltas() {
        let sse = "event: response.created\n\
data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_tc\"}}\n\
\n\
event: response.output_item.added\n\
data: {\"type\":\"response.output_item.added\",\"item_id\":\"fc_1\",\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_abc\",\"name\":\"search\"}}\n\
\n\
event: response.function_call_arguments.delta\n\
data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"{\\\"q\"}\n\
\n\
event: response.function_call_arguments.delta\n\
data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"\\\":\\\"rust\\\"}\"}\n\
\n\
event: response.completed\n\
data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_tc\",\"output\":[{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_abc\",\"name\":\"search\",\"arguments\":\"{\\\"q\\\":\\\"rust\\\"}\"}]}}\n\
\n\
";
        let chunks: Vec<_> = collect_chunks(sse)
            .into_iter()
            .collect::<Result<_, _>>()
            .unwrap();
        // Expected: role chunk, item_added (function_call registration), 2 arg deltas, finish.
        assert!(chunks.len() >= 5, "got {} chunks", chunks.len());
        let added = chunks
            .iter()
            .find_map(|c| c.choices[0].delta.tool_calls.as_ref())
            .expect("some chunk carries a tool_call delta");
        assert_eq!(added[0].index, 0);
        assert_eq!(added[0].id.as_deref(), Some("call_abc"));
        assert_eq!(
            chunks.last().unwrap().choices[0].finish_reason.as_deref(),
            Some("tool_calls")
        );
    }
}
