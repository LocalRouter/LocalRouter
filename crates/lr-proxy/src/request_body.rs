//! Bounded HTTP content decoding for policy inspection. Forward keeps the
//! original representation; rewriting sends identity bytes with fresh headers.

use std::io::{Cursor, Read};

use bytes::Bytes;
use http::HeaderMap;

const LIMIT: usize = lr_secret_scanner::wire::LIMIT;
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
        let status = decoder
            .decompress(
                &bytes[before.0 as usize..],
                &mut buffer[..available],
                flate2::FlushDecompress::Finish,
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

fn decode(headers: &HeaderMap, bytes: Bytes, limit: usize) -> Result<Bytes, DecodeError> {
    if bytes.len() > limit {
        return Err(DecodeError::TooLarge);
    }
    let mut encodings = Vec::new();
    for value in headers.get_all(http::header::CONTENT_ENCODING) {
        for encoding in value
            .to_str()
            .map_err(|_| DecodeError::Unsupported)?
            .split(',')
        {
            let encoding = encoding.trim().to_ascii_lowercase();
            if !matches!(
                encoding.as_str(),
                "identity" | "zstd" | "gzip" | "deflate" | "br"
            ) {
                return Err(DecodeError::Unsupported);
            }
            encodings.push(encoding);
            if encodings.len() > MAX_ENCODINGS {
                return Err(DecodeError::Unsupported);
            }
        }
    }
    let mut bytes = bytes;
    // Content-Encoding lists encodings in the order they were applied.
    for encoding in encodings.iter().rev() {
        let input = Cursor::new(bytes.as_ref());
        let decoded = match encoding.as_str() {
            "identity" => continue,
            "zstd" => {
                let mut decoder =
                    zstd::stream::read::Decoder::new(input).map_err(|_| DecodeError::Invalid)?;
                decoder
                    .window_log_max(24)
                    .map_err(|_| DecodeError::Invalid)?;
                bounded(decoder, limit)?
            }
            "gzip" => bounded(flate2::read::MultiGzDecoder::new(input), limit)?,
            // HTTP deflate is a zlib wrapper, not raw DEFLATE.
            "deflate" => zlib(bytes.as_ref(), limit)?,
            "br" => bounded(brotli::Decompressor::new(input, 8192), limit)?,
            _ => unreachable!("encodings validated above"),
        };
        bytes = Bytes::from(decoded);
    }
    Ok(bytes)
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
}
