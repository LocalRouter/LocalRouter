//! Codex (OpenAI) integration
//!
//! Two modes:
//! - **Try It Out**: Terminal command with env vars (LLM routing).
//! - **Permanent Config**: Write MCP server entry to `~/.codex/config.toml`.
//!
//! See: <https://developers.openai.com/codex/config-reference/>

use crate::launcher::backup;
use crate::launcher::{AppIntegration, ConfigSyncContext};
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};

pub struct CodexIntegration;

/// Environment variable Codex reads for a custom root CA (its dedicated
/// `CODEX_CA_CERTIFICATE`, with `SSL_CERT_FILE` as codex's own fallback).
pub const PROXY_CA_ENV_VAR: &str = "CODEX_CA_CERTIFICATE";

/// One-off terminal command to launch Codex through the inspection proxy.
/// Codex's HTTP client honors `HTTPS_PROXY` (with embedded Basic auth) and
/// trusts our root CA via `CODEX_CA_CERTIFICATE`.
pub fn proxy_oneoff_command(proxy_url: &str, ca_cert_path: &str) -> String {
    format!("HTTPS_PROXY={proxy_url} {PROXY_CA_ENV_VAR}={ca_cert_path} codex")
}

/// Path to Codex's dotenv file. Codex loads `~/.codex/.env` at startup
/// (codex-rs `arg0::load_dotenv`) and exports every key into its process
/// environment — except keys prefixed `CODEX_`, which it filters out. That is
/// why permanent setup uses `SSL_CERT_FILE` (codex's documented custom-CA
/// fallback, applied on top of system roots) instead of `CODEX_CA_CERTIFICATE`.
pub fn env_file_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".codex")
        .join(".env")
}

/// The `.env` fragment that configures the proxy permanently.
pub fn proxy_env_fragment(proxy_url: &str, ca_cert_path: &str) -> String {
    format!("HTTPS_PROXY={proxy_url}\nSSL_CERT_FILE={ca_cert_path}\n")
}

/// Merge the proxy keys into an existing (or empty) `.env` file body,
/// preserving any other lines/comments the user already has. Existing
/// `HTTPS_PROXY` / `SSL_CERT_FILE` assignments are replaced in place;
/// missing ones are appended.
pub fn merge_proxy_env(existing: &str, proxy_url: &str, ca_cert_path: &str) -> String {
    let updates = [("HTTPS_PROXY", proxy_url), ("SSL_CERT_FILE", ca_cert_path)];

    let mut lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();
    for (key, value) in updates {
        let assignment = format!("{key}={value}");
        // Match `KEY=`, with optional leading whitespace and `export `.
        let is_key_line = |line: &str| {
            line.trim_start()
                .strip_prefix("export ")
                .unwrap_or(line.trim_start())
                .starts_with(&format!("{key}="))
        };
        // Replace the first assignment and drop any later duplicates —
        // dotenv gives the last line precedence, so a stale duplicate
        // below ours would silently win.
        let mut found = false;
        lines.retain_mut(|line| {
            if is_key_line(line) {
                if found {
                    return false;
                }
                *line = assignment.clone();
                found = true;
            }
            true
        });
        if !found {
            lines.push(assignment);
        }
    }

    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Path to Codex global config file
fn config_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".codex")
        .join("config.toml")
}

/// Read the existing config.toml or create an empty table
fn read_config(path: &std::path::Path) -> toml::Value {
    if path.exists() {
        let data = std::fs::read_to_string(path).unwrap_or_default();
        data.parse::<toml::Value>()
            .unwrap_or(toml::Value::Table(toml::map::Map::new()))
    } else {
        toml::Value::Table(toml::map::Map::new())
    }
}

/// Insert the LocalRouter MCP server entry into the config
fn insert_mcp_entry(config: &mut toml::Value, base_url: &str, client_secret: &str) {
    if let toml::Value::Table(ref mut table) = config {
        let mcp_servers = table
            .entry("mcp_servers")
            .or_insert(toml::Value::Table(toml::map::Map::new()));
        if let toml::Value::Table(ref mut servers) = mcp_servers {
            let mut entry = toml::map::Map::new();
            entry.insert("url".to_string(), toml::Value::String(base_url.to_string()));

            let mut headers = toml::map::Map::new();
            headers.insert(
                "Authorization".to_string(),
                toml::Value::String(format!("Bearer {}", client_secret)),
            );
            entry.insert("http_headers".to_string(), toml::Value::Table(headers));

            servers.insert("localrouter".to_string(), toml::Value::Table(entry));
        }
    }
}

/// Write config TOML to disk with backup
fn write_config(path: &std::path::Path, config: &toml::Value) -> Result<LaunchResult, String> {
    let data =
        toml::to_string_pretty(config).map_err(|e| format!("Failed to serialize config: {}", e))?;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }

    let backup_path = backup::write_with_backup(path, data.as_bytes())?;
    let backup_files: Vec<String> = backup_path
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    Ok(LaunchResult {
        success: true,
        message: format!(
            "MCP configured in {}. Restart Codex or run `codex mcp list` to verify.",
            path.display()
        ),
        modified_files: vec![path.to_string_lossy().to_string()],
        backup_files,
        terminal_command: None,
    })
}

impl AppIntegration for CodexIntegration {
    fn name(&self) -> &str {
        "Codex"
    }

    fn check_installed(&self) -> AppCapabilities {
        let binary = super::find_binary("codex");

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
                "OPENAI_BASE_URL={} OPENAI_API_KEY={} codex --oss",
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
        let mut config = read_config(&path);

        insert_mcp_entry(&mut config, base_url, client_secret);

        write_config(&path, &config)
    }

    fn sync_config(&self, ctx: &ConfigSyncContext) -> Result<LaunchResult, String> {
        // Codex permanent config only writes MCP entries.
        // In mcp_via_llm/llm_only modes, remove stale MCP entry (LLM uses env vars).
        if !ctx.should_sync_mcp() {
            let path = config_path();
            if path.exists() {
                let mut config = read_config(&path);
                let removed = if let toml::Value::Table(ref mut table) = config {
                    table
                        .get_mut("mcp_servers")
                        .and_then(|s| s.as_table_mut())
                        .and_then(|servers| servers.remove("localrouter"))
                        .is_some()
                } else {
                    false
                };

                if removed {
                    return write_config(&path, &config);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_proxy_oneoff_command() {
        let cmd = proxy_oneoff_command("http://cid:sec@127.0.0.1:3626", "/ca/root-ca.pem");
        assert_eq!(
            cmd,
            "HTTPS_PROXY=http://cid:sec@127.0.0.1:3626 CODEX_CA_CERTIFICATE=/ca/root-ca.pem codex"
        );
    }

    #[test]
    fn env_fragment_uses_ssl_cert_file() {
        // Codex filters CODEX_-prefixed keys out of ~/.codex/.env, so the
        // permanent fragment must use the SSL_CERT_FILE fallback.
        let frag = proxy_env_fragment("http://p", "/ca.pem");
        assert_eq!(frag, "HTTPS_PROXY=http://p\nSSL_CERT_FILE=/ca.pem\n");
        assert!(!frag.contains("CODEX_"));
    }

    #[test]
    fn merge_into_empty_creates_both_keys() {
        let merged = merge_proxy_env("", "http://p", "/ca.pem");
        assert_eq!(merged, "HTTPS_PROXY=http://p\nSSL_CERT_FILE=/ca.pem\n");
    }

    #[test]
    fn merge_preserves_other_lines_and_comments() {
        let existing = "# my env\nOPENAI_API_KEY=sk-123\n\nHTTPS_PROXY=http://old\n";
        let merged = merge_proxy_env(existing, "http://new", "/ca.pem");
        assert_eq!(
            merged,
            "# my env\nOPENAI_API_KEY=sk-123\n\nHTTPS_PROXY=http://new\nSSL_CERT_FILE=/ca.pem\n"
        );
    }

    #[test]
    fn merge_replaces_export_prefixed_assignment() {
        let existing = "export SSL_CERT_FILE=/old.pem\n";
        let merged = merge_proxy_env(existing, "http://p", "/new.pem");
        assert_eq!(merged, "SSL_CERT_FILE=/new.pem\nHTTPS_PROXY=http://p\n");
    }

    #[test]
    fn merge_drops_duplicate_assignments() {
        // dotenv gives the last line precedence — a stale duplicate below the
        // rewritten one must not survive.
        let existing = "HTTPS_PROXY=http://a\nFOO=bar\nHTTPS_PROXY=http://b\n";
        let merged = merge_proxy_env(existing, "http://new", "/ca.pem");
        assert_eq!(
            merged,
            "HTTPS_PROXY=http://new\nFOO=bar\nSSL_CERT_FILE=/ca.pem\n"
        );
    }
}
