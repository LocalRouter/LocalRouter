//! Error handling middleware for OpenAI-compatible error responses

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::types::ErrorResponse;
use lr_types::errors::AppError;

/// Application error that can be converted to HTTP response
#[derive(Debug)]
pub struct ApiErrorResponse {
    pub status: StatusCode,
    pub error: ErrorResponse,
    /// Optional retry-after header value (seconds)
    pub retry_after_secs: Option<u64>,
}

impl ApiErrorResponse {
    pub fn new(
        status: StatusCode,
        error_type: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            error: ErrorResponse::new(error_type, message),
            retry_after_secs: None,
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

    pub fn rate_limited_with_retry(message: impl Into<String>, retry_after_secs: u64) -> Self {
        let mut resp = Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limit_error", message);
        resp.retry_after_secs = Some(retry_after_secs);
        resp
    }

    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }

    pub fn bad_gateway(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, "provider_error", message)
    }

    #[allow(dead_code)]
    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
            message,
        )
    }

    #[allow(dead_code)]
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found_error", message)
    }

    pub fn with_param(mut self, param: impl Into<String>) -> Self {
        self.error = self.error.with_param(param);
        self
    }

    #[allow(dead_code)]
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.error = self.error.with_code(code);
        self
    }
}

impl IntoResponse for ApiErrorResponse {
    fn into_response(self) -> Response {
        if let Some(retry_after) = self.retry_after_secs {
            let mut response = (self.status, Json(self.error)).into_response();
            response.headers_mut().insert(
                "retry-after",
                axum::http::HeaderValue::from_str(&retry_after.to_string())
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("60")),
            );
            response
        } else {
            (self.status, Json(self.error)).into_response()
        }
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
            AppError::RateLimitExceeded => ApiErrorResponse::rate_limited("Rate limit exceeded"),
            AppError::FreeTierExhausted { retry_after_secs } => {
                ApiErrorResponse::rate_limited_with_retry(
                    "Free tier exhausted. All free-tier providers are at capacity.",
                    retry_after_secs,
                )
            }
            AppError::FreeTierFallbackAvailable {
                retry_after_secs, ..
            } => {
                // This should be caught by the chat handler before reaching here,
                // but if it leaks through, treat it as exhausted
                ApiErrorResponse::rate_limited_with_retry(
                    "Free tier exhausted. All free-tier providers are at capacity.",
                    retry_after_secs,
                )
            }
            AppError::Unauthorized => ApiErrorResponse::unauthorized("Unauthorized"),
            AppError::ApiKey(msg) => {
                ApiErrorResponse::unauthorized(format!("API key error: {}", msg))
            }
            AppError::Mcp(msg) => ApiErrorResponse::bad_gateway(format!("MCP error: {}", msg)),
            AppError::Storage(msg) => {
                ApiErrorResponse::internal_error(format!("Storage error: {}", msg))
            }
            AppError::Router(msg) => {
                ApiErrorResponse::bad_gateway(format!("Router error: {}", msg))
            }
            AppError::Internal(msg) => ApiErrorResponse::internal_error(msg),
            AppError::Io(err) => ApiErrorResponse::internal_error(format!("IO error: {}", err)),
            AppError::Serialization(err) => {
                ApiErrorResponse::internal_error(format!("Serialization error: {}", err))
            }
            AppError::Crypto(err) => {
                ApiErrorResponse::internal_error(format!("Crypto error: {}", err))
            }
            AppError::OAuthBrowser(msg) => {
                ApiErrorResponse::internal_error(format!("OAuth browser error: {}", msg))
            }
            AppError::InvalidParams(msg) => {
                ApiErrorResponse::bad_request(format!("Invalid parameters: {}", msg))
            }
        }
    }
}

/// Result type for API handlers
pub type ApiResult<T> = Result<T, ApiErrorResponse>;
