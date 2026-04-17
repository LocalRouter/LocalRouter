//! Error types and conversions

use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Router error: {0}")]
    Router(String),

    #[error("API key error: {0}")]
    ApiKey(String),

    #[error("MCP error: {0}")]
    Mcp(String),

    #[error("OAuth browser flow error: {0}")]
    OAuthBrowser(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Invalid parameters: {0}")]
    InvalidParams(String),

    #[error("Authentication failed")]
    Unauthorized,

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    /// Structured context-length error emitted by providers that can parse
    /// their own error bodies (e.g. OpenAI's `context_length_exceeded` code).
    /// The router matches on this variant directly instead of scraping the
    /// error text, so the real `max` is preserved all the way through.
    #[error("Context length exceeded{}{}",
        .max.map(|m| format!(" (max: {})", m)).unwrap_or_default(),
        .requested.map(|r| format!(" (requested: {})", r)).unwrap_or_default())]
    ContextLengthExceeded {
        max: Option<u64>,
        requested: Option<u64>,
    },

    /// Provider returned a structured "model not found" (e.g. HTTP 404 /
    /// OpenAI `model_not_found`). Router routes past it instead of
    /// misclassifying as ContextLengthExceeded.
    #[error("Model not found: {model}")]
    ModelNotFound { model: String },

    #[error("Free tier exhausted (retry after {retry_after_secs}s)")]
    FreeTierExhausted { retry_after_secs: u64 },

    #[error("Free tier exhausted, paid fallback available")]
    FreeTierFallbackAvailable {
        retry_after_secs: u64,
        exhausted_models: Vec<(String, String)>,
    },

    #[error("Internal server error: {0}")]
    Internal(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Cryptography error: {0}")]
    Crypto(#[from] anyhow::Error),
}

pub type AppResult<T> = Result<T, AppError>;

impl From<AppError> for String {
    fn from(err: AppError) -> String {
        err.to_string()
    }
}
