//! Type definitions for the guardrails engine

use serde::{Deserialize, Serialize};

/// Category of detection
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailCategory {
    PromptInjection,
    JailbreakAttempt,
    PiiLeakage,
    CodeInjection,
    EncodedPayload,
    SensitiveData,
    MaliciousOutput,
    DataLeakage,
}

impl std::fmt::Display for GuardrailCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PromptInjection => write!(f, "Prompt Injection"),
            Self::JailbreakAttempt => write!(f, "Jailbreak Attempt"),
            Self::PiiLeakage => write!(f, "PII Leakage"),
            Self::CodeInjection => write!(f, "Code Injection"),
            Self::EncodedPayload => write!(f, "Encoded Payload"),
            Self::SensitiveData => write!(f, "Sensitive Data"),
            Self::MaliciousOutput => write!(f, "Malicious Output"),
            Self::DataLeakage => write!(f, "Data Leakage"),
        }
    }
}

/// Severity level of a detection
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailSeverity {
    Low,
    Medium,
    High,
    Critical,
}

impl std::fmt::Display for GuardrailSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Low => write!(f, "low"),
            Self::Medium => write!(f, "medium"),
            Self::High => write!(f, "high"),
            Self::Critical => write!(f, "critical"),
        }
    }
}

impl GuardrailSeverity {
    /// Parse severity from string (case-insensitive)
    pub fn from_str_lenient(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "critical" => Self::Critical,
            _ => Self::Medium,
        }
    }
}

/// When a rule applies: input scanning, output scanning, or both
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScanDirection {
    Input,
    Output,
    Both,
}

impl ScanDirection {
    /// Check if this direction matches an input scan
    pub fn matches_input(&self) -> bool {
        matches!(self, Self::Input | Self::Both)
    }

    /// Check if this direction matches an output scan
    pub fn matches_output(&self) -> bool {
        matches!(self, Self::Output | Self::Both)
    }
}

/// A raw rule parsed from a source (before compilation into RegexSet)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawRule {
    pub id: String,
    pub name: String,
    pub pattern: String,
    pub category: GuardrailCategory,
    pub severity: GuardrailSeverity,
    pub direction: ScanDirection,
    pub description: String,
}

/// A single detection match
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailMatch {
    pub rule_id: String,
    pub rule_name: String,
    pub source_id: String,
    pub source_label: String,
    pub category: GuardrailCategory,
    pub severity: GuardrailSeverity,
    pub direction: ScanDirection,
    pub matched_text: String,
    pub message_index: Option<usize>,
    pub description: String,
}

/// Per-source summary of a guardrail check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceCheckSummary {
    pub source_id: String,
    pub source_label: String,
    pub rules_checked: usize,
    pub match_count: usize,
}

/// Result of a guardrail check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailCheckResult {
    pub matches: Vec<GuardrailMatch>,
    pub check_duration_ms: u64,
    pub rules_checked: usize,
    pub sources_checked: Vec<SourceCheckSummary>,
}

impl GuardrailCheckResult {
    /// Check if any matches were found
    pub fn has_matches(&self) -> bool {
        !self.matches.is_empty()
    }

    /// Check if any matches meet or exceed the given severity threshold
    pub fn has_matches_at_severity(&self, min_severity: GuardrailSeverity) -> bool {
        self.matches.iter().any(|m| m.severity >= min_severity)
    }

    /// Get the highest severity among matches
    pub fn max_severity(&self) -> Option<GuardrailSeverity> {
        self.matches.iter().map(|m| m.severity).max()
    }
}

/// Error info for a single data_path download/parse failure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathDownloadError {
    pub path: String,
    /// Error kind: "directory_listing_failed", "parse_error", "http_404", "http_error", "download_failed"
    pub error: String,
    /// Human-readable detail
    pub detail: String,
}

/// Details sent to the approval popup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailApprovalDetails {
    pub matches: Vec<GuardrailMatch>,
    pub rules_checked: usize,
    pub check_duration_ms: u64,
    pub scan_direction: String,
    pub sources_checked: Vec<SourceCheckSummary>,
}
