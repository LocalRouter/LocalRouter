//! Safety model trait and types for LLM-based content classification
//!
//! Defines the `SafetyModel` trait that all safety model implementations must satisfy,
//! plus the unified category system, verdict types, and check input/output structures.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Unified safety categories across all supported models.
///
/// Categories are mapped from native model labels (e.g. Llama Guard S1-S14,
/// ShieldGemma categories, Nemotron S1-S23, Granite Guardian risks).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SafetyCategory {
    // Llama Guard 4 (S1-S14)
    ViolentCrimes,
    NonViolentCrimes,
    SexCrimes,
    ChildExploitation,
    Defamation,
    SpecializedAdvice,
    Privacy,
    IntellectualProperty,
    IndiscriminateWeapons,
    Hate,
    SelfHarm,
    SexualContent,
    Elections,
    CodeInterpreterAbuse,
    // ShieldGemma additions
    DangerousContent,
    Harassment,
    // Nemotron additions
    CriminalPlanning,
    GunsIllegalWeapons,
    ControlledSubstances,
    Profanity,
    NeedsCaution,
    Manipulation,
    FraudDeception,
    Malware,
    HighRiskGovDecision,
    PoliticalMisinformation,
    CopyrightPlagiarism,
    UnauthorizedAdvice,
    IllegalActivity,
    ImmoralUnethical,
    // Granite Guardian additions
    SocialBias,
    Jailbreak,
    UnethicalBehavior,
    // RAG risks (Granite)
    ContextRelevance,
    Groundedness,
    AnswerRelevance,
    // Custom/fallback
    Custom(String),
}

impl fmt::Display for SafetyCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ViolentCrimes => write!(f, "Violent Crimes"),
            Self::NonViolentCrimes => write!(f, "Non-Violent Crimes"),
            Self::SexCrimes => write!(f, "Sex Crimes"),
            Self::ChildExploitation => write!(f, "Child Exploitation"),
            Self::Defamation => write!(f, "Defamation"),
            Self::SpecializedAdvice => write!(f, "Specialized Advice"),
            Self::Privacy => write!(f, "Privacy"),
            Self::IntellectualProperty => write!(f, "Intellectual Property"),
            Self::IndiscriminateWeapons => write!(f, "Indiscriminate Weapons"),
            Self::Hate => write!(f, "Hate"),
            Self::SelfHarm => write!(f, "Self-Harm"),
            Self::SexualContent => write!(f, "Sexual Content"),
            Self::Elections => write!(f, "Elections"),
            Self::CodeInterpreterAbuse => write!(f, "Code Interpreter Abuse"),
            Self::DangerousContent => write!(f, "Dangerous Content"),
            Self::Harassment => write!(f, "Harassment"),
            Self::CriminalPlanning => write!(f, "Criminal Planning"),
            Self::GunsIllegalWeapons => write!(f, "Guns & Illegal Weapons"),
            Self::ControlledSubstances => write!(f, "Controlled Substances"),
            Self::Profanity => write!(f, "Profanity"),
            Self::NeedsCaution => write!(f, "Needs Caution"),
            Self::Manipulation => write!(f, "Manipulation"),
            Self::FraudDeception => write!(f, "Fraud & Deception"),
            Self::Malware => write!(f, "Malware"),
            Self::HighRiskGovDecision => write!(f, "High Risk Gov Decision"),
            Self::PoliticalMisinformation => write!(f, "Political Misinformation"),
            Self::CopyrightPlagiarism => write!(f, "Copyright & Plagiarism"),
            Self::UnauthorizedAdvice => write!(f, "Unauthorized Advice"),
            Self::IllegalActivity => write!(f, "Illegal Activity"),
            Self::ImmoralUnethical => write!(f, "Immoral & Unethical"),
            Self::SocialBias => write!(f, "Social Bias"),
            Self::Jailbreak => write!(f, "Jailbreak"),
            Self::UnethicalBehavior => write!(f, "Unethical Behavior"),
            Self::ContextRelevance => write!(f, "Context Relevance"),
            Self::Groundedness => write!(f, "Groundedness"),
            Self::AnswerRelevance => write!(f, "Answer Relevance"),
            Self::Custom(name) => write!(f, "{}", name),
        }
    }
}

/// Convert a SafetyCategory to its serde-serialized name (snake_case).
///
/// Config stores categories in serde format (e.g. "violent_crimes", "jailbreak"),
/// while Display uses human-readable format (e.g. "Violent Crimes", "Jailbreak").
/// This function returns the serde form for config comparisons.
fn category_to_serde_name(category: &SafetyCategory) -> String {
    match serde_json::to_value(category) {
        Ok(serde_json::Value::String(s)) => s,
        // Custom("foo") serializes as {"custom": "foo"}
        Ok(serde_json::Value::Object(obj)) => obj
            .into_values()
            .next()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| category.to_string()),
        _ => category.to_string(),
    }
}

/// What action to take when a category is flagged
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CategoryAction {
    /// Silently allow (log only)
    Allow,
    /// Show a non-blocking notification popup (request proceeds)
    Notify,
    /// Show a blocking approval popup (request paused until user decides)
    #[default]
    Ask,
    /// Silently deny the request (no popup, returns 403)
    Block,
}

/// How the model performs inference
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferenceMode {
    /// One inference call checks all categories (Llama Guard 4, Nemotron)
    MultiCategory,
    /// One call per category, run in parallel (ShieldGemma, Granite Guardian)
    SingleCategory,
}

/// Information about a supported category
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyCategoryInfo {
    pub category: SafetyCategory,
    /// The model's native label for this category (e.g. "S1", "violence")
    pub native_label: String,
    /// Human-readable description
    pub description: String,
}

/// Direction of the scan
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScanDirection {
    Input,
    Output,
}

/// A message in the conversation for safety checking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyMessage {
    pub role: String,
    pub content: String,
}

/// Input to a safety model check
#[derive(Debug, Clone)]
pub struct SafetyCheckInput {
    /// The conversation messages to check
    pub messages: Vec<SafetyMessage>,
    /// Whether we're scanning input (request) or output (response)
    pub direction: ScanDirection,
    /// For SingleCategory models: which category to check.
    /// None means check all (for MultiCategory models).
    pub target_category: Option<SafetyCategory>,
}

/// Result of a safety model check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyVerdict {
    /// Which model produced this verdict
    pub model_id: String,
    /// Human-readable display name for the model
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_label: Option<String>,
    /// Overall safe/unsafe determination
    pub is_safe: bool,
    /// Which categories were flagged
    pub flagged_categories: Vec<FlaggedCategory>,
    /// Overall confidence score (if available)
    pub confidence: Option<f32>,
    /// Raw model output for debugging
    pub raw_output: String,
    /// How long the check took in milliseconds
    pub check_duration_ms: u64,
}

/// A single flagged category from a safety verdict
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlaggedCategory {
    /// The unified category
    pub category: SafetyCategory,
    /// Confidence score (0.0-1.0) if available
    pub confidence: Option<f32>,
    /// The model's native label (e.g. "S1", "violence")
    pub native_label: String,
}

/// Aggregated result from all safety models
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyCheckResult {
    /// Per-model verdicts
    pub verdicts: Vec<SafetyVerdict>,
    /// Whether any model flagged the content as unsafe
    pub is_safe: bool,
    /// Actions required per flagged category (derived from category_actions config)
    pub actions_required: Vec<CategoryActionRequired>,
    /// Total check duration across all models
    pub total_duration_ms: u64,
    /// Errors from models that failed to run (model_id, error_message)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<SafetyModelError>,
}

/// An error from a safety model that failed to check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyModelError {
    pub model_id: String,
    pub error: String,
}

/// A specific action required for a flagged category
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryActionRequired {
    pub category: SafetyCategory,
    pub action: CategoryAction,
    /// Which model flagged this
    pub model_id: String,
    /// Confidence score
    pub confidence: Option<f32>,
}

impl SafetyCheckResult {
    /// Check if any actions requiring user interaction are needed
    pub fn needs_approval(&self) -> bool {
        self.actions_required
            .iter()
            .any(|a| matches!(a.action, CategoryAction::Ask))
    }

    /// Check if any notifications should be shown
    pub fn needs_notification(&self) -> bool {
        self.actions_required
            .iter()
            .any(|a| matches!(a.action, CategoryAction::Notify))
    }

    /// Check if any categories were flagged
    pub fn has_flags(&self) -> bool {
        !self.actions_required.is_empty()
    }

    /// Check if any actions have "block" (silent deny)
    pub fn has_blocks(&self) -> bool {
        self.actions_required
            .iter()
            .any(|a| matches!(a.action, CategoryAction::Block))
    }

    /// Check if ALL flagged actions are "block" (no popup needed at all)
    pub fn all_blocked(&self) -> bool {
        !self.actions_required.is_empty()
            && self
                .actions_required
                .iter()
                .all(|a| matches!(a.action, CategoryAction::Block | CategoryAction::Allow))
    }

    /// Re-filter actions using merged category overrides.
    ///
    /// Each entry in `overrides` maps a category name (e.g. "violent_crimes") to an action.
    /// A special entry with category `"__global"` is the fallback default applied to any
    /// flagged category not listed by name (this matches the "All Categories" UI control).
    ///
    /// Resolution per flagged category:
    ///   1. If a specific entry for that category exists in `overrides`, use its action.
    ///   2. Else if the `__global` fallback entry exists, use its action.
    ///   3. Else keep the engine's default action.
    ///
    /// Action semantics:
    ///   - `Allow`  → remove from `actions_required` (silently allow).
    ///   - `Block`/`Ask`/`Notify` → replace the engine's default action.
    pub fn apply_client_category_overrides(
        mut self,
        overrides: &[(String, CategoryAction)],
    ) -> Self {
        if overrides.is_empty() {
            return self;
        }

        // Pull out the optional __global fallback once so we don't re-scan per category.
        let global_default: Option<&CategoryAction> = overrides
            .iter()
            .find(|(cat, _)| cat == GLOBAL_DEFAULT_CATEGORY)
            .map(|(_, action)| action);

        self.actions_required.retain_mut(|action| {
            // Compare using serde serialization format (snake_case, e.g. "violent_crimes")
            // to match config values. Display format ("Violent Crimes") differs from serde.
            let category_name = category_to_serde_name(&action.category);
            let resolved: Option<&CategoryAction> = overrides
                .iter()
                .find(|(cat, _)| *cat == category_name)
                .map(|(_, a)| a)
                .or(global_default);

            if let Some(override_action) = resolved {
                if matches!(override_action, CategoryAction::Allow) {
                    return false; // Remove: this category is allowed
                }
                action.action = override_action.clone();
            }
            true
        });

        // Update is_safe if all actions were removed
        if self.actions_required.is_empty() {
            self.is_safe = true;
        }

        self
    }
}

/// Pseudo category-id used by the UI to represent "All Categories" — applies as a
/// default action for any flagged category that has no specific override entry.
pub const GLOBAL_DEFAULT_CATEGORY: &str = "__global";

/// The SafetyModel trait - implemented by each model (Llama Guard, ShieldGemma, etc.)
#[async_trait::async_trait]
pub trait SafetyModel: Send + Sync {
    /// Instance identifier (e.g. "llamaguard-4-local", "granite_guardian")
    fn id(&self) -> &str;

    /// Unique type identifier (e.g. "llama_guard_4", "shield_gemma")
    fn model_type_id(&self) -> &str;

    /// Human-readable display name
    fn display_name(&self) -> &str;

    /// List of categories this model can detect
    fn supported_categories(&self) -> Vec<SafetyCategoryInfo>;

    /// Whether this model checks all categories at once or one at a time
    fn inference_mode(&self) -> InferenceMode;

    /// Run a safety check
    async fn check(&self, input: &SafetyCheckInput) -> Result<SafetyVerdict, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_category_to_serde_name() {
        assert_eq!(
            category_to_serde_name(&SafetyCategory::Jailbreak),
            "jailbreak"
        );
        assert_eq!(
            category_to_serde_name(&SafetyCategory::ViolentCrimes),
            "violent_crimes"
        );
        assert_eq!(
            category_to_serde_name(&SafetyCategory::SexualContent),
            "sexual_content"
        );
        assert_eq!(category_to_serde_name(&SafetyCategory::Hate), "hate");
    }

    fn make_result(actions: Vec<(SafetyCategory, CategoryAction)>) -> SafetyCheckResult {
        SafetyCheckResult {
            verdicts: vec![],
            is_safe: false,
            actions_required: actions
                .into_iter()
                .map(|(category, action)| CategoryActionRequired {
                    category,
                    action,
                    model_id: "test".to_string(),
                    confidence: Some(0.9),
                })
                .collect(),
            total_duration_ms: 0,
            errors: vec![],
        }
    }

    #[test]
    fn test_apply_client_category_overrides_matches_serde_names() {
        let result = make_result(vec![
            (SafetyCategory::Jailbreak, CategoryAction::Ask),
            (SafetyCategory::ViolentCrimes, CategoryAction::Ask),
        ]);

        // Override using serde names (as config stores them)
        let overrides = vec![("jailbreak".to_string(), CategoryAction::Allow)];
        let result = result.apply_client_category_overrides(&overrides);

        // Jailbreak should be removed, ViolentCrimes remains
        assert_eq!(result.actions_required.len(), 1);
        assert_eq!(
            result.actions_required[0].category,
            SafetyCategory::ViolentCrimes
        );
    }

    #[test]
    fn test_global_default_allow_removes_unspecified_categories() {
        // Regression test for client-override bug: when "All Categories" (`__global`)
        // is set to Allow on a per-client basis, every flagged category should be
        // silently allowed unless a specific override says otherwise.
        let result = make_result(vec![
            (SafetyCategory::Jailbreak, CategoryAction::Ask),
            (SafetyCategory::ViolentCrimes, CategoryAction::Ask),
        ]);

        let overrides = vec![(GLOBAL_DEFAULT_CATEGORY.to_string(), CategoryAction::Allow)];
        let result = result.apply_client_category_overrides(&overrides);

        assert!(result.actions_required.is_empty());
        assert!(result.is_safe);
    }

    #[test]
    fn test_global_default_block_replaces_engine_default() {
        // When `__global` is set to Block, every flagged category should switch from
        // the engine's default (Ask) to Block — silent denial without a popup.
        let result = make_result(vec![
            (SafetyCategory::Jailbreak, CategoryAction::Ask),
            (SafetyCategory::Hate, CategoryAction::Ask),
        ]);

        let overrides = vec![(GLOBAL_DEFAULT_CATEGORY.to_string(), CategoryAction::Block)];
        let result = result.apply_client_category_overrides(&overrides);

        assert_eq!(result.actions_required.len(), 2);
        assert!(result
            .actions_required
            .iter()
            .all(|a| matches!(a.action, CategoryAction::Block)));
        assert!(result.all_blocked());
    }

    #[test]
    fn test_specific_override_wins_over_global_default() {
        // Specific category entries take precedence over the `__global` fallback.
        let result = make_result(vec![
            (SafetyCategory::Jailbreak, CategoryAction::Ask),
            (SafetyCategory::ViolentCrimes, CategoryAction::Ask),
        ]);

        let overrides = vec![
            (GLOBAL_DEFAULT_CATEGORY.to_string(), CategoryAction::Allow),
            ("violent_crimes".to_string(), CategoryAction::Block),
        ];
        let result = result.apply_client_category_overrides(&overrides);

        // Jailbreak removed (global Allow), ViolentCrimes kept as Block
        assert_eq!(result.actions_required.len(), 1);
        assert_eq!(
            result.actions_required[0].category,
            SafetyCategory::ViolentCrimes
        );
        assert_eq!(result.actions_required[0].action, CategoryAction::Block);
    }

    #[test]
    fn test_unmatched_category_keeps_engine_default_when_no_global() {
        // Without `__global`, categories not in the override list keep their
        // engine-assigned action.
        let result = make_result(vec![(SafetyCategory::Hate, CategoryAction::Ask)]);

        let overrides = vec![("jailbreak".to_string(), CategoryAction::Allow)];
        let result = result.apply_client_category_overrides(&overrides);

        assert_eq!(result.actions_required.len(), 1);
        assert_eq!(result.actions_required[0].action, CategoryAction::Ask);
    }
}
