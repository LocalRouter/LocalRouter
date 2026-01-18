//! Configuration migration system
//!
//! Handles migrating configuration files between versions.

use super::{AppConfig, CONFIG_VERSION};
use crate::utils::errors::AppResult;
use tracing::info;

/// Migrate configuration from an older version to the current version
pub fn migrate_config(mut config: AppConfig) -> AppResult<AppConfig> {
    let original_version = config.version;

    if original_version >= CONFIG_VERSION {
        // No migration needed
        return Ok(config);
    }

    info!(
        "Migrating configuration from version {} to {}",
        original_version, CONFIG_VERSION
    );

    // Apply migrations sequentially
    if config.version < 1 {
        config = migrate_to_v1(config)?;
    }

    // Migrate to v2: Unified client system
    if config.version < 2 {
        config = migrate_to_v2(config)?;
    }

    // Note: v2 also includes MCP server config updates (auth_config, discovered_oauth, HttpSse)
    // These are handled automatically via serde aliases and backward compatibility fields
    // No explicit migration needed since:
    // - HttpSse accepts "sse" via #[serde(alias = "sse")]
    // - oauth_config is kept for backward compatibility
    // - auth_config defaults to None
    // - discovered_oauth defaults to None

    // Update version to current
    config.version = CONFIG_VERSION;

    info!(
        "Successfully migrated configuration from version {} to {}",
        original_version, CONFIG_VERSION
    );

    Ok(config)
}

/// Migrate to version 1 (initial version)
///
/// This is a placeholder for the initial version. In practice, version 1
/// is the first version, so there's nothing to migrate from.
fn migrate_to_v1(config: AppConfig) -> AppResult<AppConfig> {
    // Version 1 is the initial version, so no actual migration is needed
    // This function exists as a template for future migrations
    Ok(config)
}

/// Migrate to version 2: Unified client system
///
/// Migrates from separate ApiKeyConfig and OAuthClientConfig to unified Client struct.
/// - ApiKeyConfig → Client (with LLM access, no MCP access)
/// - OAuthClientConfig → Client (with MCP access, no LLM access)
/// - Handles keychain migration automatically
fn migrate_to_v2(mut config: AppConfig) -> AppResult<AppConfig> {
    info!("Migrating to version 2: Unified client system");

    // Migration is disabled - no need to migrate from old format
    // API keys and OAuth clients have been replaced with unified Client system
    config.version = 2;
    return Ok(config);

//
//     let mut migrated_clients = Vec::new();
// 
//     // Migrate API keys
//     info!("Migrating {} API keys", config.api_keys.len());
//     for api_key in &config.api_keys {
//         match migrate_api_key_to_client(api_key, &keychain) {
//             Ok((client, _secret)) => {
//                 migrated_clients.push(client);
//             }
//             Err(e) => {
//                 tracing::warn!("Failed to migrate API key '{}': {}", api_key.name, e);
//                 // Continue with other migrations
//             }
//         }
//     }

//     // Migrate OAuth clients
//     info!("Migrating {} OAuth clients", config.oauth_clients.len());
//     for oauth_client in &config.oauth_clients {
//         match migrate_oauth_client_to_client(oauth_client, &keychain) {
//             Ok((client, _secret)) => {
//                 migrated_clients.push(client);
//             }
//             Err(e) => {
//                 tracing::warn!(
//                     "Failed to migrate OAuth client '{}': {}",
//                     oauth_client.name,
//                     e
//                 );
//                 // Continue with other migrations
//             }
//         }
//     }
// 
//     info!(
//         "Successfully migrated {} clients from {} API keys and {} OAuth clients",
//         migrated_clients.len(),
//         config.api_keys.len(),
//         config.oauth_clients.len()
//     );
// 
//     // Replace old configs with new unified clients
//     config.clients = migrated_clients;
//     config.api_keys = vec![];
//     config.oauth_clients = vec![];
// 
//     config.version = 2;
//     Ok(config)
}

/// Migrate ApiKeyConfig to unified Client
// fn migrate_api_key_to_client(
//     api_key: &super::ApiKeyConfig,
//     keychain: &dyn crate::api_keys::keychain_trait::KeychainStorage,
// ) -> AppResult<(super::Client, String)> {
//     use crate::config::Client;
// 
//     // Retrieve the old API key secret from keychain
//     let old_service = "LocalRouter-APIKeys";
//     let secret = keychain
//         .get(old_service, &api_key.id)?
//         .ok_or_else(|| {
//             crate::utils::errors::AppError::Config(format!(
//                 "API key secret not found in keychain: {}",
//                 api_key.id
//             ))
//         })?;
// 
//     // Create new client
//     let client = Client::new(api_key.name.clone());
// 
//     // Store secret in new keychain location
//     let new_service = "LocalRouter-Clients";
//     keychain.store(new_service, &client.id, &secret)?;
// 
//     // Delete old keychain entry
//     if let Err(e) = keychain.delete(old_service, &api_key.id) {
//         tracing::warn!(
//             "Failed to delete old API key from keychain ({}): {}",
//             api_key.id, e
//         );
//     }
// 
//     // Copy allowed LLM providers from model selection config
//     let mut migrated_client = client.clone();
//     migrated_client.allowed_llm_providers = if let Some(routing_config) = &api_key.routing_config {
//         // Use new routing_config if present
//         routing_config.available_models.all_provider_models.clone()
//     } else if let Some(model_selection) = &api_key.model_selection {
//         // Fall back to deprecated model_selection
//         match model_selection {
//             super::ModelSelection::All => vec![], // Empty means all providers allowed
//             super::ModelSelection::Custom { all_provider_models, .. } => all_provider_models.clone(),
//             _ => vec![], // For DirectModel and Router, allow all providers
//         }
//     } else {
//         vec![] // No selection means all providers allowed
//     };
// 
//     // Preserve enabled status
//     migrated_client.enabled = api_key.enabled;
// 
//     // Preserve timestamps
//     if let Some(last_used) = api_key.last_used {
//         migrated_client.last_used = Some(last_used);
//     }
// 
//     info!(
//         "Migrated API key '{}' to client '{}'",
//         api_key.name, migrated_client.id
//     );
// 
//     Ok((migrated_client, secret))
// }
// 
/// Migrate OAuthClientConfig to unified Client
fn migrate_oauth_client_to_client(
    oauth_client: &super::OAuthClientConfig,
    keychain: &dyn crate::api_keys::keychain_trait::KeychainStorage,
) -> AppResult<(super::Client, String)> {
    use crate::config::Client;

    // Retrieve the old OAuth client secret from keychain
    let old_service = "LocalRouter-OAuthClients";
    let secret = keychain
        .get(old_service, &oauth_client.id)?
        .ok_or_else(|| {
            crate::utils::errors::AppError::Config(format!(
                "OAuth client secret not found in keychain: {}",
                oauth_client.id
            ))
        })?;

    // Create new client with preserved client_id
    let mut client = Client::new(oauth_client.name.clone());

    // Store secret in new keychain location
    let new_service = "LocalRouter-Clients";
    keychain.store(new_service, &client.id, &secret)?;

    // Delete old keychain entry
    if let Err(e) = keychain.delete(old_service, &oauth_client.id) {
        tracing::warn!(
            "Failed to delete old OAuth client from keychain ({}): {}",
            oauth_client.id, e
        );
    }

    // Copy linked server IDs to allowed_mcp_servers
    client.allowed_mcp_servers = oauth_client.linked_server_ids.clone();

    // OAuth clients don't have LLM provider access by default
    client.allowed_llm_providers = vec![];

    // Preserve enabled status
    client.enabled = oauth_client.enabled;

    // Preserve timestamps
    client.created_at = oauth_client.created_at;
    if let Some(last_used) = oauth_client.last_used {
        client.last_used = Some(last_used);
    }

    info!(
        "Migrated OAuth client '{}' ({}) to unified client",
        oauth_client.name, oauth_client.client_id
    );

    Ok((client, secret))
}

// Future migration functions will follow this pattern:
//
// fn migrate_to_v2(mut config: AppConfig) -> AppResult<AppConfig> {
//     info!("Migrating to version 2");
//
//     // Example: Add new field with default value
//     // config.new_field = default_value();
//
//     // Example: Rename a field
//     // config.new_name = config.old_name.clone();
//
//     // Example: Transform data structure
//     // config.items = config.old_items.iter().map(|item| transform(item)).collect();
//
//     config.version = 2;
//     Ok(config)
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrate_current_version() {
        let config = AppConfig::default();
        let original_version = config.version;

        let migrated = migrate_config(config).unwrap();

        assert_eq!(migrated.version, original_version);
        assert_eq!(migrated.version, CONFIG_VERSION);
    }

    #[test]
    fn test_migrate_from_future_version() {
        let config = AppConfig {
            version: CONFIG_VERSION + 1,
            ..Default::default()
        };

        let result = migrate_config(config);

        // Should succeed (no migration needed)
        assert!(result.is_ok());
        assert_eq!(result.unwrap().version, CONFIG_VERSION + 1);
    }

    #[test]
    fn test_migrate_preserves_data() {
        let mut config = AppConfig::default();
        let original_host = "test.example.com".to_string();
        config.server.host = original_host.clone();
        config.version = 0; // Old version

        let migrated = migrate_config(config).unwrap();

        assert_eq!(migrated.version, CONFIG_VERSION);
        assert_eq!(migrated.server.host, original_host);
    }
}
