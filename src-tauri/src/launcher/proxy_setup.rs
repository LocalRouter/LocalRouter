//! Per-template HTTPS-inspection-proxy setup.
//!
//! One place that knows, for each client template: how to launch its tool
//! through the proxy once, what permanent config to write, which caveats the
//! user must see before applying it, and whether the root CA additionally has
//! to be trusted in the OS store.
//!
//! Feasibility research behind these entries (which tools send interceptable
//! traffic at all, and which mechanisms actually work per runtime) is in
//! `plan/2026-07-29-HTTPS_PROXY_CLIENT_AUTOCONFIG_RESEARCH.md`.

use crate::launcher::backup;
use crate::launcher::integrations::{
    aider, claude_code, codex, continue_dev, goose, jsonc, openclaw, opencode, vscode, zed,
};
use crate::ui::commands_clients::LaunchResult;
use std::path::PathBuf;

/// How the permanent config for a template is produced.
enum AutoWrite {
    /// Merge key/values into a dotenv file.
    Dotenv(Vec<(String, String)>),
    /// Replace a file we fully own with generated content.
    OwnedFile(String),
    /// Merge into a JSON document.
    Json(Box<dyn Fn(serde_json::Value) -> serde_json::Value + Send + Sync>),
    /// Merge into a JSONC document (comments are lost — warned about).
    Jsonc(Box<dyn Fn(serde_json::Value) -> serde_json::Value + Send + Sync>),
    /// Merge into a YAML document, returning the new body and a count of
    /// entries touched (0 means "nothing applicable was found").
    #[allow(clippy::type_complexity)]
    Yaml(Box<dyn Fn(&str) -> Result<(String, usize), String> + Send + Sync>),
}

/// Everything the UI and the apply path need for one template.
pub struct ProxyPlan {
    /// Env var this tool reads for a custom root CA. `None` for tools that
    /// have no such variable and validate against the OS trust store instead —
    /// callers must not synthesize a shell assignment from this.
    pub ca_env_var: Option<&'static str>,
    /// Launch-once terminal command, if the tool has one.
    pub oneoff_command: Option<String>,
    /// Copyable config snippet for the manual instructions.
    pub fragment: Option<String>,
    /// File that automatic setup writes.
    pub file: Option<PathBuf>,
    /// The root CA must be trusted in the OS store for interception to
    /// validate (tools whose TLS stack has no CA env var).
    pub requires_system_ca: bool,
    /// Caveats to render before the user applies anything.
    pub notes: Vec<String>,
    /// What the user must do for the change to take effect.
    pub restart_hint: Option<String>,
    /// How to write `file`; `None` means manual/one-off only.
    auto: Option<AutoWrite>,
    /// How to reverse it. Anything that edits a user's config file must be
    /// removable from the same UI that applied it.
    undo: Option<Undo>,
}

/// How to reverse an applied configuration.
enum Undo {
    /// Drop these keys from a dotenv file.
    DotenvKeys(Vec<String>),
    /// Delete a file we fully own.
    DeleteFile,
    /// Remove keys from a JSON document.
    Json(Box<dyn Fn(serde_json::Value) -> serde_json::Value + Send + Sync>),
    /// Remove keys from a JSONC document.
    Jsonc(Box<dyn Fn(serde_json::Value) -> serde_json::Value + Send + Sync>),
    /// Rewrite a YAML document, returning the new body and how many entries
    /// were actually removed (0 = nothing was configured).
    #[allow(clippy::type_complexity)]
    Yaml(Box<dyn Fn(&str) -> Result<(String, usize), String> + Send + Sync>),
}

impl ProxyPlan {
    pub fn supports_auto(&self) -> bool {
        self.auto.is_some() && self.file.is_some()
    }

    pub fn supports_undo(&self) -> bool {
        self.undo.is_some() && self.file.is_some()
    }
}

fn plan(ca_env_var: Option<&'static str>) -> ProxyPlan {
    ProxyPlan {
        ca_env_var,
        oneoff_command: None,
        fragment: None,
        file: None,
        requires_system_ca: false,
        notes: vec![],
        restart_hint: None,
        auto: None,
        undo: None,
    }
}

/// Shown for every tool whose config we rewrite via a YAML round-trip.
/// `serde_yaml` has no representation for comments, so they cannot survive —
/// the user has to know that before clicking Configure, not after.
const YAML_COMMENT_LOSS_NOTE: &str =
    "LocalRouter rewrites this YAML config, which drops any comments in it \
     (a backup is saved first).";

fn dotenv_undo(keys: &[&str]) -> Option<Undo> {
    Some(Undo::DotenvKeys(
        keys.iter().map(|k| k.to_string()).collect(),
    ))
}

/// Build the plan for `template_id`. `ca_cert_path` is the proxy's root CA.
///
/// `proxy_url` is `None` when the proxy listener isn't running; the plan is
/// still returned (so the UI can explain the setup) but without commands or
/// fragments that would embed a bogus URL.
pub fn plan_for(
    template_id: Option<&str>,
    proxy_url: Option<&str>,
    ca_cert_path: &str,
) -> ProxyPlan {
    match template_id {
        Some("claude-code") => {
            let mut p = plan(Some("NODE_EXTRA_CA_CERTS"));
            p.oneoff_command =
                proxy_url.map(|u| claude_code::proxy_oneoff_command(u, ca_cert_path));
            p.fragment = proxy_url.map(|u| {
                serde_json::to_string_pretty(&claude_code::proxy_settings_json(u, ca_cert_path))
                    .unwrap_or_default()
            });
            p.file = Some(claude_code::settings_json_path());
            p.restart_hint =
                Some("Restart Claude Code (this also covers background agents).".into());
            if let Some(u) = proxy_url {
                let (u, ca) = (u.to_string(), ca_cert_path.to_string());
                p.auto = Some(AutoWrite::Json(Box::new(move |existing| {
                    claude_code::merge_proxy_settings(existing, &u, &ca)
                })));
            }
            p.undo = Some(Undo::Json(Box::new(claude_code::remove_proxy_settings)));
            p
        }

        Some("codex") => {
            let mut p = plan(Some(codex::PROXY_CA_ENV_VAR));
            p.oneoff_command = proxy_url.map(|u| codex::proxy_oneoff_command(u, ca_cert_path));
            p.fragment = proxy_url.map(|u| codex::proxy_env_fragment(u, ca_cert_path));
            p.file = Some(codex::env_file_path());
            p.notes.push(
                "Codex ignores CODEX_-prefixed keys in ~/.codex/.env, so permanent setup uses \
                 SSL_CERT_FILE (its documented custom-CA fallback, applied on top of system roots)."
                    .into(),
            );
            p.restart_hint = Some("Start a new Codex session.".into());
            if let Some(u) = proxy_url {
                let (u, ca) = (u.to_string(), ca_cert_path.to_string());
                p.auto = Some(AutoWrite::Dotenv(vec![
                    ("HTTPS_PROXY".into(), u),
                    ("SSL_CERT_FILE".into(), ca),
                ]));
            }
            p.undo = dotenv_undo(&["HTTPS_PROXY", "SSL_CERT_FILE"]);
            p
        }

        Some("openclaw") => {
            let mut p = plan(Some(openclaw::PROXY_CA_ENV_VAR));
            p.oneoff_command = proxy_url.map(|u| openclaw::proxy_oneoff_command(u, ca_cert_path));
            p.fragment = proxy_url.map(|u| openclaw::proxy_env_fragment(u, ca_cert_path));
            p.file = Some(openclaw::env_file_path());
            p.notes.push(
                "HTTPS_PROXY applies to all OpenClaw egress, including its messaging transports \
                 (Telegram, Discord, …). Hosts outside the proxy's inspection list are tunneled \
                 through untouched."
                    .into(),
            );
            p.restart_hint = Some(
                "Restart the gateway: `openclaw gateway restart`. If you installed it as a \
                 service, re-bake the environment with `openclaw gateway install --force`."
                    .into(),
            );
            if let Some(u) = proxy_url {
                let (u, ca) = (u.to_string(), ca_cert_path.to_string());
                p.auto = Some(AutoWrite::Dotenv(vec![
                    ("HTTPS_PROXY".into(), u),
                    (openclaw::PROXY_CA_ENV_VAR.into(), ca),
                ]));
            }
            p.undo = dotenv_undo(&openclaw::proxy_env_keys());
            p
        }

        Some("opencode") => {
            let mut p = plan(Some(opencode::PROXY_CA_ENV_VAR));
            p.oneoff_command = proxy_url.map(|u| opencode::proxy_oneoff_command(u, ca_cert_path));
            p.fragment = proxy_url.map(|u| opencode::proxy_plugin_source(u, ca_cert_path));
            p.file = Some(opencode::plugin_file_path());
            p.notes.push(
                "opencode has no env block in opencode.json and no dotenv loading, so setup \
                 writes a small auto-loaded plugin that sets the proxy variables at startup. \
                 The file is entirely generated — delete it to undo."
                    .into(),
            );
            p.notes.push(
                "Loopback must bypass the proxy (NO_PROXY) because opencode's TUI talks to its \
                 own local server; this is set for you."
                    .into(),
            );
            p.notes.push(
                "Known upstream issue: the Bun runtime opencode ships has a proxy CONNECT-tunnel \
                 bug (oven-sh/bun#30381) that can affect streamed responses. If replies hang or \
                 look corrupted, use gateway mode instead until Bun ships the fix."
                    .into(),
            );
            p.restart_hint = Some("Restart opencode.".into());
            if let Some(u) = proxy_url {
                p.auto = Some(AutoWrite::OwnedFile(opencode::proxy_plugin_source(
                    u,
                    ca_cert_path,
                )));
            }
            // The plugin file is entirely ours, so undo is a plain delete.
            p.undo = Some(Undo::DeleteFile);
            p
        }

        Some("aider") => {
            let mut p = plan(Some(aider::PROXY_CA_ENV_VAR));
            // Aider's CA vars replace the trust store, so everything here
            // points at a combined bundle rather than the bare root CA.
            let bundle = crate::launcher::ca_bundle::combined_bundle_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| ca_cert_path.to_string());
            p.oneoff_command = proxy_url.map(|u| aider::proxy_oneoff_command(u, &bundle));
            p.fragment = proxy_url.map(|u| aider::proxy_env_fragment(u, &bundle));
            p.file = Some(aider::env_file_path());
            p.notes.push(
                "Aider's Python stack treats SSL_CERT_FILE / REQUESTS_CA_BUNDLE as a replacement \
                 for its trust store, not an addition. LocalRouter therefore generates a combined \
                 bundle (your existing roots plus its CA) and points Aider at that."
                    .into(),
            );
            p.notes.push(
                "~/.env is shared with your other tools; only the proxy keys are added and the \
                 rest of the file is preserved."
                    .into(),
            );
            p.notes.push(
                "Aider works fully in gateway mode (its base URL is repointable), so proxy mode \
                 is optional here — use it if you prefer one setup for every provider."
                    .into(),
            );
            p.restart_hint = Some("Start a new Aider session.".into());
            if let Some(u) = proxy_url {
                p.auto = Some(AutoWrite::Dotenv(vec![
                    ("HTTPS_PROXY".into(), u.to_string()),
                    ("SSL_CERT_FILE".into(), bundle.clone()),
                    ("REQUESTS_CA_BUNDLE".into(), bundle),
                    ("AIOHTTP_TRUST_ENV".into(), "1".into()),
                ]));
            }
            p.undo = dotenv_undo(&aider::proxy_env_keys());
            p
        }

        Some("vscode-continue") => {
            let mut p = plan(None);
            p.fragment = proxy_url.map(|u| continue_dev::proxy_fragment(u, ca_cert_path));
            p.file = Some(continue_dev::config_path());
            p.notes.push(
                "Only your Anthropic and OpenAI models are updated — those are the ones the \
                 proxy inspects. Other providers are left alone."
                    .into(),
            );
            p.notes.push(
                "Continue works fully in gateway mode (apiBase is settable), so proxy mode is \
                 optional here."
                    .into(),
            );
            p.notes.push(YAML_COMMENT_LOSS_NOTE.into());
            p.restart_hint = Some("Reload the Continue extension.".into());
            if let Some(u) = proxy_url {
                let (u, ca) = (u.to_string(), ca_cert_path.to_string());
                p.auto = Some(AutoWrite::Yaml(Box::new(move |existing| {
                    continue_dev::merge_proxy_config(existing, &u, &ca)
                })));
            }
            p.undo = Some(Undo::Yaml(Box::new(continue_dev::remove_proxy_config)));
            p
        }

        Some("goose") => {
            let mut p = plan(Some(goose::CA_CONFIG_KEY));
            p.oneoff_command = proxy_url.map(|u| goose::proxy_oneoff_command(u, ca_cert_path));
            p.fragment = Some(format!("{}: {}\n", goose::CA_CONFIG_KEY, ca_cert_path));
            p.file = Some(goose_config_path());
            p.requires_system_ca = true;
            p.notes.push(
                "Goose has no config-file setting for the proxy itself, so HTTPS_PROXY has to be \
                 supplied when you launch it — use the Quick Start command. Only the CA path is \
                 written to config.yaml."
                    .into(),
            );
            p.notes.push(
                "Goose's ChatGPT subscription provider builds its HTTP client without the \
                 configured CA, so LocalRouter's root CA must also be trusted in your system \
                 keychain for that traffic to work."
                    .into(),
            );
            p.notes.push(
                "The Goose desktop app can't be configured this way — it reads no proxy setting \
                 from config.yaml."
                    .into(),
            );
            p.notes.push(YAML_COMMENT_LOSS_NOTE.into());
            p.restart_hint = Some("Start a new Goose session with the command above.".into());
            if proxy_url.is_some() {
                let ca = ca_cert_path.to_string();
                p.auto = Some(AutoWrite::Yaml(Box::new(move |existing| {
                    goose::merge_proxy_config(existing, &ca).map(|s| (s, 1))
                })));
            }
            p.undo = Some(Undo::Yaml(Box::new(goose::remove_proxy_config)));
            p
        }

        Some("zed") => {
            let mut p = plan(None);
            p.fragment = proxy_url.map(|u| {
                serde_json::to_string_pretty(&zed::proxy_settings_json(u)).unwrap_or_default()
            });
            p.file = Some(zed::settings_path());
            p.requires_system_ca = true;
            p.notes.push(
                "Zed validates TLS against the operating system's trust store and has no CA \
                 setting, so LocalRouter's root CA must be trusted there."
                    .into(),
            );
            p.notes.push(
                "Zed's settings file allows comments; LocalRouter rewrites it as plain JSON, so \
                 any comments are dropped. A backup is saved first."
                    .into(),
            );
            p.notes.push(
                "Zed-hosted (Zed Pro) models and edit predictions go to Zed's own servers and \
                 stay invisible to the proxy; your own Anthropic/OpenAI keys and the ChatGPT \
                 subscription provider are inspected."
                    .into(),
            );
            p.restart_hint = Some("Restart Zed.".into());
            if let Some(u) = proxy_url {
                let u = u.to_string();
                p.auto = Some(AutoWrite::Jsonc(Box::new(move |existing| {
                    zed::merge_proxy_settings(existing, &u)
                })));
            }
            p.undo = Some(Undo::Jsonc(Box::new(zed::remove_proxy_settings)));
            p
        }

        Some("cline") | Some("roo-code") => {
            let mut p = plan(None);
            p.fragment = proxy_url.map(|u| {
                serde_json::to_string_pretty(&vscode::proxy_settings_json(u)).unwrap_or_default()
            });
            p.file = Some(vscode::settings_path());
            p.requires_system_ca = true;
            p.notes.push(
                "This edits Visual Studio Code's own settings, which are editor-wide: every VS \
                 Code request (extensions, marketplace, telemetry) will go through LocalRouter. \
                 Hosts it doesn't inspect are tunneled through untouched."
                    .into(),
            );
            p.notes.push(
                "VS Code has no per-extension CA setting, so LocalRouter's root CA must be \
                 trusted in your system keychain."
                    .into(),
            );
            p.notes.push(
                "This is what makes the ChatGPT subscription provider visible — its endpoint is \
                 fixed and can't be pointed at the gateway. API-key providers work in gateway \
                 mode without any of this."
                    .into(),
            );
            p.notes.push(
                "Other VS Code forks (Cursor, VSCodium) keep separate settings and are not \
                 changed."
                    .into(),
            );
            p.restart_hint = Some("Reload the VS Code window.".into());
            if let Some(u) = proxy_url {
                let u = u.to_string();
                p.auto = Some(AutoWrite::Jsonc(Box::new(move |existing| {
                    vscode::merge_proxy_settings(existing, &u)
                })));
            }
            p.undo = Some(Undo::Jsonc(Box::new(vscode::remove_proxy_settings)));
            p
        }

        // Generic/custom tools: SSL_CERT_FILE is the widest-supported CA
        // override; no tool-specific command or config file.
        _ => plan(Some("SSL_CERT_FILE")),
    }
}

fn goose_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".config")
        .join("goose")
        .join("config.yaml")
}

/// Apply a plan's automatic configuration, writing with a backup.
pub fn apply(plan: &ProxyPlan) -> Result<LaunchResult, String> {
    let (Some(auto), Some(path)) = (plan.auto.as_ref(), plan.file.as_ref()) else {
        return Err(
            "Automatic setup isn't available for this tool. Use the manual instructions."
                .to_string(),
        );
    };

    let existing_raw = if path.exists() {
        std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?
    } else {
        String::new()
    };

    let mut extra_message = String::new();

    let data = match auto {
        AutoWrite::Dotenv(updates) => {
            let pairs: Vec<(&str, &str)> = updates
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            crate::launcher::integrations::dotenv::merge_env(&existing_raw, &pairs)
        }
        AutoWrite::OwnedFile(content) => content.clone(),
        AutoWrite::Json(merge) => {
            // A malformed JSON settings file must not be silently replaced —
            // that would discard the user's real settings.
            let existing: serde_json::Value = if existing_raw.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&existing_raw)
                    .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?
            };
            serde_json::to_string_pretty(&merge(existing))
                .map_err(|e| format!("Failed to serialize settings: {e}"))?
        }
        AutoWrite::Jsonc(merge) => {
            let existing = jsonc::parse(&existing_raw)
                .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;
            if jsonc::has_comments(&existing_raw) {
                extra_message
                    .push_str(" Comments in the original file were removed (a backup was saved).");
            }
            serde_json::to_string_pretty(&merge(existing))
                .map_err(|e| format!("Failed to serialize settings: {e}"))?
        }
        AutoWrite::Yaml(merge) => {
            let (out, updated) = merge(&existing_raw)?;
            if updated == 0 {
                return Err(format!(
                    "Nothing to configure in {}: no models on a provider the proxy inspects \
                     (Anthropic or OpenAI) were found.",
                    path.display()
                ));
            }
            out
        }
    };

    let backup_path = backup::write_with_backup(path, data.as_bytes())?;

    let mut message = format!("Configured the proxy in {}", path.display());
    message.push_str(&extra_message);
    if let Some(hint) = &plan.restart_hint {
        message.push(' ');
        message.push_str(hint);
    }

    Ok(LaunchResult {
        success: true,
        message,
        modified_files: vec![path.to_string_lossy().to_string()],
        backup_files: backup_path
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
        terminal_command: None,
    })
}

/// Reverse a previously applied configuration.
///
/// A missing target file means there is nothing to undo — reported as success
/// rather than an error, so the button is idempotent.
pub fn unapply(plan: &ProxyPlan) -> Result<LaunchResult, String> {
    let (Some(undo), Some(path)) = (plan.undo.as_ref(), plan.file.as_ref()) else {
        return Err("There is nothing for LocalRouter to undo for this tool.".to_string());
    };

    if !path.exists() {
        return Ok(LaunchResult {
            success: true,
            message: format!("Nothing to remove — {} does not exist.", path.display()),
            modified_files: vec![],
            backup_files: vec![],
            terminal_command: None,
        });
    }

    if matches!(undo, Undo::DeleteFile) {
        std::fs::remove_file(path)
            .map_err(|e| format!("Failed to remove {}: {e}", path.display()))?;
        return Ok(LaunchResult {
            success: true,
            message: format!("Removed {}", path.display()),
            modified_files: vec![path.to_string_lossy().to_string()],
            backup_files: vec![],
            terminal_command: None,
        });
    }

    let existing_raw = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

    let mut extra_message = String::new();

    // `None` means the file holds no configuration of ours. That distinction
    // matters: rewriting a JSONC or YAML file we never touched would strip the
    // user's comments for nothing, and Remove is reachable whether or not
    // Configure was ever clicked.
    let data: Option<String> = match undo {
        Undo::DeleteFile => unreachable!("handled above"),
        Undo::DotenvKeys(keys) => {
            let refs: Vec<&str> = keys.iter().map(|k| k.as_str()).collect();
            let out = crate::launcher::integrations::dotenv::remove_env_keys(&existing_raw, &refs);
            (out != existing_raw).then_some(out)
        }
        Undo::Json(remove) => {
            let existing: serde_json::Value = if existing_raw.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&existing_raw)
                    .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?
            };
            let cleaned = remove(existing.clone());
            if cleaned == existing {
                None
            } else {
                Some(
                    serde_json::to_string_pretty(&cleaned)
                        .map_err(|e| format!("Failed to serialize settings: {e}"))?,
                )
            }
        }
        Undo::Jsonc(remove) => {
            let existing = jsonc::parse(&existing_raw)
                .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;
            let cleaned = remove(existing.clone());
            if cleaned == existing {
                None
            } else {
                if jsonc::has_comments(&existing_raw) {
                    extra_message.push_str(
                        " Comments in the original file were removed (a backup was saved).",
                    );
                }
                Some(
                    serde_json::to_string_pretty(&cleaned)
                        .map_err(|e| format!("Failed to serialize settings: {e}"))?,
                )
            }
        }
        Undo::Yaml(remove) => {
            let (out, removed) = remove(&existing_raw)?;
            (removed > 0).then_some(out)
        }
    };

    let Some(data) = data else {
        return Ok(LaunchResult {
            success: true,
            message: format!(
                "Nothing to remove — LocalRouter has no proxy configuration in {}.",
                path.display()
            ),
            modified_files: vec![],
            backup_files: vec![],
            terminal_command: None,
        });
    };

    let backup_path = backup::write_with_backup(path, data.as_bytes())?;

    let mut message = format!("Removed the proxy configuration from {}", path.display());
    message.push_str(&extra_message);

    Ok(LaunchResult {
        success: true,
        message,
        modified_files: vec![path.to_string_lossy().to_string()],
        backup_files: backup_path
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
        terminal_command: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://cid:sec@127.0.0.1:3626";
    const CA: &str = "/ca/root-ca.pem";

    /// Every template we advertise proxy support for must produce a usable
    /// plan — something to run, something to copy, or something to write.
    #[test]
    fn advertised_templates_all_produce_actionable_plans() {
        for id in [
            "claude-code",
            "codex",
            "openclaw",
            "opencode",
            "aider",
            "vscode-continue",
            "goose",
            "zed",
            "cline",
            "roo-code",
        ] {
            let p = plan_for(Some(id), Some(URL), CA);
            assert!(
                p.oneoff_command.is_some() || p.fragment.is_some() || p.supports_auto(),
                "template '{id}' produced an empty plan"
            );
        }
    }

    #[test]
    fn auto_capable_templates_have_a_target_file() {
        for id in [
            "claude-code",
            "codex",
            "openclaw",
            "opencode",
            "aider",
            "vscode-continue",
            "goose",
            "zed",
            "cline",
            "roo-code",
        ] {
            let p = plan_for(Some(id), Some(URL), CA);
            assert!(p.supports_auto(), "'{id}' should support automatic setup");
            assert!(p.file.is_some(), "'{id}' should name a config file");
        }
    }

    #[test]
    fn no_proxy_url_means_no_commands_or_auto() {
        for id in ["claude-code", "codex", "openclaw", "opencode", "zed"] {
            let p = plan_for(Some(id), None, CA);
            assert!(p.oneoff_command.is_none(), "'{id}' leaked a command");
            assert!(
                p.fragment.is_none() || id == "goose",
                "'{id}' leaked a fragment"
            );
            assert!(!p.supports_auto(), "'{id}' offered auto without a proxy");
        }
    }

    #[test]
    fn templates_without_a_ca_env_var_require_the_system_trust_store() {
        for id in ["goose", "zed", "cline", "roo-code"] {
            let p = plan_for(Some(id), Some(URL), CA);
            assert!(
                p.requires_system_ca,
                "'{id}' needs the OS trust store and must say so"
            );
        }
        // Tools with an additive CA env var must not demand it.
        for id in ["claude-code", "codex", "openclaw", "opencode"] {
            let p = plan_for(Some(id), Some(URL), CA);
            assert!(!p.requires_system_ca, "'{id}' should not need the OS store");
        }
    }

    #[test]
    fn heavyweight_and_lossy_changes_always_carry_a_warning() {
        // These edits have consequences beyond the tool being configured;
        // shipping them without an explanation would be user-hostile.
        for id in ["cline", "roo-code", "zed", "aider", "goose", "opencode"] {
            let p = plan_for(Some(id), Some(URL), CA);
            assert!(!p.notes.is_empty(), "'{id}' must explain its caveats");
        }
    }

    #[test]
    fn aider_never_points_python_ca_vars_at_the_bare_root_ca() {
        // Doing so would replace Aider's whole trust store and break every
        // host the proxy doesn't intercept.
        let p = plan_for(Some("aider"), Some(URL), CA);
        let fragment = p.fragment.unwrap();
        assert!(fragment.contains("SSL_CERT_FILE="));
        assert!(
            !fragment.contains(&format!("SSL_CERT_FILE={CA}")),
            "aider must use the combined bundle, not the raw CA"
        );
        assert!(fragment.contains("combined-ca.pem"));
    }

    #[test]
    fn unknown_and_non_interceptable_templates_get_no_auto_setup() {
        for id in [Some("cursor"), Some("jetbrains"), Some("unknown"), None] {
            let p = plan_for(id, Some(URL), CA);
            assert!(
                !p.supports_auto(),
                "{id:?} should not offer automatic setup"
            );
            assert!(p.file.is_none());
        }
    }

    /// Anything that edits a user's config file must be removable from the
    /// same UI that applied it.
    #[test]
    fn everything_we_can_apply_we_can_also_undo() {
        for id in [
            "claude-code",
            "codex",
            "openclaw",
            "opencode",
            "aider",
            "vscode-continue",
            "goose",
            "zed",
            "cline",
            "roo-code",
        ] {
            let p = plan_for(Some(id), Some(URL), CA);
            assert!(p.supports_auto(), "'{id}' should support automatic setup");
            assert!(p.supports_undo(), "'{id}' must be reversible");
        }
    }

    #[test]
    fn undo_is_offered_even_when_the_proxy_is_stopped() {
        // A stopped proxy must not strand config we already wrote.
        for id in ["claude-code", "codex", "openclaw", "opencode", "zed"] {
            let p = plan_for(Some(id), None, CA);
            assert!(p.supports_undo(), "'{id}' must stay reversible");
        }
    }

    #[test]
    fn unapply_on_a_missing_file_is_a_no_op_success() {
        let mut p = plan_for(Some("codex"), Some(URL), CA);
        p.file = Some(PathBuf::from("/nonexistent/localrouter-test/.env"));
        let result = unapply(&p).expect("undo should be idempotent");
        assert!(result.success);
        assert!(result.modified_files.is_empty());
    }

    #[test]
    fn unapply_refuses_when_there_is_nothing_to_undo() {
        let p = plan_for(Some("cursor"), Some(URL), CA);
        assert!(unapply(&p).is_err());
    }

    /// Remove is reachable whether or not Configure was ever clicked. On a
    /// file we never touched it must leave the bytes alone — rewriting a
    /// JSONC/YAML config would silently strip the user's comments.
    #[test]
    fn unapply_does_not_rewrite_a_file_it_never_configured() {
        let dir = std::env::temp_dir().join(format!("lr-unapply-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // A Zed settings file full of comments, with no proxy key.
        let zed_path = dir.join("zed-settings.json");
        let pristine = "{\n  // my theme\n  \"theme\": \"One Dark\",\n}\n";
        std::fs::write(&zed_path, pristine).unwrap();
        let mut p = plan_for(Some("zed"), Some(URL), CA);
        p.file = Some(zed_path.clone());
        let result = unapply(&p).unwrap();
        assert!(result.success);
        assert!(result.modified_files.is_empty(), "must not rewrite");
        assert_eq!(std::fs::read_to_string(&zed_path).unwrap(), pristine);

        // Same for a commented Goose config.
        let goose_path = dir.join("goose-config.yaml");
        let goose_pristine = "# my goose config\nGOOSE_PROVIDER: openai\n";
        std::fs::write(&goose_path, goose_pristine).unwrap();
        let mut p = plan_for(Some("goose"), Some(URL), CA);
        p.file = Some(goose_path.clone());
        let result = unapply(&p).unwrap();
        assert!(result.modified_files.is_empty(), "must not rewrite");
        assert_eq!(
            std::fs::read_to_string(&goose_path).unwrap(),
            goose_pristine
        );

        // And a dotenv holding only unrelated secrets.
        let env_path = dir.join("dot-env");
        let env_pristine = "# secrets\nOPENAI_API_KEY=sk-1\n";
        std::fs::write(&env_path, env_pristine).unwrap();
        let mut p = plan_for(Some("codex"), Some(URL), CA);
        p.file = Some(env_path.clone());
        let result = unapply(&p).unwrap();
        assert!(result.modified_files.is_empty(), "must not rewrite");
        assert_eq!(std::fs::read_to_string(&env_path).unwrap(), env_pristine);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Rewriting a commented settings file must say so, on undo as well as
    /// apply — the user is losing something a backup alone doesn't advertise.
    #[test]
    fn unapply_warns_when_it_drops_comments() {
        let dir = std::env::temp_dir().join(format!("lr-unapply-warn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        // This one *is* configured, so undo really does rewrite it.
        std::fs::write(
            &path,
            "{\n  // theme\n  \"theme\": \"One Dark\",\n  \"proxy\": \"http://old\"\n}\n",
        )
        .unwrap();
        let mut p = plan_for(Some("zed"), Some(URL), CA);
        p.file = Some(path.clone());
        let result = unapply(&p).unwrap();
        assert!(!result.modified_files.is_empty());
        assert!(
            result.message.to_lowercase().contains("comment"),
            "undo must warn about comment loss: {}",
            result.message
        );
        // The actual setting survives; only our key goes.
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after["theme"], "One Dark");
        assert!(after.get("proxy").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A tool with no CA environment variable must not advertise one — the UI
    /// splices this into a shell command.
    #[test]
    fn tools_without_a_ca_env_var_report_none() {
        for id in ["zed", "cline", "roo-code", "vscode-continue"] {
            let p = plan_for(Some(id), Some(URL), CA);
            assert!(
                p.ca_env_var.is_none(),
                "'{id}' has no CA env var and must not claim one"
            );
        }
        for id in ["claude-code", "codex", "openclaw", "opencode", "aider"] {
            let p = plan_for(Some(id), Some(URL), CA);
            let var = p.ca_env_var.expect("should name a real env var");
            assert!(
                !var.contains(' '),
                "'{id}' env var '{var}' is prose, not a variable name"
            );
        }
    }

    #[test]
    fn every_auto_plan_tells_the_user_how_to_make_it_take_effect() {
        for id in [
            "claude-code",
            "codex",
            "openclaw",
            "opencode",
            "aider",
            "vscode-continue",
            "goose",
            "zed",
            "cline",
        ] {
            let p = plan_for(Some(id), Some(URL), CA);
            assert!(p.restart_hint.is_some(), "'{id}' needs a restart hint");
        }
    }
}
