//! Restore only after the masked provider body has passed the monitoring tap.
use bytes::Bytes;
use futures::StreamExt;
use http_body_util::{BodyExt, Full, StreamBody};
use lr_secret_scanner::{
    placeholders::Vault,
    wire::{frame_end, ResponseRestorer, LIMIT},
};
use std::{convert::Infallible, sync::Arc};

pub(crate) async fn restore<B>(
    mut parts: http::response::Parts,
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
    if parts
        .headers
        .get(http::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value != "identity")
    {
        return Err(std::io::Error::other(
            "Unexpected compression on protected upstream response",
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
    parts.headers.remove(http::header::CONTENT_ENCODING);
    parts.headers.insert(
        http::header::CACHE_CONTROL,
        http::HeaderValue::from_static("no-store"),
    );
    let mut codec = ResponseRestorer::new(vault);
    if !streaming && !ndjson {
        let mut stream = body.into_data_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| std::io::Error::other(e.into()))?;
            if bytes.len().saturating_add(chunk.len()) > LIMIT {
                return Err(std::io::Error::other("Protected response exceeds limit"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let bytes = if bytes.is_empty()
            && matches!(
                parts.status,
                http::StatusCode::NO_CONTENT | http::StatusCode::NOT_MODIFIED
            ) {
            bytes
        } else {
            codec.json(&bytes).map_err(std::io::Error::other)?
        };
        let body = Full::new(Bytes::from(bytes))
            .map_err(|e: Infallible| match e {})
            .boxed_unsync();
        return Ok(http::Response::from_parts(parts, body));
    }
    let stream = async_stream::try_stream! {
        let mut stream = body.into_data_stream();
        let mut pending = Vec::new();
        let mut scanned = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| std::io::Error::other(e.into()))?;
            pending.extend_from_slice(&chunk);
            if pending.len() > LIMIT { Err(std::io::Error::other("Protected response frame exceeds limit"))?; }
            while let Some((relative, separator)) = if ndjson { pending[scanned..].iter().position(|b| *b == b'\n').map(|i| (i, 1)) } else { frame_end(&pending[scanned..]) } {
                let end = scanned + relative; scanned = 0;
                let frame = String::from_utf8(pending.drain(..end).collect()).map_err(std::io::Error::other)?;
                pending.drain(..separator);
                let events = if ndjson {
                    if frame.trim().is_empty() { Vec::new() } else { codec.message(&frame).map_err(std::io::Error::other)?.into_iter().map(|event| format!("{event}\n")).collect() }
                } else { codec.frame(&frame).map_err(std::io::Error::other)? };
                for event in events { yield http_body::Frame::data(Bytes::from(event)); }
            }
            scanned = pending.len().saturating_sub(3);
        }
        if !pending.is_empty() {
            if !ndjson { Err(std::io::Error::other("Incomplete protected response event"))?; }
            let frame = String::from_utf8(pending).map_err(std::io::Error::other)?;
            for event in codec.message(&frame).map_err(std::io::Error::other)? { yield http_body::Frame::data(Bytes::from(format!("{event}\n"))); }
        }
        for event in codec.finish().map_err(std::io::Error::other)? { yield http_body::Frame::data(Bytes::from(event)); }
    };
    let stream: std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<http_body::Frame<Bytes>, std::io::Error>> + Send>,
    > = Box::pin(stream);
    Ok(http::Response::from_parts(
        parts,
        StreamBody::new(stream).boxed_unsync(),
    ))
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
    async fn empty_resource_responses_preserve_status_without_accepting_invalid_json() {
        for status in [204, 304, 200] {
            let (vault, _, _) = vault();
            let response = http::Response::builder()
                .status(status)
                .body(Full::new(Bytes::new()))
                .unwrap();
            let (parts, body) = response.into_parts();
            let result = restore(parts, body, vault, false, false).await;
            if status == 200 {
                assert!(result.is_err(), "empty inference responses remain invalid");
            } else {
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
    async fn malformed_or_truncated_stream_fails_without_forwarding_executable_marker() {
        for bytes in [b"data: {bad}\n\n".to_vec(),b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"<localrouter-redacted\"}\n\n".to_vec(),vec![0xff,b'\n',b'\n']] {
            let (vault,_,_)=vault();let response=http::Response::new(Full::new(Bytes::from(bytes)));let(parts,body)=response.into_parts();
            assert!(restore(parts,body,vault,true,false).await.unwrap().into_body().collect().await.is_err());
        }
    }
    #[tokio::test]
    async fn integrity_headers_are_removed_and_unexpected_compression_is_rejected() {
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
        assert!(restore(parts, body, vault, false, false).await.is_err());
    }
}
