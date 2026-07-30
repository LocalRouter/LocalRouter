//! Zed proxy configuration.
//!
//! Zed is proxy-relevant because two of its three LLM paths hit hosts the
//! inspection proxy already intercepts: BYO-key Anthropic/OpenAI
//! (`api.anthropic.com`, `api.openai.com`) and — the interesting one — its
//! ChatGPT Plus/Pro provider, which posts to a hardcoded
//! `chatgpt.com/backend-api/codex` and therefore cannot be repointed at the
//! gateway. Zed-hosted subscription traffic goes to `cloud.zed.dev` and stays
//! invisible either way.
//!
//! There is no LLM-provider integration here (Zed is configured through its UI);
//! this module only owns the `proxy` setting.

use std::path::PathBuf;

/// Zed's settings file. Zed uses the XDG path on macOS too.
pub fn settings_path() -> PathBuf {
    let xdg = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".config"));
    xdg.join("zed").join("settings.json")
}

/// The settings fragment configuring the proxy.
pub fn proxy_settings_json(proxy_url: &str) -> serde_json::Value {
    serde_json::json!({ "proxy": proxy_url })
}

/// Merge the `proxy` key into an existing settings document.
///
/// Zed's TLS stack is rustls with the **platform verifier**, so there is no CA
/// env var to set — our root CA has to be trusted in the OS store for
/// interception to validate. Callers report this via `requires_system_ca`.
pub fn merge_proxy_settings(mut existing: serde_json::Value, proxy_url: &str) -> serde_json::Value {
    if !existing.is_object() {
        existing = serde_json::json!({});
    }
    if let Some(obj) = existing.as_object_mut() {
        obj.insert("proxy".to_string(), proxy_url.into());
    }
    existing
}

/// Remove the `proxy` key (undo).
pub fn remove_proxy_settings(mut existing: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = existing.as_object_mut() {
        obj.remove("proxy");
    }
    existing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_proxy_key() {
        let merged = merge_proxy_settings(serde_json::json!({}), "http://p");
        assert_eq!(merged["proxy"], "http://p");
    }

    #[test]
    fn preserves_unrelated_settings() {
        let existing = serde_json::json!({
            "theme": "One Dark",
            "buffer_font_size": 15,
            "languages": { "Rust": { "tab_size": 4 } }
        });
        let merged = merge_proxy_settings(existing, "http://p");
        assert_eq!(merged["theme"], "One Dark");
        assert_eq!(merged["buffer_font_size"], 15);
        assert_eq!(merged["languages"]["Rust"]["tab_size"], 4);
        assert_eq!(merged["proxy"], "http://p");
    }

    #[test]
    fn replaces_an_existing_proxy_value() {
        let existing = serde_json::json!({ "proxy": "http://old" });
        let merged = merge_proxy_settings(existing, "http://new");
        assert_eq!(merged["proxy"], "http://new");
    }

    #[test]
    fn non_object_documents_are_replaced_not_corrupted() {
        let merged = merge_proxy_settings(serde_json::json!(null), "http://p");
        assert_eq!(merged["proxy"], "http://p");
    }

    #[test]
    fn remove_clears_only_the_proxy_key() {
        let existing = serde_json::json!({ "theme": "One Dark", "proxy": "http://p" });
        let out = remove_proxy_settings(existing);
        assert_eq!(out["theme"], "One Dark");
        assert!(out.get("proxy").is_none());
    }
}
