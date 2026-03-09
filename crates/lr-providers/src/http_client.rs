//! Shared HTTP client builders for providers
//!
//! Centralizes reqwest client construction with consistent defaults.
//! Auto-decompression is disabled to prevent "error decoding response body"
//! failures during SSE streaming.

use lr_types::{AppError, AppResult};
use reqwest::Client;
use std::time::Duration;

/// Default client for standard API providers (60s timeout).
pub fn default_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(60))
        .no_gzip()
        .build()
        .unwrap_or_default()
}

/// Extended-timeout client for providers that may have slower responses (120s timeout).
pub fn extended_client() -> AppResult<Client> {
    Client::builder()
        .timeout(Duration::from_secs(120))
        .no_gzip()
        .build()
        .map_err(|e| AppError::Provider(format!("Failed to create HTTP client: {}", e)))
}

/// Short-timeout client for local service discovery (2s timeout).
pub fn discovery_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(2))
        .no_gzip()
        .build()
        .unwrap_or_default()
}
