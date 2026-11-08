//! Local administration of client-owned reversible secret originals.
use lr_api_keys::{keychain_trait::redacted_secret_storage, KeychainStorage};
use lr_config::ConfigManager;
use lr_secret_scanner::placeholders::{StoredSecret, Vault, RETENTION_SECS};
use lr_types::secret_context::SecretContext;
use serde::Serialize;
use std::{collections::BTreeSet, sync::Arc};
use tauri::{Emitter, State};

#[derive(Serialize)]
pub struct RedactedSecretEntry {
    pub client_id: String,
    pub client_name: String,
    #[serde(flatten)]
    pub secret: StoredSecret,
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
) -> Result<Vec<RedactedSecretEntry>, String> {
    let clients = clients(&config_manager);
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
            for secret in Vault::list_client(storage.clone(), &owner).map_err(|e| e.to_string())? {
                entries.push(RedactedSecretEntry {
                    client_id: owner.clone(),
                    client_name: name.clone(),
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
        Ok(entries)
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
            .restore_text(
                &format!("<localrouter-redacted id='{reference_id}'/>"),
                true,
            )
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
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
        Ok(removed)
    })
    .await
    .map_err(|e| e.to_string())?;
    let _ = app.emit("secret-store-changed", ());
    result
}
