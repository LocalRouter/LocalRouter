//! Logins saved by the Claude Code and Codex CLIs on this machine.
//!
//! Only read when the user turns on "Use Claude Code and Codex logins". The
//! tokens are used for the usage request; they are never written anywhere
//! or refreshed (refreshing would rotate the CLI's own login). Claude Code's
//! macOS Keychain item is kept in memory until it expires, so reading it
//! (which may show a Keychain prompt) is not repeated on every poll.

use std::path::PathBuf;

use parking_lot::Mutex;
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

/// A login is re-read this long before it expires.
const EXPIRY_MARGIN_MS: i64 = 60_000;
/// How long a login without an expiry time is reused.
const NO_EXPIRY_REUSE_MS: i64 = 60 * 60 * 1000;
/// How long a failed or expired read is reused before asking again.
const RETRY_READ_MS: i64 = 30 * 60 * 1000;

/// The last read of a credential store whose reads are costly (a Keychain
/// prompt), held in memory only.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct CachedRead {
    login: Option<ClaudeCodeLogin>,
    read_at_ms: i64,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl CachedRead {
    /// Whether this read still answers at `now_ms`: a usable login until
    /// shortly before it expires (or an hour without an expiry time); no or
    /// an expired login for [`RETRY_READ_MS`], so a denied prompt or a login
    /// Claude Code has not refreshed yet is not asked about on every poll.
    fn is_fresh(&self, now_ms: i64) -> bool {
        let age = now_ms.saturating_sub(self.read_at_ms);
        if age < 0 {
            return false;
        }
        match &self.login {
            Some(l) if !l.is_expired(now_ms.saturating_add(EXPIRY_MARGIN_MS)) => {
                l.expires_at_ms.is_some() || age < NO_EXPIRY_REUSE_MS
            }
            _ => age < RETRY_READ_MS,
        }
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
static KEYCHAIN_LOGIN: Mutex<Option<CachedRead>> = Mutex::new(None);

/// `read()`'s result, reused from `cache` while it is fresh.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn cached_read(
    cache: &Mutex<Option<CachedRead>>,
    now_ms: i64,
    read: impl FnOnce() -> Option<ClaudeCodeLogin>,
) -> Option<ClaudeCodeLogin> {
    let mut cache = cache.lock();
    if let Some(c) = cache.as_ref().filter(|c| c.is_fresh(now_ms)) {
        return c.login.clone();
    }
    let login = read();
    *cache = Some(CachedRead {
        login: login.clone(),
        read_at_ms: now_ms,
    });
    login
}

/// Drop the in-memory copy of Claude Code's Keychain login (it was
/// rejected), so the next poll reads it again.
pub fn forget_cached_claude_code_login() {
    *KEYCHAIN_LOGIN.lock() = None;
}

/// Claude Code's login: the credentials file, or on macOS its Keychain item
/// (read through `/usr/bin/security`, which Claude Code itself uses, and
/// kept in memory while usable). Blocking — call from a blocking task.
pub fn read_claude_code_login() -> Option<ClaudeCodeLogin> {
    if let Some(path) = claude_config_dir().map(|d| d.join(".credentials.json")) {
        if let Ok(s) = std::fs::read_to_string(&path) {
            if let Some(login) = parse_claude_code_credentials(&s) {
                return Some(login);
            }
        }
    }
    keychain_login()
}

#[cfg(target_os = "macos")]
fn keychain_login() -> Option<ClaudeCodeLogin> {
    cached_read(
        &KEYCHAIN_LOGIN,
        chrono::Utc::now().timestamp_millis(),
        read_claude_code_keychain,
    )
}

#[cfg(not(target_os = "macos"))]
fn keychain_login() -> Option<ClaudeCodeLogin> {
    None
}

#[cfg(target_os = "macos")]
fn read_claude_code_keychain() -> Option<ClaudeCodeLogin> {
    let out = std::process::Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_claude_code_credentials(&String::from_utf8_lossy(&out.stdout))
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

    fn login(expires_at_ms: Option<i64>) -> ClaudeCodeLogin {
        ClaudeCodeLogin {
            access_token: "t".into(),
            expires_at_ms,
            plan: None,
        }
    }

    #[test]
    fn keychain_read_is_reused_until_shortly_before_expiry() {
        let cache = Mutex::new(None);
        let reads = std::cell::Cell::new(0);
        let read = |l: Option<ClaudeCodeLogin>| {
            reads.set(reads.get() + 1);
            l
        };
        let expiry = 10_000_000;
        assert!(cached_read(&cache, 0, || read(Some(login(Some(expiry))))).is_some());
        assert!(cached_read(&cache, expiry - EXPIRY_MARGIN_MS - 1, || read(None)).is_some());
        assert_eq!(reads.get(), 1);
        // Close to expiry: read again.
        cached_read(&cache, expiry - EXPIRY_MARGIN_MS, || {
            read(Some(login(Some(expiry * 2))))
        });
        assert_eq!(reads.get(), 2);
    }

    #[test]
    fn failed_or_expired_reads_are_retried_after_a_while() {
        let cache = Mutex::new(None);
        let reads = std::cell::Cell::new(0);
        let read = |l: Option<ClaudeCodeLogin>| {
            reads.set(reads.get() + 1);
            l
        };
        // Denied prompt / no item.
        assert!(cached_read(&cache, 0, || read(None)).is_none());
        assert!(cached_read(&cache, RETRY_READ_MS - 1, || read(Some(login(None)))).is_none());
        assert_eq!(reads.get(), 1);
        // A login without expiry is reused for an hour.
        assert!(cached_read(&cache, RETRY_READ_MS, || read(Some(login(None)))).is_some());
        assert!(
            cached_read(&cache, RETRY_READ_MS + NO_EXPIRY_REUSE_MS - 1, || read(
                None
            ))
            .is_some()
        );
        assert_eq!(reads.get(), 2);
        // An expired login is kept (and reported expired) until the retry.
        let t = RETRY_READ_MS + NO_EXPIRY_REUSE_MS;
        let expired = cached_read(&cache, t, || read(Some(login(Some(t - 1))))).unwrap();
        assert!(expired.is_expired(t));
        cached_read(&cache, t + RETRY_READ_MS - 1, || read(None));
        assert_eq!(reads.get(), 3);
        cached_read(&cache, t + RETRY_READ_MS, || read(None));
        assert_eq!(reads.get(), 4);
        // A clock that went backwards reads again.
        cached_read(&cache, 0, || read(None));
        assert_eq!(reads.get(), 5);
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
