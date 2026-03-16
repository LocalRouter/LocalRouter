//! Memsearch watch daemon lifecycle management.
//!
//! Each client gets its own `memsearch watch` process that monitors
//! the `sessions/` directory for changes and auto-indexes.

use std::path::Path;

use tokio::process::{Child, Command};

/// A managed memsearch watch process.
pub struct MemsearchDaemon {
    child: Option<Child>,
}

impl Default for MemsearchDaemon {
    fn default() -> Self {
        Self::new()
    }
}

impl MemsearchDaemon {
    pub fn new() -> Self {
        Self { child: None }
    }

    /// Start `memsearch watch` for the given sessions directory.
    pub async fn start(&mut self, sessions_dir: &Path) -> Result<(), String> {
        if self.is_running() {
            return Ok(());
        }

        let child = Command::new("memsearch")
            .arg("watch")
            .arg(sessions_dir.to_string_lossy().as_ref())
            .current_dir(sessions_dir.parent().unwrap_or(sessions_dir))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to start memsearch watch: {}", e))?;

        tracing::info!(
            "Started memsearch watch daemon (pid={}) for {}",
            child.id().unwrap_or(0),
            sessions_dir.display()
        );

        self.child = Some(child);
        Ok(())
    }

    /// Stop the watch daemon gracefully.
    pub async fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let pid = child.id().unwrap_or(0);
            // kill_on_drop is set, but let's be explicit
            if let Err(e) = child.kill().await {
                tracing::debug!("memsearch watch (pid={}) already exited: {}", pid, e);
            } else {
                tracing::info!("Stopped memsearch watch daemon (pid={})", pid);
            }
        }
    }

    /// Check if the daemon process is still running.
    pub fn is_running(&mut self) -> bool {
        if let Some(ref mut child) = self.child {
            // try_wait returns Ok(Some(status)) if exited, Ok(None) if still running
            match child.try_wait() {
                Ok(Some(_)) => {
                    // Process has exited
                    self.child = None;
                    false
                }
                Ok(None) => true,
                Err(_) => {
                    self.child = None;
                    false
                }
            }
        } else {
            false
        }
    }
}

impl Drop for MemsearchDaemon {
    fn drop(&mut self) {
        // kill_on_drop on the Child handles this, but let's log it
        if let Some(ref child) = self.child {
            tracing::debug!(
                "Dropping memsearch watch daemon (pid={})",
                child.id().unwrap_or(0)
            );
        }
    }
}
