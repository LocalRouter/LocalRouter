//! Coding agent approval manager.
//!
//! Manages approval requests from AI coding agents (via the executors crate's
//! control protocol) and routes them to the Tauri popup UI.
//!
//! Follows the same DashMap + oneshot channel pattern as `FirewallManager`.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::protocol::JsonRpcNotification;
use lr_types::{AppError, AppResult};

/// Action taken by the user on a coding agent approval popup
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CodingAgentApprovalAction {
    /// Approve the tool/question
    Allow,
    /// Deny the tool/question
    Deny,
}

/// User response to a coding agent approval request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodingAgentApprovalResponse {
    pub action: CodingAgentApprovalAction,
    /// For questions: the user's answers (one per question)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<String>,
    /// Optional denial reason
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Type of approval request
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CodingAgentApprovalType {
    /// Tool usage approval (allow/deny)
    ToolApproval,
    /// Question requiring user answers
    Question,
}

/// A pending approval session
pub struct CodingAgentApprovalSession {
    pub request_id: String,
    pub session_id: String,
    pub approval_type: CodingAgentApprovalType,
    pub tool_name: String,
    pub question_count: usize,
    pub response_sender: Option<oneshot::Sender<CodingAgentApprovalResponse>>,
    pub created_at: Instant,
    pub timeout_seconds: u64,
}

/// Serializable info about a pending approval (for frontend display)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingCodingAgentApprovalInfo {
    pub request_id: String,
    pub session_id: String,
    pub approval_type: CodingAgentApprovalType,
    pub tool_name: String,
    pub question_count: usize,
    pub created_at_epoch_ms: u128,
    pub timeout_seconds: u64,
}

/// Manages coding agent approval sessions
pub struct CodingAgentApprovalManager {
    pending: Arc<DashMap<String, CodingAgentApprovalSession>>,
    default_timeout_secs: u64,
    notification_broadcast:
        Option<Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>>,
}

impl CodingAgentApprovalManager {
    pub fn new(default_timeout_secs: u64) -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            default_timeout_secs,
            notification_broadcast: None,
        }
    }

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

    /// Request approval for a coding agent tool call.
    /// Blocks until the user responds or timeout.
    pub async fn request_tool_approval(
        &self,
        session_id: String,
        tool_name: String,
        timeout_secs: Option<u64>,
    ) -> AppResult<CodingAgentApprovalResponse> {
        self.request_approval_internal(
            session_id,
            CodingAgentApprovalType::ToolApproval,
            tool_name,
            0,
            timeout_secs,
        )
        .await
    }

    /// Request answers to questions from a coding agent.
    /// Blocks until the user responds or timeout.
    pub async fn request_question_approval(
        &self,
        session_id: String,
        tool_name: String,
        question_count: usize,
        timeout_secs: Option<u64>,
    ) -> AppResult<CodingAgentApprovalResponse> {
        self.request_approval_internal(
            session_id,
            CodingAgentApprovalType::Question,
            tool_name,
            question_count,
            timeout_secs,
        )
        .await
    }

    async fn request_approval_internal(
        &self,
        session_id: String,
        approval_type: CodingAgentApprovalType,
        tool_name: String,
        question_count: usize,
        timeout_secs: Option<u64>,
    ) -> AppResult<CodingAgentApprovalResponse> {
        let request_id = Uuid::new_v4().to_string();
        let timeout = timeout_secs.unwrap_or(self.default_timeout_secs);

        let (tx, rx) = oneshot::channel();

        let session = CodingAgentApprovalSession {
            request_id: request_id.clone(),
            session_id: session_id.clone(),
            approval_type: approval_type.clone(),
            tool_name: tool_name.clone(),
            question_count,
            response_sender: Some(tx),
            created_at: Instant::now(),
            timeout_seconds: timeout,
        };

        self.pending.insert(request_id.clone(), session);

        info!(
            "Coding agent approval request {} created: session={}, type={:?}, tool={}",
            request_id, session_id, approval_type, tool_name
        );

        // Broadcast notification to UI
        if let Some(broadcast) = &self.notification_broadcast {
            let notification = JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: "coding_agent/approvalRequired".to_string(),
                params: Some(json!({
                    "request_id": request_id,
                    "session_id": session_id,
                    "approval_type": approval_type,
                    "tool_name": tool_name,
                    "question_count": question_count,
                    "timeout_seconds": timeout,
                })),
            };

            if let Err(e) = broadcast.send(("_coding_agent_approval".to_string(), notification)) {
                debug!("Failed to broadcast coding agent approval request: {}", e);
            }
        }

        // Wait for response with timeout
        match tokio::time::timeout(Duration::from_secs(timeout), rx).await {
            Ok(Ok(response)) => {
                info!(
                    "Received coding agent approval response for {}: {:?}",
                    request_id, response.action
                );
                self.pending.remove(&request_id);
                Ok(response)
            }
            Ok(Err(_)) => {
                warn!("Coding agent approval request {} cancelled", request_id);
                self.pending.remove(&request_id);
                Ok(CodingAgentApprovalResponse {
                    action: CodingAgentApprovalAction::Deny,
                    answers: Vec::new(),
                    reason: Some("Approval cancelled".to_string()),
                })
            }
            Err(_) => {
                warn!(
                    "Coding agent approval request {} timed out after {}s",
                    request_id, timeout
                );
                self.pending.remove(&request_id);
                Ok(CodingAgentApprovalResponse {
                    action: CodingAgentApprovalAction::Deny,
                    answers: Vec::new(),
                    reason: Some("Approval timed out".to_string()),
                })
            }
        }
    }

    /// Submit a user response to a pending approval request
    pub fn submit_response(
        &self,
        request_id: &str,
        response: CodingAgentApprovalResponse,
    ) -> AppResult<()> {
        match self.pending.remove(request_id) {
            Some((_, mut session)) => {
                debug!(
                    "Submitting coding agent approval for request {}: {:?}",
                    request_id, response.action
                );
                if let Some(sender) = session.response_sender.take() {
                    sender.send(response).map_err(|_| {
                        AppError::Internal(
                            "Failed to send coding agent approval response".to_string(),
                        )
                    })?;
                }
                Ok(())
            }
            None => Err(AppError::Internal(format!(
                "Coding agent approval request not found: {}",
                request_id
            ))),
        }
    }

    /// Get info about all pending approvals
    pub fn list_pending(&self) -> Vec<PendingCodingAgentApprovalInfo> {
        self.pending
            .iter()
            .map(|entry| {
                let s = entry.value();
                PendingCodingAgentApprovalInfo {
                    request_id: s.request_id.clone(),
                    session_id: s.session_id.clone(),
                    approval_type: s.approval_type.clone(),
                    tool_name: s.tool_name.clone(),
                    question_count: s.question_count,
                    created_at_epoch_ms: {
                        let now_epoch = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis();
                        let elapsed = s.created_at.elapsed().as_millis();
                        now_epoch.saturating_sub(elapsed)
                    },
                    timeout_seconds: s.timeout_seconds,
                }
            })
            .collect()
    }

    /// Clone for creating a new manager with the same broadcast
    pub fn clone_with_broadcast(&self) -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            default_timeout_secs: self.default_timeout_secs,
            notification_broadcast: self.notification_broadcast.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_submit_response_resolves_request() {
        let manager = CodingAgentApprovalManager::new(60);

        // Spawn a task that requests approval
        let manager_clone = CodingAgentApprovalManager {
            pending: manager.pending.clone(),
            default_timeout_secs: 60,
            notification_broadcast: None,
        };

        let handle = tokio::spawn(async move {
            manager_clone
                .request_tool_approval("session-1".to_string(), "Edit".to_string(), Some(5))
                .await
        });

        // Wait briefly for the request to be created
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Find and submit response
        let pending = manager.list_pending();
        assert_eq!(pending.len(), 1);
        let request_id = &pending[0].request_id;

        manager
            .submit_response(
                request_id,
                CodingAgentApprovalResponse {
                    action: CodingAgentApprovalAction::Allow,
                    answers: Vec::new(),
                    reason: None,
                },
            )
            .unwrap();

        let result = handle.await.unwrap().unwrap();
        assert_eq!(result.action, CodingAgentApprovalAction::Allow);
    }

    #[tokio::test]
    async fn test_timeout_auto_denies() {
        let manager = CodingAgentApprovalManager::new(1); // 1 second timeout

        let result = manager
            .request_tool_approval("session-1".to_string(), "Edit".to_string(), Some(1))
            .await
            .unwrap();

        assert_eq!(result.action, CodingAgentApprovalAction::Deny);
        assert!(result.reason.unwrap().contains("timed out"));
    }
}
