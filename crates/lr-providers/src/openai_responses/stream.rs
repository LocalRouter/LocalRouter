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

    // Some compatible servers close immediately after their final data line.
    // A final empty line flushes that frame without changing complete frames.
    let lines = crate::sse_lines::lines(bytes_stream)
        .chain(futures::stream::once(async { Ok(String::new()) }));
    let stream = lines.flat_map(move |chunk_result| {
        let events: Vec<AppResult<CompletionChunk>> = match chunk_result {
            Ok(line) => {
                buffer.push_str(&line);
                buffer.push('\n');
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
                // Parse the raw frame JSON once so we can attach it to
                // every chunk the state machine emits for this event.
                // Adapters that speak native Responses SSE (today:
                // `routes/responses.rs`) pass these envelopes through
                // verbatim instead of re-synthesizing them from the
                // lossy ChunkDelta view.
                let raw_value = serde_json::from_str::<serde_json::Value>(&data_payload).ok();
                for out in state.on_event(env, &data_payload) {
                    let out = match (out, raw_value.clone()) {
                        (Ok(mut chunk), Some(raw)) => {
                            let mut ext = chunk.extensions.unwrap_or_default();
                            ext.insert(NATIVE_RESPONSES_SSE_EXT_KEY.to_string(), raw);
                            chunk.extensions = Some(ext);
                            Ok(chunk)
                        }
                        (out, _) => out,
                    };
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

/// Key under which `responses_to_completion_chunks` stashes the raw
/// Responses API SSE envelope JSON on each `CompletionChunk`'s
/// `extensions`. Adapters that speak native Responses SSE pass these
/// through verbatim instead of re-serializing via `ResponsesEmitter`.
pub const NATIVE_RESPONSES_SSE_EXT_KEY: &str = "__native_responses_sse_envelope";

/// Locate the byte offset of the next `\n\n` (or `\r\n\r\n`) frame
/// boundary in `buf`, if present.
fn find_frame_boundary(buf: &str) -> Option<usize> {
    // Either terminator may occur first in a mixed-ending stream. Choosing
    // CRLF unconditionally would merge and discard earlier LF-only frames.
    [buf.find("\r\n\r\n"), buf.find("\n\n")]
        .into_iter()
        .flatten()
        .min()
}

// ============================================================================
// Event-to-chunk state machine
// ============================================================================

#[derive(Default)]
struct ToolSlot {
    index: u32,
    arguments: String,
    emitted: usize,
    header: Option<(String, String)>,
}

struct StreamState {
    provider_id: String,
    model: String,
    response_id: String,
    created: i64,
    /// Maps a Responses-API `item_id` → the `tool_calls[]` index we
    /// assigned in the emitted chat-completions delta stream.
    tool_call_slots: HashMap<String, ToolSlot>,
    tool_output_ids: HashMap<i64, String>,
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
            tool_output_ids: HashMap::new(),
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
            usage: None,
            provider: None,
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
            "response.function_call_arguments.done" | "response.custom_tool_call_input.done" => {
                self.on_tool_call_args_done(env)
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
                    annotations: None,
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

    fn tool_key(&self, item_id: &str, call_id: &str) -> String {
        if self.tool_call_slots.contains_key(item_id) {
            return item_id.to_string();
        }
        self.tool_call_slots
            .iter()
            .find(|(_, slot)| slot.header.as_ref().is_some_and(|(id, _)| id == call_id))
            .map(|(key, _)| key.clone())
            .unwrap_or_else(|| item_id.to_string())
    }

    fn slot(&mut self, item_id: String, output_index: Option<i64>) -> &mut ToolSlot {
        if let Some(index) = output_index {
            self.tool_output_ids.insert(index, item_id.clone());
        }
        self.tool_call_slots.entry(item_id).or_insert_with(|| {
            let index = self.next_tool_index;
            self.next_tool_index += 1;
            ToolSlot {
                index,
                ..Default::default()
            }
        })
    }

    fn tool_chunk(&self, calls: Vec<ToolCallDelta>) -> CompletionChunk {
        CompletionChunk {
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    annotations: None,
                    role: None,
                    content: None,
                    tool_calls: Some(calls),
                    reasoning_content: None,
                },
                finish_reason: None,
            }],
            ..self.base_chunk()
        }
    }

    fn take_tool_delta(
        slot: &mut ToolSlot,
        header: Option<(String, String)>,
    ) -> AppResult<Option<ToolCallDelta>> {
        let announce = slot.header.is_none() && header.is_some();
        if let Some(header) = header {
            if slot
                .header
                .as_ref()
                .is_some_and(|previous| previous != &header)
            {
                return Err(AppError::Provider(
                    "Responses tool identity changed after streaming".into(),
                ));
            }
            slot.header = Some(header);
        }
        let Some((call_id, name)) = &slot.header else {
            return Ok(None);
        };
        let suffix = &slot.arguments[slot.emitted..];
        if !announce && suffix.is_empty() {
            return Ok(None);
        }
        let delta = ToolCallDelta {
            index: slot.index,
            id: announce.then(|| call_id.clone()),
            tool_type: announce.then(|| "function".into()),
            function: Some(FunctionCallDelta {
                name: announce.then(|| name.clone()),
                arguments: (!suffix.is_empty()).then(|| suffix.to_string()),
            }),
        };
        slot.emitted = slot.arguments.len();
        Ok(Some(delta))
    }

    fn complete_arguments(slot: &mut ToolSlot, arguments: &str) -> AppResult<()> {
        if !arguments.starts_with(&slot.arguments) {
            return Err(AppError::Provider(
                "Responses tool arguments changed after streaming".into(),
            ));
        }
        slot.arguments = arguments.to_string();
        Ok(())
    }

    fn tool_item(
        &mut self,
        env: ResponsesSseEnvelope,
        terminal: bool,
    ) -> AppResult<Option<ToolCallDelta>> {
        let Some(item) = env.item.as_ref().and_then(|item| item.as_object()) else {
            return Ok(None);
        };
        let kind = item
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if !matches!(kind, "function_call" | "custom_tool_call") {
            return Ok(None);
        }
        let item_id = env
            .item_id
            .as_deref()
            .or_else(|| item.get("id").and_then(|value| value.as_str()))
            .or_else(|| item.get("call_id").and_then(|value| value.as_str()))
            .ok_or_else(|| AppError::Provider("Responses tool item has no identity".into()))?;
        let call_id = item
            .get("call_id")
            .and_then(|value| value.as_str())
            .unwrap_or(item_id);
        let name = item
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let key = self.tool_key(item_id, call_id);
        let slot = self.slot(key, env.output_index);
        if let Some(arguments) = item
            .get(if kind == "function_call" {
                "arguments"
            } else {
                "input"
            })
            .and_then(|value| value.as_str())
        {
            if terminal || !arguments.is_empty() || slot.arguments.is_empty() {
                Self::complete_arguments(slot, arguments)?;
            }
        }
        Self::take_tool_delta(slot, Some((call_id.to_string(), name.to_string())))
    }

    fn on_output_item_added(
        &mut self,
        env: ResponsesSseEnvelope,
    ) -> Vec<AppResult<CompletionChunk>> {
        match self.tool_item(env, false) {
            Ok(Some(delta)) => vec![Ok(self.tool_chunk(vec![delta]))],
            Ok(None) => vec![],
            Err(error) => vec![Err(error)],
        }
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
                    annotations: None,
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

    fn argument_item_id(&self, env: &ResponsesSseEnvelope) -> Option<String> {
        env.item_id.clone().or_else(|| {
            env.output_index
                .and_then(|index| self.tool_output_ids.get(&index).cloned())
        })
    }

    fn on_tool_call_args_delta(
        &mut self,
        env: ResponsesSseEnvelope,
    ) -> Vec<AppResult<CompletionChunk>> {
        let Some(item_id) = self.argument_item_id(&env) else {
            return vec![];
        };
        let Some(delta) = env.delta else {
            return vec![];
        };
        let slot = self.slot(item_id, env.output_index);
        slot.arguments.push_str(&delta);
        match Self::take_tool_delta(slot, None) {
            Ok(Some(delta)) => vec![Ok(self.tool_chunk(vec![delta]))],
            Ok(None) => vec![],
            Err(error) => vec![Err(error)],
        }
    }

    fn on_tool_call_args_done(
        &mut self,
        env: ResponsesSseEnvelope,
    ) -> Vec<AppResult<CompletionChunk>> {
        let Some(item_id) = self.argument_item_id(&env) else {
            return vec![];
        };
        let Some(arguments) = env.arguments.or(env.input) else {
            return vec![];
        };
        let slot = self.slot(item_id, env.output_index);
        match Self::complete_arguments(slot, &arguments)
            .and_then(|_| Self::take_tool_delta(slot, None))
        {
            Ok(Some(delta)) => vec![Ok(self.tool_chunk(vec![delta]))],
            Ok(None) => vec![],
            Err(error) => vec![Err(error)],
        }
    }

    fn on_output_item_done(
        &mut self,
        env: ResponsesSseEnvelope,
    ) -> Vec<AppResult<CompletionChunk>> {
        match self.tool_item(env, true) {
            Ok(Some(delta)) => vec![Ok(self.tool_chunk(vec![delta]))],
            Ok(None) => vec![],
            Err(error) => vec![Err(error)],
        }
    }

    fn on_reasoning_delta(&mut self, env: ResponsesSseEnvelope) -> Vec<AppResult<CompletionChunk>> {
        let Some(delta) = env.delta else {
            return vec![];
        };
        vec![Ok(CompletionChunk {
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    annotations: None,
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
        let payload = serde_json::from_str::<CompletedPayload>(raw).ok();
        let mut calls = Vec::new();
        if let Some(payload) = &payload {
            for (index, item) in payload.response.output.iter().enumerate() {
                if let OutputItem::FunctionCall {
                    id,
                    call_id,
                    name,
                    arguments,
                } = item
                {
                    let key = self.tool_key(id.as_deref().unwrap_or(call_id), call_id);
                    let slot = self.slot(key, Some(index as i64));
                    match Self::complete_arguments(slot, arguments).and_then(|_| {
                        Self::take_tool_delta(slot, Some((call_id.clone(), name.clone())))
                    }) {
                        Ok(Some(delta)) => calls.push(delta),
                        Ok(None) => {}
                        Err(error) => return vec![Err(error)],
                    }
                }
            }
        }
        let finish_reason = if payload.as_ref().is_some_and(|payload| {
            payload
                .response
                .output
                .iter()
                .any(|item| matches!(item, OutputItem::FunctionCall { .. }))
        }) {
            "tool_calls"
        } else {
            "stop"
        };
        let usage = payload.and_then(|payload| payload.response.usage);

        let extensions = usage.as_ref().map(|u| {
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
                    annotations: None,
                    role: None,
                    content: None,
                    tool_calls: (!calls.is_empty()).then_some(calls),
                    reasoning_content: None,
                },
                finish_reason: Some(finish_reason.to_string()),
            }],
            extensions,
            usage: usage.as_ref().map(ResponsesUsage::to_token_usage),
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

    #[tokio::test]
    async fn unicode_and_mixed_line_endings_survive_every_byte_boundary() {
        let sse = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"é\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"🌍\"}\r\n\r\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"中\"}",
        );
        for cuts in crate::sse_lines::test_support::split_variants(sse) {
            let chunks: Vec<_> = responses_to_completion_chunks(
                crate::sse_lines::test_support::reads_split_at(sse, &cuts),
                "fixture".into(),
                "fixture".into(),
            )
            .collect()
            .await;
            let text: String = chunks
                .into_iter()
                .map(|chunk| {
                    chunk
                        .unwrap()
                        .choices
                        .into_iter()
                        .filter_map(|choice| choice.delta.content)
                        .collect::<String>()
                })
                .collect();
            assert_eq!(text, "é🌍中", "cuts: {cuts:?}");
        }
    }

    #[test]
    fn mixed_frame_boundaries_are_processed_in_wire_order() {
        let mut buffer = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"first\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"second\"}\r\n\r\n",
        )
        .to_owned();
        let mut state = StreamState::new("fixture".into(), "fixture".into());
        let text: String = drain_frames(&mut buffer, &mut state)
            .into_iter()
            .map(|chunk| {
                chunk
                    .unwrap()
                    .choices
                    .into_iter()
                    .filter_map(|choice| choice.delta.content)
                    .collect::<String>()
            })
            .collect();
        assert_eq!(text, "firstsecond");
        assert!(buffer.is_empty());
    }

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

    #[test]
    fn chunks_carry_raw_sse_envelope_under_native_key() {
        // Every emitted chunk should have the upstream SSE envelope
        // attached verbatim under `NATIVE_RESPONSES_SSE_EXT_KEY` so
        // `routes/responses.rs` can stream it through unchanged for
        // ChatGPT Plus (where reasoning deltas + encrypted content
        // would otherwise be lost to the ChatCompletion translation).
        let sse = "event: response.output_text.delta\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\",\"response_id\":\"resp_1\"}\n\
\n\
";
        let chunks: Vec<_> = collect_chunks(sse)
            .into_iter()
            .collect::<Result<_, _>>()
            .unwrap();
        let envelope = chunks
            .iter()
            .find_map(|c| {
                c.extensions
                    .as_ref()
                    .and_then(|ext| ext.get(NATIVE_RESPONSES_SSE_EXT_KEY))
            })
            .expect("at least one chunk carries the raw envelope");
        assert_eq!(
            envelope["type"].as_str(),
            Some("response.output_text.delta")
        );
        assert_eq!(envelope["delta"].as_str(), Some("hi"));
        assert_eq!(envelope["response_id"].as_str(), Some("resp_1"));
    }

    /// `response.completed` carries the turn's usage; it is mapped onto the
    /// finish chunk, including cached input and reasoning output.
    #[test]
    fn completed_usage_is_reported_on_the_finish_chunk() {
        let sse = "event: response.created\n\
data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_u\",\"model\":\"gpt-5.5\",\"created_at\":100}}\n\
\n\
event: response.output_item.added\n\
data: {\"type\":\"response.output_item.added\",\"item_id\":\"fc_1\",\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"record\"}}\n\
\n\
event: response.function_call_arguments.delta\n\
data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"{}\"}\n\
\n\
event: response.completed\n\
data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_u\",\"output\":[{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"record\",\"arguments\":\"{}\"}],\"usage\":{\"input_tokens\":1200,\"input_tokens_details\":{\"cached_tokens\":1024},\"output_tokens\":87,\"output_tokens_details\":{\"reasoning_tokens\":64},\"total_tokens\":1287}}}\n\
\n\
";
        let chunks: Vec<_> = collect_chunks(sse)
            .into_iter()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(chunks.len(), 4, "role, tool call, args, finish");
        assert!(chunks[..3].iter().all(|c| c.usage.is_none()));
        let last = chunks.last().unwrap();
        assert_eq!(last.choices[0].finish_reason.as_deref(), Some("tool_calls"));
        let usage = last.usage.as_ref().expect("usage on the finish chunk");
        assert_eq!(
            (
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.total_tokens
            ),
            (1200, 87, 1287)
        );
        assert_eq!(
            usage.prompt_tokens_details.as_ref().unwrap().cached_tokens,
            Some(1024)
        );
        assert_eq!(
            usage
                .completion_tokens_details
                .as_ref()
                .unwrap()
                .reasoning_tokens,
            Some(64)
        );
        // The extension the native Responses route reads is unchanged.
        let ext = last.extensions.as_ref().unwrap();
        assert_eq!(ext["usage"]["prompt_tokens"], 1200);
        assert_eq!(ext["usage"]["completion_tokens"], 87);
    }

    fn translate_events(events: Vec<serde_json::Value>) -> Vec<AppResult<CompletionChunk>> {
        let wire = events
            .iter()
            .map(|event| format!("data: {event}\r\n\r\n"))
            .collect::<String>();
        let bytes = stream::iter(
            wire.as_bytes()
                .chunks(7)
                .map(|bytes| Ok::<_, reqwest::Error>(bytes.to_vec()))
                .collect::<Vec<_>>(),
        );
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(
                responses_to_completion_chunks(bytes, "fixture".into(), "fixture".into()).collect(),
            )
    }

    fn tool_item(id: &str, call_id: &str, arguments: &str) -> serde_json::Value {
        serde_json::json!({"type":"function_call","id":id,"call_id":call_id,"name":"read_section","arguments":arguments})
    }

    fn projected_tools(
        chunks: &[CompletionChunk],
    ) -> std::collections::BTreeMap<u32, (String, String, String)> {
        let mut tools = std::collections::BTreeMap::<u32, (String, String, String)>::new();
        for call in chunks
            .iter()
            .flat_map(|chunk| &chunk.choices)
            .flat_map(|choice| choice.delta.tool_calls.iter().flatten())
        {
            let tool = tools.entry(call.index).or_default();
            if let Some(id) = &call.id {
                assert!(tool.0.is_empty(), "call identity emitted twice");
                tool.0 = id.clone();
            }
            if let Some(function) = &call.function {
                if let Some(name) = &function.name {
                    assert!(tool.1.is_empty(), "call name emitted twice");
                    tool.1 = name.clone();
                }
                if let Some(arguments) = &function.arguments {
                    tool.2.push_str(arguments);
                }
            }
        }
        tools
    }

    #[test]
    fn completed_response_supplies_parallel_arguments_missing_from_deltas() {
        let first = tool_item("fc_first", "call_first", r#"{"ref":"docs/a.md#/a"}"#);
        let second = tool_item("fc_second", "call_second", r#"{"ref":"docs/b.md#/b"}"#);
        for include_added in [false, true] {
            let mut events = Vec::new();
            if include_added {
                for (index, item) in [&first, &second].into_iter().enumerate() {
                    let mut item = item.clone();
                    item["arguments"] = serde_json::json!("");
                    events.push(serde_json::json!({"type":"response.output_item.added","output_index":index,"item":item}));
                }
            }
            events.push(serde_json::json!({"type":"response.completed","response":{"output":[first, second],"usage":{"input_tokens":91,"output_tokens":56,"total_tokens":147}}}));
            let chunks: Vec<_> = translate_events(events)
                .into_iter()
                .collect::<Result<_, _>>()
                .unwrap();
            let tools = projected_tools(&chunks);
            assert_eq!(tools.len(), 2);
            assert_eq!(
                tools[&0],
                (
                    "call_first".into(),
                    "read_section".into(),
                    r#"{"ref":"docs/a.md#/a"}"#.into()
                )
            );
            assert_eq!(
                tools[&1],
                (
                    "call_second".into(),
                    "read_section".into(),
                    r#"{"ref":"docs/b.md#/b"}"#.into()
                )
            );
            assert_eq!(
                chunks.last().unwrap().choices[0].finish_reason.as_deref(),
                Some("tool_calls")
            );
            assert_eq!(
                chunks.last().unwrap().usage.as_ref().unwrap().total_tokens,
                147
            );
            assert_eq!(
                chunks
                    .iter()
                    .filter(|chunk| chunk
                        .extensions
                        .as_ref()
                        .and_then(|ext| ext.get(NATIVE_RESPONSES_SSE_EXT_KEY))
                        .is_some_and(|env| env["type"] == "response.completed"))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn done_variants_flush_only_missing_argument_suffix_once() {
        let arguments = r#"{"ref":"docs/世界.md#/a"}"#;
        let item = tool_item("fc", "call", arguments);
        let events = vec![
            serde_json::json!({"type":"response.output_item.added","output_index":3,"item":tool_item("fc","call","")}),
            serde_json::json!({"type":"response.function_call_arguments.delta","output_index":3,"delta":"{\"ref\":\"docs/"}),
            serde_json::json!({"type":"response.function_call_arguments.done","item_id":"fc","arguments":arguments}),
            serde_json::json!({"type":"response.output_item.done","output_index":3,"item":item}),
            serde_json::json!({"type":"response.completed","response":{"output":[item]}}),
        ];
        let chunks: Vec<_> = translate_events(events)
            .into_iter()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(projected_tools(&chunks)[&0].2, arguments);
    }

    #[test]
    fn completed_item_and_added_snapshot_supply_arguments_without_delta_events() {
        for terminal in ["response.output_item.added", "response.output_item.done"] {
            let arguments = r#"{"ref":"docs/é.md#/a"}"#;
            let item = tool_item("fc", "call", arguments);
            let chunks: Vec<_> = translate_events(vec![
                serde_json::json!({"type":terminal,"item":item}),
                serde_json::json!({"type":"response.completed","response":{"output":[item]}}),
            ])
            .into_iter()
            .collect::<Result<_, _>>()
            .unwrap();
            assert_eq!(projected_tools(&chunks)[&0].2, arguments);
        }
    }

    #[test]
    fn final_call_id_reuses_its_announced_slot_when_item_id_is_omitted() {
        let mut completed = tool_item("fc", "call", r#"{"ref":"a"}"#);
        completed.as_object_mut().unwrap().remove("id");
        let chunks: Vec<_> = translate_events(vec![
            serde_json::json!({"type":"response.output_item.added","item":tool_item("fc","call","")}),
            serde_json::json!({"type":"response.completed","response":{"output":[completed]}}),
        ]).into_iter().collect::<Result<_,_>>().unwrap();
        let tools = projected_tools(&chunks);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[&0].2, r#"{"ref":"a"}"#);
    }

    #[test]
    fn arguments_before_the_tool_header_are_buffered_until_identity_is_known() {
        let arguments = r#"{"ref":"a"}"#;
        let chunks: Vec<_> = translate_events(vec![
            serde_json::json!({"type":"response.function_call_arguments.delta","item_id":"fc","delta":arguments}),
            serde_json::json!({"type":"response.output_item.added","item":tool_item("fc","call","")}),
            serde_json::json!({"type":"response.completed","response":{"output":[tool_item("fc","call",arguments)]}}),
        ]).into_iter().collect::<Result<_,_>>().unwrap();
        assert_eq!(projected_tools(&chunks)[&0].2, arguments);
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn custom_tool_done_input_preserves_the_existing_chat_projection() {
        let chunks: Vec<_> = translate_events(vec![
            serde_json::json!({"type":"response.output_item.added","item":{"type":"custom_tool_call","id":"ct","call_id":"call","name":"code","input":""}}),
            serde_json::json!({"type":"response.custom_tool_call_input.delta","item_id":"ct","delta":"print("}),
            serde_json::json!({"type":"response.custom_tool_call_input.done","item_id":"ct","input":"print('hi')"}),
            serde_json::json!({"type":"response.output_item.done","item":{"type":"custom_tool_call","id":"ct","call_id":"call","name":"code","input":"print('hi')"}}),
        ]).into_iter().collect::<Result<_,_>>().unwrap();
        assert_eq!(
            projected_tools(&chunks)[&0],
            ("call".into(), "code".into(), "print('hi')".into())
        );
    }

    #[test]
    fn changed_or_missing_tool_identity_is_an_explicit_error() {
        for changed in [
            serde_json::json!({"type":"function_call","name":"read_section","arguments":"{}"}),
            tool_item("fc", "different_call", "{}"),
            serde_json::json!({"type":"function_call","id":"fc","call_id":"call","name":"different_tool","arguments":"{}"}),
        ] {
            let chunks = translate_events(vec![
                serde_json::json!({"type":"response.output_item.added","item":tool_item("fc","call","")}),
                serde_json::json!({"type":"response.output_item.done","item":changed}),
            ]);
            assert!(chunks.last().unwrap().is_err());
        }
    }

    #[test]
    fn conflicting_final_arguments_fail_instead_of_emitting_a_corrupt_call() {
        let chunks = translate_events(vec![
            serde_json::json!({"type":"response.output_item.added","item":tool_item("fc","call","")}),
            serde_json::json!({"type":"response.function_call_arguments.delta","item_id":"fc","delta":"{\"ref\":\"first"}),
            serde_json::json!({"type":"response.completed","response":{"output":[tool_item("fc","call",r#"{"ref":"second"}"#)]}}),
        ]);
        assert!(chunks
            .last()
            .unwrap()
            .as_ref()
            .err()
            .unwrap()
            .to_string()
            .contains("arguments changed"));
        assert!(chunks
            .iter()
            .filter_map(|chunk| chunk.as_ref().ok())
            .all(|chunk| chunk
                .choices
                .iter()
                .all(|choice| choice.finish_reason.is_none())));
    }

    #[test]
    fn completed_without_usage_reports_none() {
        let sse = "event: response.completed\n\
data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"output\":[]}}\n\
\n\
";
        let chunks: Vec<_> = collect_chunks(sse)
            .into_iter()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].usage.is_none());
    }
}
