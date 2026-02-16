//! Nvidia Nemotron Safety Guard model implementation
//!
//! 23 categories (S1-S23), MultiCategory mode.
//! Prompt format: Llama 3.1 format with full taxonomy.
//! Output: JSON {"User Safety": "safe/unsafe", "Safety Categories": "..."}
//!
//! HF: nvidia/Llama-3.1-Nemotron-Safety-Guard-8B-v3 (NOT gated)
//! GGUF: AXONVERTEX-AI-RESEARCH/Llama-3.1-Nemotron-Safety-Guard-8B-v3-Q8_0-GGUF

use crate::executor::{CompletionRequest, ModelExecutor};
use crate::safety_model::*;
use std::sync::Arc;

/// Nemotron category definitions (S1-S23)
const CATEGORIES: &[(SafetyCategory, &str, &str)] = &[
    (SafetyCategory::ViolentCrimes, "S1", "Violent Crimes"),
    (SafetyCategory::NonViolentCrimes, "S2", "Non-Violent Crimes"),
    (SafetyCategory::SexCrimes, "S3", "Sex-Related Crimes"),
    (SafetyCategory::ChildExploitation, "S4", "Child Sexual Exploitation"),
    (SafetyCategory::Defamation, "S5", "Defamation"),
    (SafetyCategory::SpecializedAdvice, "S6", "Specialized Advice"),
    (SafetyCategory::Privacy, "S7", "Privacy"),
    (SafetyCategory::IntellectualProperty, "S8", "Intellectual Property"),
    (SafetyCategory::IndiscriminateWeapons, "S9", "Indiscriminate Weapons"),
    (SafetyCategory::Hate, "S10", "Hate"),
    (SafetyCategory::SelfHarm, "S11", "Suicide & Self-Harm"),
    (SafetyCategory::SexualContent, "S12", "Sexual Content"),
    (SafetyCategory::Elections, "S13", "Elections"),
    (SafetyCategory::CodeInterpreterAbuse, "S14", "Code Interpreter Abuse"),
    (SafetyCategory::GunsIllegalWeapons, "S15", "Guns & Illegal Weapons"),
    (SafetyCategory::ControlledSubstances, "S16", "Controlled Substances"),
    (SafetyCategory::CriminalPlanning, "S17", "Criminal Planning"),
    (SafetyCategory::Profanity, "S18", "Profanity"),
    (SafetyCategory::NeedsCaution, "S19", "Needs Caution"),
    (SafetyCategory::Manipulation, "S20", "Manipulation"),
    (SafetyCategory::FraudDeception, "S21", "Fraud & Deception"),
    (SafetyCategory::Malware, "S22", "Malware"),
    (SafetyCategory::HighRiskGovDecision, "S23", "High Risk Gov Decision"),
];

pub struct NemotronModel {
    model_id: String,
    executor: Arc<ModelExecutor>,
    model_name: String,
    enabled_categories: Option<Vec<SafetyCategory>>,
}

impl NemotronModel {
    pub fn new(
        model_id: String,
        executor: Arc<ModelExecutor>,
        model_name: String,
        enabled_categories: Option<Vec<SafetyCategory>>,
    ) -> Self {
        Self {
            model_id,
            executor,
            model_name,
            enabled_categories,
        }
    }

    /// Build Nemotron prompt in Llama 3.1 format
    fn build_prompt(&self, input: &SafetyCheckInput) -> String {
        let mut prompt = String::new();

        prompt.push_str("<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\n");
        prompt.push_str("You are a helpful assistant that classifies content safety.\n");
        prompt.push_str("Given a conversation, determine if the content is safe or unsafe.\n");
        prompt.push_str("If unsafe, identify which safety categories are violated.\n\n");
        prompt.push_str("Safety categories:\n");

        for (cat, code, desc) in CATEGORIES {
            if let Some(ref enabled) = self.enabled_categories {
                if !enabled.contains(cat) {
                    continue;
                }
            }
            prompt.push_str(&format!("{}: {}\n", code, desc));
        }

        prompt.push_str("\nRespond with a JSON object containing:\n");
        prompt.push_str("- \"User Safety\": \"safe\" or \"unsafe\"\n");
        prompt.push_str("- \"Safety Categories\": comma-separated list of violated category codes (if unsafe)\n");
        prompt.push_str("<|eot_id|>\n");

        // Add conversation
        for msg in &input.messages {
            let role = match msg.role.as_str() {
                "assistant" => "assistant",
                "system" => "system",
                _ => "user",
            };
            prompt.push_str(&format!(
                "<|start_header_id|>{}<|end_header_id|>\n\n{}<|eot_id|>\n",
                role, msg.content
            ));
        }

        prompt.push_str("<|start_header_id|>assistant<|end_header_id|>\n\n");
        prompt
    }

    /// Parse Nemotron JSON output
    fn parse_output(&self, output: &str) -> (bool, Vec<FlaggedCategory>) {
        let trimmed = output.trim();

        // Try to parse as JSON
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
            return self.parse_json_verdict(&json);
        }

        // Try to find JSON within the output (model might output extra text)
        if let Some(start) = trimmed.find('{') {
            if let Some(end) = trimmed.rfind('}') {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&trimmed[start..=end]) {
                    return self.parse_json_verdict(&json);
                }
            }
        }

        // Fallback: try line-based parsing similar to Llama Guard
        let lower = trimmed.to_lowercase();
        if lower.starts_with("safe") {
            return (true, vec![]);
        }
        if lower.starts_with("unsafe") {
            let flagged = self.parse_codes_from_text(trimmed);
            // Even if no S-codes parsed, still unsafe
            return (false, flagged);
        }

        // Default to safe if we can't parse
        tracing::warn!("Could not parse Nemotron output: {}", trimmed);
        (true, vec![])
    }

    fn parse_json_verdict(&self, json: &serde_json::Value) -> (bool, Vec<FlaggedCategory>) {
        // Check "User Safety" field (various key formats)
        let safety = json
            .get("User Safety")
            .or_else(|| json.get("user_safety"))
            .and_then(|v| v.as_str())
            .unwrap_or("safe");

        let is_safe = safety.to_lowercase() == "safe";
        let mut flagged = Vec::new();

        if !is_safe {
            // Parse "Safety Categories" field
            let cats_str = json
                .get("Safety Categories")
                .or_else(|| json.get("safety_categories"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            flagged = self.parse_codes_from_text(cats_str);
        }

        (is_safe, flagged)
    }

    fn parse_codes_from_text(&self, text: &str) -> Vec<FlaggedCategory> {
        let mut flagged = Vec::new();
        for part in text.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            // Match by S-code (e.g. "S7") or by description name (e.g. "Privacy", "PII/Privacy")
            let found = CATEGORIES.iter().find(|(_, code, desc)| {
                *code == part || desc.eq_ignore_ascii_case(part) || {
                    // Also match partial/alternative names the model may produce
                    // e.g. "PII/Privacy" should match "Privacy" (S7)
                    let part_lower = part.to_lowercase();
                    let desc_lower = desc.to_lowercase();
                    part_lower.contains(&desc_lower) || desc_lower.contains(&part_lower)
                }
            });
            if let Some((cat, label, _)) = found {
                flagged.push(FlaggedCategory {
                    category: cat.clone(),
                    confidence: None,
                    native_label: label.to_string(),
                });
            }
        }
        flagged
    }
}

#[async_trait::async_trait]
impl SafetyModel for NemotronModel {
    fn model_type_id(&self) -> &str {
        "nemotron"
    }

    fn display_name(&self) -> &str {
        "Nemotron Safety Guard"
    }

    fn supported_categories(&self) -> Vec<SafetyCategoryInfo> {
        CATEGORIES
            .iter()
            .map(|(cat, code, desc)| SafetyCategoryInfo {
                category: cat.clone(),
                native_label: code.to_string(),
                description: desc.to_string(),
            })
            .collect()
    }

    fn inference_mode(&self) -> InferenceMode {
        InferenceMode::MultiCategory
    }

    async fn check(&self, input: &SafetyCheckInput) -> Result<SafetyVerdict, String> {
        let start = std::time::Instant::now();
        let prompt = self.build_prompt(input);

        let response = self
            .executor
            .complete(CompletionRequest {
                model: self.model_name.clone(),
                prompt,
                max_tokens: Some(64),
                temperature: Some(0.0),
                logprobs: None,
            })
            .await?;

        let (is_safe, flagged) = self.parse_output(&response.text);

        Ok(SafetyVerdict {
            model_id: self.model_id.clone(),
            is_safe,
            flagged_categories: flagged,
            confidence: None,
            raw_output: response.text,
            check_duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_model() -> NemotronModel {
        NemotronModel::new(
            "test".into(),
            Arc::new(ModelExecutor::Local(crate::executor::LocalGgufExecutor::new(
                "/tmp/fake".into(), 512,
            ))),
            "test".into(),
            None,
        )
    }

    #[test]
    fn test_parse_json_safe() {
        let model = make_model();
        let (is_safe, flagged) =
            model.parse_output(r#"{"User Safety": "safe", "Safety Categories": ""}"#);
        assert!(is_safe);
        assert!(flagged.is_empty());
    }

    #[test]
    fn test_parse_json_unsafe() {
        let model = make_model();
        let (is_safe, flagged) =
            model.parse_output(r#"{"User Safety": "unsafe", "Safety Categories": "S1, S10"}"#);
        assert!(!is_safe);
        assert_eq!(flagged.len(), 2);
        assert_eq!(flagged[0].category, SafetyCategory::ViolentCrimes);
        assert_eq!(flagged[1].category, SafetyCategory::Hate);
    }

    #[test]
    fn test_parse_plain_text_fallback() {
        let model = make_model();
        let (is_safe, _) = model.parse_output("safe");
        assert!(is_safe);
    }

    /// Bug fix test: "unsafe" with no S-codes should still be unsafe
    #[test]
    fn test_parse_unsafe_no_codes() {
        let model = make_model();
        let (is_safe, flagged) = model.parse_output("unsafe");
        assert!(!is_safe);
        assert!(flagged.is_empty()); // no specific codes, but still unsafe
    }

    /// Bug fix test: JSON with "unsafe" but empty categories should still be unsafe
    #[test]
    fn test_parse_json_unsafe_empty_categories() {
        let model = make_model();
        let (is_safe, flagged) =
            model.parse_output(r#"{"User Safety": "unsafe", "Safety Categories": ""}"#);
        assert!(!is_safe);
        assert!(flagged.is_empty());
    }

    /// Bug fix test: model returns category names instead of S-codes (e.g. "PII/Privacy")
    #[test]
    fn test_parse_json_unsafe_category_names() {
        let model = make_model();
        let (is_safe, flagged) = model
            .parse_output(r#"{"User Safety": "unsafe", "Safety Categories": "PII/Privacy"}"#);
        assert!(!is_safe);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].category, SafetyCategory::Privacy);
    }

    /// Bug fix test: model returns full description names
    #[test]
    fn test_parse_json_unsafe_description_names() {
        let model = make_model();
        let (is_safe, flagged) = model.parse_output(
            r#"{"User Safety": "unsafe", "Safety Categories": "Violent Crimes, Hate"}"#,
        );
        assert!(!is_safe);
        assert_eq!(flagged.len(), 2);
        assert_eq!(flagged[0].category, SafetyCategory::ViolentCrimes);
        assert_eq!(flagged[1].category, SafetyCategory::Hate);
    }

    /// Bug fix test: JSON with "unsafe" and invalid category codes should still be unsafe
    #[test]
    fn test_parse_json_unsafe_invalid_codes() {
        let model = make_model();
        let (is_safe, flagged) = model
            .parse_output(r#"{"User Safety": "unsafe", "Safety Categories": "INVALID, BOGUS"}"#);
        assert!(!is_safe);
        assert!(flagged.is_empty()); // codes don't match, but still unsafe
    }
}
