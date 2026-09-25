//! Tauri commands for Local Embedded providers' engines: detection, install
//! options (package-manager commands, or LocalRouter's managed download of
//! stable-diffusion.cpp), and the supervised engine processes.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use lr_engines::{
    EngineProcessInfo, EngineStatus, InstallRunner, InstallSink, OutputStream, RecipeId, Supervisor,
};

fn parse_recipe(recipe_id: &str) -> Result<RecipeId, String> {
    RecipeId::parse(recipe_id).ok_or_else(|| format!("Unknown engine '{recipe_id}'"))
}

/// Detect an engine (chosen file, managed install, then PATH) and list the
/// install options for this OS.
///
/// `refresh` re-reads PATH first (after the user installed something).
#[tauri::command]
pub async fn engine_status(
    recipe_id: String,
    binary_path: Option<String>,
    refresh: Option<bool>,
) -> Result<EngineStatus, String> {
    let recipe = parse_recipe(&recipe_id)?;
    let override_path = binary_path
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .map(PathBuf::from);
    Ok(lr_engines::detect(recipe, override_path, refresh.unwrap_or(false)).await)
}

#[derive(Clone, Serialize)]
struct InstallOutputEvent<'a> {
    run_id: &'a str,
    stream: OutputStream,
    line: &'a str,
}

#[derive(Clone, Serialize)]
struct InstallFinishedEvent<'a> {
    run_id: &'a str,
    exit_code: Option<i32>,
    cancelled: bool,
    error: Option<String>,
}

struct TauriInstallSink(AppHandle);

impl InstallSink for TauriInstallSink {
    fn on_line(&self, run_id: &str, stream: OutputStream, line: &str) {
        let _ = self.0.emit(
            "engine-install-output",
            InstallOutputEvent {
                run_id,
                stream,
                line,
            },
        );
    }

    fn on_finished(
        &self,
        run_id: &str,
        exit_code: Option<i32>,
        cancelled: bool,
        error: Option<String>,
    ) {
        let _ = self.0.emit(
            "engine-install-finished",
            InstallFinishedEvent {
                run_id,
                exit_code,
                cancelled,
                error,
            },
        );
    }
}

/// Run one of an engine's install options (only options from the compiled
/// recipes; commands needing sudo are refused). Command options run through
/// the user's shell; download options fetch the latest release in-process
/// (the only network access, on this click). Output streams as
/// `engine-install-output` events; `engine-install-finished` ends the run
/// (`exit_code` 0 on success).
#[tauri::command]
pub async fn engine_install(
    recipe_id: String,
    option_id: String,
    app: AppHandle,
    runner: State<'_, Arc<InstallRunner>>,
) -> Result<String, String> {
    let recipe = parse_recipe(&recipe_id)?;
    runner
        .start(recipe, &option_id, Arc::new(TauriInstallSink(app)))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn engine_install_cancel(
    run_id: String,
    runner: State<'_, Arc<InstallRunner>>,
) -> Result<bool, String> {
    Ok(runner.cancel(&run_id))
}

/// Engine processes started by Local Embedded providers.
#[tauri::command]
pub async fn engine_processes(
    supervisor: State<'_, Arc<Supervisor>>,
) -> Result<Vec<EngineProcessInfo>, String> {
    Ok(supervisor.processes())
}

/// Recent output of one engine process.
#[tauri::command]
pub async fn engine_logs(
    key: String,
    supervisor: State<'_, Arc<Supervisor>>,
) -> Result<Vec<String>, String> {
    Ok(supervisor.logs(&key))
}

/// Stop one engine process (it starts again on the next request).
#[tauri::command]
pub async fn engine_stop(
    key: String,
    supervisor: State<'_, Arc<Supervisor>>,
) -> Result<(), String> {
    supervisor.stop(&key).await;
    Ok(())
}
