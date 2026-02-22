//! Safety engine - orchestrates checks across multiple safety models
//!
//! For MultiCategory models: one check() call
//! For SingleCategory models: parallel check() calls per enabled category

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tracing::{debug, info, warn};

use crate::executor::{LocalGgufExecutor, ModelExecutor, ProviderExecutor};
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
    pub execution_mode: Option<String>,
    pub hf_repo_id: Option<String>,
    pub gguf_filename: Option<String>,
}

/// The main safety engine that coordinates all safety model checks
pub struct SafetyEngine {
    models: Vec<Arc<dyn SafetyModel>>,
    confidence_threshold: f32,
    /// Models that failed to load, with (model_id, error_message) pairs.
    load_errors: Vec<(String, String)>,
}

impl SafetyEngine {
    /// Create a new safety engine
    pub fn new(
        models: Vec<Arc<dyn SafetyModel>>,
        confidence_threshold: f32,
    ) -> Self {
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
        context_size: u32,
    ) -> Self {
        let mut model_instances: Vec<Arc<dyn SafetyModel>> = Vec::new();
        let mut load_errors: Vec<(String, String)> = Vec::new();

        for model_cfg in safety_models {
            // Build executor based on execution mode
            let exec_mode = model_cfg
                .execution_mode
                .as_deref()
                .unwrap_or("direct_download");

            let executor = match exec_mode {
                "direct_download" | "custom_download" => {
                    // Load GGUF model directly from disk via llama.cpp
                    let gguf_filename = match &model_cfg.gguf_filename {
                        Some(f) => f,
                        None => {
                            warn!(
                                "Safety model '{}' uses {} mode but has no gguf_filename, skipping",
                                model_cfg.id, exec_mode
                            );
                            continue;
                        }
                    };

                    let gguf_path =
                        match crate::downloader::model_file_path(&model_cfg.id, gguf_filename) {
                            Ok(p) => p,
                            Err(e) => {
                                warn!("Failed to resolve GGUF path for '{}': {}", model_cfg.id, e);
                                continue;
                            }
                        };

                    if !gguf_path.exists() {
                        debug!(
                            "GGUF file not downloaded yet for '{}': {}",
                            model_cfg.id,
                            gguf_path.display()
                        );
                        continue;
                    }

                    match LocalGgufExecutor::new(gguf_path, context_size) {
                        Ok(exec) => Arc::new(ModelExecutor::Local(exec)),
                        Err(e) => {
                            warn!(
                                "Failed to load GGUF model '{}': {}",
                                model_cfg.id, e
                            );
                            load_errors.push((model_cfg.id.clone(), e));
                            continue;
                        }
                    }
                }
                "provider" => {
                    if let (Some(provider_id), Some(model_name)) =
                        (&model_cfg.provider_id, &model_cfg.model_name)
                    {
                        if let Some(provider) = provider_lookup.get(provider_id) {
                            let use_ollama = provider.provider_type == "ollama";
                            Arc::new(ModelExecutor::Provider(ProviderExecutor::new(
                                provider.base_url.clone(),
                                provider.api_key.clone(),
                                model_name.clone(),
                                use_ollama,
                            )))
                        } else {
                            warn!(
                                "Provider '{}' not found for safety model '{}', skipping",
                                provider_id, model_cfg.id
                            );
                            continue;
                        }
                    } else {
                        warn!(
                            "Safety model '{}' has no provider_id or model_name, skipping",
                            model_cfg.id
                        );
                        continue;
                    }
                }
                _ => {
                    warn!(
                        "Unknown execution mode '{}' for safety model '{}', skipping",
                        exec_mode, model_cfg.id
                    );
                    continue;
                }
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
                other => {
                    warn!("Unknown safety model type '{}', skipping", other);
                    continue;
                }
            };

            info!(
                "Loaded safety model: {} (type: {}, mode: {})",
                model_cfg.id, model_cfg.model_type, exec_mode,
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
                role: t.label.clone(),
                content: t.text,
            })
            .collect();

        if messages.is_empty() {
            return SafetyCheckResult {
                verdicts: vec![],
                is_safe: true,
                actions_required: vec![],
                total_duration_ms: 0,
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

        for result in results {
            match result {
                Ok(verdict) => {
                    if verdict.flagged_categories.is_empty() && !verdict.is_safe {
                        // Model says unsafe but didn't specify categories (e.g. Llama Guard
                        // with no parseable S-codes). Generate a generic action.
                        all_actions.push(CategoryActionRequired {
                            category: SafetyCategory::Custom("unspecified".to_string()),
                            action: CategoryAction::Ask,
                            model_id: verdict.model_id.clone(),
                            confidence: None,
                        });
                    }

                    // Collect flagged categories as actions (default: Ask)
                    for flagged in &verdict.flagged_categories {
                        // Skip if below confidence threshold
                        if let Some(conf) = flagged.confidence {
                            if conf < self.confidence_threshold {
                                continue;
                            }
                        }

                        all_actions.push(CategoryActionRequired {
                            category: flagged.category.clone(),
                            action: CategoryAction::Ask,
                            model_id: verdict.model_id.clone(),
                            confidence: flagged.confidence,
                        });
                    }
                    verdicts.push(verdict);
                }
                Err(e) => {
                    warn!("Safety model check failed: {}", e);
                }
            }
        }

        let is_safe = verdicts.iter().all(|v| v.is_safe);

        let total_duration_ms = start.elapsed().as_millis() as u64;

        debug!(
            "Safety check: {} models, {} verdicts, {} actions, {}ms",
            self.models.len(),
            verdicts.len(),
            all_actions.len(),
            total_duration_ms
        );

        SafetyCheckResult {
            verdicts,
            is_safe,
            actions_required: all_actions,
            total_duration_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                confidence: Some(0.9),
            }],
            total_duration_ms: 0,
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
                confidence: Some(0.8),
            }],
            total_duration_ms: 0,
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
                    is_safe: false,
                    flagged_categories: vec![],
                    confidence: None,
                    raw_output: "unsafe".to_string(),
                    check_duration_ms: 1,
                },
            }
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

    #[tokio::test]
    async fn test_engine_safe_model() {
        let engine = SafetyEngine::new(
            vec![Arc::new(MockSafetyModel::safe("mock"))],
            0.5,
        );

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
