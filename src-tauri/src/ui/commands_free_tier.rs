//! Tauri commands for free tier management

use lr_config::{ConfigManager, FreeTierKind};
use lr_router::FreeTierManager;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{Emitter, State};

/// Free tier status for a single provider (for UI display)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderFreeTierStatus {
    pub provider_instance: String,
    pub provider_type: String,
    pub display_name: String,
    /// Effective config (user override or default)
    pub free_tier: FreeTierKind,
    pub is_user_override: bool,
    pub supports_credit_check: bool,
    // Rate-limited status
    pub rate_rpm_used: Option<u32>,
    pub rate_rpm_limit: Option<u32>,
    pub rate_rpd_used: Option<u32>,
    pub rate_rpd_limit: Option<u32>,
    pub rate_tpm_used: Option<u64>,
    pub rate_tpm_limit: Option<u64>,
    pub rate_tpd_used: Option<u64>,
    pub rate_tpd_limit: Option<u64>,
    pub rate_monthly_calls_used: Option<u32>,
    pub rate_monthly_calls_limit: Option<u32>,
    pub rate_monthly_tokens_used: Option<u64>,
    pub rate_monthly_tokens_limit: Option<u64>,
    // Credit-based status
    pub credit_used_usd: Option<f64>,
    pub credit_budget_usd: Option<f64>,
    pub credit_remaining_usd: Option<f64>,
    // Backoff status
    pub is_backed_off: bool,
    pub backoff_retry_after_secs: Option<u64>,
    pub backoff_reason: Option<String>,
    // Cost backoff (cost watchdog)
    pub cost_backoff_active: bool,
    pub cost_backoff_retry_after_secs: Option<u64>,
    pub cost_backoff_duration_secs: Option<u64>,
    pub cost_backoff_last_trigger: Option<String>,
    pub cost_backoff_trigger_count: u32,
    // Summary
    pub has_capacity: bool,
    pub status_message: String,
}

/// Get free tier status for all configured providers
#[tauri::command]
pub async fn get_free_tier_status(
    config_manager: State<'_, ConfigManager>,
    free_tier_manager: State<'_, Arc<FreeTierManager>>,
    provider_registry: State<'_, Arc<lr_providers::registry::ProviderRegistry>>,
) -> Result<Vec<ProviderFreeTierStatus>, String> {
    let config = config_manager.get();
    let mut statuses = Vec::new();

    for provider in &config.providers {
        if !provider.enabled {
            continue;
        }

        let is_user_override = provider.free_tier.is_some();
        let effective_free_tier = provider_free_tier(provider, &provider_registry);

        let mut status = ProviderFreeTierStatus {
            provider_instance: provider.name.clone(),
            provider_type: provider.provider_type.factory_id().to_string(),
            display_name: provider.name.clone(),
            free_tier: effective_free_tier.clone(),
            is_user_override,
            supports_credit_check: false,
            rate_rpm_used: None,
            rate_rpm_limit: None,
            rate_rpd_used: None,
            rate_rpd_limit: None,
            rate_tpm_used: None,
            rate_tpm_limit: None,
            rate_tpd_used: None,
            rate_tpd_limit: None,
            rate_monthly_calls_used: None,
            rate_monthly_calls_limit: None,
            rate_monthly_tokens_used: None,
            rate_monthly_tokens_limit: None,
            credit_used_usd: None,
            credit_budget_usd: None,
            credit_remaining_usd: None,
            is_backed_off: false,
            backoff_retry_after_secs: None,
            backoff_reason: None,
            cost_backoff_active: false,
            cost_backoff_retry_after_secs: None,
            cost_backoff_duration_secs: None,
            cost_backoff_last_trigger: None,
            cost_backoff_trigger_count: 0,
            has_capacity: true,
            status_message: "Available".to_string(),
        };

        // Fill in rate limit info
        match &effective_free_tier {
            FreeTierKind::RateLimitedFree {
                max_rpm,
                max_rpd,
                max_tpm,
                max_tpd,
                max_monthly_calls,
                max_monthly_tokens,
            } => {
                if let Some(tracker) = free_tier_manager.get_rate_tracker(&provider.name) {
                    if *max_rpm > 0 {
                        status.rate_rpm_used = Some(tracker.minute_requests);
                        status.rate_rpm_limit = Some(*max_rpm);
                    }
                    if *max_rpd > 0 {
                        status.rate_rpd_used = Some(tracker.daily_requests);
                        status.rate_rpd_limit = Some(*max_rpd);
                    }
                    if *max_tpm > 0 {
                        status.rate_tpm_used = Some(tracker.minute_tokens);
                        status.rate_tpm_limit = Some(*max_tpm);
                    }
                    if *max_tpd > 0 {
                        status.rate_tpd_used = Some(tracker.daily_tokens);
                        status.rate_tpd_limit = Some(*max_tpd);
                    }
                    if *max_monthly_calls > 0 {
                        status.rate_monthly_calls_used = Some(tracker.monthly_requests);
                        status.rate_monthly_calls_limit = Some(*max_monthly_calls);
                    }
                    if *max_monthly_tokens > 0 {
                        status.rate_monthly_tokens_used = Some(tracker.monthly_tokens);
                        status.rate_monthly_tokens_limit = Some(*max_monthly_tokens);
                    }
                }
                let cap = free_tier_manager
                    .check_rate_limit_capacity(&provider.name, &effective_free_tier);
                status.has_capacity = cap.has_capacity;
                status.status_message = cap.status_message;
            }
            FreeTierKind::CreditBased {
                budget_usd,
                detection,
                ..
            } => {
                if matches!(detection, lr_config::CreditDetection::ProviderApi) {
                    status.supports_credit_check = true;
                }
                if let Some(tracker) = free_tier_manager.get_credit_tracker(&provider.name) {
                    status.credit_used_usd = Some(tracker.current_cost_usd);
                    status.credit_remaining_usd = tracker.api_remaining_usd;
                }
                status.credit_budget_usd = Some(*budget_usd);
                let cap =
                    free_tier_manager.check_credit_balance(&provider.name, &effective_free_tier);
                status.has_capacity = cap.has_capacity;
                status.status_message = cap.status_message;
                if let Some(remaining) = cap.remaining_usd {
                    status.credit_remaining_usd = Some(remaining);
                }
            }
            FreeTierKind::AlwaysFreeLocal | FreeTierKind::Subscription => {
                status.has_capacity = true;
                status.status_message = "Always free".to_string();
            }
            FreeTierKind::FreeModelsOnly { max_rpm, .. } => {
                if *max_rpm > 0 {
                    if let Some(tracker) = free_tier_manager.get_rate_tracker(&provider.name) {
                        status.rate_rpm_used = Some(tracker.minute_requests);
                        status.rate_rpm_limit = Some(*max_rpm);
                        if tracker.minute_requests >= *max_rpm {
                            status.has_capacity = false;
                            status.status_message = format!(
                                "RPM limit reached: {}/{}",
                                tracker.minute_requests, max_rpm
                            );
                        } else {
                            status.has_capacity = true;
                            status.status_message = format!(
                                "Free models available ({}/{} RPM used)",
                                tracker.minute_requests, max_rpm
                            );
                        }
                    } else {
                        status.has_capacity = true;
                        status.status_message = "Free models available".to_string();
                    }
                } else {
                    status.has_capacity = true;
                    status.status_message = "Free models available".to_string();
                }
            }
            FreeTierKind::None => {
                status.has_capacity = false;
                status.status_message = "No free tier".to_string();
            }
        }

        // Check backoff
        if let Some(backoff) = free_tier_manager.get_provider_backoff_info(&provider.name) {
            status.is_backed_off = true;
            status.backoff_retry_after_secs = Some(backoff.retry_after_secs);
            status.backoff_reason = Some(backoff.reason);
        }

        // Check cost backoff
        let cost_status = free_tier_manager.get_cost_backoff_status(&provider.name);
        status.cost_backoff_active = cost_status.in_backoff;
        status.cost_backoff_retry_after_secs = cost_status.retry_after_secs;
        status.cost_backoff_duration_secs = if cost_status.backoff_secs > 0 {
            Some(cost_status.backoff_secs)
        } else {
            None
        };
        status.cost_backoff_last_trigger = cost_status.last_trigger.map(|t| t.to_rfc3339());
        status.cost_backoff_trigger_count = cost_status.trigger_count;

        statuses.push(status);
    }

    Ok(statuses)
}

/// The free tier a provider has: its own setting, else its type's default
/// for this instance.
pub(crate) fn provider_free_tier(
    provider: &lr_config::ProviderConfig,
    registry: &lr_providers::registry::ProviderRegistry,
) -> FreeTierKind {
    provider.free_tier.clone().unwrap_or_else(|| {
        registry.default_free_tier_for_instance(&provider.name, &provider.provider_type)
    })
}

/// The free tier of a provider instance by name (`None` when it is not in
/// the config).
pub(crate) fn instance_free_tier(
    config: &lr_config::AppConfig,
    registry: &lr_providers::registry::ProviderRegistry,
    instance_name: &str,
) -> Option<FreeTierKind> {
    config
        .providers
        .iter()
        .find(|p| p.name == instance_name)
        .map(|p| provider_free_tier(p, registry))
}

/// Rule problems as one message, e.g. "Rule 2 (pattern): Invalid regular
/// expression ...".
fn describe_rule_errors(errors: &[lr_providers::free_models::RuleError]) -> String {
    errors
        .iter()
        .map(|e| {
            let at = match e.rule_index {
                Some(i) => format!("Rule {}", i + 1),
                None if e.field == "excluded_models" => "Never-free list".to_string(),
                None => "Always-free list".to_string(),
            };
            format!("{at} ({}): {}", e.field, e.message)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Update free tier config for a provider (user override)
#[tauri::command]
pub async fn set_provider_free_tier(
    provider_instance: String,
    free_tier: Option<FreeTierKind>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Setting free tier for provider '{}': {:?}",
        provider_instance,
        free_tier.is_some()
    );

    if let Some(kind) = &free_tier {
        let errors = lr_providers::free_models::validate(kind);
        if !errors.is_empty() {
            return Err(describe_rule_errors(&errors));
        }
    }

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(provider) = cfg
                .providers
                .iter_mut()
                .find(|p| p.name == provider_instance)
            {
                provider.free_tier = free_tier.clone();
                found = true;
            }
        })
        .map_err(|e| e.to_string())?;

    if !found {
        return Err(format!("Provider not found: {}", provider_instance));
    }

    config_manager.save().await.map_err(|e| e.to_string())?;

    if let Err(e) = app.emit("config-changed", &config_manager.get()) {
        tracing::error!("Failed to emit config-changed event: {}", e);
    }
    // Per-model free badges follow the new rules.
    let _ = app.emit("models-changed", ());

    Ok(())
}

/// One model of a provider in the free-model rules preview.
#[derive(Debug, Clone, Serialize)]
pub struct FreeModelPreviewRow {
    pub model_id: String,
    pub name: String,
    /// Offered (not hidden on the Models tab)
    pub enabled: bool,
    /// The provider's listing has an entry for the model (rules on listing
    /// fields can see it)
    pub has_metadata: bool,
    /// Whether the model is free under the previewed rules, and why
    pub free: lr_providers::free_models::FreeModelDecision,
}

/// Which of a provider's models a free tier (usually unsaved rules) makes
/// free.
#[derive(Debug, Clone, Serialize)]
pub struct FreeModelRulesPreview {
    pub models: Vec<FreeModelPreviewRow>,
    /// Problems with the rules; a rule with an error never makes a model
    /// free
    pub errors: Vec<lr_providers::free_models::RuleError>,
    /// Per rule: how many models its condition matches (regardless of order
    /// or of being enabled)
    pub rule_matches: Vec<usize>,
    pub free_count: usize,
    /// Why listing the models failed, when it did
    pub discovery_error: Option<String>,
    /// The provider keeps listing entries (rules on fields, JSONPath and
    /// listed prices can work)
    pub metadata_available: bool,
}

/// Evaluate a free tier against a provider's models without saving it.
#[tauri::command]
pub async fn preview_free_model_rules(
    instance_name: String,
    free_tier: FreeTierKind,
    config_manager: State<'_, ConfigManager>,
    provider_registry: State<'_, Arc<lr_providers::registry::ProviderRegistry>>,
) -> Result<FreeModelRulesPreview, String> {
    free_model_rules_preview(
        &provider_registry,
        &config_manager.get(),
        &instance_name,
        &free_tier,
    )
    .await
}

pub(crate) async fn free_model_rules_preview(
    provider_registry: &lr_providers::registry::ProviderRegistry,
    config: &lr_config::AppConfig,
    instance_name: &str,
    free_tier: &FreeTierKind,
) -> Result<FreeModelRulesPreview, String> {
    use lr_providers::free_models::{FreeModelInput, FreeTierEvaluator};
    use lr_providers::model_settings::ModelSource;

    // Fills the cache when needed; a failed listing shows in the result.
    let _ = provider_registry
        .list_provider_models_cached(instance_name)
        .await;
    let managed = provider_registry
        .managed_models(instance_name)
        .ok_or_else(|| format!("Provider '{instance_name}' not found"))?;
    let metadata = provider_registry
        .listing_metadata(instance_name)
        .unwrap_or_default();
    let evaluator = FreeTierEvaluator::new(free_tier);
    let mut rule_matches = vec![0; evaluator.rule_count()];
    let mut models = Vec::new();
    for m in managed
        .models
        .into_iter()
        .filter(|m| m.source != ModelSource::Missing)
    {
        let price_override = provider_registry.effective_pricing_override(
            instance_name,
            &m.info.id,
            &config.pricing_overrides,
        );
        let input = FreeModelInput {
            model_id: &m.info.id,
            metadata: metadata.get(&m.info.id),
            price_override: price_override.as_ref(),
        };
        for (i, count) in rule_matches.iter_mut().enumerate() {
            if evaluator.rule_matches(i, &input) {
                *count += 1;
            }
        }
        let free = evaluator.decide(&input);
        models.push(FreeModelPreviewRow {
            has_metadata: input.metadata.is_some(),
            model_id: m.info.id.clone(),
            name: m.info.name.clone(),
            enabled: m.enabled,
            free,
        });
    }
    Ok(FreeModelRulesPreview {
        free_count: models.iter().filter(|m| m.free.is_free).count(),
        models,
        errors: evaluator.errors().to_vec(),
        rule_matches,
        discovery_error: managed.discovery_error,
        metadata_available: !metadata.is_empty(),
    })
}

/// A model's entry in its provider's model listing (raw JSON), for writing
/// rules on its fields. `None` when the provider keeps no entry for it.
#[tauri::command]
pub async fn get_provider_model_metadata(
    instance_name: String,
    model_id: String,
    provider_registry: State<'_, Arc<lr_providers::registry::ProviderRegistry>>,
) -> Result<Option<serde_json::Value>, String> {
    if !provider_registry.has_cached_models(&instance_name) {
        let _ = provider_registry
            .list_provider_models_cached(&instance_name)
            .await;
    }
    Ok(provider_registry.model_metadata(&instance_name, &model_id))
}

/// Reset free tier usage counters for a provider
#[tauri::command]
pub async fn reset_provider_free_tier_usage(
    provider_instance: String,
    free_tier_manager: State<'_, Arc<FreeTierManager>>,
) -> Result<(), String> {
    tracing::info!(
        "Resetting free tier usage for provider '{}'",
        provider_instance
    );
    free_tier_manager.reset_usage(&provider_instance);
    if let Err(e) = free_tier_manager.persist() {
        tracing::error!("Failed to persist free tier state: {}", e);
    }
    Ok(())
}

/// Manually set free tier usage for a provider (from UI)
#[tauri::command]
pub async fn set_provider_free_tier_usage(
    provider_instance: String,
    credit_used_usd: Option<f64>,
    credit_remaining_usd: Option<f64>,
    daily_requests: Option<u32>,
    monthly_requests: Option<u32>,
    monthly_tokens: Option<u64>,
    free_tier_manager: State<'_, Arc<FreeTierManager>>,
) -> Result<(), String> {
    tracing::info!(
        "Setting free tier usage for provider '{}': credit_used={:?}, credit_remaining={:?}, daily_req={:?}, monthly_req={:?}, monthly_tok={:?}",
        provider_instance, credit_used_usd, credit_remaining_usd, daily_requests, monthly_requests, monthly_tokens
    );

    if credit_used_usd.is_some() || credit_remaining_usd.is_some() {
        free_tier_manager.set_credit_usage(
            &provider_instance,
            credit_used_usd,
            credit_remaining_usd,
        );
    }

    if daily_requests.is_some() || monthly_requests.is_some() || monthly_tokens.is_some() {
        free_tier_manager.set_rate_limit_usage(
            &provider_instance,
            daily_requests,
            monthly_requests,
            monthly_tokens,
        );
    }

    if let Err(e) = free_tier_manager.persist() {
        tracing::error!("Failed to persist free tier state: {}", e);
    }

    Ok(())
}

/// Reset cost backoff for a provider (clears the cost watchdog flag)
#[tauri::command]
pub async fn reset_cost_backoff(
    provider_instance: String,
    free_tier_manager: State<'_, Arc<FreeTierManager>>,
) -> Result<(), String> {
    tracing::info!(
        "Resetting cost backoff for provider '{}'",
        provider_instance
    );
    free_tier_manager.reset_cost_backoff(&provider_instance);
    if let Err(e) = free_tier_manager.persist() {
        tracing::error!("Failed to persist free tier state: {}", e);
    }
    Ok(())
}

/// Get the default free tier config for a provider type
#[tauri::command]
pub async fn get_default_free_tier(
    provider_type: String,
    provider_registry: State<'_, Arc<lr_providers::registry::ProviderRegistry>>,
) -> Result<FreeTierKind, String> {
    if let Some(factory) = provider_registry.get_factory(&provider_type) {
        Ok(factory.default_free_tier())
    } else {
        Ok(FreeTierKind::None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_config::{FieldOp, FreeModelCondition, FreeModelRule, FreeModelRuleEffect, PriceSource};
    use lr_providers::free_models::DecidedBy;
    use lr_providers::registry::ProviderRegistry;
    use std::collections::HashMap;

    async fn registry_with_listing(body: &str) -> (ProviderRegistry, wiremock::MockServer) {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body.to_string()))
            .mount(&server)
            .await;
        let registry = ProviderRegistry::new();
        registry.register_factory(Arc::new(
            lr_providers::factory::OpenAICompatibleProviderFactory,
        ));
        registry
            .create_provider(
                "Free".into(),
                "openai_compatible".into(),
                HashMap::from([("base_url".to_string(), server.uri())]),
            )
            .await
            .unwrap();
        (registry, server)
    }

    fn rule(effect: FreeModelRuleEffect, when: FreeModelCondition) -> FreeModelRule {
        FreeModelRule {
            enabled: true,
            effect,
            when,
            label: None,
        }
    }

    #[tokio::test]
    async fn preview_explains_each_model_and_counts_rule_matches() {
        let (registry, _server) = registry_with_listing(
            r#"{"models":[
                {"id":"llama-free","tier":"free","pricing":{"input":0,"output":0}},
                {"id":"llama-pro","tier":"pro","pricing":{"input":"0.000001","output":"0.000002"}},
                {"id":"beta-free","tier":"free"}
            ]}"#,
        )
        .await;
        let kind = FreeTierKind::FreeModelsOnly {
            free_model_patterns: vec![],
            excluded_models: vec!["beta-*".into()],
            rules: vec![
                rule(
                    FreeModelRuleEffect::Include,
                    FreeModelCondition::ZeroPrice {
                        sources: vec![PriceSource::ProviderListing],
                        input_pointer: None,
                        output_pointer: None,
                    },
                ),
                rule(
                    FreeModelRuleEffect::Include,
                    FreeModelCondition::Field {
                        pointer: "/tier".into(),
                        op: FieldOp::Equals,
                        value: serde_json::json!("free"),
                    },
                ),
            ],
            max_rpm: 0,
        };
        let preview =
            free_model_rules_preview(&registry, &lr_config::AppConfig::default(), "Free", &kind)
                .await
                .unwrap();
        assert!(preview.errors.is_empty());
        assert!(preview.metadata_available);
        assert_eq!(preview.free_count, 1);
        assert_eq!(preview.rule_matches, vec![1, 2]);
        let row = |id: &str| preview.models.iter().find(|m| m.model_id == id).unwrap();
        assert_eq!(row("llama-free").free.rule_index, Some(0));
        assert!(!row("llama-pro").free.is_free);
        assert_eq!(row("llama-pro").free.decided_by, DecidedBy::NoRule);
        assert_eq!(row("beta-free").free.decided_by, DecidedBy::ExcludedList);
        assert!(row("beta-free").has_metadata);
    }

    #[tokio::test]
    async fn preview_reports_rule_errors() {
        let (registry, _server) = registry_with_listing(r#"{"data":[{"id":"a"}]}"#).await;
        let kind = FreeTierKind::FreeModelsOnly {
            free_model_patterns: vec![],
            excluded_models: vec![],
            rules: vec![rule(
                FreeModelRuleEffect::Include,
                FreeModelCondition::JsonPath {
                    expression: "@.tier ==".into(),
                },
            )],
            max_rpm: 0,
        };
        let preview =
            free_model_rules_preview(&registry, &lr_config::AppConfig::default(), "Free", &kind)
                .await
                .unwrap();
        assert_eq!(preview.errors.len(), 1);
        assert_eq!(preview.errors[0].rule_index, Some(0));
        assert_eq!(preview.free_count, 0);
        assert!(describe_rule_errors(&preview.errors).starts_with("Rule 1 (expression): "));
        assert!(free_model_rules_preview(
            &registry,
            &lr_config::AppConfig::default(),
            "nope",
            &kind
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn remote_custom_providers_are_not_free_by_default() {
        let (registry, _server) = registry_with_listing(r#"{"data":[]}"#).await;
        let mut config = lr_config::AppConfig::default();
        config.providers.push(lr_config::ProviderConfig {
            name: "Free".into(),
            provider_type: lr_config::ProviderType::Custom,
            ..lr_config::ProviderConfig::default_ollama()
        });
        // The mock server is on 127.0.0.1: a local server, always free.
        assert_eq!(
            instance_free_tier(&config, &registry, "Free"),
            Some(FreeTierKind::AlwaysFreeLocal)
        );
        assert_eq!(instance_free_tier(&config, &registry, "missing"), None);
    }
}
