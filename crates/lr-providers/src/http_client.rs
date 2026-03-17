//! Shared HTTP client builders for providers
//!
//! Centralizes reqwest client construction with consistent defaults.
//! Uses a generous overall timeout (5 min) so streaming SSE responses
//! are not cut short, plus a fast connect timeout (10 s) for quick
//! failure when a provider is unreachable.

use lr_types::{AppError, AppResult};
use reqwest::Client;
use std::time::Duration;

/// Default client for standard API providers.
///
/// * connect_timeout 10 s – fail fast if the host is unreachable
/// * overall timeout 300 s – safety net; long enough for streaming
pub fn default_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .no_gzip()
        .build()
        .unwrap_or_default()
}

/// Extended-timeout client for providers that may have slower responses.
///
/// Same as `default_client` — the previous 120 s limit was too short for
/// streaming with tool-use payloads. Both now share the 300 s ceiling.
pub fn extended_client() -> AppResult<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .no_gzip()
        .build()
        .map_err(|e| AppError::Provider(format!("Failed to create HTTP client: {}", e)))
}

/// Format a reqwest streaming error with the full cause chain.
///
/// `reqwest::Error` Display only shows the top-level kind (e.g.
/// "error decoding response body") but omits the underlying cause.
/// This helper walks the source chain so the log/error message
/// includes the real reason (timeout, connection reset, etc.).
pub fn format_stream_error(e: &reqwest::Error) -> String {
    use std::error::Error;
    let mut msg = format!("Stream error: {}", e);
    let mut source = e.source();
    while let Some(cause) = source {
        msg.push_str(&format!(": {}", cause));
        source = cause.source();
    }
    msg
}

/// Short-timeout client for local service discovery (2s timeout).
pub fn discovery_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(2))
        .no_gzip()
        .build()
        .unwrap_or_default()
}
