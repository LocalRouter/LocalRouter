//! Elicitation support for MCP Gateway
//!
//! Enables backend MCP servers to request structured user input during tool execution.
//! Supports WebSocket notifications (primary) and HTTP callbacks (fallback).

use dashmap::DashMap;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::mcp::protocol::{ElicitationRequest, ElicitationResponse};
use crate::utils::errors::{AppError, AppResult};

/// Pending elicitation session
#[derive(Debug)]
pub struct ElicitationSession {
    /// Unique request ID
    pub request_id: String,

    /// Backend MCP server ID that initiated the request
    pub server_id: String,

    /// Message to display to user
    pub message: String,

    /// JSON Schema for validating user response
    pub schema: Value,

    /// When this request was created
    pub created_at: Instant,

    /// Timeout duration in seconds
    pub timeout_seconds: u64,

    /// Channel to send response back to waiting request
    pub response_sender: Option<oneshot::Sender<ElicitationResponse>>,
}

impl ElicitationSession {
    /// Check if this session has expired
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > Duration::from_secs(self.timeout_seconds)
    }
}

/// Manages elicitation lifecycle for MCP gateway
pub struct ElicitationManager {
    /// Pending elicitation sessions (request_id -> session)
    pending: Arc<DashMap<String, ElicitationSession>>,

    /// Default timeout for elicitation requests (seconds)
    default_timeout_secs: u64,
}

impl ElicitationManager {
    /// Create a new elicitation manager
    pub fn new(default_timeout_secs: u64) -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            default_timeout_secs,
        }
    }

    /// Request user input from external client
    ///
    /// This is an async operation that waits for the user response.
    /// Returns an error if the request times out or is cancelled.
    pub async fn request_input(
        &self,
        server_id: String,
        request: ElicitationRequest,
        timeout_secs: Option<u64>,
    ) -> AppResult<ElicitationResponse> {
        let request_id = Uuid::new_v4().to_string();
        let timeout = timeout_secs.unwrap_or(self.default_timeout_secs);

        debug!(
            "Creating elicitation request {} for server {} (timeout: {}s)",
            request_id, server_id, timeout
        );

        // Create response channel
        let (tx, rx) = oneshot::channel();

        // Create session
        let session = ElicitationSession {
            request_id: request_id.clone(),
            server_id: server_id.clone(),
            message: request.message.clone(),
            schema: request.schema.clone(),
            created_at: Instant::now(),
            timeout_seconds: timeout,
            response_sender: Some(tx),
        };

        // Store session
        self.pending.insert(request_id.clone(), session);

        info!(
            "Elicitation request {} created for server {}",
            request_id, server_id
        );

        // TODO: Send WebSocket notification to external clients
        // For now, this will wait for manual response submission

        // Wait for response with timeout
        match tokio::time::timeout(Duration::from_secs(timeout), rx).await {
            Ok(Ok(response)) => {
                debug!("Received response for elicitation request {}", request_id);
                self.pending.remove(&request_id);
                Ok(response)
            }
            Ok(Err(_)) => {
                // Channel closed without response (cancelled)
                warn!("Elicitation request {} was cancelled", request_id);
                self.pending.remove(&request_id);
                Err(AppError::Internal(
                    "Elicitation request was cancelled".to_string(),
                ))
            }
            Err(_) => {
                // Timeout
                warn!("Elicitation request {} timed out", request_id);
                self.pending.remove(&request_id);
                Err(AppError::Internal(format!(
                    "Elicitation request timed out after {} seconds",
                    timeout
                )))
            }
        }
    }

    /// Submit a user response to a pending elicitation request
    pub fn submit_response(
        &self,
        request_id: &str,
        response: ElicitationResponse,
    ) -> AppResult<()> {
        match self.pending.remove(request_id) {
            Some((_, mut session)) => {
                // TODO: Validate response against schema

                debug!("Submitting response for elicitation request {}", request_id);

                if let Some(sender) = session.response_sender.take() {
                    sender.send(response).map_err(|_| {
                        AppError::Internal("Failed to send elicitation response".to_string())
                    })?;
                }

                info!(
                    "Response submitted for elicitation request {}",
                    request_id
                );
                Ok(())
            }
            None => {
                warn!(
                    "Attempted to submit response for unknown request {}",
                    request_id
                );
                Err(AppError::InvalidParams(format!(
                    "Elicitation request {} not found or expired",
                    request_id
                )))
            }
        }
    }

    /// Cancel a pending elicitation request
    pub fn cancel_request(&self, request_id: &str) -> AppResult<()> {
        match self.pending.remove(request_id) {
            Some(_) => {
                info!("Cancelled elicitation request {}", request_id);
                Ok(())
            }
            None => Err(AppError::InvalidParams(format!(
                "Elicitation request {} not found",
                request_id
            ))),
        }
    }

    /// Get a list of all pending requests (for debugging/monitoring)
    pub fn list_pending(&self) -> Vec<String> {
        self.pending
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Clean up expired sessions
    pub fn cleanup_expired(&self) {
        let expired: Vec<String> = self
            .pending
            .iter()
            .filter(|entry| entry.value().is_expired())
            .map(|entry| entry.key().clone())
            .collect();

        for request_id in expired {
            warn!("Cleaning up expired elicitation request {}", request_id);
            self.pending.remove(&request_id);
        }
    }

    /// Get the number of pending requests
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

impl Default for ElicitationManager {
    fn default() -> Self {
        Self::new(120) // 2 minute default timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_elicitation_manager_creation() {
        let manager = ElicitationManager::new(60);
        assert_eq!(manager.pending_count(), 0);
        assert_eq!(manager.default_timeout_secs, 60);
    }

    #[test]
    fn test_session_expiry() {
        let session = ElicitationSession {
            request_id: "test-123".to_string(),
            server_id: "server-1".to_string(),
            message: "Test message".to_string(),
            schema: json!({"type": "object"}),
            created_at: Instant::now() - Duration::from_secs(150),
            timeout_seconds: 120,
            response_sender: None,
        };

        assert!(session.is_expired());
    }

    #[test]
    fn test_session_not_expired() {
        let session = ElicitationSession {
            request_id: "test-123".to_string(),
            server_id: "server-1".to_string(),
            message: "Test message".to_string(),
            schema: json!({"type": "object"}),
            created_at: Instant::now(),
            timeout_seconds: 120,
            response_sender: None,
        };

        assert!(!session.is_expired());
    }

    #[tokio::test]
    async fn test_cancel_request() {
        let manager = ElicitationManager::new(120);

        // Start a request in the background
        let manager_clone = manager.clone();
        let handle = tokio::spawn(async move {
            let request = ElicitationRequest {
                message: "Test".to_string(),
                schema: json!({"type": "string"}),
            };

            manager_clone
                .request_input("server-1".to_string(), request, None)
                .await
        });

        // Give it time to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Should have 1 pending
        assert_eq!(manager.pending_count(), 1);

        // Cancel the request
        let request_id = manager.list_pending()[0].clone();
        manager.cancel_request(&request_id).unwrap();

        // Should be cancelled now
        assert_eq!(manager.pending_count(), 0);

        // The request should fail
        let result = handle.await.unwrap();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_submit_response() {
        let manager = ElicitationManager::new(120);

        // Start a request in the background
        let manager_clone = manager.clone();
        let handle = tokio::spawn(async move {
            let request = ElicitationRequest {
                message: "Enter your name".to_string(),
                schema: json!({"type": "string"}),
            };

            manager_clone
                .request_input("server-1".to_string(), request, None)
                .await
        });

        // Give it time to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Submit response
        let request_id = manager.list_pending()[0].clone();
        let response = ElicitationResponse {
            data: json!("John Doe"),
        };

        manager.submit_response(&request_id, response).unwrap();

        // Should complete successfully
        let result = handle.await.unwrap();
        assert!(result.is_ok());
        assert_eq!(result.unwrap().data, json!("John Doe"));
    }

    #[tokio::test]
    async fn test_timeout() {
        let manager = ElicitationManager::new(1); // 1 second timeout

        let request = ElicitationRequest {
            message: "This will timeout".to_string(),
            schema: json!({"type": "string"}),
        };

        let result = manager
            .request_input("server-1".to_string(), request, None)
            .await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("timed out"));
    }
}

// Implement Clone for ElicitationManager to support test scenarios
impl Clone for ElicitationManager {
    fn clone(&self) -> Self {
        Self {
            pending: self.pending.clone(),
            default_timeout_secs: self.default_timeout_secs,
        }
    }
}
