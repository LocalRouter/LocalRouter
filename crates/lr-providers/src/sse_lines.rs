//! Line framing for streamed HTTP bodies (SSE and NDJSON).
//!
//! A network read can end anywhere: in the middle of a line, or in the
//! middle of a multi-byte UTF-8 character. [`lines`] carries the trailing
//! partial line (as bytes) over to the next read and yields only complete
//! lines, so a `data: {...}` frame split across reads is parsed once, whole.

use std::collections::VecDeque;
use std::pin::Pin;

use futures::{Stream, StreamExt};

/// Split a byte stream into lines.
///
/// Lines are separated by `\n`; a trailing `\r` is removed. Each complete
/// line is decoded as UTF-8 (lossily) only once all its bytes have arrived.
/// When the body ends without a final newline, the remaining bytes are
/// yielded as the last line. Read errors are passed through in order and
/// the stream continues with the following reads.
pub(crate) fn lines<S, B, E>(bytes: S) -> impl Stream<Item = Result<String, E>> + Send + 'static
where
    S: Stream<Item = Result<B, E>> + Send + 'static,
    B: AsRef<[u8]>,
    E: Send + 'static,
{
    line_batches(bytes).flat_map(|batch| {
        futures::stream::iter(match batch {
            Ok(lines) => lines.into_iter().map(Ok).collect(),
            Err(error) => vec![Err(error)],
        })
    })
}

/// Frame complete lines in batches, preserving the grouping of each HTTP read.
/// Adapters that can emit multiple completion chunks per read can consume this
/// directly, with the same byte-boundary and EOF guarantees as [`lines`].
pub(crate) fn line_batches<S, B, E>(
    bytes: S,
) -> impl Stream<Item = Result<Vec<String>, E>> + Send + 'static
where
    S: Stream<Item = Result<B, E>> + Send + 'static,
    B: AsRef<[u8]>,
    E: Send + 'static,
{
    let state = LineState {
        inner: Box::pin(bytes),
        partial: Vec::new(),
        ready: VecDeque::new(),
        ended: false,
    };
    futures::stream::unfold(state, |mut state| async move {
        loop {
            if let Some(item) = state.ready.pop_front() {
                return Some((item, state));
            }
            if state.ended {
                return None;
            }
            match state.inner.next().await {
                Some(Ok(read)) => state.push(read.as_ref()),
                Some(Err(e)) => state.ready.push_back(Err(e)),
                None => {
                    state.ended = true;
                    if !state.partial.is_empty() {
                        let rest = std::mem::take(&mut state.partial);
                        state.ready.push_back(Ok(vec![decode(&rest)]));
                    }
                }
            }
        }
    })
}

struct LineState<S, E> {
    inner: Pin<Box<S>>,
    /// Bytes after the last `\n` seen so far.
    partial: Vec<u8>,
    ready: VecDeque<Result<Vec<String>, E>>,
    ended: bool,
}

impl<S, E> LineState<S, E> {
    fn push(&mut self, read: &[u8]) {
        self.partial.extend_from_slice(read);
        let Some(last_newline) = self.partial.iter().rposition(|&b| b == b'\n') else {
            return;
        };
        let rest = self.partial.split_off(last_newline + 1);
        let complete = std::mem::replace(&mut self.partial, rest);
        self.ready.push_back(Ok(complete[..last_newline]
            .split(|&b| b == b'\n')
            .map(decode)
            .collect()));
    }
}

fn decode(line: &[u8]) -> String {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    String::from_utf8_lossy(line).into_owned()
}

#[cfg(test)]
pub(crate) mod test_support {
    use futures::Stream;

    /// `body` delivered as consecutive reads split at each of `cuts`
    /// (byte offsets, ascending).
    pub(crate) fn reads_split_at(
        body: &str,
        cuts: &[usize],
    ) -> impl Stream<Item = Result<Vec<u8>, reqwest::Error>> + Send + 'static {
        let bytes = body.as_bytes();
        let mut reads = Vec::new();
        let mut start = 0;
        for &cut in cuts {
            reads.push(Ok(bytes[start..cut].to_vec()));
            start = cut;
        }
        reads.push(Ok(bytes[start..].to_vec()));
        futures::stream::iter(reads)
    }

    /// Every way of splitting `body` into reads that a provider test should
    /// survive: two reads split at each byte offset, and one read per byte.
    pub(crate) fn split_variants(body: &str) -> Vec<Vec<usize>> {
        let mut variants: Vec<Vec<usize>> = (1..body.len()).map(|cut| vec![cut]).collect();
        variants.push((1..body.len()).collect());
        variants
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::reads_split_at;
    use super::*;

    #[tokio::test]
    async fn batches_preserve_unicode_crlf_sentinels_and_unterminated_tail() {
        let body = "data: é🌍\r\n\r\ndata: [DONE]\r\nlast 中";
        for cuts in test_support::split_variants(body) {
            let batches: Vec<_> = line_batches(reads_split_at(body, &cuts)).collect().await;
            let decoded: Vec<_> = batches
                .into_iter()
                .flat_map(|batch| batch.unwrap())
                .collect();
            assert_eq!(
                decoded,
                ["data: é🌍", "", "data: [DONE]", "last 中"],
                "cuts: {cuts:?}"
            );
        }
    }

    async fn collect<S>(stream: S) -> Vec<Result<String, &'static str>>
    where
        S: Stream<Item = Result<String, &'static str>>,
    {
        stream.collect().await
    }

    fn ok_lines(items: Vec<Result<String, &'static str>>) -> Vec<String> {
        items.into_iter().map(|r| r.expect("line")).collect()
    }

    fn reads(parts: &[&[u8]]) -> impl Stream<Item = Result<Vec<u8>, &'static str>> + Send {
        let items: Vec<Result<Vec<u8>, &'static str>> =
            parts.iter().map(|p| Ok(p.to_vec())).collect();
        futures::stream::iter(items)
    }

    #[tokio::test]
    async fn line_split_across_reads_is_joined() {
        let got = collect(lines(reads(&[
            b"data: {\"a\":",
            b"1}\n\ndata: {\"b\"",
            b":2}\n\n",
        ])))
        .await;
        assert_eq!(
            ok_lines(got),
            vec!["data: {\"a\":1}", "", "data: {\"b\":2}", ""]
        );
    }

    #[tokio::test]
    async fn multibyte_char_split_across_reads_survives() {
        let body = "data: h\u{e9}llo \u{1f600}\n".as_bytes();
        // Split inside the 2-byte 'é' and inside the 4-byte emoji.
        let e_acute = body.iter().position(|&b| b == 0xC3).unwrap();
        let emoji = body.iter().position(|&b| b == 0xF0).unwrap();
        let got = collect(lines(reads(&[
            &body[..e_acute + 1],
            &body[e_acute + 1..emoji + 2],
            &body[emoji + 2..emoji + 3],
            &body[emoji + 3..],
        ])))
        .await;
        assert_eq!(ok_lines(got), vec!["data: h\u{e9}llo \u{1f600}"]);
    }

    #[tokio::test]
    async fn every_single_byte_read_yields_the_same_lines() {
        let body = "data: \u{e9}1\r\ndata: 2\n\ndata: [DONE]\n";
        let parts: Vec<&[u8]> = body.as_bytes().chunks(1).collect();
        let got = collect(lines(reads(&parts))).await;
        assert_eq!(
            ok_lines(got),
            vec!["data: \u{e9}1", "data: 2", "", "data: [DONE]"]
        );
    }

    #[tokio::test]
    async fn unterminated_last_line_is_flushed_at_end() {
        let got = collect(lines(reads(&[b"data: 1\ndata: ", b"2"]))).await;
        assert_eq!(ok_lines(got), vec!["data: 1", "data: 2"]);
    }

    #[tokio::test]
    async fn empty_body_yields_nothing() {
        let got = collect(lines(reads(&[]))).await;
        assert!(got.is_empty());
    }

    #[tokio::test]
    async fn read_errors_pass_through_in_order() {
        let items: Vec<Result<Vec<u8>, &'static str>> = vec![
            Ok(b"data: 1\ndata: ".to_vec()),
            Err("boom"),
            Ok(b"2\n".to_vec()),
        ];
        let got = collect(lines(futures::stream::iter(items))).await;
        assert_eq!(
            got,
            vec![
                Ok("data: 1".to_string()),
                Err("boom"),
                Ok("data: 2".to_string())
            ]
        );
    }

    #[tokio::test]
    async fn test_support_split_reassembles_body() {
        let body = "data: 1\ndata: 2\n";
        let stream = reads_split_at(body, &[3, 9, 12]);
        let got: Vec<String> = lines(stream).map(|r| r.expect("line")).collect().await;
        assert_eq!(got, vec!["data: 1", "data: 2"]);
    }
}
