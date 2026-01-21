//! POST /v1/embeddings endpoint
//!
//! Convert text to vector embeddings.

use axum::{extract::State, response::{IntoResponse, Response}, Extension, Json};
use uuid::Uuid;

use crate::server::middleware::error::{ApiErrorResponse, ApiResult};
use crate::server::state::{AppState, AuthContext};
use crate::server::types::{EmbeddingRequest, EmbeddingResponse, EmbeddingData, EmbeddingVector};

/// POST /v1/embeddings
/// Generate embeddings for input text(s)
#[utoipa::path(
    post,
    path = "/v1/embeddings",
    tag = "embeddings",
    request_body = EmbeddingRequest,
    responses(
        (status = 200, description = "Successful response", body = crate::server::types::EmbeddingResponse),
        (status = 400, description = "Bad request", body = crate::server::types::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::server::types::ErrorResponse),
        (status = 501, description = "Not implemented yet", body = crate::server::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::server::types::ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn embeddings(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(request): Json<EmbeddingRequest>,
) -> ApiResult<Response> {
    // Emit LLM request event to trigger tray icon indicator
    state.emit_event("llm-request", "embedding");

    // Validate request
    validate_request(&request)?;

    // Generate a unique ID for this request
    let request_id = format!("emb-{}", Uuid::new_v4());

    // Convert encoding_format from String to EncodingFormat
    let encoding_format = request.encoding_format.as_ref().and_then(|fmt| match fmt.as_str() {
        "float" => Some(crate::providers::EncodingFormat::Float),
        "base64" => Some(crate::providers::EncodingFormat::Base64),
        _ => None,
    });

    // Convert server EmbeddingInput to provider EmbeddingInput
    let provider_input = match request.input.clone() {
        crate::server::types::EmbeddingInput::Single(s) => crate::providers::EmbeddingInput::Single(s),
        crate::server::types::EmbeddingInput::Multiple(v) => crate::providers::EmbeddingInput::Multiple(v),
    };

    // Convert to provider format
    let provider_request = crate::providers::EmbeddingRequest {
        model: request.model.clone(),
        input: provider_input,
        encoding_format,
        dimensions: request.dimensions,
        user: request.user.clone(),
    };

    // Call router to get embeddings
    let response = match state.router.embed(&auth.api_key_id, provider_request).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("Embedding request failed: {}", e);
            return Err(ApiErrorResponse::bad_gateway(format!(
                "Provider error: {}",
                e
            )));
        }
    };

    // Convert provider response to API response
    let api_response = EmbeddingResponse {
        object: response.object,
        data: response
            .data
            .into_iter()
            .map(|emb| EmbeddingData {
                object: emb.object,
                embedding: if let Some(vec) = emb.embedding {
                    EmbeddingVector::Float(vec)
                } else {
                    EmbeddingVector::Float(vec![])  // Default to empty if none
                },
                index: emb.index as u32,
            })
            .collect(),
        model: response.model,
        usage: crate::server::types::EmbeddingUsage {
            prompt_tokens: response.usage.prompt_tokens,
            total_tokens: response.usage.total_tokens,
        },
    };

    // Log success
    tracing::info!(
        "Embedding request completed: id={}, model={}, tokens={}",
        request_id,
        api_response.model,
        api_response.usage.total_tokens
    );

    // Return JSON response
    Ok(Json(api_response).into_response())
}

/// Validate embedding request
fn validate_request(request: &EmbeddingRequest) -> ApiResult<()> {
    if request.model.is_empty() {
        return Err(ApiErrorResponse::bad_request("model is required").with_param("model"));
    }

    // Validate encoding format
    if let Some(format) = &request.encoding_format {
        if format != "float" && format != "base64" {
            return Err(ApiErrorResponse::bad_request(
                "encoding_format must be 'float' or 'base64'",
            )
            .with_param("encoding_format"));
        }
    }

    Ok(())
}
