//! Configuration migration system
//!
//! Handles migrating configuration files between versions.

use super::{AppConfig, CONFIG_VERSION};
use lr_types::AppResult;
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

    // Migrate to v3: Skills system
    if config.version < 3 {
        config = migrate_to_v3(config)?;
    }

    // Migrate to v4: Unified skill paths, disabled_skills, path-based SkillsAccess
    if config.version < 4 {
        config = migrate_to_v4(config)?;
    }

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
    Ok(config)
}

/// Migrate OAuthClientConfig to unified Client
#[allow(dead_code)]
fn migrate_oauth_client_to_client(
    oauth_client: &super::OAuthClientConfig,
    keychain: &dyn lr_api_keys::keychain_trait::KeychainStorage,
) -> AppResult<(super::Client, String)> {
    use lr_config::Client;

    // Retrieve the old OAuth client secret from keychain
    let old_service = "LocalRouter-OAuthClients";
    let secret = keychain
        .get(old_service, &oauth_client.id)?
        .ok_or_else(|| {
            lr_types::AppError::Config(format!(
                "OAuth client secret not found in keychain: {}",
                oauth_client.id
            ))
        })?;

    // Create new client with preserved client_id
    // Note: strategy_id would need to be created/assigned separately
    let mut client = Client::new_with_strategy(oauth_client.name.clone(), String::new());

    // Store secret in new keychain location
    let new_service = "LocalRouter-Clients";
    keychain.store(new_service, &client.id, &secret)?;

    // Delete old keychain entry
    if let Err(e) = keychain.delete(old_service, &oauth_client.id) {
        tracing::warn!(
            "Failed to delete old OAuth client from keychain ({}): {}",
            oauth_client.id,
            e
        );
    }

    // Copy linked server IDs to mcp_server_access
    client.mcp_server_access = if oauth_client.linked_server_ids.is_empty() {
        lr_config::McpServerAccess::None
    } else {
        lr_config::McpServerAccess::Specific(oauth_client.linked_server_ids.clone())
    };

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

/// Migrate to version 3: Skills system
///
/// Adds default skills configuration and skills_access to clients.
/// These fields use #[serde(default)] so existing configs will get
/// default values automatically. This migration just bumps the version.
fn migrate_to_v3(mut config: AppConfig) -> AppResult<AppConfig> {
    info!("Migrating to version 3: Skills system");

    // No data transformation needed - serde defaults handle the new fields:
    // - AppConfig.skills defaults to SkillsConfig::default()
    // - Client.skills_access defaults to SkillsAccess::None
    config.version = 3;
    Ok(config)
}

/// Migrate to version 4: Unified skill paths, disabled_skills, path-based SkillsAccess
///
/// - Merges `auto_scan_directories` and `skill_paths` into unified `paths`
/// - Converts any `Specific(names)` to `All` since we can't reliably map names to paths
fn migrate_to_v4(mut config: AppConfig) -> AppResult<AppConfig> {
    info!("Migrating to version 4: Unified skill paths and path-based access");

    // Merge old auto_scan_directories + skill_paths into unified paths
    let mut unified_paths = Vec::new();
    for dir in &config.skills.auto_scan_directories {
        if !unified_paths.contains(dir) {
            unified_paths.push(dir.clone());
        }
    }
    for path in &config.skills.skill_paths {
        if !unified_paths.contains(path) {
            unified_paths.push(path.clone());
        }
    }
    config.skills.paths = unified_paths;
    // Clear old fields (they won't be serialized due to skip_serializing, but clear for consistency)
    config.skills.auto_scan_directories = Vec::new();
    config.skills.skill_paths = Vec::new();

    // Convert any Specific(names) to All since we can't map skill names to source paths
    for client in &mut config.clients {
        if let super::SkillsAccess::Specific(_) = &client.skills_access {
            info!(
                "Client '{}': converting Specific skills access to All (names can't be mapped to paths)",
                client.name
            );
            client.skills_access = super::SkillsAccess::All;
        }
    }

    config.version = 4;
    Ok(config)
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
