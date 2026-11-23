//! Typed JSON ingress with validation before unknown vendor controls are lost.
use crate::middleware::error::ApiErrorResponse;
use axum::{
    body::{Body, Bytes},
    extract::{FromRequest, Request},
    http::{header, request::Parts, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use http_body::{Frame, SizeHint};
use parking_lot::Mutex;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

pub struct GatewayJson<T>(pub T);

/// A JSON body the secret boundary already parsed. [`GatewayJson`] takes the
/// value instead of serializing and parsing it again; any other consumer
/// reads the body, which serializes the value on first poll. Either way the
/// request exists once in memory, as this value.
#[derive(Clone)]
pub(crate) struct ParsedBody(Arc<Mutex<Option<Value>>>);

/// Rebuild a request whose body is `value`. `Content-Length` is dropped:
/// the serialized length is only known if a consumer asks for the bytes.
pub(crate) fn parsed_request(mut parts: Parts, value: Value) -> Request {
    let slot = Arc::new(Mutex::new(Some(value)));
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.extensions.insert(ParsedBody(slot.clone()));
    Request::from_parts(parts, Body::new(LazyJsonBody { slot, done: false }))
}

struct LazyJsonBody {
    slot: Arc<Mutex<Option<Value>>>,
    done: bool,
}

impl http_body::Body for LazyJsonBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(None);
        }
        this.done = true;
        let Some(value) = this.slot.lock().take() else {
            return Poll::Ready(None);
        };
        Poll::Ready(Some(
            serde_json::to_vec(&value)
                .map(|bytes| Frame::data(Bytes::from(bytes)))
                .map_err(axum::Error::new),
        ))
    }

    fn is_end_stream(&self) -> bool {
        self.done
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }
}

/// The native server does not decode request bodies. A compressed body is
/// refused explicitly rather than reported as malformed JSON.
pub(crate) fn reject_content_encoding(headers: &HeaderMap) -> Result<(), ApiErrorResponse> {
    let encoded = headers
        .get_all(header::CONTENT_ENCODING)
        .iter()
        .flat_map(|value| value.to_str().unwrap_or("unreadable").split(','))
        .map(str::trim)
        .any(|coding| !coding.is_empty() && !coding.eq_ignore_ascii_case("identity"));
    if encoded {
        return Err(ApiErrorResponse::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "invalid_request_error",
            "Compressed request bodies are not supported; send the JSON body uncompressed",
        )
        .with_code("unsupported_content_encoding"));
    }
    Ok(())
}

/// The same media-type rule as axum's `Json`: `application/json` or any
/// `application/*+json`.
fn json_content_type(headers: &HeaderMap) -> bool {
    let Some(value) = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let essence = value
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let Some(subtype) = essence.strip_prefix("application/") else {
        return false;
    };
    subtype == "json" || subtype.ends_with("+json")
}

fn missing_json_content_type() -> ApiErrorResponse {
    ApiErrorResponse::new(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "invalid_request_error",
        "Expected request with `Content-Type: application/json`",
    )
}

/// Read and parse the body. The raw bytes are dropped as soon as the value
/// exists, so the request is never held as bytes, value and typed copy at
/// once.
async fn read_value<S: Send + Sync>(
    request: Request,
    state: &S,
) -> Result<Value, ApiErrorResponse> {
    reject_content_encoding(request.headers())?;
    if !json_content_type(request.headers()) {
        return Err(missing_json_content_type());
    }
    let bytes = Bytes::from_request(request, state)
        .await
        .map_err(|rejection| {
            if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                ApiErrorResponse::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "invalid_request_error",
                    "Request body is too large",
                )
                .with_code("request_too_large")
            } else {
                ApiErrorResponse::bad_request("Request body could not be read")
            }
        })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        // Syntax errors carry only a position, never request content.
        ApiErrorResponse::bad_request(format!("Failed to parse the request body as JSON: {error}"))
    })
}

impl<S, T> FromRequest<S> for GatewayJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let session = [
            "x-session-id",
            "x-session-affinity",
            "session_id",
            "session-id",
        ]
        .into_iter()
        .find_map(|name| request.headers().get(name))
        .map(|value| value.to_str().map(str::to_owned))
        .transpose()
        .map_err(|_| ApiErrorResponse::bad_request("Invalid session header").into_response())?;
        let path = request.uri().path().to_owned();
        let parsed = request
            .extensions()
            .get::<ParsedBody>()
            .and_then(|parsed| parsed.0.lock().take());
        let mut value = match parsed {
            Some(value) => {
                if !json_content_type(request.headers()) {
                    return Err(missing_json_content_type().into_response());
                }
                value
            }
            None => read_value(request, state)
                .await
                .map_err(IntoResponse::into_response)?,
        };
        lr_providers::response_cache::validate_ingress(&value)
            .map_err(|error| ApiErrorResponse::from(error).into_response())?;
        lr_providers::replay::normalize_ingress(&mut value)
            .map_err(|error| ApiErrorResponse::from(error).into_response())?;
        crate::documents::normalize_ingress(&mut value, &path)
            .map_err(|error| ApiErrorResponse::from(error).into_response())?;
        crate::upstream_tools::normalize_ingress(&mut value, &path)
            .map_err(|error| ApiErrorResponse::from(error).into_response())?;
        lr_providers::session::normalize_ingress(&mut value, session.as_deref())
            .map_err(|error| ApiErrorResponse::from(error).into_response())?;
        lr_providers::gateway::reject_raw_controls(&value)
            .map_err(|error| ApiErrorResponse::from(error).into_response())?;
        // `from_value` moves strings out of the value instead of copying them.
        serde_json::from_value(value).map(Self).map_err(|error| {
            ApiErrorResponse::bad_request(format!("Invalid request: {error}")).into_response()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[derive(serde::Deserialize)]
    struct Model {
        model: String,
    }

    fn request(content_type: Option<&str>, encoding: Option<&str>, body: &str) -> Request {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions");
        if let Some(content_type) = content_type {
            builder = builder.header("content-type", content_type);
        }
        if let Some(encoding) = encoding {
            builder = builder.header("content-encoding", encoding);
        }
        builder.body(Body::from(body.to_owned())).unwrap()
    }

    async fn error(response: Response) -> (StatusCode, Value) {
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn rejections_are_structured_openai_errors() {
        for (content_type, encoding, body, status, code) in [
            (None, None, r#"{"model":"m"}"#, 415, None),
            (Some("text/plain"), None, r#"{"model":"m"}"#, 415, None),
            (
                Some("application/json"),
                Some("gzip"),
                "\x1f\u{8b}",
                415,
                Some("unsupported_content_encoding"),
            ),
            (
                Some("application/json"),
                Some("identity, br"),
                "{}",
                415,
                Some("unsupported_content_encoding"),
            ),
            (Some("application/json"), None, "{not json", 400, None),
            (Some("application/json"), None, r#"{"model":7}"#, 400, None),
        ] {
            let rejection =
                GatewayJson::<Model>::from_request(request(content_type, encoding, body), &())
                    .await
                    .err()
                    .expect("rejected");
            let (actual, value) = error(rejection).await;
            assert_eq!(
                actual.as_u16(),
                status,
                "{content_type:?} {encoding:?} {body}"
            );
            assert!(value["error"]["message"].is_string(), "{value}");
            assert_eq!(value["error"]["code"].as_str(), code);
        }
    }

    #[tokio::test]
    async fn json_variants_and_identity_encoding_are_accepted() {
        for (content_type, encoding) in [
            ("application/json", None),
            ("application/json; charset=utf-8", Some("identity")),
            ("Application/Vnd.Api+JSON", None),
        ] {
            let GatewayJson(model) = GatewayJson::<Model>::from_request(
                request(Some(content_type), encoding, r#"{"model":"m"}"#),
                &(),
            )
            .await
            .expect("accepted");
            assert_eq!(model.model, "m");
        }
    }

    #[tokio::test]
    async fn parsed_body_is_taken_without_reading_and_serialized_for_other_readers() {
        let parts = || {
            let (mut parts, _) = request(Some("application/json"), None, "").into_parts();
            parts
                .headers
                .insert(header::CONTENT_LENGTH, HeaderValue::from_static("999"));
            parts
        };
        let taken = parsed_request(parts(), serde_json::json!({"model":"parsed"}));
        assert!(taken.headers().get(header::CONTENT_LENGTH).is_none());
        let GatewayJson(model) = GatewayJson::<Model>::from_request(taken, &())
            .await
            .expect("accepted");
        assert_eq!(model.model, "parsed");

        let read = parsed_request(parts(), serde_json::json!({"model":"bytes"}));
        let bytes = axum::body::to_bytes(read.into_body(), 1024).await.unwrap();
        assert_eq!(&bytes[..], br#"{"model":"bytes"}"#);

        // A parsed body still obeys the JSON media-type rule.
        let (mut parts, _) = request(Some("text/plain"), None, "").into_parts();
        parts.headers.remove(header::CONTENT_LENGTH);
        let rejection =
            GatewayJson::<Model>::from_request(parsed_request(parts, serde_json::json!({})), &())
                .await
                .err()
                .expect("rejected");
        assert_eq!(rejection.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
}
