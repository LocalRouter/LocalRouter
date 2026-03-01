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
    /// Create an approval request. Returns:
    /// - The question ID
    /// - A `PendingQuestion` to store on the session (contains the `Sender`)
    /// - A `oneshot::Receiver` for the caller to await the response
    pub fn create_approval(
        &self,
        session_id: &str,
        question_type: QuestionType,
        questions: Vec<QuestionItem>,
    ) -> (String, PendingQuestion, oneshot::Receiver<ApprovalResponse>) {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();

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

        (id, pending, rx)
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

        let (id, _pending, _rx) = router.create_approval(
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

    #[tokio::test]
    async fn test_approval_response_flows_through_channel() {
        let router = GatewayApprovalRouter::new();

        let (_id, pending, rx) = router.create_approval(
            "session-1",
            QuestionType::ToolApproval,
            vec![QuestionItem {
                question: "Allow Edit?".to_string(),
                options: vec!["allow".to_string(), "deny".to_string()],
            }],
        );

        // Send response through the channel
        let _ = pending.resolve.send(ApprovalResponse::Approved);

        // Receiver should get it
        let response = rx.await.unwrap();
        assert!(matches!(response, ApprovalResponse::Approved));
    }

    #[tokio::test]
    async fn test_approval_denied_with_reason() {
        let router = GatewayApprovalRouter::new();

        let (_id, pending, rx) = router.create_approval(
            "session-1",
            QuestionType::ToolApproval,
            vec![QuestionItem {
                question: "Allow Edit?".to_string(),
                options: vec!["allow".to_string(), "deny".to_string()],
            }],
        );

        let _ = pending.resolve.send(ApprovalResponse::Denied {
            reason: Some("too dangerous".to_string()),
        });

        let response = rx.await.unwrap();
        match response {
            ApprovalResponse::Denied { reason } => {
                assert_eq!(reason, Some("too dangerous".to_string()));
            }
            _ => panic!("Expected Denied response"),
        }
    }

    #[test]
    fn test_pending_for_multiple_sessions() {
        let router = GatewayApprovalRouter::new();

        let (id1, _p1, _rx1) = router.create_approval(
            "session-1",
            QuestionType::ToolApproval,
            vec![QuestionItem {
                question: "Q1".to_string(),
                options: vec![],
            }],
        );

        let (_id2, _p2, _rx2) = router.create_approval(
            "session-2",
            QuestionType::Question,
            vec![QuestionItem {
                question: "Q2".to_string(),
                options: vec!["a".to_string(), "b".to_string()],
            }],
        );

        let (_id3, _p3, _rx3) = router.create_approval(
            "session-1",
            QuestionType::PlanApproval,
            vec![QuestionItem {
                question: "Q3".to_string(),
                options: vec![],
            }],
        );

        assert_eq!(router.pending_for_session("session-1").len(), 2);
        assert_eq!(router.pending_for_session("session-2").len(), 1);

        router.remove_pending(&id1);
        assert_eq!(router.pending_for_session("session-1").len(), 1);
    }
}
