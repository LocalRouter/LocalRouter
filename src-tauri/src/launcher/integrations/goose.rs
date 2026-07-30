//! Goose integration
//!
//! Two modes:
//! - **Try It Out**: Terminal command with env vars (LLM only).
//! - **Permanent Config**: Write MCP extension to `~/.config/goose/config.yaml`.

use crate::launcher::backup;
use crate::launcher::{AppIntegration, ConfigSyncContext};
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};

pub struct GooseIntegration;

fn config_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".config")
        .join("goose")
        .join("config.yaml")
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

    fn try_it_out(
        &self,
        base_url: &str,
        client_secret: &str,
        _client_id: &str,
    ) -> Result<LaunchResult, String> {
        // LLM only via env vars
        Ok(LaunchResult {
            success: true,
            message: "Run the command below in your terminal:".to_string(),
            modified_files: vec![],
            backup_files: vec![],
            terminal_command: Some(format!(
                "OPENAI_BASE_URL={} OPENAI_API_KEY={} goose",
                base_url, client_secret
            )),
        })
    }

    fn configure_permanent(
        &self,
        base_url: &str,
        client_secret: &str,
        _client_id: &str,
    ) -> Result<LaunchResult, String> {
        let path = config_path();

        // Read existing YAML, merge our extension under `extensions`
        let mut config: serde_yaml::Value = if path.exists() {
            let data = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
            serde_yaml::from_str(&data)
                .unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
        } else {
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
        };

        // Build our extension entry
        let mut extension = serde_yaml::Mapping::new();
        extension.insert(
            serde_yaml::Value::String("type".to_string()),
            serde_yaml::Value::String("streamable_http".to_string()),
        );
        extension.insert(
            serde_yaml::Value::String("name".to_string()),
            serde_yaml::Value::String("LocalRouter".to_string()),
        );
        extension.insert(
            serde_yaml::Value::String("uri".to_string()),
            serde_yaml::Value::String(base_url.to_string()),
        );
        extension.insert(
            serde_yaml::Value::String("enabled".to_string()),
            serde_yaml::Value::Bool(true),
        );
        let mut headers = serde_yaml::Mapping::new();
        headers.insert(
            serde_yaml::Value::String("Authorization".to_string()),
            serde_yaml::Value::String(format!("Bearer {}", client_secret)),
        );
        extension.insert(
            serde_yaml::Value::String("headers".to_string()),
            serde_yaml::Value::Mapping(headers),
        );

        if let serde_yaml::Value::Mapping(ref mut map) = config {
            let extensions = map
                .entry(serde_yaml::Value::String("extensions".to_string()))
                .or_insert(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
            if let serde_yaml::Value::Mapping(ref mut ext_map) = extensions {
                ext_map.insert(
                    serde_yaml::Value::String("localrouter".to_string()),
                    serde_yaml::Value::Mapping(extension),
                );
            }
        }

        let data = serde_yaml::to_string(&config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        let backup_path = backup::write_with_backup(&path, data.as_bytes())?;
        let backup_files: Vec<String> = backup_path
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        Ok(LaunchResult {
            success: true,
            message: format!("MCP extension configured in {}", path.display()),
            modified_files: vec![path.to_string_lossy().to_string()],
            backup_files,
            terminal_command: None,
        })
    }

    fn sync_config(&self, ctx: &ConfigSyncContext) -> Result<LaunchResult, String> {
        // Goose permanent config only writes MCP extension entries.
        // In mcp_via_llm/llm_only modes, remove stale MCP entry (LLM uses env vars).
        if !ctx.should_sync_mcp() {
            let path = config_path();
            if path.exists() {
                let data = std::fs::read_to_string(&path)
                    .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
                let mut config: serde_yaml::Value = serde_yaml::from_str(&data)
                    .unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));

                let removed = if let serde_yaml::Value::Mapping(ref mut map) = config {
                    map.get_mut(serde_yaml::Value::String("extensions".to_string()))
                        .and_then(|ext| ext.as_mapping_mut())
                        .and_then(|ext_map| {
                            ext_map.remove(serde_yaml::Value::String("localrouter".to_string()))
                        })
                        .is_some()
                } else {
                    false
                };

                if removed {
                    let out = serde_yaml::to_string(&config)
                        .map_err(|e| format!("Failed to serialize config: {}", e))?;
                    let backup_path = backup::write_with_backup(&path, out.as_bytes())?;
                    return Ok(LaunchResult {
                        success: true,
                        message: format!("Removed MCP extension from {}", path.display()),
                        modified_files: vec![path.to_string_lossy().to_string()],
                        backup_files: backup_path
                            .iter()
                            .map(|p| p.to_string_lossy().to_string())
                            .collect(),
                        terminal_command: None,
                    });
                }
            }
            return Ok(LaunchResult {
                success: true,
                message: "No config to sync for current client mode (LLM uses env vars)"
                    .to_string(),
                modified_files: vec![],
                backup_files: vec![],
                terminal_command: None,
            });
        }
        self.configure_permanent(&ctx.base_url, &ctx.client_secret, &ctx.client_id)
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
