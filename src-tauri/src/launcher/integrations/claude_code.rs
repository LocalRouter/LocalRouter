//! Claude Code integration
//!
//! Two modes:
//! - **Try It Out**: Terminal command with env vars + inline `--mcp-config` JSON. Zero file changes.
//! - **Permanent Config**: Write MCP server entry to `~/.claude.json`.
//!
//! # Why the MCP entry is the STDIO bridge, not HTTP
//!
//! Claude Code receives pushed events (Claude Code Channels) only from a stdio
//! MCP server that uses the handshake from before MCP revision 2026-07-28.
//! LocalRouter over HTTP can still serve tools, but cannot wake Claude when an
//! event arrives. The STDIO bridge runs `localrouter --mcp-bridge` as Claude's
//! subprocess and relays everything, including pushed events, so new setups
//! write a stdio entry and Claude is launched with
//! [`CHANNELS_LAUNCH_COMMAND`]. That flag is interactive-only, an
//! organisation's `channelsEnabled` policy can block it, and a newly added
//! push-capable upstream needs a `claude` restart.
//!
//! The HTTP entry remains available (tools only). An existing HTTP entry is
//! never rewritten to stdio behind the user's back: syncing and re-applying
//! keep it HTTP, and only the explicit [`switch_to_stdio_bridge`] action
//! converts it. See `plan/2026-10-30-PUSH_RELAY.md`, "Claude Code: STDIO
//! bridge by default".

use crate::launcher::backup;
use crate::launcher::{AppIntegration, ConfigSyncContext};
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};

pub struct ClaudeCodeIntegration;

/// Name of LocalRouter's entry in Claude Code's `mcpServers`.
pub const MCP_SERVER_NAME: &str = "localrouter";

/// How Claude Code must be started for channel messages from the STDIO bridge
/// to reach the agent (Channels is a research preview behind this flag).
pub const CHANNELS_LAUNCH_COMMAND: &str =
    "claude --dangerously-load-development-channels server:localrouter";

/// Flag appended to a `claude` invocation to load LocalRouter's channel.
const CHANNELS_FLAG: &str = "--dangerously-load-development-channels server:localrouter";

/// Which kind of `localrouter` MCP entry Claude Code's config holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeMcpTransport {
    /// No `localrouter` entry.
    None,
    /// Streamable HTTP: tools only, no pushed events.
    Http,
    /// `localrouter --mcp-bridge` over stdio: tools and Channels.
    StdioBridge,
    /// Something else under the `localrouter` name (hand-edited).
    Other,
}

/// The HTTP MCP entry (tools only).
pub fn http_entry(base_url: &str, client_secret: &str) -> serde_json::Value {
    // Include token in URL query param as fallback for clients that don't send custom headers
    serde_json::json!({
        "type": "http",
        "url": format!("{}?token={}", base_url, client_secret),
        "headers": {
            "Authorization": format!("Bearer {}", client_secret)
        }
    })
}

/// The STDIO bridge MCP entry: Claude runs LocalRouter as a subprocess, which
/// is the only way it accepts Channels (see the module docs).
pub fn stdio_bridge_entry(binary: &str, client_id: &str, client_secret: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "stdio",
        "command": binary,
        "args": ["--mcp-bridge", "--client-id", client_id],
        "env": { "LOCALROUTER_CLIENT_SECRET": client_secret }
    })
}

/// Classify the `localrouter` entry in a parsed `~/.claude.json`.
pub fn localrouter_entry_transport(config: &serde_json::Value) -> ClaudeMcpTransport {
    let Some(entry) = config
        .get("mcpServers")
        .and_then(|s| s.get(MCP_SERVER_NAME))
    else {
        return ClaudeMcpTransport::None;
    };
    let kind = entry.get("type").and_then(|t| t.as_str());
    let is_bridge = entry
        .get("args")
        .and_then(|a| a.as_array())
        .is_some_and(|args| args.iter().any(|a| a.as_str() == Some("--mcp-bridge")));
    match kind {
        Some("http") | Some("sse") => ClaudeMcpTransport::Http,
        Some("stdio") | None if is_bridge => ClaudeMcpTransport::StdioBridge,
        None if entry.get("url").is_some() => ClaudeMcpTransport::Http,
        _ => ClaudeMcpTransport::Other,
    }
}

/// Insert or replace the `localrouter` entry, keeping everything else.
pub fn upsert_localrouter_entry(
    mut config: serde_json::Value,
    entry: serde_json::Value,
) -> serde_json::Value {
    if !config.is_object() {
        config = serde_json::json!({});
    }
    if let Some(obj) = config.as_object_mut() {
        let mcp_servers = obj
            .entry("mcpServers")
            .or_insert_with(|| serde_json::json!({}));
        if !mcp_servers.is_object() {
            *mcp_servers = serde_json::json!({});
        }
        if let Some(servers_obj) = mcp_servers.as_object_mut() {
            servers_obj.insert(MCP_SERVER_NAME.to_string(), entry);
        }
    }
    config
}

/// The entry `configure_permanent` / sync writes: the STDIO bridge, unless
/// the config already holds an HTTP entry, which is refreshed as HTTP so an
/// existing setup is never switched silently.
pub fn entry_for_existing(
    existing: &serde_json::Value,
    binary: &str,
    base_url: &str,
    client_id: &str,
    client_secret: &str,
) -> serde_json::Value {
    match localrouter_entry_transport(existing) {
        ClaudeMcpTransport::Http => http_entry(base_url, client_secret),
        _ => stdio_bridge_entry(binary, client_id, client_secret),
    }
}

/// Path of the running LocalRouter binary, which Claude launches with
/// `--mcp-bridge`.
pub fn localrouter_binary() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("Failed to resolve the LocalRouter binary path: {}", e))
}

fn read_mcp_config(path: &std::path::Path) -> Result<serde_json::Value, String> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let data = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    super::config_parse::json(&data, path)
}

/// Which `localrouter` entry `~/.claude.json` currently holds.
pub fn installed_transport() -> Result<ClaudeMcpTransport, String> {
    Ok(localrouter_entry_transport(&read_mcp_config(
        &mcp_config_path(),
    )?))
}

/// Path of `~/.claude.json`, for display.
pub fn mcp_config_display_path() -> String {
    mcp_config_path().to_string_lossy().to_string()
}

fn write_mcp_entry(entry: serde_json::Value) -> Result<(std::path::PathBuf, Vec<String>), String> {
    let path = mcp_config_path();
    let config = upsert_localrouter_entry(read_mcp_config(&path)?, entry);
    let data = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    let backup_path = backup::write_with_backup(&path, data.as_bytes())?;
    let backup_files = backup_path
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    Ok((path, backup_files))
}

/// The explicit "Switch to STDIO bridge" action: replace whatever
/// `localrouter` entry exists with the bridge.
pub fn switch_to_stdio_bridge(
    client_id: &str,
    client_secret: &str,
) -> Result<LaunchResult, String> {
    let binary = localrouter_binary()?;
    let (path, backup_files) =
        write_mcp_entry(stdio_bridge_entry(&binary, client_id, client_secret))?;
    Ok(LaunchResult {
        success: true,
        message: format!(
            "Switched LocalRouter in {} to the STDIO bridge. Restart Claude Code with the command below to receive pushed events.",
            path.display()
        ),
        modified_files: vec![path.to_string_lossy().to_string()],
        backup_files,
        terminal_command: Some(CHANNELS_LAUNCH_COMMAND.to_string()),
    })
}

/// Path to Claude's global settings file
fn mcp_config_path() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_default().join(".claude.json")
}

/// Path to Claude Code's `settings.json` (per the network-config docs), where
/// the `env` block for the inspection proxy lives.
pub fn settings_json_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".claude")
        .join("settings.json")
}

/// Build the one-off terminal command that launches Claude Code through the
/// inspection proxy, trusting the proxy's root CA. This is the manual
/// instruction shown for a client in a proxy LLM mode.
///
/// Per <https://code.claude.com/docs/en/network-config.md>: `HTTPS_PROXY`
/// carries the proxy URL (with Basic auth) and `NODE_EXTRA_CA_CERTS` points at
/// the CA to trust.
pub fn proxy_oneoff_command(proxy_url: &str, ca_cert_path: &str) -> String {
    format!("HTTPS_PROXY={proxy_url} NODE_EXTRA_CA_CERTS={ca_cert_path} claude")
}

/// The `settings.json` fragment that configures the proxy permanently — the
/// `env` block Claude Code reads on startup.
pub fn proxy_settings_json(proxy_url: &str, ca_cert_path: &str) -> serde_json::Value {
    serde_json::json!({
        "env": {
            "HTTPS_PROXY": proxy_url,
            "NODE_EXTRA_CA_CERTS": ca_cert_path,
        }
    })
}

/// Merge the proxy `env` keys into an existing (or empty) `settings.json` value,
/// preserving any other settings/env the user already has.
pub fn merge_proxy_settings(
    mut existing: serde_json::Value,
    proxy_url: &str,
    ca_cert_path: &str,
) -> serde_json::Value {
    if !existing.is_object() {
        existing = serde_json::json!({});
    }
    let obj = existing.as_object_mut().expect("object");
    let env = obj.entry("env").or_insert_with(|| serde_json::json!({}));
    if let Some(env_obj) = env.as_object_mut() {
        env_obj.insert("HTTPS_PROXY".to_string(), proxy_url.into());
        env_obj.insert("NODE_EXTRA_CA_CERTS".to_string(), ca_cert_path.into());
    }
    existing
}

/// Remove the proxy keys from `settings.json` (undo), leaving any other `env`
/// entries and settings in place and dropping an `env` block that becomes
/// empty.
pub fn remove_proxy_settings(mut existing: serde_json::Value) -> serde_json::Value {
    let Some(obj) = existing.as_object_mut() else {
        return existing;
    };
    let mut env_now_empty = false;
    if let Some(env_obj) = obj.get_mut("env").and_then(|e| e.as_object_mut()) {
        env_obj.remove("HTTPS_PROXY");
        env_obj.remove("NODE_EXTRA_CA_CERTS");
        env_now_empty = env_obj.is_empty();
    }
    if env_now_empty {
        obj.remove("env");
    }
    existing
}

impl AppIntegration for ClaudeCodeIntegration {
    fn name(&self) -> &str {
        "Claude Code"
    }

    fn check_installed(&self) -> AppCapabilities {
        let binary = super::find_binary("claude").or_else(|| {
            let home = dirs::home_dir().unwrap_or_default();
            let local_path = home.join(".claude/local/claude");
            local_path.exists().then_some(local_path)
        });

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
        client_id: &str,
    ) -> Result<LaunchResult, String> {
        // Inline MCP config for --mcp-config: the STDIO bridge, so pushed
        // events reach Claude through Channels (see the module docs).
        let binary = localrouter_binary()?;
        let mcp_config = upsert_localrouter_entry(
            serde_json::json!({}),
            stdio_bridge_entry(&binary, client_id, client_secret),
        );

        let mcp_json = serde_json::to_string(&mcp_config)
            .map_err(|e| format!("Failed to serialize MCP config: {}", e))?;

        Ok(LaunchResult {
            success: true,
            message: "Run the command below in your terminal:".to_string(),
            modified_files: vec![],
            backup_files: vec![],
            terminal_command: Some(format!(
                "ANTHROPIC_BASE_URL={} ANTHROPIC_API_KEY={} claude --mcp-config '{}' {}",
                base_url, client_secret, mcp_json, CHANNELS_FLAG
            )),
        })
    }

    fn configure_permanent(
        &self,
        base_url: &str,
        client_secret: &str,
        client_id: &str,
    ) -> Result<LaunchResult, String> {
        // Configure MCP: add LocalRouter as an MCP server in ~/.claude.json.
        // New setups get the STDIO bridge; an existing HTTP entry stays HTTP
        // until the user explicitly switches (see the module docs).
        let path = mcp_config_path();
        let existing = read_mcp_config(&path)?;
        let binary = localrouter_binary()?;
        let entry = entry_for_existing(&existing, &binary, base_url, client_id, client_secret);
        let is_http = entry["type"] == "http";
        let (path, backup_files) = write_mcp_entry(entry)?;

        let (message, terminal_command) = if is_http {
            (
                format!(
                    "MCP configured in {} over HTTP (tools only). Switch to the STDIO bridge to receive pushed events. For LLM routing, use env vars at launch time.",
                    path.display()
                ),
                None,
            )
        } else {
            (
                format!(
                    "MCP configured in {} through the STDIO bridge. Launch Claude Code with the command below to receive pushed events. For LLM routing, use env vars at launch time.",
                    path.display()
                ),
                Some(CHANNELS_LAUNCH_COMMAND.to_string()),
            )
        };

        Ok(LaunchResult {
            success: true,
            message,
            modified_files: vec![path.to_string_lossy().to_string()],
            backup_files,
            terminal_command,
        })
    }

    fn sync_config(&self, ctx: &ConfigSyncContext) -> Result<LaunchResult, String> {
        // Claude Code permanent config only writes MCP entries.
        // In mcp_via_llm/llm_only modes, remove stale MCP entry (LLM uses env vars).
        if !ctx.should_sync_mcp() {
            let path = mcp_config_path();
            if path.exists() {
                let data = std::fs::read_to_string(&path)
                    .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
                let mut config: serde_json::Value = super::config_parse::json(&data, &path)?;

                let removed = config
                    .as_object_mut()
                    .and_then(|obj| obj.get_mut("mcpServers"))
                    .and_then(|s| s.as_object_mut())
                    .and_then(|servers| servers.remove("localrouter"))
                    .is_some();

                if removed {
                    let out = serde_json::to_string_pretty(&config)
                        .map_err(|e| format!("Failed to serialize config: {}", e))?;
                    let backup_path = backup::write_with_backup(&path, out.as_bytes())?;
                    return Ok(LaunchResult {
                        success: true,
                        message: format!("Removed MCP entry from {}", path.display()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdio_bridge_entry_shape() {
        let e = stdio_bridge_entry("/Applications/LocalRouter.app/bin", "cid", "sec");
        assert_eq!(
            e,
            serde_json::json!({
                "type": "stdio",
                "command": "/Applications/LocalRouter.app/bin",
                "args": ["--mcp-bridge", "--client-id", "cid"],
                "env": { "LOCALROUTER_CLIENT_SECRET": "sec" }
            })
        );
    }

    #[test]
    fn classifies_existing_entries() {
        assert_eq!(
            localrouter_entry_transport(&serde_json::json!({})),
            ClaudeMcpTransport::None
        );
        let http = upsert_localrouter_entry(serde_json::json!({}), http_entry("http://h", "s"));
        assert_eq!(localrouter_entry_transport(&http), ClaudeMcpTransport::Http);
        let bridge = upsert_localrouter_entry(
            serde_json::json!({}),
            stdio_bridge_entry("/bin/lr", "c", "s"),
        );
        assert_eq!(
            localrouter_entry_transport(&bridge),
            ClaudeMcpTransport::StdioBridge
        );
        let other = serde_json::json!({
            "mcpServers": { "localrouter": { "type": "stdio", "command": "npx" } }
        });
        assert_eq!(
            localrouter_entry_transport(&other),
            ClaudeMcpTransport::Other
        );
        let untyped_url = serde_json::json!({
            "mcpServers": { "localrouter": { "url": "http://h" } }
        });
        assert_eq!(
            localrouter_entry_transport(&untyped_url),
            ClaudeMcpTransport::Http
        );
    }

    #[test]
    fn new_setups_get_the_bridge_and_http_stays_http() {
        let fresh = entry_for_existing(&serde_json::json!({}), "/bin/lr", "http://h", "c", "s");
        assert_eq!(fresh["type"], "stdio");
        assert_eq!(fresh["args"][0], "--mcp-bridge");

        let existing_http =
            upsert_localrouter_entry(serde_json::json!({}), http_entry("http://old", "s0"));
        let refreshed = entry_for_existing(&existing_http, "/bin/lr", "http://h", "c", "s1");
        assert_eq!(refreshed["type"], "http");
        assert_eq!(refreshed["headers"]["Authorization"], "Bearer s1");

        let existing_bridge = upsert_localrouter_entry(
            serde_json::json!({}),
            stdio_bridge_entry("/old/lr", "c", "s0"),
        );
        let refreshed = entry_for_existing(&existing_bridge, "/bin/lr", "http://h", "c", "s1");
        assert_eq!(refreshed["command"], "/bin/lr");
        assert_eq!(refreshed["env"]["LOCALROUTER_CLIENT_SECRET"], "s1");
    }

    #[test]
    fn upsert_keeps_other_servers_and_settings() {
        let existing = serde_json::json!({
            "theme": "dark",
            "mcpServers": { "other": { "type": "http", "url": "http://x" } }
        });
        let merged = upsert_localrouter_entry(existing, stdio_bridge_entry("/bin/lr", "c", "s"));
        assert_eq!(merged["theme"], "dark");
        assert_eq!(merged["mcpServers"]["other"]["url"], "http://x");
        assert_eq!(merged["mcpServers"]["localrouter"]["type"], "stdio");
    }

    #[test]
    fn try_it_out_uses_bridge_and_channels_flag() {
        let result = ClaudeCodeIntegration
            .try_it_out("http://127.0.0.1:3625", "sec", "cid")
            .unwrap();
        let cmd = result.terminal_command.unwrap();
        assert!(cmd.starts_with(
            "ANTHROPIC_BASE_URL=http://127.0.0.1:3625 ANTHROPIC_API_KEY=sec claude --mcp-config '"
        ));
        assert!(cmd.contains(r#""type":"stdio""#));
        assert!(cmd.contains(r#""--mcp-bridge","--client-id","cid""#));
        assert!(cmd.ends_with(" --dangerously-load-development-channels server:localrouter"));
        assert!(CHANNELS_LAUNCH_COMMAND.ends_with(CHANNELS_FLAG));
    }

    #[test]
    fn builds_proxy_oneoff_command() {
        let cmd = proxy_oneoff_command(
            "http://cid:sec@127.0.0.1:3626",
            "/home/u/.localrouter/proxy/root-ca.pem",
        );
        assert_eq!(
            cmd,
            "HTTPS_PROXY=http://cid:sec@127.0.0.1:3626 NODE_EXTRA_CA_CERTS=/home/u/.localrouter/proxy/root-ca.pem claude"
        );
    }

    #[test]
    fn proxy_settings_json_has_env_block() {
        let v = proxy_settings_json("http://p", "/ca.pem");
        assert_eq!(v["env"]["HTTPS_PROXY"], "http://p");
        assert_eq!(v["env"]["NODE_EXTRA_CA_CERTS"], "/ca.pem");
    }

    #[test]
    fn merge_preserves_existing_settings_and_env() {
        let existing = serde_json::json!({
            "theme": "dark",
            "env": { "FOO": "bar" }
        });
        let merged = merge_proxy_settings(existing, "http://p", "/ca.pem");
        // Unrelated settings survive.
        assert_eq!(merged["theme"], "dark");
        assert_eq!(merged["env"]["FOO"], "bar");
        // Proxy keys added.
        assert_eq!(merged["env"]["HTTPS_PROXY"], "http://p");
        assert_eq!(merged["env"]["NODE_EXTRA_CA_CERTS"], "/ca.pem");
    }

    #[test]
    fn merge_into_empty_creates_env() {
        let merged = merge_proxy_settings(serde_json::json!(null), "http://p", "/ca.pem");
        assert_eq!(merged["env"]["HTTPS_PROXY"], "http://p");
    }

    #[test]
    fn remove_keeps_other_settings_and_env_vars() {
        let existing = serde_json::json!({
            "theme": "dark",
            "env": { "FOO": "bar" }
        });
        let applied = merge_proxy_settings(existing, "http://p", "/ca.pem");
        let undone = remove_proxy_settings(applied);
        assert_eq!(undone["theme"], "dark");
        assert_eq!(undone["env"]["FOO"], "bar");
        assert!(undone["env"].get("HTTPS_PROXY").is_none());
        assert!(undone["env"].get("NODE_EXTRA_CA_CERTS").is_none());
    }

    #[test]
    fn remove_drops_an_env_block_it_emptied() {
        let applied = merge_proxy_settings(serde_json::json!({}), "http://p", "/ca.pem");
        let undone = remove_proxy_settings(applied);
        assert!(undone.get("env").is_none());
    }
}
