//! A response body that streams through unchanged while capturing a bounded
//! copy for monitoring, then fires a completion callback at end-of-stream.
//!
//! This is what makes passive inspection *streaming*: the client (e.g. Claude
//! Code) receives SSE frames as they arrive, while the proxy accumulates a
//! size-capped copy and records the exchange once the stream ends.

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body::{Body, Frame};

/// Finish a pending exchange if its service future is dropped before a body
/// completion callback has taken over (for example a cancelled Codex prewarm).
pub(crate) struct CancellationGuard(Option<Box<dyn FnOnce() + Send>>);
impl CancellationGuard {
    pub(crate) fn new(callback: impl FnOnce() + Send + 'static) -> Self {
        Self(Some(Box::new(callback)))
    }
    pub(crate) fn disarm(&mut self) {
        self.0.take();
    }
}
impl Drop for CancellationGuard {
    fn drop(&mut self) {
        if let Some(callback) = self.0.take() {
            callback();
        }
    }
}

/// What a [`TappedBody`] captured of a response body.
///
/// When the whole body fit, `bytes` holds all of it and `tail` is empty.
/// Otherwise `bytes` is the head, `omitted` bytes after it were not kept, and
/// `tail` holds the end of the body, starting at a line boundary when one was
/// within reach, so the terminal SSE or NDJSON event (which carries the
/// usage) survives a long stream.
#[derive(Debug, Default)]
pub struct Capture {
    pub bytes: Vec<u8>,
    pub error: Option<String>,
    pub tail: Vec<u8>,
    pub omitted: u64,
    /// The usage-bearing events, read as the body streamed (bodies tapped
    /// [`with_tail`](TappedBody::with_tail)).
    pub usage: Option<crate::usage_stream::UsageEvents>,
}

impl Capture {
    /// Take the captured end of a body whose middle was omitted.
    pub(crate) fn take_tail(&mut self) -> Option<crate::interceptor::ResponseTail> {
        let bytes = std::mem::take(&mut self.tail);
        (self.omitted > 0 || !bytes.is_empty()).then(|| crate::interceptor::ResponseTail {
            omitted: std::mem::take(&mut self.omitted),
            bytes,
        })
    }
}

/// Bytes of a response's end a [`TappedBody`] keeps past its head.
pub(crate) const TAIL_CAP: usize = 1024 * 1024;
/// Most bytes the tail keeps so that it starts at the beginning of the line
/// (SSE or NDJSON event) holding its first byte: a terminal event up to this
/// size survives whole.
pub(crate) const TAIL_LINE_CAP: usize = 8 * 1024 * 1024;

/// The bounded end of a body, past its head.
struct Tail {
    cap: usize,
    line_cap: usize,
    buf: Vec<u8>,
    /// Bytes after the head that were discarded.
    dropped: u64,
    /// `buf` starts at a line boundary (or right after the head while
    /// nothing was dropped).
    aligned: bool,
    /// Index in `buf` of its last `\n`.
    last_newline: Option<usize>,
    /// Trim again once `buf` is longer (amortizes the copying).
    next_trim: usize,
    /// Largest `buf` held, for the memory-bound tests.
    #[cfg(test)]
    max_held: usize,
}

impl Tail {
    fn new(cap: usize, line_cap: usize) -> Self {
        Self {
            cap,
            line_cap: line_cap.max(cap),
            buf: Vec::new(),
            dropped: 0,
            aligned: true,
            last_newline: None,
            next_trim: 2 * cap,
            #[cfg(test)]
            max_held: 0,
        }
    }

    fn push(&mut self, data: &[u8]) {
        let base = self.buf.len();
        if let Some(i) = data.iter().rposition(|b| *b == b'\n') {
            self.last_newline = Some(base + i);
        }
        self.buf.extend_from_slice(data);
        #[cfg(test)]
        {
            self.max_held = self.max_held.max(self.buf.len());
        }
        if self.buf.len() > self.next_trim {
            self.trim();
        }
    }

    /// Keep the last `cap` bytes, extended back to the start of the line
    /// holding the first of them when that line starts within `line_cap`
    /// bytes of the end.
    fn trim(&mut self) {
        let len = self.buf.len();
        let target = len.saturating_sub(self.cap);
        let floor = len.saturating_sub(self.line_cap);
        // Cutting at the floor keeps the buffer's alignment only when
        // nothing is cut.
        let at_floor = (floor, floor == 0 && self.aligned);
        let (cut, aligned) = match self.last_newline {
            None => at_floor,
            Some(n) if n < floor => at_floor,
            Some(n) if n < target => (n + 1, true),
            Some(_) => match self.buf[floor..target].iter().rposition(|b| *b == b'\n') {
                Some(i) => (floor + i + 1, true),
                None => at_floor,
            },
        };
        if cut > 0 {
            self.buf.drain(..cut);
            self.dropped += cut as u64;
            self.aligned = aligned;
            self.last_newline = self.last_newline.and_then(|n| n.checked_sub(cut));
        }
        self.next_trim = self.buf.len() + self.cap;
    }

    /// The kept end and the bytes omitted before it. A tail that was cut
    /// inside a line starts at the next line, if it has one.
    fn into_parts(mut self) -> (Vec<u8>, u64) {
        if self.dropped > 0 && !self.aligned {
            if let Some(i) = self.buf.iter().position(|b| *b == b'\n') {
                self.buf.drain(..=i);
                self.dropped += i as u64 + 1;
            }
        }
        (self.buf, self.dropped)
    }
}

type Completion = Box<dyn FnOnce(Capture) + Send>;

/// Wraps an inner body, teeing data frames into a bounded capture: the first
/// `cap` bytes and, with [`with_tail`](Self::with_tail), the end of the body.
/// When the inner body ends, `on_end` is invoked exactly once.
pub struct TappedBody<B> {
    inner: Option<B>,
    buf: Vec<u8>,
    cap: usize,
    tail: Option<Tail>,
    usage: Option<crate::usage_stream::UsageScanner>,
    finished: bool,
    on_end: Option<Completion>,
}

impl<B> TappedBody<B> {
    /// Capture the first `cap` bytes only.
    pub fn new(inner: B, cap: usize, on_end: Completion) -> Self
    where
        B: Body,
    {
        let finished = inner.is_end_stream();
        Self {
            inner: Some(inner),
            buf: Vec::new(),
            cap,
            tail: None,
            usage: None,
            finished,
            on_end: Some(on_end),
        }
    }

    /// Capture the first `cap` bytes and the end of the body (see
    /// [`Capture`]); at most `cap` + [`TAIL_LINE_CAP`] + [`TAIL_CAP`] bytes
    /// plus one frame are held.
    pub fn with_tail(inner: B, cap: usize, on_end: Completion) -> Self
    where
        B: Body,
    {
        Self::with_tail_limits(inner, cap, TAIL_CAP, TAIL_LINE_CAP, on_end)
    }

    pub(crate) fn with_tail_limits(
        inner: B,
        cap: usize,
        tail_cap: usize,
        line_cap: usize,
        on_end: Completion,
    ) -> Self
    where
        B: Body,
    {
        let mut tapped = Self::new(inner, cap, on_end);
        tapped.tail = (tail_cap > 0).then(|| Tail::new(tail_cap, line_cap));
        // The terminal event may outgrow the end: its usage is read as it
        // passes.
        tapped.usage = Some(crate::usage_stream::UsageScanner::new());
        tapped
    }

    fn record(&mut self, data: &[u8]) {
        if let Some(usage) = &mut self.usage {
            usage.push(data);
        }
        let take = self.cap.saturating_sub(self.buf.len()).min(data.len());
        if take > 0 {
            self.buf.extend_from_slice(&data[..take]);
        }
        if let Some(tail) = &mut self.tail {
            if take < data.len() {
                tail.push(&data[take..]);
            }
        }
    }

    fn fire(&mut self, error: Option<String>) {
        if let Some(cb) = self.on_end.take() {
            let mut bytes = std::mem::take(&mut self.buf);
            let (mut tail, omitted) = match self.tail.take() {
                Some(tail) => tail.into_parts(),
                None => (Vec::new(), 0),
            };
            if omitted == 0 {
                // Nothing omitted: head and tail are the whole body.
                bytes.append(&mut tail);
            }
            let usage = self
                .usage
                .take()
                .and_then(crate::usage_stream::UsageScanner::finish);
            cb(Capture {
                bytes,
                error,
                tail,
                omitted,
                usage,
            });
        }
    }

    #[cfg(test)]
    fn max_tail_held(&self) -> usize {
        self.tail.as_ref().map_or(0, |t| t.max_held)
    }
}

impl<B> Body for TappedBody<B>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match Pin::new(self.inner.as_mut().expect("body exists until drop")).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    // A cheap clone of the shared bytes, to borrow `self`.
                    let data = data.clone();
                    self.record(&data);
                }
                if self.inner.as_ref().is_some_and(Body::is_end_stream) {
                    self.finished = true;
                    self.fire(None);
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(e))) => {
                // On error we still fire so the exchange is recorded with what we have.
                let error = std::io::Error::other(e.into());
                self.fire(Some(format!("Response body failed: {error}")));
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                self.finished = true;
                self.fire(None);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.as_ref().is_none_or(Body::is_end_stream)
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.as_ref().map(Body::size_hint).unwrap_or_default()
    }
}

impl<B> Drop for TappedBody<B> {
    fn drop(&mut self) {
        // If the client hangs up mid-stream, still record what we captured.
        // Drop nested taps first so their captured wire data is available to
        // the outer completion callback, including on client cancellation.
        drop(self.inner.take());
        self.fire((!self.finished).then(|| {
            "Response delivery interrupted before completion: downstream body closed".into()
        }));
    }
}

/// Whether a request body carries a non-identity `Content-Encoding`.
pub(crate) fn content_encoded(headers: &http::HeaderMap) -> bool {
    headers
        .get_all(http::header::CONTENT_ENCODING)
        .iter()
        .any(|value| match value.to_str() {
            Ok(value) => value.split(',').any(|encoding| {
                let encoding = encoding.trim();
                !encoding.is_empty() && !encoding.eq_ignore_ascii_case("identity")
            }),
            // Not even text: certainly not an identity body.
            Err(_) => true,
        })
}

#[derive(Default)]
struct RequestTapState {
    head: Vec<u8>,
    total: u64,
    complete: bool,
    error: Option<String>,
}

/// Reads what a [`RequestTap`] has seen of a request body so far.
#[derive(Clone)]
pub(crate) struct RequestCaptureHandle {
    state: std::sync::Arc<parking_lot::Mutex<RequestTapState>>,
    encoded: bool,
}

impl RequestCaptureHandle {
    pub(crate) fn snapshot(&self) -> crate::interceptor::BoundedCapture {
        let state = self.state.lock();
        crate::interceptor::BoundedCapture {
            head: state.head.clone(),
            total_len: state.total,
            truncated: state.total > state.head.len() as u64,
            complete: state.complete,
            encoded: self.encoded,
            withheld: false,
        }
    }

    /// Why reading the client's body failed, if it did.
    pub(crate) fn error(&self) -> Option<String> {
        self.state.lock().error.clone()
    }

    fn total(&self) -> u64 {
        self.state.lock().total
    }
}

/// A request body forwarded as it arrives. Nothing is retained except the
/// first `cap` bytes and a running length, which the [`RequestCaptureHandle`]
/// exposes to the monitor. Size hint and end-of-stream are the inner body's,
/// so the forwarded request keeps its framing.
pub(crate) struct RequestTap<B> {
    inner: Option<B>,
    cap: usize,
    state: std::sync::Arc<parking_lot::Mutex<RequestTapState>>,
    /// What to do with an unfinished body that is dropped.
    abandoned: Option<Abandon<B>>,
}

/// Takes over a body a request tap dropped before it ended.
type Abandon<B> = Box<dyn FnOnce(B) + Send>;

/// Most bytes of an abandoned pass-through upload read and discarded.
const ABANDONED_DRAIN_LIMIT: u64 = crate::websocket::MAX_MESSAGE as u64;
/// Most bytes read and discarded from a refused upload, and the longest time
/// spent doing so.
pub(crate) const DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

impl<B: Body> RequestTap<B> {
    pub(crate) fn new(inner: B, cap: usize, encoded: bool) -> (Self, RequestCaptureHandle) {
        let state = std::sync::Arc::new(parking_lot::Mutex::new(RequestTapState {
            // An empty body is never polled by the sender.
            complete: inner.is_end_stream(),
            ..Default::default()
        }));
        let handle = RequestCaptureHandle {
            state: state.clone(),
            encoded,
        };
        (
            Self {
                inner: Some(inner),
                cap,
                state,
                abandoned: None,
            },
            handle,
        )
    }

    fn inner(self: Pin<&mut Self>) -> Pin<&mut B>
    where
        B: Unpin,
    {
        Pin::new(
            self.get_mut()
                .inner
                .as_mut()
                .expect("body exists until drop"),
        )
    }
}

impl<B> RequestTap<B>
where
    B: Body<Data = Bytes> + Send + Unpin + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    /// When the tap is dropped before the client's body ended (the upstream
    /// could not be reached, or answered and closed mid-upload), keep
    /// reading the rest in the background, bounded in size and time, so the
    /// client finishes sending and can read the answer instead of seeing its
    /// connection reset.
    pub(crate) fn drain_when_abandoned(self) -> Self {
        let mut tap = self;
        tap.abandoned = Some(Box::new(|body: B| {
            let Ok(runtime) = tokio::runtime::Handle::try_current() else {
                return;
            };
            runtime.spawn(async move {
                let _ =
                    tokio::time::timeout(DRAIN_TIMEOUT, discard(body, ABANDONED_DRAIN_LIMIT)).await;
            });
        }));
        tap
    }
}

impl<B> RequestTap<B> {
    fn take_abandoned(&mut self) -> Option<(B, Abandon<B>)> {
        let state = self.state.lock();
        if state.complete || state.error.is_some() {
            return None;
        }
        drop(state);
        let callback = self.abandoned.take()?;
        Some((self.inner.take()?, callback))
    }
}

impl<B> Drop for RequestTap<B> {
    fn drop(&mut self) {
        if let Some((body, drain)) = self.take_abandoned() {
            drain(body);
        }
    }
}

/// Read and discard a body, up to `limit` bytes.
async fn discard<B>(body: B, limit: u64)
where
    B: Body<Data = Bytes> + Unpin,
{
    use http_body_util::BodyExt;
    let mut body = body;
    let mut total = 0u64;
    while total <= limit {
        match body.frame().await {
            Some(Ok(frame)) => total += frame.data_ref().map_or(0, |d| d.len() as u64),
            Some(Err(_)) | None => break,
        }
    }
}

impl<B> Body for RequestTap<B>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    type Data = Bytes;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let polled = self.as_mut().inner().poll_frame(cx);
        match polled {
            Poll::Ready(Some(Ok(frame))) => {
                let ended = self.inner.as_ref().is_none_or(Body::is_end_stream);
                let mut state = self.state.lock();
                if let Some(data) = frame.data_ref() {
                    state.total += data.len() as u64;
                    let room = self.cap.saturating_sub(state.head.len());
                    state.head.extend_from_slice(&data[..room.min(data.len())]);
                }
                state.complete |= ended;
                drop(state);
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(error))) => {
                let error = error.into();
                self.state.lock().error = Some(error.to_string());
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                self.state.lock().complete = true;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.as_ref().is_none_or(Body::is_end_stream)
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.as_ref().map(Body::size_hint).unwrap_or_default()
    }
}

/// Read and discard a request body that is being refused, so the client can
/// finish sending and then read the refusal instead of seeing its connection
/// reset mid-upload. Holds only the bounded capture; gives up after `limit`
/// bytes or [`DRAIN_TIMEOUT`].
pub(crate) async fn drain<B>(
    body: B,
    cap: usize,
    limit: u64,
    encoded: bool,
) -> crate::interceptor::BoundedCapture
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    drain_within(body, cap, limit, encoded, DRAIN_TIMEOUT).await
}

pub(crate) async fn drain_within<B>(
    body: B,
    cap: usize,
    limit: u64,
    encoded: bool,
    timeout: std::time::Duration,
) -> crate::interceptor::BoundedCapture
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    use http_body_util::BodyExt;
    let (mut tap, handle) = RequestTap::new(body, cap, encoded);
    let _ = tokio::time::timeout(timeout, async {
        while handle.total() <= limit {
            match tap.frame().await {
                Some(Ok(_)) => {}
                Some(Err(_)) | None => break,
            }
        }
    })
    .await;
    handle.snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::{BodyExt, Full};
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn request_tap_forwards_everything_and_keeps_a_bounded_head() {
        let chunks =
            (0..64u8).map(|i| Ok::<_, std::io::Error>(Frame::data(Bytes::from(vec![i; 1024]))));
        let body = http_body_util::StreamBody::new(futures::stream::iter(chunks));
        let (tap, handle) = RequestTap::new(body, 1500, false);
        let before = handle.snapshot();
        assert_eq!((before.total_len, before.complete), (0, false));

        let forwarded = tap.collect().await.unwrap().to_bytes();
        assert_eq!(forwarded.len(), 64 * 1024);
        let capture = handle.snapshot();
        // Exactly the cap: one whole chunk and the first part of the next.
        assert_eq!(capture.head.len(), 1500);
        assert_eq!(&capture.head[..], &forwarded[..1500]);
        assert_eq!(capture.total_len, 64 * 1024);
        assert!(capture.truncated && capture.complete && !capture.encoded);
        assert!(capture.inspectable().is_none());
        assert!(handle.error().is_none());
    }

    #[tokio::test]
    async fn request_tap_keeps_the_size_hint_and_reports_small_bodies_whole() {
        let (tap, handle) = RequestTap::new(Full::new(Bytes::from_static(b"{\"a\":1}")), 7, false);
        assert_eq!(tap.size_hint().exact(), Some(7));
        assert_eq!(
            tap.collect().await.unwrap().to_bytes().as_ref(),
            b"{\"a\":1}"
        );
        let capture = handle.snapshot();
        assert!(!capture.truncated && capture.complete);
        assert_eq!(capture.inspectable(), Some(&b"{\"a\":1}"[..]));

        // An empty body is complete without ever being polled.
        let (tap, handle) = RequestTap::new(Full::new(Bytes::new()), 16, false);
        assert!(tap.is_end_stream());
        assert!(handle.snapshot().complete);
        assert!(handle.snapshot().inspectable().is_none());

        // A zero cap records the length only.
        let (tap, handle) = RequestTap::new(Full::new(Bytes::from_static(b"abc")), 0, true);
        tap.collect().await.unwrap();
        let capture = handle.snapshot();
        assert!(capture.head.is_empty() && capture.truncated && capture.encoded);
        assert_eq!(capture.total_len, 3);
    }

    #[tokio::test]
    async fn request_tap_records_a_failed_client_body() {
        let chunks = vec![
            Ok(Frame::data(Bytes::from_static(b"partial"))),
            Err(std::io::Error::other("client went away")),
        ];
        let body = http_body_util::StreamBody::new(futures::stream::iter(chunks));
        let (tap, handle) = RequestTap::new(body, 64, false);
        assert!(tap.collect().await.is_err());
        let capture = handle.snapshot();
        assert_eq!(capture.head, b"partial");
        assert!(!capture.complete);
        assert!(capture.inspectable().is_none());
        assert_eq!(handle.error().as_deref(), Some("client went away"));
    }

    #[tokio::test]
    async fn drain_discards_the_body_and_stops_at_the_limit() {
        let chunks =
            (0..100u8).map(|_| Ok::<_, std::io::Error>(Frame::data(Bytes::from(vec![b'x'; 1000]))));
        let body = http_body_util::StreamBody::new(futures::stream::iter(chunks));
        let capture = drain(body, 10, u64::MAX, false).await;
        assert_eq!((capture.head.len(), capture.total_len), (10, 100_000));
        assert!(capture.complete);

        let chunks =
            (0..100u8).map(|_| Ok::<_, std::io::Error>(Frame::data(Bytes::from(vec![b'x'; 1000]))));
        let body = http_body_util::StreamBody::new(futures::stream::iter(chunks));
        let capture = drain(body, 10, 2500, false).await;
        // Reading stops with the first chunk that crosses the limit.
        assert_eq!(capture.total_len, 3000);
        assert!(!capture.complete);
    }

    #[test]
    fn content_encoded_ignores_identity() {
        let mut headers = http::HeaderMap::new();
        assert!(!content_encoded(&headers));
        headers.insert(http::header::CONTENT_ENCODING, "identity".parse().unwrap());
        assert!(!content_encoded(&headers));
        headers.insert(
            http::header::CONTENT_ENCODING,
            "Identity, zstd".parse().unwrap(),
        );
        assert!(content_encoded(&headers));
        headers.insert(
            http::header::CONTENT_ENCODING,
            http::HeaderValue::from_bytes(b"\xff").unwrap(),
        );
        assert!(content_encoded(&headers));
    }

    /// hyper derives the outgoing request framing from the body's size hint:
    /// an exact hint is sent as Content-Length, an unknown one as chunked.
    /// Pass-through relies on this to keep the client's Content-Length.
    #[tokio::test]
    async fn hyper_keeps_content_length_for_an_exact_size_hint() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        struct Sized(Option<Bytes>, bool);
        impl Body for Sized {
            type Data = Bytes;
            type Error = std::io::Error;
            fn poll_frame(
                mut self: Pin<&mut Self>,
                _: &mut Context<'_>,
            ) -> Poll<Option<Result<Frame<Bytes>, std::io::Error>>> {
                Poll::Ready(self.0.take().map(|data| Ok(Frame::data(data))))
            }
            fn size_hint(&self) -> http_body::SizeHint {
                match (&self.0, self.1) {
                    (Some(data), true) => http_body::SizeHint::with_exact(data.len() as u64),
                    (None, true) => http_body::SizeHint::with_exact(0),
                    _ => http_body::SizeHint::default(),
                }
            }
        }

        for exact in [true, false] {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (mut tcp, _) = listener.accept().await.unwrap();
                let mut seen = Vec::new();
                let mut buffer = [0u8; 1024];
                // Both framings end with the body's last bytes.
                while !(seen.ends_with(b"hello") || seen.ends_with(b"0\r\n\r\n")) {
                    let n = tcp.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    seen.extend_from_slice(&buffer[..n]);
                }
                tcp.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                    .await
                    .unwrap();
                String::from_utf8(seen).unwrap().to_ascii_lowercase()
            });
            let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            let (mut sender, conn) =
                hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(tcp))
                    .await
                    .unwrap();
            tokio::spawn(conn);
            let (tap, _) =
                RequestTap::new(Sized(Some(Bytes::from_static(b"hello")), exact), 0, false);
            let request = hyper::Request::builder()
                .method("POST")
                .uri("/upload")
                .header("host", "localhost")
                .body(tap)
                .unwrap();
            assert_eq!(sender.send_request(request).await.unwrap().status(), 200);
            let seen = server.await.unwrap();
            if exact {
                assert!(seen.contains("content-length: 5\r\n"), "{seen}");
                assert!(!seen.contains("transfer-encoding"), "{seen}");
                assert!(seen.ends_with("\r\n\r\nhello"), "{seen}");
            } else {
                assert!(seen.contains("transfer-encoding: chunked\r\n"), "{seen}");
                assert!(!seen.contains("content-length"), "{seen}");
                assert!(seen.ends_with("5\r\nhello\r\n0\r\n\r\n"), "{seen}");
            }
        }
    }

    #[tokio::test]
    async fn streams_through_and_captures_within_cap() {
        let captured: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
        let sink = captured.clone();

        let inner = Full::<Bytes>::new(Bytes::from_static(b"hello world"));
        let tapped = TappedBody::new(
            inner,
            5, // cap below body length
            Box::new(move |capture| *sink.lock().unwrap() = Some(capture.bytes)),
        );

        let collected = tapped.collect().await.unwrap().to_bytes();
        // Downstream sees the FULL body...
        assert_eq!(&collected[..], b"hello world");
        // ...but only the capped prefix is captured.
        assert_eq!(captured.lock().unwrap().as_deref(), Some(&b"hello"[..]));
    }

    /// Collect `frames` through a tail-keeping tap; returns the capture and
    /// the largest tail it held.
    async fn tapped(
        frames: impl Iterator<Item = Bytes> + Send + 'static,
        cap: usize,
        tail_cap: usize,
        line_cap: usize,
    ) -> (Capture, Vec<u8>, usize) {
        let captured: Arc<Mutex<Option<Capture>>> = Arc::new(Mutex::new(None));
        let sink = captured.clone();
        let body = http_body_util::StreamBody::new(futures::stream::iter(
            frames.map(|b| Ok::<_, std::io::Error>(Frame::data(b))),
        ));
        let mut tapped = TappedBody::with_tail_limits(
            body,
            cap,
            tail_cap,
            line_cap,
            Box::new(move |capture| *sink.lock().unwrap() = Some(capture)),
        );
        let mut forwarded = Vec::new();
        let mut max = 0;
        while let Some(frame) = tapped.frame().await {
            forwarded.extend_from_slice(frame.unwrap().data_ref().unwrap());
            max = max.max(tapped.max_tail_held());
        }
        drop(tapped);
        let capture = captured.lock().unwrap().take().unwrap();
        (capture, forwarded, max)
    }

    fn sse_frames(events: usize, last: &str) -> Vec<Bytes> {
        let mut frames: Vec<Bytes> = (0..events)
            .map(|i| Bytes::from(format!("data: {{\"delta\":\"event {i:06}\"}}\n\n")))
            .collect();
        frames.push(Bytes::from(format!("data: {last}\n\n")));
        frames
    }

    #[tokio::test]
    async fn the_tail_keeps_the_terminal_event_of_a_long_stream() {
        let frames = sse_frames(5000, "{\"usage\":{\"total\":42}}");
        let (capture, forwarded, max) = tapped(frames.into_iter(), 1000, 2000, 8000).await;
        assert!(capture.error.is_none());
        assert_eq!(capture.bytes, &forwarded[..1000]);
        assert!(capture.omitted > 0);
        assert_eq!(
            1000 + capture.omitted + capture.tail.len() as u64,
            forwarded.len() as u64
        );
        assert!(forwarded.ends_with(&capture.tail));
        // Cut at a line start, and holds the terminal event.
        assert!(capture.tail.starts_with(b"data: ") || capture.tail.starts_with(b"\n"));
        assert!(capture
            .tail
            .ends_with(b"data: {\"usage\":{\"total\":42}}\n\n"));
        assert!(capture.tail.len() >= 2000 && capture.tail.len() <= 2 * 2000 + 64);
        // Bounded: the kept tail, one trim interval and one frame.
        assert!(max <= 2 * 2000 + 64, "{max}");
        let (omitted, tail) = (capture.omitted, capture.tail.clone());
        let mut capture = capture;
        assert_eq!(
            capture.take_tail(),
            Some(crate::interceptor::ResponseTail {
                omitted,
                bytes: tail
            })
        );
    }

    #[tokio::test]
    async fn a_body_that_fits_head_and_tail_is_captured_whole() {
        let frames = sse_frames(100, "{\"usage\":{}}");
        let (capture, forwarded, _) = tapped(frames.into_iter(), 1000, 4096, 8192).await;
        assert_eq!(capture.bytes, forwarded);
        assert!(capture.tail.is_empty() && capture.omitted == 0);
        let mut capture = capture;
        assert!(capture.take_tail().is_none());
    }

    #[tokio::test]
    async fn a_long_terminal_event_survives_whole_up_to_the_line_cap() {
        let big = format!("{{\"text\":\"{}\",\"usage\":1}}", "y".repeat(5000));
        let frames = sse_frames(3000, &big);
        let (capture, forwarded, max) = tapped(frames.into_iter(), 500, 1000, 8000).await;
        let expected = format!("data: {big}\n\n");
        assert!(
            capture.tail.ends_with(expected.as_bytes()),
            "line kept whole"
        );
        assert!(forwarded.ends_with(&capture.tail));
        assert!(max <= 8000 + 1000 + expected.len(), "{max}");

        // Above the line cap the line is cut; the tail then starts at the
        // next line.
        let huge = format!("{{\"text\":\"{}\"}}", "z".repeat(20_000));
        let mut frames = sse_frames(10, "{}");
        frames.push(Bytes::from(format!("data: {huge}\n")));
        frames.push(Bytes::from_static(b"data: {\"usage\":7}\n\n"));
        let (capture, forwarded, _) = tapped(frames.into_iter(), 100, 1000, 4000).await;
        assert_eq!(capture.tail, b"data: {\"usage\":7}\n\n");
        assert_eq!(
            100 + capture.omitted + capture.tail.len() as u64,
            forwarded.len() as u64
        );
    }

    #[tokio::test]
    async fn the_capture_of_a_64_mib_stream_stays_bounded() {
        const FRAME: usize = 64 * 1024;
        const TOTAL: usize = 64 * 1024 * 1024;
        let line = "data: {\"type\":\"delta\",\"delta\":\"abcdefgh\"}\n\n";
        let block: String = line.repeat(FRAME / line.len());
        let block = Bytes::from(block);
        let frames = (0..TOTAL / block.len() + 1)
            .map(move |_| block.clone())
            .chain(std::iter::once(Bytes::from_static(
                b"data: {\"type\":\"response.completed\",\"usage\":9}\n\n",
            )));
        let (capture, forwarded, max) = tapped(frames, 1024 * 1024, TAIL_CAP, TAIL_LINE_CAP).await;
        assert!(forwarded.len() >= TOTAL);
        assert_eq!(capture.bytes.len(), 1024 * 1024);
        assert!(capture.tail.len() >= TAIL_CAP && capture.tail.len() <= 2 * TAIL_CAP + FRAME);
        assert!(capture
            .tail
            .ends_with(b"data: {\"type\":\"response.completed\",\"usage\":9}\n\n"));
        assert!(max <= 2 * TAIL_CAP + 2 * FRAME, "{max}");
    }

    #[tokio::test]
    async fn fires_on_end_exactly_once() {
        let count = Arc::new(Mutex::new(0));
        let c = count.clone();
        let inner = Full::<Bytes>::new(Bytes::from_static(b"x"));
        let tapped = TappedBody::new(inner, 1024, Box::new(move |_| *c.lock().unwrap() += 1));
        let _ = tapped.collect().await.unwrap();
        assert_eq!(*count.lock().unwrap(), 1);
    }
}
