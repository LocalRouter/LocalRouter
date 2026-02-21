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

    /// Re-filter actions using per-client category overrides.
    ///
    /// Each entry in `client_overrides` maps a category name (e.g. "violent_crimes") to an action.
    /// Categories overridden to `Allow` are removed from `actions_required`.
    /// Other overrides (`Block`, `Ask`, `Notify`) replace the engine's default action.
    /// Categories not in the override list keep their original action from the engine.
    pub fn apply_client_category_overrides(
        mut self,
        client_overrides: &[(String, CategoryAction)],
    ) -> Self {
        if client_overrides.is_empty() {
            return self;
        }

        self.actions_required.retain_mut(|action| {
            let category_name = action.category.to_string();
            if let Some((_, override_action)) = client_overrides
                .iter()
                .find(|(cat, _)| *cat == category_name)
            {
                if matches!(override_action, CategoryAction::Allow) {
                    return false; // Remove: client allows this category
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
