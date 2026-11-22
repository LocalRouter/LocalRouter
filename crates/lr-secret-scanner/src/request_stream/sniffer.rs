//! Order-independent capture of the top-level request fields the proxy needs.

use super::lexer::{Event, LexError, Lexer, Path, Segment};

/// Default bound, in bytes, on each captured field.
pub const DEFAULT_FIELD_LIMIT: usize = 64 * 1024;

/// What is known about one captured field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Sniffed<T> {
    /// The key has not appeared (so far).
    #[default]
    Absent,
    /// The key appeared and its value is still being read.
    Pending,
    /// The complete value.
    Value(T),
    /// The key is present with a value of another JSON type, which batch
    /// parsing reads as "not set".
    Mismatch,
    /// The value exceeded the capture bound (or sits beyond the lexer's depth
    /// or key limits) and was dropped.
    Overflow,
}

impl<T> Sniffed<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            _ => None,
        }
    }

    pub fn is_overflow(&self) -> bool {
        matches!(self, Self::Overflow)
    }

    /// The key appeared and its value has been read completely, whatever the
    /// outcome.
    pub fn is_settled(&self) -> bool {
        !matches!(self, Self::Absent | Self::Pending)
    }
}

/// JSON type of a top-level value that is counted rather than captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Array {
        len: u64,
    },
    String,
    Object,
    /// A number, boolean or `null`.
    Scalar,
    /// Nested beyond the lexer's depth limit; its length is unknown.
    Opaque,
}

impl Shape {
    fn array_len(shape: Option<Shape>) -> u64 {
        match shape {
            Some(Shape::Array { len }) => len,
            _ => 0,
        }
    }
}

/// Fields collected by [`MetadataSniffer`]. A repeated key replaces the
/// earlier value, as it does when the body is parsed into a `serde_json::Value`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestMetadata {
    pub model: Sniffed<String>,
    pub stream: Sniffed<bool>,
    /// Top-level `type` (WebSocket messages).
    pub kind: Sniffed<String>,
    pub previous_response_id: Sniffed<String>,
    /// Top-level `client_metadata`, as compact JSON text.
    pub client_metadata: Sniffed<String>,
    /// `metadata.user_id`.
    pub metadata_user_id: Sniffed<String>,
    /// `metadata.session_id`.
    pub metadata_session_id: Sniffed<String>,
    /// Top-level `extensions`, as compact JSON text.
    pub extensions: Sniffed<String>,
    /// Shape of top-level `messages`, once its value is complete.
    pub messages: Option<Shape>,
    /// Shape of top-level `input`, once its value is complete.
    pub input: Option<Shape>,
    /// Shape of top-level `tools`, once its value is complete.
    pub tools: Option<Shape>,
    /// One of the keys above appeared more than once at its level. The fields
    /// hold the last occurrence; a decision already taken on an earlier one
    /// (for example a model gate) saw a different value.
    pub duplicate_keys: bool,
    /// The root value is complete: absent fields will stay absent.
    pub complete: bool,
}

impl RequestMetadata {
    /// Elements of `messages` when it is an array, else 0.
    pub fn messages_len(&self) -> u64 {
        Shape::array_len(self.messages)
    }

    /// Items of a Responses `input`: array length, 1 for a string, else 0.
    pub fn input_len(&self) -> u64 {
        match self.input {
            Some(Shape::String) => 1,
            shape => Shape::array_len(shape),
        }
    }

    /// Elements of `tools` when it is an array, else 0.
    pub fn tools_len(&self) -> u64 {
        Shape::array_len(self.tools)
    }

    pub fn has_tools(&self) -> bool {
        self.tools_len() > 0
    }

    /// `stream`, defaulting to `false` like the batch parsers.
    pub fn is_stream(&self) -> bool {
        self.stream.value().copied().unwrap_or(false)
    }

    /// Captured `client_metadata` as a JSON value.
    pub fn client_metadata_json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(self.client_metadata.value()?).ok()
    }

    /// Captured `extensions` as a JSON value.
    pub fn extensions_json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(self.extensions.value()?).ok()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Top {
    Other,
    Model,
    Stream,
    Kind,
    PreviousResponseId,
    ClientMetadata,
    Metadata,
    Extensions,
    Messages,
    Input,
    Tools,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Member {
    Other,
    UserId,
    SessionId,
}

/// Collects [`RequestMetadata`] from lexer events. It never needs the body:
/// pass every event of the head lexer to [`observe`](Self::observe) and read
/// [`metadata`](Self::metadata) at any time.
#[derive(Clone, Debug)]
pub struct MetadataSniffer {
    meta: RequestMetadata,
    limit: usize,
    top: Top,
    member: Member,
    metadata_seen: bool,
    /// The string or subtree being captured.
    text: String,
}

impl Default for MetadataSniffer {
    fn default() -> Self {
        Self::new()
    }
}

impl MetadataSniffer {
    pub fn new() -> Self {
        Self::with_limit(DEFAULT_FIELD_LIMIT)
    }

    /// `limit` bounds each captured field, in bytes of decoded text (strings)
    /// or compact JSON (subtrees).
    pub fn with_limit(limit: usize) -> Self {
        Self {
            meta: RequestMetadata::default(),
            limit,
            top: Top::Other,
            member: Member::Other,
            metadata_seen: false,
            text: String::new(),
        }
    }

    pub fn metadata(&self) -> &RequestMetadata {
        &self.meta
    }

    pub fn into_metadata(self) -> RequestMetadata {
        self.meta
    }

    pub fn observe(&mut self, path: &Path, event: &Event<'_>) {
        let depth = path.depth();
        if depth == 0 {
            if matches!(
                event,
                Event::ObjectEnd { .. }
                    | Event::ArrayEnd { .. }
                    | Event::StringEnd
                    | Event::Scalar { .. }
                    | Event::Opaque { last: true }
            ) {
                self.meta.complete = true;
            }
            return;
        }
        // Only members of a root object are fields; a root array has none.
        if !matches!(path.segment(0), Some(Segment::Key(_))) {
            return;
        }
        if depth == 1 {
            if let Event::Key { text, truncated } = *event {
                self.begin_top(text, truncated);
                return;
            }
        }
        let limit = self.limit;
        let meta = &mut self.meta;
        match self.top {
            Top::Other => {}
            Top::Model if depth == 1 => string(&mut meta.model, &mut self.text, limit, event),
            Top::Kind if depth == 1 => string(&mut meta.kind, &mut self.text, limit, event),
            Top::PreviousResponseId if depth == 1 => {
                string(&mut meta.previous_response_id, &mut self.text, limit, event)
            }
            Top::Stream if depth == 1 => match *event {
                Event::Scalar { raw: "true", .. } => meta.stream = Sniffed::Value(true),
                Event::Scalar { raw: "false", .. } => meta.stream = Sniffed::Value(false),
                Event::Scalar { .. }
                | Event::StringStart
                | Event::ObjectStart
                | Event::ArrayStart
                | Event::Opaque { .. } => meta.stream = Sniffed::Mismatch,
                _ => {}
            },
            Top::Messages if depth == 1 => shape(&mut meta.messages, event),
            Top::Input if depth == 1 => shape(&mut meta.input, event),
            Top::Tools if depth == 1 => shape(&mut meta.tools, event),
            Top::ClientMetadata => subtree(
                &mut meta.client_metadata,
                &mut self.text,
                limit,
                depth,
                event,
            ),
            Top::Extensions => subtree(&mut meta.extensions, &mut self.text, limit, depth, event),
            Top::Metadata if depth == 2 => {
                if let Event::Key { text, truncated } = *event {
                    self.member = match text {
                        _ if truncated => Member::Other,
                        "user_id" => Member::UserId,
                        "session_id" => Member::SessionId,
                        _ => Member::Other,
                    };
                    let slot = match self.member {
                        Member::UserId => &mut meta.metadata_user_id,
                        Member::SessionId => &mut meta.metadata_session_id,
                        Member::Other => return,
                    };
                    if !matches!(slot, Sniffed::Absent) {
                        meta.duplicate_keys = true;
                    }
                    *slot = Sniffed::Pending;
                    self.text.clear();
                    return;
                }
                match self.member {
                    Member::UserId => {
                        string(&mut meta.metadata_user_id, &mut self.text, limit, event)
                    }
                    Member::SessionId => {
                        string(&mut meta.metadata_session_id, &mut self.text, limit, event)
                    }
                    Member::Other => {}
                }
            }
            _ => {}
        }
    }

    fn begin_top(&mut self, key: &str, truncated: bool) {
        self.top = match key {
            _ if truncated => Top::Other,
            "model" => Top::Model,
            "stream" => Top::Stream,
            "type" => Top::Kind,
            "previous_response_id" => Top::PreviousResponseId,
            "client_metadata" => Top::ClientMetadata,
            "metadata" => Top::Metadata,
            "extensions" => Top::Extensions,
            "messages" => Top::Messages,
            "input" => Top::Input,
            "tools" => Top::Tools,
            _ => Top::Other,
        };
        self.member = Member::Other;
        self.text.clear();
        let meta = &mut self.meta;
        fn restart<T>(slot: &mut Sniffed<T>) -> bool {
            !matches!(std::mem::replace(slot, Sniffed::Pending), Sniffed::Absent)
        }
        let repeated = match self.top {
            Top::Other => false,
            Top::Model => restart(&mut meta.model),
            Top::Stream => restart(&mut meta.stream),
            Top::Kind => restart(&mut meta.kind),
            Top::PreviousResponseId => restart(&mut meta.previous_response_id),
            Top::ClientMetadata => restart(&mut meta.client_metadata),
            Top::Extensions => restart(&mut meta.extensions),
            Top::Metadata => {
                meta.metadata_user_id = Sniffed::Absent;
                meta.metadata_session_id = Sniffed::Absent;
                std::mem::replace(&mut self.metadata_seen, true)
            }
            // A pending count is not recorded, so a repeat is noticed only
            // once the first value is complete; values never interleave.
            Top::Messages => meta.messages.take().is_some(),
            Top::Input => meta.input.take().is_some(),
            Top::Tools => meta.tools.take().is_some(),
        };
        meta.duplicate_keys |= repeated;
    }
}

/// Capture a string value whose events arrive at the field's own depth.
fn string(slot: &mut Sniffed<String>, text: &mut String, limit: usize, event: &Event<'_>) {
    if !matches!(slot, Sniffed::Pending) {
        return;
    }
    match *event {
        Event::StringStart => text.clear(),
        Event::StringChunk { text: chunk } => {
            if text.len() + chunk.len() > limit {
                text.clear();
                *slot = Sniffed::Overflow;
            } else {
                text.push_str(chunk);
            }
        }
        Event::StringEnd => *slot = Sniffed::Value(std::mem::take(text)),
        Event::ObjectStart | Event::ArrayStart | Event::Scalar { .. } | Event::Opaque { .. } => {
            *slot = Sniffed::Mismatch
        }
        _ => {}
    }
}

fn shape(slot: &mut Option<Shape>, event: &Event<'_>) {
    *slot = Some(match *event {
        Event::ArrayEnd { len } => Shape::Array { len },
        Event::StringStart => Shape::String,
        Event::ObjectStart => Shape::Object,
        Event::Scalar { .. } => Shape::Scalar,
        Event::Opaque { .. } => Shape::Opaque,
        _ => return,
    });
}

/// Re-serialize a whole value as compact JSON. Strings are escaped the way
/// `serde_json` writes them; scalars keep their source spelling.
fn subtree(
    slot: &mut Sniffed<String>,
    text: &mut String,
    limit: usize,
    depth: usize,
    event: &Event<'_>,
) {
    if !matches!(slot, Sniffed::Pending) {
        return;
    }
    // A value or key follows a comma unless it opens its container or
    // follows its key. Text inside a string never reaches this check.
    fn separate(text: &mut String) {
        if !matches!(text.as_bytes().last(), None | Some(b'{' | b'[' | b':')) {
            text.push(',');
        }
    }
    let mut overflow = false;
    let mut complete = false;
    match *event {
        Event::ObjectStart => {
            separate(text);
            text.push('{');
        }
        Event::ArrayStart => {
            separate(text);
            text.push('[');
        }
        Event::ObjectEnd { .. } => {
            text.push('}');
            complete = true;
        }
        Event::ArrayEnd { .. } => {
            text.push(']');
            complete = true;
        }
        Event::Key {
            text: key,
            truncated,
        } => {
            overflow = truncated;
            separate(text);
            text.push('"');
            escape(key, text);
            text.push_str("\":");
        }
        Event::StringStart => {
            separate(text);
            text.push('"');
        }
        Event::StringChunk { text: chunk } => {
            // Escaping at most sextuples the chunk; refuse before growing.
            overflow = text.len() + chunk.len() > limit;
            if !overflow {
                escape(chunk, text);
            }
        }
        Event::StringEnd => {
            text.push('"');
            complete = true;
        }
        Event::Scalar { raw, truncated } => {
            overflow = truncated;
            separate(text);
            text.push_str(raw);
            complete = true;
        }
        Event::Opaque { .. } => overflow = true,
    }
    if overflow || text.len() > limit {
        text.clear();
        *slot = Sniffed::Overflow;
    } else if complete && depth == 1 {
        *slot = Sniffed::Value(std::mem::take(text));
    }
}

fn escape(text: &str, out: &mut String) {
    use std::fmt::Write;
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
}

/// Sniff a complete body held in memory.
pub fn sniff(body: &[u8]) -> Result<RequestMetadata, LexError> {
    let mut lexer = Lexer::new();
    let mut sniffer = MetadataSniffer::new();
    lexer.push(body, |path, _, event| sniffer.observe(path, &event))?;
    lexer.finish(|path, _, event| sniffer.observe(path, &event))?;
    Ok(sniffer.into_metadata())
}
