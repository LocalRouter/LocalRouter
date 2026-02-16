//! Custom/generic safety model implementation
//!
//! Allows plugging in any safety model not natively supported.
//! User provides: prompt template, output regex, category list.

use crate::executor::{CompletionRequest, ModelExecutor};
use crate::safety_model::*;
use regex::Regex;
use std::sync::Arc;

pub struct CustomSafetyModel {
    model_id: String,
    display_name: String,
    executor: Arc<ModelExecutor>,
    model_name: String,
    /// Template with {content} placeholder
    prompt_template: String,
    /// Regex to extract category labels from output (group 1 = category)
    output_regex: Option<Regex>,
    /// Map of native labels to SafetyCategory
    categories: Vec<(SafetyCategory, String, String)>,
    /// Text that indicates safe output
    safe_indicator: String,
}

impl CustomSafetyModel {
    pub fn new(
        model_id: String,
        display_name: String,
        executor: Arc<ModelExecutor>,
        model_name: String,
        prompt_template: String,
        output_regex: Option<String>,
        categories: Vec<(SafetyCategory, String, String)>,
        safe_indicator: String,
    ) -> Result<Self, String> {
        let compiled_regex = if let Some(ref pattern) = output_regex {
            Some(Regex::new(pattern).map_err(|e| format!("Invalid output regex: {}", e))?)
        } else {
            None
        };

        Ok(Self {
            model_id,
            display_name,
            executor,
            model_name,
            prompt_template,
            output_regex: compiled_regex,
            categories,
            safe_indicator,
        })
    }

    fn build_prompt(&self, input: &SafetyCheckInput) -> String {
        let content: String = input
            .messages
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");

        self.prompt_template.replace("{content}", &content)
    }

    fn parse_output(&self, output: &str) -> (bool, Vec<FlaggedCategory>) {
        let trimmed = output.trim().to_lowercase();
        let indicator = self.safe_indicator.to_lowercase();

        // Use word-boundary check: the output must start with the safe indicator
        // (not just contain it, since "unsafe" contains "safe")
        if trimmed.starts_with(&indicator) {
            return (true, vec![]);
        }

        let mut flagged = Vec::new();

        if let Some(ref regex) = self.output_regex {
            for cap in regex.captures_iter(output) {
                if let Some(label) = cap.get(1) {
                    let label_str = label.as_str().trim();
                    if let Some((cat, native, _)) = self
                        .categories
                        .iter()
                        .find(|(_, l, _)| l.eq_ignore_ascii_case(label_str))
                    {
                        flagged.push(FlaggedCategory {
                            category: cat.clone(),
                            confidence: None,
                            native_label: native.clone(),
                        });
                    }
                }
            }
        }

        // If regex didn't find anything but output doesn't indicate safe, mark as custom violation
        if flagged.is_empty() {
            flagged.push(FlaggedCategory {
                category: SafetyCategory::Custom("unknown_violation".to_string()),
                confidence: None,
                native_label: "unknown".to_string(),
            });
        }

        let is_safe = flagged.is_empty();
        (is_safe, flagged)
    }
}

#[async_trait::async_trait]
impl SafetyModel for CustomSafetyModel {
    fn model_type_id(&self) -> &str {
        "custom"
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn supported_categories(&self) -> Vec<SafetyCategoryInfo> {
        self.categories
            .iter()
            .map(|(cat, label, desc)| SafetyCategoryInfo {
                category: cat.clone(),
                native_label: label.clone(),
                description: desc.clone(),
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
                max_tokens: Some(256),
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

    fn make_model() -> CustomSafetyModel {
        CustomSafetyModel::new(
            "test".into(),
            "Test Model".into(),
            Arc::new(ModelExecutor::Local(crate::executor::LocalGgufExecutor::new(
                "/tmp/fake".into(), 512,
            ))),
            "test".into(),
            "Check this: {content}".into(),
            Some(r"category:\s*(\w+)".into()),
            vec![
                (SafetyCategory::Hate, "hate".into(), "Hate speech".into()),
                (
                    SafetyCategory::ViolentCrimes,
                    "violence".into(),
                    "Violence".into(),
                ),
            ],
            "safe".into(),
        )
        .unwrap()
    }

    #[test]
    fn test_parse_safe_output() {
        let model = make_model();
        let (is_safe, flagged) = model.parse_output("safe");
        assert!(is_safe);
        assert!(flagged.is_empty());
    }

    /// Bug fix test: "unsafe" should NOT match "safe" indicator via substring
    #[test]
    fn test_parse_unsafe_not_matching_safe() {
        let model = make_model();
        let (is_safe, flagged) = model.parse_output("unsafe - category: hate");
        assert!(!is_safe);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].category, SafetyCategory::Hate);
    }

    /// Bug fix test: output that literally says "unsafe" with no regex match
    #[test]
    fn test_parse_unsafe_no_regex_match() {
        let model = make_model();
        let (is_safe, flagged) = model.parse_output("unsafe content detected");
        assert!(!is_safe);
        // Should have a generic "unknown_violation" flagged category
        assert_eq!(flagged.len(), 1);
        assert!(matches!(flagged[0].category, SafetyCategory::Custom(_)));
    }

    #[test]
    fn test_parse_with_regex_categories() {
        let model = make_model();
        let (is_safe, flagged) = model.parse_output("FLAGGED: category: violence, category: hate");
        assert!(!is_safe);
        assert_eq!(flagged.len(), 2);
    }
}
