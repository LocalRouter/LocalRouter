//! Mistral Moderation safety model implementation
//!
//! Uses Mistral's dedicated /v1/moderations endpoint.
//! MultiCategory mode — one call checks all categories.
//!
//! Model: mistral-moderation-latest (based on Ministral 8B)
//! Pricing: ~$0.10/1M tokens (as of 2026-03)

use crate::safety_model::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::debug;

/// Mistral moderation category → unified SafetyCategory mapping
const CATEGORIES: &[(&str, SafetyCategory)] = &[
    ("sexual", SafetyCategory::SexualContent),
    ("hate_and_discrimination", SafetyCategory::Hate),
    ("violence_and_threats", SafetyCategory::ViolentCrimes),
    (
        "dangerous_and_criminal_content",
        SafetyCategory::DangerousContent,
    ),
    ("selfharm", SafetyCategory::SelfHarm),
    ("health", SafetyCategory::SpecializedAdvice),
    ("financial", SafetyCategory::SpecializedAdvice),
    ("law", SafetyCategory::SpecializedAdvice),
    ("pii", SafetyCategory::Privacy),
];

pub struct MistralModerationModel {
    model_id: String,
    executor: Arc<MistralModerationExecutor>,
    model_name: String,
    enabled_categories: Option<Vec<SafetyCategory>>,
}

impl MistralModerationModel {
    pub fn new(
        model_id: String,
        executor: Arc<MistralModerationExecutor>,
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

    /// Convert Mistral moderation result into flagged categories
    fn parse_result(&self, result: &MistralModerationResult) -> Vec<FlaggedCategory> {
        let mut best_scores: HashMap<SafetyCategory, (f32, String)> = HashMap::new();

        for (native_label, category) in CATEGORIES {
            if let Some(ref enabled) = self.enabled_categories {
                if !enabled.contains(category) {
                    continue;
                }
            }

            let is_flagged = result
                .categories
                .get(*native_label)
                .copied()
                .unwrap_or(false);
            let score = result
                .category_scores
                .get(*native_label)
                .copied()
                .unwrap_or(0.0) as f32;

            if is_flagged {
                let entry = best_scores
                    .entry(category.clone())
                    .or_insert((0.0, native_label.to_string()));
                if score > entry.0 {
                    *entry = (score, native_label.to_string());
                }
            }
        }

        best_scores
            .into_iter()
            .map(|(category, (confidence, native_label))| FlaggedCategory {
                category,
                confidence: Some(confidence),
                native_label,
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl SafetyModel for MistralModerationModel {
    fn id(&self) -> &str {
        &self.model_id
    }

    fn model_type_id(&self) -> &str {
        "mistral_moderation"
    }

    fn display_name(&self) -> &str {
        "Mistral Moderation"
    }

    fn supported_categories(&self) -> Vec<SafetyCategoryInfo> {
        CATEGORIES
            .iter()
            .map(|(label, cat)| SafetyCategoryInfo {
                category: cat.clone(),
                native_label: label.to_string(),
                description: format!("Mistral moderation: {}", label),
            })
            .collect()
    }

    fn inference_mode(&self) -> InferenceMode {
        InferenceMode::MultiCategory
    }

    async fn check(&self, input: &SafetyCheckInput) -> Result<SafetyVerdict, String> {
        let start = std::time::Instant::now();

        // Concatenate all messages into a single input string
        let text: String = input
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let response = self.executor.moderate(&text, &self.model_name).await?;

        let flagged_categories = if let Some(first_result) = response.results.first() {
            self.parse_result(first_result)
        } else {
            vec![]
        };

        let is_safe = flagged_categories.is_empty();

        Ok(SafetyVerdict {
            model_id: self.model_id.clone(),
            is_safe,
            flagged_categories,
            confidence: None,
            raw_output: serde_json::to_string(&response).unwrap_or_default(),
            check_duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

// ── Mistral Moderation API types ──

#[derive(Debug, Clone, Serialize)]
struct MistralModerationRequest {
    model: String,
    input: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MistralModerationResponse {
    pub id: String,
    pub model: String,
    pub results: Vec<MistralModerationResult>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MistralModerationResult {
    pub categories: HashMap<String, bool>,
    pub category_scores: HashMap<String, f64>,
}

// ── Executor for Mistral /v1/moderations ──

/// Executor that calls Mistral's /v1/moderations endpoint
pub struct MistralModerationExecutor {
    http_client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl MistralModerationExecutor {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        Self {
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            base_url,
            api_key,
        }
    }

    /// Call the /v1/moderations endpoint
    pub async fn moderate(
        &self,
        input: &str,
        model: &str,
    ) -> Result<MistralModerationResponse, String> {
        let url = format!("{}/moderations", self.base_url.trim_end_matches('/'));

        let body = MistralModerationRequest {
            model: model.to_string(),
            input: input.to_string(),
        };

        let mut req = self.http_client.post(&url).json(&body);
        if let Some(ref key) = self.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Mistral moderation request failed: {}", e))?;

        let status = resp.status();
        let resp_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read Mistral moderation response: {}", e))?;

        if !status.is_success() {
            return Err(format!(
                "Mistral moderation endpoint returned {}: {}",
                status, resp_text
            ));
        }

        debug!("Mistral moderation response: {}", resp_text);

        serde_json::from_str(&resp_text)
            .map_err(|e| format!("Invalid Mistral moderation JSON: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(
        categories: Vec<(&str, bool)>,
        scores: Vec<(&str, f64)>,
    ) -> MistralModerationResult {
        MistralModerationResult {
            categories: categories
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            category_scores: scores
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }

    #[test]
    fn test_parse_safe_result() {
        let model = MistralModerationModel::new(
            "test".into(),
            Arc::new(MistralModerationExecutor::new(
                "http://localhost".into(),
                None,
            )),
            "mistral-moderation-latest".into(),
            None,
        );

        let result = make_result(
            vec![("sexual", false), ("violence_and_threats", false)],
            vec![("sexual", 0.01), ("violence_and_threats", 0.02)],
        );

        let flagged = model.parse_result(&result);
        assert!(flagged.is_empty());
    }

    #[test]
    fn test_parse_flagged_result() {
        let model = MistralModerationModel::new(
            "test".into(),
            Arc::new(MistralModerationExecutor::new(
                "http://localhost".into(),
                None,
            )),
            "mistral-moderation-latest".into(),
            None,
        );

        let result = make_result(
            vec![
                ("hate_and_discrimination", true),
                ("violence_and_threats", false),
                ("pii", true),
            ],
            vec![
                ("hate_and_discrimination", 0.92),
                ("violence_and_threats", 0.01),
                ("pii", 0.85),
            ],
        );

        let flagged = model.parse_result(&result);
        assert_eq!(flagged.len(), 2);

        let hate = flagged.iter().find(|f| f.category == SafetyCategory::Hate);
        assert!(hate.is_some());
        assert!((hate.unwrap().confidence.unwrap() - 0.92).abs() < 0.001);

        let privacy = flagged
            .iter()
            .find(|f| f.category == SafetyCategory::Privacy);
        assert!(privacy.is_some());
    }

    #[test]
    fn test_parse_with_enabled_categories_filter() {
        let model = MistralModerationModel::new(
            "test".into(),
            Arc::new(MistralModerationExecutor::new(
                "http://localhost".into(),
                None,
            )),
            "mistral-moderation-latest".into(),
            Some(vec![SafetyCategory::Hate]),
        );

        let result = make_result(
            vec![("hate_and_discrimination", true), ("pii", true)],
            vec![("hate_and_discrimination", 0.92), ("pii", 0.85)],
        );

        let flagged = model.parse_result(&result);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].category, SafetyCategory::Hate);
    }

    #[test]
    fn test_specialized_advice_merges_best_score() {
        let model = MistralModerationModel::new(
            "test".into(),
            Arc::new(MistralModerationExecutor::new(
                "http://localhost".into(),
                None,
            )),
            "mistral-moderation-latest".into(),
            None,
        );

        // health, financial, law all map to SpecializedAdvice
        let result = make_result(
            vec![("health", true), ("financial", true), ("law", false)],
            vec![("health", 0.7), ("financial", 0.9), ("law", 0.3)],
        );

        let flagged = model.parse_result(&result);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].category, SafetyCategory::SpecializedAdvice);
        // Should pick the highest score (financial = 0.9)
        assert!((flagged[0].confidence.unwrap() - 0.9).abs() < 0.001);
    }
}
