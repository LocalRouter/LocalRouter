//! Logins saved by the Claude Code and Codex CLIs on this machine.
//!
//! Only read when the user turns on "Use Claude Code and Codex logins". The
//! tokens are used for the usage request and dropped; they are never stored
//! or refreshed (refreshing would rotate the CLI's own login).

use std::path::PathBuf;

use serde_json::Value;

/// Claude Code's OAuth login.
#[derive(Clone)]
pub struct ClaudeCodeLogin {
    pub access_token: String,
    /// Unix milliseconds.
    pub expires_at_ms: Option<i64>,
    /// `rateLimitTier` (e.g. `default_claude_max_20x`) or `subscriptionType`.
    pub plan: Option<String>,
}

impl std::fmt::Debug for ClaudeCodeLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeCodeLogin")
            .field("expires_at_ms", &self.expires_at_ms)
            .field("plan", &self.plan)
            .finish_non_exhaustive()
    }
}

impl ClaudeCodeLogin {
    pub fn is_expired(&self, now_ms: i64) -> bool {
        self.expires_at_ms.is_some_and(|e| e <= now_ms)
    }
}

/// Codex's ChatGPT login.
#[derive(Clone)]
pub struct CodexLogin {
    pub access_token: String,
    pub account_id: Option<String>,
}

impl std::fmt::Debug for CodexLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexLogin")
            .field("account_id", &self.account_id)
            .finish_non_exhaustive()
    }
}

fn claude_config_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".claude")))
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
}

/// Parse Claude Code's credentials JSON (`{"claudeAiOauth": {...}}`).
pub fn parse_claude_code_credentials(json: &str) -> Option<ClaudeCodeLogin> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    let o = v.get("claudeAiOauth")?;
    let access_token = o
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?
        .to_string();
    let plan = o
        .get("rateLimitTier")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| o.get("subscriptionType").and_then(Value::as_str))
        .map(str::to_string);
    Some(ClaudeCodeLogin {
        access_token,
        expires_at_ms: o.get("expiresAt").and_then(Value::as_i64),
        plan,
    })
}

/// Parse Codex's `auth.json`. API-key logins (no ChatGPT tokens) give `None`.
pub fn parse_codex_auth(json: &str) -> Option<CodexLogin> {
    let v: Value = serde_json::from_str(json).ok()?;
    let tokens = v.get("tokens")?;
    let access_token = tokens
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?
        .to_string();
    Some(CodexLogin {
        access_token,
        account_id: tokens
            .get("account_id")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// Claude Code's login: the credentials file, or on macOS its Keychain item
/// (read through `/usr/bin/security`, which Claude Code itself uses).
/// Blocking — call from a blocking task.
pub fn read_claude_code_login() -> Option<ClaudeCodeLogin> {
    if let Some(path) = claude_config_dir().map(|d| d.join(".credentials.json")) {
        if let Ok(s) = std::fs::read_to_string(&path) {
            if let Some(login) = parse_claude_code_credentials(&s) {
                return Some(login);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/bin/security")
            .args([
                "find-generic-password",
                "-s",
                "Claude Code-credentials",
                "-w",
            ])
            .output()
            .ok()?;
        if out.status.success() {
            return parse_claude_code_credentials(&String::from_utf8_lossy(&out.stdout));
        }
    }
    None
}

/// Codex's ChatGPT login from `$CODEX_HOME/auth.json`. Blocking.
pub fn read_codex_login() -> Option<CodexLogin> {
    let path = codex_home()?.join("auth.json");
    parse_codex_auth(&std::fs::read_to_string(path).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_credentials() {
        let json = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-x","refreshToken":"r",
            "expiresAt":1760000000000,"scopes":["user:inference","user:profile"],
            "subscriptionType":"max","rateLimitTier":"default_claude_max_20x"}}"#;
        let l = parse_claude_code_credentials(json).unwrap();
        assert_eq!(l.access_token, "sk-ant-oat01-x");
        assert_eq!(l.plan.as_deref(), Some("default_claude_max_20x"));
        assert!(l.is_expired(1_760_000_000_001));
        assert!(!l.is_expired(1_759_000_000_000));
        assert!(
            !format!("{l:?}").contains("sk-ant"),
            "Debug must not leak the token"
        );
        assert!(parse_claude_code_credentials("{}").is_none());
    }

    #[test]
    fn codex_auth() {
        let json = r#"{"OPENAI_API_KEY":null,"tokens":{"id_token":"i","access_token":"a","refresh_token":"r","account_id":"acc"}}"#;
        let l = parse_codex_auth(json).unwrap();
        assert_eq!(l.access_token, "a");
        assert_eq!(l.account_id.as_deref(), Some("acc"));
        assert!(parse_codex_auth(r#"{"OPENAI_API_KEY":"sk-x"}"#).is_none());
    }
}
