//! Restore only after the masked provider body has passed the monitoring tap.
use bytes::Bytes;
use futures::StreamExt;
#[cfg(test)]
use http_body_util::Full;
use http_body_util::{BodyExt, StreamBody};
use lr_secret_scanner::placeholders::Vault;
use std::sync::Arc;

async fn restore_with_completion<B>(
    mut parts: http::response::Parts,
    body: B,
    vault: Arc<Vault>,
    streaming: bool,
    ndjson: bool,
    completed: Arc<std::sync::atomic::AtomicBool>,
) -> Result<
    http::Response<http_body_util::combinators::UnsyncBoxBody<Bytes, std::io::Error>>,
    std::io::Error,
>
where
    B: http_body::Body<Data = Bytes> + Send + Unpin + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send,
{
    if parts
        .headers
        .get(http::header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v != "identity")
    {
        vault.diagnostic("Compressed response cannot be restored; original response delivered");
        let stream = async_stream::try_stream! {
            let mut body=body;
            while let Some(frame)=body.frame().await { yield frame.map_err(|e|std::io::Error::other(e.into()))?; }
            completed.store(true,std::sync::atomic::Ordering::Release);
        };
        let stream: std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<http_body::Frame<Bytes>, std::io::Error>> + Send>,
        > = Box::pin(stream);
        return Ok(http::Response::from_parts(
            parts,
            StreamBody::new(stream).boxed_unsync(),
        ));
    }
    for header in [
        "content-length",
        "content-encoding",
        "etag",
        "content-md5",
        "digest",
        "content-digest",
        "trailer",
    ] {
        parts.headers.remove(header);
    }
    parts.headers.insert(
        http::header::CACHE_CONTROL,
        http::HeaderValue::from_static("no-store"),
    );
    let mode = if ndjson {
        lr_secret_scanner::wire::StreamMode::Ndjson
    } else if streaming {
        lr_secret_scanner::wire::StreamMode::Sse
    } else {
        lr_secret_scanner::wire::StreamMode::Json
    };
    let source = body
        .into_data_stream()
        .map(|chunk| chunk.map_err(|e| std::io::Error::other(e.into())));
    let restored = lr_secret_scanner::wire::restore_stream(source, vault, mode, completed)
        .map(|chunk| chunk.map(http_body::Frame::data));
    Ok(http::Response::from_parts(
        parts,
        StreamBody::new(restored).boxed_unsync(),
    ))
}

/// Capture the masked provider wire, but complete monitoring only after the
/// restored client body succeeds or fails. Restoration errors cannot race a
/// successful upstream tap into an incorrect Complete event.
pub(crate) async fn restore_recorded<B>(
    parts: http::response::Parts,
    body: B,
    vault: Arc<Vault>,
    streaming: bool,
    ndjson: bool,
    on_complete: Box<dyn FnOnce(crate::tap::Capture) + Send>,
) -> Result<
    http::Response<http_body_util::combinators::UnsyncBoxBody<Bytes, std::io::Error>>,
    std::io::Error,
>
where
    B: http_body::Body<Data = Bytes> + Send + Unpin + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send,
{
    let captured = Arc::new(std::sync::Mutex::new(None::<crate::tap::Capture>));
    let wire = captured.clone();
    let tapped = crate::tap::TappedBody::new(
        body,
        1024 * 1024,
        Box::new(move |capture| *wire.lock().expect("wire capture") = Some(capture)),
    );
    let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let response =
        match restore_with_completion(parts, tapped, vault, streaming, ndjson, completed.clone())
            .await
        {
            Ok(response) => response,
            Err(error) => {
                let mut capture =
                    captured
                        .lock()
                        .expect("wire capture")
                        .take()
                        .unwrap_or(crate::tap::Capture {
                            bytes: Vec::new(),
                            error: None,
                        });
                capture.error = Some(format!("Protected response restoration failed: {error}"));
                on_complete(capture);
                return Err(error);
            }
        };
    let (parts, body) = response.into_parts();
    let observed = crate::tap::TappedBody::new(
        body,
        0,
        Box::new(move |outcome| {
            let mut wire =
                captured
                    .lock()
                    .expect("wire capture")
                    .take()
                    .unwrap_or(crate::tap::Capture {
                        bytes: Vec::new(),
                        error: None,
                    });
            let dropped = "Response delivery interrupted before completion: downstream body closed";
            let terminal_delivered = completed.load(std::sync::atomic::Ordering::Acquire);
            if terminal_delivered && wire.error.as_deref() == Some(dropped) {
                wire.error = None;
            }
            if outcome.error.is_some()
                && !(terminal_delivered && outcome.error.as_deref() == Some(dropped))
            {
                wire.error = outcome.error;
            }
            on_complete(wire);
        }),
    );
    Ok(http::Response::from_parts(parts, observed.boxed_unsync()))
}

#[cfg(test)]
async fn restore<B>(
    parts: http::response::Parts,
    body: B,
    vault: Arc<Vault>,
    streaming: bool,
    ndjson: bool,
) -> Result<
    http::Response<http_body_util::combinators::UnsyncBoxBody<Bytes, std::io::Error>>,
    std::io::Error,
>
where
    B: http_body::Body<Data = Bytes> + Send + Unpin + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send,
{
    restore_with_completion(
        parts,
        body,
        vault,
        streaming,
        ndjson,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_types::secret_context::SecretContext;
    use serde_json::json;
    fn vault() -> (Arc<Vault>, Arc<lr_api_keys::MockKeychain>, String) {
        let storage = Arc::new(lr_api_keys::MockKeychain::new());
        let vault = Arc::new(Vault::new(storage.clone(), "cancel-owner", "request", true));
        vault.register("synthetic secret").unwrap();
        let mut value = json!({"content":"synthetic secret"});
        vault.protect(&mut value).unwrap();
        (vault, storage, value["content"].as_str().unwrap().into())
    }
    #[tokio::test]
    async fn monitor_completion_waits_for_restoration_and_distinguishes_terminal_from_abort() {
        for terminal in [false, true] {
            let (vault, _, _) = vault();
            let value = if terminal {
                json!({"type":"response.completed","response":{"id":"terminal","output":[]}})
            } else {
                json!({"type":"response.output_text.delta","item_id":"text","delta":"hello"})
            };
            let wire = format!("data: {value}\n\n");
            let source = futures::stream::iter(vec![Ok::<_, std::io::Error>(
                http_body::Frame::data(Bytes::from(wire.clone())),
            )])
            .chain(futures::stream::pending());
            let response = http::Response::new(StreamBody::new(source));
            let (parts, body) = response.into_parts();
            let captured = Arc::new(std::sync::Mutex::new(None));
            let sink = captured.clone();
            let mut restored = restore_recorded(
                parts,
                body,
                vault,
                true,
                false,
                Box::new(move |v| *sink.lock().unwrap() = Some(v)),
            )
            .await
            .unwrap()
            .into_body();
            assert!(restored.frame().await.unwrap().is_ok());
            drop(restored);
            let result = captured.lock().unwrap().take().unwrap();
            assert_eq!(String::from_utf8(result.bytes).unwrap(), wire);
            assert_eq!(result.error.is_none(),terminal,"a delivered terminal event completes a stream even when the client closes before EOF");
        }
        let (vault, _, _) = vault();
        let wire = "data: {invalid}\n\n";
        let response = http::Response::new(Full::new(Bytes::from(wire)));
        let (parts, body) = response.into_parts();
        let captured = Arc::new(std::sync::Mutex::new(None));
        let sink = captured.clone();
        let body = restore_recorded(
            parts,
            body,
            vault,
            true,
            false,
            Box::new(move |v| *sink.lock().unwrap() = Some(v)),
        )
        .await
        .unwrap()
        .into_body();
        assert_eq!(
            body.collect().await.unwrap().to_bytes().as_ref(),
            wire.as_bytes()
        );
        let capture = captured.lock().unwrap().take().unwrap();
        assert_eq!(capture.bytes, wire.as_bytes());
        assert!(capture.error.is_none());
    }

    #[tokio::test]
    async fn json_completion_is_recorded_before_a_consumer_closes_without_polling_eof() {
        for wire in ["{\"content\":\"complete\"}", "{\"content\":\"partial"] {
            let (vault, _, _) = vault();
            let source = futures::stream::iter(vec![Ok::<_, std::io::Error>(
                http_body::Frame::data(Bytes::from_static(wire.as_bytes())),
            )])
            .chain(futures::stream::pending());
            let response = http::Response::new(StreamBody::new(source));
            let (parts, body) = response.into_parts();
            let captured = Arc::new(std::sync::Mutex::new(None));
            let sink = captured.clone();
            let mut restored = restore_recorded(
                parts,
                body,
                vault,
                false,
                false,
                Box::new(move |v| *sink.lock().unwrap() = Some(v)),
            )
            .await
            .unwrap()
            .into_body();
            assert!(restored.frame().await.unwrap().is_ok());
            drop(restored);
            let result = captured.lock().unwrap().take().unwrap();
            assert_eq!(result.error.is_none(), wire.ends_with('}'));
            assert_eq!(result.bytes, wire.as_bytes());
        }
    }
    #[tokio::test]
    async fn empty_resource_responses_preserve_status_without_accepting_invalid_json() {
        for status in [204, 304, 200] {
            let (vault, _, _) = vault();
            let response = http::Response::builder()
                .status(status)
                .body(Full::new(Bytes::new()))
                .unwrap();
            let (parts, body) = response.into_parts();
            let result = restore(parts, body, vault, false, false).await;
            {
                let response = result.unwrap();
                assert_eq!(response.status().as_u16(), status);
                assert!(response
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .is_empty());
            }
        }
    }
    #[tokio::test]
    async fn dropping_unpolled_stream_releases_ephemeral_scope() {
        let (vault, store, _) = vault();
        vault.bind_response("resp_cancel").unwrap();
        let weak = Arc::downgrade(&vault);
        let response = http::Response::builder()
            .header("content-type", "text/event-stream")
            .body(Full::new(Bytes::from_static(b"data: [DONE]\n\n")))
            .unwrap();
        let (parts, body) = response.into_parts();
        let response = restore(parts, body, vault, true, false).await.unwrap();
        assert!(weak.upgrade().is_some());
        drop(response);
        assert!(weak.upgrade().is_none());
        assert!(Vault::continue_response(store, "cancel-owner", "resp_cancel").is_err());
    }
    #[tokio::test]
    async fn malformed_or_truncated_stream_is_delivered_without_restoration_failure() {
        for bytes in [b"data: {bad}\n\n".to_vec(),b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"<localrouter-redacted\"}\n\n".to_vec(),vec![0xff,b'\n',b'\n']] {
            let (vault,_,_)=vault();let original=bytes.clone();let response=http::Response::new(Full::new(Bytes::from(bytes)));let(parts,body)=response.into_parts();
            assert_eq!(restore(parts,body,vault,true,false).await.unwrap().into_body().collect().await.unwrap().to_bytes().as_ref(), original.as_slice());
        }
    }
    #[tokio::test]
    async fn integrity_headers_are_removed_and_compression_is_forwarded_unchanged() {
        let (vault, _, tag) = vault();
        let response = http::Response::builder()
            .header("content-length", "1")
            .header("etag", "masked")
            .header("digest", "old")
            .body(Full::new(Bytes::from(json!({"content":tag}).to_string())))
            .unwrap();
        let (parts, body) = response.into_parts();
        let response = restore(parts, body, vault, false, false).await.unwrap();
        assert!(response.headers().get("content-length").is_none());
        assert!(response.headers().get("etag").is_none());
        assert!(response.headers().get("digest").is_none());
        let (vault, _, _) = self::vault();
        let response = http::Response::builder()
            .header("content-encoding", "gzip")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let (parts, body) = response.into_parts();
        let response = restore(parts, body, vault, false, false).await.unwrap();
        assert_eq!(response.headers()["content-encoding"], "gzip");
        assert!(response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty());
    }
}
