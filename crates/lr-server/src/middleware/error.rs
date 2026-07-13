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

    pub fn payment_required(message: impl Into<String>) -> Self {
        Self::new(StatusCode::PAYMENT_REQUIRED, "free_tier_error", message)
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
            // Upstream 4xx passes through with its real status so clients can
            // tell a permanent request error from a transient upstream one.
            // A non-4xx (or malformed) status falls back to 502.
            AppError::ProviderStatus { status, message } => match StatusCode::from_u16(status) {
                Ok(code) if code.is_client_error() => ApiErrorResponse::new(
                    code,
                    "provider_error",
                    format!("Provider error: {}", message),
                ),
                _ => ApiErrorResponse::bad_gateway(format!("Provider error: {}", message)),
            },
            AppError::RateLimitExceeded => ApiErrorResponse::rate_limited("Rate limit exceeded"),
            AppError::FreeTierExhausted { .. } => ApiErrorResponse::payment_required(
                "Free tier exhausted. All free-tier providers are at capacity.",
            ),
            AppError::FreeTierFallbackAvailable { .. } => {
                // This should be caught by the chat handler before reaching here,
                // but if it leaks through, treat it as exhausted
                ApiErrorResponse::payment_required(
                    "Free tier exhausted. All free-tier providers are at capacity.",
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
            // Typed provider errors surface as 400/404 so clients can react
            // to them distinctly from generic upstream failures.
            AppError::ContextLengthExceeded { .. } => {
                ApiErrorResponse::bad_request(err.to_string())
            }
            AppError::ModelNotFound { .. } => ApiErrorResponse::bad_request(err.to_string()),
        }
    }
}

/// Result type for API handlers
pub type ApiResult<T> = Result<T, ApiErrorResponse>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_status_4xx_passes_through() {
        // An upstream 4xx must surface with its real status, not a 502, so
        // clients can tell a permanent request error from a transient one.
        let resp: ApiErrorResponse = AppError::ProviderStatus {
            status: 400,
            message: "API error (400 Bad Request): unsupported param".to_string(),
        }
        .into();
        assert_eq!(resp.status, StatusCode::BAD_REQUEST);

        let resp403: ApiErrorResponse = AppError::ProviderStatus {
            status: 403,
            message: "forbidden".to_string(),
        }
        .into();
        assert_eq!(resp403.status, StatusCode::FORBIDDEN);
    }

    #[test]
    fn provider_status_non_4xx_falls_back_to_502() {
        // A 5xx (or otherwise non-client) upstream status is an upstream
        // failure and stays a 502.
        let resp: ApiErrorResponse = AppError::ProviderStatus {
            status: 503,
            message: "upstream down".to_string(),
        }
        .into();
        assert_eq!(resp.status, StatusCode::BAD_GATEWAY);

        // Plain Provider errors remain 502 as before.
        let resp2: ApiErrorResponse = AppError::Provider("boom".to_string()).into();
        assert_eq!(resp2.status, StatusCode::BAD_GATEWAY);
    }
}
