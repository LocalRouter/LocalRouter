//! Pi (pi.dev) coding agent integration
//!
//! Config-file only (no try-it-out). Writes two files in Pi's agent directory
//! (`$PI_CODING_AGENT_DIR`, default `~/.pi/agent`):
//! - **LLM**: `providers.localrouter` in `models.json`
//! - **Defaults**: `defaultProvider` / `defaultModel` in `settings.json`, only
//!   while LocalRouter already owns them, or when no default provider is set
//!   and LocalRouter is the only custom provider. A default the user chose for
//!   another provider (including Pi's built-in ones) is never replaced.
//!
//! Docs: https://pi.dev/docs/latest/models · https://pi.dev/docs/latest/settings

use crate::launcher::backup;
use crate::launcher::{AppIntegration, ConfigSyncContext};
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub struct PiIntegration;

/// Provider key LocalRouter owns in `models.json` and `defaultProvider`.
const PROVIDER_ID: &str = "localrouter";

/// Model advertised when no model list is available (direct configure).
const FALLBACK_MODEL: &str = "localrouter/auto";

/// Files the integration reads and writes.
struct PiPaths {
    models: PathBuf,
    settings: PathBuf,
    backups: PathBuf,
}

impl PiPaths {
    fn user() -> Self {
        let agent = agent_dir();
        Self {
            models: agent.join("models.json"),
            settings: agent.join("settings.json"),
            backups: backup::default_backup_dir(),
        }
    }
}

/// Pi's agent directory: `$PI_CODING_AGENT_DIR` when set, else `~/.pi/agent`.
fn agent_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    resolve_agent_dir(std::env::var("PI_CODING_AGENT_DIR").ok().as_deref(), &home)
}

fn resolve_agent_dir(env_value: Option<&str>, home: &Path) -> PathBuf {
    match env_value.map(str::trim).filter(|dir| !dir.is_empty()) {
        Some("~") => home.to_path_buf(),
        Some(dir) => match dir.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(dir),
        },
        None => home.join(".pi").join("agent"),
    }
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

/// Read a JSON object, treating a missing file as empty. Unreadable,
/// malformed or non-object files are errors so they are never overwritten.
fn read_json(path: &Path) -> Result<Value, String> {
    match std::fs::read_to_string(path) {
        Ok(data) => super::config_parse::json(&data, path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(error) => Err(format!("Failed to read {}: {error}", path.display())),
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
        write_config_at(&PiPaths::user(), base_url, client_secret, true, None)
    }

    fn sync_config(&self, ctx: &ConfigSyncContext) -> Result<LaunchResult, String> {
        write_config_at(
            &PiPaths::user(),
            &ctx.base_url,
            &ctx.client_secret,
            ctx.should_sync_llm(),
            Some(ctx.models.as_slice()),
        )
    }
}

/// Files written by one configuration pass.
#[derive(Default)]
struct Written {
    modified_files: Vec<String>,
    backup_files: Vec<String>,
    parts: Vec<String>,
}

impl Written {
    fn write(
        &mut self,
        paths: &PiPaths,
        path: &Path,
        value: &Value,
        part: String,
    ) -> Result<(), String> {
        let data = serde_json::to_string_pretty(value)
            .map_err(|e| format!("Failed to serialize {}: {e}", path.display()))?;
        let backup_path = backup::write_with_backup_in(path, data.as_bytes(), &paths.backups)?;
        self.modified_files.push(path.to_string_lossy().to_string());
        if let Some(bp) = backup_path {
            self.backup_files.push(bp.to_string_lossy().to_string());
        }
        self.parts.push(part);
        Ok(())
    }
}

/// Core writer used by production paths and unit tests (injectable paths).
fn write_config_at(
    paths: &PiPaths,
    base_url: &str,
    client_secret: &str,
    sync_llm: bool,
    models: Option<&[String]>,
) -> Result<LaunchResult, String> {
    let no_changes = |message: &str| LaunchResult {
        success: true,
        message: message.to_string(),
        modified_files: vec![],
        backup_files: vec![],
        terminal_command: None,
    };

    if !sync_llm && !paths.models.exists() && !paths.settings.exists() {
        return Ok(no_changes("No config to sync for current client mode"));
    }

    // Parse both files before writing either, so a malformed settings.json
    // cannot leave models.json half-configured.
    let mut models_config = read_json(&paths.models)?;
    let mut settings = read_json(&paths.settings)?;
    let mut written = Written::default();

    if sync_llm {
        let original = models_config.clone();
        let invalid_providers = || {
            format!(
                "Expected \"providers\" to be an object in {}",
                paths.models.display()
            )
        };
        let providers = models_config
            .as_object_mut()
            .ok_or_else(invalid_providers)?
            .entry("providers")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(invalid_providers)?;

        let had_other_providers = providers.keys().any(|k| k != PROVIDER_ID);

        let model_ids: Vec<String> = match models {
            Some(list) if !list.is_empty() => list.to_vec(),
            _ => vec![FALLBACK_MODEL.to_string()],
        };
        let model_entries: Vec<Value> = model_ids
            .iter()
            .map(|id| json!({ "id": id, "name": id }))
            .collect();

        providers.insert(
            PROVIDER_ID.to_string(),
            json!({
                "baseUrl": openai_compatible_base_url(base_url),
                "api": "openai-completions",
                "apiKey": client_secret,
                "models": model_entries
            }),
        );

        if models_config != original {
            written.write(
                paths,
                &paths.models,
                &models_config,
                format!("LLM provider at {}", paths.models.display()),
            )?;
        }

        if claim_defaults(&mut settings, &model_ids, had_other_providers) {
            written.write(
                paths,
                &paths.settings,
                &settings,
                format!("defaults at {}", paths.settings.display()),
            )?;
        }
    } else {
        let removed = models_config
            .get_mut("providers")
            .and_then(Value::as_object_mut)
            .is_some_and(|providers| providers.remove(PROVIDER_ID).is_some());
        if removed {
            written.write(
                paths,
                &paths.models,
                &models_config,
                format!("removed LLM provider from {}", paths.models.display()),
            )?;
        }

        if clear_defaults(&mut settings) {
            written.write(
                paths,
                &paths.settings,
                &settings,
                format!(
                    "cleared LocalRouter defaults from {}",
                    paths.settings.display()
                ),
            )?;
        }
    }

    if written.modified_files.is_empty() {
        return Ok(no_changes("No config changes needed"));
    }

    Ok(LaunchResult {
        success: true,
        message: format!("Configured Pi: {}", written.parts.join(", ")),
        modified_files: written.modified_files,
        backup_files: written.backup_files,
        terminal_command: None,
    })
}

/// Who currently owns Pi's startup default provider.
enum DefaultOwner {
    Unset,
    LocalRouter,
    Other,
}

fn default_owner(settings: &serde_json::Map<String, Value>) -> DefaultOwner {
    match settings.get("defaultProvider") {
        None | Some(Value::Null) => DefaultOwner::Unset,
        Some(Value::String(provider)) if provider.is_empty() => DefaultOwner::Unset,
        Some(Value::String(provider)) if provider == PROVIDER_ID => DefaultOwner::LocalRouter,
        Some(_) => DefaultOwner::Other,
    }
}

/// Point Pi's startup defaults at LocalRouter when allowed. Keeps a
/// LocalRouter model the user picked as long as it is still offered.
/// Returns whether `settings` changed.
fn claim_defaults(settings: &mut Value, model_ids: &[String], had_other_providers: bool) -> bool {
    let Some(obj) = settings.as_object_mut() else {
        return false;
    };
    let owned = match default_owner(obj) {
        DefaultOwner::LocalRouter => true,
        DefaultOwner::Unset if !had_other_providers => false,
        DefaultOwner::Unset | DefaultOwner::Other => return false,
    };

    let current_model_offered = obj
        .get("defaultModel")
        .and_then(Value::as_str)
        .is_some_and(|model| model_ids.iter().any(|id| id == model));

    let mut changed = false;
    if !owned {
        obj.insert("defaultProvider".to_string(), json!(PROVIDER_ID));
        changed = true;
    }
    if !(owned && current_model_offered) {
        if let Some(first) = model_ids.first() {
            obj.insert("defaultModel".to_string(), json!(first));
            changed = true;
        }
    }
    changed
}

/// Remove Pi's startup defaults if they still point at LocalRouter.
/// Returns whether `settings` changed.
fn clear_defaults(settings: &mut Value) -> bool {
    let Some(obj) = settings.as_object_mut() else {
        return false;
    };
    if !matches!(default_owner(obj), DefaultOwner::LocalRouter) {
        return false;
    }
    obj.remove("defaultProvider");
    obj.remove("defaultModel");
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{tempdir, TempDir};

    fn paths(dir: &TempDir) -> PiPaths {
        let agent = dir.path().join("agent");
        std::fs::create_dir_all(&agent).unwrap();
        PiPaths {
            models: agent.join("models.json"),
            settings: agent.join("settings.json"),
            backups: dir.path().join("backups"),
        }
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn models(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    fn sync(paths: &PiPaths, ids: &[&str]) -> Result<LaunchResult, String> {
        write_config_at(
            paths,
            "http://localhost:3625",
            "secret",
            true,
            Some(models(ids).as_slice()),
        )
    }

    fn unsync(paths: &PiPaths) -> Result<LaunchResult, String> {
        write_config_at(paths, "http://localhost:3625", "secret", false, None)
    }

    #[test]
    fn agent_dir_honours_pi_coding_agent_dir() {
        let home = Path::new("/home/user");
        assert_eq!(
            resolve_agent_dir(None, home),
            PathBuf::from("/home/user/.pi/agent")
        );
        assert_eq!(
            resolve_agent_dir(Some("  "), home),
            PathBuf::from("/home/user/.pi/agent")
        );
        assert_eq!(
            resolve_agent_dir(Some("/opt/pi"), home),
            PathBuf::from("/opt/pi")
        );
        assert_eq!(
            resolve_agent_dir(Some("~/pi-agent"), home),
            PathBuf::from("/home/user/pi-agent")
        );
        assert_eq!(
            resolve_agent_dir(Some("~"), home),
            PathBuf::from("/home/user")
        );
    }

    #[test]
    fn openai_compatible_base_url_avoids_double_v1() {
        for input in [
            "http://localhost:3625",
            "http://localhost:3625/",
            "http://localhost:3625/v1",
            "http://localhost:3625/v1/",
        ] {
            assert_eq!(
                openai_compatible_base_url(input),
                "http://localhost:3625/v1"
            );
        }
    }

    #[test]
    fn sole_provider_sets_models_and_defaults() {
        let dir = tempdir().unwrap();
        let paths = paths(&dir);

        let result = sync(&paths, &["anthropic/claude-sonnet"]).unwrap();
        assert!(result.success);
        assert_eq!(result.modified_files.len(), 2);

        let provider = &read(&paths.models)["providers"]["localrouter"];
        assert_eq!(provider["baseUrl"], "http://localhost:3625/v1");
        assert_eq!(provider["api"], "openai-completions");
        assert_eq!(provider["apiKey"], "secret");
        assert_eq!(provider["models"][0]["id"], "anthropic/claude-sonnet");

        let settings = read(&paths.settings);
        assert_eq!(settings["defaultProvider"], "localrouter");
        assert_eq!(settings["defaultModel"], "anthropic/claude-sonnet");
    }

    #[test]
    fn multi_provider_preserves_existing_defaults() {
        let dir = tempdir().unwrap();
        let paths = paths(&dir);

        std::fs::write(
            &paths.models,
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
            &paths.settings,
            r#"{
  "defaultProvider": "ollama",
  "defaultModel": "llama3",
  "theme": "dark"
}"#,
        )
        .unwrap();

        let result = sync(&paths, &["auto"]).unwrap();
        assert_eq!(result.modified_files.len(), 1);
        assert_eq!(result.backup_files.len(), 1);

        let models = read(&paths.models);
        assert!(models["providers"]["localrouter"].is_object());
        assert_eq!(models["providers"]["ollama"]["models"][0]["id"], "llama3");

        let settings = read(&paths.settings);
        assert_eq!(settings["defaultProvider"], "ollama");
        assert_eq!(settings["defaultModel"], "llama3");
        assert_eq!(settings["theme"], "dark");
    }

    /// Pi's built-in providers (signed in through `/login` or environment
    /// variables) never appear in models.json, so an explicit default for one
    /// must be respected even when LocalRouter is the only custom provider.
    #[test]
    fn built_in_provider_default_is_not_replaced() {
        let dir = tempdir().unwrap();
        let paths = paths(&dir);
        std::fs::write(
            &paths.settings,
            r#"{ "defaultProvider": "anthropic", "defaultModel": "claude-sonnet-5" }"#,
        )
        .unwrap();

        let result = sync(&paths, &["auto"]).unwrap();
        assert_eq!(
            result.modified_files,
            vec![paths.models.to_string_lossy().to_string()]
        );

        let settings = read(&paths.settings);
        assert_eq!(settings["defaultProvider"], "anthropic");
        assert_eq!(settings["defaultModel"], "claude-sonnet-5");
    }

    #[test]
    fn unset_default_with_other_custom_providers_is_left_unset() {
        let dir = tempdir().unwrap();
        let paths = paths(&dir);
        std::fs::write(
            &paths.models,
            r#"{ "providers": { "ollama": { "models": [] } } }"#,
        )
        .unwrap();

        sync(&paths, &["auto"]).unwrap();
        assert!(!paths.settings.exists());
    }

    #[test]
    fn refresh_default_model_when_previous_one_is_gone() {
        let dir = tempdir().unwrap();
        let paths = paths(&dir);

        sync(&paths, &["model-a"]).unwrap();
        sync(&paths, &["model-b", "model-c"]).unwrap();

        let settings = read(&paths.settings);
        assert_eq!(settings["defaultProvider"], "localrouter");
        assert_eq!(settings["defaultModel"], "model-b");
        assert_eq!(
            read(&paths.models)["providers"]["localrouter"]["models"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn user_chosen_localrouter_model_survives_resync() {
        let dir = tempdir().unwrap();
        let paths = paths(&dir);
        std::fs::write(
            &paths.settings,
            r#"{ "defaultProvider": "localrouter", "defaultModel": "model-c" }"#,
        )
        .unwrap();

        sync(&paths, &["model-b", "model-c"]).unwrap();
        assert_eq!(read(&paths.settings)["defaultModel"], "model-c");
    }

    #[test]
    fn identical_resync_reports_no_changes() {
        let dir = tempdir().unwrap();
        let paths = paths(&dir);

        sync(&paths, &["model-a"]).unwrap();
        let result = sync(&paths, &["model-a"]).unwrap();
        assert!(result.modified_files.is_empty());
        assert!(result.backup_files.is_empty());
        assert_eq!(result.message, "No config changes needed");
    }

    #[test]
    fn missing_model_list_falls_back_to_auto() {
        let dir = tempdir().unwrap();
        let paths = paths(&dir);

        write_config_at(&paths, "http://localhost:3625", "secret", true, None).unwrap();
        assert_eq!(
            read(&paths.models)["providers"]["localrouter"]["models"][0]["id"],
            FALLBACK_MODEL
        );

        sync(&paths, &[]).unwrap();
        assert_eq!(
            read(&paths.models)["providers"]["localrouter"]["models"][0]["id"],
            FALLBACK_MODEL
        );
        assert_eq!(read(&paths.settings)["defaultModel"], FALLBACK_MODEL);
    }

    #[test]
    fn unsync_removes_provider_and_clears_defaults() {
        let dir = tempdir().unwrap();
        let paths = paths(&dir);
        std::fs::write(&paths.models, r#"{ "providers": { "ollama": {} } }"#).unwrap();
        std::fs::write(
            &paths.settings,
            r#"{ "theme": "dark", "defaultProvider": "localrouter", "defaultModel": "auto" }"#,
        )
        .unwrap();
        sync(&paths, &["auto"]).unwrap();

        let result = unsync(&paths).unwrap();
        assert_eq!(result.modified_files.len(), 2);

        let models = read(&paths.models);
        assert!(models["providers"].get("localrouter").is_none());
        assert!(models["providers"]["ollama"].is_object());

        let settings = read(&paths.settings);
        assert!(settings.get("defaultProvider").is_none());
        assert!(settings.get("defaultModel").is_none());
        assert_eq!(settings["theme"], "dark");
    }

    #[test]
    fn unsync_keeps_defaults_owned_by_another_provider() {
        let dir = tempdir().unwrap();
        let paths = paths(&dir);
        sync(&paths, &["auto"]).unwrap();
        std::fs::write(
            &paths.settings,
            r#"{ "defaultProvider": "anthropic", "defaultModel": "claude-sonnet-5" }"#,
        )
        .unwrap();

        let result = unsync(&paths).unwrap();
        assert_eq!(
            result.modified_files,
            vec![paths.models.to_string_lossy().to_string()]
        );
        assert_eq!(read(&paths.settings)["defaultProvider"], "anthropic");
    }

    #[test]
    fn unsync_without_files_creates_nothing() {
        let dir = tempdir().unwrap();
        let paths = paths(&dir);

        let result = unsync(&paths).unwrap();
        assert!(result.modified_files.is_empty());
        assert!(!paths.models.exists());
        assert!(!paths.settings.exists());

        // settings.json alone (no models.json) must not create models.json.
        std::fs::write(&paths.settings, r#"{ "theme": "dark" }"#).unwrap();
        let result = unsync(&paths).unwrap();
        assert!(result.modified_files.is_empty());
        assert!(!paths.models.exists());
    }

    #[test]
    fn merge_preserves_unrelated_models_json_keys() {
        let dir = tempdir().unwrap();
        let paths = paths(&dir);
        std::fs::write(
            &paths.models,
            r#"{ "providers": {}, "modelOverrides": { "x": {} }, "customMeta": true }"#,
        )
        .unwrap();

        sync(&paths, &["auto"]).unwrap();

        let models = read(&paths.models);
        assert_eq!(models["customMeta"], true);
        assert!(models["modelOverrides"]["x"].is_object());
        assert_eq!(
            models["providers"]["localrouter"]["models"][0]["id"],
            "auto"
        );
    }

    #[test]
    fn malformed_files_are_never_overwritten() {
        for (models_body, settings_body) in [
            ("{ not json", "{}"),
            ("[]", "{}"),
            (r#"{ "providers": [] }"#, "{}"),
            ("{}", "{ not json"),
            ("{}", "\"text\""),
        ] {
            let dir = tempdir().unwrap();
            let paths = paths(&dir);
            std::fs::write(&paths.models, models_body).unwrap();
            std::fs::write(&paths.settings, settings_body).unwrap();

            assert!(
                sync(&paths, &["auto"]).is_err(),
                "accepted models={models_body:?} settings={settings_body:?}"
            );
            assert_eq!(std::fs::read_to_string(&paths.models).unwrap(), models_body);
            assert_eq!(
                std::fs::read_to_string(&paths.settings).unwrap(),
                settings_body
            );
            assert!(!paths.backups.exists());
        }
    }

    #[test]
    fn unsync_rejects_malformed_files() {
        let dir = tempdir().unwrap();
        let paths = paths(&dir);
        std::fs::write(&paths.models, "{ not json").unwrap();

        assert!(unsync(&paths).is_err());
        assert_eq!(
            std::fs::read_to_string(&paths.models).unwrap(),
            "{ not json"
        );
    }
}
