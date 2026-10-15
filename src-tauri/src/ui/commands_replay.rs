//! User-initiated local replay; no operation implicitly submits inference.
use lr_replay::{types::*, ReplayManager, RunContext};
use lr_server::manager::ServerManager;
use std::sync::Arc;
use tauri::State;
fn context(server: &ServerManager) -> Result<RunContext, String> {
    let state = server
        .get_state()
        .ok_or("Start the local server before running replay")?;
    let host = state.config_manager.get().server.host.clone();
    let host = match host.as_str() {
        "localhost" | "0.0.0.0" | "127.0.0.1" => "127.0.0.1".to_string(),
        "::" | "::1" | "[::1]" => "[::1]".to_string(),
        other => {
            let ip: std::net::IpAddr = other
                .parse()
                .map_err(|_| "Replay requires a loopback listener")?;
            if !ip.is_loopback() {
                return Err("Replay requires a loopback listener".into());
            }
            other.to_string()
        }
    };
    let port = server
        .get_actual_port()
        .ok_or("Server port is unavailable")?;
    RunContext::new(state, format!("http://{host}:{port}/"))
}
#[tauri::command]
pub fn replay_list(manager: State<'_, Arc<ReplayManager>>) -> Result<ArtifactList, String> {
    manager.store.list()
}
#[tauri::command]
pub fn replay_read(
    kind: ArtifactKind,
    id: String,
    manager: State<'_, Arc<ReplayManager>>,
) -> Result<SavedArtifact, String> {
    manager.store.read(kind, &id)
}
#[tauri::command]
pub fn replay_save_dataset(
    dataset: ReplayDataset,
    manager: State<'_, Arc<ReplayManager>>,
) -> Result<SavedArtifact, String> {
    manager.store.save_dataset(dataset)
}
#[tauri::command]
pub fn replay_delete(
    kind: ArtifactKind,
    id: String,
    revision: u64,
    manager: State<'_, Arc<ReplayManager>>,
) -> Result<(), String> {
    manager.delete_saved(kind, &id, revision)
}
#[tauri::command]
pub fn replay_start(
    request: ReplayRunRequest,
    manager: State<'_, Arc<ReplayManager>>,
    server: State<'_, Arc<ServerManager>>,
) -> Result<String, String> {
    manager.inner().start(context(&server)?, request)
}
#[tauri::command]
pub fn replay_runs(manager: State<'_, Arc<ReplayManager>>) -> Vec<ReplayRunSummary> {
    manager.runs()
}
#[tauri::command]
pub fn replay_status(
    id: String,
    manager: State<'_, Arc<ReplayManager>>,
) -> Result<ReplayRunSummary, String> {
    manager.status(&id)
}
#[tauri::command]
pub fn replay_result(
    id: String,
    index: usize,
    manager: State<'_, Arc<ReplayManager>>,
) -> Result<ReplayResult, String> {
    manager.result(&id, index)
}
#[tauri::command]
pub fn replay_get_run(
    id: String,
    manager: State<'_, Arc<ReplayManager>>,
) -> Result<ReplayRun, String> {
    manager.get(&id)
}
#[tauri::command]
pub fn replay_cancel(id: String, manager: State<'_, Arc<ReplayManager>>) -> Result<(), String> {
    manager.cancel(&id)
}
#[tauri::command]
pub fn replay_save_run(
    id: String,
    manager: State<'_, Arc<ReplayManager>>,
) -> Result<SavedArtifact, String> {
    manager.save_run(&id)
}
#[tauri::command]
pub fn replay_judge(
    id: String,
    model: String,
    rubric: String,
    indices: Vec<usize>,
    manager: State<'_, Arc<ReplayManager>>,
    server: State<'_, Arc<ServerManager>>,
) -> Result<String, String> {
    manager
        .inner()
        .judge(context(&server)?, &id, model, rubric, indices)
}
/// Import creates a fresh draft only. Saving and running remain separate actions.
#[tauri::command]
pub fn replay_import(json: String) -> Result<ReplayDataset, String> {
    if json.len() > 8 * 1024 * 1024 {
        return Err("Import exceeds 8 MiB".into());
    }
    let value: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let value = if value.get("schema_version").is_some() {
        let artifact: SavedArtifact = serde_json::from_value(value).map_err(|e| e.to_string())?;
        if artifact.schema_version != 1 {
            return Err("Unsupported import version".into());
        }
        artifact.value
    } else {
        value
    };
    let mut dataset = ReplayDataset::empty();
    dataset.name = value
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Imported dataset")
        .to_string();
    dataset.cases =
        serde_json::from_value(value.get("cases").cloned().ok_or("Import needs cases")?)
            .map_err(|e| e.to_string())?;
    for case in &mut dataset.cases {
        case.input = lr_replay::source::sanitize_input(&case.input)?;
    }
    lr_replay::source::validate_cases(&dataset.cases)?;
    Ok(dataset)
}

#[tauri::command]
pub fn replay_current_policy(
    client_id: String,
    config: State<'_, Arc<lr_config::ConfigManager>>,
) -> Result<Option<lr_config::RoutingPolicy>, String> {
    let cfg = config.get();
    let client = cfg
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or("Client not found")?;
    let strategy = cfg
        .strategies
        .iter()
        .find(|s| s.id == client.strategy_id)
        .ok_or("Strategy not found")?;
    Ok(strategy
        .auto_config
        .as_ref()
        .and_then(|a| a.routing_policy.clone()))
}

#[tauri::command]
pub fn replay_preview(
    request: ReplayRunRequest,
    manager: State<'_, Arc<ReplayManager>>,
    server: State<'_, Arc<ServerManager>>,
) -> Result<Vec<ReplayEstimate>, String> {
    manager.preview(context(&server)?, request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn source() -> serde_json::Value {
        json!({"name":"Imported","cases":[{"id":"one","name":"One","input":{"endpoint":"chat_completions","body":{"messages":[{"role":"user","content":"Hello"}],"api_key":"private"},"source_event_id":null},"checks":[]}]})
    }
    #[test]
    fn import_is_a_new_unsaved_sanitized_draft() {
        let a = replay_import(source().to_string()).unwrap();
        let b = replay_import(source().to_string()).unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!(a.revision, 0);
        assert!(a.saved_at.is_none());
        assert!(!serde_json::to_string(&a).unwrap().contains("private"));
    }
    #[test]
    fn import_rejects_incomplete_and_invalid_artifacts() {
        let mut value = source();
        value["cases"][0]["input"]["body"] = json!({"_truncated":true,"_preview":"Hello"});
        assert!(replay_import(value.to_string()).is_err());
        assert!(replay_import("{}".into()).is_err());
        let mut value = source();
        value["cases"] = json!([]);
        assert!(replay_import(value.to_string()).is_err());
    }
}

#[tauri::command]
pub fn replay_import_run(
    json: String,
    manager: State<'_, Arc<ReplayManager>>,
) -> Result<ReplayRunSummary, String> {
    manager.import_run(&json)
}
