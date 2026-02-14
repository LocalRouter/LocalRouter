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

/// Migrate to version 8: Custom guardrail rules
///
/// Adds custom_rules field to GuardrailsConfig. The field uses `#[serde(default)]`
/// so existing configs get an empty vec automatically. This just bumps the version.
fn migrate_to_v8(mut config: AppConfig) -> AppResult<AppConfig> {
    info!("Migrating to version 8: Custom guardrail rules");

    // No data transformation needed - serde defaults handle the new field:
    // - GuardrailsConfig.custom_rules defaults to vec![]
    config.version = 8;
    Ok(config)
}

/// Migrate to version 9: ML model guardrail sources
///
/// Adds Prompt Guard 2 model source to guardrail defaults and new fields
/// (confidence_threshold, model_architecture, hf_repo_id) to GuardrailSourceConfig.
/// All new fields use `#[serde(default)]` so existing configs get defaults automatically.
fn migrate_to_v9(mut config: AppConfig) -> AppResult<AppConfig> {
    info!("Migrating to version 9: ML model guardrail sources");

    // Add Prompt Guard 2 if not already present
    let has_pg2 = config
        .guardrails
        .sources
        .iter()
        .any(|s| s.id == "prompt_guard_2");
    if !has_pg2 {
        use super::GuardrailSourceConfig;
        config.guardrails.sources.push(GuardrailSourceConfig {
            id: "prompt_guard_2".to_string(),
            label: "Prompt Guard 2 (Meta)".to_string(),
            source_type: "model".to_string(),
            enabled: false,
            url: "https://huggingface.co/meta-llama/Prompt-Guard-86M".to_string(),
            data_paths: vec![],
            branch: "main".to_string(),
            predefined: true,
            confidence_threshold: 0.7,
            model_architecture: Some("bert".to_string()),
            hf_repo_id: Some("meta-llama/Prompt-Guard-86M".to_string()),
            requires_auth: true,
        });
        info!("Added Prompt Guard 2 model source to guardrails config");
    }

    config.version = 9;
    Ok(config)
}

/// Migrate to version 10: DeBERTa-v2 architecture fix + new ML models + requires_auth
///
/// - Fix prompt_guard_2 architecture from "bert" to "deberta_v2" and set requires_auth=true
/// - Add protectai_injection_v2 and jailbreak_classifier model sources
/// - Set requires_auth=false default for all existing non-gated sources
fn migrate_to_v10(mut config: AppConfig) -> AppResult<AppConfig> {
    info!("Migrating to version 10: DeBERTa-v2 fix + new ML models");

    // Fix prompt_guard_2: architecture should be deberta_v2, not bert
    for source in &mut config.guardrails.sources {
        if source.id == "prompt_guard_2" {
            source.model_architecture = Some("deberta_v2".to_string());
            source.requires_auth = true;
            info!("Fixed prompt_guard_2: architecture -> deberta_v2, requires_auth -> true");
        }
    }

    // Add protectai_injection_v2 if not present
    let has_protectai = config
        .guardrails
        .sources
        .iter()
        .any(|s| s.id == "protectai_injection_v2");
    if !has_protectai {
        use super::GuardrailSourceConfig;
        config.guardrails.sources.push(GuardrailSourceConfig {
            id: "protectai_injection_v2".to_string(),
            label: "ProtectAI Injection v2".to_string(),
            source_type: "model".to_string(),
            enabled: false,
            url: "https://huggingface.co/protectai/deberta-v3-base-prompt-injection-v2".to_string(),
            data_paths: vec![],
            branch: "main".to_string(),
            predefined: true,
            confidence_threshold: 0.7,
            model_architecture: Some("deberta_v2".to_string()),
            hf_repo_id: Some(
                "protectai/deberta-v3-base-prompt-injection-v2".to_string(),
            ),
            requires_auth: false,
        });
        info!("Added ProtectAI Injection v2 model source");
    }

    // Add jailbreak_classifier if not present
    let has_jailbreak = config
        .guardrails
        .sources
        .iter()
        .any(|s| s.id == "jailbreak_classifier");
    if !has_jailbreak {
        use super::GuardrailSourceConfig;
        config.guardrails.sources.push(GuardrailSourceConfig {
            id: "jailbreak_classifier".to_string(),
            label: "Jailbreak Classifier (jackhhao)".to_string(),
            source_type: "model".to_string(),
            enabled: false,
            url: "https://huggingface.co/jackhhao/jailbreak-classifier".to_string(),
            data_paths: vec![],
            branch: "main".to_string(),
            predefined: true,
            confidence_threshold: 0.7,
            model_architecture: Some("bert".to_string()),
            hf_repo_id: Some("jackhhao/jailbreak-classifier".to_string()),
            requires_auth: false,
        });
        info!("Added Jailbreak Classifier model source");
    }

    config.version = 10;
    Ok(config)
}

/// Migrate to version 11: Fix hf_repo_id for existing model sources
///
/// - Extract hf_repo_id from URL for model sources where it's None
/// - Rename deberta_injection → protectai_injection_v2 (canonical ID)
/// - Remove defunct sources: purple_llama, payloads_all_the_things, nemo_guardrails
fn migrate_to_v11(mut config: AppConfig) -> AppResult<AppConfig> {
    info!("Migrating to version 11: Fix hf_repo_id for existing model sources");

    // Remove defunct sources that produce 0 useful rules
    let defunct_sources = [
        "purple_llama",           // Benchmark dataset, not regex patterns; path was 404
        "payloads_all_the_things", // README prose, not pattern lists (produces garbage rules)
        "nemo_guardrails",        // ML-only detection (GPT-2 perplexity), zero regex patterns
        "presidio",               // Python recognizer classes, not regex pattern files
    ];
    let before = config.guardrails.sources.len();
    config
        .guardrails
        .sources
        .retain(|s| !defunct_sources.contains(&s.id.as_str()));
    let removed = before - config.guardrails.sources.len();
    if removed > 0 {
        info!("Removed {} defunct guardrail sources", removed);
    }

    // Rename deberta_injection → protectai_injection_v2
    for source in &mut config.guardrails.sources {
        if source.id == "deberta_injection" {
            source.id = "protectai_injection_v2".to_string();
            source.label = "ProtectAI Injection v2".to_string();
            source.predefined = true;
            info!("Renamed deberta_injection → protectai_injection_v2");
        }
    }

    // Fix hf_repo_id for model sources by extracting from URL
    for source in &mut config.guardrails.sources {
        if source.source_type == "model" && source.hf_repo_id.is_none() {
            if let Some(repo_id) = extract_hf_repo_id_from_url(&source.url) {
                info!(
                    "Set hf_repo_id for '{}' from URL: {}",
                    source.id, repo_id
                );
                source.hf_repo_id = Some(repo_id);
            }
        }
    }

    config.version = 11;
    Ok(config)
}

/// Extract HuggingFace repo ID (owner/model) from a HuggingFace URL
fn extract_hf_repo_id_from_url(url: &str) -> Option<String> {
    // Match https://huggingface.co/owner/model (with optional trailing path/slash)
    let prefix = "https://huggingface.co/";
    let path = url.strip_prefix(prefix)?;
    let parts: Vec<&str> = path.trim_end_matches('/').splitn(3, '/').collect();
    if parts.len() >= 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        Some(format!("{}/{}", parts[0], parts[1]))
    } else {
        None
    }
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
