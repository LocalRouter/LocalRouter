//! Bounded HTTP content decoding for policy inspection. Forward keeps the
//! original representation; rewriting sends identity bytes with fresh headers.
//!
//! Two entry points accept the same encodings: [`inspect`] decodes a buffered
//! body in one step, [`StreamDecoder`] decodes it chunk by chunk without
//! holding it.

use std::io::{Cursor, Read};
use std::ops::ControlFlow;

use bytes::Bytes;
use http::HeaderMap;

/// Decoded request bodies may be as large as the proxy buffers them: Codex
/// compresses its HTTPS fallback requests, and a long session resends its whole
/// conversation (tens of MiB) once its websocket is gone.
const LIMIT: usize = crate::websocket::MAX_MESSAGE;
const MAX_ENCODINGS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    Unsupported,
    Invalid,
    TooLarge,
}

impl DecodeError {
    pub fn status(self) -> u16 {
        match self {
            Self::Unsupported => 415,
            Self::Invalid => 400,
            Self::TooLarge => 413,
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Unsupported => "Unsupported request Content-Encoding for policy inspection",
            Self::Invalid => "Invalid compressed request body",
            Self::TooLarge => "Decoded request body exceeds the policy inspection limit",
        }
    }
}

fn bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, DecodeError> {
    let mut decoded = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut decoded)
        .map_err(|_| DecodeError::Invalid)?;
    if decoded.len() > limit {
        return Err(DecodeError::TooLarge);
    }
    Ok(decoded)
}

fn zlib(bytes: &[u8], limit: usize) -> Result<Vec<u8>, DecodeError> {
    let mut decoder = flate2::Decompress::new(true);
    let mut output = Vec::new();
    loop {
        let mut buffer = [0u8; 8192];
        let available = buffer.len().min(limit + 1 - output.len());
        let before = (decoder.total_in(), decoder.total_out());
        // Not `Finish`: miniz_oxide treats a first `Finish` call as one-shot
        // decoding and fails once the output exceeds the buffer.
        let status = decoder
            .decompress(
                &bytes[before.0 as usize..],
                &mut buffer[..available],
                flate2::FlushDecompress::None,
            )
            .map_err(|_| DecodeError::Invalid)?;
        output.extend_from_slice(&buffer[..(decoder.total_out() - before.1) as usize]);
        if output.len() > limit {
            return Err(DecodeError::TooLarge);
        }
        if status == flate2::Status::StreamEnd {
            return if decoder.total_in() == bytes.len() as u64 {
                Ok(output)
            } else {
                Err(DecodeError::Invalid)
            };
        }
        if before == (decoder.total_in(), decoder.total_out()) {
            return Err(DecodeError::Invalid);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Coding {
    Identity,
    Zstd,
    Gzip,
    Deflate,
    Brotli,
}

/// Largest zstd window accepted, as a power of two (16 MiB).
const ZSTD_WINDOW_LOG_MAX: u32 = 24;

/// The request's content codings, in the order they were applied.
fn codings(headers: &HeaderMap) -> Result<Vec<Coding>, DecodeError> {
    let mut codings = Vec::new();
    for value in headers.get_all(http::header::CONTENT_ENCODING) {
        for coding in value
            .to_str()
            .map_err(|_| DecodeError::Unsupported)?
            .split(',')
        {
            codings.push(match coding.trim().to_ascii_lowercase().as_str() {
                "identity" => Coding::Identity,
                "zstd" => Coding::Zstd,
                "gzip" => Coding::Gzip,
                "deflate" => Coding::Deflate,
                "br" => Coding::Brotli,
                _ => return Err(DecodeError::Unsupported),
            });
            if codings.len() > MAX_ENCODINGS {
                return Err(DecodeError::Unsupported);
            }
        }
    }
    Ok(codings)
}

fn decode(headers: &HeaderMap, bytes: Bytes, limit: usize) -> Result<Bytes, DecodeError> {
    if bytes.len() > limit {
        return Err(DecodeError::TooLarge);
    }
    let codings = codings(headers)?;
    let mut bytes = bytes;
    // Content-Encoding lists encodings in the order they were applied.
    for coding in codings.iter().rev() {
        let input = Cursor::new(bytes.as_ref());
        let decoded = match coding {
            Coding::Identity => continue,
            Coding::Zstd => {
                let mut decoder =
                    zstd::stream::read::Decoder::new(input).map_err(|_| DecodeError::Invalid)?;
                decoder
                    .window_log_max(ZSTD_WINDOW_LOG_MAX)
                    .map_err(|_| DecodeError::Invalid)?;
                bounded(decoder, limit)?
            }
            Coding::Gzip => bounded(flate2::read::MultiGzDecoder::new(input), limit)?,
            // HTTP deflate is a zlib wrapper, not raw DEFLATE.
            Coding::Deflate => zlib(bytes.as_ref(), limit)?,
            Coding::Brotli => bounded(brotli::Decompressor::new(input, 8192), limit)?,
        };
        bytes = Bytes::from(decoded);
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Streaming decode
// ---------------------------------------------------------------------------

/// Default cap on the bytes a [`StreamDecoder`] produces: 1 GiB.
pub const DEFAULT_STREAM_BUDGET: u64 = 1 << 30;

/// Largest piece of decoded data handed to a sink at once.
pub const DECODED_BLOCK: usize = 64 * 1024;

/// Bounds on what a [`StreamDecoder`] is willing to produce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeLimits {
    /// Most bytes any one decoding stage may produce, and most encoded bytes
    /// accepted. Exactly this many is allowed.
    pub max_decoded: u64,
    /// Most bytes a stage may produce per encoded byte received so far.
    pub max_ratio: u64,
    /// The ratio is not checked until a stage has produced more than this, so
    /// small, highly repetitive bodies pass.
    pub ratio_floor: u64,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_decoded: DEFAULT_STREAM_BUDGET,
            // DEFLATE cannot expand beyond 1032:1; zstd and brotli can.
            max_ratio: 1024,
            ratio_floor: 64 * 1024 * 1024,
        }
    }
}

/// Why a [`StreamDecoder`] stopped. Decoded bytes delivered before the error
/// were valid output of the stream so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamDecodeError {
    /// A Content-Encoding this proxy does not decode, or more than four.
    Unsupported,
    /// The bytes are not a valid stream of the declared encoding.
    Corrupt,
    /// The input ended before the encoded stream did.
    Truncated,
    /// More than [`DecodeLimits::max_decoded`] bytes.
    BudgetExceeded,
    /// Expansion beyond [`DecodeLimits::max_ratio`].
    ExpansionRatio,
    /// The sink asked to stop.
    Aborted,
}

impl StreamDecodeError {
    /// The error [`inspect`] reports for the same body. `None` for
    /// [`Aborted`](Self::Aborted) and [`ExpansionRatio`](Self::ExpansionRatio),
    /// which buffered decoding does not have.
    pub fn as_batch(self) -> Option<DecodeError> {
        match self {
            Self::Unsupported => Some(DecodeError::Unsupported),
            Self::Corrupt | Self::Truncated => Some(DecodeError::Invalid),
            Self::BudgetExceeded => Some(DecodeError::TooLarge),
            Self::ExpansionRatio | Self::Aborted => None,
        }
    }

    /// Status to answer the client with when nothing was forwarded yet.
    pub fn status(self) -> u16 {
        match self {
            Self::Unsupported => 415,
            Self::Corrupt | Self::Truncated => 400,
            Self::BudgetExceeded | Self::ExpansionRatio => 413,
            Self::Aborted => 500,
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Unsupported => DecodeError::Unsupported.message(),
            Self::Corrupt => DecodeError::Invalid.message(),
            Self::Truncated => "Compressed request body ended early",
            Self::BudgetExceeded => DecodeError::TooLarge.message(),
            Self::ExpansionRatio => "Compressed request body expands beyond the allowed ratio",
            Self::Aborted => "Request body decoding was stopped",
        }
    }
}

impl std::fmt::Display for StreamDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for StreamDecodeError {}

type Emit<'a> = dyn FnMut(&[u8]) -> Result<(), StreamDecodeError> + 'a;

/// Limits applied to every stage's output.
#[derive(Clone, Copy)]
struct Meter {
    limits: DecodeLimits,
    /// Encoded bytes received so far.
    encoded: u64,
}

impl Meter {
    fn charge(&self, produced: &mut u64, more: usize) -> Result<(), StreamDecodeError> {
        let total = *produced + more as u64;
        if total > self.limits.max_decoded {
            return Err(StreamDecodeError::BudgetExceeded);
        }
        if total > self.limits.ratio_floor
            && total > self.limits.max_ratio.saturating_mul(self.encoded)
        {
            return Err(StreamDecodeError::ExpansionRatio);
        }
        *produced = total;
        Ok(())
    }
}

/// Run raw or zlib-wrapped DEFLATE over `input`. Returns the bytes consumed
/// and whether the stream ended; without an end, unconsumed input means the
/// decoder cannot proceed.
fn inflate(
    inflater: &mut flate2::Decompress,
    input: &[u8],
    block: &mut [u8],
    emit: &mut Emit<'_>,
) -> Result<(usize, bool), StreamDecodeError> {
    let mut used = 0;
    loop {
        let before = (inflater.total_in(), inflater.total_out());
        let status = inflater
            .decompress(&input[used..], block, flate2::FlushDecompress::None)
            .map_err(|_| StreamDecodeError::Corrupt)?;
        let consumed = (inflater.total_in() - before.0) as usize;
        let produced = (inflater.total_out() - before.1) as usize;
        used += consumed;
        if produced > 0 {
            emit(&block[..produced])?;
        }
        if status == flate2::Status::StreamEnd {
            return Ok((used, true));
        }
        // Stop once the input is spent and the output block was not filled
        // (nothing more is buffered), or when no progress is possible.
        let drained = used == input.len() && produced < block.len();
        if drained || (consumed == 0 && produced == 0) {
            return Ok((used, false));
        }
    }
}

struct ZstdStage {
    decoder: zstd::stream::raw::Decoder<'static>,
    /// The last frame is complete and flushed. False until a frame is, so an
    /// empty body is truncated, as in buffered decoding.
    frame_done: bool,
}

impl ZstdStage {
    fn new() -> Result<Self, StreamDecodeError> {
        let mut decoder =
            zstd::stream::raw::Decoder::new().map_err(|_| StreamDecodeError::Corrupt)?;
        decoder
            .set_parameter(zstd::stream::raw::DParameter::WindowLogMax(
                ZSTD_WINDOW_LOG_MAX,
            ))
            .map_err(|_| StreamDecodeError::Corrupt)?;
        Ok(Self {
            decoder,
            frame_done: false,
        })
    }

    fn push(
        &mut self,
        input: &[u8],
        block: &mut [u8],
        emit: &mut Emit<'_>,
    ) -> Result<(), StreamDecodeError> {
        use zstd::stream::raw::{InBuffer, Operation, OutBuffer};
        let mut source = InBuffer::around(input);
        // Output may remain inside the decoder after the input is consumed.
        let mut buffered = false;
        while source.pos < input.len() || buffered {
            if self.frame_done {
                // Concatenated frames decode as one body.
                self.decoder
                    .reinit()
                    .map_err(|_| StreamDecodeError::Corrupt)?;
                self.frame_done = false;
            }
            let before = source.pos;
            let (written, hint) = {
                let mut target = OutBuffer::around(&mut *block);
                let hint = self
                    .decoder
                    .run(&mut source, &mut target)
                    .map_err(|_| StreamDecodeError::Corrupt)?;
                (target.pos(), hint)
            };
            self.frame_done = hint == 0;
            buffered = !self.frame_done && written == block.len();
            if written > 0 {
                emit(&block[..written])?;
            } else if source.pos == before && source.pos < input.len() {
                return Err(StreamDecodeError::Corrupt);
            }
        }
        Ok(())
    }

    fn finish(&self) -> Result<(), StreamDecodeError> {
        if self.frame_done {
            Ok(())
        } else {
            Err(StreamDecodeError::Truncated)
        }
    }
}

/// HTTP `deflate`: one zlib stream, nothing after it.
struct ZlibStage {
    inflater: flate2::Decompress,
    done: bool,
}

impl ZlibStage {
    fn push(
        &mut self,
        input: &[u8],
        block: &mut [u8],
        emit: &mut Emit<'_>,
    ) -> Result<(), StreamDecodeError> {
        if input.is_empty() {
            return Ok(());
        }
        if self.done {
            return Err(StreamDecodeError::Corrupt);
        }
        let (used, end) = inflate(&mut self.inflater, input, block, emit)?;
        self.done = end;
        if used < input.len() {
            return Err(StreamDecodeError::Corrupt);
        }
        Ok(())
    }
}

const GZ_FHCRC: u8 = 1 << 1;
const GZ_FEXTRA: u8 = 1 << 2;
const GZ_FNAME: u8 = 1 << 3;
const GZ_FCOMMENT: u8 = 1 << 4;
const GZ_RESERVED: u8 = 0b1110_0000;
/// Longest file name or comment `flate2` accepts in a gzip header.
const GZ_FIELD_MAX: usize = 65535;

#[derive(Clone, Copy, PartialEq, Eq)]
enum GzField {
    Fixed,
    ExtraLen,
    Extra,
    Name,
    Comment,
    Crc,
    Done,
}

/// Incremental RFC 1952 member header, accepting what
/// `flate2::read::MultiGzDecoder` accepts.
struct GzHeader {
    field: GzField,
    /// Bytes of this header consumed.
    seen: usize,
    flags: u8,
    fixed: [u8; 10],
    pair: [u8; 2],
    /// Bytes read of the current fixed-size field, or its remaining length
    /// (extra field), or its length so far (name, comment).
    count: usize,
    crc: Option<flate2::Crc>,
}

impl GzHeader {
    fn new() -> Self {
        Self {
            field: GzField::Fixed,
            seen: 0,
            flags: 0,
            fixed: [0; 10],
            pair: [0; 2],
            count: 0,
            crc: None,
        }
    }

    /// Move to `field`, skipping the optional fields this header lacks.
    fn enter(&mut self, mut field: GzField) {
        loop {
            let present = match field {
                GzField::ExtraLen => self.flags & GZ_FEXTRA != 0,
                GzField::Name => self.flags & GZ_FNAME != 0,
                GzField::Comment => self.flags & GZ_FCOMMENT != 0,
                GzField::Crc => self.flags & GZ_FHCRC != 0,
                GzField::Fixed | GzField::Extra | GzField::Done => true,
            };
            if present {
                break;
            }
            field = match field {
                GzField::ExtraLen => GzField::Name,
                GzField::Name => GzField::Comment,
                GzField::Comment => GzField::Crc,
                _ => GzField::Done,
            };
        }
        self.field = field;
        self.count = 0;
    }

    /// Consume one header byte. Returns true when the header is complete.
    fn byte(&mut self, byte: u8) -> Result<bool, StreamDecodeError> {
        self.seen += 1;
        if !matches!(self.field, GzField::Fixed | GzField::Crc) {
            if let Some(crc) = self.crc.as_mut() {
                crc.update(&[byte]);
            }
        }
        match self.field {
            GzField::Fixed => {
                let valid = match self.count {
                    0 => byte == 0x1f,
                    1 => byte == 0x8b,
                    // Compression method: DEFLATE.
                    2 => byte == 8,
                    3 => byte & GZ_RESERVED == 0,
                    _ => true,
                };
                if !valid {
                    return Err(StreamDecodeError::Corrupt);
                }
                self.fixed[self.count] = byte;
                self.count += 1;
                if self.count == self.fixed.len() {
                    self.flags = self.fixed[3];
                    if self.flags & GZ_FHCRC != 0 {
                        let mut crc = flate2::Crc::new();
                        crc.update(&self.fixed);
                        self.crc = Some(crc);
                    }
                    self.enter(GzField::ExtraLen);
                }
            }
            GzField::ExtraLen => {
                self.pair[self.count] = byte;
                self.count += 1;
                if self.count == 2 {
                    let len = usize::from(u16::from_le_bytes(self.pair));
                    if len == 0 {
                        self.enter(GzField::Name);
                    } else {
                        self.enter(GzField::Extra);
                        self.count = len;
                    }
                }
            }
            GzField::Extra => {
                self.count -= 1;
                if self.count == 0 {
                    self.enter(GzField::Name);
                }
            }
            GzField::Name | GzField::Comment => {
                if byte == 0 {
                    self.enter(if self.field == GzField::Name {
                        GzField::Comment
                    } else {
                        GzField::Crc
                    });
                } else {
                    self.count += 1;
                    if self.count > GZ_FIELD_MAX {
                        return Err(StreamDecodeError::Corrupt);
                    }
                }
            }
            GzField::Crc => {
                self.pair[self.count] = byte;
                self.count += 1;
                if self.count == 2 {
                    let computed = self.crc.as_ref().map_or(0, flate2::Crc::sum) as u16;
                    if u16::from_le_bytes(self.pair) != computed {
                        return Err(StreamDecodeError::Corrupt);
                    }
                    self.field = GzField::Done;
                }
            }
            GzField::Done => {}
        }
        Ok(self.field == GzField::Done)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GzState {
    Header,
    Body,
    Trailer,
}

/// Gzip with any number of members, each checked against its CRC and length.
struct GzipStage {
    state: GzState,
    header: GzHeader,
    inflater: flate2::Decompress,
    crc: flate2::Crc,
    trailer: [u8; 8],
    trailer_len: usize,
    members: u64,
}

impl GzipStage {
    fn new() -> Self {
        Self {
            state: GzState::Header,
            header: GzHeader::new(),
            inflater: flate2::Decompress::new(false),
            crc: flate2::Crc::new(),
            trailer: [0; 8],
            trailer_len: 0,
            members: 0,
        }
    }

    fn push(
        &mut self,
        mut input: &[u8],
        block: &mut [u8],
        emit: &mut Emit<'_>,
    ) -> Result<(), StreamDecodeError> {
        while !input.is_empty() {
            match self.state {
                GzState::Header => {
                    let mut used = 0;
                    for &byte in input {
                        used += 1;
                        if self.header.byte(byte)? {
                            self.state = GzState::Body;
                            self.inflater.reset(false);
                            self.crc.reset();
                            break;
                        }
                    }
                    input = &input[used..];
                }
                GzState::Body => {
                    let crc = &mut self.crc;
                    let (used, end) = inflate(&mut self.inflater, input, block, &mut |decoded| {
                        crc.update(decoded);
                        emit(decoded)
                    })?;
                    input = &input[used..];
                    if end {
                        self.state = GzState::Trailer;
                        self.trailer_len = 0;
                    } else if !input.is_empty() {
                        return Err(StreamDecodeError::Corrupt);
                    }
                }
                GzState::Trailer => {
                    let take = (self.trailer.len() - self.trailer_len).min(input.len());
                    self.trailer[self.trailer_len..self.trailer_len + take]
                        .copy_from_slice(&input[..take]);
                    self.trailer_len += take;
                    input = &input[take..];
                    if self.trailer_len == self.trailer.len() {
                        let word = |at: usize| {
                            u32::from_le_bytes([
                                self.trailer[at],
                                self.trailer[at + 1],
                                self.trailer[at + 2],
                                self.trailer[at + 3],
                            ])
                        };
                        if word(0) != self.crc.sum() || word(4) != self.crc.amount() {
                            return Err(StreamDecodeError::Corrupt);
                        }
                        self.members += 1;
                        // Any further bytes must be another member.
                        self.header = GzHeader::new();
                        self.state = GzState::Header;
                    }
                }
            }
        }
        Ok(())
    }

    fn finish(&self) -> Result<(), StreamDecodeError> {
        if self.state == GzState::Header && self.header.seen == 0 && self.members > 0 {
            Ok(())
        } else {
            Err(StreamDecodeError::Truncated)
        }
    }
}

type BrotliAlloc = brotli::enc::StandardAlloc;

struct BrotliStage {
    state: brotli::BrotliState<BrotliAlloc, BrotliAlloc, BrotliAlloc>,
    total: usize,
    done: bool,
}

impl BrotliStage {
    fn new() -> Self {
        Self {
            state: brotli::BrotliState::new(
                BrotliAlloc::default(),
                BrotliAlloc::default(),
                BrotliAlloc::default(),
            ),
            total: 0,
            done: false,
        }
    }

    fn push(
        &mut self,
        input: &[u8],
        block: &mut [u8],
        emit: &mut Emit<'_>,
    ) -> Result<(), StreamDecodeError> {
        // Buffered decoding stops reading at the end of the brotli stream and
        // never looks at what follows; the same bytes are ignored here.
        if self.done {
            return Ok(());
        }
        let mut available = input.len();
        let mut offset = 0;
        loop {
            let mut room = block.len();
            let mut written = 0;
            let result = brotli::BrotliDecompressStream(
                &mut available,
                &mut offset,
                input,
                &mut room,
                &mut written,
                block,
                &mut self.total,
                &mut self.state,
            );
            if written > 0 {
                emit(&block[..written])?;
            }
            match result {
                brotli::BrotliResult::ResultSuccess => {
                    self.done = true;
                    return Ok(());
                }
                brotli::BrotliResult::NeedsMoreInput => return Ok(()),
                brotli::BrotliResult::NeedsMoreOutput => {}
                brotli::BrotliResult::ResultFailure => return Err(StreamDecodeError::Corrupt),
            }
        }
    }
}

enum Codec {
    Zstd(ZstdStage),
    Gzip(Box<GzipStage>),
    Zlib(ZlibStage),
    Brotli(Box<BrotliStage>),
}

impl Codec {
    fn push(
        &mut self,
        input: &[u8],
        block: &mut [u8],
        emit: &mut Emit<'_>,
    ) -> Result<(), StreamDecodeError> {
        match self {
            Self::Zstd(stage) => stage.push(input, block, emit),
            Self::Gzip(stage) => stage.push(input, block, emit),
            Self::Zlib(stage) => stage.push(input, block, emit),
            Self::Brotli(stage) => stage.push(input, block, emit),
        }
    }

    /// Every stage emits all it can while pushing, so the end of input only
    /// has to be checked against the end of the encoded stream.
    fn finish(&self) -> Result<(), StreamDecodeError> {
        let complete = match self {
            Self::Zstd(stage) => return stage.finish(),
            Self::Gzip(stage) => return stage.finish(),
            Self::Zlib(stage) => stage.done,
            Self::Brotli(stage) => stage.done,
        };
        if complete {
            Ok(())
        } else {
            Err(StreamDecodeError::Truncated)
        }
    }
}

struct Stage {
    codec: Codec,
    /// Bytes this stage has produced.
    produced: u64,
    block: Box<[u8]>,
}

/// Push `input` through `stages` in order and the result into `sink`.
fn feed(
    stages: &mut [Stage],
    meter: Meter,
    input: &[u8],
    sink: &mut Emit<'_>,
) -> Result<(), StreamDecodeError> {
    let Some((stage, rest)) = stages.split_first_mut() else {
        return if input.is_empty() {
            Ok(())
        } else {
            sink(input)
        };
    };
    let Stage {
        codec,
        produced,
        block,
    } = stage;
    codec.push(input, block, &mut |decoded| {
        meter.charge(produced, decoded.len())?;
        feed(rest, meter, decoded, sink)
    })
}

/// Incremental counterpart of [`inspect`]'s decoding: push encoded chunks as
/// they arrive, receive decoded bytes, then [`finish`](Self::finish).
///
/// It accepts the same Content-Encoding headers (zstd with a window of at
/// most 16 MiB, multi-member gzip, zlib-wrapped deflate, brotli, identity,
/// stacks of up to four) and rejects the same bodies. Two things differ by
/// nature: output delivered before an error cannot be taken back, and the
/// limits are [`DecodeLimits`] rather than the buffered path's fixed cap.
///
/// Decoded bytes reach the sink in pieces of at most [`DECODED_BLOCK`] bytes
/// (identity bodies pass through in the pieces they arrive in), so a small
/// chunk that expands enormously never materializes; the sink can stop the
/// decoder by returning [`ControlFlow::Break`].
///
/// Decoding is CPU-bound and synchronous. A caller on an async executor
/// should move large chunks to a blocking thread; the decoder is `Send`.
///
/// After an error the decoder is spent: every later call returns that error.
pub struct StreamDecoder {
    /// Decoders in the order they run: the last encoding applied first.
    stages: Vec<Stage>,
    limits: DecodeLimits,
    encoded: u64,
    failed: Option<StreamDecodeError>,
}

impl std::fmt::Debug for StreamDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamDecoder")
            .field("stages", &self.stages.len())
            .field("limits", &self.limits)
            .field("encoded", &self.encoded)
            .field("decoded", &self.decoded_bytes())
            .field("failed", &self.failed)
            .finish()
    }
}

impl StreamDecoder {
    /// A decoder for these request headers with the default limits.
    pub fn new(headers: &HeaderMap) -> Result<Self, StreamDecodeError> {
        Self::with_limits(headers, DecodeLimits::default())
    }

    pub fn with_limits(
        headers: &HeaderMap,
        limits: DecodeLimits,
    ) -> Result<Self, StreamDecodeError> {
        let codings = codings(headers).map_err(|_| StreamDecodeError::Unsupported)?;
        let mut stages = Vec::new();
        for coding in codings.iter().rev() {
            let codec = match coding {
                Coding::Identity => continue,
                Coding::Zstd => Codec::Zstd(ZstdStage::new()?),
                Coding::Gzip => Codec::Gzip(Box::new(GzipStage::new())),
                Coding::Deflate => Codec::Zlib(ZlibStage {
                    inflater: flate2::Decompress::new(true),
                    done: false,
                }),
                Coding::Brotli => Codec::Brotli(Box::new(BrotliStage::new())),
            };
            stages.push(Stage {
                codec,
                produced: 0,
                block: vec![0; DECODED_BLOCK].into_boxed_slice(),
            });
        }
        Ok(Self {
            stages,
            limits,
            encoded: 0,
            failed: None,
        })
    }

    /// The body is not encoded: decoded bytes are the bytes pushed.
    pub fn is_identity(&self) -> bool {
        self.stages.is_empty()
    }

    /// Encoded bytes pushed so far.
    pub fn encoded_bytes(&self) -> u64 {
        self.encoded
    }

    /// Decoded bytes delivered to sinks so far.
    pub fn decoded_bytes(&self) -> u64 {
        self.stages
            .last()
            .map_or(self.encoded, |stage| stage.produced)
    }

    /// Decode the next chunk, calling `sink` with each piece of decoded data
    /// in order. The slice is only valid during the call.
    pub fn push(
        &mut self,
        chunk: &[u8],
        mut sink: impl FnMut(&[u8]) -> ControlFlow<()>,
    ) -> Result<(), StreamDecodeError> {
        if let Some(error) = self.failed {
            return Err(error);
        }
        let result = self.push_inner(chunk, &mut |decoded| match sink(decoded) {
            ControlFlow::Continue(()) => Ok(()),
            ControlFlow::Break(()) => Err(StreamDecodeError::Aborted),
        });
        if let Err(error) = result {
            self.failed = Some(error);
        }
        result
    }

    fn push_inner(&mut self, chunk: &[u8], sink: &mut Emit<'_>) -> Result<(), StreamDecodeError> {
        let encoded = self.encoded + chunk.len() as u64;
        if encoded > self.limits.max_decoded {
            return Err(StreamDecodeError::BudgetExceeded);
        }
        self.encoded = encoded;
        let meter = Meter {
            limits: self.limits,
            encoded,
        };
        feed(&mut self.stages, meter, chunk, sink)
    }

    /// Declare the end of the body. Fails with
    /// [`Truncated`](StreamDecodeError::Truncated) when an encoded stream is
    /// incomplete. Returns the total of decoded bytes.
    pub fn finish(self) -> Result<u64, StreamDecodeError> {
        if let Some(error) = self.failed {
            return Err(error);
        }
        for stage in &self.stages {
            stage.codec.finish()?;
        }
        Ok(self.decoded_bytes())
    }
}

/// Decode known text inference paths off the async executor. Other endpoints
/// (auth, telemetry and binary uploads) retain their original representation.
pub(crate) async fn inspect(
    path: &str,
    headers: &HeaderMap,
    bytes: Bytes,
) -> Result<Bytes, DecodeError> {
    if crate::wire::detect(path).is_none() || !headers.contains_key(http::header::CONTENT_ENCODING)
    {
        return Ok(bytes);
    }
    let headers = headers.clone();
    tokio::task::spawn_blocking(move || decode(&headers, bytes, LIMIT))
        .await
        .map_err(|_| DecodeError::Invalid)?
}

/// Encoded-body checksums cannot describe the rewritten JSON body.
pub(crate) fn rewritten(headers: &mut HeaderMap) {
    for name in [
        "content-encoding",
        "content-length",
        "transfer-encoding",
        "trailer",
        "content-md5",
        "digest",
        "content-digest",
        "repr-digest",
    ] {
        headers.remove(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn encoded(encoding: &str, body: &[u8]) -> Vec<u8> {
        match encoding {
            "identity" => body.to_vec(),
            "zstd" => zstd::stream::encode_all(body, 3).unwrap(),
            "gzip" => {
                let mut w =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                w.write_all(body).unwrap();
                w.finish().unwrap()
            }
            "deflate" => {
                let mut w =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                w.write_all(body).unwrap();
                w.finish().unwrap()
            }
            "br" => {
                let mut w = brotli::CompressorWriter::new(Vec::new(), 8192, 4, 22);
                w.write_all(body).unwrap();
                w.into_inner()
            }
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn compressed_codex_requests_above_sixteen_mib_are_decoded() {
        let text = "x".repeat(20 * 1024 * 1024);
        let body = format!(r#"{{"model":"m","input":"{text}"}}"#);
        let mut headers = HeaderMap::new();
        headers.insert(http::header::CONTENT_ENCODING, "zstd".parse().unwrap());
        let decoded = inspect(
            "/backend-api/codex/responses",
            &headers,
            Bytes::from(encoded("zstd", body.as_bytes())),
        )
        .await
        .unwrap();
        assert_eq!(decoded.len(), body.len());
    }

    #[test]
    fn decodes_each_coding_and_rejects_corruption_and_expansion() {
        let body = br#"{"model":"codex","input":"synthetic secret"}"#;
        for encoding in ["identity", "zstd", "gzip", "deflate", "br"] {
            let mut headers = HeaderMap::new();
            headers.insert(
                "content-encoding",
                encoding.to_ascii_uppercase().parse().unwrap(),
            );
            let bytes = encoded(encoding, body);
            assert_eq!(decode(&headers, bytes.into(), 1024).unwrap().as_ref(), body);
            let bomb = encoded(encoding, &[b'x'; 2048]);
            assert_eq!(
                decode(&headers, bomb.into(), 1024),
                Err(DecodeError::TooLarge)
            );
            if encoding != "identity" {
                assert_eq!(
                    decode(&headers, Bytes::from_static(b"corrupt"), 1024),
                    Err(DecodeError::Invalid)
                );
                let mut truncated = encoded(encoding, body);
                truncated.pop();
                assert!(
                    decode(&headers, truncated.into(), 1024).is_err(),
                    "{encoding} accepted truncated data"
                );
            }
        }
    }

    #[test]
    fn buffered_deflate_decodes_bodies_larger_than_its_output_buffer() {
        let headers = headers("deflate");
        let body = vec![b'x'; 300 * 1024];
        let decoded = decode(&headers, encoded("deflate", &body).into(), 1024 * 1024).unwrap();
        assert!(decoded.as_ref() == body.as_slice());
        assert_eq!(
            decode(&headers, encoded("deflate", &body).into(), 64 * 1024),
            Err(DecodeError::TooLarge)
        );
        let mut truncated = encoded("deflate", &body);
        truncated.pop();
        assert_eq!(
            decode(&headers, truncated.into(), 1024 * 1024),
            Err(DecodeError::Invalid)
        );
    }

    #[test]
    fn repeated_headers_decode_in_reverse_order_and_enforce_chain_limits() {
        let body = b"synthetic";
        let mut headers = HeaderMap::new();
        headers.append("content-encoding", "GZip".parse().unwrap());
        headers.append("content-encoding", "identity, zstd".parse().unwrap());
        let bytes = encoded("zstd", &encoded("gzip", body));
        assert_eq!(decode(&headers, bytes.into(), 1024).unwrap().as_ref(), body);
        for value in [
            "unknown",
            "",
            "gzip,",
            "identity,identity,identity,identity,identity",
        ] {
            headers.insert("content-encoding", value.parse().unwrap());
            assert_eq!(
                decode(&headers, Bytes::new(), 1024),
                Err(DecodeError::Unsupported)
            );
        }
    }

    #[test]
    fn gzip_members_are_all_inspected_and_rewriting_removes_stale_representation_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("content-encoding", "gzip".parse().unwrap());
        let mut body = encoded("gzip", b"one");
        body.extend(encoded("gzip", b"two"));
        assert_eq!(decode(&headers, body.into(), 1024).unwrap(), "onetwo");
        for name in [
            "content-length",
            "transfer-encoding",
            "trailer",
            "content-md5",
            "digest",
            "content-digest",
            "repr-digest",
        ] {
            headers.insert(name, "stale".parse().unwrap());
        }
        headers.insert("content-type", "application/json".parse().unwrap());
        headers.insert("authorization", "synthetic".parse().unwrap());
        rewritten(&mut headers);
        assert_eq!(headers.len(), 2);
        assert!(headers.contains_key("authorization"));
    }

    #[tokio::test]
    async fn non_inference_paths_are_opaque() {
        let mut headers = HeaderMap::new();
        headers.insert("content-encoding", "unsupported".parse().unwrap());
        assert_eq!(
            inspect("/oauth/token", &headers, Bytes::from_static(b"opaque"))
                .await
                .unwrap(),
            "opaque"
        );
    }

    // -----------------------------------------------------------------------
    // StreamDecoder
    // -----------------------------------------------------------------------

    const CODINGS: [&str; 4] = ["zstd", "gzip", "deflate", "br"];

    fn headers(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("content-encoding", value.parse().unwrap());
        headers
    }

    /// Apply a Content-Encoding list the way a client would: left to right.
    fn encoded_stack(stack: &str, body: &[u8]) -> Vec<u8> {
        stack.split(',').fold(body.to_vec(), |data, coding| {
            encoded(coding.trim().to_ascii_lowercase().as_str(), &data)
        })
    }

    /// Limits equal to the buffered path's: a byte cap and nothing else.
    fn cap(limit: u64) -> DecodeLimits {
        DecodeLimits {
            max_decoded: limit,
            max_ratio: u64::MAX,
            ratio_floor: u64::MAX,
        }
    }

    /// Decode `body` pushed in pieces of `chunk` bytes. Returns the output
    /// delivered and the outcome.
    fn stream(
        headers: &HeaderMap,
        limits: DecodeLimits,
        body: &[u8],
        chunk: usize,
    ) -> (Vec<u8>, Result<u64, StreamDecodeError>) {
        let mut output = Vec::new();
        let mut decoder = match StreamDecoder::with_limits(headers, limits) {
            Ok(decoder) => decoder,
            Err(error) => return (output, Err(error)),
        };
        let identity = decoder.is_identity();
        let mut pushed = 0;
        for piece in body.chunks(chunk) {
            let result = decoder.push(piece, |decoded| {
                // A bomb arrives in bounded pieces, never as one allocation.
                assert!(!decoded.is_empty());
                assert!(identity || decoded.len() <= DECODED_BLOCK);
                output.extend_from_slice(decoded);
                ControlFlow::Continue(())
            });
            if let Err(error) = result {
                // The decoder is spent.
                assert_eq!(decoder.push(b"x", |_| panic!("spent")), Err(error));
                assert_eq!(decoder.finish(), Err(error));
                return (output, Err(error));
            }
            pushed += piece.len() as u64;
            assert_eq!(decoder.encoded_bytes(), pushed);
            assert_eq!(decoder.decoded_bytes(), output.len() as u64);
        }
        let result = decoder.finish();
        if let Ok(total) = result {
            assert_eq!(total, output.len() as u64);
        }
        (output, result)
    }

    /// Streaming agrees with buffered decoding on the outcome, and on every
    /// byte when both succeed.
    fn assert_parity(stack: &str, body: &[u8], limit: usize, chunks: &[usize]) {
        let headers = headers(stack);
        let batch = decode(&headers, Bytes::copy_from_slice(body), limit);
        for &chunk in chunks {
            let (output, result) = stream(&headers, cap(limit as u64), body, chunk);
            match (&batch, result) {
                (Ok(expected), Ok(_)) => {
                    assert!(
                        expected.as_ref() == output.as_slice(),
                        "{stack} in {chunk}s"
                    )
                }
                (Err(expected), Err(error)) => {
                    assert_eq!(error.as_batch(), Some(*expected), "{stack} in {chunk}s")
                }
                (batch, streamed) => panic!(
                    "{stack} in {chunk}s: buffered {:?}, streamed {streamed:?}",
                    batch.as_ref().map(Bytes::len)
                ),
            }
            // Output delivered before an error is a prefix of the real body.
            assert!(output.len() <= limit);
        }
    }

    /// Text that compresses, but not to nothing.
    fn sample(len: usize) -> Vec<u8> {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut out = Vec::with_capacity(len + 64);
        out.extend_from_slice(br#"{"model":"m","input":[""#);
        while out.len() < len {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            let word = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
            out.extend_from_slice(match word % 7 {
                0 => b"the quick brown fox ",
                1 => b"\",\"",
                2 => b"synthetic secret ",
                _ => b"",
            });
            out.extend_from_slice(format!("{:x}", word >> 40).as_bytes());
        }
        out.truncate(len);
        out
    }

    #[test]
    fn stream_decoder_matches_buffered_decoding_for_every_coding_and_stack() {
        let bodies = [
            Vec::new(),
            b"x".to_vec(),
            br#"{"model":"codex","input":"synthetic secret"}"#.to_vec(),
            sample(300 * 1024),
            // Far more output than input, several blocks per chunk.
            vec![b'x'; 1024 * 1024],
        ];
        let stacks = [
            "identity",
            "zstd",
            "gzip",
            "deflate",
            "br",
            "GZip, ZSTD",
            "zstd, gzip",
            "br, deflate",
            "deflate, br, gzip",
            "gzip, identity, zstd",
            "gzip, br, zstd, deflate",
            "zstd, zstd, zstd, zstd",
            "identity, identity",
        ];
        for body in &bodies {
            for stack in stacks {
                let wire = encoded_stack(stack, body);
                // Every-byte fragmentation is quadratic in stages; keep it to
                // encoded bodies it finishes quickly on.
                let chunks: &[usize] = if wire.len() <= 64 * 1024 {
                    &[1, 2, 3, 7, 100, 4096, 65_536, usize::MAX]
                } else {
                    &[7, 4096, 65_537, usize::MAX]
                };
                assert_parity(stack, &wire, 4 * 1024 * 1024, chunks);
                let headers = headers(stack);
                let (output, result) = stream(&headers, DecodeLimits::default(), &wire, 1000);
                assert_eq!(result, Ok(body.len() as u64), "{stack}");
                assert!(&output == body, "{stack}");
            }
        }
    }

    #[test]
    fn stream_decoder_accepts_the_same_headers() {
        let mut repeated = HeaderMap::new();
        repeated.append("content-encoding", "GZip".parse().unwrap());
        repeated.append("content-encoding", "identity, zstd".parse().unwrap());
        let wire = encoded("zstd", &encoded("gzip", b"synthetic"));
        let (output, result) = stream(&repeated, DecodeLimits::default(), &wire, 5);
        assert_eq!((output.as_slice(), result), (&b"synthetic"[..], Ok(9)));

        for value in [
            "unknown",
            "",
            "gzip,",
            "x-gzip",
            "identity,identity,identity,identity,identity",
            "gzip, gzip, gzip, gzip, gzip",
        ] {
            let headers = headers(value);
            assert_eq!(
                decode(&headers, Bytes::new(), 1024),
                Err(DecodeError::Unsupported)
            );
            let error = StreamDecoder::new(&headers).err();
            assert_eq!(error, Some(StreamDecodeError::Unsupported), "{value:?}");
        }
        let mut opaque = HeaderMap::new();
        opaque.insert(
            "content-encoding",
            http::HeaderValue::from_bytes(b"gz\xffip").unwrap(),
        );
        assert_eq!(
            StreamDecoder::new(&opaque).err(),
            Some(StreamDecodeError::Unsupported)
        );

        // No header at all, or only identity: bytes pass through as pushed.
        for headers in [
            HeaderMap::new(),
            headers("identity"),
            headers(" Identity ,identity"),
        ] {
            let mut decoder = StreamDecoder::new(&headers).unwrap();
            assert!(decoder.is_identity());
            let mut pieces = Vec::new();
            for piece in [&b"ab"[..], b"", b"cde"] {
                decoder
                    .push(piece, |decoded| {
                        pieces.push(decoded.to_vec());
                        ControlFlow::Continue(())
                    })
                    .unwrap();
            }
            assert_eq!(pieces, [b"ab".to_vec(), b"cde".to_vec()]);
            assert_eq!(decoder.finish(), Ok(5));
        }
        assert!(!StreamDecoder::new(&headers("br")).unwrap().is_identity());
        // An empty identity body is complete; an empty encoded one is not.
        assert_eq!(
            StreamDecoder::new(&HeaderMap::new()).unwrap().finish(),
            Ok(0)
        );
        for coding in CODINGS {
            let headers = headers(coding);
            assert_eq!(
                StreamDecoder::new(&headers).unwrap().finish(),
                Err(StreamDecodeError::Truncated),
                "{coding}"
            );
            assert_eq!(
                decode(&headers, Bytes::new(), 1024),
                Err(DecodeError::Invalid)
            );
        }
        fn assert_send<T: Send>() {}
        assert_send::<StreamDecoder>();
    }

    #[test]
    fn truncated_streams_are_reported_as_truncated() {
        let body = sample(700);
        for coding in CODINGS {
            let wire = encoded(coding, &body);
            let headers = headers(coding);
            for len in 0..wire.len() {
                assert_parity(coding, &wire[..len], 4096, &[1, 5, usize::MAX]);
                for chunk in [1, 5, usize::MAX] {
                    let (output, result) = stream(&headers, cap(4096), &wire[..len], chunk);
                    assert_eq!(
                        result,
                        Err(StreamDecodeError::Truncated),
                        "{coding} cut at {len} of {}",
                        wire.len()
                    );
                    assert!(body.starts_with(&output));
                }
            }
        }
        // A stack is truncated wherever its outermost stream is cut.
        let wire = encoded_stack("gzip, br, zstd", &body);
        for len in (0..wire.len()).step_by(7) {
            assert_parity("gzip, br, zstd", &wire[..len], 4096, &[3, usize::MAX]);
            assert_eq!(
                stream(&headers("gzip, br, zstd"), cap(4096), &wire[..len], 3).1,
                Err(StreamDecodeError::Truncated)
            );
        }
        // Cutting an inner stream of a stack is truncation too.
        for (stack, inner) in [("zstd, gzip", "zstd"), ("deflate, br", "deflate")] {
            let mut inner_wire = encoded(inner, &body);
            inner_wire.truncate(inner_wire.len() - 3);
            let outer = stack.rsplit(", ").next().unwrap();
            let wire = encoded(outer, &inner_wire);
            assert_parity(stack, &wire, 4096, &[1, usize::MAX]);
            assert_eq!(
                stream(&headers(stack), cap(4096), &wire, 4).1,
                Err(StreamDecodeError::Truncated)
            );
        }
        // Multi-member gzip cut between members is complete; inside the next
        // header or trailer it is truncated.
        let mut members = encoded("gzip", b"one");
        let first = members.len();
        members.extend(encoded("gzip", b"two"));
        let gzip = headers("gzip");
        assert_eq!(stream(&gzip, cap(64), &members[..first], 1).1, Ok(3));
        for len in [first - 1, first + 1, first + 9, members.len() - 1] {
            assert_eq!(
                stream(&gzip, cap(64), &members[..len], 1).1,
                Err(StreamDecodeError::Truncated)
            );
        }
    }

    #[test]
    fn corrupt_streams_fail_like_buffered_decoding() {
        let body = sample(400);
        for coding in CODINGS {
            let wire = encoded(coding, &body);
            let headers = headers(coding);
            // Damage every byte in turn. Buffered decoding accepts a few of
            // these (unused gzip header bytes, for one); so must streaming.
            let mut corrupt = 0;
            for at in 0..wire.len() {
                for flip in [0x01, 0x80, 0xff] {
                    let mut damaged = wire.clone();
                    damaged[at] ^= flip;
                    assert_parity(coding, &damaged, 8192, &[1, usize::MAX]);
                    let result = stream(&headers, cap(8192), &damaged, 3).1;
                    corrupt += usize::from(result == Err(StreamDecodeError::Corrupt));
                }
            }
            assert!(corrupt > 10, "{coding}: {corrupt}");

            // Bytes that were never this encoding. (A run of 0xff is a valid
            // empty brotli stream, so only agreement is asserted for all.)
            for garbage in [&b"corrupt"[..], &[0xff; 64], &[0; 64], body.as_slice()] {
                assert_parity(coding, garbage, 8192, &[1, 3, usize::MAX]);
            }
            if coding != "br" {
                assert_eq!(
                    stream(&headers, cap(8192), &[0xff; 64], 3).1,
                    Err(StreamDecodeError::Corrupt),
                    "{coding}"
                );
            }

            // Bytes after the end of the stream.
            let mut trailing = wire.clone();
            trailing.extend_from_slice(b"trailing garbage");
            assert_parity(coding, &trailing, 8192, &[1, 3, usize::MAX]);
            let result = stream(&headers, cap(8192), &trailing, 3).1;
            if coding == "br" {
                // Buffered brotli decoding stops reading at the end of the
                // stream; streaming ignores the same bytes.
                assert_eq!(result, Ok(body.len() as u64));
            } else {
                assert_eq!(result, Err(StreamDecodeError::Corrupt), "{coding}");
            }
        }
        // A damaged inner stream of a stack.
        let mut inner = encoded("deflate", &body);
        let middle = inner.len() / 2;
        inner[middle] ^= 0x55;
        let wire = encoded("zstd", &inner);
        assert_parity("deflate, zstd", &wire, 8192, &[1, usize::MAX]);
        assert_eq!(
            stream(&headers("deflate, zstd"), cap(8192), &wire, 2).1,
            Err(StreamDecodeError::Corrupt)
        );
    }

    #[test]
    fn gzip_members_and_optional_header_fields_decode_like_buffered() {
        let builder = || {
            flate2::GzBuilder::new()
                .filename("body.json")
                .comment("synthetic")
                .extra(vec![1, 2, 3, 0, 5])
        };
        let mut full = builder().write(Vec::new(), flate2::Compression::default());
        full.write_all(b"alpha").unwrap();
        let full = full.finish().unwrap();

        // The same member with a header checksum (FHCRC), right and wrong.
        let plain = encoded("gzip", b"beta");
        let with_crc = |delta: u16| {
            let mut member = plain[..10].to_vec();
            member[3] |= GZ_FHCRC;
            let mut crc = flate2::Crc::new();
            crc.update(&member);
            member.extend_from_slice(&(crc.sum() as u16).wrapping_add(delta).to_le_bytes());
            member.extend_from_slice(&plain[10..]);
            member
        };
        let mut every_field = full.clone();
        every_field[3] |= GZ_FHCRC;
        let header_len = full.len() - encoded("gzip", b"alpha").len() + 10;
        let mut crc = flate2::Crc::new();
        crc.update(&every_field[..header_len]);
        let sum = (crc.sum() as u16).to_le_bytes();
        every_field.insert(header_len, sum[0]);
        every_field.insert(header_len + 1, sum[1]);

        let gzip = headers("gzip");
        let chunks = [1, 2, 9, usize::MAX];
        for (wire, expected) in [
            (full.clone(), Some(&b"alpha"[..])),
            (with_crc(0), Some(&b"beta"[..])),
            (with_crc(1), None),
            (every_field.clone(), Some(&b"alpha"[..])),
            (
                [
                    full.clone(),
                    with_crc(0),
                    plain.clone(),
                    every_field.clone(),
                ]
                .concat(),
                Some(&b"alphabetabetaalpha"[..]),
            ),
            (encoded("gzip", b""), Some(&b""[..])),
            (
                [encoded("gzip", b""), encoded("gzip", b"")].concat(),
                Some(&b""[..]),
            ),
            // A second member that is not gzip.
            ([plain.clone(), b"\x1f\x8b\x07".to_vec()].concat(), None),
            // Reserved flag bits, wrong method, wrong CRC, wrong length.
            (flip(&plain, 3, 0x20), None),
            (flip(&plain, 2, 0x01), None),
            (flip(&plain, plain.len() - 8, 0x01), None),
            (flip(&plain, plain.len() - 1, 0x01), None),
        ] {
            assert_parity("gzip", &wire, 4096, &chunks);
            for chunk in chunks {
                let (output, result) = stream(&gzip, cap(4096), &wire, chunk);
                match expected {
                    Some(expected) => {
                        assert_eq!(result, Ok(expected.len() as u64));
                        assert_eq!(output, expected);
                    }
                    None => assert_eq!(result, Err(StreamDecodeError::Corrupt)),
                }
            }
        }

        // An over-long header field is refused, as flate2 refuses it.
        for len in [GZ_FIELD_MAX, GZ_FIELD_MAX + 1] {
            let mut member = plain[..10].to_vec();
            member[3] |= GZ_FNAME;
            member.extend(std::iter::repeat_n(b'n', len));
            member.push(0);
            member.extend_from_slice(&plain[10..]);
            assert_parity("gzip", &member, 1 << 20, &[4096, usize::MAX]);
            assert_eq!(
                stream(&gzip, cap(1 << 20), &member, 4096).1.is_ok(),
                len == GZ_FIELD_MAX
            );
        }
    }

    fn flip(bytes: &[u8], at: usize, mask: u8) -> Vec<u8> {
        let mut bytes = bytes.to_vec();
        bytes[at] ^= mask;
        bytes
    }

    #[test]
    fn zstd_frames_concatenate_and_oversized_windows_are_refused() {
        let frames = [
            encoded("zstd", b"one"),
            encoded("zstd", b""),
            encoded("zstd", &sample(200_000)),
        ]
        .concat();
        assert_parity("zstd", &frames, 1 << 20, &[1, 1000, usize::MAX]);
        let (output, result) = stream(&headers("zstd"), cap(1 << 20), &frames, 1000);
        assert_eq!(result, Ok(200_003));
        assert!(output.starts_with(b"one{"));

        // A frame declaring a window of 2^log bytes: the header is written
        // before the encoder knows the body is small.
        let windowed = |log: u32| {
            let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
            encoder.window_log(log).unwrap();
            encoder.write_all(b"synthetic ").unwrap();
            encoder.flush().unwrap();
            encoder.write_all(b"body").unwrap();
            encoder.finish().unwrap()
        };
        let zstd = headers("zstd");
        let accepted = windowed(ZSTD_WINDOW_LOG_MAX);
        assert_parity("zstd", &accepted, 4096, &[1, usize::MAX]);
        assert_eq!(stream(&zstd, cap(4096), &accepted, 1).1, Ok(14));
        let refused = windowed(ZSTD_WINDOW_LOG_MAX + 1);
        assert_parity("zstd", &refused, 4096, &[1, usize::MAX]);
        for chunk in [1, usize::MAX] {
            assert_eq!(
                stream(&zstd, cap(4096), &refused, chunk),
                (Vec::new(), Err(StreamDecodeError::Corrupt))
            );
        }
    }

    #[test]
    fn decoded_byte_budget_is_enforced_per_stage() {
        for coding in ["identity", "zstd", "gzip", "deflate", "br"] {
            let headers = headers(coding);
            // Exactly the budget passes; one byte more does not.
            let fits = encoded(coding, &[b'x'; 1024]);
            let over = encoded(coding, &[b'x'; 1025]);
            for chunk in [1, 3, usize::MAX] {
                assert_eq!(stream(&headers, cap(1024), &fits, chunk).1, Ok(1024));
                let (output, result) = stream(&headers, cap(1024), &over, chunk);
                assert_eq!(result, Err(StreamDecodeError::BudgetExceeded), "{coding}");
                assert!(output.len() <= 1024 && output.iter().all(|b| *b == b'x'));
            }
            assert_parity(coding, &over, 1024, &[1, usize::MAX]);
            assert_parity(coding, &fits, 1024, &[1, usize::MAX]);
            // A larger bomb is cut off at the budget, not after it.
            let bomb = encoded(coding, &vec![0; 8 * 1024 * 1024]);
            let (output, result) = stream(&headers, cap(1024 * 1024), &bomb, 512);
            assert_eq!(result, Err(StreamDecodeError::BudgetExceeded), "{coding}");
            assert!(output.len() <= 1024 * 1024);
        }
        // Encoded input alone may not exceed the budget either.
        let incompressible = encoded("gzip", &sample(64));
        assert!(incompressible.len() > 64);
        assert_parity("gzip", &incompressible, 64, &[1, usize::MAX]);
        assert_eq!(
            stream(&headers("gzip"), cap(64), &incompressible, usize::MAX),
            (Vec::new(), Err(StreamDecodeError::BudgetExceeded))
        );
        // Stacked bombs: each stage is held to the budget, so the innermost
        // expansion is stopped although the outer layers are tiny.
        for stack in ["gzip, gzip", "zstd, br, gzip", "deflate, zstd, zstd, zstd"] {
            let bomb = encoded_stack(stack, &vec![0; 16 * 1024 * 1024]);
            assert!(bomb.len() < 4096, "{stack}: {}", bomb.len());
            let (output, result) = stream(&headers(stack), cap(1024 * 1024), &bomb, 64);
            assert_eq!(result, Err(StreamDecodeError::BudgetExceeded), "{stack}");
            assert!(output.len() <= 1024 * 1024);
            assert_parity(stack, &bomb, 1024 * 1024, &[usize::MAX]);
        }
        assert_eq!(DecodeLimits::default().max_decoded, 1 << 30);
    }

    #[test]
    fn expansion_ratio_guard_stops_bombs_above_the_floor() {
        let megabyte = 1024 * 1024;
        let zeros = vec![0u8; 4 * megabyte];
        let text = sample(megabyte);
        for coding in CODINGS {
            let headers = headers(coding);
            let bomb = encoded(coding, &zeros);
            let ratio = zeros.len() / bomb.len();
            assert!(ratio > 500, "{coding}: {ratio}");
            let limits = |max_ratio, ratio_floor| DecodeLimits {
                max_decoded: 1 << 30,
                max_ratio,
                ratio_floor,
            };
            for chunk in [1, 100, usize::MAX] {
                let (output, result) = stream(&headers, limits(100, megabyte as u64), &bomb, chunk);
                assert_eq!(result, Err(StreamDecodeError::ExpansionRatio), "{coding}");
                // Stopped near the floor, long before the whole expansion.
                assert!(output.len() <= megabyte + DECODED_BLOCK, "{coding}");
            }
            // Below the floor, or within the ratio, the same body decodes.
            let floor = zeros.len() as u64;
            assert_eq!(
                stream(&headers, limits(100, floor), &bomb, 100).1,
                Ok(floor)
            );
            assert_eq!(
                stream(&headers, limits(1 << 20, 0), &bomb, 100).1,
                Ok(floor)
            );
            // Ordinary text is far inside the ratio even with no floor.
            let wire = encoded(coding, &text);
            assert_eq!(
                stream(&headers, limits(100, 0), &wire, 100).1,
                Ok(megabyte as u64)
            );
            // The defaults leave bodies under 64 MiB alone, whatever the ratio.
            assert_eq!(
                stream(&headers, DecodeLimits::default(), &bomb, 100).1,
                Ok(floor)
            );
        }
        // A stack is measured against the bytes received, not a stage's input.
        let bomb = encoded_stack("gzip, zstd", &zeros);
        let limits = DecodeLimits {
            max_decoded: 1 << 30,
            max_ratio: 100,
            ratio_floor: megabyte as u64,
        };
        assert_eq!(
            stream(&headers("gzip, zstd"), limits, &bomb, 16).1,
            Err(StreamDecodeError::ExpansionRatio)
        );
        // The default guard does stop a bomb above the floor.
        let defaults = DecodeLimits::default();
        assert_eq!((defaults.max_ratio, defaults.ratio_floor), (1024, 64 << 20));
        let huge = encoded("zstd", &vec![0u8; 80 * megabyte]);
        let mut decoder = StreamDecoder::new(&headers("zstd")).unwrap();
        let mut delivered = 0u64;
        let result = decoder.push(&huge, |decoded| {
            delivered += decoded.len() as u64;
            ControlFlow::Continue(())
        });
        assert_eq!(result, Err(StreamDecodeError::ExpansionRatio));
        assert!(delivered <= defaults.ratio_floor + DECODED_BLOCK as u64);
    }

    #[test]
    fn a_sink_can_stop_the_decoder() {
        let wire = encoded("gzip", &vec![7u8; 1024 * 1024]);
        let mut decoder = StreamDecoder::new(&headers("gzip")).unwrap();
        let mut calls = 0;
        let result = decoder.push(&wire, |_| {
            calls += 1;
            if calls == 2 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        assert_eq!(result, Err(StreamDecodeError::Aborted));
        assert_eq!(calls, 2);
        assert_eq!(
            decoder.push(b"more", |_| panic!("spent")),
            Err(StreamDecodeError::Aborted)
        );
        assert_eq!(decoder.finish(), Err(StreamDecodeError::Aborted));

        // Identity bodies stop the same way.
        let mut decoder = StreamDecoder::new(&HeaderMap::new()).unwrap();
        assert_eq!(
            decoder.push(b"abc", |_| ControlFlow::Break(())),
            Err(StreamDecodeError::Aborted)
        );
    }

    #[test]
    fn stream_errors_map_onto_the_buffered_errors() {
        for (error, batch, status) in [
            (
                StreamDecodeError::Unsupported,
                Some(DecodeError::Unsupported),
                415,
            ),
            (StreamDecodeError::Corrupt, Some(DecodeError::Invalid), 400),
            (
                StreamDecodeError::Truncated,
                Some(DecodeError::Invalid),
                400,
            ),
            (
                StreamDecodeError::BudgetExceeded,
                Some(DecodeError::TooLarge),
                413,
            ),
            (StreamDecodeError::ExpansionRatio, None, 413),
            (StreamDecodeError::Aborted, None, 500),
        ] {
            assert_eq!(error.as_batch(), batch);
            assert_eq!(error.status(), status);
            if let Some(batch) = batch {
                assert_eq!(error.status(), batch.status());
            }
            assert_eq!(error.to_string(), error.message());
            assert!(!error.message().is_empty());
        }
    }
}
