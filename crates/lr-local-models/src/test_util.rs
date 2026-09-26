//! Shared test helpers.

use wiremock::{Request, Respond, ResponseTemplate};

/// Serves `data`, honouring `Range: bytes=N-` / `bytes=N-M` with 206 (and 416
/// past the end) unless `ignore_range` is set, in which case it always answers
/// 200 with the full body.
#[derive(Clone)]
pub(crate) struct RangeResponder {
    data: Vec<u8>,
    ignore_range: bool,
}

impl RangeResponder {
    pub(crate) fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            ignore_range: false,
        }
    }

    pub(crate) fn ignoring_range(data: Vec<u8>) -> Self {
        Self {
            data,
            ignore_range: true,
        }
    }
}

impl Respond for RangeResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let len = self.data.len() as u64;
        let range = request
            .headers
            .get("range")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("bytes="))
            .and_then(|v| {
                let (a, b) = v.split_once('-')?;
                let start: u64 = a.parse().ok()?;
                let end: Option<u64> = if b.is_empty() { None } else { b.parse().ok() };
                Some((start, end))
            });
        match range {
            Some((start, end)) if !self.ignore_range => {
                if start >= len {
                    return ResponseTemplate::new(416)
                        .insert_header("content-range", format!("bytes */{len}").as_str());
                }
                let end = end.unwrap_or(len - 1).min(len - 1);
                ResponseTemplate::new(206)
                    .insert_header(
                        "content-range",
                        format!("bytes {start}-{end}/{len}").as_str(),
                    )
                    .set_body_bytes(self.data[start as usize..=end as usize].to_vec())
            }
            _ => ResponseTemplate::new(200).set_body_bytes(self.data.clone()),
        }
    }
}
