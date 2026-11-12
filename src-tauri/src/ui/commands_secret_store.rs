//! Local administration of client-owned reversible secret originals.
use lr_api_keys::{keychain_trait::redacted_secret_storage, KeychainStorage};
use lr_config::ConfigManager;
use lr_secret_scanner::placeholders::{StoredSecret, Vault, RETENTION_SECS};
use serde::Serialize;
use std::{collections::BTreeSet, sync::Arc};
use tauri::{Emitter, State};

#[derive(Serialize)]
pub struct RedactedSecretEntry {
    pub client_id: String,
    pub client_name: String,
    pub ignored: bool,
    #[serde(flatten)]
    pub secret: StoredSecret,
}

#[derive(Serialize)]
pub struct RedactedSecretInventory {
    pub entries: Vec<RedactedSecretEntry>,
    pub persistence_warning: Option<String>,
}

fn owners(
    storage: &dyn KeychainStorage,
    clients: &[(String, String)],
    selected: Option<String>,
) -> Result<BTreeSet<String>, String> {
    let mut owners: BTreeSet<_> = clients.iter().map(|(id, _)| id.clone()).collect();
    if selected.as_ref().is_none_or(|id| !owners.contains(id)) {
        owners.extend(Vault::stored_owners(storage).map_err(|e| e.to_string())?);
    }
    if let Some(selected) = selected {
        if !owners.contains(&selected) {
            return Err("Client was not found in the secret store".into());
        }
        Ok(BTreeSet::from([selected]))
    } else {
        Ok(owners)
    }
}
fn clients(config: &ConfigManager) -> Vec<(String, String)> {
    config
        .get()
        .clients
        .iter()
        .map(|client| (client.id.clone(), client.name.clone()))
        .collect()
}
fn validate_reference(id: &str) -> Result<(), String> {
    if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Invalid secret reference ID".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn list_redacted_secrets(
    client_id: Option<String>,
    config_manager: State<'_, ConfigManager>,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<RedactedSecretInventory, String> {
    let clients = clients(&config_manager);
    let state = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let storage = redacted_secret_storage();
        let owners = owners(storage.as_ref(), &clients, client_id)?;
        let mut entries = Vec::new();
        for owner in owners {
            let name = clients
                .iter()
                .find(|(id, _)| id == &owner)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| "Removed client".into());
            let vault = Vault::for_client(storage.clone(), &owner);
            lr_server::secret_policies::attach(&state, &vault, &owner);
            let ignored = vault.mask_exempt_references().map_err(|e| e.to_string())?;
            for secret in Vault::list_client(storage.clone(), &owner).map_err(|e| e.to_string())? {
                entries.push(RedactedSecretEntry {
                    client_id: owner.clone(),
                    client_name: name.clone(),
                    ignored: ignored.contains(&secret.reference_id),
                    secret,
                });
            }
        }
        entries.sort_by(|a, b| {
            (&a.client_name, &a.client_id, &a.secret.reference_id).cmp(&(
                &b.client_name,
                &b.client_id,
                &b.secret.reference_id,
            ))
        });
        Ok(RedactedSecretInventory {
            entries,
            persistence_warning: storage.persistence_details().map(|notice| {
                format!(
                    "{} Pending records: {}. The inventory may exclude unreadable saved records.",
                    notice.message(),
                    notice.pending_entries
                )
            }),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn reveal_redacted_secret(
    client_id: String,
    reference_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<String, String> {
    validate_reference(&reference_id)?;
    let clients = clients(&config_manager);
    tokio::task::spawn_blocking(move || {
        let storage = redacted_secret_storage();
        owners(storage.as_ref(), &clients, Some(client_id.clone()))?;
        let vault = Vault::find_client(storage, &client_id, RETENTION_SECS)
            .map_err(|e| e.to_string())?
            .ok_or("Secret mapping has expired or was purged")?;
        vault
            .reveal_reference(&reference_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Secret mapping has expired or was purged".into())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Persist an allow decision while retaining every issued restoration reference.
#[tauri::command]
pub async fn mark_redacted_secret_not_secret(
    client_id: String,
    reference_id: String,
    config_manager: State<'_, ConfigManager>,
    state: State<'_, Arc<lr_server::state::AppState>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    validate_reference(&reference_id)?;
    if config_manager.client_secret_scanning(&client_id).is_none() {
        return Err("Client not found".into());
    }
    let owner = client_id.clone();
    let finding = tokio::task::spawn_blocking(move || {
        let storage = redacted_secret_storage();
        let vault = Vault::find_client(storage.clone(), &owner, RETENTION_SECS)
            .map_err(|e| e.to_string())?
            .ok_or("Secret mapping has expired or was purged")?;
        let entry = Vault::list_client(storage, &owner)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|entry| entry.reference_id == reference_id && !entry.expired && !entry.revoked)
            .ok_or("Secret mapping has expired or was purged")?;
        let original = vault
            .reveal_reference(&reference_id)
            .map_err(|e| e.to_string())?
            .ok_or("Secret mapping has expired or was purged")?;
        Ok::<_, String>(lr_server::secret_policies::RememberedValue::new(
            original,
            "stored-secret-reference".into(),
            "Marked as not a secret".into(),
            entry.masked_value,
        ))
    })
    .await
    .map_err(|_| "Could not read the stored value".to_string())??;
    lr_server::secret_policies::remember(
        &config_manager,
        &state.secret_dismissal_cache,
        &client_id,
        finding,
        false,
    )
    .await?;
    let _ = app.emit("clients-changed", ());
    let _ = app.emit("secret-store-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn purge_redacted_secrets(
    client_id: Option<String>,
    reference_id: Option<String>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<usize, String> {
    if let Some(reference_id) = &reference_id {
        validate_reference(reference_id)?;
        if client_id.is_none() {
            return Err("Select a client to purge a single secret".into());
        }
    }
    let clients = clients(&config_manager);
    let result = tokio::task::spawn_blocking(move || {
        let storage: Arc<dyn KeychainStorage> = redacted_secret_storage();
        let owners = owners(storage.as_ref(), &clients, client_id)?;
        let mut removed = 0;
        for owner in owners {
            removed += Vault::purge_client(storage.clone(), &owner, reference_id.as_deref())
                .map_err(|e| {
                    format!("Purge stopped: {e}. Already removed {removed} references.")
                })?;
        }
        confirm_purge(storage.as_ref(), removed)
    })
    .await
    .map_err(|e| e.to_string())?;
    let _ = app.emit("secret-store-changed", ());
    result
}

fn confirm_purge(storage: &dyn KeychainStorage, removed: usize) -> Result<usize, String> {
    if storage.persistence_warning() {
        let cause = storage
            .persistence_details()
            .map(|notice| format!(" {}", notice.message()))
            .unwrap_or_default();
        return Err(format!("Removed {removed} in-memory references, but durable deletion cannot be confirmed while credential storage is unavailable. Keep LocalRouter running; automatic recovery will retry the pending deletion.{cause}"));
    }
    Ok(removed)
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Volatile;
    impl KeychainStorage for Volatile {
        fn persistence_warning(&self) -> bool {
            true
        }
        fn get(&self, _: &str, _: &str) -> lr_types::AppResult<Option<String>> {
            Ok(None)
        }
        fn store(&self, _: &str, _: &str, _: &str) -> lr_types::AppResult<()> {
            Ok(())
        }
        fn delete(&self, _: &str, _: &str) -> lr_types::AppResult<()> {
            Ok(())
        }
    }
    #[test]
    fn purge_does_not_claim_durable_success_during_a_storage_outage() {
        assert!(confirm_purge(&Volatile, 2)
            .unwrap_err()
            .contains("durable deletion cannot be confirmed"));
        assert_eq!(
            confirm_purge(&lr_api_keys::MockKeychain::new(), 2).unwrap(),
            2
        );
    }
}
