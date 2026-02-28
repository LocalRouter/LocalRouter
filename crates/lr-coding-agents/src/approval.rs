//! Approval routing for coding agent sessions.
//!
//! Routes tool/plan approvals and questions from the agent process
//! either through MCP elicitation (if the client supports it) or
//! via the polling pattern ({agent}_status + {agent}_respond).

use crate::types::*;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::oneshot;
use tracing::debug;

/// Routes approvals from agent processes to MCP clients.
///
/// When an agent requests approval (e.g., to run a tool), this router:
/// 1. Creates a `PendingQuestion` with a oneshot channel
/// 2. Stores it so `{agent}_status` can surface it
/// 3. Blocks until `{agent}_respond` resolves the channel
pub struct GatewayApprovalRouter {
    /// Pending questions indexed by question ID
    pending: Arc<DashMap<String, PendingQuestionEntry>>,
}

struct PendingQuestionEntry {
    pub session_id: String,
    pub info: PendingQuestionInfo,
}

impl GatewayApprovalRouter {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
        }
    }

    /// Create a tool approval request and return its ID.
    /// The caller should then store the PendingQuestion on the session
    /// and block on the oneshot receiver.
    pub fn create_approval(
        &self,
        session_id: &str,
        question_type: QuestionType,
        questions: Vec<QuestionItem>,
    ) -> (String, PendingQuestion) {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, _rx) = oneshot::channel();

        let pending = PendingQuestion {
            id: id.clone(),
            question_type: question_type.clone(),
            questions: questions.clone(),
            resolve: tx,
        };

        let info = PendingQuestionInfo {
            id: id.clone(),
            question_type,
            questions,
        };

        self.pending.insert(
            id.clone(),
            PendingQuestionEntry {
                session_id: session_id.to_string(),
                info,
            },
        );

        debug!(question_id = %id, session_id = %session_id, "Created approval request");

        (id, pending)
    }

    /// Remove a pending question (called after it's resolved)
    pub fn remove_pending(&self, question_id: &str) {
        self.pending.remove(question_id);
    }

    /// Get pending questions for a session
    pub fn pending_for_session(&self, session_id: &str) -> Vec<PendingQuestionInfo> {
        self.pending
            .iter()
            .filter(|entry| entry.value().session_id == session_id)
            .map(|entry| entry.value().info.clone())
            .collect()
    }
}

impl Default for GatewayApprovalRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_remove_approval() {
        let router = GatewayApprovalRouter::new();

        let (id, _pending) = router.create_approval(
            "session-1",
            QuestionType::ToolApproval,
            vec![QuestionItem {
                question: "Allow Edit on auth.ts?".to_string(),
                options: vec!["allow".to_string(), "deny".to_string()],
            }],
        );

        assert_eq!(router.pending_for_session("session-1").len(), 1);
        assert_eq!(router.pending_for_session("session-2").len(), 0);

        router.remove_pending(&id);
        assert_eq!(router.pending_for_session("session-1").len(), 0);
    }
}
