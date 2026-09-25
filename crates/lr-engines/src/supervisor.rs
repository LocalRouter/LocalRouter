//! Supervising engine processes: start on demand, wait until ready, route
//! requests, stop when idle, clean up after crashes and app restarts.
//!
//! Every engine binds `127.0.0.1` on a port we pick and requires a random API
//! key generated per launch and passed through the environment (never argv,
//! never logged).

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::oneshot;

use crate::process::host_command;

/// Lines of engine output kept in memory per process.
const LOG_RING_LINES: usize = 500;
/// Engine log files rotate at this size.
const LOG_FILE_MAX_BYTES: u64 = 10 * 1024 * 1024;
/// Unexpected exits allowed within [`FAILURE_WINDOW`] before giving up.
const MAX_FAILURES: usize = 3;
const FAILURE_WINDOW: Duration = Duration::from_secs(300);
/// Grace period between asking an engine to stop and killing it.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// How the engine learns its port.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PortArg {
    /// A command-line flag, e.g. `--port`.
    Flag(String),
    /// An environment variable, e.g. `LAYA_PORT`.
    Env(String),
}

/// Everything needed to launch one engine process.
#[derive(Debug, Clone)]
pub struct LaunchSpec {
    /// Unique key for this process, e.g. `llamacpp_embedded:qwen3-8b`.
    pub key: String,
    /// Human-readable name for logs and the UI.
    pub label: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub port: PortArg,
    /// Environment variable that receives the per-launch API key.
    pub api_key_env: String,
    /// Path polled with GET until it answers 2xx. Connection refused and
    /// 503 mean "still starting".
    pub ready_path: String,
    pub start_timeout: Duration,
    /// Stop after this long without requests. `None` keeps it running.
    pub idle_timeout: Option<Duration>,
}

impl LaunchSpec {
    /// Identity of the launch configuration (not of the running instance):
    /// a different fingerprint means the process must be restarted.
    fn fingerprint(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.program.hash(&mut h);
        self.args.hash(&mut h);
        self.env.hash(&mut h);
        self.port.hash(&mut h);
        self.api_key_env.hash(&mut h);
        h.finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("could not start {label}: {message}")]
    Spawn { label: String, message: String },
    #[error("{label} exited while starting (exit code {code:?}). Last output:\n{tail}")]
    ExitedDuringStart {
        label: String,
        code: Option<i32>,
        tail: String,
    },
    #[error("{label} did not become ready within {secs}s. Last output:\n{tail}")]
    StartTimeout {
        label: String,
        secs: u64,
        tail: String,
    },
    #[error("{label} failed {count} times in the last 5 minutes and was not restarted. Last error: {last}")]
    TooManyFailures {
        label: String,
        count: usize,
        last: String,
    },
}

/// A ready engine. Cheap to clone.
#[derive(Clone)]
pub struct EngineHandle {
    pub key: String,
    pub port: u16,
    api_key: String,
    in_flight: Arc<AtomicUsize>,
    last_used: Arc<Mutex<Instant>>,
}

impl std::fmt::Debug for EngineHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the API key.
        f.debug_struct("EngineHandle")
            .field("key", &self.key)
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl EngineHandle {
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Mark a request in flight until the lease is dropped, so idle stopping
    /// never kills an engine mid-request.
    pub fn lease(&self) -> Lease {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        *self.last_used.lock() = Instant::now();
        Lease {
            in_flight: self.in_flight.clone(),
            last_used: self.last_used.clone(),
        }
    }
}

pub struct Lease {
    in_flight: Arc<AtomicUsize>,
    last_used: Arc<Mutex<Instant>>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        *self.last_used.lock() = Instant::now();
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    Running,
    Exited,
    Failed,
}

/// A process as shown in the UI.
#[derive(Debug, Clone, Serialize)]
pub struct EngineProcessInfo {
    pub key: String,
    pub label: String,
    pub state: EngineState,
    pub port: Option<u16>,
    pub pid: Option<u32>,
    pub uptime_secs: Option<u64>,
    pub idle_secs: Option<u64>,
    pub in_flight: usize,
    pub restarts: usize,
    pub last_error: Option<String>,
}

struct Running {
    handle: EngineHandle,
    pid: Option<u32>,
    started_at: Instant,
    fingerprint: u64,
    kill: Option<oneshot::Sender<()>>,
    exited: Arc<AtomicBool>,
    exit_code: Arc<Mutex<Option<i32>>>,
    idle_timeout: Option<Duration>,
}

#[derive(Default)]
struct Slot {
    label: String,
    running: Option<Running>,
    failures: Vec<Instant>,
    restarts: usize,
    last_error: Option<String>,
    logs: Arc<Mutex<VecDeque<String>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PidRecord {
    key: String,
    pid: u32,
    exe: String,
}

pub struct Supervisor {
    slots: Mutex<HashMap<String, Slot>>,
    start_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    state_dir: PathBuf,
    log_dir: PathBuf,
    pid_file: PathBuf,
    http: reqwest::Client,
}

impl Supervisor {
    /// `state_dir` holds `logs/engines/` and `run/engines.json`. Engines left
    /// running by a previous session (crash, force quit) are killed here.
    pub fn new(state_dir: &Path) -> Arc<Self> {
        let log_dir = state_dir.join("logs").join("engines");
        let pid_file = state_dir.join("run").join("engines.json");
        let _ = std::fs::create_dir_all(&log_dir);
        if let Some(parent) = pid_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        kill_orphans(&pid_file);
        Arc::new(Self {
            slots: Mutex::new(HashMap::new()),
            start_locks: Mutex::new(HashMap::new()),
            state_dir: state_dir.to_path_buf(),
            log_dir,
            pid_file,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(3))
                .no_proxy()
                .build()
                .unwrap_or_default(),
        })
    }

    /// The directory given to [`Supervisor::new`].
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Return the running engine for `spec.key`, starting (or restarting on a
    /// changed configuration or after a crash) as needed. Concurrent callers
    /// for the same key share one start.
    pub async fn ensure(&self, spec: LaunchSpec) -> Result<EngineHandle, EngineError> {
        let lock = self
            .start_locks
            .lock()
            .entry(spec.key.clone())
            .or_default()
            .clone();
        let _guard = lock.lock().await;
        let fingerprint = spec.fingerprint();

        let stale = {
            let mut slots = self.slots.lock();
            let slot = slots.entry(spec.key.clone()).or_default();
            slot.label = spec.label.clone();
            match &slot.running {
                Some(r) if !r.exited.load(Ordering::SeqCst) && r.fingerprint == fingerprint => {
                    return Ok(r.handle.clone());
                }
                Some(r) if r.exited.load(Ordering::SeqCst) => {
                    // Crashed since the last request.
                    let code = *r.exit_code.lock();
                    slot.failures.push(Instant::now());
                    slot.last_error = Some(format!("exited with code {code:?}"));
                    slot.running.take();
                    None
                }
                Some(_) => slot.running.take(), // configuration changed
                None => None,
            }
        };
        if let Some(old) = stale {
            self.terminate(old).await;
        }

        {
            let mut slots = self.slots.lock();
            let slot = slots.entry(spec.key.clone()).or_default();
            slot.failures.retain(|t| t.elapsed() < FAILURE_WINDOW);
            if slot.failures.len() >= MAX_FAILURES {
                return Err(EngineError::TooManyFailures {
                    label: spec.label.clone(),
                    count: slot.failures.len(),
                    last: slot.last_error.clone().unwrap_or_default(),
                });
            }
        }

        let logs = self
            .slots
            .lock()
            .entry(spec.key.clone())
            .or_default()
            .logs
            .clone();
        match self.start(&spec, fingerprint, logs).await {
            Ok(running) => {
                let handle = running.handle.clone();
                let mut slots = self.slots.lock();
                let slot = slots.entry(spec.key.clone()).or_default();
                if slot.last_error.is_some() || !slot.failures.is_empty() {
                    slot.restarts += 1;
                }
                slot.running = Some(running);
                Ok(handle)
            }
            Err(e) => {
                let mut slots = self.slots.lock();
                let slot = slots.entry(spec.key.clone()).or_default();
                slot.failures.push(Instant::now());
                slot.last_error = Some(e.to_string());
                Err(e)
            }
        }
    }

    async fn start(
        &self,
        spec: &LaunchSpec,
        fingerprint: u64,
        logs: Arc<Mutex<VecDeque<String>>>,
    ) -> Result<Running, EngineError> {
        let spawn_err = |message: String| EngineError::Spawn {
            label: spec.label.clone(),
            message,
        };
        let port = free_port().map_err(|e| spawn_err(format!("no free port: {e}")))?;
        let api_key = random_key();

        let mut args = spec.args.clone();
        let mut env = spec.env.clone();
        env.push((spec.api_key_env.clone(), api_key.clone()));
        match &spec.port {
            PortArg::Flag(flag) => {
                args.push(flag.clone());
                args.push(port.to_string());
            }
            PortArg::Env(var) => env.push((var.clone(), port.to_string())),
        }

        let mut cmd = host_command(&spec.program, args, env);
        // stdin stays open: engines that watch it can exit with us.
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| spawn_err(e.to_string()))?;
        let pid = child.id();
        push_log(
            &logs,
            format!(
                "[localrouter] started {} (pid {:?}, port {port})",
                spec.label, pid
            ),
        );

        let log_path = self.log_dir.join(format!("{}.log", sanitize(&spec.key)));
        for reader in [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
        ]
        .into_iter()
        .flatten()
        {
            let logs = logs.clone();
            let log_path = log_path.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(reader).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    append_log_file(&log_path, &line);
                    push_log(&logs, line);
                }
            });
        }

        let exited = Arc::new(AtomicBool::new(false));
        let exit_code = Arc::new(Mutex::new(None));
        let (kill_tx, kill_rx) = oneshot::channel::<()>();
        {
            let exited = exited.clone();
            let exit_code = exit_code.clone();
            let logs = logs.clone();
            let label = spec.label.clone();
            let pid_file = self.pid_file.clone();
            let key = spec.key.clone();
            let stdin = child.stdin.take();
            tokio::spawn(async move {
                let _stdin = stdin; // dropped when the process ends
                let status = tokio::select! {
                    status = child.wait() => status.ok(),
                    _ = kill_rx => {
                        graceful_stop(pid, &mut child).await;
                        child.wait().await.ok()
                    }
                };
                let code = status.and_then(|s| s.code());
                *exit_code.lock() = code;
                exited.store(true, Ordering::SeqCst);
                push_log(
                    &logs,
                    format!("[localrouter] {label} exited (code {code:?})"),
                );
                remove_pid_record(&pid_file, &key);
            });
        }
        if let Some(pid) = pid {
            add_pid_record(
                &self.pid_file,
                PidRecord {
                    key: spec.key.clone(),
                    pid,
                    exe: spec.program.display().to_string(),
                },
            );
        }

        let running = Running {
            handle: EngineHandle {
                key: spec.key.clone(),
                port,
                api_key,
                in_flight: Arc::new(AtomicUsize::new(0)),
                last_used: Arc::new(Mutex::new(Instant::now())),
            },
            pid,
            started_at: Instant::now(),
            fingerprint,
            kill: Some(kill_tx),
            exited: exited.clone(),
            exit_code: exit_code.clone(),
            idle_timeout: spec.idle_timeout,
        };

        // Wait until ready, the process exits, or we time out.
        let url = format!("http://127.0.0.1:{port}{}", spec.ready_path);
        let deadline = Instant::now() + spec.start_timeout;
        loop {
            if exited.load(Ordering::SeqCst) {
                return Err(EngineError::ExitedDuringStart {
                    label: spec.label.clone(),
                    code: *exit_code.lock(),
                    tail: tail(&logs, 20),
                });
            }
            if let Ok(resp) = self.http.get(&url).send().await {
                if resp.status().is_success() {
                    push_log(&logs, format!("[localrouter] {} is ready", spec.label));
                    return Ok(running);
                }
            }
            if Instant::now() >= deadline {
                let tail = tail(&logs, 20);
                self.terminate(running).await;
                return Err(EngineError::StartTimeout {
                    label: spec.label.clone(),
                    secs: spec.start_timeout.as_secs(),
                    tail,
                });
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    async fn terminate(&self, mut running: Running) {
        if let Some(kill) = running.kill.take() {
            let _ = kill.send(());
        }
        let deadline = Instant::now() + STOP_GRACE + Duration::from_secs(2);
        while !running.exited.load(Ordering::SeqCst) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Stop one engine (e.g. the user clicked Stop or unloaded a model).
    /// Also clears its failure history so it may start again.
    pub async fn stop(&self, key: &str) {
        let running = {
            let mut slots = self.slots.lock();
            slots.get_mut(key).and_then(|slot| {
                slot.failures.clear();
                slot.last_error = None;
                slot.running.take()
            })
        };
        if let Some(r) = running {
            self.terminate(r).await;
        }
    }

    /// Stop every engine whose key starts with `prefix` (a provider's engines).
    pub async fn stop_prefix(&self, prefix: &str) {
        let keys: Vec<String> = self
            .slots
            .lock()
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect();
        for key in keys {
            self.stop(&key).await;
        }
    }

    /// Stop everything (app exit).
    pub async fn stop_all(&self) {
        self.stop_prefix("").await;
    }

    pub fn is_running(&self, key: &str) -> bool {
        self.slots
            .lock()
            .get(key)
            .and_then(|s| s.running.as_ref())
            .is_some_and(|r| !r.exited.load(Ordering::SeqCst))
    }

    pub fn processes(&self) -> Vec<EngineProcessInfo> {
        let slots = self.slots.lock();
        let mut out: Vec<EngineProcessInfo> = slots
            .iter()
            .filter(|(_, s)| s.running.is_some() || s.last_error.is_some())
            .map(|(key, s)| {
                let running = s.running.as_ref();
                let alive = running.is_some_and(|r| !r.exited.load(Ordering::SeqCst));
                let state = if alive {
                    EngineState::Running
                } else if s.failures.len() >= MAX_FAILURES {
                    EngineState::Failed
                } else {
                    EngineState::Exited
                };
                EngineProcessInfo {
                    key: key.clone(),
                    label: s.label.clone(),
                    state,
                    port: running.filter(|_| alive).map(|r| r.handle.port),
                    pid: running.filter(|_| alive).and_then(|r| r.pid),
                    uptime_secs: running
                        .filter(|_| alive)
                        .map(|r| r.started_at.elapsed().as_secs()),
                    idle_secs: running
                        .filter(|_| alive)
                        .map(|r| r.handle.last_used.lock().elapsed().as_secs()),
                    in_flight: running
                        .map(|r| r.handle.in_flight.load(Ordering::SeqCst))
                        .unwrap_or(0),
                    restarts: s.restarts,
                    last_error: s.last_error.clone(),
                }
            })
            .collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out
    }

    /// Recent output of an engine (ring buffer; the full log is on disk).
    pub fn logs(&self, key: &str) -> Vec<String> {
        self.slots
            .lock()
            .get(key)
            .map(|s| s.logs.lock().iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Stop engines idle longer than their `idle_timeout` with no requests in
    /// flight. Returns the keys stopped.
    pub async fn reap_idle(&self) -> Vec<String> {
        let idle: Vec<String> = self
            .slots
            .lock()
            .iter()
            .filter_map(|(key, s)| {
                let r = s.running.as_ref()?;
                let timeout = r.idle_timeout?;
                let busy = r.handle.in_flight.load(Ordering::SeqCst) > 0;
                let idle_for = r.handle.last_used.lock().elapsed();
                (!busy && idle_for >= timeout && !r.exited.load(Ordering::SeqCst))
                    .then(|| key.clone())
            })
            .collect();
        for key in &idle {
            tracing::info!("Stopping idle engine {key}");
            self.stop(key).await;
        }
        idle
    }

    /// Run [`Self::reap_idle`] periodically until the supervisor is dropped.
    pub fn spawn_idle_reaper(self: &Arc<Self>, every: Duration) -> tokio::task::JoinHandle<()> {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            loop {
                tick.tick().await;
                let Some(sup) = weak.upgrade() else { break };
                sup.reap_idle().await;
            }
        })
    }
}

fn free_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn random_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sanitize(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn push_log(logs: &Arc<Mutex<VecDeque<String>>>, line: String) {
    let mut logs = logs.lock();
    if logs.len() >= LOG_RING_LINES {
        logs.pop_front();
    }
    logs.push_back(line);
}

fn tail(logs: &Arc<Mutex<VecDeque<String>>>, n: usize) -> String {
    let logs = logs.lock();
    let skip = logs.len().saturating_sub(n);
    logs.iter()
        .skip(skip)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

fn append_log_file(path: &Path, line: &str) {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > LOG_FILE_MAX_BYTES) {
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

async fn graceful_stop(pid: Option<u32>, child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = pid {
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
        if tokio::time::timeout(STOP_GRACE, child.wait()).await.is_ok() {
            return;
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
    let _ = child.kill().await;
}

fn read_pid_records(path: &Path) -> Vec<PidRecord> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_pid_records(path: &Path, records: &[PidRecord]) {
    if let Ok(json) = serde_json::to_string_pretty(records) {
        let _ = std::fs::write(path, json);
    }
}

static PID_FILE_LOCK: Mutex<()> = Mutex::new(());

fn add_pid_record(path: &Path, record: PidRecord) {
    let _g = PID_FILE_LOCK.lock();
    let mut records = read_pid_records(path);
    records.retain(|r| r.key != record.key);
    records.push(record);
    write_pid_records(path, &records);
}

fn remove_pid_record(path: &Path, key: &str) {
    let _g = PID_FILE_LOCK.lock();
    let mut records = read_pid_records(path);
    records.retain(|r| r.key != key);
    write_pid_records(path, &records);
}

/// Kill engines recorded by a previous session that are still running. A
/// process is only killed when both its pid and executable match the record,
/// so a reused pid never takes down an unrelated program.
fn kill_orphans(path: &Path) {
    let _g = PID_FILE_LOCK.lock();
    let records = read_pid_records(path);
    if records.is_empty() {
        return;
    }
    let mut sys = sysinfo::System::new();
    for record in &records {
        let pid = sysinfo::Pid::from_u32(record.pid);
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        if let Some(process) = sys.process(pid) {
            let matches = process
                .exe()
                .is_some_and(|exe| same_executable(exe, Path::new(&record.exe)));
            if matches {
                tracing::warn!(
                    "Killing orphaned engine {} (pid {})",
                    record.key,
                    record.pid
                );
                process.kill();
            }
        }
    }
    write_pid_records(path, &[]);
}

fn same_executable(running: &Path, recorded: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(running) == canon(recorded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_ignores_nothing_that_matters() {
        let base = LaunchSpec {
            key: "k".into(),
            label: "l".into(),
            program: "/bin/x".into(),
            args: vec!["-m".into(), "a".into()],
            env: vec![],
            port: PortArg::Flag("--port".into()),
            api_key_env: "KEY".into(),
            ready_path: "/health".into(),
            start_timeout: Duration::from_secs(1),
            idle_timeout: None,
        };
        let mut other = base.clone();
        assert_eq!(base.fingerprint(), other.fingerprint());
        other.args.push("-c".into());
        assert_ne!(base.fingerprint(), other.fingerprint());
    }

    #[test]
    fn keys_are_random_hex() {
        let a = random_key();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, random_key());
    }

    #[test]
    fn log_ring_is_bounded() {
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        for i in 0..(LOG_RING_LINES + 10) {
            push_log(&logs, i.to_string());
        }
        assert_eq!(logs.lock().len(), LOG_RING_LINES);
        assert_eq!(
            tail(&logs, 2),
            format!("{}\n{}", LOG_RING_LINES + 8, LOG_RING_LINES + 9)
        );
    }

    #[test]
    fn pid_records_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("engines.json");
        add_pid_record(
            &path,
            PidRecord {
                key: "a".into(),
                pid: 1,
                exe: "/x".into(),
            },
        );
        add_pid_record(
            &path,
            PidRecord {
                key: "b".into(),
                pid: 2,
                exe: "/y".into(),
            },
        );
        add_pid_record(
            &path,
            PidRecord {
                key: "a".into(),
                pid: 3,
                exe: "/x".into(),
            },
        );
        let records = read_pid_records(&path);
        assert_eq!(records.len(), 2);
        assert!(records.iter().any(|r| r.key == "a" && r.pid == 3));
        remove_pid_record(&path, "a");
        assert_eq!(read_pid_records(&path).len(), 1);
    }
}
