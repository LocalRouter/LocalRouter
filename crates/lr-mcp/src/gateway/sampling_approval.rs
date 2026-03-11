//! Sampling approval support for MCP Gateway
//!
//! Manages user approval flow for sampling requests when permission is set to "Ask".
//! Following the ElicitationManager pattern.
#![allow(dead_code)]

use dashmap::DashMap;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use tracing::{debug, error, info, warn};

use crate::protocol::{JsonRpcNotification, SamplingRequest};
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

    /// Broadcast sender for notifications (optional)
    notification_broadcast:
        Option<Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>>,
}

impl SamplingApprovalManager {
    /// Create a new sampling approval manager
    pub fn new(default_timeout_secs: u64) -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            default_timeout_secs,
            notification_broadcast: None,
        }
    }

    /// Create a new sampling approval manager with notification broadcast support
    pub fn new_with_broadcast(
        default_timeout_secs: u64,
        notification_broadcast: Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>,
    ) -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            default_timeout_secs,
            notification_broadcast: Some(notification_broadcast),
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

        // Broadcast notification to listeners (Tauri popup opener)
        if let Some(broadcast) = &self.notification_broadcast {
            let notification = JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: "sampling/approvalRequired".to_string(),
                params: Some(json!({
                    "request_id": request_id,
                    "server_id": server_id,
                    "timeout_seconds": timeout,
                })),
            };

            if let Err(e) = broadcast.send(("_sampling_approval".to_string(), notification)) {
                error!("Failed to broadcast sampling approval request: {}", e);
            } else {
                debug!(
                    "Broadcasted sampling approval request {} via notification",
                    request_id
                );
            }
        }

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

// ---------------------------------------------------------------------------
// Sampling passthrough for Both/McpOnly mode
// ---------------------------------------------------------------------------

/// Pending sampling passthrough session.
/// Used when a backend server requests sampling and the client mode is Both/McpOnly.
/// The request is forwarded to the external client, which processes it and responds.
#[derive(Debug)]
pub struct SamplingPassthroughSession {
    /// Unique request ID
    pub request_id: String,
    /// When this request was created
    pub created_at: Instant,
    /// Timeout duration in seconds
    pub timeout_seconds: u64,
    /// Channel to receive the response
    pub response_sender: Option<oneshot::Sender<serde_json::Value>>,
}

impl SamplingPassthroughSession {
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > Duration::from_secs(self.timeout_seconds)
    }
}

/// Manages sampling passthrough for Both/McpOnly clients.
/// Forwards sampling requests to external clients and waits for their responses.
pub struct SamplingPassthroughManager {
    pending: Arc<DashMap<String, SamplingPassthroughSession>>,
    default_timeout_secs: u64,
}

impl SamplingPassthroughManager {
    pub fn new(default_timeout_secs: u64) -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            default_timeout_secs,
        }
    }

    /// Create a pending passthrough session and return a receiver for the response.
    ///
    /// The caller should:
    /// 1. Call this to get a (request_id, receiver)
    /// 2. Send the sampling request to the external client via SSE notification
    /// 3. Await the receiver for the response
    pub fn create_pending(
        &self,
        timeout_secs: Option<u64>,
    ) -> (String, tokio::sync::oneshot::Receiver<serde_json::Value>) {
        let request_id = uuid::Uuid::new_v4().to_string();
        let timeout = timeout_secs.unwrap_or(self.default_timeout_secs);

        let (tx, rx) = oneshot::channel();

        let session = SamplingPassthroughSession {
            request_id: request_id.clone(),
            created_at: Instant::now(),
            timeout_seconds: timeout,
            response_sender: Some(tx),
        };

        self.pending.insert(request_id.clone(), session);
        debug!("Created sampling passthrough session {}", request_id);

        (request_id, rx)
    }

    /// Submit a response from the external client
    pub fn submit_response(
        &self,
        request_id: &str,
        response: serde_json::Value,
    ) -> lr_types::AppResult<()> {
        match self.pending.remove(request_id) {
            Some((_, mut session)) => {
                if let Some(sender) = session.response_sender.take() {
                    sender.send(response).map_err(|_| {
                        AppError::Internal(
                            "Failed to send sampling passthrough response".to_string(),
                        )
                    })?;
                }
                debug!("Submitted passthrough response for {}", request_id);
                Ok(())
            }
            None => Err(AppError::InvalidParams(format!(
                "Sampling passthrough request {} not found or expired",
                request_id
            ))),
        }
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
                "Cleaning up expired sampling passthrough session {}",
                request_id
            );
            self.pending.remove(&request_id);
        }
    }
}

impl Default for SamplingPassthroughManager {
    fn default() -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            default_timeout_secs: 120,
        }
    }
}

impl Clone for SamplingPassthroughManager {
    fn clone(&self) -> Self {
        Self {
            pending: self.pending.clone(),
            default_timeout_secs: self.default_timeout_secs,
        }
    }
}

impl Default for SamplingApprovalManager {
    fn default() -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            default_timeout_secs: 120,
            notification_broadcast: None,
        }
    }
}

impl Clone for SamplingApprovalManager {
    fn clone(&self) -> Self {
        Self {
            pending: self.pending.clone(),
            default_timeout_secs: self.default_timeout_secs,
            notification_broadcast: self.notification_broadcast.clone(),
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

    // --- SamplingPassthroughManager tests ---

    #[tokio::test]
    async fn test_passthrough_submit_response() {
        let manager = SamplingPassthroughManager::new(120);
        let (request_id, rx) = manager.create_pending(None);

        let response_val = serde_json::json!({
            "model": "gpt-4",
            "role": "assistant",
            "content": { "type": "text", "text": "Hello!" }
        });

        manager
            .submit_response(&request_id, response_val.clone())
            .unwrap();

        let received = rx.await.unwrap();
        assert_eq!(received, response_val);
    }

    #[tokio::test]
    async fn test_passthrough_timeout() {
        let manager = SamplingPassthroughManager::new(1);
        let (_request_id, rx) = manager.create_pending(Some(1));

        // Don't submit a response, just wait for timeout
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), rx).await;
        // The receiver should error because the sender was dropped during cleanup
        // or timeout on the receiver side
        assert!(result.is_err() || result.unwrap().is_err());
    }

    #[test]
    fn test_passthrough_submit_unknown_id() {
        let manager = SamplingPassthroughManager::new(120);
        let result = manager.submit_response("nonexistent", serde_json::json!({}));
        assert!(result.is_err());
    }
}
