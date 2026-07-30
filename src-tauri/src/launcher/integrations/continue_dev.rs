//! Continue (VS Code / JetBrains extension) proxy configuration.
//!
//! Continue is the best-behaved tool of the set: it does its own HTTP setup
//! rather than relying on the editor's patched stack, and its `caBundlePath`
//! is genuinely **additive** — the cert is appended to Node's bundled roots
//! rather than replacing them. Proxy and CA are configured per model via
//! `requestOptions`, so we can scope the change to just the models whose
//! traffic the proxy actually intercepts and leave everything else alone.
//!
//! Honest caveat carried in the UI: Continue is API-key only with a settable
//! `apiBase`, so gateway mode already covers it completely. Proxy mode is
//! offered for uniform observability without per-provider setup, not because
//! anything is otherwise invisible.

use std::path::PathBuf;

/// Continue's global config file.
pub fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".continue")
        .join("config.yaml")
}

/// Model providers whose default endpoints the proxy intercepts. Models on
/// other providers are left untouched: routing them through the proxy would
/// just add a blind tunnel hop.
const INTERCEPTED_PROVIDERS: &[&str] = &["anthropic", "openai"];

/// The config fragment, for the manual instructions.
pub fn proxy_fragment(proxy_url: &str, ca_cert_path: &str) -> String {
    format!(
        "models:\n  - name: <your anthropic/openai model>\n    requestOptions:\n      proxy: {proxy_url}\n      caBundlePath: {ca_cert_path}\n"
    )
}

/// Add `requestOptions.proxy` / `requestOptions.caBundlePath` to every model
/// entry on an intercepted provider, preserving all other config.
///
/// Returns the new YAML and how many model entries were updated — zero means
/// the user has no Anthropic/OpenAI models configured, which the caller
/// surfaces rather than reporting a hollow success.
pub fn merge_proxy_config(
    existing: &str,
    proxy_url: &str,
    ca_cert_path: &str,
) -> Result<(String, usize), String> {
    let mut config: serde_yaml::Value = if existing.trim().is_empty() {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    } else {
        serde_yaml::from_str(existing).map_err(|e| format!("Failed to parse config.yaml: {e}"))?
    };
    if !config.is_mapping() {
        return Err("config.yaml is not a mapping".to_string());
    }

    let mut updated = 0usize;
    if let Some(models) = config
        .as_mapping_mut()
        .and_then(|m| m.get_mut(serde_yaml::Value::String("models".to_string())))
        .and_then(|m| m.as_sequence_mut())
    {
        for model in models.iter_mut() {
            let is_intercepted = model
                .as_mapping()
                .and_then(|m| m.get(serde_yaml::Value::String("provider".to_string())))
                .and_then(|p| p.as_str())
                .map(|p| INTERCEPTED_PROVIDERS.contains(&p))
                .unwrap_or(false);
            if !is_intercepted {
                continue;
            }
            let Some(map) = model.as_mapping_mut() else {
                continue;
            };
            let req = map
                .entry(serde_yaml::Value::String("requestOptions".to_string()))
                .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
            // A non-mapping requestOptions is malformed config; replace it
            // rather than silently skipping the model.
            if !req.is_mapping() {
                *req = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
            }
            if let Some(req_map) = req.as_mapping_mut() {
                req_map.insert(
                    serde_yaml::Value::String("proxy".to_string()),
                    serde_yaml::Value::String(proxy_url.to_string()),
                );
                req_map.insert(
                    serde_yaml::Value::String("caBundlePath".to_string()),
                    serde_yaml::Value::String(ca_cert_path.to_string()),
                );
            }
            updated += 1;
        }
    }

    let out = serde_yaml::to_string(&config)
        .map_err(|e| format!("Failed to serialize config.yaml: {e}"))?;
    Ok((out, updated))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
name: my-config
models:
  - name: Claude
    provider: anthropic
    model: claude-sonnet-4
    apiKey: sk-ant-1
  - name: Local Llama
    provider: ollama
    model: llama3
"#;

    #[test]
    fn adds_request_options_to_intercepted_models_only() {
        let (out, updated) = merge_proxy_config(CONFIG, "http://p", "/ca.pem").unwrap();
        assert_eq!(updated, 1);
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        let models = v["models"].as_sequence().unwrap();
        assert_eq!(models[0]["requestOptions"]["proxy"], "http://p");
        assert_eq!(models[0]["requestOptions"]["caBundlePath"], "/ca.pem");
        // The ollama model is untouched — proxying it would add nothing.
        assert!(models[1].get("requestOptions").is_none());
    }

    #[test]
    fn preserves_unrelated_config_and_model_fields() {
        let (out, _) = merge_proxy_config(CONFIG, "http://p", "/ca.pem").unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["name"], "my-config");
        assert_eq!(v["models"][0]["apiKey"], "sk-ant-1");
        assert_eq!(v["models"][0]["model"], "claude-sonnet-4");
    }

    #[test]
    fn merges_into_existing_request_options() {
        let cfg = r#"
models:
  - name: GPT
    provider: openai
    requestOptions:
      timeout: 30
"#;
        let (out, updated) = merge_proxy_config(cfg, "http://p", "/ca.pem").unwrap();
        assert_eq!(updated, 1);
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["models"][0]["requestOptions"]["timeout"], 30);
        assert_eq!(v["models"][0]["requestOptions"]["proxy"], "http://p");
    }

    #[test]
    fn reports_zero_when_no_intercepted_models_exist() {
        let cfg = "models:\n  - name: L\n    provider: ollama\n";
        let (_, updated) = merge_proxy_config(cfg, "http://p", "/ca.pem").unwrap();
        assert_eq!(updated, 0);
    }

    #[test]
    fn handles_config_with_no_models_key() {
        let (_, updated) = merge_proxy_config("name: empty\n", "http://p", "/ca.pem").unwrap();
        assert_eq!(updated, 0);
    }

    #[test]
    fn rerunning_does_not_duplicate_keys() {
        let (first, _) = merge_proxy_config(CONFIG, "http://a", "/one.pem").unwrap();
        let (second, updated) = merge_proxy_config(&first, "http://b", "/two.pem").unwrap();
        assert_eq!(updated, 1);
        let v: serde_yaml::Value = serde_yaml::from_str(&second).unwrap();
        assert_eq!(v["models"][0]["requestOptions"]["proxy"], "http://b");
        assert_eq!(v["models"][0]["requestOptions"]["caBundlePath"], "/two.pem");
    }

    #[test]
    fn malformed_yaml_is_an_error_not_a_silent_overwrite() {
        assert!(merge_proxy_config("::: nope :::", "http://p", "/ca.pem").is_err());
    }
}

/// Strip the proxy keys from every model's `requestOptions`, dropping the
/// block entirely when nothing else was in it. Returns the new YAML and how
/// many models were changed.
pub fn remove_proxy_config(existing: &str) -> Result<(String, usize), String> {
    let mut config: serde_yaml::Value = if existing.trim().is_empty() {
        return Ok((existing.to_string(), 0));
    } else {
        serde_yaml::from_str(existing).map_err(|e| format!("Failed to parse config.yaml: {e}"))?
    };

    let mut removed = 0usize;
    if let Some(models) = config
        .as_mapping_mut()
        .and_then(|m| m.get_mut(serde_yaml::Value::String("models".to_string())))
        .and_then(|m| m.as_sequence_mut())
    {
        for model in models.iter_mut() {
            let Some(map) = model.as_mapping_mut() else {
                continue;
            };
            let key = serde_yaml::Value::String("requestOptions".to_string());
            let Some(req) = map.get_mut(&key).and_then(|r| r.as_mapping_mut()) else {
                continue;
            };
            let had = req
                .remove(serde_yaml::Value::String("proxy".to_string()))
                .is_some()
                | req
                    .remove(serde_yaml::Value::String("caBundlePath".to_string()))
                    .is_some();
            if had {
                removed += 1;
            }
            // Don't leave an empty requestOptions behind.
            if req.is_empty() {
                map.remove(&key);
            }
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
    fn removes_proxy_keys_and_drops_empty_request_options() {
        let cfg = "models:\n  - name: C\n    provider: anthropic\n";
        let (applied, _) = merge_proxy_config(cfg, "http://p", "/ca.pem").unwrap();
        let (undone, removed) = remove_proxy_config(&applied).unwrap();
        assert_eq!(removed, 1);
        let v: serde_yaml::Value = serde_yaml::from_str(&undone).unwrap();
        assert!(v["models"][0].get("requestOptions").is_none());
        assert_eq!(v["models"][0]["provider"], "anthropic");
    }

    #[test]
    fn keeps_unrelated_request_options() {
        let cfg =
            "models:\n  - name: G\n    provider: openai\n    requestOptions:\n      timeout: 30\n";
        let (applied, _) = merge_proxy_config(cfg, "http://p", "/ca.pem").unwrap();
        let (undone, _) = remove_proxy_config(&applied).unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&undone).unwrap();
        assert_eq!(v["models"][0]["requestOptions"]["timeout"], 30);
        assert!(v["models"][0]["requestOptions"].get("proxy").is_none());
    }

    #[test]
    fn undo_is_a_noop_when_nothing_was_applied() {
        let cfg = "models:\n  - name: L\n    provider: ollama\n";
        let (_, removed) = remove_proxy_config(cfg).unwrap();
        assert_eq!(removed, 0);
    }
}
