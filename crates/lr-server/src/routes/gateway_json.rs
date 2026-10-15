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
        let Json(mut value) = Json::<Value>::from_request(request, state)
            .await
            .map_err(IntoResponse::into_response)?;
        lr_providers::replay::normalize_ingress(&mut value)
            .map_err(|error| ApiErrorResponse::from(error).into_response())?;
        lr_providers::session::normalize_ingress(&mut value, session.as_deref())
            .map_err(|error| ApiErrorResponse::from(error).into_response())?;
        lr_providers::gateway::reject_raw_controls(&value)
            .map_err(|error| ApiErrorResponse::from(error).into_response())?;
        serde_json::from_value(value).map(Self).map_err(|error| {
            ApiErrorResponse::bad_request(format!("Invalid request: {error}")).into_response()
        })
    }
}
