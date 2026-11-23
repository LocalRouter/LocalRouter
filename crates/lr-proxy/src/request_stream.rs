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
use std::future::Future;
use std::pin::Pin;

use lr_secret_scanner::request_stream::{
    Event, Lexer, MetadataSniffer, Path, RequestMetadata, Segment, Span,
};
use serde_json::Value;

use crate::spill::{SpillBuffer, SpillError};

/// Most decoded bytes a streamed request holds in RAM when no spill is
/// configured (the buffered request limit).
pub const MAX_HOLD: usize = crate::websocket::MAX_MESSAGE;
/// Decoded bytes the driver gathers before one processing step, which runs
/// on a blocking thread.
pub(crate) const STEP: usize = 256 * 1024;
/// Units at most this large are copied out of the hold buffer and parsed
/// from memory; larger ones are parsed from a decrypting reader.
const PARSE_IN_MEMORY: u64 = 8 * 1024 * 1024;
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
    /// A JSON body to answer the client with instead of the plain-text
    /// proxy error (for example a Secret Scan denial).
    pub json: Option<Vec<u8>>,
}

impl StreamAbort {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            json: None,
        }
    }

    /// An abort answered with this JSON body.
    pub fn with_json(status: u16, message: impl Into<String>, json: Vec<u8>) -> Self {
        Self {
            json: Some(json),
            ..Self::new(status, message)
        }
    }

    pub(crate) fn spill(error: SpillError) -> Self {
        Self::new(error.status(), error.message())
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

/// A question a session needs answered (for example a Secret Scan Ask
/// popup) before content from the unit that raised it may be released. The
/// transport awaits it outside the blocking thread, while it keeps holding
/// the client's upload, and hands the result to [`StreamSession::answer`].
pub struct Question(pub Pin<Box<dyn Future<Output = Answer> + Send>>);

/// The result of a [`Question`], understood only by the session that asked.
pub struct Answer(pub Box<dyn std::any::Any + Send>);

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
    /// Whether a question is open or queued. While it is, nothing is
    /// released (the units before the one that raised it may hold earlier
    /// occurrences the answer decides about).
    fn pending(&self) -> bool {
        false
    }
    /// The next question to ask, one at a time.
    fn question(&mut self) -> Option<Question> {
        None
    }
    /// Apply the answer to the last question. Returns the originals newly
    /// registered for masking; an error ends the upload.
    fn answer(&mut self, _answer: Answer) -> Result<Vec<String>, StreamAbort> {
        Ok(Vec::new())
    }
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
    /// Bytes the body must exceed before the stream may open.
    open_after: u64,
    /// Bytes the release cursor stays behind the inspected frontier;
    /// `u64::MAX` releases nothing before the end of the body.
    lag: u64,
    /// Decoded bytes not yet released, by absolute offset.
    held: SpillBuffer,
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
    /// Released units are written to the output (false when the transport
    /// forwards the original bytes and only inspects).
    emit: bool,
    /// Start of the unit whose inspection raised the open question; nothing
    /// is released while it is set.
    gate: Option<u64>,
    root_comma: bool,
    element_comma: bool,
    fingerprints: Option<Fingerprints>,
    messages: Option<lr_monitor::MessageHistory>,
    input: Option<lr_monitor::MessageHistory>,
    input_string: Option<String>,
    forwarded_session: Option<Value>,
}

impl RequestPipeline {
    /// A pipeline that holds up to `hold_cap` bytes in RAM and opens, and
    /// releases, `lag` bytes behind its head.
    pub fn new(lag: usize, hold_cap: usize) -> Self {
        Self::with_hold(lag, SpillBuffer::ram_only(hold_cap as u64))
    }

    /// A pipeline that holds what it may not release yet in `held` (which
    /// may spill to disk).
    pub fn with_hold(lag: usize, held: SpillBuffer) -> Self {
        Self {
            open_after: lag as u64,
            lag: lag as u64,
            held,
            live_from: 0,
            lexer: Lexer::new(),
            sniffer: MetadataSniffer::new(),
            structure: Structure::default(),
            pieces: VecDeque::new(),
            failed: None,
            open: false,
            finished: false,
            max_held: 0,
            emit: true,
            gate: None,
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

    /// Bytes of the held buffer that went to disk.
    pub fn spilled_bytes(&self) -> u64 {
        self.held.spilled_bytes()
    }

    /// Why this body cannot be streamed, once that is known. The transport
    /// then finishes the request on the buffered path. A repeated key at any
    /// level the sniffer reads counts: a decision taken on the first value
    /// would not hold for the one the upstream acts on.
    pub fn unstreamable(&self) -> Option<&str> {
        self.failed
            .as_deref()
            .or(self.structure.problem.as_deref())
            .or_else(|| {
                self.sniffer
                    .metadata()
                    .duplicate_keys
                    .then_some("The request has a duplicate key")
            })
    }

    /// Whether the body has grown beyond the open threshold with everything
    /// that fixes its treatment known: the transport may now ask the
    /// firewall to start streaming.
    pub fn ready_to_open(&self) -> bool {
        !self.open
            && self.unstreamable().is_none()
            && self.lexer.bytes_fed() > self.open_after
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

    /// Release nothing before the end of the body (strict mode): every
    /// byte is inspected, and every question answered, before the first
    /// one leaves.
    pub fn hold_until_end(&mut self) {
        self.lag = u64::MAX;
    }

    /// Inspect every unit as soon as it is complete but emit nothing: the
    /// transport forwards the original bytes itself.
    pub fn inspect_only(&mut self) {
        self.lag = 0;
        self.emit = false;
    }

    /// Whether a question currently stops the release.
    pub fn gated(&self) -> bool {
        self.gate.is_some()
    }

    fn problem(&self) -> Option<StreamAbort> {
        self.unstreamable().map(|p| StreamAbort::new(400, p))
    }

    /// The first `limit` decoded bytes, while they are all still held
    /// (before the stream opened).
    pub fn head(&mut self, limit: usize) -> Option<Vec<u8>> {
        if self.held.start() > 0 {
            return None;
        }
        let to = self.held.end().min(limit as u64);
        self.held.read(0, to).ok()
    }

    /// Feed decoded body bytes. Before [`open`](Self::open) this only
    /// lexes and holds; afterwards it also inspects and appends releasable
    /// output to `out` when given a session (without one it only holds, as
    /// while a question is being asked).
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
        if let Err(error) = self.held.append(data) {
            if !self.open {
                // The buffered path applies its own limits to the raw body.
                self.failed = Some(error.message());
                self.held = SpillBuffer::ram_only(0);
                return Ok(());
            }
            return Err(StreamAbort::spill(error));
        }
        let live = (self.held.end() - self.live_from) as usize;
        self.max_held = self.max_held.max(live);
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
                self.held = SpillBuffer::ram_only(0);
                return Ok(());
            }
            return Err(problem);
        }
        if let (true, Some(session)) = (self.open, session) {
            self.advance(session, false, out)?;
        }
        Ok(())
    }

    /// Originals a session registered outside [`StreamSession::inspect`]
    /// (from an answer): report when one was already forwarded.
    pub fn registered(&mut self, secrets: &[String], session: &mut dyn StreamSession) {
        if let Some(fingerprints) = &self.fingerprints {
            if secrets.iter().any(|secret| fingerprints.contains(secret)) {
                session.diagnostic(EARLIER_OCCURRENCE);
            }
        }
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
        if self.gate.is_some() {
            // The driver answers every question before it finishes.
            return Err(StreamAbort::new(
                500,
                "Request stream ended with an unanswered question",
            ));
        }
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

    fn read_into(&mut self, span: Span, out: &mut Vec<u8>) -> Result<(), StreamAbort> {
        self.held
            .read_into(span.start, span.end, out)
            .map_err(StreamAbort::spill)
    }

    fn parse(&mut self, span: Span) -> Result<Value, StreamAbort> {
        let invalid = |_| StreamAbort::new(400, "Request JSON could not be inspected");
        if span.end - span.start <= PARSE_IN_MEMORY {
            let mut bytes = Vec::new();
            self.read_into(span, &mut bytes)?;
            let value = serde_json::from_slice(&bytes).map_err(invalid);
            zeroize::Zeroize::zeroize(&mut bytes);
            return value;
        }
        let reader = std::io::BufReader::new(self.held.reader(span.start, span.end));
        serde_json::from_reader(reader).map_err(invalid)
    }

    fn advance(
        &mut self,
        session: &mut dyn StreamSession,
        eof: bool,
        out: &mut Vec<u8>,
    ) -> Result<(), StreamAbort> {
        // Every question was answered: release again.
        if !session.pending() {
            self.gate = None;
        }
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
            self.registered(&registered, session);
            self.pieces[i].inspected = true;
            if self.gate.is_none() && session.pending() {
                // Nothing may leave before the answer.
                self.gate = Some(self.pieces[i].start);
            }
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
                // While a question is open nothing is released: a unit
                // before the one that raised it may hold an earlier
                // occurrence the answer decides about, as Redact masks it.
                if size >= TAIL_BATCH
                    || self.gate.is_some()
                    || piece.end.saturating_add(self.lag) > frontier
                {
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
        self.held.discard_before(self.live_from);
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
        // Inspect-only: the units were observed; nothing is written.
        let mut sink = Vec::new();
        let emit = self.emit;
        let out = if emit { out } else { &mut sink };
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
                    if emit {
                        self.read_into(*key_span, out)?;
                    }
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
                    // A member dropped from the output; the original bytes
                    // still carry it when nothing is emitted.
                    if unit.drop && emit {
                        continue;
                    }
                    if self.root_comma {
                        out.push(b',');
                    }
                    if emit {
                        self.read_into(*key_span, out)?;
                    }
                    out.push(b':');
                    self.emit_value(&unit.value, &original, *value, out)?;
                    self.root_comma = true;
                    // Without emitting, the original bytes are forwarded.
                    self.observe_member(key, if emit { &unit.value } else { &original });
                }
                PieceKind::Element { key, value, .. } => {
                    let (unit, original) = units.next().expect("one unit per element");
                    if self.element_comma {
                        out.push(b',');
                    }
                    self.emit_value(&unit.value, &original, *value, out)?;
                    self.element_comma = true;
                    let forwarded = if emit { &unit.value } else { &original };
                    self.history(key)
                        .get_or_insert_with(Default::default)
                        .push(forwarded);
                }
            }
            if !emit {
                out.clear();
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
        if !self.emit {
            return Ok(());
        }
        let start = out.len();
        if value == original {
            self.read_into(span, out)?;
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

pub(crate) use crate::stream_driver::{streamed_request, StreamProgress, StreamStart};

#[cfg(test)]
#[path = "request_stream_tests.rs"]
mod tests;
