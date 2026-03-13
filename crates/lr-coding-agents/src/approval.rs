//! Approval routing for coding agent sessions.
//!
//! Bridges the executors crate's `ExecutorApprovalService` trait with
//! LocalRouter's approval system (Allow/Ask popup/Elicitation).

use lr_config::CodingAgentApprovalMode;

/// Describes the approval mode configuration.
/// The actual `ExecutorApprovalService` implementations will be wired
/// in Phase 3 when the popup and elicitation infrastructure is connected.
///
/// For now:
/// - `Allow` mode: executors' `NoopExecutorApprovalService` auto-approves
/// - `Ask` mode: will route to a new popup window (TODO: Phase 3)
/// - `Elicitation` mode: will forward via `ElicitationManager` (TODO: Phase 3)
pub fn describe_mode(mode: CodingAgentApprovalMode) -> &'static str {
    match mode {
        CodingAgentApprovalMode::Allow => {
            "Auto-approve all tool usage and questions (autonomous mode)"
        }
        CodingAgentApprovalMode::Ask => {
            "Show approval popup in LocalRouter UI for each tool/question request"
        }
        CodingAgentApprovalMode::Elicitation => {
            "Forward approval requests to MCP client via elicitation (falls back to Ask)"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_describe_mode() {
        assert!(describe_mode(CodingAgentApprovalMode::Allow).contains("Auto-approve"));
        assert!(describe_mode(CodingAgentApprovalMode::Ask).contains("popup"));
        assert!(describe_mode(CodingAgentApprovalMode::Elicitation).contains("elicitation"));
    }
}
