//! Running a recipe's install command on the user's behalf, streaming its
//! output. Only commands compiled into the recipes can run: callers pick an
//! option by `(recipe, option id)`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;

use crate::platform::{Os, Platform};
use crate::process::host_command;
use crate::recipes::{recipe, RecipeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    Stdout,
    Stderr,
}

/// Receives install progress (wired to Tauri events by the app).
pub trait InstallSink: Send + Sync {
    fn on_line(&self, run_id: &str, stream: OutputStream, line: &str);
    /// `exit_code` is `None` when the process was killed or could not start.
    fn on_finished(
        &self,
        run_id: &str,
        exit_code: Option<i32>,
        cancelled: bool,
        error: Option<String>,
    );
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("unknown install option '{option}' for {recipe}")]
    UnknownOption {
        recipe: &'static str,
        option: String,
    },
    #[error("this command needs sudo; run it in a terminal instead")]
    NeedsSudo,
    #[error("{0}")]
    Unsupported(String),
}

#[derive(Default)]
pub struct InstallRunner {
    runs: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

impl InstallRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start the install command for `(recipe, option_id)`; returns a run id.
    pub fn start(
        &self,
        recipe_id: RecipeId,
        option_id: &str,
        sink: Arc<dyn InstallSink>,
    ) -> Result<String, InstallError> {
        let platform = Platform::current();
        let recipe = recipe(recipe_id, &platform);
        if let Some(reason) = recipe.unsupported_reason {
            return Err(InstallError::Unsupported(reason.to_string()));
        }
        let option = recipe
            .install
            .iter()
            .find(|o| o.id == option_id)
            .ok_or_else(|| InstallError::UnknownOption {
                recipe: recipe_id.as_str(),
                option: option_id.to_string(),
            })?;
        if option.needs_sudo {
            return Err(InstallError::NeedsSudo);
        }
        Ok(self.run_shell(option.command.clone(), platform.os, sink))
    }

    /// Run `command` through the user's shell. Kept separate from `start` so
    /// tests can run harmless commands; not exposed to the frontend.
    pub(crate) fn run_shell(&self, command: String, os: Os, sink: Arc<dyn InstallSink>) -> String {
        let run_id = uuid::Uuid::new_v4().to_string();
        let token = CancellationToken::new();
        self.runs.lock().insert(run_id.clone(), token.clone());
        let runs = self.runs.clone();
        let id = run_id.clone();
        tokio::spawn(async move {
            let result = run(&id, &command, os, &sink, token.clone()).await;
            runs.lock().remove(&id);
            match result {
                Ok(code) => sink.on_finished(&id, code, token.is_cancelled(), None),
                Err(e) => sink.on_finished(&id, None, token.is_cancelled(), Some(e)),
            }
        });
        run_id
    }

    /// Cancel a running install (kills its whole process group).
    pub fn cancel(&self, run_id: &str) -> bool {
        match self.runs.lock().get(run_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    pub fn is_running(&self, run_id: &str) -> bool {
        self.runs.lock().contains_key(run_id)
    }
}

fn shell_for(os: Os, command: &str) -> (PathBuf, Vec<String>) {
    match os {
        Os::Windows => (
            PathBuf::from("powershell"),
            vec![
                "-NoProfile".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-Command".into(),
                command.to_string(),
            ],
        ),
        // `host_command` already supplies the login shell's environment, so
        // a plain `sh` runs the command without sourcing the user's shell
        // startup files (whose warnings would read like install errors).
        Os::MacOs | Os::Linux => (
            PathBuf::from("/bin/sh"),
            vec!["-c".into(), command.to_string()],
        ),
    }
}

async fn run(
    run_id: &str,
    command: &str,
    os: Os,
    sink: &Arc<dyn InstallSink>,
    token: CancellationToken,
) -> Result<Option<i32>, String> {
    let (shell, args) = shell_for(os, command);
    // Package managers must not wait for input we can't give them.
    let env = vec![
        ("HOMEBREW_NO_ENV_HINTS".to_string(), "1".to_string()),
        ("NONINTERACTIVE".to_string(), "1".to_string()),
        ("CI".to_string(), "1".to_string()),
    ];
    let mut cmd = host_command(&shell, args, env);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", shell.display()))?;
    let pid = child.id();

    let pipe = |reader: Option<Box<dyn tokio::io::AsyncRead + Send + Unpin>>, stream| {
        let sink = sink.clone();
        let id = run_id.to_string();
        tokio::spawn(async move {
            let Some(reader) = reader else { return };
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                sink.on_line(&id, stream, &line);
            }
        })
    };
    let out = pipe(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
        OutputStream::Stdout,
    );
    let err = pipe(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
        OutputStream::Stderr,
    );

    let status = tokio::select! {
        status = child.wait() => status.map_err(|e| e.to_string())?,
        _ = token.cancelled() => {
            kill_tree(pid, &mut child).await;
            let _ = out.await;
            let _ = err.await;
            return Ok(None);
        }
    };
    let _ = out.await;
    let _ = err.await;
    Ok(status.code())
}

/// Kill the install and everything it started.
async fn kill_tree(pid: Option<u32>, child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = pid {
        // The child leads its own process group (process_group(0)).
        unsafe {
            libc::killpg(pid as libc::pid_t, libc::SIGTERM);
        }
        if tokio::time::timeout(std::time::Duration::from_secs(3), child.wait())
            .await
            .is_ok()
        {
            return;
        }
        unsafe {
            libc::killpg(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    if let Some(pid) = pid {
        let _ = tokio::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .output()
            .await;
    }
    let _ = child.kill().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Default)]
    struct Collect {
        lines: Mutex<Vec<(OutputStream, String)>>,
        finished: Mutex<Option<(Option<i32>, bool)>>,
        done: AtomicBool,
    }
    impl InstallSink for Collect {
        fn on_line(&self, _run_id: &str, stream: OutputStream, line: &str) {
            self.lines.lock().push((stream, line.to_string()));
        }
        fn on_finished(
            &self,
            _run_id: &str,
            code: Option<i32>,
            cancelled: bool,
            _e: Option<String>,
        ) {
            *self.finished.lock() = Some((code, cancelled));
            self.done.store(true, Ordering::SeqCst);
        }
    }

    async fn wait(sink: &Collect) {
        for _ in 0..200 {
            if sink.done.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("install run did not finish");
    }

    #[test]
    fn sudo_and_unknown_options_are_refused() {
        let runner = InstallRunner::new();
        let sink: Arc<dyn InstallSink> = Arc::new(Collect::default());
        assert!(matches!(
            runner.start(RecipeId::LlamaCpp, "does-not-exist", sink),
            Err(InstallError::UnknownOption { .. })
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn streams_output_and_exit_code() {
        let runner = InstallRunner::new();
        let sink = Arc::new(Collect::default());
        runner.run_shell(
            "echo hello; echo oops 1>&2; exit 3".to_string(),
            Os::Linux,
            sink.clone(),
        );
        wait(&sink).await;
        let lines = sink.lines.lock().clone();
        assert!(lines.contains(&(OutputStream::Stdout, "hello".to_string())));
        assert!(lines.contains(&(OutputStream::Stderr, "oops".to_string())));
        assert_eq!(*sink.finished.lock(), Some((Some(3), false)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_kills_the_process_group() {
        let runner = InstallRunner::new();
        let sink = Arc::new(Collect::default());
        let id = runner.run_shell(
            "sleep 30 & sleep 30; echo never".to_string(),
            Os::Linux,
            sink.clone(),
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(runner.is_running(&id));
        assert!(runner.cancel(&id));
        wait(&sink).await;
        assert_eq!(*sink.finished.lock(), Some((None, true)));
        assert!(!sink.lines.lock().iter().any(|(_, l)| l == "never"));
    }
}
