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

/// Wraps an inner body, teeing data frames into a capped buffer. When the inner
/// body ends, `on_end` is invoked exactly once with the captured bytes.
pub struct Capture {
    pub bytes: Vec<u8>,
    pub error: Option<String>,
}

type Completion = Box<dyn FnOnce(Capture) + Send>;

pub struct TappedBody<B> {
    inner: Option<B>,
    buf: Vec<u8>,
    cap: usize,
    finished: bool,
    on_end: Option<Completion>,
}

impl<B> TappedBody<B> {
    pub fn new(inner: B, cap: usize, on_end: Completion) -> Self
    where
        B: Body,
    {
        let finished = inner.is_end_stream();
        Self {
            inner: Some(inner),
            buf: Vec::new(),
            cap,
            finished,
            on_end: Some(on_end),
        }
    }

    fn fire(&mut self, error: Option<String>) {
        if let Some(cb) = self.on_end.take() {
            cb(Capture {
                bytes: std::mem::take(&mut self.buf),
                error,
            });
        }
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
                    let remaining = self.cap.saturating_sub(self.buf.len());
                    if remaining > 0 {
                        let take = remaining.min(data.len());
                        self.buf.extend_from_slice(&data[..take]);
                    }
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
    abandoned: Option<Box<dyn FnOnce(B) + Send>>,
}

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
    fn take_abandoned(&mut self) -> Option<(B, Box<dyn FnOnce(B) + Send>)> {
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
