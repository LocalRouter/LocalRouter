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
