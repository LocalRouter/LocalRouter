//! Secret scan engine orchestrator
//!
//! Coordinates the scan pipeline: text extraction -> keyword pre-filter ->
//! regex matching -> entropy filtering.

use std::time::Instant;

use tracing::{debug, info};

use crate::regex_engine::RegexEngine;
use crate::types::{ExtractedText, ScanResult};

/// Configuration passed to the engine at construction time
#[derive(Debug, Clone)]
pub struct SecretScanEngineConfig {
    /// Global entropy threshold
    pub entropy_threshold: f32,
    /// Allowlist regex patterns
    pub allowlist: Vec<String>,
    /// Whether to scan system messages
    pub scan_system_messages: bool,
}

/// The main secret scanning engine
pub struct SecretScanEngine {
    regex_engine: RegexEngine,
    scan_system_messages: bool,
}

impl SecretScanEngine {
    /// Create a new engine from configuration
    pub fn new(config: &SecretScanEngineConfig) -> Result<Self, String> {
        let regex_engine = RegexEngine::new(config.entropy_threshold, &config.allowlist)?;

        info!(
            "Secret scanner initialized with {} rules",
            regex_engine.rule_count()
        );

        Ok(Self {
            regex_engine,
            scan_system_messages: config.scan_system_messages,
        })
    }

    /// Whether this engine has any compiled rules
    pub fn has_rules(&self) -> bool {
        self.regex_engine.rule_count() > 0
    }

    /// Get metadata about all compiled rules (for UI display)
    pub fn rule_metadata(&self) -> Vec<crate::regex_engine::RuleMetadata> {
        self.regex_engine.rule_metadata()
    }

    /// Scan extracted texts for secrets
    pub fn scan(&self, texts: &[ExtractedText]) -> ScanResult {
        let start = Instant::now();

        // Filter out system messages if configured to skip them
        let filtered: Vec<&ExtractedText> = if self.scan_system_messages {
            texts.iter().collect()
        } else {
            texts
                .iter()
                .filter(|t| !t.label.starts_with("system"))
                .collect()
        };

        let owned: Vec<ExtractedText> = filtered.into_iter().cloned().collect();
        let findings = self.regex_engine.scan(&owned, None);
        let duration = start.elapsed();

        debug!(
            "Secret scan completed in {}ms: {} findings from {} rules",
            duration.as_millis(),
            findings.len(),
            self.regex_engine.rule_count()
        );

        ScanResult {
            findings,
            scan_duration_ms: duration.as_millis() as u64,
            rules_evaluated: self.regex_engine.rule_count(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> SecretScanEngineConfig {
        SecretScanEngineConfig {
            entropy_threshold: 3.0,
            allowlist: vec![],
            scan_system_messages: false,
        }
    }

    #[test]
    fn test_engine_creation() {
        let engine = SecretScanEngine::new(&default_config()).unwrap();
        assert!(engine.has_rules());
    }

    #[test]
    fn test_engine_scan_no_secrets() {
        let engine = SecretScanEngine::new(&default_config()).unwrap();
        let texts = vec![ExtractedText {
            label: "user[0]".to_string(),
            text: "Hello, how are you today?".to_string(),
            message_index: 0,
        }];
        let result = engine.scan(&texts);
        assert!(result.findings.is_empty());
        assert!(result.scan_duration_ms < 100);
    }

    #[test]
    fn test_engine_scan_with_secret() {
        let engine = SecretScanEngine::new(&default_config()).unwrap();
        let texts = vec![ExtractedText {
            label: "user[0]".to_string(),
            text: "My GitHub token is ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij".to_string(),
            message_index: 0,
        }];
        let result = engine.scan(&texts);
        assert!(!result.findings.is_empty(), "Should detect GitHub PAT");
    }

    #[test]
    fn test_engine_skips_system_messages() {
        let engine = SecretScanEngine::new(&default_config()).unwrap();
        let texts = vec![ExtractedText {
            label: "system".to_string(),
            text: "System prompt with ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij".to_string(),
            message_index: 0,
        }];
        let result = engine.scan(&texts);
        assert!(
            result.findings.is_empty(),
            "Should skip system messages by default"
        );
    }

    #[test]
    fn test_engine_scans_system_messages_when_enabled() {
        let mut config = default_config();
        config.scan_system_messages = true;
        let engine = SecretScanEngine::new(&config).unwrap();
        let texts = vec![ExtractedText {
            label: "system".to_string(),
            text: "System prompt with ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij".to_string(),
            message_index: 0,
        }];
        let result = engine.scan(&texts);
        assert!(
            !result.findings.is_empty(),
            "Should scan system messages when enabled"
        );
    }
}
