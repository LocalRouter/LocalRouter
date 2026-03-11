//! Sampling approval support for MCP Gateway
//!
//! Manages user approval flow for sampling requests when permission is set to "Ask".
//! Following the ElicitationManager pattern.
#![allow(dead_code)]

use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use tracing::{debug, info, warn};

use crate::protocol::SamplingRequest;
use lr_types::{AppError, AppResult};

/// User's decision on a sampling approval request
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SamplingApprovalAction {
    Allow,
    Deny,
}

/// Pending sampling approval session
#[derive(Debug)]
pub struct SamplingApprovalSession {
    /// Unique request ID
    pub request_id: String,

    /// Backend MCP server ID that initiated the sampling request
    pub server_id: String,

    /// The original sampling request
    pub sampling_request: SamplingRequest,

    /// When this approval request was created
    pub created_at: Instant,

    /// Timeout duration in seconds
    pub timeout_seconds: u64,

    /// Channel to send approval decision back to waiting handler
    pub response_sender: Option<oneshot::Sender<SamplingApprovalAction>>,
}

impl SamplingApprovalSession {
    /// Check if this session has expired
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > Duration::from_secs(self.timeout_seconds)
    }
}

/// Manages sampling approval lifecycle for MCP gateway
pub struct SamplingApprovalManager {
    /// Pending approval sessions (request_id -> session)
    pending: Arc<DashMap<String, SamplingApprovalSession>>,

    /// Default timeout for approval requests (seconds)
    default_timeout_secs: u64,
}

impl SamplingApprovalManager {
    /// Create a new sampling approval manager
    pub fn new(default_timeout_secs: u64) -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            default_timeout_secs,
        }
    }

    /// Request user approval for a sampling request
    ///
    /// Returns the user's decision (Allow/Deny) or an error on timeout/cancellation.
    pub async fn request_approval(
        &self,
        request_id: String,
        server_id: String,
        sampling_request: SamplingRequest,
        timeout_secs: Option<u64>,
    ) -> AppResult<SamplingApprovalAction> {
        let timeout = timeout_secs.unwrap_or(self.default_timeout_secs);

        debug!(
            "Creating sampling approval request {} for server {} (timeout: {}s)",
            request_id, server_id, timeout
        );

        // Create response channel
        let (tx, rx) = oneshot::channel();

        // Create session
        let session = SamplingApprovalSession {
            request_id: request_id.clone(),
            server_id: server_id.clone(),
            sampling_request,
            created_at: Instant::now(),
            timeout_seconds: timeout,
            response_sender: Some(tx),
        };

        // Store session
        self.pending.insert(request_id.clone(), session);

        info!(
            "Sampling approval request {} created for server {}",
            request_id, server_id
        );

        // Wait for response with timeout
        match tokio::time::timeout(Duration::from_secs(timeout), rx).await {
            Ok(Ok(action)) => {
                debug!(
                    "Received approval decision for request {}: {:?}",
                    request_id, action
                );
                self.pending.remove(&request_id);
                Ok(action)
            }
            Ok(Err(_)) => {
                // Channel closed without response (cancelled)
                warn!("Sampling approval request {} was cancelled", request_id);
                self.pending.remove(&request_id);
                Err(AppError::Internal(
                    "Sampling approval request was cancelled".to_string(),
                ))
            }
            Err(_) => {
                // Timeout
                warn!("Sampling approval request {} timed out", request_id);
                self.pending.remove(&request_id);
                Err(AppError::Internal(format!(
                    "Sampling approval request timed out after {} seconds",
                    timeout
                )))
            }
        }
    }

    /// Submit an approval decision for a pending request
    pub fn submit_approval(
        &self,
        request_id: &str,
        action: SamplingApprovalAction,
    ) -> AppResult<()> {
        match self.pending.remove(request_id) {
            Some((_, mut session)) => {
                debug!(
                    "Submitting approval {:?} for request {}",
                    action, request_id
                );

                if let Some(sender) = session.response_sender.take() {
                    sender.send(action).map_err(|_| {
                        AppError::Internal("Failed to send sampling approval decision".to_string())
                    })?;
                }

                info!("Approval submitted for sampling request {}", request_id);
                Ok(())
            }
            None => {
                warn!(
                    "Attempted to submit approval for unknown request {}",
                    request_id
                );
                Err(AppError::InvalidParams(format!(
                    "Sampling approval request {} not found or expired",
                    request_id
                )))
            }
        }
    }

    /// Cancel a pending approval request
    pub fn cancel(&self, request_id: &str) -> AppResult<()> {
        match self.pending.remove(request_id) {
            Some(_) => {
                info!("Cancelled sampling approval request {}", request_id);
                Ok(())
            }
            None => Err(AppError::InvalidParams(format!(
                "Sampling approval request {} not found",
                request_id
            ))),
        }
    }

    /// Get details of a pending request (for popup display)
    pub fn get_details(&self, request_id: &str) -> Option<SamplingApprovalDetails> {
        self.pending.get(request_id).map(|session| {
            let req = &session.sampling_request;
            SamplingApprovalDetails {
                request_id: session.request_id.clone(),
                server_id: session.server_id.clone(),
                message_count: req.messages.len(),
                system_prompt: req.system_prompt.clone(),
                model_preferences: req
                    .model_preferences
                    .as_ref()
                    .map(|p| serde_json::to_value(p).unwrap_or_default()),
                max_tokens: req.max_tokens.map(|v| v as u64),
                timeout_seconds: session.timeout_seconds,
                created_at_secs_ago: session.created_at.elapsed().as_secs(),
            }
        })
    }

    /// Get a list of all pending request IDs
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
            warn!(
                "Cleaning up expired sampling approval request {}",
                request_id
            );
            self.pending.remove(&request_id);
        }
    }

    /// Get the number of pending requests
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

/// Details of a sampling approval request (for popup display)
#[derive(Debug, Clone, serde::Serialize)]
pub struct SamplingApprovalDetails {
    pub request_id: String,
    pub server_id: String,
    pub message_count: usize,
    pub system_prompt: Option<String>,
    pub model_preferences: Option<serde_json::Value>,
    pub max_tokens: Option<u64>,
    pub timeout_seconds: u64,
    pub created_at_secs_ago: u64,
}

impl Default for SamplingApprovalManager {
    fn default() -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            default_timeout_secs: 120,
        }
    }
}

impl Clone for SamplingApprovalManager {
    fn clone(&self) -> Self {
        Self {
            pending: self.pending.clone(),
            default_timeout_secs: self.default_timeout_secs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{SamplingContent, SamplingMessage};

    fn make_test_request() -> SamplingRequest {
        SamplingRequest {
            messages: vec![SamplingMessage {
                role: "user".to_string(),
                content: SamplingContent::Text("Hello".to_string()),
            }],
            model_preferences: None,
            system_prompt: Some("You are a test assistant".to_string()),
            temperature: None,
            max_tokens: Some(100),
            stop_sequences: None,
            metadata: None,
        }
    }

    #[test]
    fn test_manager_creation() {
        let manager = SamplingApprovalManager::new(60);
        assert_eq!(manager.pending_count(), 0);
        assert_eq!(manager.default_timeout_secs, 60);
    }

    #[test]
    fn test_session_expiry() {
        let session = SamplingApprovalSession {
            request_id: "test-123".to_string(),
            server_id: "server-1".to_string(),
            sampling_request: make_test_request(),
            created_at: Instant::now() - Duration::from_secs(150),
            timeout_seconds: 120,
            response_sender: None,
        };
        assert!(session.is_expired());
    }

    #[tokio::test]
    async fn test_submit_approval_allow() {
        let manager = SamplingApprovalManager::new(120);

        let manager_clone = manager.clone();
        let handle = tokio::spawn(async move {
            manager_clone
                .request_approval(
                    "req-1".to_string(),
                    "server-1".to_string(),
                    make_test_request(),
                    None,
                )
                .await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(manager.pending_count(), 1);

        manager
            .submit_approval("req-1", SamplingApprovalAction::Allow)
            .unwrap();

        let result = handle.await.unwrap();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), SamplingApprovalAction::Allow);
    }

    #[tokio::test]
    async fn test_submit_approval_deny() {
        let manager = SamplingApprovalManager::new(120);

        let manager_clone = manager.clone();
        let handle = tokio::spawn(async move {
            manager_clone
                .request_approval(
                    "req-2".to_string(),
                    "server-1".to_string(),
                    make_test_request(),
                    None,
                )
                .await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        manager
            .submit_approval("req-2", SamplingApprovalAction::Deny)
            .unwrap();

        let result = handle.await.unwrap();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), SamplingApprovalAction::Deny);
    }

    #[tokio::test]
    async fn test_approval_timeout() {
        let manager = SamplingApprovalManager::new(1); // 1 second timeout

        let result = manager
            .request_approval(
                "req-timeout".to_string(),
                "server-1".to_string(),
                make_test_request(),
                None,
            )
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn test_cancel_approval() {
        let manager = SamplingApprovalManager::new(120);

        let manager_clone = manager.clone();
        let handle = tokio::spawn(async move {
            manager_clone
                .request_approval(
                    "req-cancel".to_string(),
                    "server-1".to_string(),
                    make_test_request(),
                    None,
                )
                .await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(manager.pending_count(), 1);

        manager.cancel("req-cancel").unwrap();
        assert_eq!(manager.pending_count(), 0);

        let result = handle.await.unwrap();
        assert!(result.is_err());
    }
}
