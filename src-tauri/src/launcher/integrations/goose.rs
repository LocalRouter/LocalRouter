//! Goose integration
//!
//! Native gateway setup writes a custom OpenAI-compatible provider and selects
//! an available model in Goose's desktop/CLI configuration. MCP setup is
//! independent and adds a Streamable HTTP extension when enabled.

use crate::launcher::backup;
use crate::launcher::{AppIntegration, ConfigSyncContext};
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};
use serde_json::json;
use serde_yaml::{Mapping, Value};
use std::path::{Path, PathBuf};

pub struct GooseIntegration;

const PROVIDER_ID: &str = "localrouter";

struct GoosePaths {
    config: PathBuf,
    provider: PathBuf,
    backups: PathBuf,
}

impl GoosePaths {
    fn user() -> Self {
        let dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".config")
            .join("goose");
        Self {
            config: dir.join("config.yaml"),
            provider: dir.join("custom_providers").join("localrouter.json"),
            backups: backup::default_backup_dir(),
        }
    }
}

impl AppIntegration for GooseIntegration {
    fn name(&self) -> &str {
        "Goose"
    }

    fn check_installed(&self) -> AppCapabilities {
        let binary = super::find_binary("goose");

        AppCapabilities {
            installed: binary.is_some(),
            binary_path: binary.map(|p| p.to_string_lossy().to_string()),
            version: None,
            supports_try_it_out: self.supports_try_it_out(),
            supports_permanent_config: self.supports_permanent_config(),
        }
    }

    fn supports_try_it_out(&self) -> bool {
        true
    }

    fn supports_permanent_config(&self) -> bool {
        true
    }

    fn needs_model_list(&self) -> bool {
        true
    }

    fn try_it_out(
        &self,
        _base_url: &str,
        _client_secret: &str,
        _client_id: &str,
    ) -> Result<LaunchResult, String> {
        Err("Goose setup requires the client's available models. Refresh the client setup and try again."
            .to_string())
    }

    fn try_it_out_with_context(&self, ctx: &ConfigSyncContext) -> Result<LaunchResult, String> {
        if !ctx.should_sync_llm() {
            return Err("Goose Try It Out requires LLM gateway mode. Configure the MCP extension permanently for MCP-only access."
                .to_string());
        }
        let model = ctx.models.first().ok_or_else(no_models_error)?;
        let host = gateway_root(&ctx.base_url);
        Ok(LaunchResult {
            success: true,
            message: "Run the command below in your terminal to start a new Goose session:".to_string(),
            modified_files: vec![],
            backup_files: vec![],
            terminal_command: Some(format!(
                "OPENAI_HOST={} OPENAI_BASE_PATH=chat/completions OPENAI_API_KEY={} goose session --provider openai --model {}",
                shell_quote(host),
                shell_quote(&ctx.client_secret),
                shell_quote(model),
            )),
        })
    }

    fn configure_permanent(
        &self,
        _base_url: &str,
        _client_secret: &str,
        _client_id: &str,
    ) -> Result<LaunchResult, String> {
        Err("Goose setup requires the client's available models. Refresh the client setup and try again."
            .to_string())
    }

    fn sync_config(&self, ctx: &ConfigSyncContext) -> Result<LaunchResult, String> {
        write_config_at(&GoosePaths::user(), ctx)
    }
}

fn no_models_error() -> String {
    "No models are available to this Goose client. Allow a provider or model, refresh models, and try again."
        .to_string()
}

fn gateway_root(base_url: &str) -> &str {
    let trimmed = base_url.trim_end_matches('/');
    trimmed.strip_suffix("/v1").unwrap_or(trimmed)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn yaml_key(key: &str) -> Value {
    Value::String(key.to_string())
}

fn validate_owned_mapping(map: &Mapping, key: &str, path: &Path) -> Result<(), String> {
    if let Some(value) = map.get(yaml_key(key)) {
        let nested = value
            .as_mapping()
            .ok_or_else(|| format!("Expected '{key}' to be a mapping in {}", path.display()))?;
        if nested
            .get(yaml_key(PROVIDER_ID))
            .is_some_and(|entry| !entry.is_mapping())
        {
            return Err(format!(
                "Expected '{key}.{PROVIDER_ID}' to be a mapping in {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn owned_mapping<'a>(map: &'a mut Mapping, key: &str) -> &'a mut Mapping {
    map.entry(yaml_key(key))
        .or_insert_with(|| Value::Mapping(Mapping::new()))
        .as_mapping_mut()
        .expect("nested configuration mappings were validated")
}

fn remove_owned_mapping(map: &mut Mapping, key: &str) {
    if let Some(nested) = map.get_mut(yaml_key(key)).and_then(Value::as_mapping_mut) {
        nested.remove(yaml_key(PROVIDER_ID));
    }
}

/// All existing files and owned mappings are validated before changing either
/// file, so malformed settings cannot cause a partial configuration overwrite.
fn write_config_at(paths: &GoosePaths, ctx: &ConfigSyncContext) -> Result<LaunchResult, String> {
    let existing_config = match std::fs::read_to_string(&paths.config) {
        Ok(data) => Some(data),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "Failed to read {}: {error}",
                paths.config.display()
            ))
        }
    };
    let mut config = match &existing_config {
        Some(data) => super::config_parse::yaml(data, &paths.config)?,
        None => Value::Mapping(Mapping::new()),
    };
    let map = config
        .as_mapping_mut()
        .expect("configuration parser requires a mapping");
    validate_owned_mapping(map, "extensions", &paths.config)?;
    validate_owned_mapping(map, "providers", &paths.config)?;

    let existing_provider = match std::fs::read_to_string(&paths.provider) {
        Ok(data) => {
            super::config_parse::json(&data, &paths.provider)?;
            Some(data)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "Failed to read {}: {error}",
                paths.provider.display()
            ))
        }
    };

    let provider_data = if ctx.should_sync_llm() {
        let first_model = ctx.models.first().ok_or_else(no_models_error)?;
        // Goose's modern per-provider selection takes precedence over its
        // legacy GOOSE_MODEL key. Retain an allowed LocalRouter selection.
        let selected = map
            .get(yaml_key("providers"))
            .and_then(Value::as_mapping)
            .and_then(|providers| providers.get(yaml_key(PROVIDER_ID)))
            .and_then(Value::as_mapping)
            .and_then(|entry| entry.get(yaml_key("model")))
            .and_then(Value::as_str)
            .or_else(|| {
                let is_localrouter = ["active_provider", "GOOSE_PROVIDER"]
                    .iter()
                    .any(|key| map.get(yaml_key(key)).and_then(Value::as_str) == Some(PROVIDER_ID));
                is_localrouter
                    .then(|| map.get(yaml_key("GOOSE_MODEL")).and_then(Value::as_str))
                    .flatten()
            })
            .filter(|model| ctx.models.iter().any(|allowed| allowed == model))
            .unwrap_or(first_model)
            .to_string();

        let providers = owned_mapping(map, "providers");
        let entry = providers
            .entry(yaml_key(PROVIDER_ID))
            .or_insert_with(|| Value::Mapping(Mapping::new()))
            .as_mapping_mut()
            .expect("owned provider mapping was validated");
        entry.insert(yaml_key("enabled"), Value::Bool(true));
        entry.insert(yaml_key("configured"), Value::Bool(true));
        entry.insert(yaml_key("model"), yaml_key(&selected));
        map.insert(yaml_key("active_provider"), yaml_key(PROVIDER_ID));
        map.insert(yaml_key("GOOSE_PROVIDER"), yaml_key(PROVIDER_ID));
        map.insert(yaml_key("GOOSE_MODEL"), yaml_key(&selected));

        // These are Goose's declarative-provider fields. In particular, plain
        // OPENAI_API_KEY in config.yaml is ignored by its secret resolver.
        // Static canonical IDs keep provider names unambiguous and are refreshed
        // by the model-list sync whenever the client's permissions change.
        let provider = json!({
            "name": PROVIDER_ID,
            "engine": "openai",
            "display_name": "LocalRouter",
            "api_key_env": "",
            "base_url": gateway_root(&ctx.base_url),
            // The explicit custom path keeps Goose on Chat Completions for
            // GPT models too, instead of automatically choosing Responses.
            // LocalRouter serves this versionless alias of /v1/chat/completions.
            "base_path": "chat/completions",
            "models": ctx.models.iter().map(|model| json!({ "name": model })).collect::<Vec<_>>(),
            "headers": { "Authorization": format!("Bearer {}", ctx.client_secret) },
            "requires_auth": false,
            "supports_streaming": true,
            "dynamic_models": false,
        });
        Some(
            serde_json::to_string_pretty(&provider)
                .map_err(|error| format!("Failed to serialize Goose provider: {error}"))?,
        )
    } else {
        remove_owned_mapping(map, "providers");
        let was_active =
            map.get(yaml_key("active_provider")).and_then(Value::as_str) == Some(PROVIDER_ID);
        if was_active {
            map.remove(yaml_key("active_provider"));
        }
        if map.get(yaml_key("GOOSE_PROVIDER")).and_then(Value::as_str) == Some(PROVIDER_ID) {
            map.remove(yaml_key("GOOSE_PROVIDER"));
            map.remove(yaml_key("GOOSE_MODEL"));
        } else if was_active && !map.contains_key(yaml_key("GOOSE_PROVIDER")) {
            map.remove(yaml_key("GOOSE_MODEL"));
        }
        None
    };

    if ctx.should_sync_mcp() {
        let extension = serde_yaml::to_value(json!({
            "type": "streamable_http",
            "name": "LocalRouter",
            "uri": ctx.base_url,
            "enabled": true,
            "headers": { "Authorization": format!("Bearer {}", ctx.client_secret) },
        }))
        .map_err(|error| format!("Failed to serialize Goose extension: {error}"))?;
        owned_mapping(map, "extensions").insert(yaml_key(PROVIDER_ID), extension);
    } else {
        remove_owned_mapping(map, "extensions");
    }

    let config_data = serde_yaml::to_string(&config)
        .map_err(|error| format!("Failed to serialize Goose config: {error}"))?;
    let mut result = LaunchResult {
        success: true,
        message:
            "Goose configuration synced. Restart Goose or start a new session to apply changes."
                .to_string(),
        modified_files: vec![],
        backup_files: vec![],
        terminal_command: None,
    };

    // Publish the provider before selecting it. When disabling it, stop selecting
    // it before removing its file. The shared writer creates private backups
    // and atomically replaces each file with owner-only permissions.
    if let Some(data) = provider_data {
        if existing_provider.as_deref() != Some(data.as_str()) {
            record_write(paths, &paths.provider, data.as_bytes(), &mut result)?;
        }
    }
    if (existing_config.is_some() || ctx.should_sync_llm() || ctx.should_sync_mcp())
        && existing_config.as_deref() != Some(config_data.as_str())
    {
        record_write(paths, &paths.config, config_data.as_bytes(), &mut result)?;
    }
    if !ctx.should_sync_llm() {
        if let Some(data) = existing_provider {
            remove_provider_with_backup(paths, data.as_bytes(), &mut result)?;
        }
    }
    Ok(result)
}

fn record_write(
    paths: &GoosePaths,
    path: &Path,
    data: &[u8],
    result: &mut LaunchResult,
) -> Result<(), String> {
    let saved = backup::write_with_backup_in(path, data, &paths.backups)?;
    result
        .modified_files
        .push(path.to_string_lossy().to_string());
    if let Some(saved) = saved {
        result
            .backup_files
            .push(saved.to_string_lossy().to_string());
    }
    Ok(())
}

fn remove_provider_with_backup(
    paths: &GoosePaths,
    data: &[u8],
    result: &mut LaunchResult,
) -> Result<(), String> {
    use std::io::Write;

    std::fs::create_dir_all(&paths.backups)
        .map_err(|error| format!("Failed to create Goose backup directory: {error}"))?;
    let mut saved = tempfile::Builder::new()
        .prefix("localrouter.json.removed.")
        .suffix(".bak")
        .tempfile_in(&paths.backups)
        .map_err(|error| format!("Failed to create private Goose provider backup: {error}"))?;
    saved
        .write_all(data)
        .and_then(|()| saved.as_file().sync_all())
        .map_err(|error| format!("Failed to back up Goose provider: {error}"))?;
    let (_, saved_path) = saved
        .keep()
        .map_err(|error| format!("Failed to preserve Goose provider backup: {error}"))?;
    std::fs::remove_file(&paths.provider)
        .map_err(|error| format!("Failed to remove {}: {error}", paths.provider.display()))?;
    result
        .modified_files
        .push(paths.provider.to_string_lossy().to_string());
    result
        .backup_files
        .push(saved_path.to_string_lossy().to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_config::{LlmMode, McpMode};

    fn paths(dir: &Path) -> GoosePaths {
        GoosePaths {
            config: dir.join("config.yaml"),
            provider: dir.join("custom_providers/localrouter.json"),
            backups: dir.join("backups"),
        }
    }

    fn context() -> ConfigSyncContext {
        ConfigSyncContext {
            base_url: "http://127.0.0.1:3625".into(),
            client_secret: "test-client-key".into(),
            client_id: "test-goose".into(),
            models: vec![
                "ChatGPT Plus/gpt-5.6-sol".into(),
                "ChatGPT Plus/gpt-5.6".into(),
            ],
            llm_mode: LlmMode::Gateway,
            mcp_mode: McpMode::Gateway,
            proxy_url: None,
            ca_cert_path: None,
        }
    }

    fn read_config(paths: &GoosePaths) -> Value {
        serde_yaml::from_str(&std::fs::read_to_string(&paths.config).unwrap()).unwrap()
    }

    fn read_provider(paths: &GoosePaths) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(&paths.provider).unwrap()).unwrap()
    }

    #[test]
    fn setup_replaces_stale_ollama_defaults_and_advertises_only_permitted_models() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        std::fs::write(&paths.config, "active_provider: ollama\nGOOSE_PROVIDER: ollama\nGOOSE_MODEL: llama3.2:1b\nproviders:\n  ollama:\n    enabled: true\n    configured: true\n    model: llama3.2:1b\nextensions:\n  developer:\n    type: builtin\n    enabled: true\nGOOSE_MODE: approve\n").unwrap();
        let mut ctx = context();
        ctx.mcp_mode = McpMode::Off;
        let result = write_config_at(&paths, &ctx).unwrap();
        let config = read_config(&paths);
        assert_eq!(config["active_provider"].as_str(), Some(PROVIDER_ID));
        assert_eq!(config["GOOSE_PROVIDER"].as_str(), Some(PROVIDER_ID));
        assert_eq!(config["GOOSE_MODEL"].as_str(), Some(ctx.models[0].as_str()));
        assert_eq!(
            config["providers"][PROVIDER_ID]["model"].as_str(),
            Some(ctx.models[0].as_str())
        );
        assert_eq!(
            config["providers"][PROVIDER_ID]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(
            config["providers"][PROVIDER_ID]["configured"].as_bool(),
            Some(true)
        );
        assert_eq!(
            config["providers"]["ollama"]["model"].as_str(),
            Some("llama3.2:1b")
        );
        assert_eq!(
            config["extensions"]["developer"]["type"].as_str(),
            Some("builtin")
        );
        assert_eq!(config["GOOSE_MODE"].as_str(), Some("approve"));
        assert!(config["extensions"][PROVIDER_ID].is_null());
        assert_eq!(
            read_provider(&paths),
            json!({
                "name": "localrouter",
                "engine": "openai",
                "display_name": "LocalRouter",
                "api_key_env": "",
                "base_url": "http://127.0.0.1:3625",
                "base_path": "chat/completions",
                "models": [{"name": ctx.models[0]}, {"name": ctx.models[1]}],
                "headers": {"Authorization": "Bearer test-client-key"},
                "requires_auth": false,
                "supports_streaming": true,
                "dynamic_models": false,
            })
        );
        assert_eq!(result.modified_files.len(), 2);
        assert_eq!(result.backup_files.len(), 1);
        assert!(GooseIntegration.needs_model_list());
    }

    #[test]
    fn valid_modern_selection_wins_over_legacy_and_survives_reordering() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let ctx = context();
        std::fs::write(&paths.config, format!("active_provider: localrouter\nGOOSE_PROVIDER: localrouter\nGOOSE_MODEL: {}\nproviders:\n  localrouter:\n    model: {}\n    custom_setting: kept\n", ctx.models[0], ctx.models[1])).unwrap();
        write_config_at(&paths, &ctx).unwrap();
        let config = read_config(&paths);
        assert_eq!(config["GOOSE_MODEL"].as_str(), Some(ctx.models[1].as_str()));
        assert_eq!(
            config["providers"][PROVIDER_ID]["model"].as_str(),
            Some(ctx.models[1].as_str())
        );
        assert_eq!(
            config["providers"][PROVIDER_ID]["custom_setting"].as_str(),
            Some("kept")
        );
    }

    #[test]
    fn removed_selection_is_replaced_with_first_permitted_model() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        std::fs::write(&paths.config, "active_provider: localrouter\nGOOSE_PROVIDER: localrouter\nGOOSE_MODEL: localrouter/auto\nproviders:\n  localrouter:\n    model: Ollama/llama3.2:1b\n").unwrap();
        let ctx = context();
        write_config_at(&paths, &ctx).unwrap();
        assert_eq!(
            read_config(&paths)["GOOSE_MODEL"].as_str(),
            Some(ctx.models[0].as_str())
        );
    }

    #[test]
    fn valid_legacy_localrouter_selection_is_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let ctx = context();
        std::fs::write(
            &paths.config,
            format!(
                "GOOSE_PROVIDER: localrouter\nGOOSE_MODEL: {}\n",
                ctx.models[1]
            ),
        )
        .unwrap();
        write_config_at(&paths, &ctx).unwrap();
        assert_eq!(
            read_config(&paths)["providers"][PROVIDER_ID]["model"].as_str(),
            Some(ctx.models[1].as_str())
        );
    }

    #[test]
    fn llm_and_mcp_modes_sync_and_clean_up_independently() {
        for llm in [
            LlmMode::Gateway,
            LlmMode::Off,
            LlmMode::Proxy,
            LlmMode::ReverseProxy,
        ] {
            for mcp in [McpMode::Gateway, McpMode::Off, McpMode::ViaLlm] {
                let dir = tempfile::tempdir().unwrap();
                let paths = paths(dir.path());
                let mut ctx = context();
                write_config_at(&paths, &ctx).unwrap();
                ctx.llm_mode = llm;
                ctx.mcp_mode = mcp;
                write_config_at(&paths, &ctx).unwrap();
                let config = read_config(&paths);
                assert_eq!(paths.provider.exists(), llm == LlmMode::Gateway);
                assert_eq!(
                    !config["providers"][PROVIDER_ID].is_null(),
                    llm == LlmMode::Gateway
                );
                assert_eq!(
                    !config["extensions"][PROVIDER_ID].is_null(),
                    mcp == McpMode::Gateway
                );
                if mcp == McpMode::Gateway {
                    assert_eq!(
                        config["extensions"][PROVIDER_ID]["uri"].as_str(),
                        Some(ctx.base_url.as_str())
                    );
                    assert_eq!(
                        config["extensions"][PROVIDER_ID]["headers"]["Authorization"].as_str(),
                        Some("Bearer test-client-key")
                    );
                }
                if llm != LlmMode::Gateway {
                    assert!(config["active_provider"].is_null());
                    assert!(config["GOOSE_PROVIDER"].is_null());
                    assert!(config["GOOSE_MODEL"].is_null());
                }
            }
        }
    }

    #[test]
    fn cleanup_preserves_modern_and_legacy_external_selections() {
        for (active, legacy, expected_legacy) in [
            ("anthropic", "anthropic", Some("anthropic")),
            ("localrouter", "anthropic", Some("anthropic")),
            ("anthropic", "localrouter", None),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let paths = paths(dir.path());
            let mut ctx = context();
            write_config_at(&paths, &ctx).unwrap();
            std::fs::write(&paths.config, format!("active_provider: {active}\nGOOSE_PROVIDER: {legacy}\nGOOSE_MODEL: external-model\nproviders:\n  localrouter:\n    model: old-localrouter-model\n  anthropic:\n    model: claude-chosen\n    enabled: true\nextensions:\n  localrouter:\n    enabled: true\n  developer:\n    enabled: true\n")).unwrap();
            ctx.llm_mode = LlmMode::Off;
            ctx.mcp_mode = McpMode::Off;
            write_config_at(&paths, &ctx).unwrap();
            let config = read_config(&paths);
            assert_eq!(
                config["active_provider"].as_str(),
                (active != PROVIDER_ID).then_some(active)
            );
            assert_eq!(config["GOOSE_PROVIDER"].as_str(), expected_legacy);
            assert_eq!(
                config["GOOSE_MODEL"].as_str(),
                expected_legacy.map(|_| "external-model")
            );
            assert_eq!(
                config["providers"]["anthropic"]["model"].as_str(),
                Some("claude-chosen")
            );
            assert_eq!(
                config["extensions"]["developer"]["enabled"].as_bool(),
                Some(true)
            );
            assert!(config["providers"][PROVIDER_ID].is_null());
            assert!(config["extensions"][PROVIDER_ID].is_null());
            assert!(!paths.provider.exists());
        }
    }

    #[test]
    fn malformed_yaml_or_nested_mapping_does_not_mutate_either_file() {
        for invalid in [
            "broken: [",
            "- sequence",
            "providers: []\n",
            "providers:\n  localrouter: invalid\n",
            "extensions: null\n",
            "extensions:\n  localrouter: []\n",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let paths = paths(dir.path());
            std::fs::create_dir_all(paths.provider.parent().unwrap()).unwrap();
            std::fs::write(&paths.config, invalid).unwrap();
            std::fs::write(&paths.provider, "{\"name\":\"localrouter\",\"keep\":true}").unwrap();
            let before = std::fs::read(&paths.provider).unwrap();
            assert!(
                write_config_at(&paths, &context()).is_err(),
                "accepted {invalid:?}"
            );
            assert_eq!(std::fs::read_to_string(&paths.config).unwrap(), invalid);
            assert_eq!(std::fs::read(&paths.provider).unwrap(), before);
            assert!(!paths.backups.exists());
        }
    }

    #[test]
    fn malformed_provider_does_not_change_goose_defaults() {
        for invalid in ["{", "[]", "null"] {
            let dir = tempfile::tempdir().unwrap();
            let paths = paths(dir.path());
            std::fs::create_dir_all(paths.provider.parent().unwrap()).unwrap();
            std::fs::write(&paths.config, "active_provider: ollama\n").unwrap();
            std::fs::write(&paths.provider, invalid).unwrap();
            assert!(write_config_at(&paths, &context()).is_err());
            assert_eq!(
                std::fs::read_to_string(&paths.config).unwrap(),
                "active_provider: ollama\n"
            );
            assert_eq!(std::fs::read_to_string(&paths.provider).unwrap(), invalid);
            assert!(!paths.backups.exists());
        }
    }

    #[test]
    fn empty_model_list_is_an_error_without_configuration_changes() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let mut ctx = context();
        write_config_at(&paths, &ctx).unwrap();
        let before_config = std::fs::read(&paths.config).unwrap();
        let before_provider = std::fs::read(&paths.provider).unwrap();
        ctx.models.clear();
        assert!(write_config_at(&paths, &ctx)
            .unwrap_err()
            .contains("No models"));
        assert_eq!(std::fs::read(&paths.config).unwrap(), before_config);
        assert_eq!(std::fs::read(&paths.provider).unwrap(), before_provider);
        assert!(GooseIntegration.try_it_out_with_context(&ctx).is_err());
        assert!(GooseIntegration
            .configure_permanent("http://localhost:3625", "key", "client")
            .is_err());
    }

    #[test]
    fn repeated_sync_does_not_create_more_backups_or_report_changes() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let ctx = context();
        write_config_at(&paths, &ctx).unwrap();
        let result = write_config_at(&paths, &ctx).unwrap();
        assert!(result.modified_files.is_empty());
        assert!(result.backup_files.is_empty());
        assert!(!paths.backups.exists());
    }

    #[test]
    fn disabled_modes_do_not_create_empty_configuration_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let mut ctx = context();
        ctx.llm_mode = LlmMode::Off;
        ctx.mcp_mode = McpMode::Off;
        let result = write_config_at(&paths, &ctx).unwrap();
        assert!(result.modified_files.is_empty());
        assert!(!paths.config.exists());
        assert!(!paths.provider.exists());
    }

    #[test]
    fn one_time_setup_requires_gateway_mode_and_a_real_model() {
        let mut ctx = context();
        let result = GooseIntegration.try_it_out_with_context(&ctx).unwrap();
        assert!(result.modified_files.is_empty());
        assert!(result.backup_files.is_empty());
        let command = result.terminal_command.unwrap();
        assert!(command.contains("--provider openai --model 'ChatGPT Plus/gpt-5.6-sol'"));
        assert!(!command.contains("localrouter/auto"));
        ctx.llm_mode = LlmMode::Off;
        assert!(GooseIntegration.try_it_out_with_context(&ctx).is_err());
        assert!(GooseIntegration
            .try_it_out("http://localhost:3625", "key", "client")
            .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn one_time_command_quotes_secret_and_model_as_literal_shell_arguments() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("goose");
        let capture = dir.path().join("capture");
        let sentinel = dir.path().join("injected");
        std::fs::write(&executable, "#!/bin/sh\nprintf '%s\\n' \"$OPENAI_HOST\" \"$OPENAI_BASE_PATH\" \"$OPENAI_API_KEY\" \"$@\" > \"$GOOSE_CAPTURE\"\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut ctx = context();
        ctx.base_url = "http://127.0.0.1:3625/v1/".into();
        ctx.client_secret = format!("key'$(touch {})", sentinel.display());
        ctx.models = vec![format!("Provider/model'`touch {}`", sentinel.display())];
        let command = GooseIntegration
            .try_it_out_with_context(&ctx)
            .unwrap()
            .terminal_command
            .unwrap();
        let status = std::process::Command::new("/bin/sh")
            .args(["-c", &command])
            .env("PATH", dir.path())
            .env("GOOSE_CAPTURE", &capture)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(!sentinel.exists());
        assert_eq!(std::fs::read_to_string(capture).unwrap(), format!("http://127.0.0.1:3625\nchat/completions\n{}\nsession\n--provider\nopenai\n--model\n{}\n", ctx.client_secret, ctx.models[0]));
    }

    #[cfg(unix)]
    #[test]
    fn credential_files_and_update_and_removal_backups_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let mut ctx = context();
        write_config_at(&paths, &ctx).unwrap();
        let initial_provider = std::fs::read(&paths.provider).unwrap();
        ctx.client_secret = "updated-test-key".into();
        let updated = write_config_at(&paths, &ctx).unwrap();
        assert!(updated
            .backup_files
            .iter()
            .any(|path| std::fs::read(path).unwrap() == initial_provider));
        for path in [&paths.config, &paths.provider] {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let previous_provider = std::fs::read(&paths.provider).unwrap();
        ctx.llm_mode = LlmMode::Off;
        ctx.mcp_mode = McpMode::Off;
        let removed = write_config_at(&paths, &ctx).unwrap();
        assert!(!paths.provider.exists());
        assert!(removed
            .backup_files
            .iter()
            .any(|path| std::fs::read(path).unwrap() == previous_provider));
        for path in updated
            .backup_files
            .iter()
            .chain(removed.backup_files.iter())
        {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

/// Config key Goose reads for an extra root CA. Additive
/// (`ClientBuilder::add_root_certificate`), and resolvable from either the
/// environment or `config.yaml`.
pub const CA_CONFIG_KEY: &str = "GOOSE_CA_CERT_PATH";

/// Goose builds its HTTP clients with reqwest defaults, so `HTTPS_PROXY` is
/// honored for free — but there is **no** config-file or dotenv hook for it,
/// so the proxy env var can only be supplied at launch time.
pub fn proxy_oneoff_command(proxy_url: &str, ca_cert_path: &str) -> String {
    format!("HTTPS_PROXY={proxy_url} {CA_CONFIG_KEY}={ca_cert_path} goose")
}

/// Set `GOOSE_CA_CERT_PATH` in an existing `config.yaml` body, preserving
/// every other key.
///
/// Note this covers Goose's *provider* clients only. The ChatGPT-Codex
/// provider (the subscription path this proxy exists to inspect) builds a bare
/// reqwest client that ignores the TLS config entirely — for that traffic the
/// root CA has to be trusted in the OS store, which is why proxy setup for
/// Goose reports `requires_system_ca`.
pub fn merge_proxy_config(existing: &str, ca_cert_path: &str) -> Result<String, String> {
    let mut config: serde_yaml::Value = if existing.trim().is_empty() {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    } else {
        serde_yaml::from_str(existing).map_err(|e| format!("Failed to parse config.yaml: {e}"))?
    };
    if !config.is_mapping() {
        config = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    }
    if let serde_yaml::Value::Mapping(ref mut map) = config {
        map.insert(
            serde_yaml::Value::String(CA_CONFIG_KEY.to_string()),
            serde_yaml::Value::String(ca_cert_path.to_string()),
        );
    }
    serde_yaml::to_string(&config).map_err(|e| format!("Failed to serialize config.yaml: {e}"))
}

#[cfg(test)]
mod proxy_tests {
    use super::*;

    #[test]
    fn sets_ca_key_on_empty_config() {
        let out = merge_proxy_config("", "/ca.pem").unwrap();
        assert!(out.contains("GOOSE_CA_CERT_PATH: /ca.pem"));
    }

    #[test]
    fn preserves_existing_keys() {
        let existing = "GOOSE_PROVIDER: openai\nextensions:\n  localrouter:\n    enabled: true\n";
        let out = merge_proxy_config(existing, "/ca.pem").unwrap();
        assert!(out.contains("GOOSE_PROVIDER: openai"));
        assert!(out.contains("localrouter"));
        assert!(out.contains("GOOSE_CA_CERT_PATH: /ca.pem"));
    }

    #[test]
    fn replaces_previous_ca_path_without_duplicating() {
        let first = merge_proxy_config("", "/one.pem").unwrap();
        let second = merge_proxy_config(&first, "/two.pem").unwrap();
        assert_eq!(second.matches("GOOSE_CA_CERT_PATH").count(), 1);
        assert!(second.contains("/two.pem"));
        assert!(!second.contains("/one.pem"));
    }

    #[test]
    fn malformed_yaml_is_an_error_not_a_silent_overwrite() {
        // Clobbering a config we failed to parse would lose the user's
        // providers and extensions.
        let err = merge_proxy_config("::: not yaml :::", "/ca.pem");
        assert!(err.is_err());
    }

    #[test]
    fn oneoff_command_carries_proxy_and_ca() {
        let cmd = proxy_oneoff_command("http://p", "/ca.pem");
        assert_eq!(cmd, "HTTPS_PROXY=http://p GOOSE_CA_CERT_PATH=/ca.pem goose");
    }
}

/// Remove `GOOSE_CA_CERT_PATH` from config.yaml (undo). Returns the new body
/// and how many keys were actually removed — zero means nothing was
/// configured, so the caller can skip rewriting the file.
pub fn remove_proxy_config(existing: &str) -> Result<(String, usize), String> {
    if existing.trim().is_empty() {
        return Ok((existing.to_string(), 0));
    }
    let mut config: serde_yaml::Value =
        serde_yaml::from_str(existing).map_err(|e| format!("Failed to parse config.yaml: {e}"))?;
    let mut removed = 0usize;
    if let Some(map) = config.as_mapping_mut() {
        if map
            .remove(serde_yaml::Value::String(CA_CONFIG_KEY.to_string()))
            .is_some()
        {
            removed = 1;
        }
    }
    let out = serde_yaml::to_string(&config)
        .map_err(|e| format!("Failed to serialize config.yaml: {e}"))?;
    Ok((out, removed))
}

#[cfg(test)]
mod undo_tests {
    use super::*;

    #[test]
    fn removes_only_the_ca_key() {
        let applied = merge_proxy_config("GOOSE_PROVIDER: openai\n", "/ca.pem").unwrap();
        let (undone, removed) = remove_proxy_config(&applied).unwrap();
        assert_eq!(removed, 1);
        assert!(!undone.contains("GOOSE_CA_CERT_PATH"));
        assert!(undone.contains("GOOSE_PROVIDER: openai"));
    }

    #[test]
    fn reports_nothing_removed_when_never_configured() {
        // Drives the caller's "don't rewrite the file" path — rewriting a
        // YAML config we never touched would strip the user's comments.
        let (_, removed) = remove_proxy_config("GOOSE_PROVIDER: openai\n").unwrap();
        assert_eq!(removed, 0);
    }
}
