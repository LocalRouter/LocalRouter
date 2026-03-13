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
/// channel. The popup system (via `CodingAgentApprovalManager`) resolves it.
pub struct AskPopupApprovalService {
    /// Pending tool approvals: approval_id -> sender
    pending_approvals: Arc<DashMap<String, oneshot::Sender<ApprovalStatus>>>,
    /// Pending question approvals: approval_id -> sender
    pending_questions: Arc<DashMap<String, oneshot::Sender<QuestionStatus>>>,
    /// Callback to trigger popup (set by the gateway when wiring up)
    popup_callback: Option<Arc<dyn PopupTrigger>>,
}

/// Trait for triggering the popup UI from the approval service.
/// Implemented by the gateway layer that has access to the broadcast channel.
pub trait PopupTrigger: Send + Sync {
    fn trigger_tool_approval(&self, approval_id: &str, tool_name: &str);
    fn trigger_question_approval(
        &self,
        approval_id: &str,
        tool_name: &str,
        question_count: usize,
    );
}

impl AskPopupApprovalService {
    pub fn new() -> Self {
        Self {
            pending_approvals: Arc::new(DashMap::new()),
            pending_questions: Arc::new(DashMap::new()),
            popup_callback: None,
        }
    }

    pub fn with_popup_trigger(mut self, trigger: Arc<dyn PopupTrigger>) -> Self {
        self.popup_callback = Some(trigger);
        self
    }

    /// Resolve a pending tool approval (called by the popup system)
    pub fn resolve_tool_approval(&self, approval_id: &str, status: ApprovalStatus) {
        if let Some((_, sender)) = self.pending_approvals.remove(approval_id) {
            let _ = sender.send(status);
        }
    }

    /// Resolve a pending question (called by the popup system)
    pub fn resolve_question(&self, approval_id: &str, status: QuestionStatus) {
        if let Some((_, sender)) = self.pending_questions.remove(approval_id) {
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
    async fn create_tool_approval(&self, tool_name: &str) -> Result<String, ExecutorApprovalError> {
        let approval_id = uuid::Uuid::new_v4().to_string();
        let (tx, _) = oneshot::channel();
        self.pending_approvals
            .insert(approval_id.clone(), tx);

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
        let (tx, _) = oneshot::channel();
        self.pending_questions
            .insert(approval_id.clone(), tx);

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
        // Replace the existing sender with a fresh channel and get the receiver
        let rx = {
            let (tx, rx) = oneshot::channel();
            if let Some((_, old_tx)) = self.pending_approvals.remove(approval_id) {
                // Drop old sender (closes old channel)
                drop(old_tx);
            }
            self.pending_approvals
                .insert(approval_id.to_string(), tx);
            rx
        };

        tokio::select! {
            _ = cancel.cancelled() => {
                self.pending_approvals.remove(approval_id);
                Err(ExecutorApprovalError::Cancelled)
            }
            result = rx => {
                self.pending_approvals.remove(approval_id);
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
        let rx = {
            let (tx, rx) = oneshot::channel();
            if let Some((_, old_tx)) = self.pending_questions.remove(approval_id) {
                drop(old_tx);
            }
            self.pending_questions
                .insert(approval_id.to_string(), tx);
            rx
        };

        tokio::select! {
            _ = cancel.cancelled() => {
                self.pending_questions.remove(approval_id);
                Err(ExecutorApprovalError::Cancelled)
            }
            result = rx => {
                self.pending_questions.remove(approval_id);
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
        assert!(describe_mode(lr_config::CodingAgentApprovalMode::Elicitation).contains("elicitation"));
    }

    #[tokio::test]
    async fn test_ask_popup_service_create_and_resolve() {
        let service = AskPopupApprovalService::new();

        let approval_id = service
            .create_tool_approval("Edit")
            .await
            .unwrap();

        // Resolve it
        service.resolve_tool_approval(&approval_id, ApprovalStatus::Approved);

        // The pending map should be empty (resolved removes it on create side,
        // but wait_tool_approval does the actual receive)
    }

    #[tokio::test]
    async fn test_ask_popup_service_cancel() {
        let service = Arc::new(AskPopupApprovalService::new());
        let cancel = CancellationToken::new();

        let service_clone = service.clone();
        let cancel_clone = cancel.clone();
        let handle = tokio::spawn(async move {
            let id = service_clone
                .create_tool_approval("Edit")
                .await
                .unwrap();
            service_clone
                .wait_tool_approval(&id, cancel_clone)
                .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        cancel.cancel();

        let result = handle.await.unwrap();
        assert!(matches!(result, Err(ExecutorApprovalError::Cancelled)));
    }
}
