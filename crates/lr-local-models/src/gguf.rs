//! Minimal, defensive GGUF v2/v3 header reader.
//!
//! Only the header is parsed: magic, version, tensor count, the metadata
//! key/value section and the tensor-info table (from which only tensor names
//! are kept). Tensor data is never read.
//!
//! The parser is written for untrusted input: every length is bounds-checked
//! against hard limits before anything is allocated, so a malformed or hostile
//! file cannot exhaust memory. When the input simply ends early the parser
//! returns [`GgufError::Incomplete`] with the minimum number of bytes it needs,
//! which lets remote readers fetch more bytes with HTTP Range requests.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use reqwest::header::{HeaderValue, CONTENT_RANGE, RANGE};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};

use crate::http;
use crate::hub::HubError;

/// Maximum accepted length of a single GGUF string.
pub const MAX_STRING_LEN: u64 = 16 * 1024 * 1024;
/// Maximum accepted number of elements in a GGUF array.
pub const MAX_ARRAY_LEN: u64 = 10_000_000;
/// Maximum accepted number of metadata key/value pairs.
pub const MAX_KV_COUNT: u64 = 100_000;
/// Maximum accepted number of tensors.
pub const MAX_TENSOR_COUNT: u64 = 1_000_000;
/// Arrays longer than this are not stored (only their length is recorded in
/// [`GgufHeader::array_lengths`]).
pub const MAX_STORED_ARRAY_LEN: u64 = 1000;
/// Maximum number of header bytes read from a local file.
pub const MAX_LOCAL_HEADER_BYTES: u64 = 64 * 1024 * 1024;
/// Maximum number of header bytes fetched from a remote file.
pub const MAX_REMOTE_HEADER_BYTES: u64 = 32 * 1024 * 1024;
/// Size of each HTTP Range request when reading a remote header.
pub const REMOTE_CHUNK_BYTES: u64 = 2 * 1024 * 1024;

/// Maximum number of tensor dimensions accepted (ggml uses 4).
const MAX_DIMS: u32 = 8;
/// Maximum nesting depth of arrays.
const MAX_ARRAY_DEPTH: u32 = 4;

/// Errors produced while reading a GGUF header.
#[derive(Debug, thiserror::Error)]
pub enum GgufError {
    /// The input ended before the header was complete. Supply at least
    /// `needed_at_least` bytes (counted from the start of the file) and retry.
    #[error("GGUF header incomplete: need at least {needed_at_least} bytes")]
    Incomplete { needed_at_least: u64 },
    /// The input is not a valid (supported) GGUF file.
    #[error("invalid GGUF file: {0}")]
    Invalid(String),
    /// A local I/O error.
    #[error("could not read GGUF file: {0}")]
    Io(String),
    /// A remote fetch failed.
    #[error(transparent)]
    Hub(#[from] HubError),
}

/// A GGUF metadata value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GgufValue {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    F32(f32),
    Bool(bool),
    String(String),
    Array(Vec<GgufValue>),
    U64(u64),
    I64(i64),
    F64(f64),
}

impl GgufValue {
    /// Integer value as `u64` (negative integers and non-integers yield `None`).
    pub fn as_u64(&self) -> Option<u64> {
        match *self {
            GgufValue::U8(v) => Some(v as u64),
            GgufValue::U16(v) => Some(v as u64),
            GgufValue::U32(v) => Some(v as u64),
            GgufValue::U64(v) => Some(v),
            GgufValue::I8(v) => u64::try_from(v).ok(),
            GgufValue::I16(v) => u64::try_from(v).ok(),
            GgufValue::I32(v) => u64::try_from(v).ok(),
            GgufValue::I64(v) => u64::try_from(v).ok(),
            _ => None,
        }
    }

    /// String value.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            GgufValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Array value (empty for arrays that were too long to store).
    pub fn as_array(&self) -> Option<&[GgufValue]> {
        match self {
            GgufValue::Array(v) => Some(v),
            _ => None,
        }
    }

    /// Boolean value (integers are accepted as 0 = false, otherwise true).
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            GgufValue::Bool(b) => Some(*b),
            other => other.as_u64().map(|v| v != 0),
        }
    }
}

/// A parsed GGUF header.
#[derive(Debug, Clone, PartialEq)]
pub struct GgufHeader {
    pub version: u32,
    pub tensor_count: u64,
    pub metadata: BTreeMap<String, GgufValue>,
    pub tensor_names: Vec<String>,
    /// Length of every array-valued metadata key. Arrays longer than
    /// [`MAX_STORED_ARRAY_LEN`] (e.g. `tokenizer.ggml.tokens`) are stored as an
    /// empty `GgufValue::Array`; their real length is only available here.
    pub array_lengths: BTreeMap<String, u64>,
}

impl GgufHeader {
    /// Metadata lookup.
    pub fn get(&self, key: &str) -> Option<&GgufValue> {
        self.metadata.get(key)
    }

    /// `general.architecture`.
    pub fn architecture(&self) -> Option<&str> {
        self.get("general.architecture").and_then(GgufValue::as_str)
    }

    /// `general.type` (e.g. `model`, `adapter`, `mmproj`).
    pub fn general_type(&self) -> Option<&str> {
        self.get("general.type").and_then(GgufValue::as_str)
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

const T_U8: u32 = 0;
const T_I8: u32 = 1;
const T_U16: u32 = 2;
const T_I16: u32 = 3;
const T_U32: u32 = 4;
const T_I32: u32 = 5;
const T_F32: u32 = 6;
const T_BOOL: u32 = 7;
const T_STRING: u32 = 8;
const T_ARRAY: u32 = 9;
const T_U64: u32 = 10;
const T_I64: u32 = 11;
const T_F64: u32 = 12;

fn fixed_size(ty: u32) -> Option<u64> {
    match ty {
        T_U8 | T_I8 | T_BOOL => Some(1),
        T_U16 | T_I16 => Some(2),
        T_U32 | T_I32 | T_F32 => Some(4),
        T_U64 | T_I64 | T_F64 => Some(8),
        _ => None,
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: u64,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: u64) -> Result<&'a [u8], GgufError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| GgufError::Invalid("length overflow".into()))?;
        if end > self.buf.len() as u64 {
            return Err(GgufError::Incomplete {
                needed_at_least: end,
            });
        }
        let slice = &self.buf[self.pos as usize..end as usize];
        self.pos = end;
        Ok(slice)
    }

    fn skip(&mut self, n: u64) -> Result<(), GgufError> {
        self.take(n).map(|_| ())
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], GgufError> {
        let s = self.take(N as u64)?;
        let mut out = [0u8; N];
        out.copy_from_slice(s);
        Ok(out)
    }

    fn u32(&mut self) -> Result<u32, GgufError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, GgufError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn string_len(&mut self) -> Result<u64, GgufError> {
        let len = self.u64()?;
        if len > MAX_STRING_LEN {
            return Err(GgufError::Invalid(format!(
                "string length {len} exceeds the {MAX_STRING_LEN} byte limit"
            )));
        }
        Ok(len)
    }

    fn string(&mut self) -> Result<String, GgufError> {
        let len = self.string_len()?;
        let bytes = self.take(len)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }

    fn skip_string(&mut self) -> Result<(), GgufError> {
        let len = self.string_len()?;
        self.skip(len)
    }

    /// Read a value of type `ty`. When `store` is false the value is skipped
    /// and `None` is returned.
    fn value(
        &mut self,
        ty: u32,
        depth: u32,
        store: bool,
    ) -> Result<(Option<GgufValue>, Option<u64>), GgufError> {
        let v = match ty {
            T_U8 => GgufValue::U8(self.array::<1>()?[0]),
            T_I8 => GgufValue::I8(i8::from_le_bytes(self.array()?)),
            T_U16 => GgufValue::U16(u16::from_le_bytes(self.array()?)),
            T_I16 => GgufValue::I16(i16::from_le_bytes(self.array()?)),
            T_U32 => GgufValue::U32(self.u32()?),
            T_I32 => GgufValue::I32(i32::from_le_bytes(self.array()?)),
            T_F32 => GgufValue::F32(f32::from_le_bytes(self.array()?)),
            T_BOOL => GgufValue::Bool(self.array::<1>()?[0] != 0),
            T_U64 => GgufValue::U64(self.u64()?),
            T_I64 => GgufValue::I64(i64::from_le_bytes(self.array()?)),
            T_F64 => GgufValue::F64(f64::from_le_bytes(self.array()?)),
            T_STRING => {
                if store {
                    GgufValue::String(self.string()?)
                } else {
                    self.skip_string()?;
                    return Ok((None, None));
                }
            }
            T_ARRAY => {
                if depth >= MAX_ARRAY_DEPTH {
                    return Err(GgufError::Invalid("arrays nested too deeply".into()));
                }
                let elem_ty = self.u32()?;
                let len = self.u64()?;
                if len > MAX_ARRAY_LEN {
                    return Err(GgufError::Invalid(format!(
                        "array length {len} exceeds the {MAX_ARRAY_LEN} element limit"
                    )));
                }
                if elem_ty > T_F64 {
                    return Err(GgufError::Invalid(format!(
                        "unknown array element type {elem_ty}"
                    )));
                }
                let keep = store && len <= MAX_STORED_ARRAY_LEN;
                if !keep {
                    if let Some(sz) = fixed_size(elem_ty) {
                        self.skip(sz * len)?;
                    } else {
                        for _ in 0..len {
                            self.value(elem_ty, depth + 1, false)?;
                        }
                    }
                    return Ok((store.then(|| GgufValue::Array(Vec::new())), Some(len)));
                }
                let mut items = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    if let (Some(item), _) = self.value(elem_ty, depth + 1, true)? {
                        items.push(item);
                    }
                }
                return Ok((Some(GgufValue::Array(items)), Some(len)));
            }
            other => {
                return Err(GgufError::Invalid(format!(
                    "unknown metadata value type {other}"
                )))
            }
        };
        Ok((store.then_some(v), None))
    }
}

/// Parse a GGUF header from the beginning of a file.
///
/// Returns [`GgufError::Incomplete`] when `bytes` ends before the header (the
/// metadata and tensor-info table) is complete, and [`GgufError::Invalid`] for
/// bad magic, unsupported versions or counts beyond the hard limits.
pub fn parse_header(bytes: &[u8]) -> Result<GgufHeader, GgufError> {
    let mut r = Reader { buf: bytes, pos: 0 };
    let magic = r.array::<4>()?;
    if &magic != b"GGUF" {
        return Err(GgufError::Invalid("bad magic (not a GGUF file)".into()));
    }
    let version = r.u32()?;
    match version {
        2 | 3 => {}
        1 => return Err(GgufError::Invalid("GGUF version 1 is not supported".into())),
        v if v.swap_bytes() == 2 || v.swap_bytes() == 3 => {
            return Err(GgufError::Invalid(
                "big-endian GGUF files are not supported".into(),
            ))
        }
        v => return Err(GgufError::Invalid(format!("unsupported GGUF version {v}"))),
    }
    let tensor_count = r.u64()?;
    if tensor_count > MAX_TENSOR_COUNT {
        return Err(GgufError::Invalid(format!(
            "tensor count {tensor_count} exceeds the {MAX_TENSOR_COUNT} limit"
        )));
    }
    let kv_count = r.u64()?;
    if kv_count > MAX_KV_COUNT {
        return Err(GgufError::Invalid(format!(
            "metadata count {kv_count} exceeds the {MAX_KV_COUNT} limit"
        )));
    }

    let mut metadata = BTreeMap::new();
    let mut array_lengths = BTreeMap::new();
    for _ in 0..kv_count {
        let key = r.string()?;
        let ty = r.u32()?;
        let (value, arr_len) = r.value(ty, 0, true)?;
        if let Some(len) = arr_len {
            array_lengths.insert(key.clone(), len);
        }
        if let Some(value) = value {
            metadata.insert(key, value);
        }
    }

    let mut tensor_names = Vec::with_capacity(tensor_count.min(4096) as usize);
    for _ in 0..tensor_count {
        let name = r.string()?;
        let n_dims = r.u32()?;
        if n_dims > MAX_DIMS {
            return Err(GgufError::Invalid(format!(
                "tensor {name} has {n_dims} dimensions"
            )));
        }
        r.skip(8 * n_dims as u64)?; // dims
        r.skip(4)?; // ggml type
        r.skip(8)?; // data offset
        tensor_names.push(name);
    }

    Ok(GgufHeader {
        version,
        tensor_count,
        metadata,
        tensor_names,
        array_lengths,
    })
}

/// Read the header of a local GGUF file, reading only as many bytes as the
/// header needs (at most [`MAX_LOCAL_HEADER_BYTES`]).
pub fn read_local_header(path: &Path) -> Result<GgufHeader, GgufError> {
    let mut file = std::fs::File::open(path).map_err(|e| GgufError::Io(e.to_string()))?;
    let file_len = file
        .metadata()
        .map_err(|e| GgufError::Io(e.to_string()))?
        .len();
    let mut buf: Vec<u8> = Vec::new();
    let mut target = (1024 * 1024u64).min(file_len);
    loop {
        if target > buf.len() as u64 {
            let want = target - buf.len() as u64;
            let start = buf.len();
            buf.resize(target as usize, 0);
            let mut filled = 0usize;
            while (filled as u64) < want {
                let n = file
                    .read(&mut buf[start + filled..])
                    .map_err(|e| GgufError::Io(e.to_string()))?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
            buf.truncate(start + filled);
        }
        match parse_header(&buf) {
            Err(GgufError::Incomplete { needed_at_least }) => {
                if needed_at_least > MAX_LOCAL_HEADER_BYTES {
                    return Err(GgufError::Invalid(format!(
                        "header larger than {} MiB",
                        MAX_LOCAL_HEADER_BYTES / (1024 * 1024)
                    )));
                }
                if needed_at_least > file_len || (buf.len() as u64) >= file_len {
                    return Err(GgufError::Invalid(
                        "file is truncated (header ends early)".into(),
                    ));
                }
                let doubled = (buf.len() as u64).saturating_mul(2);
                target = needed_at_least
                    .max(doubled)
                    .min(MAX_LOCAL_HEADER_BYTES)
                    .min(file_len);
            }
            other => return other,
        }
    }
}

/// Read the header of a remote GGUF file with HTTP Range requests (2 MiB
/// chunks, at most [`MAX_REMOTE_HEADER_BYTES`]).
///
/// Token handling: the bearer token is only sent to the origin of `url`
/// itself and to `https://huggingface.co`. Redirects are handled manually so
/// that the token is never forwarded to a CDN host. If the supplied client is
/// configured to follow redirects on its own, reqwest strips the
/// `Authorization` header on cross-origin redirects, so the token still does
/// not leak; prefer [`crate::HubClient::gguf_header`], which uses a
/// redirect-disabled client.
pub async fn read_remote_header(
    client: &reqwest::Client,
    url: &str,
    token: Option<&str>,
) -> Result<GgufHeader, GgufError> {
    let parsed = Url::parse(url).map_err(|e| GgufError::Invalid(format!("bad URL: {e}")))?;
    read_remote_header_with(client, parsed.clone(), &parsed, token, "").await
}

/// Remote header reader with an explicit trusted origin (the Hub endpoint).
pub(crate) async fn read_remote_header_with(
    client: &reqwest::Client,
    url: Url,
    trusted: &Url,
    token: Option<&str>,
    repo: &str,
) -> Result<GgufHeader, GgufError> {
    let mut buf: Vec<u8> = Vec::new();
    // After the first request, reuse the final (possibly CDN) URL for the
    // following ranges so the Hub is only asked to resolve once.
    let mut fetch_url = url.clone();
    let mut target = REMOTE_CHUNK_BYTES;
    let mut eof = false;
    loop {
        if !eof && target > buf.len() as u64 {
            let start = buf.len() as u64;
            let range = format!("bytes={}-{}", start, target - 1);
            let headers = [(
                RANGE,
                HeaderValue::from_str(&range).map_err(|e| GgufError::Invalid(e.to_string()))?,
            )];
            let followed =
                http::get_following(client, fetch_url.clone(), trusted, token, &headers, None)
                    .await?;
            let resp = followed.response;
            let status = resp.status();
            if status == StatusCode::RANGE_NOT_SATISFIABLE {
                eof = true;
            } else if status == StatusCode::PARTIAL_CONTENT || status == StatusCode::OK {
                let from = if status == StatusCode::PARTIAL_CONTENT {
                    content_range_start(resp.headers()).unwrap_or(start)
                } else {
                    0
                };
                if from != start && from != 0 {
                    return Err(GgufError::Invalid(
                        "server returned an unexpected range".into(),
                    ));
                }
                if from == 0 {
                    buf.clear();
                }
                fetch_url = followed.final_url;
                let want = target - buf.len() as u64;
                let got = read_body_limited(resp, want, &mut buf).await?;
                if got < want {
                    eof = true;
                }
            } else {
                return Err(GgufError::Hub(http::error_from_response(resp, repo).await));
            }
        }
        match parse_header(&buf) {
            Err(GgufError::Incomplete { needed_at_least }) => {
                if needed_at_least > MAX_REMOTE_HEADER_BYTES {
                    return Err(GgufError::Invalid(format!(
                        "header larger than {} MiB",
                        MAX_REMOTE_HEADER_BYTES / (1024 * 1024)
                    )));
                }
                if eof {
                    return Err(GgufError::Invalid(
                        "file is truncated (header ends early)".into(),
                    ));
                }
                let next = (buf.len() as u64 + REMOTE_CHUNK_BYTES).max(needed_at_least);
                target = next.min(MAX_REMOTE_HEADER_BYTES);
            }
            other => return other,
        }
    }
}

fn content_range_start(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let v = headers.get(CONTENT_RANGE)?.to_str().ok()?;
    let rest = v.trim().strip_prefix("bytes")?.trim_start();
    let (start, _) = rest.split_once('-')?;
    start.trim().parse().ok()
}

/// Append at most `limit` bytes of the response body to `buf`; returns the
/// number of bytes appended.
async fn read_body_limited(
    mut resp: reqwest::Response,
    limit: u64,
    buf: &mut Vec<u8>,
) -> Result<u64, GgufError> {
    let mut got = 0u64;
    while got < limit {
        let chunk = resp
            .chunk()
            .await
            .map_err(|e| GgufError::Hub(HubError::Network(e.without_url().to_string())))?;
        let Some(chunk) = chunk else { break };
        let take = (limit - got).min(chunk.len() as u64) as usize;
        buf.extend_from_slice(&chunk[..take]);
        got += take as u64;
    }
    Ok(got)
}

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

/// The facts about a GGUF file that the rest of the app cares about.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GgufSummary {
    pub architecture: Option<String>,
    pub name: Option<String>,
    pub file_type: Option<u32>,
    /// Quantisation label derived from `general.file_type` (e.g. `Q4_K_M`).
    pub quant: Option<String>,
    pub context_length: Option<u64>,
    pub embedding_length: Option<u64>,
    pub block_count: Option<u64>,
    pub head_count: Option<u64>,
    pub head_count_kv: Option<u64>,
    pub key_length: Option<u64>,
    pub value_length: Option<u64>,
    pub sliding_window: Option<u64>,
    pub pooling_type: Option<u32>,
    pub causal: Option<bool>,
    pub has_chat_template: bool,
    pub chat_template_mentions_tools: bool,
    pub split_count: Option<u64>,
    pub expert_count: Option<u64>,
    pub is_projector: bool,
    pub has_cls_tensors: bool,
}

impl GgufSummary {
    /// Extract the summary from a parsed header.
    pub fn from_header(h: &GgufHeader) -> Self {
        let architecture = h.architecture().map(str::to_string);
        let arch = architecture.clone().unwrap_or_default();
        let akey = |suffix: &str| format!("{arch}.{suffix}");
        let u64_of = |key: &str| h.get(key).and_then(value_u64_or_max);

        let file_type = h
            .get("general.file_type")
            .and_then(GgufValue::as_u64)
            .and_then(|v| u32::try_from(v).ok());

        let mut has_chat_template = false;
        let mut mentions_tools = false;
        for (key, value) in h.metadata.range("tokenizer.chat_template".to_string()..) {
            if !key.starts_with("tokenizer.chat_template") {
                break;
            }
            let is_named = key.len() > "tokenizer.chat_template".len();
            if is_named && !key.starts_with("tokenizer.chat_template.") {
                continue;
            }
            let Some(text) = value.as_str() else { continue };
            has_chat_template = true;
            if text.contains("tools") || key == "tokenizer.chat_template.tool_use" {
                mentions_tools = true;
            }
        }

        let general_type = h.general_type();
        GgufSummary {
            is_projector: arch == "clip" || general_type == Some("mmproj"),
            name: h
                .get("general.name")
                .and_then(GgufValue::as_str)
                .map(str::to_string),
            quant: file_type.and_then(ftype_name).map(str::to_string),
            file_type,
            context_length: u64_of(&akey("context_length")),
            embedding_length: u64_of(&akey("embedding_length")),
            block_count: u64_of(&akey("block_count")),
            head_count: u64_of(&akey("attention.head_count")),
            head_count_kv: u64_of(&akey("attention.head_count_kv")),
            key_length: u64_of(&akey("attention.key_length")),
            value_length: u64_of(&akey("attention.value_length")),
            sliding_window: u64_of(&akey("attention.sliding_window")),
            pooling_type: u64_of(&akey("pooling_type")).and_then(|v| u32::try_from(v).ok()),
            causal: h
                .get(&akey("attention.causal"))
                .and_then(GgufValue::as_bool),
            has_chat_template,
            chat_template_mentions_tools: mentions_tools,
            split_count: u64_of("split.count"),
            expert_count: u64_of(&akey("expert_count")),
            has_cls_tensors: h
                .tensor_names
                .iter()
                .any(|n| n == "cls" || n.starts_with("cls.")),
            architecture,
        }
    }
}

/// Integer value; per-layer arrays (some architectures store head counts per
/// layer) yield their maximum.
fn value_u64_or_max(v: &GgufValue) -> Option<u64> {
    match v {
        GgufValue::Array(items) => items.iter().filter_map(GgufValue::as_u64).max(),
        other => other.as_u64(),
    }
}

/// Quantisation name for llama.cpp's `llama_ftype` value (`general.file_type`).
pub fn ftype_name(ftype: u32) -> Option<&'static str> {
    Some(match ftype {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        7 => "Q8_0",
        8 => "Q5_0",
        9 => "Q5_1",
        10 => "Q2_K",
        11 => "Q3_K_S",
        12 => "Q3_K_M",
        13 => "Q3_K_L",
        14 => "Q4_K_S",
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        19 => "IQ2_XXS",
        20 => "IQ2_XS",
        21 => "Q2_K_S",
        22 => "IQ3_XS",
        23 => "IQ3_XXS",
        24 => "IQ1_S",
        25 => "IQ4_NL",
        26 => "IQ3_S",
        27 => "IQ3_M",
        28 => "IQ2_S",
        29 => "IQ2_M",
        30 => "IQ4_XS",
        31 => "IQ1_M",
        32 => "BF16",
        36 => "TQ1_0",
        37 => "TQ2_0",
        38 => "MXFP4_MOE",
        _ => return None,
    })
}

/// Strip a `-00001-of-00003` split suffix (after removing `.gguf`).
pub(crate) fn split_suffix(stem: &str) -> Option<(&str, u32, u32)> {
    // "<prefix>-NNNNN-of-NNNNN"
    let (rest, total) = stem.rsplit_once("-of-")?;
    let (prefix, part) = rest.rsplit_once('-')?;
    if part.len() != 5 || total.len() != 5 {
        return None;
    }
    let part: u32 = part.parse().ok()?;
    let total: u32 = total.parse().ok()?;
    if part == 0 || total == 0 || part > total {
        return None;
    }
    Some((prefix, part, total))
}

/// Strip a trailing `.gguf` (case-insensitive).
pub(crate) fn strip_gguf_ext(name: &str) -> &str {
    if name.len() >= 5 && name[name.len() - 5..].eq_ignore_ascii_case(".gguf") {
        &name[..name.len() - 5]
    } else {
        name
    }
}

fn is_quant_token(tok: &str) -> bool {
    const PLAIN: &[&str] = &["F16", "F32", "BF16", "F64", "MXFP4", "MXFP4_MOE"];
    if PLAIN.contains(&tok) {
        return true;
    }
    let rest = if let Some(r) = tok.strip_prefix("IQ") {
        r
    } else if let Some(r) = tok.strip_prefix("TQ") {
        r
    } else if let Some(r) = tok.strip_prefix('Q') {
        r
    } else {
        return false;
    };
    let mut chars = rest.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_digit() {
        return false;
    }
    let tail: &str = chars.as_str();
    // Q4, Q4_K_M, Q8_0, IQ2_XXS, Q4_0_4_4, Q4_K_XL ...
    (tail.is_empty() || tail.starts_with('_'))
        && tail
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Quantisation label guessed from a file name. Display fallback only; the
/// authoritative source is `general.file_type` in the header.
///
/// `"Qwen3-8B-Q4_K_M.gguf"` → `"Q4_K_M"`, `"gemma-3-4b-it-UD-Q4_K_XL.gguf"` →
/// `"UD-Q4_K_XL"`.
pub fn quant_from_filename(name: &str) -> Option<String> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = strip_gguf_ext(base);
    let stem = split_suffix(stem).map(|(p, _, _)| p).unwrap_or(stem);
    let upper = stem.to_ascii_uppercase();
    let tokens: Vec<&str> = upper.split(['-', '.']).collect();
    for i in (0..tokens.len()).rev() {
        if is_quant_token(tokens[i]) {
            if i > 0 && tokens[i - 1] == "UD" {
                return Some(format!("UD-{}", tokens[i]));
            }
            return Some(tokens[i].to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Test support: synthetic GGUF builder
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod test_support {
    /// A metadata value for [`GgufBuilder`].
    #[derive(Clone)]
    pub enum Val {
        U32(u32),
        U64(u64),
        I32(i32),
        F32(f32),
        Bool(bool),
        Str(String),
        StrArray(Vec<String>),
        U32Array(Vec<u32>),
    }

    /// Builds a minimal GGUF v3 file in memory.
    pub struct GgufBuilder {
        pub version: u32,
        kvs: Vec<(String, Val)>,
        tensors: Vec<String>,
    }

    impl Default for GgufBuilder {
        fn default() -> Self {
            Self::new()
        }
    }

    fn put_str(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(&(s.len() as u64).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }

    impl GgufBuilder {
        pub fn new() -> Self {
            Self {
                version: 3,
                kvs: Vec::new(),
                tensors: Vec::new(),
            }
        }

        pub fn kv(mut self, key: &str, v: Val) -> Self {
            self.kvs.push((key.to_string(), v));
            self
        }

        pub fn str(self, key: &str, v: &str) -> Self {
            self.kv(key, Val::Str(v.to_string()))
        }

        pub fn u32(self, key: &str, v: u32) -> Self {
            self.kv(key, Val::U32(v))
        }

        pub fn u64(self, key: &str, v: u64) -> Self {
            self.kv(key, Val::U64(v))
        }

        pub fn bool(self, key: &str, v: bool) -> Self {
            self.kv(key, Val::Bool(v))
        }

        pub fn tensor(mut self, name: &str) -> Self {
            self.tensors.push(name.to_string());
            self
        }

        /// A llama-like decoder with the given architecture.
        pub fn decoder(arch: &str) -> Self {
            Self::new()
                .str("general.architecture", arch)
                .str("general.name", "Test Model")
                .u32("general.file_type", 15)
                .u32(&format!("{arch}.context_length"), 32768)
                .u32(&format!("{arch}.embedding_length"), 4096)
                .u32(&format!("{arch}.block_count"), 36)
                .u32(&format!("{arch}.attention.head_count"), 32)
                .u32(&format!("{arch}.attention.head_count_kv"), 8)
                .tensor("token_embd.weight")
                .tensor("blk.0.attn_q.weight")
        }

        pub fn build(&self) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(b"GGUF");
            out.extend_from_slice(&self.version.to_le_bytes());
            out.extend_from_slice(&(self.tensors.len() as u64).to_le_bytes());
            out.extend_from_slice(&(self.kvs.len() as u64).to_le_bytes());
            for (k, v) in &self.kvs {
                put_str(&mut out, k);
                match v {
                    Val::U32(x) => {
                        out.extend_from_slice(&4u32.to_le_bytes());
                        out.extend_from_slice(&x.to_le_bytes());
                    }
                    Val::I32(x) => {
                        out.extend_from_slice(&5u32.to_le_bytes());
                        out.extend_from_slice(&x.to_le_bytes());
                    }
                    Val::F32(x) => {
                        out.extend_from_slice(&6u32.to_le_bytes());
                        out.extend_from_slice(&x.to_le_bytes());
                    }
                    Val::U64(x) => {
                        out.extend_from_slice(&10u32.to_le_bytes());
                        out.extend_from_slice(&x.to_le_bytes());
                    }
                    Val::Bool(x) => {
                        out.extend_from_slice(&7u32.to_le_bytes());
                        out.push(u8::from(*x));
                    }
                    Val::Str(s) => {
                        out.extend_from_slice(&8u32.to_le_bytes());
                        put_str(&mut out, s);
                    }
                    Val::StrArray(items) => {
                        out.extend_from_slice(&9u32.to_le_bytes());
                        out.extend_from_slice(&8u32.to_le_bytes());
                        out.extend_from_slice(&(items.len() as u64).to_le_bytes());
                        for s in items {
                            put_str(&mut out, s);
                        }
                    }
                    Val::U32Array(items) => {
                        out.extend_from_slice(&9u32.to_le_bytes());
                        out.extend_from_slice(&4u32.to_le_bytes());
                        out.extend_from_slice(&(items.len() as u64).to_le_bytes());
                        for x in items {
                            out.extend_from_slice(&x.to_le_bytes());
                        }
                    }
                }
            }
            for name in &self.tensors {
                put_str(&mut out, name);
                out.extend_from_slice(&2u32.to_le_bytes()); // n_dims
                out.extend_from_slice(&16u64.to_le_bytes());
                out.extend_from_slice(&16u64.to_le_bytes());
                out.extend_from_slice(&0u32.to_le_bytes()); // type
                out.extend_from_slice(&0u64.to_le_bytes()); // offset
            }
            out
        }

        /// Header plus `padding` bytes of fake tensor data.
        pub fn build_file(&self, padding: usize) -> Vec<u8> {
            let mut v = self.build();
            v.extend(std::iter::repeat_n(0xABu8, padding));
            v
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{GgufBuilder, Val};
    use super::*;

    #[test]
    fn parses_basic_header() {
        let bytes = GgufBuilder::decoder("qwen3")
            .str("tokenizer.chat_template", "{% if tools %}...{% endif %}")
            .kv("x.i32", Val::I32(-5))
            .kv("x.f32", Val::F32(1.5))
            .u64("x.u64", 1 << 40)
            .build_file(128);
        let h = parse_header(&bytes).unwrap();
        assert_eq!(h.version, 3);
        assert_eq!(h.tensor_count, 2);
        assert_eq!(
            h.tensor_names,
            vec!["token_embd.weight", "blk.0.attn_q.weight"]
        );
        assert_eq!(h.architecture(), Some("qwen3"));
        assert_eq!(h.get("x.i32"), Some(&GgufValue::I32(-5)));
        assert_eq!(h.get("x.i32").unwrap().as_u64(), None);
        assert_eq!(h.get("x.f32"), Some(&GgufValue::F32(1.5)));
        assert_eq!(h.get("x.u64").unwrap().as_u64(), Some(1 << 40));

        let s = GgufSummary::from_header(&h);
        assert_eq!(s.architecture.as_deref(), Some("qwen3"));
        assert_eq!(s.quant.as_deref(), Some("Q4_K_M"));
        assert_eq!(s.file_type, Some(15));
        assert_eq!(s.context_length, Some(32768));
        assert_eq!(s.block_count, Some(36));
        assert_eq!(s.head_count, Some(32));
        assert_eq!(s.head_count_kv, Some(8));
        assert!(s.has_chat_template);
        assert!(s.chat_template_mentions_tools);
        assert!(!s.is_projector);
        assert!(!s.has_cls_tensors);
    }

    #[test]
    fn version_2_is_accepted() {
        let mut b = GgufBuilder::decoder("llama");
        b.version = 2;
        assert_eq!(parse_header(&b.build()).unwrap().version, 2);
    }

    #[test]
    fn every_truncation_is_incomplete() {
        let bytes = GgufBuilder::decoder("llama")
            .kv(
                "tokenizer.ggml.tokens",
                Val::StrArray(vec!["a".into(), "bb".into()]),
            )
            .build();
        for cut in 0..bytes.len() {
            match parse_header(&bytes[..cut]) {
                Err(GgufError::Incomplete { needed_at_least }) => {
                    assert!(needed_at_least > cut as u64, "cut {cut}");
                    assert!(needed_at_least <= bytes.len() as u64, "cut {cut}");
                }
                other => panic!("cut {cut}: expected Incomplete, got {other:?}"),
            }
        }
        assert!(parse_header(&bytes).is_ok());
    }

    #[test]
    fn garbage_is_invalid() {
        assert!(matches!(
            parse_header(b"NOTAGGUFFILE AT ALL"),
            Err(GgufError::Invalid(_))
        ));
        let mut v = b"GGUF".to_vec();
        v.extend_from_slice(&7u32.to_le_bytes());
        assert!(matches!(parse_header(&v), Err(GgufError::Invalid(_))));
        let mut v = b"GGUF".to_vec();
        v.extend_from_slice(&1u32.to_le_bytes());
        assert!(matches!(parse_header(&v), Err(GgufError::Invalid(_))));
        let mut v = b"GGUF".to_vec();
        v.extend_from_slice(&3u32.to_be_bytes());
        let err = parse_header(&v).unwrap_err().to_string();
        assert!(err.contains("big-endian"), "{err}");
    }

    #[test]
    fn absurd_counts_are_invalid() {
        // Tensor count.
        let mut v = b"GGUF".to_vec();
        v.extend_from_slice(&3u32.to_le_bytes());
        v.extend_from_slice(&(MAX_TENSOR_COUNT + 1).to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes());
        assert!(matches!(parse_header(&v), Err(GgufError::Invalid(_))));
        // KV count.
        let mut v = b"GGUF".to_vec();
        v.extend_from_slice(&3u32.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes());
        v.extend_from_slice(&(MAX_KV_COUNT + 1).to_le_bytes());
        assert!(matches!(parse_header(&v), Err(GgufError::Invalid(_))));
        // Huge string length.
        let mut v = b"GGUF".to_vec();
        v.extend_from_slice(&3u32.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes());
        v.extend_from_slice(&1u64.to_le_bytes());
        v.extend_from_slice(&(MAX_STRING_LEN + 1).to_le_bytes());
        assert!(matches!(parse_header(&v), Err(GgufError::Invalid(_))));
        // Huge array length.
        let mut v = b"GGUF".to_vec();
        v.extend_from_slice(&3u32.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes());
        v.extend_from_slice(&1u64.to_le_bytes());
        v.extend_from_slice(&1u64.to_le_bytes());
        v.push(b'k');
        v.extend_from_slice(&9u32.to_le_bytes());
        v.extend_from_slice(&4u32.to_le_bytes());
        v.extend_from_slice(&(MAX_ARRAY_LEN + 1).to_le_bytes());
        assert!(matches!(parse_header(&v), Err(GgufError::Invalid(_))));
        // Unknown value type.
        let mut v = b"GGUF".to_vec();
        v.extend_from_slice(&3u32.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes());
        v.extend_from_slice(&1u64.to_le_bytes());
        v.extend_from_slice(&1u64.to_le_bytes());
        v.push(b'k');
        v.extend_from_slice(&99u32.to_le_bytes());
        assert!(matches!(parse_header(&v), Err(GgufError::Invalid(_))));
    }

    #[test]
    fn long_arrays_are_skipped_but_counted() {
        let tokens: Vec<String> = (0..1500).map(|i| format!("t{i}")).collect();
        let bytes = GgufBuilder::decoder("llama")
            .kv("tokenizer.ggml.tokens", Val::StrArray(tokens))
            .kv("small", Val::U32Array(vec![1, 2, 3]))
            .kv("big.u32", Val::U32Array(vec![7; 5000]))
            .build();
        let h = parse_header(&bytes).unwrap();
        assert_eq!(
            h.get("tokenizer.ggml.tokens")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            0
        );
        assert_eq!(h.array_lengths["tokenizer.ggml.tokens"], 1500);
        assert_eq!(h.array_lengths["big.u32"], 5000);
        assert_eq!(h.get("small").unwrap().as_array().unwrap().len(), 3);
        assert_eq!(h.array_lengths["small"], 3);
        // Tensor table after the skipped arrays still parses.
        assert_eq!(h.tensor_names.len(), 2);
    }

    #[test]
    fn per_layer_head_counts_use_max() {
        let bytes = GgufBuilder::new()
            .str("general.architecture", "x")
            .kv("x.attention.head_count_kv", Val::U32Array(vec![0, 8, 4]))
            .build();
        let s = GgufSummary::from_header(&parse_header(&bytes).unwrap());
        assert_eq!(s.head_count_kv, Some(8));
    }

    #[test]
    fn named_tool_use_template_counts_as_tools() {
        let bytes = GgufBuilder::decoder("llama")
            .str("tokenizer.chat_template", "{{ messages }}")
            .str("tokenizer.chat_template.tool_use", "{{ x }}")
            .build();
        let s = GgufSummary::from_header(&parse_header(&bytes).unwrap());
        assert!(s.has_chat_template);
        assert!(s.chat_template_mentions_tools);

        let bytes = GgufBuilder::decoder("llama")
            .str("tokenizer.chat_template", "{{ messages }}")
            .build();
        let s = GgufSummary::from_header(&parse_header(&bytes).unwrap());
        assert!(s.has_chat_template);
        assert!(!s.chat_template_mentions_tools);
    }

    #[test]
    fn split_and_projector_and_cls_fields() {
        let bytes = GgufBuilder::decoder("llama")
            .kv("split.count", Val::U32(3))
            .tensor("cls.output.weight")
            .build();
        let s = GgufSummary::from_header(&parse_header(&bytes).unwrap());
        assert_eq!(s.split_count, Some(3));
        assert!(s.has_cls_tensors);

        let bytes = GgufBuilder::new()
            .str("general.architecture", "clip")
            .bool("clip.has_vision_encoder", true)
            .build();
        let s = GgufSummary::from_header(&parse_header(&bytes).unwrap());
        assert!(s.is_projector);
    }

    #[test]
    fn local_header_reads_and_rejects_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.gguf");
        let bytes = GgufBuilder::decoder("llama").build_file(3 * 1024 * 1024);
        std::fs::write(&path, &bytes).unwrap();
        let h = read_local_header(&path).unwrap();
        assert_eq!(h.architecture(), Some("llama"));

        // A header bigger than the first 1 MiB read.
        let big: Vec<String> = (0..80_000).map(|i| format!("token-{i:08}")).collect();
        let bytes = GgufBuilder::decoder("llama")
            .kv("tokenizer.ggml.tokens", Val::StrArray(big))
            .build_file(10);
        assert!(bytes.len() > 1024 * 1024);
        std::fs::write(&path, &bytes).unwrap();
        let h = read_local_header(&path).unwrap();
        assert_eq!(h.array_lengths["tokenizer.ggml.tokens"], 80_000);

        let header_only = GgufBuilder::decoder("llama").build();
        std::fs::write(&path, &header_only[..header_only.len() - 3]).unwrap();
        assert!(matches!(
            read_local_header(&path),
            Err(GgufError::Invalid(_))
        ));
        assert!(matches!(
            read_local_header(&dir.path().join("missing.gguf")),
            Err(GgufError::Io(_))
        ));
    }

    #[test]
    fn quant_names() {
        assert_eq!(ftype_name(15), Some("Q4_K_M"));
        assert_eq!(ftype_name(32), Some("BF16"));
        assert_eq!(ftype_name(38), Some("MXFP4_MOE"));
        assert_eq!(ftype_name(4), None);
    }

    #[test]
    fn quant_from_filenames() {
        let q = |s: &str| quant_from_filename(s);
        assert_eq!(q("Qwen3-8B-Q4_K_M.gguf").as_deref(), Some("Q4_K_M"));
        assert_eq!(
            q("gemma-3-4b-it-UD-Q4_K_XL.gguf").as_deref(),
            Some("UD-Q4_K_XL")
        );
        assert_eq!(q("model.Q8_0.gguf").as_deref(), Some("Q8_0"));
        assert_eq!(q("Llama-3.2-1B-Instruct-f16.gguf").as_deref(), Some("F16"));
        assert_eq!(
            q("qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf").as_deref(),
            Some("Q4_K_M")
        );
        assert_eq!(q("sub/dir/x-IQ2_XXS.gguf").as_deref(), Some("IQ2_XXS"));
        assert_eq!(q("mmproj-model-bf16.gguf").as_deref(), Some("BF16"));
        assert_eq!(q("QwQ-32B.gguf"), None);
        assert_eq!(q("Qwen3-8B.gguf"), None);
    }

    #[test]
    fn split_suffix_parsing() {
        assert_eq!(split_suffix("a-b-00001-of-00003"), Some(("a-b", 1, 3)));
        assert_eq!(split_suffix("a-00004-of-00003"), None);
        assert_eq!(split_suffix("a-1-of-3"), None);
        assert_eq!(strip_gguf_ext("x.GGUF"), "x");
    }
}
