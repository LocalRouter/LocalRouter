//! ShieldGemma safety model implementation
//!
//! 4 categories: dangerous_content, harassment, hate_speech, sexually_explicit.
//! SingleCategory mode - one call per category, run in parallel.
//! Output: logprobs on Yes/No tokens -> probability score.
//!
//! HF: google/shieldgemma-2b, google/shieldgemma-9b (gated)
//! GGUF: QuantFactory/shieldgemma-2b-GGUF

use crate::executor::{self, CompletionRequest, ModelExecutor};
use crate::safety_model::*;
use std::sync::Arc;

/// ShieldGemma category definitions with policy text
const CATEGORIES: &[(SafetyCategory, &str, &str)] = &[
    (
        SafetyCategory::DangerousContent,
        "dangerous_content",
        "Content that facilitates or encourages harmful activities",
    ),
    (
        SafetyCategory::Harassment,
        "harassment",
        "Content that targets individuals or groups with harmful intent",
    ),
    (
        SafetyCategory::Hate,
        "hate_speech",
        "Content that promotes hatred or discrimination based on protected characteristics",
    ),
    (
        SafetyCategory::SexualContent,
        "sexually_explicit",
        "Content that contains explicit sexual material",
    ),
];

/// Policy text for each ShieldGemma category
fn category_policy(cat: &SafetyCategory) -> &'static str {
    match cat {
        SafetyCategory::DangerousContent => {
            "\"Dangerous Content\" refers to content that facilitates or encourages harmful, \
             illegal, or violent activities. This includes, but is not limited to: instructions \
             for creating weapons, drugs, or other harmful substances; promotion of self-harm or \
             suicide; guidance on how to commit crimes; and content that glorifies violence."
        }
        SafetyCategory::Harassment => {
            "\"Harassment\" refers to content that targets an individual or group with the intent \
             to intimidate, bully, threaten, or demean. This includes, but is not limited to: \
             stalking, cyberbullying, targeted insults, doxxing, and threats of physical harm."
        }
        SafetyCategory::Hate => {
            "\"Hate Speech\" refers to content that promotes hatred, discrimination, or violence \
             against individuals or groups based on protected characteristics such as race, \
             ethnicity, religion, gender, sexual orientation, disability, or national origin."
        }
        SafetyCategory::SexualContent => {
            "\"Sexually Explicit Content\" refers to content that depicts or describes sexual acts \
             in graphic detail, or content intended to arouse sexual excitement."
        }
        _ => "Unknown category",
    }
}

pub struct ShieldGemmaModel {
    model_id: String,
    executor: Arc<ModelExecutor>,
    model_name: String,
    enabled_categories: Option<Vec<SafetyCategory>>,
}

impl ShieldGemmaModel {
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

    /// Build prompt for a single category check
    ///
    /// ShieldGemma prompt format: policy definition followed by user content to classify.
    fn build_prompt(&self, input: &SafetyCheckInput, category: &SafetyCategory) -> String {
        let policy = category_policy(category);

        let mut prompt = String::new();
        prompt.push_str("You are a policy expert trying to help determine whether a user prompt is in violation of the defined safety policies.\n\n");
        prompt.push_str("<start_of_turn>\n");
        prompt.push_str(&format!("Policy: {}\n", policy));
        prompt.push_str("<end_of_turn>\n\n");

        // Add the actual user content to check
        prompt.push_str("<start_of_turn>\n");
        prompt.push_str("Human Question: ");
        for msg in &input.messages {
            if msg.role == "user" || msg.role == "system" {
                prompt.push_str(&msg.content);
                prompt.push('\n');
            }
        }
        prompt.push_str("<end_of_turn>\n\n");

        prompt.push_str("Does the human question violate the above policy? Answer with Yes or No.\n");
        prompt
    }

    /// Check a single category
    async fn check_category(
        &self,
        input: &SafetyCheckInput,
        category: &SafetyCategory,
        native_label: &str,
    ) -> Result<Option<FlaggedCategory>, String> {
        let prompt = self.build_prompt(input, category);

        let response = self
            .executor
            .complete(CompletionRequest {
                model: self.model_name.clone(),
                prompt,
                max_tokens: Some(8),
                temperature: Some(0.0),
                logprobs: Some(5),
            })
            .await?;

        // Try logprobs first, fall back to text parsing.
        // Note: confidence threshold filtering is done by the engine, not here.
        // We report the raw probability so the engine can apply its threshold.
        let (is_violation, confidence) = if let Some(ref lp) = response.logprobs {
            if let Some(prob) = executor::extract_yes_probability(lp) {
                (prob > 0.0, Some(prob))
            } else {
                // Logprobs present but no Yes/No tokens found
                let is_yes = executor::parse_yes_no_text(&response.text).unwrap_or(false);
                (is_yes, None)
            }
        } else {
            let is_yes = executor::parse_yes_no_text(&response.text).unwrap_or(false);
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

    fn active_categories(&self) -> Vec<(SafetyCategory, String)> {
        CATEGORIES
            .iter()
            .filter(|(cat, _, _)| {
                if let Some(ref enabled) = self.enabled_categories {
                    enabled.contains(cat)
                } else {
                    true
                }
            })
            .map(|(cat, label, _)| (cat.clone(), label.to_string()))
            .collect()
    }
}

#[async_trait::async_trait]
impl SafetyModel for ShieldGemmaModel {
    fn model_type_id(&self) -> &str {
        "shield_gemma"
    }

    fn display_name(&self) -> &str {
        "ShieldGemma"
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
            .map(|(cat, label)| self.check_category(input, cat, label))
            .collect();

        let results = futures::future::join_all(futures).await;

        let mut flagged = Vec::new();
        let mut raw_parts = Vec::new();

        for (i, result) in results.into_iter().enumerate() {
            let (_, label) = &categories[i];
            match result {
                Ok(Some(f)) => {
                    raw_parts.push(format!("{}: UNSAFE ({:.2})", label, f.confidence.unwrap_or(0.0)));
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
