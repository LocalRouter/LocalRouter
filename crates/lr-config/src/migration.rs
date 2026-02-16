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

    // Migrate to v5: Per-skill-name access (replaces source-path-based SkillsAccess)
    if config.version < 5 {
        config = migrate_to_v5(config)?;
    }

    // Migrate to v6: Unified permission system (replaces old access fields)
    if config.version < 6 {
        config = migrate_to_v6(config)?;
    }

    // Migrate to v7: GuardRails configuration
    if config.version < 7 {
        config = migrate_to_v7(config)?;
    }

    // Migrate to v8: Custom guardrail rules
    if config.version < 8 {
        config = migrate_to_v8(config)?;
    }

    // Migrate to v9: ML model guardrail sources
    if config.version < 9 {
        config = migrate_to_v9(config)?;
    }

    // Migrate to v10: DeBERTa-v2 fix + new ML models + requires_auth
    if config.version < 10 {
        config = migrate_to_v10(config)?;
    }

    // Migrate to v11: Fix hf_repo_id for existing model sources + rename deberta_injection
    if config.version < 11 {
        config = migrate_to_v11(config)?;
    }

    // Migrate to v12: LLM-based safety models (replace regex/YARA/ML classifier sources)
    if config.version < 12 {
        config = migrate_to_v12(config)?;
    }

    // Migrate to v13: Per-client guardrails configuration
    if config.version < 13 {
        config = migrate_to_v13(config)?;
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
    use crate::Client;

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
        crate::McpServerAccess::None
    } else {
        crate::McpServerAccess::Specific(oauth_client.linked_server_ids.clone())
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

/// Migrate to version 5: Per-skill-name access
///
/// SkillsAccess::Specific now contains skill names instead of source paths.
/// Since we can't reliably map source paths to skill names at migration time,
/// convert any `Specific(paths)` → `All`.
fn migrate_to_v5(mut config: AppConfig) -> AppResult<AppConfig> {
    info!("Migrating to version 5: Per-skill-name access");

    for client in &mut config.clients {
        if let super::SkillsAccess::Specific(_) = &client.skills_access {
            info!(
                "Client '{}': converting Specific skills access to All (paths can't be mapped to names)",
                client.name
            );
            client.skills_access = super::SkillsAccess::All;
        }
    }

    config.version = 5;
    Ok(config)
}

/// Migrate to version 6: Unified permission system
///
/// Replaces old access fields with new hierarchical permission system:
/// - allowed_llm_providers → model_permissions
/// - mcp_server_access → mcp_permissions
/// - skills_access → skills_permissions
/// - marketplace_enabled → marketplace_permission
fn migrate_to_v6(mut config: AppConfig) -> AppResult<AppConfig> {
    use super::{McpServerAccess, PermissionState, SkillsAccess};

    info!("Migrating to version 6: Unified permission system");

    for client in &mut config.clients {
        // Migrate allowed_llm_providers → model_permissions
        if !client.allowed_llm_providers.is_empty() {
            // Set global to Off, then set specific providers to Allow
            client.model_permissions.global = PermissionState::Off;
            for provider in &client.allowed_llm_providers {
                client
                    .model_permissions
                    .providers
                    .insert(provider.clone(), PermissionState::Allow);
            }
            info!(
                "Client '{}': migrated {} LLM providers to model_permissions",
                client.name,
                client.allowed_llm_providers.len()
            );
        } else {
            // Empty list means no access
            client.model_permissions.global = PermissionState::Off;
        }

        // Migrate mcp_server_access → mcp_permissions
        match &client.mcp_server_access {
            McpServerAccess::None => {
                client.mcp_permissions.global = PermissionState::Off;
            }
            McpServerAccess::All => {
                client.mcp_permissions.global = PermissionState::Allow;
            }
            McpServerAccess::Specific(servers) => {
                client.mcp_permissions.global = PermissionState::Off;
                for server_id in servers {
                    client
                        .mcp_permissions
                        .servers
                        .insert(server_id.clone(), PermissionState::Allow);
                }
                info!(
                    "Client '{}': migrated {} MCP servers to mcp_permissions",
                    client.name,
                    servers.len()
                );
            }
        }

        // Migrate skills_access → skills_permissions
        match &client.skills_access {
            SkillsAccess::None => {
                client.skills_permissions.global = PermissionState::Off;
            }
            SkillsAccess::All => {
                client.skills_permissions.global = PermissionState::Allow;
            }
            SkillsAccess::Specific(skills) => {
                client.skills_permissions.global = PermissionState::Off;
                for skill_name in skills {
                    client
                        .skills_permissions
                        .skills
                        .insert(skill_name.clone(), PermissionState::Allow);
                }
                info!(
                    "Client '{}': migrated {} skills to skills_permissions",
                    client.name,
                    skills.len()
                );
            }
        }

        // Migrate marketplace_enabled → marketplace_permission
        if client.marketplace_enabled {
            client.marketplace_permission = PermissionState::Allow;
        } else {
            client.marketplace_permission = PermissionState::Off;
        }
    }

    config.version = 6;
    Ok(config)
}

/// Migrate to version 7: GuardRails configuration
///
/// Adds default guardrails configuration. All new fields have `#[serde(default)]`
/// so existing configs get defaults automatically. This just bumps the version.
fn migrate_to_v7(mut config: AppConfig) -> AppResult<AppConfig> {
    info!("Migrating to version 7: GuardRails configuration");

    // No data transformation needed - serde defaults handle the new fields:
    // - AppConfig.guardrails defaults to GuardrailsConfig::default()
    // - Client.guardrails_enabled defaults to None (inherit global)
    config.version = 7;
    Ok(config)
}

/// Migrate to version 8: Custom guardrail rules (legacy)
///
/// Previously added custom_rules field. Now a no-op since v12 replaces the entire
/// guardrails config with the new safety model system.
fn migrate_to_v8(mut config: AppConfig) -> AppResult<AppConfig> {
    info!("Migrating to version 8: Custom guardrail rules (legacy, no-op)");
    config.version = 8;
    Ok(config)
}

/// Migrate to version 9: ML model guardrail sources (legacy)
///
/// Previously added ML model sources. Now a no-op since v12 replaces the entire
/// guardrails config with the new safety model system.
fn migrate_to_v9(mut config: AppConfig) -> AppResult<AppConfig> {
    info!("Migrating to version 9: ML model guardrail sources (legacy, no-op)");
    config.version = 9;
    Ok(config)
}

/// Migrate to version 10: DeBERTa-v2 architecture fix + new ML models (legacy)
///
/// Previously fixed model architectures and added new sources. Now a no-op since v12
/// replaces the entire guardrails config with the new safety model system.
fn migrate_to_v10(mut config: AppConfig) -> AppResult<AppConfig> {
    info!("Migrating to version 10: DeBERTa-v2 fix + new ML models (legacy, no-op)");
    config.version = 10;
    Ok(config)
}

/// Migrate to version 11: Fix hf_repo_id for existing model sources (legacy)
///
/// Previously fixed repo IDs and removed defunct sources. Now a no-op since v12
/// replaces the entire guardrails config with the new safety model system.
fn migrate_to_v11(mut config: AppConfig) -> AppResult<AppConfig> {
    info!("Migrating to version 11: Fix hf_repo_id (legacy, no-op)");
    config.version = 11;
    Ok(config)
}

/// Migrate to version 12: LLM-based safety models
///
/// Replaces old regex/YARA/ML classifier sources with new LLM-based safety model config.
/// - Drops all old `sources` (regex, yara, model entries)
/// - Drops `custom_rules`, `update_interval_hours`, `min_popup_severity`
/// - Maps `min_popup_severity` → `default_confidence_threshold`
/// - Sets `safety_models` to defaults (all disabled)
/// - Preserves `enabled`, `scan_requests`, `scan_responses`
fn migrate_to_v12(mut config: AppConfig) -> AppResult<AppConfig> {
    info!("Migrating to version 12: LLM-based safety models");

    // Note: old fields (min_popup_severity, sources, custom_rules) are already lost
    // during deserialization since the struct no longer has them. We use a default
    // confidence threshold of 0.5 (medium).

    // Preserve the fields that carry over
    let enabled = config.guardrails.enabled;
    let scan_requests = config.guardrails.scan_requests;
    let scan_responses = config.guardrails.scan_responses;

    // Replace with new config using defaults
    config.guardrails = super::GuardrailsConfig {
        enabled,
        scan_requests,
        scan_responses,
        default_confidence_threshold: 0.5,
        ..Default::default()
    };

    info!("Migrated guardrails to LLM-based safety models (confidence threshold: 0.5)");

    config.version = 12;
    Ok(config)
}

/// Migrate to version 13: Per-client guardrails configuration
///
/// Moves guardrails from global config to per-client:
/// - Old `guardrails_enabled: Some(true)` → `guardrails.enabled = true`
/// - Global `category_actions` are dropped (users reconfigure per-client)
/// - Global `enabled` and `scan_responses` become migration shims (not serialized)
fn migrate_to_v13(mut config: AppConfig) -> AppResult<AppConfig> {
    info!("Migrating to version 13: Per-client guardrails configuration");

    let global_enabled = config.guardrails.enabled;

    for client in &mut config.clients {
        // Migrate old guardrails_enabled override
        let was_enabled = client.guardrails_enabled.unwrap_or(global_enabled);
        if was_enabled {
            client.guardrails.enabled = true;
        }
        // Clear old field
        client.guardrails_enabled = None;
    }

    // Clear global fields that are no longer used
    config.guardrails.enabled = false;
    config.guardrails.scan_responses = false;
    config.guardrails.category_actions = vec![];

    config.version = 13;
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
