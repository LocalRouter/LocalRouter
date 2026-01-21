//! RouteLLM-specific error types

use thiserror::Error;

/// RouteLLM-specific errors
#[derive(Error, Debug)]
pub enum RouteLLMError {
    #[error("Model not downloaded: {0}")]
    ModelNotDownloaded(String),

    #[error("Model loading failed: {0}")]
    ModelLoadingFailed(String),

    #[error("Prediction failed: {0}")]
    PredictionFailed(String),

    #[error("Download failed: {0}")]
    DownloadFailed(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

pub type RouteLLMResult<T> = Result<T, RouteLLMError>;
