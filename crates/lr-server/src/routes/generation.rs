//! GET /v1/generation endpoint
//!
//! Retrieve detailed information about a specific generation.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::server::middleware::error::{ApiErrorResponse, ApiResult};
use crate::server::state::AppState;
use crate::server::types::GenerationDetailsResponse;

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct GenerationQuery {
    #[param(example = "chatcmpl-123")]
    pub id: String,
}

/// GET /v1/generation?id={generation_id}
/// Get detailed information about a generation
#[utoipa::path(
    get,
    path = "/v1/generation",
    tag = "monitoring",
    params(GenerationQuery),
    responses(
        (status = 200, description = "Generation details", body = GenerationDetailsResponse),
        (status = 404, description = "Generation not found", body = crate::server::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::server::types::ErrorResponse)
    )
)]
pub async fn get_generation(
    State(state): State<AppState>,
    Query(query): Query<GenerationQuery>,
) -> ApiResult<Json<GenerationDetailsResponse>> {
    let details = state.generation_tracker.get(&query.id).ok_or_else(|| {
        ApiErrorResponse::new(
            axum::http::StatusCode::NOT_FOUND,
            "not_found_error",
            format!("Generation '{}' not found", query.id),
        )
    })?;

    Ok(Json(details))
}
