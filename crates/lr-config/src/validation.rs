//! Configuration validation

#![allow(deprecated)]

use super::{
    AppConfig, McpAuthConfig, McpServerConfig, McpTransportConfig, ProviderConfig,
    CLIENT_KEYRING_SERVICE,
};
use lr_api_keys::keychain_trait::KeychainStorage;
use lr_api_keys::CachedKeychain;
use lr_types::{AppError, AppResult};
use std::collections::HashSet;

/// LocalRouter client API keys start with this prefix
const LOCALROUTER_KEY_PREFIX: &str = "lr-";
/// LocalRouter client API keys are exactly this length
const LOCALROUTER_KEY_LENGTH: usize = 46;

/// Quick check if an API key looks like a LocalRouter client key format.
/// Used as a fast filter before checking against actual keychain secrets.
fn looks_like_localrouter_key(api_key: &str) -> bool {
    api_key.starts_with(LOCALROUTER_KEY_PREFIX) && api_key.len() == LOCALROUTER_KEY_LENGTH
}

/// Validate the entire configuration
pub fn validate_config(config: &AppConfig) -> AppResult<()> {
    // Validate server configuration
    validate_server_config(config)?;

    // Validate providers
    validate_providers(&config.providers)?;

    // Validate providers are not self-referential (pointing back to LocalRouter)
    validate_providers_not_self_referential(config)?;

    // Validate strategies
    validate_strategies(config)?;

    // Validate cross-references
    validate_cross_references(config)?;

    // Validate client strategy references
    validate_client_strategy_refs(config)?;

    // Validate health check bounds
    validate_health_check_config(config)?;

    // Validate guardrails bounds
    validate_guardrails_config(config)?;

    // Validate MCP server configurations
    validate_mcp_servers(&config.mcp_servers)?;

    Ok(())
}

/// Validate health check configuration bounds
fn validate_health_check_config(config: &AppConfig) -> AppResult<()> {
    if config.health_check.timeout_secs < 1 || config.health_check.timeout_secs > 300 {
        return Err(AppError::Config(
            "Health check timeout must be between 1 and 300 seconds".to_string(),
        ));
    }

    if config.health_check.interval_secs < 1 {
        return Err(AppError::Config(
            "Health check interval must be at least 1 second".to_string(),
        ));
    }

    Ok(())
}

/// Validate guardrails configuration bounds
fn validate_guardrails_config(config: &AppConfig) -> AppResult<()> {
    let g = &config.guardrails;

    if !(0.0..=1.0).contains(&g.default_confidence_threshold) {
        return Err(AppError::Config(format!(
            "Guardrails default_confidence_threshold must be between 0.0 and 1.0, got {}",
            g.default_confidence_threshold
        )));
    }

    Ok(())
}

/// Validate server configuration
fn validate_server_config(config: &AppConfig) -> AppResult<()> {
    let server = &config.server;

    // Validate host is not empty
    if server.host.is_empty() {
        return Err(AppError::Config("Server host cannot be empty".to_string()));
    }

    // Validate port is in valid range (1-65535)
    if server.port == 0 {
        return Err(AppError::Config(
            "Server port must be greater than 0".to_string(),
        ));
    }

    Ok(())
}

/// Validate providers
fn validate_providers(providers: &[ProviderConfig]) -> AppResult<()> {
    // Empty providers list is allowed - user may want to start fresh
    // and add providers later through the UI

    let mut names = HashSet::new();
    for provider in providers {
        // Validate name is not empty or whitespace-only (check before duplicate check)
        if provider.name.trim().is_empty() {
            return Err(AppError::Config(
                "Provider name cannot be empty".to_string(),
            ));
        }

        // Check for duplicate provider names
        if !names.insert(&provider.name) {
            return Err(AppError::Config(format!(
                "Duplicate provider name: {}",
                provider.name
            )));
        }

        // Validate provider_config format if present
        // Note: Each provider is responsible for validating its own configuration structure
        // We only do basic checks here to ensure the JSON is valid (already validated by serde)
        if let Some(config) = &provider.provider_config {
            // Check that it's an object (not a primitive)
            if !config.is_object() {
                return Err(AppError::Config(format!(
                    "Provider '{}' config must be a JSON object, not a primitive value",
                    provider.name
                )));
            }
        }
    }

    Ok(())
}

/// Validate that no provider is configured with a LocalRouter client API key.
/// This prevents accidental self-referential configurations that would cause request loops.
fn validate_providers_not_self_referential(config: &AppConfig) -> AppResult<()> {
    // Collect provider API keys that look like LocalRouter keys (fast filter)
    let suspect_keys: Vec<(&str, &str)> = config
        .providers
        .iter()
        .filter_map(|provider| {
            let api_key = provider
                .provider_config
                .as_ref()
                .and_then(|c| c.get("api_key"))
                .and_then(|v| v.as_str())?;

            if looks_like_localrouter_key(api_key) {
                Some((provider.name.as_str(), api_key))
            } else {
                None
            }
        })
        .collect();

    // If no suspect keys, we're done
    if suspect_keys.is_empty() {
        return Ok(());
    }

    // Get keychain to check actual client secrets
    let keychain = match CachedKeychain::auto() {
        Ok(k) => k,
        Err(e) => {
            // If we can't access keychain, log warning but don't fail validation
            tracing::warn!(
                "Could not access keychain to validate provider API keys: {}",
                e
            );
            return Ok(());
        }
    };

    // Collect actual client secrets from keychain
    let client_secrets: HashSet<String> = config
        .clients
        .iter()
        .filter_map(|client| keychain.get(CLIENT_KEYRING_SERVICE, &client.id).ok().flatten())
        .collect();

    // Check if any suspect provider key matches an actual client secret
    for (provider_name, api_key) in suspect_keys {
        if client_secrets.contains(api_key) {
            return Err(AppError::Config(format!(
                "Provider '{}' is configured with a LocalRouter client API key. \
                 This would create a request loop. Use an external provider's API key instead.",
                provider_name
            )));
        }
    }

    Ok(())
}

/// Validate strategies
fn validate_strategies(config: &AppConfig) -> AppResult<()> {
    // Check for duplicate strategy IDs
    let mut ids = HashSet::new();
    for strategy in &config.strategies {
        if !ids.insert(&strategy.id) {
            return Err(AppError::Config(format!(
                "Duplicate strategy ID: {}",
                strategy.id
            )));
        }

        // Validate ID is not empty or whitespace-only
        if strategy.id.trim().is_empty() {
            return Err(AppError::Config("Strategy ID cannot be empty".to_string()));
        }

        // Validate name is not empty or whitespace-only
        if strategy.name.trim().is_empty() {
            return Err(AppError::Config(
                "Strategy name cannot be empty".to_string(),
            ));
        }

        // Check parent references point to existing clients
        if let Some(parent_id) = &strategy.parent {
            if !config.clients.iter().any(|c| c.id == *parent_id) {
                // Auto-clear orphaned parent references instead of failing
                // This is handled during load, but we log a warning
                tracing::warn!(
                    "Strategy '{}' references non-existent parent client '{}' - will be auto-cleared",
                    strategy.name, parent_id
                );
            }
        }

        // Validate rate limits
        for limit in &strategy.rate_limits {
            if limit.value <= 0.0 || !limit.value.is_finite() {
                return Err(AppError::Config(format!(
                    "Strategy '{}' has invalid rate limit value: {}",
                    strategy.name, limit.value
                )));
            }
        }

        // Validate auto config if present
        if let Some(auto_config) = &strategy.auto_config {
            // Allow empty prioritized_models - router will handle error at runtime

            // Check no overlap between prioritized and available
            for model in &auto_config.prioritized_models {
                if auto_config.available_models.contains(model) {
                    return Err(AppError::Config(format!(
                        "Strategy '{}' has model {:?} in both prioritized and available lists",
                        strategy.name, model
                    )));
                }
            }
        }
    }

    Ok(())
}

/// Validate client strategy references
fn validate_client_strategy_refs(config: &AppConfig) -> AppResult<()> {
    // Check all client.strategy_id references exist
    for client in &config.clients {
        if !config.strategies.iter().any(|s| s.id == client.strategy_id) {
            return Err(AppError::Config(format!(
                "Client '{}' references non-existent strategy '{}'",
                client.name, client.strategy_id
            )));
        }
    }
    Ok(())
}

/// Validate MCP server configurations
fn validate_mcp_servers(servers: &[McpServerConfig]) -> AppResult<()> {
    for server in servers {
        if server.id.trim().is_empty() {
            return Err(AppError::Config(
                "MCP server ID cannot be empty".to_string(),
            ));
        }

        match &server.transport_config {
            McpTransportConfig::HttpSse { url, .. }
            | McpTransportConfig::Sse { url, .. }
            | McpTransportConfig::WebSocket { url, .. } => {
                // Validate URL is not empty
                let trimmed = url.trim();
                if trimmed.is_empty() {
                    return Err(AppError::Config(format!(
                        "MCP server '{}': URL cannot be empty",
                        server.id
                    )));
                }

                // Validate URL scheme is http or https (or ws/wss for WebSocket)
                let allowed_schemes: &[&str] = if matches!(
                    &server.transport_config,
                    McpTransportConfig::WebSocket { .. }
                ) {
                    &["http://", "https://", "ws://", "wss://"]
                } else {
                    &["http://", "https://"]
                };

                let has_valid_scheme = allowed_schemes
                    .iter()
                    .any(|scheme| trimmed.to_lowercase().starts_with(scheme));

                if !has_valid_scheme {
                    return Err(AppError::Config(format!(
                        "MCP server '{}': URL must start with {} (got '{}')",
                        server.id,
                        allowed_schemes.join(" or "),
                        trimmed
                    )));
                }
            }
            McpTransportConfig::Stdio { command, .. } => {
                if command.trim().is_empty() {
                    return Err(AppError::Config(format!(
                        "MCP server '{}': command cannot be empty",
                        server.id
                    )));
                }
            }
        }

        // Validate OAuth redirect_uri if present
        if let Some(McpAuthConfig::OAuthBrowser { redirect_uri, .. }) = &server.auth_config {
            let uri_lower = redirect_uri.to_lowercase();
            if !uri_lower.starts_with("http://localhost")
                && !uri_lower.starts_with("http://127.0.0.1")
                && !uri_lower.starts_with("http://[::1]")
            {
                return Err(AppError::Config(format!(
                    "MCP server '{}': OAuth redirect_uri must use localhost (http://localhost or http://127.0.0.1), got '{}'",
                    server.id, redirect_uri
                )));
            }
        }
    }
    Ok(())
}

/// Validate cross-references between configuration objects
fn validate_cross_references(config: &AppConfig) -> AppResult<()> {
    // Build set of provider names
    let provider_names: HashSet<&str> = config.providers.iter().map(|p| p.name.as_str()).collect();

    // Validate strategy allowed_models reference valid providers
    for strategy in &config.strategies {
        for provider in &strategy.allowed_models.selected_providers {
            if !provider_names.contains(provider.as_str()) {
                tracing::warn!(
                    "Strategy '{}' references provider '{}' which is not configured - model availability may be limited",
                    strategy.name, provider
                );
            }
        }
        for (provider, _model) in &strategy.allowed_models.selected_models {
            if !provider_names.contains(provider.as_str()) {
                tracing::warn!(
                    "Strategy '{}' references provider '{}' which is not configured - model may not be accessible",
                    strategy.name, provider
                );
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AvailableModelsSelection, ProviderType, RateLimitTimeWindow, RateLimitType, Strategy,
        StrategyRateLimit,
    };

    #[test]
    fn test_validate_default_config() {
        let config = AppConfig::default();
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_validate_empty_server_host() {
        let mut config = AppConfig::default();
        config.server.host = String::new();
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_zero_port() {
        let mut config = AppConfig::default();
        config.server.port = 0;
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_duplicate_provider_names() {
        let provider = ProviderConfig {
            name: "Ollama".to_string(),
            provider_type: ProviderType::Ollama,
            enabled: true,
            provider_config: None,
            api_key_ref: None,
            free_tier: None,
        };
        let config = AppConfig {
            providers: vec![provider.clone(), provider],
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_empty_provider_name() {
        let config = AppConfig {
            providers: vec![ProviderConfig {
                name: "".to_string(),
                provider_type: ProviderType::Ollama,
                enabled: true,
                provider_config: None,
                api_key_ref: None,
                free_tier: None,
            }],
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_whitespace_provider_name() {
        let config = AppConfig {
            providers: vec![ProviderConfig {
                name: "   ".to_string(),
                provider_type: ProviderType::Ollama,
                enabled: true,
                provider_config: None,
                api_key_ref: None,
                free_tier: None,
            }],
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    fn make_strategy(name: &str) -> Strategy {
        Strategy {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            parent: None,
            allowed_models: AvailableModelsSelection::all(),
            auto_config: None,
            rate_limits: vec![],
            free_tier_only: false,
            free_tier_fallback: crate::FreeTierFallback::default(),
        }
    }

    #[test]
    fn test_validate_duplicate_strategy_ids() {
        let s1 = make_strategy("Strategy A");
        let mut s2 = make_strategy("Strategy B");
        s2.id = s1.id.clone(); // same ID
        let config = AppConfig {
            strategies: vec![s1, s2],
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_empty_strategy_name() {
        let mut s = make_strategy("");
        s.name = "".to_string();
        let config = AppConfig {
            strategies: vec![s],
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_whitespace_strategy_name() {
        let mut s = make_strategy("  ");
        s.name = "  ".to_string();
        let config = AppConfig {
            strategies: vec![s],
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_rate_limit_nan() {
        let mut s = make_strategy("Test");
        s.rate_limits = vec![StrategyRateLimit {
            limit_type: RateLimitType::Requests,
            value: f64::NAN,
            time_window: RateLimitTimeWindow::Minute,
            enabled: true,
        }];
        let config = AppConfig {
            strategies: vec![s],
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_rate_limit_infinity() {
        let mut s = make_strategy("Test");
        s.rate_limits = vec![StrategyRateLimit {
            limit_type: RateLimitType::Requests,
            value: f64::INFINITY,
            time_window: RateLimitTimeWindow::Minute,
            enabled: true,
        }];
        let config = AppConfig {
            strategies: vec![s],
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_rate_limit_negative() {
        let mut s = make_strategy("Test");
        s.rate_limits = vec![StrategyRateLimit {
            limit_type: RateLimitType::Requests,
            value: -1.0,
            time_window: RateLimitTimeWindow::Minute,
            enabled: true,
        }];
        let config = AppConfig {
            strategies: vec![s],
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_rate_limit_zero() {
        let mut s = make_strategy("Test");
        s.rate_limits = vec![StrategyRateLimit {
            limit_type: RateLimitType::Requests,
            value: 0.0,
            time_window: RateLimitTimeWindow::Minute,
            enabled: true,
        }];
        let config = AppConfig {
            strategies: vec![s],
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_rate_limit_valid() {
        let mut s = make_strategy("Test");
        s.rate_limits = vec![StrategyRateLimit {
            limit_type: RateLimitType::Requests,
            value: 100.0,
            time_window: RateLimitTimeWindow::Minute,
            enabled: true,
        }];
        let config = AppConfig {
            strategies: vec![s],
            ..Default::default()
        };
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_validate_provider_config_not_object() {
        let config = AppConfig {
            providers: vec![ProviderConfig {
                name: "Test".to_string(),
                provider_type: ProviderType::Ollama,
                enabled: true,
                provider_config: Some(serde_json::json!("not an object")),
                api_key_ref: None,
                free_tier: None,
            }],
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_client_strategy_ref() {
        let mut client =
            crate::Client::new_with_strategy("Test".to_string(), "nonexistent".to_string());
        client.strategy_id = "nonexistent".to_string();
        let config = AppConfig {
            clients: vec![client],
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_empty_name_before_duplicate_check() {
        // BUG 8: Two empty-name providers should both report "empty name", not "duplicate"
        let config = AppConfig {
            providers: vec![
                ProviderConfig {
                    name: "".to_string(),
                    provider_type: ProviderType::Ollama,
                    enabled: true,
                    provider_config: None,
                    api_key_ref: None,
                    free_tier: None,
                },
                ProviderConfig {
                    name: "".to_string(),
                    provider_type: ProviderType::Ollama,
                    enabled: true,
                    provider_config: None,
                    api_key_ref: None,
                    free_tier: None,
                },
            ],
            ..Default::default()
        };
        let err = validate_config(&config).unwrap_err();
        let msg = err.to_string();
        // The first empty name should be caught as "empty", not "duplicate"
        assert!(
            msg.contains("cannot be empty"),
            "Expected 'cannot be empty' error, got: {}",
            msg
        );
    }

    #[test]
    fn test_validate_guardrails_confidence_threshold_too_low() {
        let mut config = AppConfig::default();
        config.guardrails.default_confidence_threshold = -0.1;
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_guardrails_confidence_threshold_too_high() {
        let mut config = AppConfig::default();
        config.guardrails.default_confidence_threshold = 1.1;
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_guardrails_confidence_threshold_valid_bounds() {
        let mut config = AppConfig::default();
        config.guardrails.default_confidence_threshold = 0.0;
        assert!(validate_config(&config).is_ok());

        config.guardrails.default_confidence_threshold = 1.0;
        assert!(validate_config(&config).is_ok());
    }
}

#[cfg(test)]
mod self_referential_tests {
    use super::*;
    use crate::ProviderType;

    #[test]
    fn test_looks_like_localrouter_key_valid() {
        // Valid LocalRouter key format (lr- prefix + 43 chars = 46 total)
        assert!(looks_like_localrouter_key(
            "lr-8xIF-tmewuD4eOm1dxHKRjiCAD57nLAGRLEJISS1K6E"
        ));
    }

    #[test]
    fn test_looks_like_localrouter_key_too_short() {
        assert!(!looks_like_localrouter_key("lr-short"));
    }

    #[test]
    fn test_looks_like_localrouter_key_too_long() {
        assert!(!looks_like_localrouter_key(
            "lr-8xIF-tmewuD4eOm1dxHKRjiCAD57nLAGRLEJISS1K6E-extra"
        ));
    }

    #[test]
    fn test_looks_like_localrouter_key_wrong_prefix() {
        // Same length but wrong prefix
        assert!(!looks_like_localrouter_key(
            "sk-8xIF-tmewuD4eOm1dxHKRjiCAD57nLAGRLEJISS1K6E"
        ));
    }

    #[test]
    fn test_looks_like_localrouter_key_openai_format() {
        // OpenAI key format
        assert!(!looks_like_localrouter_key(
            "sk-proj-abcdefghijklmnopqrstuvwxyz123456"
        ));
    }

    #[test]
    fn test_looks_like_localrouter_key_anthropic_format() {
        // Anthropic key format
        assert!(!looks_like_localrouter_key(
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz"
        ));
    }

    #[test]
    fn test_external_provider_allowed() {
        // External provider keys (non lr- prefix) should always pass without keychain check
        let config = AppConfig {
            providers: vec![ProviderConfig {
                name: "OpenAI".to_string(),
                provider_type: ProviderType::OpenAI,
                enabled: true,
                provider_config: Some(serde_json::json!({
                    "api_key": "sk-proj-abcdefghijklmnopqrstuvwxyz123456"
                })),
                api_key_ref: None,
                free_tier: None,
            }],
            ..Default::default()
        };

        assert!(validate_providers_not_self_referential(&config).is_ok());
    }

    #[test]
    fn test_provider_without_api_key_allowed() {
        // Provider without api_key (e.g., Ollama) should pass
        let config = AppConfig {
            providers: vec![ProviderConfig {
                name: "Ollama".to_string(),
                provider_type: ProviderType::Ollama,
                enabled: true,
                provider_config: Some(serde_json::json!({
                    "base_url": "http://localhost:11434"
                })),
                api_key_ref: None,
                free_tier: None,
            }],
            ..Default::default()
        };

        assert!(validate_providers_not_self_referential(&config).is_ok());
    }

    #[test]
    fn test_provider_without_config_allowed() {
        // Provider without provider_config should pass
        let config = AppConfig {
            providers: vec![ProviderConfig {
                name: "Default".to_string(),
                provider_type: ProviderType::Ollama,
                enabled: true,
                provider_config: None,
                api_key_ref: None,
                free_tier: None,
            }],
            ..Default::default()
        };

        assert!(validate_providers_not_self_referential(&config).is_ok());
    }

    #[test]
    fn test_lr_key_passes_when_no_clients() {
        // An lr- prefixed key should pass if there are no clients configured
        // (can't be self-referential if no clients exist)
        let config = AppConfig {
            clients: vec![], // No clients
            providers: vec![ProviderConfig {
                name: "Some Provider".to_string(),
                provider_type: ProviderType::Custom,
                enabled: true,
                provider_config: Some(serde_json::json!({
                    "api_key": "lr-8xIF-tmewuD4eOm1dxHKRjiCAD57nLAGRLEJISS1K6E"
                })),
                api_key_ref: None,
                free_tier: None,
            }],
            ..Default::default()
        };

        // Should pass because no clients exist to match against
        assert!(validate_providers_not_self_referential(&config).is_ok());
    }
}
