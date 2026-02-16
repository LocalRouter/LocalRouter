//! Type definitions for the guardrails engine
//!
//! Re-exports from safety_model.rs plus approval-specific types.

use serde::{Deserialize, Serialize};

// Re-export core types from safety_model
pub use crate::safety_model::{
    CategoryAction, CategoryActionRequired, FlaggedCategory, SafetyCategory, SafetyCheckResult,
    SafetyVerdict, ScanDirection,
};

/// Details sent to the approval popup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailApprovalDetails {
    pub verdicts: Vec<SafetyVerdict>,
    pub actions_required: Vec<CategoryActionRequired>,
    pub total_duration_ms: u64,
    pub scan_direction: String,
    /// The text content that was scanned and triggered the guardrail
    pub flagged_text: String,
}
