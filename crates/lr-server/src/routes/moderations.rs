//! POST /v1/moderations endpoint
//!
//! Content safety classification using configured guardrails safety models.
//! Returns results in OpenAI-compatible moderation response format.

use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Extension, Json,
};
use std::collections::HashMap;
use uuid::Uuid;

use super::helpers::{check_llm_access, get_enabled_client};
use crate::middleware::error::{ApiErrorResponse, ApiResult};
use crate::state::{AppState, AuthContext};
use crate::types::{ModerationInput, ModerationRequest, ModerationResponse, ModerationResult};
use lr_guardrails::{SafetyCategory, SafetyCheckResult, ScanDirection};

/// POST /v1/moderations
/// Classify content for safety using configured guardrails models
#[utoipa::path(
    post,
    path = "/v1/moderations",
    tag = "moderations",
    request_body = ModerationRequest,
    responses(
        (status = 200, description = "Successful response", body = ModerationResponse),
        (status = 400, description = "Bad request", body = crate::types::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 503, description = "Service unavailable", body = crate::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::types::ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn moderations(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(request): Json<ModerationRequest>,
) -> ApiResult<Response> {
    // Emit event
    state.emit_event("llm-request", "moderation");

    // Record client activity
    state.record_client_activity(&auth.api_key_id);

    // Validate client is enabled and has LLM access
    if auth.api_key_id != "internal-test" {
        let client = get_enabled_client(&state, &auth.api_key_id)?;
        check_llm_access(&client)?;
    }

    // Check if moderation endpoint is enabled
    let config = state.config_manager.get();
    if !config.guardrails.moderation_api_enabled {
        return Err(ApiErrorResponse::service_unavailable(
            "Moderation API endpoint is disabled. Enable it in Settings > GuardRails.",
        ));
    }

    // Get safety engine (clone the Arc to avoid holding the RwLock across await)
    let engine = {
        let guard = state.safety_engine.read();
        guard
            .as_ref()
            .ok_or_else(|| {
                ApiErrorResponse::service_unavailable(
                    "No safety models configured. Add safety models in Settings > GuardRails.",
                )
            })?
            .clone()
    };

    if !engine.has_models() {
        return Err(ApiErrorResponse::service_unavailable(
            "No safety models loaded. Add safety models in Settings > GuardRails.",
        ));
    }

    // Extract input texts
    let texts = match &request.input {
        ModerationInput::Single(s) => vec![s.clone()],
        ModerationInput::Multiple(v) => v.clone(),
    };

    if texts.is_empty() || texts.iter().all(|t| t.is_empty()) {
        return Err(ApiErrorResponse::bad_request("input cannot be empty").with_param("input"));
    }

    // Run safety check for each input text
    let mut results = Vec::with_capacity(texts.len());
    for text in &texts {
        let check_result = engine.check_text(text, ScanDirection::Input).await;
        results.push(translate_to_moderation_result(
            &check_result,
            config.guardrails.default_confidence_threshold,
        ));
    }

    let response = ModerationResponse {
        id: format!("modr-{}", Uuid::new_v4()),
        model: request
            .model
            .unwrap_or_else(|| "localrouter-guardrails".to_string()),
        results,
    };

    tracing::info!(
        "Moderation request: client={}, inputs={}, flagged={}",
        &auth.api_key_id[..8.min(auth.api_key_id.len())],
        texts.len(),
        response.results.iter().filter(|r| r.flagged).count(),
    );

    Ok(Json(response).into_response())
}

// ── Category mapping: SafetyCategory → OpenAI moderation fields ──

/// Standard OpenAI moderation categories and their SafetyCategory sources
const STANDARD_CATEGORY_MAP: &[(&str, &[SafetyCategory])] = &[
    ("hate", &[SafetyCategory::Hate]),
    ("hate/threatening", &[SafetyCategory::Hate]),
    ("harassment", &[SafetyCategory::Harassment]),
    ("harassment/threatening", &[SafetyCategory::Harassment]),
    ("self-harm", &[SafetyCategory::SelfHarm]),
    ("self-harm/intent", &[SafetyCategory::SelfHarm]),
    ("self-harm/instructions", &[SafetyCategory::SelfHarm]),
    ("sexual", &[SafetyCategory::SexualContent]),
    ("sexual/minors", &[SafetyCategory::ChildExploitation]),
    ("violence", &[SafetyCategory::ViolentCrimes]),
    ("violence/graphic", &[SafetyCategory::ViolentCrimes]),
    ("illicit", &[SafetyCategory::IllegalActivity]),
    ("illicit/violent", &[SafetyCategory::CriminalPlanning]),
];

/// Extra SafetyCategories not in the OpenAI spec, returned alongside standard ones
const EXTRA_CATEGORIES: &[(SafetyCategory, &str)] = &[
    (SafetyCategory::NonViolentCrimes, "non_violent_crimes"),
    (SafetyCategory::SexCrimes, "sex_crimes"),
    (SafetyCategory::Defamation, "defamation"),
    (SafetyCategory::SpecializedAdvice, "specialized_advice"),
    (SafetyCategory::Privacy, "privacy"),
    (
        SafetyCategory::IntellectualProperty,
        "intellectual_property",
    ),
    (
        SafetyCategory::IndiscriminateWeapons,
        "indiscriminate_weapons",
    ),
    (SafetyCategory::Elections, "elections"),
    (
        SafetyCategory::CodeInterpreterAbuse,
        "code_interpreter_abuse",
    ),
    (SafetyCategory::DangerousContent, "dangerous_content"),
    (SafetyCategory::GunsIllegalWeapons, "guns_illegal_weapons"),
    (
        SafetyCategory::ControlledSubstances,
        "controlled_substances",
    ),
    (SafetyCategory::Profanity, "profanity"),
    (SafetyCategory::NeedsCaution, "needs_caution"),
    (SafetyCategory::Manipulation, "manipulation"),
    (SafetyCategory::FraudDeception, "fraud_deception"),
    (SafetyCategory::Malware, "malware"),
    (
        SafetyCategory::HighRiskGovDecision,
        "high_risk_gov_decision",
    ),
    (
        SafetyCategory::PoliticalMisinformation,
        "political_misinformation",
    ),
    (SafetyCategory::CopyrightPlagiarism, "copyright_plagiarism"),
    (SafetyCategory::UnauthorizedAdvice, "unauthorized_advice"),
    (SafetyCategory::ImmoralUnethical, "immoral_unethical"),
    (SafetyCategory::SocialBias, "social_bias"),
    (SafetyCategory::Jailbreak, "jailbreak"),
    (SafetyCategory::UnethicalBehavior, "unethical_behavior"),
];

/// Get the max confidence for a SafetyCategory from the check result
fn max_confidence_for(result: &SafetyCheckResult, categories: &[SafetyCategory]) -> f64 {
    result
        .verdicts
        .iter()
        .flat_map(|v| &v.flagged_categories)
        .filter(|f| categories.contains(&f.category))
        .filter_map(|f| f.confidence)
        .fold(0.0_f64, |acc, c| acc.max(c as f64))
}

/// Check if a SafetyCategory was flagged (with any confidence above zero)
fn is_flagged(result: &SafetyCheckResult, categories: &[SafetyCategory]) -> bool {
    result
        .verdicts
        .iter()
        .flat_map(|v| &v.flagged_categories)
        .any(|f| categories.contains(&f.category))
}

/// Translate a SafetyCheckResult into an OpenAI-compatible ModerationResult
fn translate_to_moderation_result(result: &SafetyCheckResult, _threshold: f32) -> ModerationResult {
    let mut categories = HashMap::new();
    let mut category_scores = HashMap::new();

    // Standard OpenAI categories
    for (name, sources) in STANDARD_CATEGORY_MAP {
        let flagged = is_flagged(result, sources);
        let score = if flagged {
            let s = max_confidence_for(result, sources);
            if s == 0.0 {
                1.0
            } else {
                s
            } // Binary models: use 1.0 when flagged but no score
        } else {
            0.0
        };
        categories.insert(name.to_string(), flagged);
        category_scores.insert(name.to_string(), score);
    }

    // Extra categories from safety models (not in OpenAI spec)
    for (safety_cat, name) in EXTRA_CATEGORIES {
        let cat_slice = std::slice::from_ref(safety_cat);
        let flagged = is_flagged(result, cat_slice);
        if flagged {
            let score = max_confidence_for(result, cat_slice);
            categories.insert(name.to_string(), true);
            category_scores.insert(name.to_string(), if score == 0.0 { 1.0 } else { score });
        }
    }

    let any_flagged = categories.values().any(|&v| v);

    ModerationResult {
        flagged: any_flagged,
        categories,
        category_scores,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_guardrails::{FlaggedCategory, SafetyVerdict};

    fn make_result(flagged: Vec<(SafetyCategory, Option<f32>)>) -> SafetyCheckResult {
        let is_safe = flagged.is_empty();
        let flagged_categories: Vec<FlaggedCategory> = flagged
            .into_iter()
            .map(|(cat, conf)| FlaggedCategory {
                category: cat,
                confidence: conf,
                native_label: String::new(),
            })
            .collect();

        SafetyCheckResult {
            verdicts: vec![SafetyVerdict {
                model_id: "test".into(),
                is_safe,
                flagged_categories,
                confidence: None,
                raw_output: String::new(),
                check_duration_ms: 0,
            }],
            is_safe,
            actions_required: vec![],
            total_duration_ms: 0,
        }
    }

    #[test]
    fn test_translate_safe_result() {
        let result = make_result(vec![]);
        let moderation = translate_to_moderation_result(&result, 0.5);
        assert!(!moderation.flagged);
        assert_eq!(moderation.categories.get("hate"), Some(&false));
        assert_eq!(moderation.category_scores.get("hate"), Some(&0.0));
    }

    #[test]
    fn test_translate_hate_flagged() {
        let result = make_result(vec![(SafetyCategory::Hate, Some(0.92))]);
        let moderation = translate_to_moderation_result(&result, 0.5);
        assert!(moderation.flagged);
        assert_eq!(moderation.categories.get("hate"), Some(&true));
        assert_eq!(moderation.categories.get("hate/threatening"), Some(&true));
        assert!((moderation.category_scores["hate"] - 0.92).abs() < 0.001);
    }

    #[test]
    fn test_translate_binary_model_no_score() {
        // Binary models (e.g. Llama Guard) flag without confidence scores
        let result = make_result(vec![(SafetyCategory::ViolentCrimes, None)]);
        let moderation = translate_to_moderation_result(&result, 0.5);
        assert!(moderation.flagged);
        assert_eq!(moderation.categories.get("violence"), Some(&true));
        // Should default to 1.0 for binary flagged
        assert_eq!(moderation.category_scores.get("violence"), Some(&1.0));
    }

    #[test]
    fn test_translate_extra_categories() {
        let result = make_result(vec![(SafetyCategory::Jailbreak, Some(0.88))]);
        let moderation = translate_to_moderation_result(&result, 0.5);
        assert!(moderation.flagged);
        // Standard categories should be false
        assert_eq!(moderation.categories.get("hate"), Some(&false));
        // Extra category should appear
        assert_eq!(moderation.categories.get("jailbreak"), Some(&true));
        assert!((moderation.category_scores["jailbreak"] - 0.88).abs() < 0.001);
    }

    #[test]
    fn test_translate_mixed_standard_and_extra() {
        let result = make_result(vec![
            (SafetyCategory::Hate, Some(0.95)),
            (SafetyCategory::Profanity, Some(0.70)),
        ]);
        let moderation = translate_to_moderation_result(&result, 0.5);
        assert!(moderation.flagged);
        assert_eq!(moderation.categories.get("hate"), Some(&true));
        assert_eq!(moderation.categories.get("profanity"), Some(&true));
        assert_eq!(moderation.categories.get("violence"), Some(&false));
    }
}
