//! OpenAI Moderation safety model implementation
//!
//! Uses OpenAI's dedicated /v1/moderations endpoint instead of chat completions.
//! MultiCategory mode — one call checks all categories.
//!
//! Models: omni-moderation-latest, text-moderation-latest
//! Pricing: Free (as of 2026-03)

use crate::safety_model::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::debug;

/// OpenAI moderation category → unified SafetyCategory mapping
const CATEGORIES: &[(&str, SafetyCategory)] = &[
    ("hate", SafetyCategory::Hate),
    ("harassment", SafetyCategory::Harassment),
    ("self-harm", SafetyCategory::SelfHarm),
    ("sexual", SafetyCategory::SexualContent),
    ("sexual/minors", SafetyCategory::ChildExploitation),
    ("violence", SafetyCategory::ViolentCrimes),
    ("illicit", SafetyCategory::IllegalActivity),
    ("illicit/violent", SafetyCategory::CriminalPlanning),
];

pub struct OpenAIModerationModel {
    model_id: String,
    executor: Arc<ModerationExecutor>,
    model_name: String,
    enabled_categories: Option<Vec<SafetyCategory>>,
}

impl OpenAIModerationModel {
    pub fn new(
        model_id: String,
        executor: Arc<ModerationExecutor>,
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

    /// Convert OpenAI moderation result into flagged categories
    fn parse_result(&self, result: &OpenAIModerationResult) -> Vec<FlaggedCategory> {
        let mut flagged = Vec::new();
        // Track max confidence per SafetyCategory when subcategories merge
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

        for (category, (confidence, native_label)) in best_scores {
            flagged.push(FlaggedCategory {
                category,
                confidence: Some(confidence),
                native_label,
            });
        }

        flagged
    }
}

#[async_trait::async_trait]
impl SafetyModel for OpenAIModerationModel {
    fn id(&self) -> &str {
        &self.model_id
    }

    fn model_type_id(&self) -> &str {
        "openai_moderation"
    }

    fn display_name(&self) -> &str {
        "OpenAI Moderation"
    }

    fn supported_categories(&self) -> Vec<SafetyCategoryInfo> {
        CATEGORIES
            .iter()
            .map(|(label, cat)| SafetyCategoryInfo {
                category: cat.clone(),
                native_label: label.to_string(),
                description: format!("OpenAI moderation: {}", label),
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

// ── OpenAI Moderation API types ──

#[derive(Debug, Clone, Serialize)]
struct ModerationRequest {
    input: String,
    model: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAIModerationResponse {
    pub id: String,
    pub model: String,
    pub results: Vec<OpenAIModerationResult>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAIModerationResult {
    pub flagged: bool,
    pub categories: HashMap<String, bool>,
    pub category_scores: HashMap<String, f64>,
}

// ── Executor for /v1/moderations ──

/// Executor that calls OpenAI's /v1/moderations endpoint
pub struct ModerationExecutor {
    http_client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl ModerationExecutor {
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
    ) -> Result<OpenAIModerationResponse, String> {
        let url = format!("{}/v1/moderations", self.base_url.trim_end_matches('/'));

        let body = ModerationRequest {
            input: input.to_string(),
            model: model.to_string(),
        };

        let mut req = self.http_client.post(&url).json(&body);
        if let Some(ref key) = self.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Moderation request failed: {}", e))?;

        let status = resp.status();
        let resp_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read moderation response: {}", e))?;

        if !status.is_success() {
            return Err(format!(
                "Moderation endpoint returned {}: {}",
                status, resp_text
            ));
        }

        debug!("OpenAI moderation response: {}", resp_text);

        serde_json::from_str(&resp_text).map_err(|e| format!("Invalid moderation JSON: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(
        categories: Vec<(&str, bool)>,
        scores: Vec<(&str, f64)>,
    ) -> OpenAIModerationResult {
        OpenAIModerationResult {
            flagged: categories.iter().any(|(_, v)| *v),
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
        let model = OpenAIModerationModel::new(
            "test".into(),
            Arc::new(ModerationExecutor::new("http://localhost".into(), None)),
            "omni-moderation-latest".into(),
            None,
        );

        let result = make_result(
            vec![("hate", false), ("violence", false)],
            vec![("hate", 0.01), ("violence", 0.02)],
        );

        let flagged = model.parse_result(&result);
        assert!(flagged.is_empty());
    }

    #[test]
    fn test_parse_flagged_result() {
        let model = OpenAIModerationModel::new(
            "test".into(),
            Arc::new(ModerationExecutor::new("http://localhost".into(), None)),
            "omni-moderation-latest".into(),
            None,
        );

        let result = make_result(
            vec![("hate", true), ("violence", false), ("harassment", true)],
            vec![("hate", 0.92), ("violence", 0.01), ("harassment", 0.85)],
        );

        let flagged = model.parse_result(&result);
        assert_eq!(flagged.len(), 2);

        let hate = flagged.iter().find(|f| f.category == SafetyCategory::Hate);
        assert!(hate.is_some());
        assert!((hate.unwrap().confidence.unwrap() - 0.92).abs() < 0.001);

        let harassment = flagged
            .iter()
            .find(|f| f.category == SafetyCategory::Harassment);
        assert!(harassment.is_some());
    }

    #[test]
    fn test_parse_with_enabled_categories_filter() {
        let model = OpenAIModerationModel::new(
            "test".into(),
            Arc::new(ModerationExecutor::new("http://localhost".into(), None)),
            "omni-moderation-latest".into(),
            Some(vec![SafetyCategory::Hate]), // Only monitor hate
        );

        let result = make_result(
            vec![("hate", true), ("harassment", true)],
            vec![("hate", 0.92), ("harassment", 0.85)],
        );

        let flagged = model.parse_result(&result);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].category, SafetyCategory::Hate);
    }
}
