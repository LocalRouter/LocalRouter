//! Thin HTTP client for `POST <base_url>/responses`.
//!
//! Used by `OpenAIProvider` when `is_chatgpt_backend()` — or any
//! future native-Responses provider — to hit the upstream directly.
//! Translation of the request/response shape lives in `request.rs`
//! and `response.rs`; this file just handles the transport.

use std::pin::Pin;

use futures::stream::Stream;
use reqwest::Client;

use super::types::{ResponseObject, ResponsesApiRequest};
use crate::{CompletionChunk, CompletionResponse};
use lr_types::{AppError, AppResult};

/// Non-streaming POST /responses.
///
/// `base_url` is e.g. `https://chatgpt.com/backend-api/codex`. We append
/// `/responses` and send the JSON-serialized request with a Bearer
/// Authorization header.
pub async fn create_response(
    client: &Client,
    base_url: &str,
    access_token: &str,
    provider_name: &str,
    request: ResponsesApiRequest,
) -> AppResult<CompletionResponse> {
    let (raw_value, raw_object) =
        create_response_raw(client, base_url, access_token, request).await?;
    let mut completion = super::response::response_to_completion(raw_object, provider_name);
    // Stash the upstream JSON verbatim so `/v1/responses` adapters can
    // bypass the lossy ChatCompletion translation and serve native
    // fields (reasoning items, encrypted content carry-over, built-in
    // tool results, `include[]`). Other adapters ignore the key.
    let mut ext = completion.extensions.unwrap_or_default();
    ext.insert(NATIVE_RESPONSES_API_EXT_KEY.to_string(), raw_value);
    completion.extensions = Some(ext);
    Ok(completion)
}

/// Key under which `OpenAIProvider` stashes the upstream Responses
/// API JSON inside `CompletionResponse.extensions` when ChatGPT Plus
/// routes through `/responses`. Adapters that speak native Responses
/// (today: `routes/responses.rs`) pull the value out verbatim and
/// bypass the lossy `response_to_completion` translation.
pub const NATIVE_RESPONSES_API_EXT_KEY: &str = "__native_responses_api_object";

/// Raw non-streaming `POST /responses` — returns both the verbatim
/// upstream JSON (for native-format pass-through) and the decoded
/// `ResponseObject` (for internal ChatCompletion translation).
///
/// We parse the body as a `Value` first, then deserialize that into
/// `ResponseObject`, so the verbatim JSON is available to callers
/// without requiring `Serialize` on our internal view of the response.
pub async fn create_response_raw(
    client: &Client,
    base_url: &str,
    access_token: &str,
    request: ResponsesApiRequest,
) -> AppResult<(serde_json::Value, ResponseObject)> {
    let url = format!("{}/responses", base_url);
    let response = client
        .post(&url)
        .bearer_auth(access_token)
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| AppError::Provider(format!("Responses API request failed: {}", e)))?;

    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(crate::http_client::classify_openai_error(status, &body));
    }

    let raw_value: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Provider(format!("Failed to parse /responses body: {}", e)))?;
    let parsed: ResponseObject = serde_json::from_value(raw_value.clone())
        .map_err(|e| AppError::Provider(format!("Failed to decode /responses body: {}", e)))?;
    Ok((raw_value, parsed))
}

/// Streaming POST /responses.
///
/// Returns a `Stream<CompletionChunk>` — the SSE event-to-chunk
/// translator (`stream.rs`) is applied automatically.
pub async fn stream_response(
    client: &Client,
    base_url: &str,
    access_token: &str,
    provider_name: &str,
    model: String,
    mut request: ResponsesApiRequest,
) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
    // Defensive: the caller may have constructed the request with
    // `stream=false`. Force it on for this path.
    request.stream = true;

    let url = format!("{}/responses", base_url);
    let response = client
        .post(&url)
        .bearer_auth(access_token)
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .json(&request)
        .send()
        .await
        .map_err(|e| AppError::Provider(format!("Responses API stream request failed: {}", e)))?;

    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(crate::http_client::classify_openai_error(status, &body));
    }

    Ok(super::stream::responses_to_completion_chunks(
        response.bytes_stream(),
        provider_name.to_string(),
        model,
    ))
}
