//! Byte-level incremental JSON lexer with absolute offsets.

use std::fmt;

/// Half-open range of absolute byte offsets into the lexed stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u64,
    pub end: u64,
}

impl Span {
    fn at(start: u64, len: usize) -> Self {
        Self {
            start,
            end: start + len as u64,
        }
    }

    pub fn len(&self) -> u64 {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

/// One lexical event. Every event comes with the [`Span`] of the raw bytes it
/// covers and the [`Path`] of the value it belongs to.
///
/// Borrowed text is valid only for the duration of the sink call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event<'a> {
    /// `{`. The path is the object's own path.
    ObjectStart,
    /// `}`. The path is the object's own path; `len` counts its members.
    ObjectEnd { len: u64 },
    /// `[`. The path is the array's own path.
    ArrayStart,
    /// `]`. The path is the array's own path; `len` counts its elements.
    ArrayEnd { len: u64 },
    /// A complete object key, decoded. The span covers the key with both
    /// quotes; the path already ends in this key. Keys longer than
    /// [`LexerLimits::max_key_bytes`] report their first bytes (cut at a
    /// character boundary) with `truncated` set, and the path holds the same
    /// shortened text.
    Key { text: &'a str, truncated: bool },
    /// Opening quote of a string value.
    StringStart,
    /// A piece of a string value: decoded `text` and the span of its raw
    /// spelling. One chunk is either a run of verbatim characters
    /// (`text.len() == span.len()`, the raw bytes equal `text`) or exactly one
    /// escape sequence, a surrogate pair counting as one (`text.len() <
    /// span.len()`). Chunks of one string are contiguous.
    StringChunk { text: &'a str },
    /// Closing quote of a string value.
    StringEnd,
    /// A number, `true`, `false` or `null`, spelled as in the source. Longer
    /// than [`LexerLimits::max_scalar_bytes`]: `raw` holds the first bytes and
    /// `truncated` is set; the span still covers the whole token.
    Scalar { raw: &'a str, truncated: bool },
    /// Raw bytes of a container nested deeper than
    /// [`LexerLimits::max_depth`]. Its content is not lexed: no keys, strings
    /// or paths are reported for it, only contiguous spans, the first starting
    /// at the opening bracket and the one with `last` set ending at the
    /// matching closing bracket. The path is the container's own path.
    Opaque { last: bool },
}

/// Why lexing stopped. Offsets are absolute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LexError {
    /// A byte sequence that is not UTF-8 starts at `offset` (or the input
    /// ended inside a multi-byte character starting there).
    InvalidUtf8 { offset: u64 },
    /// The byte at `offset` cannot continue a JSON document.
    Syntax { offset: u64 },
    /// The input ended at `offset` before the document was complete.
    Truncated { offset: u64 },
}

impl LexError {
    pub fn offset(&self) -> u64 {
        match *self {
            Self::InvalidUtf8 { offset } | Self::Syntax { offset } | Self::Truncated { offset } => {
                offset
            }
        }
    }
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUtf8 { offset } => write!(f, "invalid UTF-8 at byte {offset}"),
            Self::Syntax { offset } => write!(f, "invalid JSON at byte {offset}"),
            Self::Truncated { offset } => write!(f, "JSON document ends early at byte {offset}"),
        }
    }
}

impl std::error::Error for LexError {}

/// Bounds on what the lexer keeps in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LexerLimits {
    /// Containers tracked on the path. A container that would be nested deeper
    /// is reported as [`Event::Opaque`]. The default, 127, is the deepest
    /// nesting `serde_json` parses.
    pub max_depth: usize,
    /// Decoded key bytes reported and kept on the path.
    pub max_key_bytes: usize,
    /// Scalar bytes reported.
    pub max_scalar_bytes: usize,
}

impl Default for LexerLimits {
    fn default() -> Self {
        Self {
            max_depth: 127,
            max_key_bytes: 4 * 1024,
            max_scalar_bytes: 4 * 1024,
        }
    }
}

/// One step of a [`Path`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment<'a> {
    Key(&'a str),
    Index(u64),
}

/// One step of a pattern for [`Path::matches`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step<'a> {
    Key(&'a str),
    Index(u64),
    /// Any array element.
    AnyIndex,
    /// Any object member.
    AnyKey,
}

impl Step<'_> {
    fn accepts(&self, segment: Segment<'_>) -> bool {
        match (*self, segment) {
            (Step::Key(want), Segment::Key(key)) => want == key,
            (Step::Index(want), Segment::Index(index)) => want == index,
            (Step::AnyIndex, Segment::Index(_)) | (Step::AnyKey, Segment::Key(_)) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
struct Frame {
    object: bool,
    /// Members (object) or elements (array) started so far.
    count: u64,
    /// Where this object's current key starts in `Path::keys`.
    key_start: usize,
}

/// Location of the value an event belongs to: one segment per enclosing
/// container, outermost first. The root value has the empty path.
#[derive(Clone, Debug, Default)]
pub struct Path {
    frames: Vec<Frame>,
    keys: String,
}

impl Path {
    /// Number of segments, which is the number of containers enclosing the
    /// value.
    pub fn depth(&self) -> usize {
        self.frames.len()
    }

    pub fn is_root(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn segment(&self, index: usize) -> Option<Segment<'_>> {
        let frame = self.frames.get(index)?;
        Some(if frame.object {
            let end = self
                .frames
                .get(index + 1)
                .map_or(self.keys.len(), |next| next.key_start);
            Segment::Key(&self.keys[frame.key_start..end])
        } else {
            Segment::Index(frame.count.saturating_sub(1))
        })
    }

    pub fn last(&self) -> Option<Segment<'_>> {
        self.segment(self.frames.len().checked_sub(1)?)
    }

    pub fn segments(&self) -> impl Iterator<Item = Segment<'_>> {
        (0..self.frames.len()).filter_map(|index| self.segment(index))
    }

    /// Whether the path has exactly these steps.
    pub fn matches(&self, pattern: &[Step<'_>]) -> bool {
        pattern.len() == self.frames.len() && self.starts_with(pattern)
    }

    /// Whether the path begins with these steps.
    pub fn starts_with(&self, pattern: &[Step<'_>]) -> bool {
        pattern.len() <= self.frames.len()
            && pattern
                .iter()
                .enumerate()
                .all(|(index, step)| self.segment(index).is_some_and(|s| step.accepts(s)))
    }

    /// RFC 6901 JSON pointer (`/input/12/content/3/text`; empty for the root).
    pub fn pointer(&self) -> String {
        self.to_string()
    }

    fn push(&mut self, object: bool) {
        self.frames.push(Frame {
            object,
            count: 0,
            key_start: self.keys.len(),
        });
    }

    fn pop(&mut self) -> u64 {
        let frame = self.frames.pop().expect("open container");
        self.keys.truncate(frame.key_start);
        frame.count
    }

    fn set_key(&mut self, key: &str) {
        let frame = self.frames.last_mut().expect("open object");
        self.keys.truncate(frame.key_start);
        self.keys.push_str(key);
        frame.count += 1;
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for segment in self.segments() {
            f.write_str("/")?;
            match segment {
                Segment::Index(index) => write!(f, "{index}")?,
                Segment::Key(key) => {
                    for ch in key.chars() {
                        match ch {
                            '~' => f.write_str("~0")?,
                            '/' => f.write_str("~1")?,
                            ch => fmt::Write::write_char(f, ch)?,
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    Value,
    ValueOrEnd,
    KeyOrEnd,
    Key,
    Colon,
    CommaOrEnd,
    /// The root value is complete; only whitespace may follow.
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Between,
    Str,
    Esc,
    Scalar,
    Opaque,
}

/// Grammar position inside a number or literal.
#[derive(Clone, Copy, Debug)]
enum Token {
    Literal(&'static [u8]),
    Minus,
    Zero,
    Int,
    Dot,
    Frac,
    ExpStart,
    ExpSign,
    Exp,
}

impl Token {
    fn start(byte: u8) -> Option<Self> {
        Some(match byte {
            b't' => Self::Literal(b"rue"),
            b'f' => Self::Literal(b"alse"),
            b'n' => Self::Literal(b"ull"),
            b'-' => Self::Minus,
            b'0' => Self::Zero,
            b'1'..=b'9' => Self::Int,
            _ => return None,
        })
    }

    fn step(self, byte: u8) -> Option<Self> {
        let digit = byte.is_ascii_digit();
        Some(match (self, byte) {
            (Self::Literal(rest), _) if rest.first() == Some(&byte) => Self::Literal(&rest[1..]),
            (Self::Minus, b'0') => Self::Zero,
            (Self::Minus, b'1'..=b'9') => Self::Int,
            (Self::Int, _) if digit => Self::Int,
            (Self::Zero | Self::Int, b'.') => Self::Dot,
            (Self::Dot | Self::Frac, _) if digit => Self::Frac,
            (Self::Zero | Self::Int | Self::Frac, b'e' | b'E') => Self::ExpStart,
            (Self::ExpStart, b'+' | b'-') => Self::ExpSign,
            (Self::ExpStart | Self::ExpSign | Self::Exp, _) if digit => Self::Exp,
            _ => return None,
        })
    }

    fn complete(self) -> bool {
        match self {
            Self::Literal(rest) => rest.is_empty(),
            Self::Zero | Self::Int | Self::Frac | Self::Exp => true,
            Self::Minus | Self::Dot | Self::ExpStart | Self::ExpSign => false,
        }
    }
}

type Sink<'s> = dyn FnMut(&Path, Span, Event<'_>) + 's;

/// Incremental JSON lexer. Feed it the document in any chunking with
/// [`push`](Self::push), then call [`finish`](Self::finish).
///
/// It accepts exactly the documents `serde_json` parses into a `Value`, apart
/// from two deliberate differences: nesting deeper than
/// [`LexerLimits::max_depth`] is reported as [`Event::Opaque`] instead of an
/// error, and number magnitudes are not checked (`1e999` lexes).
///
/// After an error the lexer is spent: every later call returns the same error.
/// Events delivered before the error describe a valid prefix of the input.
#[derive(Clone, Debug)]
pub struct Lexer {
    limits: LexerLimits,
    path: Path,
    expect: Expect,
    mode: Mode,
    /// Absolute offset of the next byte to lex. Bytes of an incomplete UTF-8
    /// character wait in `carry` and are not counted yet.
    offset: u64,
    carry: [u8; 4],
    carry_len: usize,
    /// Start of the key or scalar being read.
    token_start: u64,
    in_key: bool,
    key: String,
    key_truncated: bool,
    scalar: String,
    scalar_truncated: bool,
    token: Token,
    esc: [u8; 12],
    esc_len: usize,
    esc_start: u64,
    opaque_depth: u64,
    opaque_string: bool,
    opaque_escape: bool,
    /// Start of the opaque bytes not yet reported.
    opaque_start: u64,
    failed: Option<LexError>,
}

impl Default for Lexer {
    fn default() -> Self {
        Self::new()
    }
}

impl Lexer {
    pub fn new() -> Self {
        Self::with_limits(LexerLimits::default())
    }

    pub fn with_limits(limits: LexerLimits) -> Self {
        Self {
            limits,
            path: Path::default(),
            expect: Expect::Value,
            mode: Mode::Between,
            offset: 0,
            carry: [0; 4],
            carry_len: 0,
            token_start: 0,
            in_key: false,
            key: String::new(),
            key_truncated: false,
            scalar: String::new(),
            scalar_truncated: false,
            token: Token::Zero,
            esc: [0; 12],
            esc_len: 0,
            esc_start: 0,
            opaque_depth: 0,
            opaque_string: false,
            opaque_escape: false,
            opaque_start: 0,
            failed: None,
        }
    }

    /// Total bytes accepted so far, including a carried partial character.
    pub fn bytes_fed(&self) -> u64 {
        self.offset + self.carry_len as u64
    }

    /// Every byte before this offset is settled: it is covered by an event
    /// already delivered, or is whitespace or punctuation between events. A
    /// key, scalar, escape sequence or multi-byte character still being read
    /// starts here. A consumer holding the raw bytes may release or copy
    /// everything before it.
    pub fn settled(&self) -> u64 {
        match self.mode {
            Mode::Scalar => self.token_start,
            Mode::Str | Mode::Esc if self.in_key => self.token_start,
            Mode::Esc => self.esc_start,
            Mode::Opaque => self.opaque_start,
            Mode::Str | Mode::Between => self.offset,
        }
    }

    /// Path of the value being read (or of the last one, between values).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The root value is complete; only whitespace may follow.
    pub fn is_complete(&self) -> bool {
        self.failed.is_none() && self.expect == Expect::End && self.carry_len == 0
    }

    pub fn error(&self) -> Option<LexError> {
        self.failed
    }

    /// Lex the next chunk, calling `sink` once per event in stream order.
    pub fn push(
        &mut self,
        chunk: &[u8],
        mut sink: impl FnMut(&Path, Span, Event<'_>),
    ) -> Result<(), LexError> {
        if let Some(error) = self.failed {
            return Err(error);
        }
        let result = self.push_bytes(chunk, &mut sink);
        if let Err(error) = result {
            self.failed = Some(error);
        }
        result
    }

    /// Declare the end of input. Delivers a trailing root scalar, then checks
    /// that the document is complete.
    pub fn finish(&mut self, mut sink: impl FnMut(&Path, Span, Event<'_>)) -> Result<(), LexError> {
        if let Some(error) = self.failed {
            return Err(error);
        }
        let result = self.finish_inner(&mut sink);
        if let Err(error) = result {
            self.failed = Some(error);
        }
        result
    }

    fn finish_inner(&mut self, sink: &mut Sink<'_>) -> Result<(), LexError> {
        if self.carry_len > 0 {
            return Err(LexError::InvalidUtf8 {
                offset: self.offset,
            });
        }
        if self.mode == Mode::Scalar && self.token.complete() {
            self.end_scalar(sink)?;
        }
        if self.mode != Mode::Between || self.expect != Expect::End {
            return Err(LexError::Truncated {
                offset: self.offset,
            });
        }
        Ok(())
    }

    fn push_bytes(&mut self, mut chunk: &[u8], sink: &mut Sink<'_>) -> Result<(), LexError> {
        // Complete a character split by the previous chunk boundary.
        while self.carry_len > 0 && !chunk.is_empty() {
            self.carry[self.carry_len] = chunk[0];
            self.carry_len += 1;
            chunk = &chunk[1..];
            match std::str::from_utf8(&self.carry[..self.carry_len]) {
                Ok(_) => {
                    let (carry, len) = (self.carry, self.carry_len);
                    self.carry_len = 0;
                    let text = std::str::from_utf8(&carry[..len]).expect("validated character");
                    self.lex(text, sink)?;
                }
                Err(error) if error.error_len().is_none() => {}
                Err(_) => {
                    return Err(LexError::InvalidUtf8 {
                        offset: self.offset,
                    })
                }
            }
        }
        if chunk.is_empty() {
            return Ok(());
        }
        let (valid, invalid) = match std::str::from_utf8(chunk) {
            Ok(text) => (text, None),
            Err(error) => (
                std::str::from_utf8(&chunk[..error.valid_up_to()]).expect("validated prefix"),
                Some(error),
            ),
        };
        self.lex(valid, sink)?;
        if let Some(error) = invalid {
            if error.error_len().is_some() {
                return Err(LexError::InvalidUtf8 {
                    offset: self.offset,
                });
            }
            // At most three bytes of an incomplete character remain.
            let tail = &chunk[valid.len()..];
            self.carry[..tail.len()].copy_from_slice(tail);
            self.carry_len = tail.len();
        }
        Ok(())
    }

    fn syntax<T>(&self) -> Result<T, LexError> {
        Err(LexError::Syntax {
            offset: self.offset,
        })
    }

    fn begin_value(&mut self) -> Result<(), LexError> {
        if !matches!(self.expect, Expect::Value | Expect::ValueOrEnd) {
            return self.syntax();
        }
        if let Some(frame) = self.path.frames.last_mut() {
            if !frame.object {
                frame.count += 1;
            }
        }
        Ok(())
    }

    fn value_done(&mut self) {
        self.mode = Mode::Between;
        self.expect = if self.path.is_root() {
            Expect::End
        } else {
            Expect::CommaOrEnd
        };
    }

    fn end_scalar(&mut self, sink: &mut Sink<'_>) -> Result<(), LexError> {
        if !self.token.complete() {
            return self.syntax();
        }
        sink(
            &self.path,
            Span {
                start: self.token_start,
                end: self.offset,
            },
            Event::Scalar {
                raw: &self.scalar,
                truncated: self.scalar_truncated,
            },
        );
        self.value_done();
        Ok(())
    }

    fn push_key(&mut self, text: &str) {
        if self.key_truncated {
            return;
        }
        let room = self.limits.max_key_bytes.saturating_sub(self.key.len());
        if text.len() <= room {
            self.key.push_str(text);
        } else {
            let mut cut = room;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            self.key.push_str(&text[..cut]);
            self.key_truncated = true;
        }
    }

    /// Lex whole characters. `text` never starts or ends inside a character.
    fn lex(&mut self, text: &str, sink: &mut Sink<'_>) -> Result<(), LexError> {
        let bytes = text.as_bytes();
        let mut at = 0;
        while at < bytes.len() {
            let byte = bytes[at];
            match self.mode {
                Mode::Between => {
                    match byte {
                        b' ' | b'\t' | b'\n' | b'\r' => {}
                        b'{' | b'[' => {
                            let object = byte == b'{';
                            self.begin_value()?;
                            if self.path.depth() >= self.limits.max_depth {
                                self.mode = Mode::Opaque;
                                self.opaque_depth = 1;
                                self.opaque_string = false;
                                self.opaque_escape = false;
                                self.opaque_start = self.offset;
                            } else {
                                sink(
                                    &self.path,
                                    Span::at(self.offset, 1),
                                    if object {
                                        Event::ObjectStart
                                    } else {
                                        Event::ArrayStart
                                    },
                                );
                                self.path.push(object);
                                self.expect = if object {
                                    Expect::KeyOrEnd
                                } else {
                                    Expect::ValueOrEnd
                                };
                            }
                        }
                        b'}' | b']' => {
                            let object = byte == b'}';
                            let open = self.path.frames.last().map(|frame| frame.object);
                            let allowed = match self.expect {
                                Expect::CommaOrEnd => true,
                                Expect::KeyOrEnd => object,
                                Expect::ValueOrEnd => !object,
                                _ => false,
                            };
                            if open != Some(object) || !allowed {
                                return self.syntax();
                            }
                            let len = self.path.pop();
                            sink(
                                &self.path,
                                Span::at(self.offset, 1),
                                if object {
                                    Event::ObjectEnd { len }
                                } else {
                                    Event::ArrayEnd { len }
                                },
                            );
                            self.value_done();
                        }
                        b',' => {
                            if self.expect != Expect::CommaOrEnd {
                                return self.syntax();
                            }
                            let object = self.path.frames.last().is_some_and(|frame| frame.object);
                            self.expect = if object { Expect::Key } else { Expect::Value };
                        }
                        b':' => {
                            if self.expect != Expect::Colon {
                                return self.syntax();
                            }
                            self.expect = Expect::Value;
                        }
                        b'"' => {
                            if matches!(self.expect, Expect::KeyOrEnd | Expect::Key) {
                                self.in_key = true;
                                self.key.clear();
                                self.key_truncated = false;
                                self.token_start = self.offset;
                            } else {
                                self.begin_value()?;
                                self.in_key = false;
                                sink(&self.path, Span::at(self.offset, 1), Event::StringStart);
                            }
                            self.mode = Mode::Str;
                        }
                        _ => {
                            let Some(token) = Token::start(byte) else {
                                return self.syntax();
                            };
                            self.begin_value()?;
                            self.token = token;
                            self.token_start = self.offset;
                            self.scalar.clear();
                            self.scalar_truncated = self.limits.max_scalar_bytes == 0;
                            if !self.scalar_truncated {
                                self.scalar.push(byte as char);
                            }
                            self.mode = Mode::Scalar;
                        }
                    }
                    at += 1;
                    self.offset += 1;
                }
                Mode::Str => {
                    let rest = &bytes[at..];
                    let run = rest
                        .iter()
                        .position(|&b| b == b'"' || b == b'\\' || b < 0x20)
                        .unwrap_or(rest.len());
                    if run > 0 {
                        // Both ends are character boundaries: the run starts
                        // after an ASCII byte or at the start of `text`, and
                        // stops before one or at its end.
                        let plain = &text[at..at + run];
                        if self.in_key {
                            self.push_key(plain);
                        } else {
                            sink(
                                &self.path,
                                Span::at(self.offset, run),
                                Event::StringChunk { text: plain },
                            );
                        }
                        at += run;
                        self.offset += run as u64;
                        continue;
                    }
                    match byte {
                        b'"' => {
                            if self.in_key {
                                self.path.set_key(&self.key);
                                sink(
                                    &self.path,
                                    Span {
                                        start: self.token_start,
                                        end: self.offset + 1,
                                    },
                                    Event::Key {
                                        text: &self.key,
                                        truncated: self.key_truncated,
                                    },
                                );
                                self.mode = Mode::Between;
                                self.expect = Expect::Colon;
                            } else {
                                sink(&self.path, Span::at(self.offset, 1), Event::StringEnd);
                                self.value_done();
                            }
                        }
                        b'\\' => {
                            self.esc[0] = byte;
                            self.esc_len = 1;
                            self.esc_start = self.offset;
                            self.mode = Mode::Esc;
                        }
                        // Raw control characters are not allowed in strings.
                        _ => return self.syntax(),
                    }
                    at += 1;
                    self.offset += 1;
                }
                Mode::Esc => {
                    let decoded = self.escape_byte(byte)?;
                    at += 1;
                    self.offset += 1;
                    if let Some(ch) = decoded {
                        let mut buffer = [0; 4];
                        let decoded = ch.encode_utf8(&mut buffer);
                        if self.in_key {
                            self.push_key(decoded);
                        } else {
                            sink(
                                &self.path,
                                Span {
                                    start: self.esc_start,
                                    end: self.offset,
                                },
                                Event::StringChunk { text: decoded },
                            );
                        }
                        self.mode = Mode::Str;
                    }
                }
                Mode::Scalar => {
                    if matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b',' | b'}' | b']') {
                        // The delimiter is lexed again in `Between`.
                        self.end_scalar(sink)?;
                        continue;
                    }
                    let Some(token) = self.token.step(byte) else {
                        return self.syntax();
                    };
                    self.token = token;
                    if self.scalar.len() < self.limits.max_scalar_bytes {
                        self.scalar.push(byte as char);
                    } else {
                        self.scalar_truncated = true;
                    }
                    at += 1;
                    self.offset += 1;
                }
                Mode::Opaque => {
                    at += 1;
                    self.offset += 1;
                    if self.opaque_string {
                        if self.opaque_escape {
                            self.opaque_escape = false;
                        } else if byte == b'\\' {
                            self.opaque_escape = true;
                        } else if byte == b'"' {
                            self.opaque_string = false;
                        }
                        continue;
                    }
                    match byte {
                        b'"' => self.opaque_string = true,
                        b'{' | b'[' => self.opaque_depth += 1,
                        b'}' | b']' => {
                            self.opaque_depth -= 1;
                            if self.opaque_depth == 0 {
                                sink(
                                    &self.path,
                                    Span {
                                        start: self.opaque_start,
                                        end: self.offset,
                                    },
                                    Event::Opaque { last: true },
                                );
                                self.opaque_start = self.offset;
                                self.value_done();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        if self.mode == Mode::Opaque && self.offset > self.opaque_start {
            sink(
                &self.path,
                Span {
                    start: self.opaque_start,
                    end: self.offset,
                },
                Event::Opaque { last: false },
            );
            self.opaque_start = self.offset;
        }
        Ok(())
    }

    /// Add one byte to the pending escape sequence. Returns the decoded
    /// character once the sequence (or surrogate pair) is complete.
    fn escape_byte(&mut self, byte: u8) -> Result<Option<char>, LexError> {
        self.esc[self.esc_len] = byte;
        self.esc_len += 1;
        let hex = byte.is_ascii_hexdigit();
        let unit = |digits: &[u8]| {
            digits.iter().fold(0u32, |value, digit| {
                value * 16 + (*digit as char).to_digit(16).unwrap_or(0)
            })
        };
        match self.esc_len {
            2 => Ok(Some(match byte {
                b'"' => '"',
                b'\\' => '\\',
                b'/' => '/',
                b'b' => '\u{8}',
                b'f' => '\u{c}',
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                b'u' => return Ok(None),
                _ => return self.syntax(),
            })),
            3..=5 | 9..=11 if hex => Ok(None),
            6 if hex => match unit(&self.esc[2..6]) {
                // A leading surrogate needs its trailing half.
                0xD800..=0xDBFF => Ok(None),
                0xDC00..=0xDFFF => self.syntax(),
                code => Ok(char::from_u32(code)),
            },
            7 if byte == b'\\' => Ok(None),
            8 if byte == b'u' => Ok(None),
            12 if hex => {
                let (high, low) = (unit(&self.esc[2..6]), unit(&self.esc[8..12]));
                if !(0xDC00..=0xDFFF).contains(&low) {
                    return self.syntax();
                }
                Ok(char::from_u32(
                    0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00),
                ))
            }
            _ => self.syntax(),
        }
    }
}
