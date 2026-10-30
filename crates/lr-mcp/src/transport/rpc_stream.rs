//! Long-lived JSON-RPC requests that produce many messages.
//!
//! An MCP Events `events/stream` request (SEP-3415) is answered with any
//! number of notifications tagged with
//! `_meta["io.modelcontextprotocol/subscriptionId"] == <request id>` and,
//! eventually, one final result. [`RpcStream`] delivers exactly those
//! messages for one request, with backpressure (a bounded channel), no total
//! timeout, and cancellation when it is dropped before the final result.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures_util::Stream;
use parking_lot::RwLock;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::protocol::{JsonRpcNotification, JsonRpcResponse};
use lr_types::{AppError, AppResult};

/// `_meta` key correlating a notification with the request it belongs to.
pub const SUBSCRIPTION_ID_META_KEY: &str = "io.modelcontextprotocol/subscriptionId";

/// Messages buffered per stream before the transport stops reading.
pub const DEFAULT_RPC_STREAM_BUFFER: usize = 256;

/// One message of a long-lived request.
#[derive(Debug, Clone)]
pub enum RpcStreamMessage {
    /// A notification correlated with the request.
    Notification(JsonRpcNotification),
    /// The final result (or error). Always the last message.
    Result(JsonRpcResponse),
}

/// The subscription id a notification is tagged with, if any.
pub fn subscription_id(notification: &JsonRpcNotification) -> Option<&Value> {
    notification
        .params
        .as_ref()?
        .get("_meta")?
        .get(SUBSCRIPTION_ID_META_KEY)
}

/// Lookup key for a JSON-RPC id (`1` and `"1"` are different ids).
pub(crate) fn id_key(id: &Value) -> String {
    match id {
        Value::Null => "__null_id__".to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("\"{}\"", s),
        _ => id.to_string(),
    }
}

/// Senders of open streams, keyed by the wire id of their request.
pub(crate) type StreamRegistry = Arc<RwLock<HashMap<String, StreamSink>>>;

/// The transport side of one [`RpcStream`].
#[derive(Clone)]
pub(crate) struct StreamSink {
    sender: mpsc::Sender<RpcStreamMessage>,
    /// The caller's request id, restored on everything delivered.
    original_id: Value,
}

impl StreamSink {
    pub(crate) fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    /// Deliver a correlated notification, waiting while the consumer's buffer
    /// is full. Returns `false` when the consumer is gone.
    pub(crate) async fn notify(&self, mut notification: JsonRpcNotification) -> bool {
        if let Some(meta) = notification
            .params
            .as_mut()
            .and_then(|p| p.get_mut("_meta"))
            .and_then(Value::as_object_mut)
        {
            meta.insert(
                SUBSCRIPTION_ID_META_KEY.to_string(),
                self.original_id.clone(),
            );
        }
        self.sender
            .send(RpcStreamMessage::Notification(notification))
            .await
            .is_ok()
    }

    /// Deliver the final result.
    pub(crate) async fn finish(&self, mut response: JsonRpcResponse) {
        response.id = self.original_id.clone();
        let _ = self.sender.send(RpcStreamMessage::Result(response)).await;
    }
}

/// Remove and return the stream waiting for this response, if any.
pub(crate) fn take_stream_for_response(
    registry: &StreamRegistry,
    response: &JsonRpcResponse,
) -> Option<StreamSink> {
    registry.write().remove(&id_key(&response.id))
}

/// The open stream a notification is correlated with, if any.
pub(crate) fn stream_for_notification(
    registry: &StreamRegistry,
    notification: &JsonRpcNotification,
) -> Option<StreamSink> {
    let id = subscription_id(notification)?;
    registry.read().get(&id_key(id)).cloned()
}

type CancelFn = Box<dyn FnOnce() + Send>;

/// Messages of one long-lived request, ending with its final result.
///
/// Yields `Ok(Notification)` for every correlated notification and one
/// `Ok(Result)` as the last item. If the connection ends first, the last item
/// is an `Err` instead. Dropping the stream before the final result cancels
/// the request upstream (`notifications/cancelled` on stdio; the HTTP
/// response body is dropped on HTTP transports).
pub struct RpcStream {
    request_id: Value,
    receiver: mpsc::Receiver<RpcStreamMessage>,
    cancel: Option<CancelFn>,
    done: bool,
}

impl RpcStream {
    /// Create a stream and the sink a transport delivers into. `cancel` runs
    /// if the stream is dropped before the final result.
    pub(crate) fn channel(
        request_id: Value,
        buffer: usize,
        cancel: Option<CancelFn>,
    ) -> (Self, StreamSink) {
        let (sender, receiver) = mpsc::channel(buffer.max(1));
        (
            Self {
                request_id: request_id.clone(),
                receiver,
                cancel,
                done: false,
            },
            StreamSink {
                sender,
                original_id: request_id,
            },
        )
    }

    pub(crate) fn set_cancel(&mut self, cancel: CancelFn) {
        self.cancel = Some(cancel);
    }

    /// The caller's request id (also the subscription id on notifications).
    pub fn request_id(&self) -> &Value {
        &self.request_id
    }

    /// Cancel the request now (same as dropping the stream).
    pub fn cancel(mut self) {
        self.run_cancel();
    }

    fn run_cancel(&mut self) {
        if !self.done {
            self.done = true;
            if let Some(cancel) = self.cancel.take() {
                cancel();
            }
        }
    }
}

impl Stream for RpcStream {
    type Item = AppResult<RpcStreamMessage>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done {
            return Poll::Ready(None);
        }
        match self.receiver.poll_recv(cx) {
            Poll::Ready(Some(message)) => {
                if matches!(message, RpcStreamMessage::Result(_)) {
                    self.done = true;
                    self.cancel = None;
                }
                Poll::Ready(Some(Ok(message)))
            }
            Poll::Ready(None) => {
                self.done = true;
                self.cancel = None;
                Poll::Ready(Some(Err(AppError::Mcp(
                    "Upstream connection ended before the stream completed".to_string(),
                ))))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for RpcStream {
    fn drop(&mut self) {
        self.run_cancel();
    }
}

/// Splits an SSE byte stream into the `data` payloads of its events.
#[derive(Default)]
pub(crate) struct SseEventSplitter {
    buffer: Vec<u8>,
}

/// Largest SSE event accepted before a terminator (matches the other paths).
pub(crate) const MAX_SSE_EVENT_BYTES: usize = 16 * 1024 * 1024;

impl SseEventSplitter {
    /// Append a chunk and return the `data` of every complete event.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> AppResult<Vec<String>> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        loop {
            let boundary = self
                .buffer
                .windows(2)
                .position(|w| w == b"\n\n")
                .map(|i| (i, 2))
                .into_iter()
                .chain(
                    self.buffer
                        .windows(4)
                        .position(|w| w == b"\r\n\r\n")
                        .map(|i| (i, 4)),
                )
                .min_by_key(|(i, _)| *i);
            let Some((end, delimiter)) = boundary else {
                break;
            };
            let raw: Vec<u8> = self.buffer.drain(..end + delimiter).collect();
            let event = String::from_utf8_lossy(&raw);
            let data = event
                .lines()
                .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
                .collect::<Vec<_>>()
                .join("\n");
            if !data.is_empty() {
                events.push(data);
            }
        }
        if self.buffer.len() > MAX_SSE_EVENT_BYTES {
            return Err(AppError::Mcp("MCP SSE event exceeds 16 MiB".into()));
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use serde_json::json;

    fn tagged(id: Value) -> JsonRpcNotification {
        JsonRpcNotification {
            jsonrpc: "2.0".into(),
            method: "notifications/events/event".into(),
            params: Some(json!({ "_meta": { SUBSCRIPTION_ID_META_KEY: id } })),
        }
    }

    #[test]
    fn subscription_ids_distinguish_numbers_and_strings() {
        assert_eq!(subscription_id(&tagged(json!(7))), Some(&json!(7)));
        assert_ne!(id_key(&json!(1)), id_key(&json!("1")));
    }

    #[tokio::test]
    async fn stream_ends_with_result_and_restores_ids() {
        let (mut stream, sink) = RpcStream::channel(json!("caller-1"), 4, None);
        assert!(sink.notify(tagged(json!(42))).await);
        sink.finish(JsonRpcResponse::success(json!(42), json!({"done": true})))
            .await;
        match stream.next().await.unwrap().unwrap() {
            RpcStreamMessage::Notification(n) => {
                assert_eq!(subscription_id(&n), Some(&json!("caller-1")))
            }
            other => panic!("unexpected {other:?}"),
        }
        match stream.next().await.unwrap().unwrap() {
            RpcStreamMessage::Result(r) => assert_eq!(r.id, json!("caller-1")),
            other => panic!("unexpected {other:?}"),
        }
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn closed_connection_is_an_error_not_a_result() {
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = cancelled.clone();
        let (mut stream, sink) = RpcStream::channel(
            json!(1),
            4,
            Some(Box::new(move || {
                flag.store(true, std::sync::atomic::Ordering::SeqCst)
            })),
        );
        drop(sink);
        assert!(stream.next().await.unwrap().is_err());
        assert!(stream.next().await.is_none());
        drop(stream);
        // The connection already ended; there is nothing to cancel
        assert!(!cancelled.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn dropping_before_result_cancels_once() {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = count.clone();
        let (stream, sink) = RpcStream::channel(
            json!(1),
            4,
            Some(Box::new(move || {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            })),
        );
        drop(stream);
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(sink.is_closed());
    }

    #[tokio::test]
    async fn full_buffer_applies_backpressure() {
        let (mut stream, sink) = RpcStream::channel(json!(1), 1, None);
        assert!(sink.notify(tagged(json!(1))).await);
        let blocked = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            sink.notify(tagged(json!(1))),
        )
        .await;
        assert!(blocked.is_err(), "second send must wait for the consumer");
        stream.next().await.unwrap().unwrap();
        assert!(sink.notify(tagged(json!(1))).await);
    }

    #[test]
    fn splitter_handles_split_chunks_and_crlf() {
        let mut splitter = SseEventSplitter::default();
        assert!(splitter
            .push(b"event: message\ndata: {\"a\"")
            .unwrap()
            .is_empty());
        assert_eq!(splitter.push(b":1}\n\n").unwrap(), vec!["{\"a\":1}"]);
        assert_eq!(
            splitter
                .push(b"data: one\r\n\r\n: comment\n\ndata: two\n\n")
                .unwrap(),
            vec!["one", "two"]
        );
    }
}
