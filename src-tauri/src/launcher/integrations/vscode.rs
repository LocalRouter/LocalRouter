//! VS Code proxy configuration (covers the Cline and Roo Code templates).
//!
//! Why this exists: both extensions ship a ChatGPT-Codex subscription provider
//! that posts to a hardcoded `chatgpt.com/backend-api/codex` — traffic that
//! cannot be repointed at the gateway, so the inspection proxy is the only way
//! to see it. Their API-key providers are repointable and don't need this.
//!
//! How it works: VS Code patches the extension host's network stack. The
//! `http.proxy` setting (whose own schema permits embedded credentials) is
//! applied to `http`/`https` requests, and with `http.fetchAdditionalSupport`
//! (default on) to global `fetch` too — which is what these extensions use.
//! `http.systemCertificates` (default on) loads the OS trust store, so our
//! root CA must live there; there is no per-extension CA setting.
//!
//! **The catch, which must always be shown to the user:** `http.proxy` is
//! editor-global. Every VS Code request — marketplace, telemetry, other
//! extensions — goes through the proxy. That is survivable because the proxy
//! blind-tunnels every host outside its MITM allowlist, but it is not a
//! narrowly-scoped change and must never be applied silently.

use std::path::PathBuf;

/// Per-fork user-settings location. VS Code forks keep separate settings
/// directories, so configuring "VS Code" does not configure Cursor/VSCodium.
pub fn settings_path_for_dir_name(dir_name: &str) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir()
            .unwrap_or_default()
            .join("Library")
            .join("Application Support")
            .join(dir_name)
            .join("User")
            .join("settings.json")
    }
    #[cfg(target_os = "windows")]
    {
        dirs::config_dir()
            .unwrap_or_default()
            .join(dir_name)
            .join("User")
            .join("settings.json")
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        dirs::config_dir()
            .unwrap_or_default()
            .join(dir_name)
            .join("User")
            .join("settings.json")
    }
}

/// Stable VS Code (the host for the Cline / Roo Code templates).
pub fn settings_path() -> PathBuf {
    settings_path_for_dir_name("Code")
}

/// The settings fragment configuring the proxy.
pub fn proxy_settings_json(proxy_url: &str) -> serde_json::Value {
    serde_json::json!({
        "http.proxy": proxy_url,
        "http.proxySupport": "override",
        "http.systemCertificates": true,
        "http.fetchAdditionalSupport": true,
    })
}

/// Merge the proxy keys into an existing settings document.
///
/// `http.proxySupport: "override"` and the two boolean defaults are written
/// explicitly rather than relied on: a user who previously turned any of them
/// off would otherwise get a proxy setting that silently does nothing for
/// extension `fetch` calls or rejects our CA.
pub fn merge_proxy_settings(mut existing: serde_json::Value, proxy_url: &str) -> serde_json::Value {
    if !existing.is_object() {
        existing = serde_json::json!({});
    }
    if let Some(obj) = existing.as_object_mut() {
        obj.insert("http.proxy".to_string(), proxy_url.into());
        obj.insert("http.proxySupport".to_string(), "override".into());
        obj.insert("http.systemCertificates".to_string(), true.into());
        obj.insert("http.fetchAdditionalSupport".to_string(), true.into());
    }
    existing
}

/// Remove the proxy keys we set (undo). Leaves the certificate/fetch toggles
/// at VS Code's defaults by deleting them rather than forcing `false`.
pub fn remove_proxy_settings(mut existing: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = existing.as_object_mut() {
        for key in [
            "http.proxy",
            "http.proxySupport",
            "http.systemCertificates",
            "http.fetchAdditionalSupport",
        ] {
            obj.remove(key);
        }
    }
    existing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_proxy_and_supporting_toggles() {
        let merged = merge_proxy_settings(serde_json::json!({}), "http://u:p@127.0.0.1:3626");
        assert_eq!(merged["http.proxy"], "http://u:p@127.0.0.1:3626");
        assert_eq!(merged["http.proxySupport"], "override");
        assert_eq!(merged["http.systemCertificates"], true);
        assert_eq!(merged["http.fetchAdditionalSupport"], true);
    }

    #[test]
    fn preserves_unrelated_user_settings() {
        let existing = serde_json::json!({
            "editor.fontSize": 13,
            "workbench.colorTheme": "Default Dark+"
        });
        let merged = merge_proxy_settings(existing, "http://p");
        assert_eq!(merged["editor.fontSize"], 13);
        assert_eq!(merged["workbench.colorTheme"], "Default Dark+");
    }

    #[test]
    fn overrides_settings_that_would_disable_interception() {
        // A user who had turned these off would otherwise get a proxy that
        // silently doesn't apply to extension fetch calls / rejects our CA.
        let existing = serde_json::json!({
            "http.proxySupport": "off",
            "http.systemCertificates": false,
            "http.fetchAdditionalSupport": false
        });
        let merged = merge_proxy_settings(existing, "http://p");
        assert_eq!(merged["http.proxySupport"], "override");
        assert_eq!(merged["http.systemCertificates"], true);
        assert_eq!(merged["http.fetchAdditionalSupport"], true);
    }

    #[test]
    fn remove_restores_defaults_by_deleting_keys() {
        let merged = merge_proxy_settings(serde_json::json!({"editor.fontSize": 13}), "http://p");
        let cleaned = remove_proxy_settings(merged);
        assert_eq!(cleaned["editor.fontSize"], 13);
        assert!(cleaned.get("http.proxy").is_none());
        assert!(cleaned.get("http.proxySupport").is_none());
        assert!(cleaned.get("http.systemCertificates").is_none());
    }

    #[test]
    fn settings_path_is_fork_specific() {
        let code = settings_path_for_dir_name("Code");
        let cursor = settings_path_for_dir_name("Cursor");
        assert_ne!(code, cursor);
        assert!(code.ends_with("Code/User/settings.json"));
    }
}
