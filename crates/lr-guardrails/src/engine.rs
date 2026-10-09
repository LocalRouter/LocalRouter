//! Safety engine - orchestrates checks across multiple safety models
//!
//! For MultiCategory models: one check() call
//! For SingleCategory models: parallel check() calls per enabled category

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tracing::{debug, info, warn};

use crate::executor::{ChatCompletionExecutor, ModelExecutor, ProviderExecutor};
use crate::models;
use crate::safety_model::*;
use crate::text_extractor;

/// Provider info needed to build executors for safety models
pub struct ProviderInfo {
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
    /// e.g. "ollama", "openai", "lmstudio"
    pub provider_type: String,
}

/// Simplified safety model config input for engine construction
pub struct SafetyModelConfigInput {
    pub id: String,
    pub model_type: String,
    pub provider_id: Option<String>,
    pub model_name: Option<String>,
    pub enabled_categories: Option<Vec<SafetyCategory>>,
}

/// Pseudo-category reported when a configured safety model fails to run
/// (provider error, timeout, oversized input, ...). Guardrails fail closed:
/// the failure is flagged with the default `Ask` action so the user decides,
/// and it can be tuned per client like any other category (for example
/// `guardrail_error: allow` to fail open deliberately).
pub const GUARDRAIL_ERROR_CATEGORY: &str = "guardrail_error";

/// The main safety engine that coordinates all safety model checks
pub struct SafetyEngine {
    models: Vec<Arc<dyn SafetyModel>>,
    confidence_threshold: f32,
    /// Models that failed to load, with (model_id, error_message) pairs.
    load_errors: Vec<(String, String)>,
}

impl SafetyEngine {
    /// Create a new safety engine
    pub fn new(models: Vec<Arc<dyn SafetyModel>>, confidence_threshold: f32) -> Self {
        Self {
            models,
            confidence_threshold,
            load_errors: Vec::new(),
        }
    }

    /// Create an empty engine (no models loaded)
    pub fn empty() -> Self {
        Self {
            models: Vec::new(),
            confidence_threshold: 0.5,
            load_errors: Vec::new(),
        }
    }

    /// Get models that failed to load: `(model_id, error_message)` pairs.
    pub fn load_errors(&self) -> &[(String, String)] {
        &self.load_errors
    }

    /// Build an engine from guardrails config
    ///
    /// `provider_lookup` maps provider names to their connection info.
    /// This allows the engine to be built without depending on the provider registry.
    pub fn from_config(
        safety_models: &[SafetyModelConfigInput],
        confidence_threshold: f32,
        provider_lookup: &HashMap<String, ProviderInfo>,
    ) -> Self {
        let mut model_instances: Vec<Arc<dyn SafetyModel>> = Vec::new();
        let mut load_errors: Vec<(String, String)> = Vec::new();

        for model_cfg in safety_models {
            // Build provider-based executor, choosing between legacy completions
            // and chat completions based on provider type
            let executor = if let (Some(provider_id), Some(model_name)) =
                (&model_cfg.provider_id, &model_cfg.model_name)
            {
                if let Some(provider) = provider_lookup.get(provider_id) {
                    match provider.provider_type.as_str() {
                        "ollama" => Arc::new(ModelExecutor::Provider(ProviderExecutor::new(
                            provider.base_url.clone(),
                            provider.api_key.clone(),
                            model_name.clone(),
                            false, // use OpenAI-compatible /v1/completions for logprobs support
                        ))),
                        // Cloud providers use /v1/chat/completions
                        "groq" | "deepinfra" | "togetherai" | "mistral" | "anthropic"
                        | "openai" | "openrouter" | "cohere" | "gemini" | "perplexity"
                        | "cerebras" | "xai" => {
                            Arc::new(ModelExecutor::ChatProvider(ChatCompletionExecutor::new(
                                provider.base_url.clone(),
                                provider.api_key.clone(),
                            )))
                        }
                        // Local providers (lmstudio, localai, etc.) use legacy /v1/completions
                        _ => Arc::new(ModelExecutor::Provider(ProviderExecutor::new(
                            provider.base_url.clone(),
                            provider.api_key.clone(),
                            model_name.clone(),
                            false,
                        ))),
                    }
                } else {
                    let error = format!("Provider '{provider_id}' not found");
                    warn!("Safety model '{}': {error}, skipping", model_cfg.id);
                    load_errors.push((model_cfg.id.clone(), error));
                    continue;
                }
            } else {
                let error = "Safety model requires provider_id and model_name".to_string();
                warn!("Safety model '{}': {error}, skipping", model_cfg.id);
                load_errors.push((model_cfg.id.clone(), error));
                continue;
            };

            let enabled_cats = model_cfg.enabled_categories.clone();

            let model: Arc<dyn SafetyModel> = match model_cfg.model_type.as_str() {
                "llama_guard_4" | "llama_guard" => {
                    Arc::new(models::llama_guard::LlamaGuardModel::new(
                        model_cfg.id.clone(),
                        executor,
                        model_cfg.model_name.clone().unwrap_or_default(),
                        enabled_cats,
                    ))
                }
                "shield_gemma" => Arc::new(models::shield_gemma::ShieldGemmaModel::new(
                    model_cfg.id.clone(),
                    executor,
                    model_cfg.model_name.clone().unwrap_or_default(),
                    enabled_cats,
                )),
                "nemotron" => Arc::new(models::nemotron::NemotronModel::new(
                    model_cfg.id.clone(),
                    executor,
                    model_cfg.model_name.clone().unwrap_or_default(),
                    enabled_cats,
                )),
                "granite_guardian" => {
                    Arc::new(models::granite_guardian::GraniteGuardianModel::new(
                        model_cfg.id.clone(),
                        executor,
                        model_cfg.model_name.clone().unwrap_or_default(),
                        enabled_cats,
                    ))
                }
                "openai_moderation" => {
                    // OpenAI moderation uses its own executor (calls /v1/moderations)
                    if let Some(provider) = model_cfg
                        .provider_id
                        .as_ref()
                        .and_then(|id| provider_lookup.get(id))
                    {
                        let mod_executor =
                            Arc::new(models::openai_moderation::ModerationExecutor::new(
                                provider.base_url.clone(),
                                provider.api_key.clone(),
                            ));
                        Arc::new(models::openai_moderation::OpenAIModerationModel::new(
                            model_cfg.id.clone(),
                            mod_executor,
                            model_cfg
                                .model_name
                                .clone()
                                .unwrap_or_else(|| "omni-moderation-latest".to_string()),
                            enabled_cats,
                        ))
                    } else {
                        warn!(
                            "Provider not found for OpenAI moderation model '{}', skipping",
                            model_cfg.id
                        );
                        continue;
                    }
                }
                "mistral_moderation" => {
                    // Mistral moderation uses its own executor (calls /v1/moderations)
                    if let Some(provider) = model_cfg
                        .provider_id
                        .as_ref()
                        .and_then(|id| provider_lookup.get(id))
                    {
                        let mod_executor =
                            Arc::new(models::mistral_moderation::MistralModerationExecutor::new(
                                provider.base_url.clone(),
                                provider.api_key.clone(),
                            ));
                        Arc::new(models::mistral_moderation::MistralModerationModel::new(
                            model_cfg.id.clone(),
                            mod_executor,
                            model_cfg
                                .model_name
                                .clone()
                                .unwrap_or_else(|| "mistral-moderation-latest".to_string()),
                            enabled_cats,
                        ))
                    } else {
                        warn!(
                            "Provider not found for Mistral moderation model '{}', skipping",
                            model_cfg.id
                        );
                        continue;
                    }
                }
                other => {
                    let error = format!("Unknown safety model type '{other}'");
                    warn!("Safety model '{}': {error}, skipping", model_cfg.id);
                    load_errors.push((model_cfg.id.clone(), error));
                    continue;
                }
            };

            info!(
                "Loaded safety model: {} (type: {}, provider: {})",
                model_cfg.id,
                model_cfg.model_type,
                model_cfg.provider_id.as_deref().unwrap_or("unknown"),
            );
            model_instances.push(model);
        }

        if !load_errors.is_empty() {
            warn!(
                "Safety engine: {} model(s) failed to load",
                load_errors.len()
            );
        }
        info!(
            "Safety engine initialized: {} models, {} load errors",
            model_instances.len(),
            load_errors.len(),
        );

        Self {
            models: model_instances,
            confidence_threshold,
            load_errors,
        }
    }

    /// Check if any models are configured
    pub fn has_models(&self) -> bool {
        !self.models.is_empty()
    }

    /// Get the number of configured models
    pub fn model_count(&self) -> usize {
        self.models.len()
    }

    /// Check input (request) content
    pub async fn check_input(&self, request_body: &serde_json::Value) -> SafetyCheckResult {
        let texts = text_extractor::extract_request_text(request_body);
        let messages: Vec<SafetyMessage> = texts
            .into_iter()
            .map(|t| SafetyMessage {
                role: t.role,
                content: t.text,
            })
            .collect();

        if messages.is_empty() {
            return SafetyCheckResult {
                verdicts: vec![],
                is_safe: true,
                actions_required: vec![],
                total_duration_ms: 0,
                errors: vec![],
            };
        }

        let input = SafetyCheckInput {
            messages,
            direction: ScanDirection::Input,
            target_category: None,
        };

        self.run_checks(&input).await
    }

    /// Check output (response) content
    pub async fn check_output(&self, response_body: &serde_json::Value) -> SafetyCheckResult {
        let texts = text_extractor::extract_response_text(response_body);
        let messages: Vec<SafetyMessage> = texts
            .into_iter()
            .map(|t| SafetyMessage {
                role: "assistant".to_string(),
                content: t.text,
            })
            .collect();

        if messages.is_empty() {
            return SafetyCheckResult {
                verdicts: vec![],
                is_safe: true,
                actions_required: vec![],
                total_duration_ms: 0,
                errors: vec![],
            };
        }

        let input = SafetyCheckInput {
            messages,
            direction: ScanDirection::Output,
            target_category: None,
        };

        self.run_checks(&input).await
    }

    /// Check raw text content against all models (for test panel)
    pub async fn check_text(&self, text: &str, direction: ScanDirection) -> SafetyCheckResult {
        let input = SafetyCheckInput {
            messages: vec![SafetyMessage {
                role: "user".to_string(),
                content: text.to_string(),
            }],
            direction,
            target_category: None,
        };

        self.run_checks(&input).await
    }

    /// Check raw text content against a single model by model_id
    pub async fn check_text_single_model(
        &self,
        text: &str,
        direction: ScanDirection,
        model_id: &str,
    ) -> SafetyCheckResult {
        let input = SafetyCheckInput {
            messages: vec![SafetyMessage {
                role: "user".to_string(),
                content: text.to_string(),
            }],
            direction,
            target_category: None,
        };

        self.run_checks_filtered(&input, Some(model_id)).await
    }

    /// Run all model checks
    async fn run_checks(&self, input: &SafetyCheckInput) -> SafetyCheckResult {
        self.run_checks_filtered(input, None).await
    }

    /// Run model checks, optionally filtered to a single model by ID
    async fn run_checks_filtered(
        &self,
        input: &SafetyCheckInput,
        model_id_filter: Option<&str>,
    ) -> SafetyCheckResult {
        let start = Instant::now();

        let models_to_run: Vec<_> = self
            .models
            .iter()
            .filter(|m| {
                if let Some(filter) = model_id_filter {
                    m.id() == filter
                } else {
                    true
                }
            })
            .collect();

        if models_to_run.is_empty() {
            return SafetyCheckResult {
                verdicts: vec![],
                is_safe: true,
                actions_required: vec![],
                total_duration_ms: 0,
                errors: vec![],
            };
        }

        // Run selected models in parallel
        let futures: Vec<_> = models_to_run
            .iter()
            .map(|model| {
                let model = (*model).clone();
                let input = input.clone();
                async move { model.check(&input).await }
            })
            .collect();

        let results = futures::future::join_all(futures).await;

        let mut verdicts = Vec::new();
        let mut all_actions = Vec::new();
        let mut errors = Vec::new();

        for (i, result) in results.into_iter().enumerate() {
            match result {
                Ok(mut verdict) => {
                    // Set the human-readable label from the model's display_name
                    if let Some(model) = models_to_run.get(i) {
                        verdict.model_label = Some(model.display_name().to_string());
                    }
                    // Carried on each action so per-model-type (`__model:<type>`)
                    // category overrides can be resolved downstream.
                    let model_type = models_to_run
                        .get(i)
                        .map(|m| m.model_type_id().to_string())
                        .unwrap_or_default();
                    if verdict.flagged_categories.is_empty() && !verdict.is_safe {
                        // Model says unsafe but didn't specify categories (e.g. Llama Guard
                        // with no parseable S-codes). Generate a generic action.
                        all_actions.push(CategoryActionRequired {
                            category: SafetyCategory::Custom("unspecified".to_string()),
                            action: CategoryAction::Ask,
                            model_id: verdict.model_id.clone(),
                            model_type: model_type.clone(),
                            confidence: None,
                        });
                    }

                    // Collect flagged categories as actions (default: Ask)
                    for flagged in &verdict.flagged_categories {
                        // Skip if below confidence threshold.
                        // When confidence is None (text-parsed, no logprobs),
                        // treat as full confidence (fail-closed).
                        let conf = flagged.confidence.unwrap_or(1.0);
                        if conf < self.confidence_threshold {
                            continue;
                        }

                        all_actions.push(CategoryActionRequired {
                            category: flagged.category.clone(),
                            action: CategoryAction::Ask,
                            model_id: verdict.model_id.clone(),
                            model_type: model_type.clone(),
                            confidence: Some(conf),
                        });
                    }
                    verdicts.push(verdict);
                }
                Err(e) => {
                    let model_id = models_to_run
                        .get(i)
                        .map(|m| m.id().to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    let model_type = models_to_run
                        .get(i)
                        .map(|m| m.model_type_id().to_string())
                        .unwrap_or_default();
                    warn!("Safety model '{}' check failed: {}", model_id, e);
                    // Fail closed: a model that could not run has not cleared the
                    // content. Surface it as a flagged pseudo-category so the request
                    // goes through the normal Ask/Block/Allow policy instead of being
                    // silently treated as safe (an attacker could otherwise bypass the
                    // guardrail by making the model error, e.g. with an oversized prompt).
                    all_actions.push(CategoryActionRequired {
                        category: SafetyCategory::Custom(GUARDRAIL_ERROR_CATEGORY.to_string()),
                        action: CategoryAction::Ask,
                        model_id: model_id.clone(),
                        model_type,
                        confidence: None,
                    });
                    errors.push(SafetyModelError { model_id, error: e });
                }
            }
        }

        // A check is only "safe" when every model ran and none flagged anything.
        let is_safe = errors.is_empty() && verdicts.iter().all(|v| v.is_safe);

        let total_duration_ms = start.elapsed().as_millis() as u64;

        debug!(
            "Safety check: {} models, {} verdicts, {} errors, {} actions, {}ms",
            self.models.len(),
            verdicts.len(),
            errors.len(),
            all_actions.len(),
            total_duration_ms
        );

        SafetyCheckResult {
            verdicts,
            is_safe,
            actions_required: all_actions,
            total_duration_ms,
            errors,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_models_report_load_errors_instead_of_disappearing() {
        let providers = HashMap::from([(
            "local".to_string(),
            ProviderInfo {
                name: "local".into(),
                base_url: "http://127.0.0.1:1".into(),
                api_key: None,
                provider_type: "ollama".into(),
            },
        )]);
        let config = vec![
            SafetyModelConfigInput {
                id: "missing-provider".into(),
                model_type: "llama_guard".into(),
                provider_id: Some("absent".into()),
                model_name: Some("guard".into()),
                enabled_categories: None,
            },
            SafetyModelConfigInput {
                id: "missing-model".into(),
                model_type: "llama_guard".into(),
                provider_id: Some("local".into()),
                model_name: None,
                enabled_categories: None,
            },
            SafetyModelConfigInput {
                id: "unknown-type".into(),
                model_type: "unsupported".into(),
                provider_id: Some("local".into()),
                model_name: Some("guard".into()),
                enabled_categories: None,
            },
        ];
        // Building executors performs no requests or model inference.
        let engine = SafetyEngine::from_config(&config, 0.5, &providers);
        assert_eq!(engine.model_count(), 0);
        assert_eq!(engine.load_errors().len(), 3);
        for model in config {
            assert!(engine
                .load_errors()
                .iter()
                .any(|(id, error)| id == &model.id && !error.is_empty()));
        }
    }

    #[test]
    fn test_empty_engine() {
        let engine = SafetyEngine::empty();
        assert!(!engine.has_models());
        assert_eq!(engine.model_count(), 0);
    }

    #[tokio::test]
    async fn test_check_empty_input() {
        let engine = SafetyEngine::empty();
        let body = serde_json::json!({});
        let result = engine.check_input(&body).await;
        assert!(result.is_safe);
        assert!(result.verdicts.is_empty());
    }

    #[tokio::test]
    async fn test_check_empty_messages() {
        let engine = SafetyEngine::empty();
        let body = serde_json::json!({"messages": []});
        let result = engine.check_input(&body).await;
        assert!(result.is_safe);
    }

    #[test]
    fn test_safety_check_result_methods() {
        let result = SafetyCheckResult {
            verdicts: vec![],
            is_safe: true,
            actions_required: vec![],
            total_duration_ms: 0,
            errors: vec![],
        };
        assert!(!result.needs_approval());
        assert!(!result.needs_notification());
        assert!(!result.has_flags());

        let result_with_ask = SafetyCheckResult {
            verdicts: vec![],
            is_safe: false,
            actions_required: vec![CategoryActionRequired {
                category: SafetyCategory::Hate,
                action: CategoryAction::Ask,
                model_id: "test".to_string(),
                model_type: "llama_guard".to_string(),
                confidence: Some(0.9),
            }],
            total_duration_ms: 0,
            errors: vec![],
        };
        assert!(result_with_ask.needs_approval());
        assert!(result_with_ask.has_flags());

        let result_with_notify = SafetyCheckResult {
            verdicts: vec![],
            is_safe: false,
            actions_required: vec![CategoryActionRequired {
                category: SafetyCategory::Profanity,
                action: CategoryAction::Notify,
                model_id: "test".to_string(),
                model_type: "llama_guard".to_string(),
                confidence: Some(0.8),
            }],
            total_duration_ms: 0,
            errors: vec![],
        };
        assert!(result_with_notify.needs_notification());
        assert!(!result_with_notify.needs_approval());
    }

    /// Mock safety model for testing engine behavior
    struct MockSafetyModel {
        id: String,
        verdict: SafetyVerdict,
    }

    impl MockSafetyModel {
        fn safe(id: &str) -> Self {
            Self {
                id: id.to_string(),
                verdict: SafetyVerdict {
                    model_id: id.to_string(),
                    model_label: None,
                    is_safe: true,
                    flagged_categories: vec![],
                    confidence: None,
                    raw_output: "safe".to_string(),
                    check_duration_ms: 1,
                },
            }
        }

        fn unsafe_with_categories(id: &str, categories: Vec<FlaggedCategory>) -> Self {
            Self {
                id: id.to_string(),
                verdict: SafetyVerdict {
                    model_id: id.to_string(),
                    model_label: None,
                    is_safe: false,
                    flagged_categories: categories,
                    confidence: None,
                    raw_output: "unsafe".to_string(),
                    check_duration_ms: 1,
                },
            }
        }

        fn unsafe_no_categories(id: &str) -> Self {
            Self {
                id: id.to_string(),
                verdict: SafetyVerdict {
                    model_id: id.to_string(),
                    model_label: None,
                    is_safe: false,
                    flagged_categories: vec![],
                    confidence: None,
                    raw_output: "unsafe".to_string(),
                    check_duration_ms: 1,
                },
            }
        }
    }

    /// Mock safety model whose check always fails (provider down, 4xx, timeout...)
    struct FailingSafetyModel {
        id: String,
    }

    #[async_trait::async_trait]
    impl SafetyModel for FailingSafetyModel {
        fn id(&self) -> &str {
            &self.id
        }
        fn model_type_id(&self) -> &str {
            "failing_type"
        }
        fn display_name(&self) -> &str {
            &self.id
        }
        fn supported_categories(&self) -> Vec<SafetyCategoryInfo> {
            vec![]
        }
        fn inference_mode(&self) -> InferenceMode {
            InferenceMode::MultiCategory
        }
        async fn check(&self, _input: &SafetyCheckInput) -> Result<SafetyVerdict, String> {
            Err("Provider returned 400: context length exceeded".to_string())
        }
    }

    #[async_trait::async_trait]
    impl SafetyModel for MockSafetyModel {
        fn id(&self) -> &str {
            &self.id
        }
        fn model_type_id(&self) -> &str {
            &self.id
        }
        fn display_name(&self) -> &str {
            &self.id
        }
        fn supported_categories(&self) -> Vec<SafetyCategoryInfo> {
            vec![]
        }
        fn inference_mode(&self) -> InferenceMode {
            InferenceMode::MultiCategory
        }
        async fn check(&self, _input: &SafetyCheckInput) -> Result<SafetyVerdict, String> {
            Ok(self.verdict.clone())
        }
    }

    /// Security regression: a safety model that errors must not be treated as a
    /// clean verdict. Before this test, `verdicts.iter().all(..)` on an empty
    /// list made the request "safe" whenever every model failed.
    #[tokio::test]
    async fn test_engine_all_models_failing_fails_closed() {
        let engine = SafetyEngine::new(
            vec![Arc::new(FailingSafetyModel {
                id: "broken".to_string(),
            })],
            0.5,
        );

        let result = engine.check_text("anything", ScanDirection::Input).await;
        assert!(!result.is_safe, "errors must not yield a safe verdict");
        assert!(result.verdicts.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].model_id, "broken");
        assert_eq!(result.actions_required.len(), 1);
        let action = &result.actions_required[0];
        assert!(matches!(action.action, CategoryAction::Ask));
        assert_eq!(
            action.category,
            SafetyCategory::Custom(GUARDRAIL_ERROR_CATEGORY.to_string())
        );
        assert_eq!(action.model_id, "broken");
        assert_eq!(action.model_type, "failing_type");
        assert!(result.needs_approval());
    }

    /// A partial failure is also not safe: the failed model's categories were
    /// never checked, even though the surviving model returned "safe".
    #[tokio::test]
    async fn test_engine_partial_failure_fails_closed() {
        let engine = SafetyEngine::new(
            vec![
                Arc::new(MockSafetyModel::safe("ok")),
                Arc::new(FailingSafetyModel {
                    id: "broken".to_string(),
                }),
            ],
            0.5,
        );

        let result = engine.check_text("anything", ScanDirection::Input).await;
        assert!(!result.is_safe);
        assert_eq!(result.verdicts.len(), 1);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.actions_required.len(), 1);
        assert_eq!(result.actions_required[0].model_id, "broken");
    }

    /// The error pseudo-category can be tuned like any other category, so a user
    /// who prefers fail-open can configure `guardrail_error: allow` explicitly.
    #[tokio::test]
    async fn test_engine_failure_category_can_be_overridden_to_allow() {
        let engine = SafetyEngine::new(
            vec![Arc::new(FailingSafetyModel {
                id: "broken".to_string(),
            })],
            0.5,
        );

        let result = engine
            .check_text("anything", ScanDirection::Input)
            .await
            .apply_client_category_overrides(&[(
                GUARDRAIL_ERROR_CATEGORY.to_string(),
                CategoryAction::Allow,
            )]);
        assert!(result.is_safe);
        assert!(result.actions_required.is_empty());
        // The error is still reported for diagnostics.
        assert_eq!(result.errors.len(), 1);
    }

    #[tokio::test]
    async fn test_engine_safe_model() {
        let engine = SafetyEngine::new(vec![Arc::new(MockSafetyModel::safe("mock"))], 0.5);

        let result = engine.check_text("hello world", ScanDirection::Input).await;
        assert!(result.is_safe);
        assert_eq!(result.verdicts.len(), 1);
        assert!(result.actions_required.is_empty());
    }

    #[tokio::test]
    async fn test_engine_unsafe_model_with_categories() {
        let engine = SafetyEngine::new(
            vec![Arc::new(MockSafetyModel::unsafe_with_categories(
                "mock",
                vec![FlaggedCategory {
                    category: SafetyCategory::Hate,
                    confidence: Some(0.9),
                    native_label: "S10".to_string(),
                }],
            ))],
            0.5,
        );

        let result = engine
            .check_text("hateful content", ScanDirection::Input)
            .await;
        assert!(!result.is_safe);
        assert_eq!(result.verdicts.len(), 1);
        assert_eq!(result.actions_required.len(), 1);
        // All flagged categories default to Ask
        assert!(matches!(
            result.actions_required[0].action,
            CategoryAction::Ask
        ));
    }

    /// Bug fix test: unsafe verdict with no categories should still generate action
    #[tokio::test]
    async fn test_engine_unsafe_no_categories_generates_action() {
        let engine = SafetyEngine::new(
            vec![Arc::new(MockSafetyModel::unsafe_no_categories("mock"))],
            0.5,
        );

        let result = engine.check_text("bad content", ScanDirection::Input).await;
        assert!(!result.is_safe);
        assert_eq!(result.actions_required.len(), 1);
        assert!(matches!(
            result.actions_required[0].category,
            SafetyCategory::Custom(_)
        ));
    }

    /// Test confidence threshold filtering
    #[tokio::test]
    async fn test_engine_confidence_threshold() {
        let engine = SafetyEngine::new(
            vec![Arc::new(MockSafetyModel::unsafe_with_categories(
                "mock",
                vec![FlaggedCategory {
                    category: SafetyCategory::Hate,
                    confidence: Some(0.3), // below threshold
                    native_label: "S10".to_string(),
                }],
            ))],
            0.5, // threshold
        );

        let result = engine
            .check_text("borderline content", ScanDirection::Input)
            .await;
        // Verdict is still unsafe, but the action is filtered out by threshold
        assert!(!result.is_safe); // is_safe comes from verdict, not actions
        assert!(result.actions_required.is_empty()); // filtered by threshold
    }

    /// Test multiple models running in parallel
    #[tokio::test]
    async fn test_engine_multiple_models() {
        let engine = SafetyEngine::new(
            vec![
                Arc::new(MockSafetyModel::safe("model_a")),
                Arc::new(MockSafetyModel::unsafe_with_categories(
                    "model_b",
                    vec![FlaggedCategory {
                        category: SafetyCategory::ViolentCrimes,
                        confidence: Some(0.95),
                        native_label: "S1".to_string(),
                    }],
                )),
            ],
            0.5,
        );

        let result = engine
            .check_text("potentially violent", ScanDirection::Input)
            .await;
        assert!(!result.is_safe); // one model flagged it
        assert_eq!(result.verdicts.len(), 2);
        assert_eq!(result.actions_required.len(), 1);
    }

    /// Test that None confidence (text-parsed, no logprobs) is treated as 1.0
    #[tokio::test]
    async fn test_engine_none_confidence_treated_as_full() {
        let engine = SafetyEngine::new(
            vec![Arc::new(MockSafetyModel::unsafe_with_categories(
                "mock",
                vec![FlaggedCategory {
                    category: SafetyCategory::Jailbreak,
                    confidence: None, // no logprobs available
                    native_label: "jailbreak".to_string(),
                }],
            ))],
            0.99, // very high threshold
        );

        let result = engine
            .check_text("test message", ScanDirection::Input)
            .await;
        // None confidence → treated as 1.0, which passes even 0.99 threshold
        assert!(!result.is_safe);
        assert_eq!(result.actions_required.len(), 1);
        assert_eq!(result.actions_required[0].confidence, Some(1.0));
    }

    /// Test single model filtering (Bug 7 fix)
    #[tokio::test]
    async fn test_engine_single_model_check() {
        let engine = SafetyEngine::new(
            vec![
                Arc::new(MockSafetyModel::safe("model_a")),
                Arc::new(MockSafetyModel::unsafe_with_categories(
                    "model_b",
                    vec![FlaggedCategory {
                        category: SafetyCategory::Hate,
                        confidence: Some(0.9),
                        native_label: "hate".to_string(),
                    }],
                )),
            ],
            0.5,
        );

        // Check only model_a (safe)
        let result = engine
            .check_text_single_model("test", ScanDirection::Input, "model_a")
            .await;
        assert!(result.is_safe);
        assert_eq!(result.verdicts.len(), 1);

        // Check only model_b (unsafe)
        let result = engine
            .check_text_single_model("test", ScanDirection::Input, "model_b")
            .await;
        assert!(!result.is_safe);
        assert_eq!(result.verdicts.len(), 1);
        assert_eq!(result.actions_required.len(), 1);
    }
}
