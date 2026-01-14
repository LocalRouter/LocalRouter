//! GET /v1/generation endpoint
//!
//! Retrieve detailed information about a specific generation.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;

use crate::server::middleware::error::{ApiErrorResponse, ApiResult};
use crate::server::state::AppState;
use crate::server::types::GenerationDetailsResponse;

#[derive(Debug, Deserialize)]
pub struct GenerationQuery {
    pub id: String,
}

/// GET /v1/generation?id={generation_id}
/// Get detailed information about a generation
pub async fn get_generation(
    State(state): State<AppState>,
    Query(query): Query<GenerationQuery>,
) -> ApiResult<Json<GenerationDetailsResponse>> {
    let details = state
        .generation_tracker
        .get(&query.id)
        .ok_or_else(|| {
            ApiErrorResponse::new(
                axum::http::StatusCode::NOT_FOUND,
                "not_found_error",
                format!("Generation '{}' not found", query.id),
            )
        })?;

    Ok(Json(details))
}
