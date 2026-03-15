//! Core types for secret scanning

use serde::{Deserialize, Serialize};

/// What action to take when a secret is detected
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SecretScanAction {
    /// Block the request and show a popup for user decision
    Ask,
    /// Allow the request but show a notification
    Notify,
    /// No scanning
    #[default]
    Off,
}

/// Categories of secrets (for display grouping only, no per-category actions)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecretCategory {
    CloudProvider,
    AiService,
    VersionControl,
    Database,
    Financial,
    OAuth,
    Generic,
}

impl std::fmt::Display for SecretCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CloudProvider => write!(f, "Cloud Provider"),
            Self::AiService => write!(f, "AI Service"),
            Self::VersionControl => write!(f, "Version Control"),
            Self::Database => write!(f, "Database"),
            Self::Financial => write!(f, "Financial"),
            Self::OAuth => write!(f, "OAuth"),
            Self::Generic => write!(f, "Generic"),
        }
    }
}

/// A compiled secret detection rule
pub struct SecretRule {
    pub id: String,
    pub description: String,
    pub compiled_regex: regex::Regex,
    /// Which capture group contains the secret (0 = entire match)
    pub secret_group: usize,
    /// Per-rule entropy threshold override (None = use global)
    pub entropy_threshold: Option<f32>,
    /// Fast pre-filter keywords
    pub keywords: Vec<String>,
    pub category: SecretCategory,
}

/// A single secret match found during scanning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretFinding {
    pub rule_id: String,
    pub rule_description: String,
    pub category: String,
    /// The regex pattern that matched
    pub regex_pattern: String,
    /// Keywords used for pre-filtering this rule
    pub keywords: Vec<String>,
    /// Per-rule entropy threshold (None = uses global)
    pub rule_entropy_threshold: Option<f32>,
    /// Index of the message in the conversation that contained the match
    pub message_index: usize,
    /// Truncated preview of matched text (~40 chars)
    pub matched_text: String,
    /// Calculated Shannon entropy of the matched text
    pub entropy: f32,
}

/// Result of scanning a request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub findings: Vec<SecretFinding>,
    pub scan_duration_ms: u64,
    pub rules_evaluated: usize,
}

/// Text extracted from a request for scanning
#[derive(Debug, Clone)]
pub struct ExtractedText {
    /// Label identifying the source (e.g., "user[0]", "system")
    pub label: String,
    /// The text content to scan
    pub text: String,
    /// Index of the message in the conversation
    pub message_index: usize,
}
