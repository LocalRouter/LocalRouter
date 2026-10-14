//! OpenAI-compatible typed Decisions API, sharing the System One policy pipeline.
use super::gateway_json::GatewayJson;
use crate::{
    middleware::{
        client_auth::ClientAuthContext,
        error::{ApiErrorResponse, ApiResult},
    },
    state::{AppState, AuthContext},
};
use axum::{extract::State, response::Response, Extension};
use lr_providers::systemone::decisions::DecisionRequest;

#[utoipa::path(
    post, path = "/v1/decisions", tag = "systemone",
    request_body = lr_providers::systemone::decisions::DecisionRequest,
    responses(
        (status = 200, description = "Ordered typed answers, including per-question refusals", body = lr_providers::systemone::decisions::DecisionResponse,
            headers(("x-localrouter-decisions-backend" = String, description = "native, letter_logprobs, or json; translated probabilities are not asserted to be calibrated"))),
        (status = 400, description = "Invalid request or unsupported image input", body = crate::types::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 403, description = "Access or content policy denied", body = crate::types::ErrorResponse),
        (status = 429, description = "Rate limited", body = crate::types::ErrorResponse),
        (status = 502, description = "Invalid upstream response", body = crate::types::ErrorResponse)
    ), security(("bearer_auth" = []))
)]
pub async fn decisions(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    GatewayJson(request): GatewayJson<DecisionRequest>,
) -> ApiResult<Response> {
    request.validate().map_err(ApiErrorResponse::bad_request)?;
    super::systemone::handle_decision(state, auth, client_auth, request.into_systemone()).await
}
