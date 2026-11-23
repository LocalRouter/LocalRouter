//! The streaming request pipeline: an inspected request body is forwarded
//! while it is still uploading, without ever releasing a byte that detection
//! has not yet had the chance to protect.
//!
//! # Units and cursors
//!
//! The body is the top-level request object. It is cut into *units*: every
//! member of that object (`"key": value`), except that the `messages` and
//! `input` arrays, which carry the conversation and so nearly all the bytes,
//! are cut into their elements. A unit is the granularity of everything that
//! inspects or rewrites the body: detection, masking, attachments, the
//! restoration contract and conversation correlation all run on one unit at
//! a time with the same functions the buffered path runs on the whole
//! document, which is what keeps the two paths equal.
//!
//! - The **head** lexes bytes as they arrive (tracking units and sniffing
//!   metadata) and inspects each unit as soon as it is complete: the session
//!   detects secrets in it and registers them for masking.
//! - The **tail** releases units in document order. A unit is released only
//!   once every byte up to `lag` bytes past its end has been inspected, so a
//!   secret detected within that distance masks its earlier occurrences too.
//!   The tail re-parses the unit, lets the session mask it, and emits the
//!   original bytes when nothing changed (or its re-serialization when
//!   something did).
//!
//! # Fail toward hold
//!
//! Nothing is released before [`RequestPipeline::ready_to_open`] holds: more
//! than `lag` bytes have arrived (so a body of at most `lag` bytes is never
//! streamed and keeps the buffered path exactly), the body is an object, its
//! string `model` is known and a conversation member has started, which fixes
//! how every unit is walked. Anything this design cannot keep equal to the
//! buffered path (a duplicate top-level key, a member whose detection needs
//! the whole document, a lexing error) makes the pipeline give up before
//! release, so the transport finishes the request on the buffered path; after
//! release it aborts the upload instead. An incomplete unit stops the head's
//! inspected frontier, so a huge unit holds everything behind it up to the
//! hold cap rather than letting anything uninspected through.

use std::collections::{HashSet, VecDeque};
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body::{Body, Frame};
use lr_secret_scanner::request_stream::{
    Event, Lexer, MetadataSniffer, Path, RequestMetadata, Segment, Span,
};
use serde_json::Value;

/// Most decoded bytes a streamed request holds at once (today's buffered
/// request limit).
pub const MAX_HOLD: usize = crate::websocket::MAX_MESSAGE;
/// Units the tail masks in one call, by size, so the vault is read once per
/// batch rather than once per message.
const TAIL_BATCH: usize = 4 * 1024 * 1024;
/// Top-level arrays cut into their elements.
const SPLIT_KEYS: [&str; 2] = ["messages", "input"];
/// Top-level members that make the request a protocol request object.
const CONVERSATION_KEYS: [&str; 3] = ["messages", "input", "prompt"];
/// Top-level members whose meaning depends on the whole document: detection
/// reads them together with other members, or they change how every other
/// member is walked.
const WHOLE_DOCUMENT_KEYS: [&str; 5] = [
    "commands",
    "questions",
    "state",
    "signature",
    "thought_signature",
];
/// Diagnostic for a secret first detected after an occurrence of it had
/// already been released (only possible for bodies above the lag).
pub const EARLIER_OCCURRENCE: &str =
    "A detected secret also occurred earlier in this request, in content that was forwarded before it was detected";

/// Where a unit sits in the request document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitScope {
    /// The value of the top-level member `key`.
    RootMember { key: String },
    /// Element `index` of the top-level `messages`/`input` array `key`.
    Element { key: String, index: u64 },
}

impl UnitScope {
    pub fn key(&self) -> &str {
        match self {
            Self::RootMember { key } | Self::Element { key, .. } => key,
        }
    }
}

/// Why a streamed upload was ended early. The upstream connection is reset,
/// so it never sees a complete body; the client is answered with `status`
/// when the upstream has not answered yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamAbort {
    pub status: u16,
    pub message: String,
}

impl StreamAbort {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

/// One unit handed to [`StreamSession::mask`].
#[derive(Debug)]
pub struct TailUnit {
    pub scope: UnitScope,
    /// The unit's value; the session rewrites it in place.
    pub value: Value,
    /// Set by the session to leave a top-level member out of the forwarded
    /// body (only honored for [`UnitScope::RootMember`]).
    pub drop: bool,
}

/// The policy side of one streamed request, implemented by the firewall.
/// Called from a blocking thread, one call at a time.
pub trait StreamSession: Send {
    /// A unit is complete. Detect and register its secrets now; returns the
    /// originals newly registered for masking.
    fn inspect(&mut self, scope: &UnitScope, value: &Value) -> Result<Vec<String>, StreamAbort>;
    /// Units about to be released, in document order. Rewrite them in place.
    fn mask(&mut self, units: &mut [TailUnit]) -> Result<(), StreamAbort>;
    /// Report a protection diagnostic for this request.
    fn diagnostic(&mut self, message: &str);
    /// The whole body was forwarded.
    fn finish(&mut self, metadata: &RequestMetadata);
}

/// The pieces of the forwarded document, in order.
#[derive(Debug)]
enum PieceKind {
    RootOpen,
    Member {
        key: String,
        key_span: Span,
        value: Span,
    },
    SplitOpen {
        key: String,
        key_span: Span,
    },
    Element {
        key: String,
        index: u64,
        value: Span,
    },
    SplitClose,
    RootClose,
}

#[derive(Debug)]
struct Piece {
    kind: PieceKind,
    /// First byte the piece needs from the hold buffer.
    start: u64,
    /// One past its last byte.
    end: u64,
    inspected: bool,
}

impl Piece {
    fn unit(&self) -> Option<(UnitScope, Span)> {
        match &self.kind {
            PieceKind::Member { key, value, .. } => {
                Some((UnitScope::RootMember { key: key.clone() }, *value))
            }
            PieceKind::Element { key, index, value } => Some((
                UnitScope::Element {
                    key: key.clone(),
                    index: *index,
                },
                *value,
            )),
            _ => None,
        }
    }
}

/// Unit boundaries, tracked from lexer events.
#[derive(Debug, Default)]
struct Structure {
    root_open: bool,
    root_closed: bool,
    split: Option<String>,
    pending_key: Option<(String, Span)>,
    open_member: Option<(String, Span, u64)>,
    open_element: Option<(u64, u64)>,
    keys: HashSet<String>,
    conversation: bool,
    /// Why this body cannot be streamed.
    problem: Option<String>,
}

impl Structure {
    /// Start of the unit being lexed, if one is open.
    fn open_start(&self) -> Option<u64> {
        self.open_member
            .as_ref()
            .map(|(_, key, _)| key.start)
            .or(self.open_element.map(|(_, start)| start))
            .or(self.pending_key.as_ref().map(|(_, span)| span.start))
    }

    fn fail(&mut self, problem: impl Into<String>) {
        self.problem.get_or_insert_with(|| problem.into());
    }

    fn observe(
        &mut self,
        pieces: &mut VecDeque<Piece>,
        path: &Path,
        span: Span,
        event: &Event<'_>,
    ) {
        let value_start = matches!(
            event,
            Event::ObjectStart | Event::ArrayStart | Event::StringStart
        );
        let value_end = matches!(
            event,
            Event::ObjectEnd { .. } | Event::ArrayEnd { .. } | Event::StringEnd
        );
        let scalar = matches!(event, Event::Scalar { .. });
        match path.depth() {
            0 => match event {
                Event::ObjectStart => {
                    self.root_open = true;
                    pieces.push_back(Piece {
                        kind: PieceKind::RootOpen,
                        start: span.start,
                        end: span.end,
                        inspected: true,
                    });
                }
                Event::ObjectEnd { .. } => {
                    self.root_closed = true;
                    pieces.push_back(Piece {
                        kind: PieceKind::RootClose,
                        start: span.start,
                        end: span.end,
                        inspected: true,
                    });
                }
                _ => self.fail("The request body is not a JSON object"),
            },
            1 => {
                if let Event::Key { text, truncated } = event {
                    if *truncated {
                        self.fail("A top-level key is too long to inspect");
                    }
                    if !self.keys.insert((*text).to_owned()) {
                        self.fail("The request has a duplicate top-level key");
                    }
                    if WHOLE_DOCUMENT_KEYS.contains(text) {
                        self.fail(format!(
                            "The top-level `{text}` member needs the whole request"
                        ));
                    }
                    self.conversation |= CONVERSATION_KEYS.contains(text);
                    self.pending_key = Some(((*text).to_owned(), span));
                    return;
                }
                if value_start || scalar {
                    let Some((key, key_span)) = self.pending_key.take() else {
                        return;
                    };
                    if matches!(event, Event::ArrayStart) && SPLIT_KEYS.contains(&key.as_str()) {
                        pieces.push_back(Piece {
                            kind: PieceKind::SplitOpen {
                                key: key.clone(),
                                key_span,
                            },
                            start: key_span.start,
                            end: span.end,
                            inspected: true,
                        });
                        self.split = Some(key);
                    } else if scalar {
                        pieces.push_back(Piece {
                            kind: PieceKind::Member {
                                key,
                                key_span,
                                value: span,
                            },
                            start: key_span.start,
                            end: span.end,
                            inspected: false,
                        });
                    } else {
                        self.open_member = Some((key, key_span, span.start));
                    }
                } else if value_end {
                    if self.split.is_some() && matches!(event, Event::ArrayEnd { .. }) {
                        self.split = None;
                        pieces.push_back(Piece {
                            kind: PieceKind::SplitClose,
                            start: span.start,
                            end: span.end,
                            inspected: true,
                        });
                    } else if let Some((key, key_span, start)) = self.open_member.take() {
                        pieces.push_back(Piece {
                            kind: PieceKind::Member {
                                key,
                                key_span,
                                value: Span {
                                    start,
                                    end: span.end,
                                },
                            },
                            start: key_span.start,
                            end: span.end,
                            inspected: false,
                        });
                    }
                }
            }
            2 => {
                let Some(split) = self.split.as_deref() else {
                    return;
                };
                if path.segment(0) != Some(Segment::Key(split)) {
                    return;
                }
                let Some(Segment::Index(index)) = path.segment(1) else {
                    return;
                };
                if scalar {
                    pieces.push_back(Piece {
                        kind: PieceKind::Element {
                            key: split.to_owned(),
                            index,
                            value: span,
                        },
                        start: span.start,
                        end: span.end,
                        inspected: false,
                    });
                } else if value_start {
                    self.open_element = Some((index, span.start));
                } else if value_end {
                    if let Some((index, start)) = self.open_element.take() {
                        pieces.push_back(Piece {
                            kind: PieceKind::Element {
                                key: split.to_owned(),
                                index,
                                value: Span {
                                    start,
                                    end: span.end,
                                },
                            },
                            start,
                            end: span.end,
                            inspected: false,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

/// Bit set of 64-bit fingerprints of released tokens (a Bloom filter), so a
/// secret detected late can be recognized as already forwarded. False
/// positives only cause a spurious diagnostic.
struct Fingerprints {
    bits: Vec<u64>,
}

/// Shortest token remembered (the plan's "string tokens of 8 or more bytes").
const MIN_TOKEN: usize = 8;
const FINGERPRINT_BITS: usize = 1 << 24;

impl Fingerprints {
    fn new() -> Self {
        Self {
            bits: vec![0; FINGERPRINT_BITS / 64],
        }
    }

    fn separator(byte: u8) -> bool {
        byte.is_ascii_whitespace()
            || matches!(
                byte,
                b'"' | b'\\'
                    | b','
                    | b'{'
                    | b'}'
                    | b'['
                    | b']'
                    | b'\''
                    | b'<'
                    | b'>'
                    | b'('
                    | b')'
                    | b';'
                    | b'`'
            )
    }

    fn tokens(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
        bytes
            .split(|b| Self::separator(*b))
            .filter(|token| token.len() >= MIN_TOKEN)
    }

    fn positions(token: &[u8]) -> [usize; 3] {
        // FNV-1a, then two derived probes.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in token {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        let second = hash.rotate_left(21).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let third = hash.rotate_left(42) ^ second;
        [hash, second, third].map(|h| (h as usize) % FINGERPRINT_BITS)
    }

    fn add(&mut self, bytes: &[u8]) {
        for token in Self::tokens(bytes) {
            for bit in Self::positions(token) {
                self.bits[bit / 64] |= 1 << (bit % 64);
            }
        }
    }

    fn contains(&self, secret: &str) -> bool {
        Self::tokens(secret.as_bytes()).any(|token| {
            Self::positions(token)
                .iter()
                .all(|bit| self.bits[bit / 64] & (1 << (bit % 64)) != 0)
        })
    }
}

/// What the monitor learns about a streamed body once it was forwarded.
#[derive(Debug, Clone, Default)]
pub struct StreamSummary {
    pub metadata: RequestMetadata,
    pub correlation: lr_monitor::CorrelationParts,
    /// Most decoded bytes held at once.
    pub max_held: usize,
}

/// The streaming pipeline of one request body (decoded bytes in, forwarded
/// bytes out). Synchronous; the transport drives it.
pub struct RequestPipeline {
    lag: u64,
    hold_cap: usize,
    held: Vec<u8>,
    /// Absolute offset of `held[0]`.
    base: u64,
    /// Bytes below this offset are no longer needed.
    live_from: u64,
    lexer: Lexer,
    sniffer: MetadataSniffer,
    structure: Structure,
    pieces: VecDeque<Piece>,
    failed: Option<String>,
    open: bool,
    finished: bool,
    max_held: usize,
    root_comma: bool,
    element_comma: bool,
    fingerprints: Option<Fingerprints>,
    messages: Option<lr_monitor::MessageHistory>,
    input: Option<lr_monitor::MessageHistory>,
    input_string: Option<String>,
    forwarded_session: Option<Value>,
}

impl RequestPipeline {
    pub fn new(lag: usize, hold_cap: usize) -> Self {
        Self {
            lag: lag as u64,
            hold_cap,
            held: Vec::new(),
            base: 0,
            live_from: 0,
            lexer: Lexer::new(),
            sniffer: MetadataSniffer::new(),
            structure: Structure::default(),
            pieces: VecDeque::new(),
            failed: None,
            open: false,
            finished: false,
            max_held: 0,
            root_comma: false,
            element_comma: false,
            fingerprints: None,
            messages: None,
            input: None,
            input_string: None,
            forwarded_session: None,
        }
    }

    /// Sniffed top-level metadata so far.
    pub fn metadata(&self) -> &RequestMetadata {
        self.sniffer.metadata()
    }

    /// Decoded bytes received so far.
    pub fn received(&self) -> u64 {
        self.lexer.bytes_fed()
    }

    /// Most decoded bytes held at once so far.
    pub fn max_held(&self) -> usize {
        self.max_held
    }

    /// Why this body cannot be streamed, once that is known. The transport
    /// then finishes the request on the buffered path.
    pub fn unstreamable(&self) -> Option<&str> {
        self.failed.as_deref().or(self.structure.problem.as_deref())
    }

    /// Whether the body has grown beyond the lag with everything that fixes
    /// its treatment known: the transport may now ask the firewall to start
    /// streaming.
    pub fn ready_to_open(&self) -> bool {
        !self.open
            && self.unstreamable().is_none()
            && self.lexer.bytes_fed() > self.lag
            && self.root_ready()
    }

    fn root_ready(&self) -> bool {
        self.structure.root_open
            && self.structure.conversation
            && self.sniffer.metadata().model.value().is_some()
            && !self.sniffer.metadata().duplicate_keys
    }

    /// Start releasing: from now on `push` inspects and forwards.
    pub fn open(&mut self) {
        self.open = true;
        self.fingerprints = Some(Fingerprints::new());
    }

    fn problem(&self) -> Option<StreamAbort> {
        let problem = self.unstreamable().map(str::to_owned).or_else(|| {
            self.sniffer
                .metadata()
                .duplicate_keys
                .then(|| "The request has a duplicate top-level key".to_owned())
        })?;
        Some(StreamAbort::new(400, problem))
    }

    /// Feed decoded body bytes. Before [`open`](Self::open) this only
    /// lexes and holds; afterwards it also inspects and appends releasable
    /// output to `out`.
    pub fn push(
        &mut self,
        data: &[u8],
        session: Option<&mut dyn StreamSession>,
        out: &mut Vec<u8>,
    ) -> Result<(), StreamAbort> {
        if self.finished {
            return Err(StreamAbort::new(
                400,
                "Request body continued after its end",
            ));
        }
        if self.unstreamable().is_some() && !self.open {
            // Given up: the transport keeps the raw body for the buffered path.
            return Ok(());
        }
        self.held.extend_from_slice(data);
        let live = self.held.len() - (self.live_from - self.base) as usize;
        self.max_held = self.max_held.max(live);
        if live > self.hold_cap {
            let message = format!(
                "request body exceeds LocalRouter's {} MiB proxy limit for content it cannot yet forward",
                self.hold_cap / (1024 * 1024)
            );
            if !self.open {
                // The buffered path applies its own limits to the raw body.
                self.failed = Some(message);
                self.held = Vec::new();
                return Ok(());
            }
            return Err(StreamAbort::new(413, message));
        }
        let lexed = self.lexer.push(data, |path, span, event| {
            self.structure.observe(&mut self.pieces, path, span, &event);
            self.sniffer.observe(path, &event);
        });
        if let Err(error) = lexed {
            self.failed = Some(format!(
                "Request JSON is invalid at byte {}",
                error.offset()
            ));
        }
        if let Some(problem) = self.problem() {
            if !self.open {
                self.held = Vec::new();
                return Ok(());
            }
            return Err(problem);
        }
        if let (true, Some(session)) = (self.open, session) {
            self.advance(session, false, out)?;
        }
        Ok(())
    }

    /// The body ended: release everything that is left.
    pub fn finish(
        &mut self,
        session: &mut dyn StreamSession,
        out: &mut Vec<u8>,
    ) -> Result<StreamSummary, StreamAbort> {
        if !self.open {
            return Err(StreamAbort::new(
                500,
                "Request stream finished before it opened",
            ));
        }
        let lexed = self.lexer.finish(|path, span, event| {
            self.structure.observe(&mut self.pieces, path, span, &event);
            self.sniffer.observe(path, &event);
        });
        if let Err(error) = lexed {
            return Err(StreamAbort::new(
                400,
                format!("Request JSON is invalid at byte {}", error.offset()),
            ));
        }
        if let Some(problem) = self.problem() {
            return Err(problem);
        }
        if !self.structure.root_closed {
            return Err(StreamAbort::new(400, "Request JSON ended early"));
        }
        self.advance(session, true, out)?;
        if !self.pieces.is_empty() {
            return Err(StreamAbort::new(
                500,
                "Request stream ended with unreleased content",
            ));
        }
        self.finished = true;
        session.finish(self.sniffer.metadata());
        let history = if self.structure.keys.contains("messages") {
            self.messages.take()
        } else {
            self.input.take()
        };
        Ok(StreamSummary {
            metadata: self.sniffer.metadata().clone(),
            correlation: lr_monitor::CorrelationParts {
                session: self.forwarded_session.take(),
                previous_response_id: self
                    .sniffer
                    .metadata()
                    .previous_response_id
                    .value()
                    .cloned(),
                history,
                input_string: self.input_string.take(),
            },
            max_held: self.max_held,
        })
    }

    fn bytes(&self, span: Span) -> &[u8] {
        let start = (span.start - self.base) as usize;
        let end = (span.end - self.base) as usize;
        &self.held[start..end]
    }

    fn parse(&self, span: Span) -> Result<Value, StreamAbort> {
        serde_json::from_slice(self.bytes(span))
            .map_err(|_| StreamAbort::new(400, "Request JSON could not be inspected"))
    }

    fn advance(
        &mut self,
        session: &mut dyn StreamSession,
        eof: bool,
        out: &mut Vec<u8>,
    ) -> Result<(), StreamAbort> {
        // Head: inspect every unit that completed.
        for i in 0..self.pieces.len() {
            if self.pieces[i].inspected {
                continue;
            }
            let Some((scope, span)) = self.pieces[i].unit() else {
                continue;
            };
            let value = self.parse(span)?;
            let registered = session.inspect(&scope, &value)?;
            if let Some(fingerprints) = &self.fingerprints {
                if registered
                    .iter()
                    .any(|secret| fingerprints.contains(secret))
                {
                    session.diagnostic(EARLIER_OCCURRENCE);
                }
            }
            self.pieces[i].inspected = true;
        }
        // Everything before the first unit still being lexed is inspected.
        let frontier = if eof {
            u64::MAX
        } else {
            self.structure.open_start().unwrap_or(self.lexer.settled())
        };
        loop {
            let mut count = 0;
            let mut size = 0;
            while let Some(piece) = self.pieces.get(count) {
                if size >= TAIL_BATCH || piece.end.saturating_add(self.lag) > frontier {
                    break;
                }
                size += (piece.end - piece.start) as usize;
                count += 1;
            }
            if count == 0 {
                break;
            }
            let batch: Vec<Piece> = self.pieces.drain(..count).collect();
            self.release(session, batch, out)?;
        }
        self.live_from = self
            .pieces
            .front()
            .map(|p| p.start)
            .or(self.structure.open_start())
            .unwrap_or(self.lexer.settled())
            .max(self.live_from);
        let dead = (self.live_from - self.base) as usize;
        if dead > 0 && (dead >= self.held.len() / 2 || eof) {
            self.held.drain(..dead);
            self.base = self.live_from;
        }
        Ok(())
    }

    fn release(
        &mut self,
        session: &mut dyn StreamSession,
        batch: Vec<Piece>,
        out: &mut Vec<u8>,
    ) -> Result<(), StreamAbort> {
        let mut originals = Vec::new();
        let mut units = Vec::new();
        for piece in &batch {
            if let Some((scope, span)) = piece.unit() {
                let value = self.parse(span)?;
                originals.push(value.clone());
                units.push(TailUnit {
                    scope,
                    value,
                    drop: false,
                });
            }
        }
        session.mask(&mut units)?;
        let mut units = units.into_iter().zip(originals);
        for piece in batch {
            match &piece.kind {
                PieceKind::RootOpen => {
                    out.push(b'{');
                    self.root_comma = false;
                }
                PieceKind::RootClose => out.push(b'}'),
                PieceKind::SplitOpen { key, key_span } => {
                    if self.root_comma {
                        out.push(b',');
                    }
                    out.extend_from_slice(self.bytes(*key_span));
                    out.extend_from_slice(b":[");
                    self.root_comma = true;
                    self.element_comma = false;
                    *self.history(key) = Some(Default::default());
                }
                PieceKind::SplitClose => out.push(b']'),
                PieceKind::Member {
                    key,
                    key_span,
                    value,
                } => {
                    let (unit, original) = units.next().expect("one unit per member");
                    if unit.drop {
                        continue;
                    }
                    if self.root_comma {
                        out.push(b',');
                    }
                    out.extend_from_slice(self.bytes(*key_span));
                    out.push(b':');
                    self.emit_value(&unit.value, &original, *value, out)?;
                    self.root_comma = true;
                    self.observe_member(key, &unit.value);
                }
                PieceKind::Element { key, value, .. } => {
                    let (unit, original) = units.next().expect("one unit per element");
                    if self.element_comma {
                        out.push(b',');
                    }
                    self.emit_value(&unit.value, &original, *value, out)?;
                    self.element_comma = true;
                    self.history(key)
                        .get_or_insert_with(Default::default)
                        .push(&unit.value);
                }
            }
        }
        Ok(())
    }

    /// The original bytes of an unchanged unit, else its new serialization.
    fn emit_value(
        &mut self,
        value: &Value,
        original: &Value,
        span: Span,
        out: &mut Vec<u8>,
    ) -> Result<(), StreamAbort> {
        let start = out.len();
        if value == original {
            out.extend_from_slice(self.bytes(span));
        } else {
            serde_json::to_writer(&mut *out, value)
                .map_err(|_| StreamAbort::new(500, "Masked request could not be encoded"))?;
        }
        if let Some(fingerprints) = &mut self.fingerprints {
            fingerprints.add(&out[start..]);
        }
        Ok(())
    }

    fn history(&mut self, key: &str) -> &mut Option<lr_monitor::MessageHistory> {
        if key == "messages" {
            &mut self.messages
        } else {
            &mut self.input
        }
    }

    fn observe_member(&mut self, key: &str, value: &Value) {
        match key {
            "extensions" => {
                self.forwarded_session = value.get("localrouter.session").cloned();
            }
            "input" => self.input_string = value.as_str().map(str::to_owned),
            _ => {}
        }
    }
}

/// The upstream half of a streamed request body.
pub(crate) enum Chunk {
    Data(Bytes),
    End,
}

/// Sends a streamed body to the upstream. Dropping it without
/// [`end`](Self::end) fails the body, so the upstream connection is reset
/// rather than sent a complete request.
pub(crate) struct UpstreamSender(tokio::sync::mpsc::Sender<Chunk>);

impl UpstreamSender {
    /// Queue forwarded bytes; `false` once the upstream stopped reading.
    pub(crate) async fn send(&self, data: Bytes) -> bool {
        data.is_empty() || self.0.send(Chunk::Data(data)).await.is_ok()
    }

    /// End the body normally.
    pub(crate) async fn end(self) {
        let _ = self.0.send(Chunk::End).await;
    }
}

/// The request body hyper sends upstream for a streamed request. It has no
/// size hint, so HTTP/1.1 frames it as chunked; it ends only on an explicit
/// end, and fails (which makes hyper close the connection without the final
/// chunk) when the sender is dropped.
pub(crate) struct UpstreamBody {
    rx: tokio::sync::mpsc::Receiver<Chunk>,
    done: bool,
}

pub(crate) fn upstream_channel() -> (UpstreamSender, UpstreamBody) {
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    (UpstreamSender(tx), UpstreamBody { rx, done: false })
}

impl Body for UpstreamBody {
    type Data = Bytes;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        if self.done {
            return Poll::Ready(None);
        }
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(Chunk::Data(data))) => Poll::Ready(Some(Ok(Frame::data(data)))),
            Poll::Ready(Some(Chunk::End)) => {
                self.done = true;
                Poll::Ready(None)
            }
            Poll::Ready(None) => {
                self.done = true;
                Poll::Ready(Some(Err("request body aborted".into())))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.done
    }
}

/// Decoded bytes the driver gathers before one processing step, which runs
/// on a blocking thread.
const STEP: usize = 256 * 1024;

#[derive(Default)]
struct ProgressState {
    capture: crate::interceptor::BoundedCapture,
    abort: Option<StreamAbort>,
    summary: Option<crate::interceptor::StreamedRequest>,
}

/// What a streamed upload has forwarded so far, shared with the transport
/// for the monitor and for the status of an aborted upload.
#[derive(Clone, Default)]
pub(crate) struct StreamProgress(std::sync::Arc<parking_lot::Mutex<ProgressState>>);

impl StreamProgress {
    /// The head of the forwarded bytes, for the monitor.
    pub(crate) fn snapshot(&self) -> crate::interceptor::BoundedCapture {
        self.0.lock().capture.clone()
    }

    /// Why the upload was aborted, if it was.
    pub(crate) fn abort(&self) -> Option<StreamAbort> {
        self.0.lock().abort.clone()
    }

    /// The monitor's view of the body, once it was forwarded completely.
    pub(crate) fn summary(&self) -> Option<crate::interceptor::StreamedRequest> {
        self.0.lock().summary.clone()
    }

    fn record(&self, forwarded: &[u8], cap: usize) {
        let mut state = self.0.lock();
        let capture = &mut state.capture;
        let room = cap.saturating_sub(capture.head.len());
        capture
            .head
            .extend_from_slice(&forwarded[..room.min(forwarded.len())]);
        capture.total_len += forwarded.len() as u64;
        capture.truncated = capture.total_len > capture.head.len() as u64;
    }
}

/// The monitor's view of a streamed body from its sniffed metadata.
pub(crate) fn streamed_request(
    format: crate::wire::WireFormat,
    metadata: &RequestMetadata,
    correlation: Option<lr_monitor::CorrelationParts>,
) -> crate::interceptor::StreamedRequest {
    crate::interceptor::StreamedRequest {
        meta: crate::wire::request_meta_from_sniffed(format, metadata).unwrap_or_default(),
        tool_count: usize::try_from(metadata.tools_len()).unwrap_or(usize::MAX),
        conversation_id: crate::wire::conversation_id_from_sniffed(metadata),
        correlation,
    }
}

/// A request body that is about to stream: the client body still being
/// read, the decoder and pipeline holding what arrived so far, and the
/// firewall's session.
pub(crate) struct StreamStart {
    pub(crate) body: hyper::body::Incoming,
    pub(crate) decoder: crate::request_body::StreamDecoder,
    pub(crate) pipeline: RequestPipeline,
    pub(crate) session: Box<dyn StreamSession>,
    pub(crate) format: crate::wire::WireFormat,
    /// Bytes of forwarded content kept for the monitor.
    pub(crate) capture_cap: usize,
    /// Restores references in the response.
    pub(crate) vault: Option<std::sync::Arc<lr_secret_scanner::placeholders::Vault>>,
}

struct Work {
    decoder: crate::request_body::StreamDecoder,
    pipeline: RequestPipeline,
    session: Box<dyn StreamSession>,
}

impl Work {
    /// Decode and process one batch of client bytes; at the end of the body,
    /// release the rest.
    fn step(
        &mut self,
        data: &[u8],
        eof: bool,
    ) -> Result<(Vec<u8>, Option<StreamSummary>), StreamAbort> {
        let mut out = Vec::new();
        let mut failed = None;
        let Work {
            decoder,
            pipeline,
            session,
        } = self;
        let decoded = decoder.push(data, |piece| {
            match pipeline.push(piece, Some(session.as_mut()), &mut out) {
                Ok(()) => std::ops::ControlFlow::Continue(()),
                Err(abort) => {
                    failed = Some(abort);
                    std::ops::ControlFlow::Break(())
                }
            }
        });
        if let Some(abort) = failed {
            return Err(abort);
        }
        if let Err(error) = decoded {
            return Err(StreamAbort::new(error.status(), error.message()));
        }
        if data.is_empty() && !eof {
            // The first step releases what arrived before the stream opened.
            pipeline.push(&[], Some(session.as_mut()), &mut out)?;
        }
        if !eof {
            return Ok((out, None));
        }
        let decoder = std::mem::replace(
            decoder,
            crate::request_body::StreamDecoder::new(&http::HeaderMap::new())
                .expect("identity decoder"),
        );
        decoder
            .finish()
            .map_err(|error| StreamAbort::new(error.status(), error.message()))?;
        let summary = pipeline.finish(session.as_mut(), &mut out)?;
        Ok((out, Some(summary)))
    }
}

impl StreamStart {
    /// Start forwarding: returns the upstream body and the progress handle.
    pub(crate) fn spawn(self) -> (UpstreamBody, StreamProgress) {
        let (tx, body) = upstream_channel();
        let progress = StreamProgress::default();
        tokio::spawn(drive(self, tx, progress.clone()));
        (body, progress)
    }
}

async fn drive(start: StreamStart, tx: UpstreamSender, progress: StreamProgress) {
    use http_body_util::BodyExt;
    let StreamStart {
        mut body,
        decoder,
        pipeline,
        session,
        format,
        capture_cap,
        vault: _,
    } = start;
    let mut work = Some(Work {
        decoder,
        pipeline,
        session,
    });
    let mut eof = false;
    let mut first = true;
    let result: Result<StreamSummary, StreamAbort> = async {
        loop {
            let mut data = Vec::new();
            while !first && !eof && data.len() < STEP {
                match body.frame().await {
                    None => eof = true,
                    Some(Err(error)) => {
                        return Err(StreamAbort::new(
                            400,
                            format!("client request body: {error}"),
                        ))
                    }
                    Some(Ok(frame)) => {
                        if let Ok(chunk) = frame.into_data() {
                            data.extend_from_slice(&chunk);
                        }
                    }
                }
            }
            first = false;
            let mut taken = work.take().expect("work between steps");
            let (returned, stepped) = tokio::task::spawn_blocking(move || {
                let stepped = taken.step(&data, eof);
                (taken, stepped)
            })
            .await
            .map_err(|_| StreamAbort::new(500, "Request inspection failed"))?;
            work = Some(returned);
            let (out, summary) = stepped?;
            progress.record(&out, capture_cap);
            if !tx.send(Bytes::from(out)).await {
                return Err(StreamAbort::new(
                    502,
                    "The upstream stopped reading the request",
                ));
            }
            if let Some(summary) = summary {
                return Ok(summary);
            }
        }
    }
    .await;
    match result {
        Ok(summary) => {
            {
                let mut state = progress.0.lock();
                state.capture.complete = true;
                state.summary = Some(streamed_request(
                    format,
                    &summary.metadata,
                    Some(summary.correlation),
                ));
            }
            tx.end().await;
        }
        Err(abort) => {
            tracing::warn!("proxy: streamed request aborted: {}", abort.message);
            progress.0.lock().abort = Some(abort);
            // Dropping the sender fails the upstream body: hyper closes the
            // connection without the final chunk.
            drop(tx);
        }
    }
}

#[cfg(test)]
#[path = "request_stream_tests.rs"]
mod tests;
