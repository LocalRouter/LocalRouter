use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use crate::mcp::manager::McpServerManager;
use crate::mcp::protocol::JsonRpcRequest;
use crate::utils::errors::AppError;
use serde_json::Value;

use super::types::ServerFailure;

type AppResult<T> = Result<T, AppError>;

/// Broadcast request to multiple servers in parallel with retry logic
pub async fn broadcast_request(
    server_ids: &[String],
    request: JsonRpcRequest,
    server_manager: &Arc<McpServerManager>,
    request_timeout: Duration,
    max_retries: u8,
) -> Vec<(String, AppResult<Value>)> {
    let futures = server_ids.iter().map(|server_id| {
        let request = request.clone();
        let server_id = server_id.clone();
        let server_manager = server_manager.clone();

        async move {
            let mut retries = 0;
            loop {
                let result = timeout(
                    request_timeout,
                    server_manager.send_request(&server_id, request.clone()),
                )
                .await;

                match result {
                    // Success
                    Ok(Ok(resp)) => return (server_id, Ok(resp.result.unwrap_or(Value::Null))),

                    // Request failed but we can retry
                    Ok(Err(e)) if retries < max_retries && is_retryable(&e) => {
                        retries += 1;
                        // Exponential backoff with cap at 10 seconds
                        let backoff_ms = (100 * (1 << retries)).min(10_000);
                        let backoff = Duration::from_millis(backoff_ms);
                        tokio::time::sleep(backoff).await;
                        continue;
                    }

                    // Request failed and we can't retry
                    Ok(Err(e)) => return (server_id, Err(e)),

                    // Timeout but we can retry
                    Err(_) if retries < max_retries => {
                        retries += 1;
                        // Add exponential backoff for timeouts too (was missing!)
                        let backoff_ms = (100 * (1 << retries)).min(10_000);
                        let backoff = Duration::from_millis(backoff_ms);
                        tokio::time::sleep(backoff).await;
                        continue;
                    }

                    // Timeout and no more retries
                    Err(_) => {
                        return (
                            server_id,
                            Err(AppError::Mcp(format!(
                                "Request timeout after {}ms",
                                request_timeout.as_millis()
                            ))),
                        )
                    }
                }
            }
        }
    });

    futures::future::join_all(futures).await
}

/// Check if an error is retryable
fn is_retryable(error: &AppError) -> bool {
    match error {
        // Network errors are typically retryable
        AppError::Mcp(msg) if msg.contains("timeout") => true,
        AppError::Mcp(msg) if msg.contains("connection") => true,

        // Auth errors, method not found, etc. are not retryable
        AppError::Mcp(msg) if msg.contains("auth") => false,
        AppError::Mcp(msg) if msg.contains("not found") => false,

        // Default to not retryable
        _ => false,
    }
}

/// Determine if a method should be broadcast to all servers
pub fn should_broadcast(method: &str) -> bool {
    matches!(
        method,
        "initialize"
            | "tools/list"
            | "resources/list"
            | "prompts/list"
            | "logging/setLevel"
            | "ping"
    )
}

/// Separate successful and failed results
pub fn separate_results<T>(
    results: Vec<(String, AppResult<T>)>,
) -> (Vec<(String, T)>, Vec<ServerFailure>) {
    let mut successes = Vec::new();
    let mut failures = Vec::new();

    for (server_id, result) in results {
        match result {
            Ok(value) => successes.push((server_id, value)),
            Err(error) => failures.push(ServerFailure {
                server_id,
                error: error.to_string(),
            }),
        }
    }

    (successes, failures)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_broadcast() {
        assert!(should_broadcast("initialize"));
        assert!(should_broadcast("tools/list"));
        assert!(should_broadcast("resources/list"));
        assert!(should_broadcast("prompts/list"));
        assert!(should_broadcast("logging/setLevel"));
        assert!(should_broadcast("ping"));

        assert!(!should_broadcast("tools/call"));
        assert!(!should_broadcast("resources/read"));
        assert!(!should_broadcast("prompts/get"));
    }

    #[test]
    fn test_is_retryable() {
        assert!(is_retryable(&AppError::Mcp("timeout error".to_string())));
        assert!(is_retryable(&AppError::Mcp(
            "connection failed".to_string()
        )));

        assert!(!is_retryable(&AppError::Mcp("auth failed".to_string())));
        assert!(!is_retryable(&AppError::Mcp(
            "method not found".to_string()
        )));
    }

    #[test]
    fn test_separate_results() {
        let results: Vec<(String, AppResult<String>)> = vec![
            ("server1".to_string(), Ok("success".to_string())),
            (
                "server2".to_string(),
                Err(AppError::Mcp("failed".to_string())),
            ),
            ("server3".to_string(), Ok("another success".to_string())),
        ];

        let (successes, failures) = separate_results(results);

        assert_eq!(successes.len(), 2);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].server_id, "server2");
    }
}
