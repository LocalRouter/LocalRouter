//! Llama Guard 4 safety model implementation
//!
//! 14 categories (S1-S14), MultiCategory mode.
//! Prompt format: Llama 4 chat format with safety taxonomy.
//! Output: line 1 = "safe"/"unsafe", line 2 = comma-separated S-codes.
//!
//! HF: meta-llama/Llama-Guard-4-12B (gated)
//! Ollama: llama-guard4

use crate::executor::{CompletionRequest, ModelExecutor};
use crate::safety_model::*;
use std::sync::Arc;

/// Llama Guard 4 category definitions (S1-S14)
const CATEGORIES: &[(SafetyCategory, &str, &str)] = &[
    (SafetyCategory::ViolentCrimes, "S1", "Violent Crimes"),
    (SafetyCategory::NonViolentCrimes, "S2", "Non-Violent Crimes"),
    (SafetyCategory::SexCrimes, "S3", "Sex-Related Crimes"),
    (
        SafetyCategory::ChildExploitation,
        "S4",
        "Child Sexual Exploitation",
    ),
    (SafetyCategory::Defamation, "S5", "Defamation"),
    (
        SafetyCategory::SpecializedAdvice,
        "S6",
        "Specialized Advice",
    ),
    (SafetyCategory::Privacy, "S7", "Privacy"),
    (
        SafetyCategory::IntellectualProperty,
        "S8",
        "Intellectual Property",
    ),
    (
        SafetyCategory::IndiscriminateWeapons,
        "S9",
        "Indiscriminate Weapons",
    ),
    (SafetyCategory::Hate, "S10", "Hate"),
    (SafetyCategory::SelfHarm, "S11", "Suicide & Self-Harm"),
    (SafetyCategory::SexualContent, "S12", "Sexual Content"),
    (SafetyCategory::Elections, "S13", "Elections"),
    (
        SafetyCategory::CodeInterpreterAbuse,
        "S14",
        "Code Interpreter Abuse",
    ),
];

pub struct LlamaGuardModel {
    model_id: String,
    executor: Arc<ModelExecutor>,
    model_name: String,
    enabled_categories: Option<Vec<SafetyCategory>>,
}

impl LlamaGuardModel {
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

    /// Build the Llama Guard 4 prompt with taxonomy
    fn build_prompt(&self, input: &SafetyCheckInput) -> String {
        let mut prompt = String::new();

        // System message with taxonomy
        prompt.push_str("<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\n");
        prompt.push_str("Task: Check if there is unsafe content in the conversation according to our safety policy with the below categories.\n\n");
        prompt.push_str("<BEGIN UNSAFE CONTENT CATEGORIES>\n");

        for (cat, code, desc) in CATEGORIES {
            if let Some(ref enabled) = self.enabled_categories {
                if !enabled.contains(cat) {
                    continue;
                }
            }
            prompt.push_str(&format!("{}: {}\n", code, desc));
        }

        prompt.push_str("<END UNSAFE CONTENT CATEGORIES>\n\n");
        prompt.push_str("<BEGIN CONVERSATION>\n\n");

        // Add conversation messages
        for msg in &input.messages {
            let role = match msg.role.as_str() {
                "system" | "assistant" => "Agent",
                _ => "User",
            };
            prompt.push_str(&format!("{}: {}\n\n", role, msg.content));
        }

        prompt.push_str("<END CONVERSATION>\n\n");
        prompt.push_str("Provide your safety assessment for the above conversation.\n");
        prompt.push_str("- First line must read 'safe' or 'unsafe'.\n");
        prompt.push_str("- If unsafe, a second line must include a comma-separated list of violated categories.\n");
        prompt.push_str("<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n");

        prompt
    }

    /// Parse the model output into a verdict
    fn parse_output(&self, output: &str) -> (bool, Vec<FlaggedCategory>) {
        let trimmed = output.trim();
        let lines: Vec<&str> = trimmed.lines().collect();

        if lines.is_empty() {
            return (true, vec![]);
        }

        let is_safe = lines[0].trim().to_lowercase() == "safe";
        let mut flagged = Vec::new();

        if !is_safe && lines.len() > 1 {
            let codes_line = lines[1].trim();
            for code in codes_line.split(',') {
                let code = code.trim();
                if let Some((cat, native_label, _)) = CATEGORIES.iter().find(|(_, c, _)| *c == code)
                {
                    flagged.push(FlaggedCategory {
                        category: cat.clone(),
                        confidence: None,
                        native_label: native_label.to_string(),
                    });
                }
            }
        }

        (is_safe, flagged)
    }
}

#[async_trait::async_trait]
impl SafetyModel for LlamaGuardModel {
    fn id(&self) -> &str {
        &self.model_id
    }

    fn model_type_id(&self) -> &str {
        "llama_guard"
    }

    fn display_name(&self) -> &str {
        "Llama Guard 4"
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
                max_tokens: Some(32),
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

    #[test]
    fn test_parse_safe_output() {
        let model = LlamaGuardModel::new(
            "test".into(),
            Arc::new(ModelExecutor::Local(
                crate::executor::LocalGgufExecutor::new("/tmp/fake".into(), 512).unwrap(),
            )),
            "test".into(),
            None,
        );

        let (is_safe, flagged) = model.parse_output("safe");
        assert!(is_safe);
        assert!(flagged.is_empty());
    }

    #[test]
    fn test_parse_unsafe_output() {
        let model = LlamaGuardModel::new(
            "test".into(),
            Arc::new(ModelExecutor::Local(
                crate::executor::LocalGgufExecutor::new("/tmp/fake".into(), 512).unwrap(),
            )),
            "test".into(),
            None,
        );

        let (is_safe, flagged) = model.parse_output("unsafe\nS1, S7, S11");
        assert!(!is_safe);
        assert_eq!(flagged.len(), 3);
        assert_eq!(flagged[0].category, SafetyCategory::ViolentCrimes);
        assert_eq!(flagged[1].category, SafetyCategory::Privacy);
        assert_eq!(flagged[2].category, SafetyCategory::SelfHarm);
    }

    #[test]
    fn test_parse_empty_output() {
        let model = LlamaGuardModel::new(
            "test".into(),
            Arc::new(ModelExecutor::Local(
                crate::executor::LocalGgufExecutor::new("/tmp/fake".into(), 512).unwrap(),
            )),
            "test".into(),
            None,
        );

        let (is_safe, flagged) = model.parse_output("");
        assert!(is_safe);
        assert!(flagged.is_empty());
    }

    /// Bug fix test: "unsafe" with no second line of S-codes
    #[test]
    fn test_parse_unsafe_no_codes() {
        let model = LlamaGuardModel::new(
            "test".into(),
            Arc::new(ModelExecutor::Local(
                crate::executor::LocalGgufExecutor::new("/tmp/fake".into(), 512).unwrap(),
            )),
            "test".into(),
            None,
        );

        let (is_safe, flagged) = model.parse_output("unsafe");
        assert!(!is_safe);
        assert!(flagged.is_empty()); // no codes, but model still says unsafe
    }

    /// Bug fix test: "unsafe" with invalid codes on second line
    #[test]
    fn test_parse_unsafe_invalid_codes() {
        let model = LlamaGuardModel::new(
            "test".into(),
            Arc::new(ModelExecutor::Local(
                crate::executor::LocalGgufExecutor::new("/tmp/fake".into(), 512).unwrap(),
            )),
            "test".into(),
            None,
        );

        let (is_safe, flagged) = model.parse_output("unsafe\nINVALID, BOGUS");
        assert!(!is_safe);
        assert!(flagged.is_empty()); // codes don't match, but still unsafe
    }
}
