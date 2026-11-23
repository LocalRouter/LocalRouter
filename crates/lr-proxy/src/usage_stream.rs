//! Token usage read from a response while it streams through, so that it
//! survives whatever the size of the event that carries it.
//!
//! The monitor keeps a bounded head and tail of a response
//! ([`crate::tap::TappedBody::with_tail`]); a terminal event larger than the
//! tail (a Responses `response.completed` repeating a long output) would lose
//! its usage there. [`UsageScanner`] reads every event as it passes, with the
//! streaming JSON lexer (which holds no content), and keeps only a skeleton
//! of the events that report usage: their `type`, `model`, `usage` objects
//! (bounded) and Ollama's token counters. The skeletons are parsed by the
//! same per-format code as the captured stream, so the numbers are read
//! exactly as they would be from the whole event.

use std::collections::VecDeque;

use lr_secret_scanner::request_stream::{Event, Lexer, Path, Segment};
use serde_json::{Map, Value};

use crate::wire::{ResponseMeta, WireFormat};

/// Largest `usage` object kept (its compact size, roughly).
const MAX_USAGE: usize = 16 * 1024;
/// Longest `type` / `model` string kept.
const MAX_LABEL: usize = 256;
/// Usage-bearing events kept from the start of the stream (Anthropic's
/// `message_start` carries the prompt tokens) ...
const FIRST: usize = 4;
/// ... and from its end (the terminal event).
const LAST: usize = 8;
/// Longest SSE field name looked at (`data`).
const MAX_FIELD: usize = 16;

/// How a response's events are delimited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsageFraming {
    /// Server-sent events (`data:` lines, blank line between events).
    #[default]
    Sse,
    /// JSON documents: one (possibly pretty-printed) body, or one per line
    /// (NDJSON).
    Json,
}

/// The usage-bearing events of a response, reduced to their usage.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageEvents {
    pub framing: UsageFraming,
    /// Skeletons in stream order: the first few and the last few.
    pub events: Vec<Value>,
}

impl UsageEvents {
    /// The usage metadata these events report, read by the format's own
    /// parser.
    pub(crate) fn meta(&self, format: WireFormat) -> ResponseMeta {
        match self.framing {
            UsageFraming::Sse => {
                let raw: String = self
                    .events
                    .iter()
                    .map(|event| format!("data: {event}\n\n"))
                    .collect();
                crate::wire::reconstruct_sse(format, &raw).0
            }
            UsageFraming::Json => match format {
                WireFormat::Ollama(_) => {
                    let raw: String = self
                        .events
                        .iter()
                        .map(|event| format!("{event}\n"))
                        .collect();
                    crate::wire::reconstruct_ndjson(format, &raw).0
                }
                _ => self
                    .events
                    .last()
                    .map(|event| crate::wire::parse_response(format, event))
                    .unwrap_or_default(),
            },
        }
    }

    /// Fill what `meta` lacks (usage and model) from these events.
    pub(crate) fn complete(&self, format: WireFormat, meta: &mut ResponseMeta) {
        if self.events.is_empty() {
            return;
        }
        let found = self.meta(format);
        let fill = |slot: &mut Option<u64>, value: Option<u64>| {
            if slot.is_none() {
                *slot = value;
            }
        };
        fill(&mut meta.input_tokens, found.input_tokens);
        fill(&mut meta.output_tokens, found.output_tokens);
        fill(&mut meta.cache_creation_tokens, found.cache_creation_tokens);
        fill(&mut meta.cache_read_tokens, found.cache_read_tokens);
        fill(&mut meta.reasoning_tokens, found.reasoning_tokens);
        if meta.model.is_none() {
            meta.model = found.model;
        }
    }
}

/// Builds one JSON value from lexer events, bounded.
#[derive(Default)]
struct Builder {
    stack: Vec<(Value, Option<String>)>,
    key: Option<String>,
    string: Option<String>,
    done: Option<Value>,
    size: usize,
    overflow: bool,
}

impl Builder {
    fn event(&mut self, event: &Event<'_>) {
        if self.overflow {
            return;
        }
        match *event {
            Event::ObjectStart => {
                self.size += 2;
                self.stack
                    .push((Value::Object(Map::new()), self.key.take()));
            }
            Event::ArrayStart => {
                self.size += 2;
                self.stack.push((Value::Array(Vec::new()), self.key.take()));
            }
            Event::ObjectEnd { .. } | Event::ArrayEnd { .. } => {
                if let Some((value, key)) = self.stack.pop() {
                    self.key = key;
                    self.value(value);
                }
            }
            Event::Key { text, .. } => {
                self.size += text.len() + 3;
                self.key = Some(text.to_owned());
            }
            Event::StringStart => self.string = Some(String::new()),
            Event::StringChunk { text } => {
                self.size += text.len();
                if let Some(string) = &mut self.string {
                    string.push_str(text);
                }
            }
            Event::StringEnd => {
                let string = self.string.take().unwrap_or_default();
                self.value(Value::String(string));
            }
            Event::Scalar { raw, truncated } => {
                self.size += raw.len();
                let value = if truncated {
                    Value::Null
                } else {
                    serde_json::from_str(raw).unwrap_or(Value::Null)
                };
                self.value(value);
            }
            // Nested beyond the lexer's depth: not a usage object.
            Event::Opaque { .. } => self.overflow = true,
        }
        if self.size > MAX_USAGE {
            self.overflow = true;
        }
    }

    fn value(&mut self, value: Value) {
        match self.stack.last_mut() {
            None => self.done = Some(value),
            Some((Value::Object(map), _)) => {
                if let Some(key) = self.key.take() {
                    map.insert(key, value);
                }
            }
            Some((Value::Array(items), _)) => items.push(value),
            Some(_) => {}
        }
    }
}

/// The skeleton of the event being read.
#[derive(Default)]
struct EventCapture {
    skeleton: Map<String, Value>,
    /// The event reports usage (a `usage` object or Ollama's counters).
    reports: bool,
    usage: Option<(Vec<String>, usize, Builder)>,
    label: Option<(Vec<String>, String)>,
}

fn keys(path: &Path) -> Option<Vec<String>> {
    path.segments()
        .map(|segment| match segment {
            Segment::Key(key) => Some(key.to_owned()),
            Segment::Index(_) => None,
        })
        .collect()
}

/// Scalars kept, by path: the event type, the model, and Ollama's counters
/// (which report usage).
fn kept_scalar(path: &[String]) -> Option<bool> {
    match path {
        [key] if key == "prompt_eval_count" || key == "eval_count" => Some(true),
        [key] if key == "type" || key == "model" || key == "done" => Some(false),
        [outer, key] if (outer == "message" || outer == "response") && key == "model" => {
            Some(false)
        }
        _ => None,
    }
}

fn insert(skeleton: &mut Map<String, Value>, path: &[String], value: Value) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut map = skeleton;
    for parent in parents {
        let entry = map
            .entry(parent.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        map = entry.as_object_mut().expect("an object");
    }
    map.insert(last.clone(), value);
}

impl EventCapture {
    fn observe(&mut self, path: &Path, event: &Event<'_>) {
        if let Some((_, depth, builder)) = &mut self.usage {
            builder.event(event);
            let ended = matches!(event, Event::ObjectEnd { .. }) && path.depth() == *depth;
            if ended {
                let (path, _, builder) = self.usage.take().expect("capturing");
                if let (false, Some(value)) = (builder.overflow, builder.done) {
                    insert(&mut self.skeleton, &path, value);
                    self.reports = true;
                }
            }
            return;
        }
        match *event {
            Event::ObjectStart
                if (1..=2).contains(&path.depth())
                    && matches!(path.last(), Some(Segment::Key("usage"))) =>
            {
                if let Some(keys) = keys(path) {
                    let mut builder = Builder::default();
                    builder.event(event);
                    self.usage = Some((keys, path.depth(), builder));
                }
            }
            Event::Scalar {
                raw,
                truncated: false,
            } => {
                if let Some(keys) = keys(path) {
                    if let Some(reports) = kept_scalar(&keys) {
                        if let Ok(value) = serde_json::from_str::<Value>(raw) {
                            if !value.is_null() {
                                self.reports |= reports;
                                insert(&mut self.skeleton, &keys, value);
                            }
                        }
                    }
                }
            }
            Event::StringStart => {
                if let Some(keys) = keys(path) {
                    if kept_scalar(&keys).is_some() {
                        self.label = Some((keys, String::new()));
                    }
                }
            }
            Event::StringChunk { text } => {
                if let Some((_, label)) = &mut self.label {
                    if label.len() + text.len() <= MAX_LABEL {
                        label.push_str(text);
                    }
                }
            }
            Event::StringEnd => {
                if let Some((keys, label)) = self.label.take() {
                    insert(&mut self.skeleton, &keys, Value::String(label));
                }
            }
            _ => {}
        }
    }
}

enum Line {
    /// At the start of a line.
    Start,
    /// Reading an SSE field name.
    Field(Vec<u8>),
    /// Right after `data:` (an optional space follows).
    DataStart,
    /// Inside a `data:` payload.
    Data,
    /// Inside a line nothing is read from.
    Skip,
}

/// Reads the usage of a response as it streams; see the module
/// documentation. Holds no content beyond the skeletons it keeps.
pub(crate) struct UsageScanner {
    framing: Option<UsageFraming>,
    line: Line,
    lexer: Option<Lexer>,
    /// The current event cannot be read (invalid JSON, `[DONE]`).
    failed: bool,
    /// `data:` lines of the current SSE event.
    data_lines: usize,
    event: EventCapture,
    first: Vec<Value>,
    last: VecDeque<Value>,
}

impl UsageScanner {
    pub(crate) fn new() -> Self {
        Self {
            framing: None,
            line: Line::Start,
            lexer: None,
            failed: false,
            data_lines: 0,
            event: EventCapture::default(),
            first: Vec::new(),
            last: VecDeque::new(),
        }
    }

    /// Read the next bytes of the response.
    pub(crate) fn push(&mut self, mut data: &[u8]) {
        if self.framing.is_none() {
            let Some(start) = data.iter().position(|b| !b.is_ascii_whitespace()) else {
                return;
            };
            data = &data[start..];
            self.framing = Some(if matches!(data[0], b'{' | b'[') {
                UsageFraming::Json
            } else {
                UsageFraming::Sse
            });
        }
        match self.framing {
            Some(UsageFraming::Sse) => self.push_sse(data),
            Some(UsageFraming::Json) => self.push_json(data),
            None => {}
        }
    }

    /// The response ended (or was cut off): what was read.
    pub(crate) fn finish(mut self) -> Option<UsageEvents> {
        match self.framing? {
            // An event without its closing blank line still counts.
            UsageFraming::Sse => self.end_event(),
            UsageFraming::Json => {
                if self.lexer.is_some() {
                    self.end_event();
                }
            }
        }
        let framing = self.framing?;
        let mut events = self.first;
        events.extend(self.last);
        (!events.is_empty()).then_some(UsageEvents { framing, events })
    }

    fn feed(&mut self, bytes: &[u8]) {
        if self.failed || bytes.is_empty() {
            return;
        }
        let Self { lexer, event, .. } = self;
        let lexer = lexer.get_or_insert_with(Lexer::new);
        if lexer
            .push(bytes, |path, _, lexed| event.observe(path, &lexed))
            .is_err()
        {
            self.failed = true;
        }
    }

    /// The current event ended: keep its skeleton if it reports usage.
    fn end_event(&mut self) {
        let event = std::mem::take(&mut self.event);
        let lexer = self.lexer.take();
        let failed = std::mem::take(&mut self.failed);
        self.data_lines = 0;
        let Some(mut lexer) = lexer else {
            return;
        };
        let mut event = event;
        if failed
            || lexer
                .finish(|path, _, lexed| event.observe(path, &lexed))
                .is_err()
            || !event.reports
        {
            return;
        }
        let skeleton = Value::Object(event.skeleton);
        if self.first.len() < FIRST {
            self.first.push(skeleton);
        } else {
            if self.last.len() == LAST {
                self.last.pop_front();
            }
            self.last.push_back(skeleton);
        }
    }

    fn push_sse(&mut self, data: &[u8]) {
        let mut i = 0;
        while i < data.len() {
            match &mut self.line {
                Line::Start => match data[i] {
                    // A blank line ends the event.
                    b'\n' => {
                        self.end_event();
                        i += 1;
                    }
                    b'\r' => i += 1,
                    _ => self.line = Line::Field(Vec::new()),
                },
                Line::Field(name) => {
                    let byte = data[i];
                    i += 1;
                    match byte {
                        b':' => {
                            self.line = if name == b"data" {
                                Line::DataStart
                            } else {
                                Line::Skip
                            };
                        }
                        b'\n' => self.line = Line::Start,
                        _ if name.len() < MAX_FIELD => name.push(byte),
                        _ => self.line = Line::Skip,
                    }
                }
                Line::DataStart => {
                    if data[i] == b' ' {
                        i += 1;
                    }
                    // Data lines of one event join with a line break.
                    if self.data_lines > 0 {
                        self.feed(b"\n");
                    }
                    self.data_lines += 1;
                    self.line = Line::Data;
                }
                Line::Data => {
                    let end = data[i..].iter().position(|b| *b == b'\n');
                    let to = end.map_or(data.len(), |at| i + at);
                    self.feed(&data[i..to]);
                    if end.is_some() {
                        self.line = Line::Start;
                        i = to + 1;
                    } else {
                        i = to;
                    }
                }
                Line::Skip => match data[i..].iter().position(|b| *b == b'\n') {
                    Some(at) => {
                        self.line = Line::Start;
                        i += at + 1;
                    }
                    None => i = data.len(),
                },
            }
        }
    }

    fn push_json(&mut self, data: &[u8]) {
        let mut i = 0;
        while i < data.len() {
            let end = data[i..].iter().position(|b| *b == b'\n');
            let to = end.map_or(data.len(), |at| i + at + 1);
            let piece = &data[i..to];
            if self.lexer.is_some() || piece.iter().any(|b| !b.is_ascii_whitespace()) {
                self.feed(piece);
            }
            if end.is_some() {
                // A document ends at a line end (NDJSON, or the end of a
                // pretty-printed body); an unreadable one is dropped there.
                let complete = self.lexer.as_ref().is_some_and(Lexer::is_complete);
                if complete {
                    self.end_event();
                } else if self.failed {
                    self.event = EventCapture::default();
                    self.lexer = None;
                    self.failed = false;
                }
            }
            i = to;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scan(body: &[u8], chunk: usize) -> Option<UsageEvents> {
        let mut scanner = UsageScanner::new();
        for piece in body.chunks(chunk.max(1)) {
            scanner.push(piece);
        }
        scanner.finish()
    }

    /// A Responses stream whose `response.completed` (with the usage) is far
    /// larger than any capture: usage read at every chunking, nothing of the
    /// output kept.
    #[test]
    fn usage_survives_a_terminal_event_of_any_size() {
        let output = "x".repeat(12 * 1024 * 1024);
        let completed = json!({"type":"response.completed","response":{
            "id":"r","model":"gpt-test","output":[{"type":"message","content":[{"type":"output_text","text":output}]}],
            "usage":{"input_tokens":11,"output_tokens":22,"input_tokens_details":{"cached_tokens":3},
                     "output_tokens_details":{"reasoning_tokens":5}}}});
        let body = format!(
            "event: response.created\ndata: {}\n\nevent: response.completed\ndata: {completed}\n\ndata: [DONE]\n\n",
            json!({"type":"response.created","response":{"id":"r","usage":null}})
        );
        for chunk in [7, 4096, 1 << 20] {
            let events = scan(body.as_bytes(), chunk).expect("usage");
            assert_eq!(events.framing, UsageFraming::Sse);
            assert_eq!(events.events.len(), 1, "only the usage-bearing event");
            let kept = events.events[0].to_string();
            assert!(kept.len() < 1024, "{kept}");
            let meta = events.meta(WireFormat::OpenAiResponses);
            assert_eq!(meta.input_tokens, Some(11), "chunk {chunk}");
            assert_eq!(meta.output_tokens, Some(22));
            assert_eq!(meta.reasoning_tokens, Some(5));
        }
    }

    #[test]
    fn anthropic_keeps_the_prompt_from_the_start_and_the_output_from_the_end() {
        let mut body = String::from(
            "event: message_start\r\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"model\":\"claude-x\",\"usage\":{\"input_tokens\":7,\"cache_read_input_tokens\":2}}}\r\n\r\n",
        );
        for _ in 0..50 {
            body.push_str("event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n");
        }
        body.push_str("event: message_delta\ndata: {\"type\":\"message_delta\",\n");
        body.push_str("data: \"usage\":{\"output_tokens\":9}}\n\n");
        for chunk in [1, 3, 64] {
            let events = scan(body.as_bytes(), chunk).expect("usage");
            assert_eq!(events.events.len(), 2);
            let meta = events.meta(WireFormat::AnthropicMessages);
            assert_eq!(meta.input_tokens, Some(7), "chunk {chunk}");
            assert_eq!(meta.cache_read_tokens, Some(2));
            assert_eq!(meta.output_tokens, Some(9));
            assert_eq!(meta.model.as_deref(), Some("claude-x"));
        }
    }

    #[test]
    fn chat_ndjson_and_json_bodies() {
        // OpenAI Chat: `usage: null` on every chunk but the last.
        let mut chat = String::new();
        for _ in 0..20 {
            chat.push_str(
                "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}],\"usage\":null}\n\n",
            );
        }
        chat.push_str("data: {\"choices\":[],\"model\":\"gpt\",\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":6}}\n\ndata: [DONE]\n\n");
        let meta = scan(chat.as_bytes(), 5)
            .unwrap()
            .meta(WireFormat::OpenAiChat);
        assert_eq!((meta.input_tokens, meta.output_tokens), (Some(4), Some(6)));

        // Ollama NDJSON: counters on the last line.
        let ollama = "{\"model\":\"llama\",\"message\":{\"content\":\"a\"},\"done\":false}\n{\"model\":\"llama\",\"message\":{\"content\":\"\"},\"done\":true,\"prompt_eval_count\":26,\"eval_count\":8}\n";
        let format = WireFormat::Ollama(crate::ollama::OllamaEndpoint::Chat);
        let events = scan(ollama.as_bytes(), 3).unwrap();
        assert_eq!(events.framing, UsageFraming::Json);
        let meta = events.meta(format);
        assert_eq!((meta.input_tokens, meta.output_tokens), (Some(26), Some(8)));

        // A pretty-printed JSON body.
        let body = serde_json::to_string_pretty(&json!({
            "id":"x","model":"gpt","choices":[{"message":{"content":"y".repeat(10_000)}}],
            "usage":{"prompt_tokens":1,"completion_tokens":2}
        }))
        .unwrap();
        let meta = scan(body.as_bytes(), 100)
            .unwrap()
            .meta(WireFormat::OpenAiChat);
        assert_eq!((meta.input_tokens, meta.output_tokens), (Some(1), Some(2)));
    }

    #[test]
    fn bounded_whatever_the_stream() {
        // Many usage-bearing events: only the first and last few are kept.
        let mut body = String::new();
        for i in 0..1000 {
            body.push_str(&format!(
                "data: {{\"usage\":{{\"output_tokens\":{i}}}}}\n\n"
            ));
        }
        let events = scan(body.as_bytes(), 977).unwrap();
        assert_eq!(events.events.len(), FIRST + LAST);
        assert_eq!(events.events.last().unwrap()["usage"]["output_tokens"], 999);
        // An oversized `usage` object is not kept; garbage is ignored.
        let huge = format!(
            "data: {{\"usage\":{{\"note\":\"{}\"}}}}\n\ndata: not json\n\n: comment\n\n",
            "z".repeat(MAX_USAGE * 2)
        );
        assert_eq!(scan(huge.as_bytes(), 1000), None);
        assert_eq!(scan(b"", 1), None);
        // A body cut off inside an event keeps what completed before it.
        let cut = "data: {\"usage\":{\"output_tokens\":3}}\n\ndata: {\"usage\":{\"output_tok";
        let events = scan(cut.as_bytes(), 4).unwrap();
        assert_eq!(events.events.len(), 1);
    }

    #[test]
    fn missing_fields_are_completed_and_present_ones_kept() {
        let events = UsageEvents {
            framing: UsageFraming::Sse,
            events: vec![json!({"type":"message_delta","usage":{"output_tokens":9}})],
        };
        let mut meta = ResponseMeta {
            input_tokens: Some(7),
            ..Default::default()
        };
        events.complete(WireFormat::AnthropicMessages, &mut meta);
        assert_eq!((meta.input_tokens, meta.output_tokens), (Some(7), Some(9)));
    }
}
