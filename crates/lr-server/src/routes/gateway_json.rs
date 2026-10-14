//! Typed JSON ingress with validation before unknown vendor controls are lost.
use crate::middleware::error::ApiErrorResponse;
use axum::{
    extract::{FromRequest, Request},
    response::{IntoResponse, Response},
    Json,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

pub struct GatewayJson<T>(pub T);

impl<S, T> FromRequest<S> for GatewayJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<Value>::from_request(request, state)
            .await
            .map_err(IntoResponse::into_response)?;
        lr_providers::gateway::reject_raw_controls(&value)
            .map_err(|error| ApiErrorResponse::from(error).into_response())?;
        serde_json::from_value(value).map(Self).map_err(|error| {
            ApiErrorResponse::bad_request(format!("Invalid request: {error}")).into_response()
        })
    }
}
