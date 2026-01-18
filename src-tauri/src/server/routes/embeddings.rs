//! POST /v1/embeddings endpoint
//!
//! Convert text to vector embeddings.

use axum::{extract::State, response::Response, Extension, Json};

use crate::server::middleware::error::{ApiErrorResponse, ApiResult};
use crate::server::state::{AppState, AuthContext};
use crate::server::types::EmbeddingRequest;

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
    Extension(_auth): Extension<AuthContext>,
    Json(request): Json<EmbeddingRequest>,
) -> ApiResult<Response> {
    // Emit LLM request event to trigger tray icon indicator
    state.emit_event("llm-request", "embedding");

    // Validate request
    validate_request(&request)?;

    // TODO: Implement embeddings support
    // This requires:
    // 1. Adding an embeddings method to the ModelProvider trait
    // 2. Implementing embeddings for each provider
    // 3. Router support for embedding models
    //
    // For now, return a not implemented error

    Err(ApiErrorResponse::new(
        axum::http::StatusCode::NOT_IMPLEMENTED,
        "not_implemented",
        "Embeddings endpoint not yet implemented. This is planned for a future release.",
    ))
}

/// Validate embedding request
fn validate_request(request: &EmbeddingRequest) -> ApiResult<()> {
    if request.model.is_empty() {
        return Err(ApiErrorResponse::bad_request("model is required").with_param("model"));
    }

    // Validate encoding format
    if let Some(format) = &request.encoding_format {
        if format != "float" && format != "base64" {
            return Err(
                ApiErrorResponse::bad_request("encoding_format must be 'float' or 'base64'")
                    .with_param("encoding_format"),
            );
        }
    }

    Ok(())
}
