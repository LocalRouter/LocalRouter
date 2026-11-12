//! Client-owned permanent value decisions, independent of restoration retention.
use lr_config::{ConfigManager, DismissedSecret};
use lr_secret_scanner::{
    dismissal::{DismissalCache, DismissedDigest},
    placeholders::Vault,
};
use std::sync::Arc;
use zeroize::Zeroizing;

pub fn exemptions(
    config: &ConfigManager,
    cache: &DismissalCache,
    owner: &str,
    values: &[&str],
) -> Vec<bool> {
    let Some(policy) = config.client_secret_scanning(owner) else {
        return vec![false; values.len()];
    };
    let ignored: Vec<_> = policy
        .dismissed_secrets
        .iter()
        .filter(|entry| !entry.redact)
        .map(|entry| DismissedDigest {
            hash: &entry.hash,
            iterations: entry.iterations,
            hint: Some(&entry.hint),
        })
        .collect();
    if ignored.is_empty() {
        return vec![false; values.len()];
    }
    let redacted: Vec<_> = policy
        .dismissed_secrets
        .iter()
        .filter(|entry| entry.redact)
        .map(|entry| DismissedDigest {
            hash: &entry.hash,
            iterations: entry.iterations,
            hint: Some(&entry.hint),
        })
        .collect();
    values
        .iter()
        .map(|value| {
            !cache.is_dismissed(
                &format!("redact:{owner}"),
                policy.dismiss_salt.as_deref(),
                &redacted,
                value,
            ) && cache.is_dismissed(owner, policy.dismiss_salt.as_deref(), &ignored, value)
        })
        .collect()
}

pub fn attach(state: &crate::state::AppState, vault: &Vault, owner: &str) {
    let config = state.config_manager.clone();
    let cache = state.secret_dismissal_cache.clone();
    let owner = owner.to_owned();
    vault.set_mask_exemptions(Arc::new(move |values| {
        exemptions(&config, &cache, &owner, values)
    }));
}

pub struct RememberedValue {
    pub value: Zeroizing<String>,
    pub rule_id: String,
    pub rule_description: String,
    pub hint: String,
}

impl RememberedValue {
    pub fn new(value: String, rule_id: String, rule_description: String, hint: String) -> Self {
        Self {
            value: Zeroizing::new(value),
            rule_id,
            rule_description,
            hint,
        }
    }
}

pub async fn remember(
    config: &ConfigManager,
    cache: &DismissalCache,
    owner: &str,
    finding: RememberedValue,
    redact: bool,
) -> Result<(), String> {
    let fresh_salt = lr_secret_scanner::new_salt()?;
    let mut salt = None;
    config
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|client| client.id == owner) {
                salt = Some(
                    client
                        .secret_scanning
                        .dismiss_salt
                        .get_or_insert(fresh_salt)
                        .clone(),
                );
            }
        })
        .map_err(|e| e.to_string())?;
    let salt = salt.ok_or("Client not found")?;
    let previous = config
        .client_secret_scanning(owner)
        .ok_or("Client not found")?
        .dismissed_secrets;
    let iterations = lr_secret_scanner::DEFAULT_ITERATIONS;
    let hash_salt = salt.clone();
    let (hash, superseded) = tokio::task::spawn_blocking(move || {
        let hash = lr_secret_scanner::hash_secret(&finding.value, &hash_salt, iterations)?;
        let superseded: Vec<_> = previous
            .into_iter()
            .filter(|entry| {
                entry.hash == hash
                    || (entry.iterations != iterations
                        && lr_secret_scanner::verify_secret(
                            &finding.value,
                            &hash_salt,
                            entry.iterations,
                            &entry.hash,
                        ))
            })
            .map(|entry| entry.id)
            .collect();
        Ok::<_, String>((hash, superseded))
    })
    .await
    .map_err(|_| "Could not hash remembered value".to_string())??;
    let entry = DismissedSecret {
        redact,
        id: uuid::Uuid::new_v4().to_string(),
        hash,
        iterations,
        rule_id: finding.rule_id,
        rule_description: finding.rule_description,
        hint: finding.hint,
        dismissed_at: chrono::Utc::now().to_rfc3339(),
    };
    let mut found = false;
    let entry_id = entry.id.clone();
    let mut replaced = Vec::new();
    config
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|client| {
                client.id == owner && client.secret_scanning.dismiss_salt.as_deref() == Some(&salt)
            }) {
                found = true;
                client.secret_scanning.dismissed_secrets.retain(|old| {
                    let keep = old.hash != entry.hash && !superseded.contains(&old.id);
                    if !keep {
                        replaced.push(old.clone());
                    }
                    keep
                });
                client.secret_scanning.dismissed_secrets.push(entry);
            }
        })
        .map_err(|e| e.to_string())?;
    if !found {
        return Err("Client was removed or its secret policy changed; try again".into());
    }
    if let Err(error) = config.save().await {
        // Undo only this decision. A concurrent decision for the same value
        // supersedes its ID and must not be overwritten by this rollback.
        let _ = config.update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|client| client.id == owner) {
                let decisions = &mut client.secret_scanning.dismissed_secrets;
                if decisions.iter().any(|decision| decision.id == entry_id) {
                    decisions.retain(|decision| decision.id != entry_id);
                    decisions.extend(replaced);
                }
            }
        });
        cache.invalidate(owner);
        cache.invalidate(&format!("redact:{owner}"));
        return Err(format!(
            "Could not save the permanent decision; try again: {error}"
        ));
    }
    cache.invalidate(owner);
    cache.invalidate(&format!("redact:{owner}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_api_keys::{KeychainStorage, MockKeychain};
    use lr_config::{AppConfig, Client, Strategy};
    use lr_types::secret_context::SecretContext;
    use serde_json::json;

    const VALUE: &str = "synthetic-false-positive-r8P2mW9qV7xL4nS6";
    fn fixture(
        path: &std::path::Path,
    ) -> (Arc<ConfigManager>, String, String, Arc<DismissalCache>) {
        let mut config = AppConfig::default();
        let a = Client::new_with_strategy("A".into(), "strategy".into());
        let b = Client::new_with_strategy("B".into(), "strategy".into());
        let mut strategy = Strategy::new("Synthetic".into());
        strategy.id = "strategy".into();
        config.strategies = vec![strategy];
        config.clients = vec![a.clone(), b.clone()];
        (
            Arc::new(ConfigManager::new(config, path.into())),
            a.id,
            b.id,
            Arc::new(DismissalCache::new()),
        )
    }
    fn attach_test(
        config: Arc<ConfigManager>,
        cache: Arc<DismissalCache>,
        vault: &Vault,
        owner: &str,
    ) {
        let owner = owner.to_owned();
        vault.set_mask_exemptions(Arc::new(move |values| {
            exemptions(&config, &cache, &owner, values)
        }));
    }
    fn decision() -> RememberedValue {
        RememberedValue::new(
            VALUE.into(),
            "synthetic-rule".into(),
            "Synthetic rule".into(),
            "synthe…nS6".into(),
        )
    }

    #[tokio::test]
    async fn not_secret_stops_masking_without_revoking_inflight_references_or_other_clients() {
        let root = tempfile::tempdir().unwrap();
        let (config, a, b, cache) = fixture(&root.path().join("settings.yaml"));
        let storage: Arc<dyn KeychainStorage> = Arc::new(MockKeychain::new());
        let vault = Vault::for_client(storage.clone(), &a);
        attach_test(config.clone(), cache.clone(), &vault, &a);
        vault.register(VALUE).unwrap();
        let mut first = json!({"input":VALUE});
        vault.protect(&mut first).unwrap();
        let old = first["input"].as_str().unwrap().to_owned();
        assert_ne!(old, VALUE);
        remember(&config, &cache, &a, decision(), false)
            .await
            .unwrap();
        let mut next = json!({"input":VALUE});
        vault.protect(&mut next).unwrap();
        assert_eq!(next["input"], VALUE);
        assert_eq!(vault.restore_text(&old, true).unwrap(), VALUE);
        let fresh = Vault::find_client(
            storage.clone(),
            &a,
            lr_secret_scanner::placeholders::RETENTION_SECS,
        )
        .unwrap()
        .unwrap();
        attach_test(config.clone(), cache.clone(), &fresh, &a);
        let mut replay = json!({"input":old});
        fresh.protect(&mut replay).unwrap();
        fresh.restore(&mut replay, true).unwrap();
        assert_eq!(replay["input"], VALUE);
        let other = Vault::for_client(storage, &b);
        attach_test(config.clone(), cache, &other, &b);
        other.register(VALUE).unwrap();
        let mut other_body = json!({"input":VALUE});
        other.protect(&mut other_body).unwrap();
        assert_ne!(other_body["input"], VALUE);
        assert_eq!(other.restore_text(&old, false).unwrap(), old);
        let raw = std::fs::read_to_string(config.config_path()).unwrap();
        assert!(!raw.contains(VALUE));
        assert_eq!(
            config
                .client_secret_scanning(&a)
                .unwrap()
                .dismissed_secrets
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn permanent_decision_survives_restart_and_store_purge_and_can_be_removed_or_reversed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.yaml");
        let (config, a, _, cache) = fixture(&path);
        remember(&config, &cache, &a, decision(), false)
            .await
            .unwrap();
        let restarted = Arc::new(ConfigManager::load_from_path(path).await.unwrap());
        let restarted_cache = Arc::new(DismissalCache::new());
        assert_eq!(
            exemptions(&restarted, &restarted_cache, &a, &[VALUE, "different"]),
            vec![true, false]
        );
        let storage: Arc<dyn KeychainStorage> = Arc::new(
            lr_api_keys::keychain_trait::FileKeychain::new(root.path().join("originals.json"))
                .unwrap(),
        );
        let vault = Vault::for_client(storage.clone(), &a);
        attach_test(restarted.clone(), restarted_cache.clone(), &vault, &a);
        vault.register(VALUE).unwrap();
        let before = Vault::list_client(storage.clone(), &a).unwrap();
        let exempt = vault.mask_exempt_references().unwrap();
        assert_eq!(exempt.len(), 1);
        assert_eq!(
            before[0].last_used_at,
            Vault::list_client(storage.clone(), &a).unwrap()[0].last_used_at
        );
        Vault::purge_client(storage, &a, None).unwrap();
        assert_eq!(
            exemptions(&restarted, &restarted_cache, &a, &[VALUE]),
            vec![true]
        );
        remember(&restarted, &restarted_cache, &a, decision(), true)
            .await
            .unwrap();
        assert_eq!(
            exemptions(&restarted, &restarted_cache, &a, &[VALUE]),
            vec![false]
        );
        assert_eq!(
            restarted
                .client_secret_scanning(&a)
                .unwrap()
                .dismissed_secrets
                .len(),
            1
        );
        remember(&restarted, &restarted_cache, &a, decision(), false)
            .await
            .unwrap();
        restarted
            .update(|config| {
                config
                    .clients
                    .iter_mut()
                    .find(|c| c.id == a)
                    .unwrap()
                    .secret_scanning
                    .dismissed_secrets
                    .clear()
            })
            .unwrap();
        assert_eq!(
            exemptions(&restarted, &restarted_cache, &a, &[VALUE]),
            vec![false]
        );
    }

    #[tokio::test]
    async fn concurrent_dismissals_keep_one_salt_and_both_decisions() {
        let root = tempfile::tempdir().unwrap();
        let (config, a, _, cache) = fixture(&root.path().join("settings.yaml"));
        let other = RememberedValue::new(
            "synthetic-other-r9nP8mV2qT6s".into(),
            "r".into(),
            "Rule".into(),
            "masked".into(),
        );
        let (first, second) = tokio::join!(
            remember(&config, &cache, &a, decision(), false),
            remember(&config, &cache, &a, other, false)
        );
        first.unwrap();
        second.unwrap();
        assert_eq!(
            exemptions(
                &config,
                &cache,
                &a,
                &[VALUE, "synthetic-other-r9nP8mV2qT6s"]
            ),
            vec![true, true]
        );
        assert_eq!(
            config
                .client_secret_scanning(&a)
                .unwrap()
                .dismissed_secrets
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn missing_client_and_save_failure_never_remove_restoration_mappings() {
        let root = tempfile::tempdir().unwrap();
        let (config, a, _, cache) = fixture(root.path());
        let vault = Vault::for_client(Arc::new(MockKeychain::new()), &a);
        vault.register(VALUE).unwrap();
        let mut body = json!({"input":VALUE});
        vault.protect(&mut body).unwrap();
        let tag = body["input"].as_str().unwrap();
        assert!(remember(&config, &cache, "unknown", decision(), false)
            .await
            .is_err());
        assert!(remember(&config, &cache, &a, decision(), false)
            .await
            .is_err());
        assert!(config
            .client_secret_scanning(&a)
            .unwrap()
            .dismissed_secrets
            .is_empty());
        assert_eq!(vault.restore_text(tag, true).unwrap(), VALUE);
    }
}
