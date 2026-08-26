//! Pi (pi.dev) coding agent integration
//!
//! Config-file only (no try-it-out). Writes TWO files:
//! - **LLM**: `providers.localrouter` in `~/.pi/agent/models.json`
//! - **Defaults**: `defaultProvider` / `defaultModel` in `~/.pi/agent/settings.json`
//!   (only when LocalRouter is the sole provider, matching OpenClaw's sole-provider rule)
//!
//! Docs: https://pi.dev/docs/latest/models · https://pi.dev/docs/latest/settings

use crate::launcher::backup;
use crate::launcher::{AppIntegration, ConfigSyncContext};
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};
use std::path::{Path, PathBuf};

pub struct PiIntegration;

fn agent_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".pi")
        .join("agent")
}

fn models_path() -> PathBuf {
    agent_dir().join("models.json")
}

fn settings_path() -> PathBuf {
    agent_dir().join("settings.json")
}

/// Ensure `base_url` ends with `/v1` without doubling an existing suffix.
fn openai_compatible_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
}

fn read_json(path: &Path) -> serde_json::Value {
    if path.exists() {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_else(|| serde_json::json!({}))
    } else {
        serde_json::json!({})
    }
}

impl AppIntegration for PiIntegration {
    fn name(&self) -> &str {
        "Pi"
    }

    fn check_installed(&self) -> AppCapabilities {
        let binary = super::find_binary("pi");

        AppCapabilities {
            installed: binary.is_some(),
            binary_path: binary.map(|p| p.to_string_lossy().to_string()),
            version: None,
            supports_try_it_out: self.supports_try_it_out(),
            supports_permanent_config: self.supports_permanent_config(),
        }
    }

    fn supports_permanent_config(&self) -> bool {
        true
    }

    fn needs_model_list(&self) -> bool {
        true
    }

    fn configure_permanent(
        &self,
        base_url: &str,
        client_secret: &str,
        _client_id: &str,
    ) -> Result<LaunchResult, String> {
        self.write_config(base_url, client_secret, true, None)
    }

    fn sync_config(&self, ctx: &ConfigSyncContext) -> Result<LaunchResult, String> {
        self.write_config(
            &ctx.base_url,
            &ctx.client_secret,
            ctx.should_sync_llm(),
            Some(&ctx.models),
        )
    }
}

impl PiIntegration {
    fn write_config(
        &self,
        base_url: &str,
        client_secret: &str,
        sync_llm: bool,
        models: Option<&Vec<String>>,
    ) -> Result<LaunchResult, String> {
        write_config_at(
            &models_path(),
            &settings_path(),
            base_url,
            client_secret,
            sync_llm,
            models,
        )
    }
}

/// Core writer used by production paths and unit tests (injectable paths).
fn write_config_at(
    models_file: &Path,
    settings_file: &Path,
    base_url: &str,
    client_secret: &str,
    sync_llm: bool,
    models: Option<&Vec<String>>,
) -> Result<LaunchResult, String> {
    if !sync_llm && !models_file.exists() && !settings_file.exists() {
        return Ok(LaunchResult {
            success: true,
            message: "No config to sync for current client mode".to_string(),
            modified_files: vec![],
            backup_files: vec![],
            terminal_command: None,
        });
    }

    let mut modified_files = vec![];
    let mut all_backup_files = vec![];
    let mut parts = vec![];

    // --- models.json ---
    {
        let mut config = read_json(models_file);
        let obj = config
            .as_object_mut()
            .ok_or("Invalid Pi models.json format")?;
        let mut changed = false;

        if sync_llm {
            let providers = obj
                .entry("providers")
                .or_insert_with(|| serde_json::json!({}));

            let had_other_providers = providers
                .as_object()
                .map(|m| m.keys().any(|k| k != "localrouter"))
                .unwrap_or(false);

            let model_ids = models
                .cloned()
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| vec!["localrouter/auto".to_string()]);
            let default_model = model_ids
                .first()
                .cloned()
                .unwrap_or_else(|| "localrouter/auto".to_string());
            let model_entries: Vec<serde_json::Value> = model_ids
                .iter()
                .map(|id| {
                    serde_json::json!({
                        "id": id,
                        "name": id
                    })
                })
                .collect();

            let provider_entry = serde_json::json!({
                "baseUrl": openai_compatible_base_url(base_url),
                "api": "openai-completions",
                "apiKey": client_secret,
                "models": model_entries
            });

            if let Some(prov_obj) = providers.as_object_mut() {
                prov_obj.insert("localrouter".to_string(), provider_entry);
                changed = true;
            }

            parts.push(format!("LLM provider at {}", models_file.display()));

            // Persist models.json before touching settings so settings can
            // observe the provider map we just built.
            if changed {
                if let Some(parent) = models_file.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("Failed to create Pi agent dir: {e}"))?;
                }
                let data = serde_json::to_string_pretty(&config)
                    .map_err(|e| format!("Failed to serialize models.json: {e}"))?;
                let backup_path = backup::write_with_backup(models_file, data.as_bytes())?;
                modified_files.push(models_file.to_string_lossy().to_string());
                if let Some(bp) = backup_path {
                    all_backup_files.push(bp.to_string_lossy().to_string());
                }
            }

            // Defaults in settings.json
            sync_settings_defaults(
                settings_file,
                &default_model,
                had_other_providers,
                &mut modified_files,
                &mut all_backup_files,
                &mut parts,
            )?;
        } else {
            // Remove stale LLM entry
            if let Some(providers) = obj.get_mut("providers") {
                if let Some(prov_obj) = providers.as_object_mut() {
                    if prov_obj.remove("localrouter").is_some() {
                        changed = true;
                        parts.push(format!(
                            "removed LLM provider from {}",
                            models_file.display()
                        ));
                    }
                }
            }

            if changed {
                let data = serde_json::to_string_pretty(&config)
                    .map_err(|e| format!("Failed to serialize models.json: {e}"))?;
                let backup_path = backup::write_with_backup(models_file, data.as_bytes())?;
                modified_files.push(models_file.to_string_lossy().to_string());
                if let Some(bp) = backup_path {
                    all_backup_files.push(bp.to_string_lossy().to_string());
                }
            }

            // Clear LocalRouter defaults if we still own them
            clear_localrouter_defaults(
                settings_file,
                &mut modified_files,
                &mut all_backup_files,
                &mut parts,
            )?;
        }
    }

    if modified_files.is_empty() {
        return Ok(LaunchResult {
            success: true,
            message: "No config changes needed".to_string(),
            modified_files: vec![],
            backup_files: vec![],
            terminal_command: None,
        });
    }

    Ok(LaunchResult {
        success: true,
        message: format!("Configured Pi: {}", parts.join(", ")),
        modified_files,
        backup_files: all_backup_files,
        terminal_command: None,
    })
}

/// Set or refresh `defaultProvider` / `defaultModel` under the sole-provider rule.
fn sync_settings_defaults(
    settings_file: &Path,
    default_model: &str,
    had_other_providers: bool,
    modified_files: &mut Vec<String>,
    all_backup_files: &mut Vec<String>,
    parts: &mut Vec<String>,
) -> Result<(), String> {
    let mut settings = read_json(settings_file);
    let obj = settings
        .as_object_mut()
        .ok_or("Invalid Pi settings.json format")?;

    let current_provider = obj
        .get("defaultProvider")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Refresh when still on LocalRouter; claim defaults only when LocalRouter
    // is the sole provider (same rule as OpenClaw).
    let should_write = current_provider == "localrouter" || !had_other_providers;
    if !should_write {
        return Ok(());
    }

    obj.insert(
        "defaultProvider".to_string(),
        serde_json::json!("localrouter"),
    );
    obj.insert("defaultModel".to_string(), serde_json::json!(default_model));

    if let Some(parent) = settings_file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create Pi agent dir: {e}"))?;
    }
    let data = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("Failed to serialize settings.json: {e}"))?;
    let backup_path = backup::write_with_backup(settings_file, data.as_bytes())?;
    modified_files.push(settings_file.to_string_lossy().to_string());
    if let Some(bp) = backup_path {
        all_backup_files.push(bp.to_string_lossy().to_string());
    }
    parts.push(format!("defaults at {}", settings_file.display()));
    Ok(())
}

fn clear_localrouter_defaults(
    settings_file: &Path,
    modified_files: &mut Vec<String>,
    all_backup_files: &mut Vec<String>,
    parts: &mut Vec<String>,
) -> Result<(), String> {
    if !settings_file.exists() {
        return Ok(());
    }

    let mut settings = read_json(settings_file);
    let Some(obj) = settings.as_object_mut() else {
        return Ok(());
    };

    let is_ours = obj
        .get("defaultProvider")
        .and_then(|v| v.as_str())
        .is_some_and(|p| p == "localrouter");
    if !is_ours {
        return Ok(());
    }

    obj.remove("defaultProvider");
    obj.remove("defaultModel");

    let data = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("Failed to serialize settings.json: {e}"))?;
    let backup_path = backup::write_with_backup(settings_file, data.as_bytes())?;
    modified_files.push(settings_file.to_string_lossy().to_string());
    if let Some(bp) = backup_path {
        all_backup_files.push(bp.to_string_lossy().to_string());
    }
    parts.push(format!(
        "cleared LocalRouter defaults from {}",
        settings_file.display()
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn paths(dir: &tempfile::TempDir) -> (PathBuf, PathBuf) {
        let agent = dir.path().join("agent");
        std::fs::create_dir_all(&agent).unwrap();
        (agent.join("models.json"), agent.join("settings.json"))
    }

    #[test]
    fn openai_compatible_base_url_avoids_double_v1() {
        assert_eq!(
            openai_compatible_base_url("http://localhost:3625"),
            "http://localhost:3625/v1"
        );
        assert_eq!(
            openai_compatible_base_url("http://localhost:3625/"),
            "http://localhost:3625/v1"
        );
        assert_eq!(
            openai_compatible_base_url("http://localhost:3625/v1"),
            "http://localhost:3625/v1"
        );
        assert_eq!(
            openai_compatible_base_url("http://localhost:3625/v1/"),
            "http://localhost:3625/v1"
        );
    }

    #[test]
    fn sole_provider_sets_models_and_defaults() {
        let dir = tempdir().unwrap();
        let (models_file, settings_file) = paths(&dir);

        let result = write_config_at(
            &models_file,
            &settings_file,
            "http://localhost:3625",
            "secret",
            true,
            Some(&vec!["anthropic/claude-sonnet".to_string()]),
        )
        .unwrap();
        assert!(result.success);
        assert_eq!(result.modified_files.len(), 2);

        let models: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&models_file).unwrap()).unwrap();
        let provider = &models["providers"]["localrouter"];
        assert_eq!(provider["baseUrl"], "http://localhost:3625/v1");
        assert_eq!(provider["api"], "openai-completions");
        assert_eq!(provider["apiKey"], "secret");
        assert_eq!(provider["models"][0]["id"], "anthropic/claude-sonnet");

        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_file).unwrap()).unwrap();
        assert_eq!(settings["defaultProvider"], "localrouter");
        assert_eq!(settings["defaultModel"], "anthropic/claude-sonnet");
    }

    #[test]
    fn multi_provider_preserves_existing_defaults() {
        let dir = tempdir().unwrap();
        let (models_file, settings_file) = paths(&dir);

        std::fs::write(
            &models_file,
            r#"{
  "providers": {
    "ollama": {
      "baseUrl": "http://localhost:11434/v1",
      "api": "openai-completions",
      "apiKey": "ollama",
      "models": [{ "id": "llama3" }]
    }
  }
}"#,
        )
        .unwrap();
        std::fs::write(
            &settings_file,
            r#"{
  "defaultProvider": "ollama",
  "defaultModel": "llama3",
  "theme": "dark"
}"#,
        )
        .unwrap();

        write_config_at(
            &models_file,
            &settings_file,
            "http://localhost:3625",
            "secret",
            true,
            Some(&vec!["localrouter/auto".to_string()]),
        )
        .unwrap();

        let models: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&models_file).unwrap()).unwrap();
        assert!(models["providers"]["localrouter"].is_object());
        assert!(models["providers"]["ollama"].is_object());

        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_file).unwrap()).unwrap();
        assert_eq!(settings["defaultProvider"], "ollama");
        assert_eq!(settings["defaultModel"], "llama3");
        assert_eq!(settings["theme"], "dark");
    }

    #[test]
    fn refresh_default_model_when_still_localrouter() {
        let dir = tempdir().unwrap();
        let (models_file, settings_file) = paths(&dir);

        write_config_at(
            &models_file,
            &settings_file,
            "http://localhost:3625",
            "secret",
            true,
            Some(&vec!["model-a".to_string()]),
        )
        .unwrap();

        write_config_at(
            &models_file,
            &settings_file,
            "http://localhost:3625",
            "secret",
            true,
            Some(&vec!["model-b".to_string(), "model-c".to_string()]),
        )
        .unwrap();

        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_file).unwrap()).unwrap();
        assert_eq!(settings["defaultProvider"], "localrouter");
        assert_eq!(settings["defaultModel"], "model-b");

        let models: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&models_file).unwrap()).unwrap();
        assert_eq!(
            models["providers"]["localrouter"]["models"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn unsync_removes_provider_and_clears_defaults() {
        let dir = tempdir().unwrap();
        let (models_file, settings_file) = paths(&dir);

        write_config_at(
            &models_file,
            &settings_file,
            "http://localhost:3625",
            "secret",
            true,
            Some(&vec!["localrouter/auto".to_string()]),
        )
        .unwrap();

        write_config_at(
            &models_file,
            &settings_file,
            "http://localhost:3625",
            "secret",
            false,
            None,
        )
        .unwrap();

        let models: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&models_file).unwrap()).unwrap();
        assert!(models["providers"].get("localrouter").is_none());

        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_file).unwrap()).unwrap();
        assert!(settings.get("defaultProvider").is_none());
        assert!(settings.get("defaultModel").is_none());
    }

    #[test]
    fn merge_preserves_unrelated_models_json_keys() {
        let dir = tempdir().unwrap();
        let (models_file, settings_file) = paths(&dir);

        std::fs::write(&models_file, r#"{ "providers": {}, "customMeta": true }"#).unwrap();

        write_config_at(
            &models_file,
            &settings_file,
            "http://localhost:3625",
            "secret",
            true,
            None,
        )
        .unwrap();

        let models: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&models_file).unwrap()).unwrap();
        assert_eq!(models["customMeta"], true);
        assert_eq!(
            models["providers"]["localrouter"]["models"][0]["id"],
            "localrouter/auto"
        );
    }
}
