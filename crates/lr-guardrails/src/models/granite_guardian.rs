//! IBM Granite Guardian safety model implementation
//!
//! 7 harm + 3 RAG + 1 agentic categories.
//! SingleCategory mode - one call per category, run in parallel.
//! Output: logprobs on Yes/No tokens, or <score>Yes/No</score> text parsing.
//!
//! Ollama: granite3-guardian:2b, granite3-guardian:8b, ibm/granite3.3-guardian:8b
//! NOT gated, Apache 2.0

use crate::executor::{self, CompletionRequest, ModelExecutor};
use crate::safety_model::*;
use std::sync::Arc;

/// Granite Guardian category definitions
const CATEGORIES: &[(SafetyCategory, &str, &str)] = &[
    (
        SafetyCategory::Hate,
        "harm/hate",
        "Content that demeans or discriminates against individuals or groups",
    ),
    (
        SafetyCategory::SexualContent,
        "harm/sexual_content",
        "Sexually explicit or inappropriate content",
    ),
    (
        SafetyCategory::ViolentCrimes,
        "harm/violence",
        "Content promoting or depicting violence",
    ),
    (
        SafetyCategory::Profanity,
        "harm/profanity",
        "Vulgar or offensive language",
    ),
    (
        SafetyCategory::UnethicalBehavior,
        "harm/unethical_behavior",
        "Actions that violate ethical norms or standards",
    ),
    (
        SafetyCategory::Jailbreak,
        "harm/jailbreak",
        "Attempts to bypass AI safety guidelines",
    ),
    (
        SafetyCategory::SocialBias,
        "harm/social_bias",
        "Content reinforcing social stereotypes or biases",
    ),
    // RAG risks
    (
        SafetyCategory::ContextRelevance,
        "rag/context_relevance",
        "Retrieved context is not relevant to the query",
    ),
    (
        SafetyCategory::Groundedness,
        "rag/groundedness",
        "Response is not grounded in the provided context",
    ),
    (
        SafetyCategory::AnswerRelevance,
        "rag/answer_relevance",
        "Response does not address the original question",
    ),
];

pub struct GraniteGuardianModel {
    model_id: String,
    executor: Arc<ModelExecutor>,
    model_name: String,
    enabled_categories: Option<Vec<SafetyCategory>>,
}

impl GraniteGuardianModel {
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

    /// Build Granite Guardian prompt with guardian_config for a specific risk
    fn build_prompt(
        &self,
        input: &SafetyCheckInput,
        native_label: &str,
        description: &str,
    ) -> String {
        let mut prompt = String::new();

        prompt.push_str("<|start_of_role|>system<|end_of_role|>");
        prompt.push_str("You are a safety guardian. Assess the following content for risks.\n");
        prompt.push_str(&format!(
            "guardian_config: {{ \"risk_name\": \"{}\", \"description\": \"{}\" }}\n",
            native_label, description
        ));
        prompt.push_str("<|end_of_text|>\n");

        // Add content to check
        prompt.push_str("<|start_of_role|>user<|end_of_role|>");
        for msg in &input.messages {
            prompt.push_str(&msg.content);
            prompt.push('\n');
        }
        prompt.push_str("<|end_of_text|>\n");

        prompt.push_str("<|start_of_role|>assistant<|end_of_role|>");
        prompt
    }

    /// Check a single category
    async fn check_category(
        &self,
        input: &SafetyCheckInput,
        category: &SafetyCategory,
        native_label: &str,
        description: &str,
    ) -> Result<Option<FlaggedCategory>, String> {
        let prompt = self.build_prompt(input, native_label, description);

        let response = self
            .executor
            .complete(CompletionRequest {
                model: self.model_name.clone(),
                prompt,
                max_tokens: Some(16),
                temperature: Some(0.0),
                logprobs: Some(5),
            })
            .await?;

        // Try logprobs first.
        // Note: confidence threshold filtering is done by the engine, not here.
        // We report the raw probability so the engine can apply its threshold.
        let (is_violation, confidence) = if let Some(ref lp) = response.logprobs {
            if let Some(prob) = executor::extract_yes_probability(lp) {
                // prob is P(Yes) from softmax — use > 0.5 as the binary decision
                // (Yes is more likely than No). The engine applies its own
                // confidence threshold on top of this.
                (prob > 0.5, Some(prob))
            } else {
                let is_yes = self.parse_granite_text(&response.text);
                (is_yes, None)
            }
        } else {
            let is_yes = self.parse_granite_text(&response.text);
            (is_yes, None)
        };

        if is_violation {
            Ok(Some(FlaggedCategory {
                category: category.clone(),
                confidence,
                native_label: native_label.to_string(),
            }))
        } else {
            Ok(None)
        }
    }

    /// Parse Granite Guardian text output (may use <score>Yes</score> format)
    fn parse_granite_text(&self, text: &str) -> bool {
        let trimmed = text.trim().to_lowercase();

        // Check for <score>Yes</score> format
        if let Some(start) = trimmed.find("<score>") {
            if let Some(end) = trimmed.find("</score>") {
                let score_text = &trimmed[start + 7..end].trim().to_lowercase();
                return score_text == "yes";
            }
        }

        // Fallback to simple Yes/No
        executor::parse_yes_no_text(text).unwrap_or(false)
    }

    /// RAG categories that only make sense with retrieval context.
    /// Skipped by default unless explicitly enabled in `enabled_categories`.
    fn is_rag_category(cat: &SafetyCategory) -> bool {
        matches!(
            cat,
            SafetyCategory::ContextRelevance
                | SafetyCategory::Groundedness
                | SafetyCategory::AnswerRelevance
        )
    }

    fn active_categories(&self) -> Vec<(SafetyCategory, String, String)> {
        CATEGORIES
            .iter()
            .filter(|(cat, _, _)| {
                if let Some(ref enabled) = self.enabled_categories {
                    enabled.contains(cat)
                } else {
                    // Default: skip RAG categories (they need retrieval context
                    // and produce noise on standard text input)
                    !Self::is_rag_category(cat)
                }
            })
            .map(|(cat, label, desc)| (cat.clone(), label.to_string(), desc.to_string()))
            .collect()
    }
}

#[async_trait::async_trait]
impl SafetyModel for GraniteGuardianModel {
    fn model_type_id(&self) -> &str {
        "granite_guardian"
    }

    fn display_name(&self) -> &str {
        "Granite Guardian"
    }

    fn supported_categories(&self) -> Vec<SafetyCategoryInfo> {
        CATEGORIES
            .iter()
            .map(|(cat, label, desc)| SafetyCategoryInfo {
                category: cat.clone(),
                native_label: label.to_string(),
                description: desc.to_string(),
            })
            .collect()
    }

    fn inference_mode(&self) -> InferenceMode {
        InferenceMode::SingleCategory
    }

    async fn check(&self, input: &SafetyCheckInput) -> Result<SafetyVerdict, String> {
        let start = std::time::Instant::now();
        let categories = self.active_categories();

        // Run all category checks in parallel
        let futures: Vec<_> = categories
            .iter()
            .map(|(cat, label, desc)| self.check_category(input, cat, label, desc))
            .collect();

        let results = futures::future::join_all(futures).await;

        let mut flagged = Vec::new();
        let mut raw_parts = Vec::new();

        for (i, result) in results.into_iter().enumerate() {
            let (_, label, _) = &categories[i];
            match result {
                Ok(Some(f)) => {
                    raw_parts.push(format!(
                        "{}: UNSAFE ({:.2})",
                        label,
                        f.confidence.unwrap_or(0.0)
                    ));
                    flagged.push(f);
                }
                Ok(None) => {
                    raw_parts.push(format!("{}: safe", label));
                }
                Err(e) => {
                    raw_parts.push(format!("{}: error ({})", label, e));
                }
            }
        }

        let is_safe = flagged.is_empty();

        Ok(SafetyVerdict {
            model_id: self.model_id.clone(),
            is_safe,
            flagged_categories: flagged,
            confidence: None,
            raw_output: raw_parts.join("\n"),
            check_duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_granite_score_tag() {
        let model = GraniteGuardianModel::new(
            "test".into(),
            Arc::new(ModelExecutor::Local(crate::executor::LocalGgufExecutor::new(
                "/tmp/fake".into(), 512,
            ))),
            "test".into(),
            None,
        );

        assert!(model.parse_granite_text("<score>Yes</score>"));
        assert!(!model.parse_granite_text("<score>No</score>"));
        assert!(model.parse_granite_text("Yes"));
        assert!(!model.parse_granite_text("No"));
    }
}
