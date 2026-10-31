//! Durable UI state (folded sections, dock side, filters) kept in
//! `ui_state.json` beside the config. See `lr_config::UiStateStore`.

use lr_config::UiStateStore;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use tauri::State;

/// Every stored UI state entry.
#[tauri::command]
pub async fn get_ui_state(
    store: State<'_, Arc<UiStateStore>>,
) -> Result<BTreeMap<String, Value>, String> {
    Ok(store.get_all())
}

/// Stores one UI state value (JSON `null` removes the key). The file is
/// written before this returns.
#[tauri::command]
pub async fn set_ui_state(
    key: String,
    value: Value,
    store: State<'_, Arc<UiStateStore>>,
) -> Result<(), String> {
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move || store.set(&key, value))
        .await
        .map_err(|e| format!("UI state write task failed: {e}"))?
}
