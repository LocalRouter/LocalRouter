//! Approval routing for coding agent sessions.
//!
//! Implements the executors crate's `ExecutorApprovalService` trait to bridge
//! agent tool/question approvals into LocalRouter's UI popup system.
//!
//! Three modes:
//! - **Allow**: Auto-approve everything (uses executors' `NoopExecutorApprovalService`)
//! - **Ask**: Route to LocalRouter's popup UI via `CodingAgentApprovalManager`
//! - **Elicitation**: Forward via MCP elicitation to the client (falls back to Ask)

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use executors::approvals::{ExecutorApprovalError, ExecutorApprovalService};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use workspace_utils::approvals::{ApprovalStatus, QuestionStatus};

/// Approval service that routes to LocalRouter's popup UI.
///
/// Each `create_*` call generates a unique approval ID and stores a oneshot
/// receiver. The `wait_*` method awaits on that receiver. The popup system
/// resolves it by calling `resolve_*` which sends on the stored sender.
pub struct AskPopupApprovalService {
    /// Pending tool approvals: approval_id -> (sender, receiver)
    /// Sender is held so `resolve_tool_approval` can send on it.
    /// Receiver is consumed by `wait_tool_approval`.
    pending_tool_senders: Arc<DashMap<String, oneshot::Sender<ApprovalStatus>>>,
    pending_tool_receivers: Arc<DashMap<String, oneshot::Receiver<ApprovalStatus>>>,
    /// Pending question approvals
    pending_question_senders: Arc<DashMap<String, oneshot::Sender<QuestionStatus>>>,
    pending_question_receivers: Arc<DashMap<String, oneshot::Receiver<QuestionStatus>>>,
    /// Callback to trigger popup (set by the gateway when wiring up)
    popup_callback: Option<Arc<dyn PopupTrigger>>,
}

/// Trait for triggering the popup UI from the approval service.
/// Implemented by the gateway layer that has access to the broadcast channel.
pub trait PopupTrigger: Send + Sync {
    fn trigger_tool_approval(&self, approval_id: &str, tool_name: &str);
    fn trigger_question_approval(&self, approval_id: &str, tool_name: &str, question_count: usize);
}

impl AskPopupApprovalService {
    pub fn new() -> Self {
        Self {
            pending_tool_senders: Arc::new(DashMap::new()),
            pending_tool_receivers: Arc::new(DashMap::new()),
            pending_question_senders: Arc::new(DashMap::new()),
            pending_question_receivers: Arc::new(DashMap::new()),
            popup_callback: None,
        }
    }

    pub fn with_popup_trigger(mut self, trigger: Arc<dyn PopupTrigger>) -> Self {
        self.popup_callback = Some(trigger);
        self
    }

    /// Resolve a pending tool approval (called by the popup system)
    pub fn resolve_tool_approval(&self, approval_id: &str, status: ApprovalStatus) {
        if let Some((_, sender)) = self.pending_tool_senders.remove(approval_id) {
            let _ = sender.send(status);
        }
    }

    /// Resolve a pending question (called by the popup system)
    pub fn resolve_question(&self, approval_id: &str, status: QuestionStatus) {
        if let Some((_, sender)) = self.pending_question_senders.remove(approval_id) {
            let _ = sender.send(status);
        }
    }
}

impl Default for AskPopupApprovalService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExecutorApprovalService for AskPopupApprovalService {
    async fn create_tool_approval(
        &self,
        tool_name: &str,
        _tool_input: Option<&serde_json::Value>,
    ) -> Result<String, ExecutorApprovalError> {
        // The popup-based service ignores tool_input today — its UI
        // surfaces only the tool name. Hosts that want richer prompts
        // (e.g. Direktor's bus-backed bridge) can use it.
        let approval_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending_tool_senders.insert(approval_id.clone(), tx);
        self.pending_tool_receivers.insert(approval_id.clone(), rx);

        if let Some(ref trigger) = self.popup_callback {
            trigger.trigger_tool_approval(&approval_id, tool_name);
        }

        debug!(
            approval_id = %approval_id,
            tool = %tool_name,
            "Created tool approval request"
        );
        Ok(approval_id)
    }

    async fn create_question_approval(
        &self,
        tool_name: &str,
        question_count: usize,
    ) -> Result<String, ExecutorApprovalError> {
        let approval_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending_question_senders
            .insert(approval_id.clone(), tx);
        self.pending_question_receivers
            .insert(approval_id.clone(), rx);

        if let Some(ref trigger) = self.popup_callback {
            trigger.trigger_question_approval(&approval_id, tool_name, question_count);
        }

        debug!(
            approval_id = %approval_id,
            tool = %tool_name,
            questions = question_count,
            "Created question approval request"
        );
        Ok(approval_id)
    }

    async fn wait_tool_approval(
        &self,
        approval_id: &str,
        cancel: CancellationToken,
    ) -> Result<ApprovalStatus, ExecutorApprovalError> {
        // Take the receiver that was stored by create_tool_approval
        let rx = self
            .pending_tool_receivers
            .remove(approval_id)
            .map(|(_, rx)| rx)
            .ok_or_else(|| {
                ExecutorApprovalError::RequestFailed(format!(
                    "No pending tool approval for {}",
                    approval_id
                ))
            })?;

        tokio::select! {
            _ = cancel.cancelled() => {
                self.pending_tool_senders.remove(approval_id);
                Err(ExecutorApprovalError::Cancelled)
            }
            result = rx => {
                self.pending_tool_senders.remove(approval_id);
                match result {
                    Ok(status) => Ok(status),
                    Err(_) => {
                        warn!("Tool approval channel closed for {}", approval_id);
                        Err(ExecutorApprovalError::RequestFailed("Channel closed".to_string()))
                    }
                }
            }
        }
    }

    async fn wait_question_answer(
        &self,
        approval_id: &str,
        cancel: CancellationToken,
    ) -> Result<QuestionStatus, ExecutorApprovalError> {
        let rx = self
            .pending_question_receivers
            .remove(approval_id)
            .map(|(_, rx)| rx)
            .ok_or_else(|| {
                ExecutorApprovalError::RequestFailed(format!(
                    "No pending question approval for {}",
                    approval_id
                ))
            })?;

        tokio::select! {
            _ = cancel.cancelled() => {
                self.pending_question_senders.remove(approval_id);
                Err(ExecutorApprovalError::Cancelled)
            }
            result = rx => {
                self.pending_question_senders.remove(approval_id);
                match result {
                    Ok(status) => Ok(status),
                    Err(_) => {
                        warn!("Question approval channel closed for {}", approval_id);
                        Ok(QuestionStatus::TimedOut)
                    }
                }
            }
        }
    }
}

/// Describes the approval mode configuration.
pub fn describe_mode(mode: lr_config::CodingAgentApprovalMode) -> &'static str {
    match mode {
        lr_config::CodingAgentApprovalMode::Allow => {
            "Auto-approve all tool usage and questions (autonomous mode)"
        }
        lr_config::CodingAgentApprovalMode::Ask => {
            "Show approval popup in LocalRouter UI for each tool/question request"
        }
        lr_config::CodingAgentApprovalMode::Elicitation => {
            "Forward approval requests to MCP client via elicitation (falls back to Ask)"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_describe_mode() {
        assert!(describe_mode(lr_config::CodingAgentApprovalMode::Allow).contains("Auto-approve"));
        assert!(describe_mode(lr_config::CodingAgentApprovalMode::Ask).contains("popup"));
        assert!(
            describe_mode(lr_config::CodingAgentApprovalMode::Elicitation).contains("elicitation")
        );
    }

    #[tokio::test]
    async fn test_ask_popup_service_create_wait_resolve() {
        let service = Arc::new(AskPopupApprovalService::new());

        let service_clone = service.clone();
        let handle = tokio::spawn(async move {
            let id = service_clone.create_tool_approval("Edit", None).await.unwrap();
            let cancel = CancellationToken::new();
            service_clone.wait_tool_approval(&id, cancel).await
        });

        // Wait for the request to be created
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Resolve it via the sender
        let pending_ids: Vec<String> = service
            .pending_tool_senders
            .iter()
            .map(|e| e.key().clone())
            .collect();
        assert_eq!(pending_ids.len(), 1);
        service.resolve_tool_approval(&pending_ids[0], ApprovalStatus::Approved);

        let result = handle.await.unwrap().unwrap();
        assert!(matches!(result, ApprovalStatus::Approved));
    }

    #[tokio::test]
    async fn test_ask_popup_service_cancel() {
        let service = Arc::new(AskPopupApprovalService::new());
        let cancel = CancellationToken::new();

        let service_clone = service.clone();
        let cancel_clone = cancel.clone();
        let handle = tokio::spawn(async move {
            let id = service_clone.create_tool_approval("Edit", None).await.unwrap();
            service_clone.wait_tool_approval(&id, cancel_clone).await
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        cancel.cancel();

        let result = handle.await.unwrap();
        assert!(matches!(result, Err(ExecutorApprovalError::Cancelled)));
    }
}
