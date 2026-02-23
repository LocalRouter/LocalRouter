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
    pub rate_monthly_calls_used: Option<u32>,
    pub rate_monthly_calls_limit: Option<u32>,
    // Credit-based status
    pub credit_used_usd: Option<f64>,
    pub credit_budget_usd: Option<f64>,
    pub credit_remaining_usd: Option<f64>,
    // Backoff status
    pub is_backed_off: bool,
    pub backoff_retry_after_secs: Option<u64>,
    pub backoff_reason: Option<String>,
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
        let effective_free_tier = provider.free_tier.clone().unwrap_or_else(|| {
            provider_registry.get_factory_default_free_tier(&provider.provider_type)
        });

        let mut status = ProviderFreeTierStatus {
            provider_instance: provider.name.clone(),
            provider_type: serde_json::to_string(&provider.provider_type)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string(),
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
            rate_monthly_calls_used: None,
            rate_monthly_calls_limit: None,
            credit_used_usd: None,
            credit_budget_usd: None,
            credit_remaining_usd: None,
            is_backed_off: false,
            backoff_retry_after_secs: None,
            backoff_reason: None,
            has_capacity: true,
            status_message: "Available".to_string(),
        };

        // Fill in rate limit info
        match &effective_free_tier {
            FreeTierKind::RateLimitedFree {
                max_rpm,
                max_rpd,
                max_tpm,
                max_monthly_calls,
                ..
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
                    if *max_monthly_calls > 0 {
                        status.rate_monthly_calls_used = Some(tracker.monthly_requests);
                        status.rate_monthly_calls_limit = Some(*max_monthly_calls);
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
            FreeTierKind::FreeModelsOnly { .. } => {
                status.has_capacity = true;
                status.status_message = "Free models available".to_string();
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

        statuses.push(status);
    }

    Ok(statuses)
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

    Ok(())
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
