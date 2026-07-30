//! Aider integration
//!
//! Two modes:
//! - **Try It Out**: Terminal command with env vars (LLM only, no MCP support).
//! - **Permanent Config**: Write LLM settings to `~/.aider.conf.yml`.

use crate::launcher::backup;
use crate::launcher::{AppIntegration, ConfigSyncContext};
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};

pub struct AiderIntegration;

fn config_path() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_default().join(".aider.conf.yml")
}

/// Environment variable Aider's stack reads for a custom CA.
///
/// Aider routes LLM calls through litellm → httpx/aiohttp, which read
/// `SSL_CERT_FILE`; its non-LLM traffic (version check, model metadata) uses
/// `requests`, which reads `REQUESTS_CA_BUNDLE`. **Both replace the default
/// certifi bundle rather than extending it**, which is why proxy setup points
/// them at a combined bundle (see `launcher::ca_bundle`) instead of the bare
/// root CA.
pub const PROXY_CA_ENV_VAR: &str = "SSL_CERT_FILE";

/// Aider loads `.env` from the home directory, the git repo root, and the cwd
/// (later wins). The home-directory file is the one that applies to every
/// invocation, so that is what permanent proxy setup writes.
///
/// Note this file is a shared user namespace — not Aider-private — so the
/// merge is line-preserving and only touches the keys we own.
pub fn env_file_path() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_default().join(".env")
}

/// Keys we own in `~/.env`.
pub fn proxy_env_keys() -> [&'static str; 4] {
    [
        "HTTPS_PROXY",
        "SSL_CERT_FILE",
        "REQUESTS_CA_BUNDLE",
        "AIOHTTP_TRUST_ENV",
    ]
}

/// The `.env` fragment configuring the proxy permanently.
///
/// `bundle_path` must be a **combined** CA bundle (system/certifi roots + our
/// root CA); passing the bare root CA here would narrow Aider's trust store to
/// only hosts the proxy intercepts.
pub fn proxy_env_fragment(proxy_url: &str, bundle_path: &str) -> String {
    format!(
        "HTTPS_PROXY={proxy_url}\nSSL_CERT_FILE={bundle_path}\nREQUESTS_CA_BUNDLE={bundle_path}\nAIOHTTP_TRUST_ENV=1\n"
    )
}

/// One-off terminal command to launch Aider through the inspection proxy.
pub fn proxy_oneoff_command(proxy_url: &str, bundle_path: &str) -> String {
    format!(
        "HTTPS_PROXY={proxy_url} SSL_CERT_FILE={bundle_path} REQUESTS_CA_BUNDLE={bundle_path} aider"
    )
}

impl AppIntegration for AiderIntegration {
    fn name(&self) -> &str {
        "Aider"
    }

    fn check_installed(&self) -> AppCapabilities {
        let binary = super::find_binary("aider");

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
        Ok(LaunchResult {
            success: true,
            message: "Run the command below in your terminal:".to_string(),
            modified_files: vec![],
            backup_files: vec![],
            terminal_command: Some(format!(
                "OPENAI_API_BASE={} OPENAI_API_KEY={} aider",
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

        // Read existing YAML, merge our keys, write back
        let mut config: serde_yaml::Value = if path.exists() {
            let data = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
            serde_yaml::from_str(&data)
                .unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
        } else {
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
        };

        if let serde_yaml::Value::Mapping(ref mut map) = config {
            map.insert(
                serde_yaml::Value::String("openai-api-base".to_string()),
                serde_yaml::Value::String(base_url.to_string()),
            );
            map.insert(
                serde_yaml::Value::String("openai-api-key".to_string()),
                serde_yaml::Value::String(client_secret.to_string()),
            );
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
            message: format!("Configured Aider at {}", path.display()),
            modified_files: vec![path.to_string_lossy().to_string()],
            backup_files,
            terminal_command: None,
        })
    }

    fn sync_config(&self, ctx: &ConfigSyncContext) -> Result<LaunchResult, String> {
        // Aider only writes LLM config (no MCP support).
        // In mcp_only mode, remove stale LLM entries.
        if !ctx.should_sync_llm() {
            let path = config_path();
            if path.exists() {
                let data = std::fs::read_to_string(&path)
                    .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
                let mut config: serde_yaml::Value = serde_yaml::from_str(&data)
                    .unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));

                let mut removed = false;
                if let serde_yaml::Value::Mapping(ref mut map) = config {
                    removed |= map
                        .remove(serde_yaml::Value::String("openai-api-base".to_string()))
                        .is_some();
                    removed |= map
                        .remove(serde_yaml::Value::String("openai-api-key".to_string()))
                        .is_some();
                }

                if removed {
                    let out = serde_yaml::to_string(&config)
                        .map_err(|e| format!("Failed to serialize config: {}", e))?;
                    let backup_path = backup::write_with_backup(&path, out.as_bytes())?;
                    return Ok(LaunchResult {
                        success: true,
                        message: format!("Removed LLM config from {}", path.display()),
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
                message: "No config to sync for current client mode (Aider has no MCP support)"
                    .to_string(),
                modified_files: vec![],
                backup_files: vec![],
                terminal_command: None,
            });
        }
        self.configure_permanent(&ctx.base_url, &ctx.client_secret, &ctx.client_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_fragment_points_both_python_ca_vars_at_the_same_bundle() {
        let frag = proxy_env_fragment("http://p", "/combined.pem");
        assert!(frag.contains("SSL_CERT_FILE=/combined.pem"));
        assert!(frag.contains("REQUESTS_CA_BUNDLE=/combined.pem"));
        assert!(frag.contains("HTTPS_PROXY=http://p"));
        assert!(frag.contains("AIOHTTP_TRUST_ENV=1"));
    }

    /// The keys the proxy plan writes for Aider, through the shared merge.
    fn merge(existing: &str, url: &str, bundle: &str) -> String {
        super::super::dotenv::merge_env(
            existing,
            &[
                ("HTTPS_PROXY", url),
                ("SSL_CERT_FILE", bundle),
                ("REQUESTS_CA_BUNDLE", bundle),
                ("AIOHTTP_TRUST_ENV", "1"),
            ],
        )
    }

    #[test]
    fn merge_preserves_user_secrets_in_shared_home_env() {
        // ~/.env is not Aider-private; unrelated keys must survive untouched.
        let existing = "# personal\nAWS_PROFILE=work\nOPENAI_API_KEY=sk-1\n";
        let merged = merge(existing, "http://p", "/combined.pem");
        assert!(merged.starts_with("# personal\nAWS_PROFILE=work\nOPENAI_API_KEY=sk-1\n"));
        assert!(merged.contains("HTTPS_PROXY=http://p"));
    }

    #[test]
    fn merge_replaces_previous_proxy_values_without_duplicating() {
        let first = merge("", "http://a", "/one.pem");
        let second = merge(&first, "http://b", "/two.pem");
        assert_eq!(second.matches("HTTPS_PROXY=").count(), 1);
        assert_eq!(second.matches("SSL_CERT_FILE=").count(), 1);
        assert!(second.contains("HTTPS_PROXY=http://b"));
        assert!(second.contains("SSL_CERT_FILE=/two.pem"));
        assert!(!second.contains("/one.pem"));
    }
}
