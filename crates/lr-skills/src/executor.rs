//! Script execution for skills
//!
//! Supports synchronous and asynchronous script execution with
//! output capture and timeout enforcement.

use super::types::{AsyncScriptStatus, ScriptRunResult};
use dashmap::DashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Maximum timeout for synchronous scripts (seconds)
const MAX_SYNC_TIMEOUT: u64 = 20;

/// Maximum timeout for async scripts (seconds)
const MAX_ASYNC_TIMEOUT: u64 = 3600;

/// Default tail lines for output
const DEFAULT_TAIL: usize = 30;

/// Tracked async process
struct TrackedProcess {
    running: Arc<RwLock<bool>>,
    exit_code: Arc<RwLock<Option<i32>>>,
    timed_out: Arc<RwLock<bool>>,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

/// Script executor for skills
pub struct ScriptExecutor {
    /// Tracked async processes: pid -> process info
    tracked: Arc<DashMap<u32, TrackedProcess>>,
}

impl Default for ScriptExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptExecutor {
    pub fn new() -> Self {
        Self {
            tracked: Arc::new(DashMap::new()),
        }
    }

    /// Infer the command interpreter for a script based on file extension
    fn infer_command(script_path: &Path) -> String {
        let ext = script_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        match ext.as_str() {
            "py" => "python3".to_string(),
            "sh" => "bash".to_string(),
            "js" | "mjs" => "node".to_string(),
            "ts" => "npx".to_string(),
            "rb" => "ruby".to_string(),
            "pl" => "perl".to_string(),
            _ => "bash".to_string(),
        }
    }

    /// Validate that the script path is within the skill's scripts/ directory
    fn validate_script_path(skill_dir: &Path, script: &str) -> Result<PathBuf, String> {
        let script_path = skill_dir.join(script);

        // Canonicalize both paths
        let canonical = script_path
            .canonicalize()
            .map_err(|e| format!("Script not found: {}: {}", script, e))?;

        let scripts_dir = skill_dir.join("scripts");
        if !scripts_dir.exists() {
            return Err(format!(
                "No scripts/ directory in skill at {}",
                skill_dir.display()
            ));
        }

        let scripts_canonical = scripts_dir
            .canonicalize()
            .map_err(|e| format!("Failed to resolve scripts directory: {}", e))?;

        if !canonical.starts_with(&scripts_canonical) {
            return Err(format!(
                "Script '{}' is outside the scripts/ directory",
                script
            ));
        }

        if !canonical.is_file() {
            return Err(format!("Script '{}' is not a file", script));
        }

        Ok(canonical)
    }

    /// Run a script synchronously with timeout
    pub async fn run_sync(
        &self,
        skill_dir: &Path,
        script: &str,
        command: Option<&str>,
        timeout_secs: Option<u64>,
        tail: Option<usize>,
    ) -> Result<ScriptRunResult, String> {
        let script_path = Self::validate_script_path(skill_dir, script)?;
        let timeout =
            std::time::Duration::from_secs(timeout_secs.unwrap_or(10).min(MAX_SYNC_TIMEOUT));
        let tail_lines = tail.unwrap_or(DEFAULT_TAIL);

        let cmd = command
            .map(|c| c.to_string())
            .unwrap_or_else(|| Self::infer_command(&script_path));

        let mut args = Vec::new();
        // For npx with TypeScript, add tsx
        if cmd == "npx" {
            args.push("tsx".to_string());
        }
        args.push(script_path.display().to_string());

        info!(
            "Running sync script: {} {:?} (timeout: {:?})",
            cmd, args, timeout
        );

        let child = Command::new(&cmd)
            .args(&args)
            .current_dir(skill_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to spawn '{}': {}", cmd, e))?;

        let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = tail_string(&String::from_utf8_lossy(&output.stdout), tail_lines);
                let stderr = tail_string(&String::from_utf8_lossy(&output.stderr), tail_lines);

                Ok(ScriptRunResult {
                    exit_code: output.status.code(),
                    stdout,
                    stderr,
                    timed_out: false,
                })
            }
            Ok(Err(e)) => Err(format!("Script execution failed: {}", e)),
            Err(_) => {
                // Timeout - process is killed on drop via kill_on_drop(true)
                Ok(ScriptRunResult {
                    exit_code: None,
                    stdout: String::new(),
                    stderr: format!("Process timed out after {} seconds", timeout.as_secs()),
                    timed_out: true,
                })
            }
        }
    }

    /// Run a script asynchronously, returning a tracking PID
    pub async fn run_async(
        &self,
        skill_dir: &Path,
        script: &str,
        command: Option<&str>,
        timeout_secs: Option<u64>,
    ) -> Result<u32, String> {
        let script_path = Self::validate_script_path(skill_dir, script)?;
        let timeout =
            std::time::Duration::from_secs(timeout_secs.unwrap_or(300).min(MAX_ASYNC_TIMEOUT));

        let cmd = command
            .map(|c| c.to_string())
            .unwrap_or_else(|| Self::infer_command(&script_path));

        let mut args = Vec::new();
        if cmd == "npx" {
            args.push("tsx".to_string());
        }
        args.push(script_path.display().to_string());

        // Create output directory
        let temp_base = std::env::temp_dir().join("localrouter-skills");
        let _ = std::fs::create_dir_all(&temp_base);

        let child = Command::new(&cmd)
            .args(&args)
            .current_dir(skill_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(false)
            .spawn()
            .map_err(|e| format!("Failed to spawn '{}': {}", cmd, e))?;

        let pid = child.id().ok_or("Failed to get process ID")?;

        let pid_dir = temp_base.join(pid.to_string());
        let _ = std::fs::create_dir_all(&pid_dir);
        let stdout_path = pid_dir.join("stdout.txt");
        let stderr_path = pid_dir.join("stderr.txt");

        let running = Arc::new(RwLock::new(true));
        let exit_code: Arc<RwLock<Option<i32>>> = Arc::new(RwLock::new(None));
        let timed_out = Arc::new(RwLock::new(false));

        self.tracked.insert(
            pid,
            TrackedProcess {
                running: running.clone(),
                exit_code: exit_code.clone(),
                timed_out: timed_out.clone(),
                stdout_path: stdout_path.clone(),
                stderr_path: stderr_path.clone(),
            },
        );

        // Spawn background task to monitor and capture output
        let tracked = self.tracked.clone();
        tokio::spawn(async move {
            let mut child = child;

            // Capture stdout and stderr
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();

            let stdout_path_clone = stdout_path.clone();
            let stderr_path_clone = stderr_path.clone();

            let stdout_task = tokio::spawn(async move {
                if let Some(mut reader) = stdout {
                    let mut buf = Vec::new();
                    let _ = reader.read_to_end(&mut buf).await;
                    let _ = std::fs::write(&stdout_path_clone, &buf);
                }
            });

            let stderr_task = tokio::spawn(async move {
                if let Some(mut reader) = stderr {
                    let mut buf = Vec::new();
                    let _ = reader.read_to_end(&mut buf).await;
                    let _ = std::fs::write(&stderr_path_clone, &buf);
                }
            });

            // Wait for process with timeout
            let wait_result = tokio::time::timeout(timeout, child.wait()).await;

            // Wait for output capture to complete
            let _ = stdout_task.await;
            let _ = stderr_task.await;

            match wait_result {
                Ok(Ok(status)) => {
                    *exit_code.write().await = status.code();
                }
                Ok(Err(e)) => {
                    warn!("Async script {} error: {}", pid, e);
                }
                Err(_) => {
                    // Timeout
                    let _ = child.kill().await;
                    *timed_out.write().await = true;
                    warn!("Async script {} timed out", pid);
                }
            }

            *running.write().await = false;

            // Clean up after a delay
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            tracked.remove(&pid);
        });

        info!("Started async script with PID {}", pid);
        Ok(pid)
    }

    /// Get the status of an async script
    pub async fn get_async_status(
        &self,
        pid: u32,
        tail: Option<usize>,
    ) -> Result<AsyncScriptStatus, String> {
        let tracked = self
            .tracked
            .get(&pid)
            .ok_or_else(|| format!("No tracked process with PID {}", pid))?;

        let tail_lines = tail.unwrap_or(DEFAULT_TAIL);
        let running = *tracked.running.read().await;
        let exit_code = *tracked.exit_code.read().await;
        let timed_out = *tracked.timed_out.read().await;

        let stdout = read_tail_file(&tracked.stdout_path, tail_lines);
        let stderr = read_tail_file(&tracked.stderr_path, tail_lines);

        Ok(AsyncScriptStatus {
            pid,
            running,
            exit_code,
            stdout,
            stderr,
            timed_out,
        })
    }
}

/// Get the last N lines of a string
fn tail_string(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    if lines.len() <= n {
        s.to_string()
    } else {
        lines[lines.len() - n..].join("\n")
    }
}

/// Read the last N lines from a file
fn read_tail_file(path: &Path, n: usize) -> String {
    match std::fs::read_to_string(path) {
        Ok(content) => tail_string(&content, n),
        Err(_) => String::new(),
    }
}
