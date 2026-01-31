//! Configuration validation

#![allow(deprecated)]

use super::{AppConfig, ProviderConfig};
use crate::api_keys::keychain_trait::KeychainStorage;
use crate::api_keys::CachedKeychain;
use crate::utils::errors::{AppError, AppResult};
use std::collections::HashSet;

/// Service name for LocalRouter client secrets in keychain
const CLIENT_SERVICE: &str = "LocalRouter-Clients";

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

    // Check for duplicate provider names
    let mut names = HashSet::new();
    for provider in providers {
        if !names.insert(&provider.name) {
            return Err(AppError::Config(format!(
                "Duplicate provider name: {}",
                provider.name
            )));
        }

        // Validate name is not empty
        if provider.name.is_empty() {
            return Err(AppError::Config(
                "Provider name cannot be empty".to_string(),
            ));
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
        .filter_map(|client| {
            keychain.get(CLIENT_SERVICE, &client.id).ok().flatten()
        })
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

        // Validate ID is not empty
        if strategy.id.is_empty() {
            return Err(AppError::Config("Strategy ID cannot be empty".to_string()));
        }

        // Validate name is not empty
        if strategy.name.is_empty() {
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
            if limit.value <= 0.0 {
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

// #[cfg(test)]
// mod tests {
//     use super::*;
//
//     #[test]
//     fn test_validate_default_config() {
//         let config = AppConfig::default();
//         assert!(validate_config(&config).is_ok());
//     }
//
//     #[test]
//     fn test_validate_empty_server_host() {
//         let mut config = AppConfig::default();
//         config.server.host = String::new();
//         assert!(validate_config(&config).is_err());
//     }
//
//     #[test]
//     fn test_validate_invalid_port() {
//         let mut config = AppConfig::default();
//         config.server.port = 0;
//         assert!(validate_config(&config).is_err());
//     }
//
//     #[test]
//     fn test_validate_duplicate_api_key_ids() {
//         let mut config = AppConfig::default();
//         let key1 = ApiKeyConfig::with_model(
//             "key1".to_string(),
//             ModelSelection::Router {
//                 router_name: "Minimum Cost".to_string(),
//             },
//         );
//         let mut key2 = key1.clone();
//         key2.name = "key2".to_string();
//
//         config.api_keys = vec![key1, key2];
//         assert!(validate_config(&config).is_err());
//     }
//
//     #[test]
//     fn test_validate_empty_api_key_name() {
//         let mut config = AppConfig::default();
//         let key = ApiKeyConfig::with_model(
//             String::new(),
//             ModelSelection::Router {
//                 router_name: "Minimum Cost".to_string(),
//             },
//         );
//         config.api_keys = vec![key];
//         assert!(validate_config(&config).is_err());
//     }
//
//     #[test]
//     fn test_validate_no_routers() {
//         let mut config = AppConfig::default();
//         config.routers.clear();
//         assert!(validate_config(&config).is_err());
//     }
//
//     #[test]
//     fn test_validate_duplicate_router_names() {
//         let mut config = AppConfig::default();
//         let router1 = RouterConfig::default_minimum_cost();
//         let router2 = RouterConfig::default_minimum_cost();
//         config.routers = vec![router1, router2];
//         assert!(validate_config(&config).is_err());
//     }
//
//     #[test]
//     fn test_validate_router_no_strategies() {
//         let mut config = AppConfig::default();
//         config.routers[0].strategies.clear();
//         assert!(validate_config(&config).is_err());
//     }
//
//     #[test]
//     fn test_validate_no_providers() {
//         let mut config = AppConfig::default();
//         config.providers.clear();
//         assert!(validate_config(&config).is_err());
//     }
//
//     #[test]
//     fn test_validate_duplicate_provider_names() {
//         let mut config = AppConfig::default();
//         let provider1 = ProviderConfig::default_ollama();
//         let provider2 = ProviderConfig::default_ollama();
//         config.providers = vec![provider1, provider2];
//         assert!(validate_config(&config).is_err());
//     }
//
//     #[test]
//     fn test_validate_invalid_provider_config() {
//         use serde_json::json;
//         let mut config = AppConfig::default();
//         // Provider config must be an object, not a primitive
//         config.providers[0].provider_config = Some(json!("not an object"));
//         assert!(validate_config(&config).is_err());
//     }
//
//     #[test]
//     fn test_validate_api_key_references_nonexistent_router() {
//         let mut config = AppConfig::default();
//         let key = ApiKeyConfig::with_model(
//             "test".to_string(),
//             ModelSelection::Router {
//                 router_name: "NonExistent".to_string(),
//             },
//         );
//         config.api_keys = vec![key];
//         assert!(validate_config(&config).is_err());
//     }
//
//     #[test]
//     fn test_validate_api_key_references_nonexistent_provider() {
//         let mut config = AppConfig::default();
//         let key = ApiKeyConfig::with_model(
//             "test".to_string(),
//             ModelSelection::DirectModel {
//                 provider: "NonExistent".to_string(),
//                 model: "model".to_string(),
//             },
//         );
//         config.api_keys = vec![key];
//         assert!(validate_config(&config).is_err());
//     }
// }

#[cfg(test)]
mod self_referential_tests {
    use super::*;
    use crate::config::ProviderType;

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
        let mut config = AppConfig::default();
        config.providers = vec![ProviderConfig {
            name: "OpenAI".to_string(),
            provider_type: ProviderType::OpenAI,
            enabled: true,
            provider_config: Some(serde_json::json!({
                "api_key": "sk-proj-abcdefghijklmnopqrstuvwxyz123456"
            })),
            api_key_ref: None,
        }];

        assert!(validate_providers_not_self_referential(&config).is_ok());
    }

    #[test]
    fn test_provider_without_api_key_allowed() {
        // Provider without api_key (e.g., Ollama) should pass
        let mut config = AppConfig::default();
        config.providers = vec![ProviderConfig {
            name: "Ollama".to_string(),
            provider_type: ProviderType::Ollama,
            enabled: true,
            provider_config: Some(serde_json::json!({
                "base_url": "http://localhost:11434"
            })),
            api_key_ref: None,
        }];

        assert!(validate_providers_not_self_referential(&config).is_ok());
    }

    #[test]
    fn test_provider_without_config_allowed() {
        // Provider without provider_config should pass
        let mut config = AppConfig::default();
        config.providers = vec![ProviderConfig {
            name: "Default".to_string(),
            provider_type: ProviderType::Ollama,
            enabled: true,
            provider_config: None,
            api_key_ref: None,
        }];

        assert!(validate_providers_not_self_referential(&config).is_ok());
    }

    #[test]
    fn test_lr_key_passes_when_no_clients() {
        // An lr- prefixed key should pass if there are no clients configured
        // (can't be self-referential if no clients exist)
        let mut config = AppConfig::default();
        config.clients = vec![]; // No clients
        config.providers = vec![ProviderConfig {
            name: "Some Provider".to_string(),
            provider_type: ProviderType::Custom,
            enabled: true,
            provider_config: Some(serde_json::json!({
                "api_key": "lr-8xIF-tmewuD4eOm1dxHKRjiCAD57nLAGRLEJISS1K6E"
            })),
            api_key_ref: None,
        }];

        // Should pass because no clients exist to match against
        assert!(validate_providers_not_self_referential(&config).is_ok());
    }
}
