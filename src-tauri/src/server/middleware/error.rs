//! Error handling middleware for OpenAI-compatible error responses

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::server::types::ErrorResponse;
use crate::utils::errors::AppError;

/// Application error that can be converted to HTTP response
pub struct ApiErrorResponse {
    pub status: StatusCode,
    pub error: ErrorResponse,
}

impl ApiErrorResponse {
    pub fn new(status: StatusCode, error_type: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            error: ErrorResponse::new(error_type, message),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request_error", message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "authentication_error", message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "permission_error", message)
    }

    pub fn rate_limited(message: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limit_error", message)
    }

    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }

    pub fn bad_gateway(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, "provider_error", message)
    }

    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "service_unavailable", message)
    }

    pub fn with_param(mut self, param: impl Into<String>) -> Self {
        self.error = self.error.with_param(param);
        self
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.error = self.error.with_code(code);
        self
    }
}

impl IntoResponse for ApiErrorResponse {
    fn into_response(self) -> Response {
        (self.status, Json(self.error)).into_response()
    }
}

/// Convert AppError to ApiErrorResponse
impl From<AppError> for ApiErrorResponse {
    fn from(err: AppError) -> Self {
        match err {
            AppError::Config(msg) => {
                ApiErrorResponse::bad_request(format!("Configuration error: {}", msg))
            }
            AppError::Provider(msg) => {
                ApiErrorResponse::bad_gateway(format!("Provider error: {}", msg))
            }
            AppError::RateLimitExceeded => {
                ApiErrorResponse::rate_limited("Rate limit exceeded")
            }
            AppError::Unauthorized => {
                ApiErrorResponse::unauthorized("Unauthorized")
            }
            AppError::ApiKey(msg) => {
                ApiErrorResponse::unauthorized(format!("API key error: {}", msg))
            }
            AppError::Router(msg) => {
                ApiErrorResponse::bad_gateway(format!("Router error: {}", msg))
            }
            AppError::Internal(msg) => {
                ApiErrorResponse::internal_error(msg)
            }
            AppError::Io(err) => {
                ApiErrorResponse::internal_error(format!("IO error: {}", err))
            }
            AppError::Serialization(err) => {
                ApiErrorResponse::internal_error(format!("Serialization error: {}", err))
            }
            AppError::Crypto(err) => {
                ApiErrorResponse::internal_error(format!("Crypto error: {}", err))
            }
        }
    }
}

/// Result type for API handlers
pub type ApiResult<T> = Result<T, ApiErrorResponse>;
