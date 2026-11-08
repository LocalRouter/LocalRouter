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
        self.fire((!self.finished).then(|| "Response body was dropped before completion".into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::{BodyExt, Full};
    use std::sync::{Arc, Mutex};

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
