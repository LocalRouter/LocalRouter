//! Resumable, verified downloads from the Hugging Face Hub.
//!
//! Design:
//!
//! * A **job** is one set of files from one repo pinned to one commit (e.g. a
//!   GGUF and its projector). Jobs run `max_concurrent` at a time; files within
//!   a job download one after another.
//! * Files are fetched from `{endpoint}/{repo}/resolve/{commit}/{path}` with
//!   redirects followed by hand: the Hub's `302` to the CDN is followed
//!   **without** the `Authorization` header, relative `307`s on the endpoint
//!   keep it. `x-linked-etag` (the LFS SHA-256) and `x-linked-size` are taken
//!   from the Hub's response. Signed CDN URLs are never persisted; an expired
//!   one (`403` from the CDN) is re-resolved.
//! * Bytes go to `{dest}.partial` next to `{dest}.partial.json` (`{sha256,
//!   size, commit}`). Resuming sends `Range: bytes=N-`; a `200` instead of a
//!   `206`, a `416` or a mismatching `Content-Range` restarts the file.
//! * A streaming SHA-256 is computed while writing (the existing partial bytes
//!   are re-hashed before resuming) and compared with the SHA-256 from the
//!   repo listing (falling back to `x-linked-etag`). A mismatch deletes the
//!   partial and retries once, then fails with "checksum mismatch".
//!   Verified files are renamed into place atomically.
//! * Jobs persist to `{storage_dir}/downloads.json` on every state change and
//!   are reloaded as `Paused`; nothing resumes without a user action.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use futures::StreamExt;
use parking_lot::{Mutex, RwLock};
use reqwest::header::{HeaderValue, CONTENT_RANGE, RANGE};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::http::{self, LinkedMeta};
use crate::hub::{validate_repo_id, HubClient, HubError};
use crate::util;

/// Minimum spacing of progress events per job.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// Window over which the download speed is measured.
const SPEED_WINDOW: Duration = Duration::from_secs(1);
/// Free space kept in reserve on the target volume.
const DISK_HEADROOM_BYTES: u64 = 1_000_000_000;
/// Write buffer size.
const WRITE_BUFFER: usize = 1024 * 1024;
/// Persisted file format version.
const DOWNLOADS_VERSION: u32 = 1;

/// Receives job updates (throttled progress plus every state change).
pub trait DownloadEvents: Send + Sync {
    fn on_update(&self, job: &DownloadJobView);
}

/// A [`DownloadEvents`] sink that ignores everything.
pub struct NoopDownloadEvents;

impl DownloadEvents for NoopDownloadEvents {
    fn on_update(&self, _job: &DownloadJobView) {}
}

/// Job state.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    Queued,
    Running,
    Paused,
    Verifying,
    Done,
    Failed,
    Cancelled,
}

impl DownloadState {
    /// Done, failed or cancelled.
    pub fn is_finished(self) -> bool {
        matches!(
            self,
            DownloadState::Done | DownloadState::Failed | DownloadState::Cancelled
        )
    }
}

/// Snapshot of a job for the UI.
#[derive(Serialize, Clone, Debug)]
pub struct DownloadJobView {
    pub id: String,
    pub repo: String,
    /// The commit SHA the job is pinned to.
    pub revision: String,
    pub files: Vec<String>,
    pub state: DownloadState,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub speed_bps: u64,
    pub current_file: Option<String>,
    pub error: Option<String>,
    pub target_dir: String,
}

/// Passed to completion hooks once every file of a job is verified and in
/// place.
#[derive(Clone, Debug)]
pub struct CompletedDownload {
    pub repo: String,
    /// The commit SHA.
    pub revision: String,
    /// `(repo path, local path, size, sha256)`.
    pub files: Vec<(String, PathBuf, u64, Option<String>)>,
}

type CompletionHook = Arc<dyn Fn(CompletedDownload) + Send + Sync>;
type TokenFn = Arc<dyn Fn() -> Option<String> + Send + Sync>;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct FileRecord {
    path: String,
    size: Option<u64>,
    sha256: Option<String>,
    #[serde(default)]
    done: bool,
    /// SHA-256 computed when the file was verified.
    #[serde(default)]
    verified_sha256: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct JobRecord {
    id: String,
    repo: String,
    requested_revision: String,
    commit: String,
    files: Vec<FileRecord>,
    state: DownloadState,
    bytes_done: u64,
    bytes_total: u64,
    error: Option<String>,
    target_dir: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct PersistedJobs {
    version: u32,
    jobs: Vec<JobRecord>,
}

/// Sidecar written next to a partial file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct PartialMeta {
    sha256: Option<String>,
    size: Option<u64>,
    commit: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StopReason {
    Pause,
    Cancel,
}

#[derive(Default)]
struct Runtime {
    /// Set while a task is running the job.
    cancel: Option<CancellationToken>,
    stop: Option<StopReason>,
    speed_bps: u64,
    current_file: Option<String>,
    last_emit: Option<Instant>,
    speed_mark: Option<(Instant, u64)>,
}

struct Job {
    rec: JobRecord,
    rt: Runtime,
}

impl Job {
    fn view(&self) -> DownloadJobView {
        DownloadJobView {
            id: self.rec.id.clone(),
            repo: self.rec.repo.clone(),
            revision: self.rec.commit.clone(),
            files: self.rec.files.iter().map(|f| f.path.clone()).collect(),
            state: self.rec.state,
            bytes_done: self.rec.bytes_done,
            bytes_total: self.rec.bytes_total,
            speed_bps: self.rt.speed_bps,
            current_file: self.rt.current_file.clone(),
            error: self.rec.error.clone(),
            target_dir: self.rec.target_dir.to_string_lossy().into_owned(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum DlError {
    #[error(transparent)]
    Hub(#[from] HubError),
    #[error("checksum mismatch")]
    Checksum,
    #[error("the download link expired (HTTP 403 from the CDN)")]
    CdnExpired,
    #[error("the server could not resume the download")]
    Restart,
    #[error("download interrupted: {0}")]
    Interrupted(String),
    #[error("disk error: {0}")]
    Io(String),
    #[error("stopped")]
    Stopped,
}

impl From<std::io::Error> for DlError {
    fn from(e: std::io::Error) -> Self {
        DlError::Io(e.to_string())
    }
}

/// Download queue and workers.
pub struct DownloadManager {
    me: Weak<DownloadManager>,
    storage_dir: PathBuf,
    hub: HubClient,
    max_concurrent: usize,
    events: Arc<dyn DownloadEvents>,
    token: TokenFn,
    jobs: Mutex<Vec<Job>>,
    persist_lock: Mutex<()>,
    hooks: RwLock<Vec<CompletionHook>>,
}

impl DownloadManager {
    /// Create the manager and load persisted jobs from
    /// `{storage_dir}/downloads.json`. Unfinished jobs come back as `Paused`;
    /// nothing is resumed automatically.
    pub fn new(
        storage_dir: PathBuf,
        hub: HubClient,
        max_concurrent: usize,
        events: Arc<dyn DownloadEvents>,
        token: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    ) -> Arc<Self> {
        let jobs = load_jobs(&storage_dir);
        Arc::new_cyclic(|me| DownloadManager {
            me: me.clone(),
            storage_dir,
            hub,
            max_concurrent: max_concurrent.max(1),
            events,
            token,
            jobs: Mutex::new(jobs),
            persist_lock: Mutex::new(()),
            hooks: RwLock::new(Vec::new()),
        })
    }

    /// The storage directory.
    pub fn storage_dir(&self) -> &Path {
        &self.storage_dir
    }

    /// Register a hook called (on a blocking thread) when a job completes,
    /// before the job is reported as `Done`.
    pub fn on_complete(&self, f: Arc<dyn Fn(CompletedDownload) + Send + Sync>) {
        self.hooks.write().push(f);
    }

    /// Snapshot of all jobs.
    pub fn jobs(&self) -> Vec<DownloadJobView> {
        self.jobs.lock().iter().map(Job::view).collect()
    }

    /// Queue a download of `files` from `repo` at `revision` (default `main`).
    /// Resolves the revision to a commit and the file sizes/SHA-256s first,
    /// and checks free disk space. Returns the job id. An identical unfinished
    /// job is resumed instead of duplicated.
    pub async fn start(
        &self,
        repo: &str,
        revision: Option<&str>,
        files: Vec<String>,
    ) -> Result<String, HubError> {
        validate_repo_id(repo)?;
        if files.is_empty() {
            return Err(HubError::InvalidRequest("no files selected".into()));
        }
        let mut seen = HashSet::new();
        let mut unique = Vec::new();
        for f in files {
            validate_repo_path(&f)?;
            if seen.insert(f.clone()) {
                unique.push(f);
            }
        }
        let requested = revision.filter(|r| !r.is_empty()).unwrap_or("main");
        let token = (self.token)();
        let info = self
            .hub
            .model_info(repo, Some(requested), token.as_deref())
            .await?;
        let commit = match info.sha.as_deref() {
            Some(sha) if is_safe_segment(sha) => sha.to_string(),
            Some(_) => return Err(HubError::Parse("unexpected commit id".into())),
            None if is_safe_segment(requested) => requested.to_string(),
            None => {
                return Err(HubError::Parse(
                    "the Hub did not report a commit for this revision".into(),
                ))
            }
        };
        let mut records = Vec::with_capacity(unique.len());
        for path in &unique {
            let sibling = info
                .siblings
                .iter()
                .find(|s| &s.path == path)
                .ok_or_else(|| HubError::NotFound(format!("{path} in {repo}")))?;
            records.push(FileRecord {
                path: path.clone(),
                size: sibling.size,
                sha256: sibling.sha256.clone().filter(|s| is_sha256_hex(s)),
                done: false,
                verified_sha256: None,
            });
        }
        let target_dir = self.target_dir(repo, &commit);

        // Reuse an identical unfinished job; refuse overlapping ones.
        let existing = {
            let jobs = self.jobs.lock();
            let mut found = None;
            for j in jobs.iter().filter(|j| !j.rec.state.is_finished()) {
                if j.rec.target_dir != target_dir {
                    continue;
                }
                let theirs: HashSet<&str> = j.rec.files.iter().map(|f| f.path.as_str()).collect();
                let ours: HashSet<&str> = unique.iter().map(String::as_str).collect();
                if theirs == ours {
                    found = Some(j.rec.id.clone());
                    break;
                }
                if !theirs.is_disjoint(&ours) {
                    return Err(HubError::InvalidRequest(
                        "some of these files are already part of another download".into(),
                    ));
                }
            }
            found
        };
        if let Some(id) = existing {
            self.resume(&id).await;
            return Ok(id);
        }

        let remaining = remaining_bytes(&target_dir, &records);
        check_disk_space(&self.storage_dir, remaining)?;

        let bytes_total = records.iter().filter_map(|f| f.size).sum();
        let id = uuid::Uuid::new_v4().to_string();
        let rec = JobRecord {
            id: id.clone(),
            repo: repo.to_string(),
            requested_revision: requested.to_string(),
            commit,
            files: records,
            state: DownloadState::Queued,
            bytes_done: 0,
            bytes_total,
            error: None,
            target_dir,
        };
        let view = {
            let mut jobs = self.jobs.lock();
            let job = Job {
                rec,
                rt: Runtime::default(),
            };
            let view = job.view();
            jobs.push(job);
            view
        };
        self.persist();
        self.events.on_update(&view);
        self.schedule();
        Ok(id)
    }

    /// Pause a queued or running job (partial files are kept).
    pub fn pause(&self, id: &str) {
        let view = {
            let mut jobs = self.jobs.lock();
            let Some(job) = jobs.iter_mut().find(|j| j.rec.id == id) else {
                return;
            };
            if is_finishing(job) {
                return;
            }
            match job.rec.state {
                DownloadState::Queued | DownloadState::Running | DownloadState::Verifying => {
                    job.rec.state = DownloadState::Paused;
                    job.rt.speed_bps = 0;
                    if let Some(token) = &job.rt.cancel {
                        job.rt.stop = Some(StopReason::Pause);
                        token.cancel();
                    }
                    job.view()
                }
                _ => return,
            }
        };
        self.persist();
        self.events.on_update(&view);
    }

    /// Resume a paused or failed job (re-checks free disk space).
    pub async fn resume(&self, id: &str) {
        let target = {
            let jobs = self.jobs.lock();
            let Some(job) = jobs.iter().find(|j| j.rec.id == id) else {
                return;
            };
            if !matches!(job.rec.state, DownloadState::Paused | DownloadState::Failed) {
                return;
            }
            (job.rec.target_dir.clone(), job.rec.files.clone())
        };
        let remaining = remaining_bytes(&target.0, &target.1);
        let space = check_disk_space(&self.storage_dir, remaining);
        let view = {
            let mut jobs = self.jobs.lock();
            let Some(job) = jobs.iter_mut().find(|j| j.rec.id == id) else {
                return;
            };
            if !matches!(job.rec.state, DownloadState::Paused | DownloadState::Failed) {
                return;
            }
            match space {
                Ok(()) => {
                    // If the paused task has not exited yet, its stop reason
                    // stays `Pause` so it leaves this state alone; schedule()
                    // starts a fresh task once it is gone.
                    job.rec.state = DownloadState::Queued;
                    job.rec.error = None;
                }
                Err(e) => {
                    job.rec.state = DownloadState::Failed;
                    job.rec.error = Some(e.to_string());
                }
            }
            job.view()
        };
        self.persist();
        self.events.on_update(&view);
        self.schedule();
    }

    /// Cancel a job and delete its partial files. Completed files of the job
    /// are kept.
    pub fn cancel(&self, id: &str) {
        let (view, cleanup) = {
            let mut jobs = self.jobs.lock();
            let Some(job) = jobs.iter_mut().find(|j| j.rec.id == id) else {
                return;
            };
            if matches!(
                job.rec.state,
                DownloadState::Done | DownloadState::Cancelled
            ) || is_finishing(job)
            {
                return;
            }
            job.rec.state = DownloadState::Cancelled;
            job.rt.speed_bps = 0;
            job.rt.current_file = None;
            let cleanup = match &job.rt.cancel {
                // The running task removes the partials once it has stopped
                // writing.
                Some(token) => {
                    job.rt.stop = Some(StopReason::Cancel);
                    token.cancel();
                    None
                }
                None => Some(job.rec.clone()),
            };
            (job.view(), cleanup)
        };
        if let Some(rec) = cleanup {
            remove_partials(&rec);
        }
        self.persist();
        self.events.on_update(&view);
    }

    /// Forget finished (done, failed, cancelled) jobs. Partials of failed jobs
    /// are deleted.
    pub fn remove_finished(&self) {
        let removed: Vec<JobRecord> = {
            let mut jobs = self.jobs.lock();
            let (gone, keep): (Vec<Job>, Vec<Job>) = std::mem::take(&mut *jobs)
                .into_iter()
                .partition(|j| j.rec.state.is_finished() && j.rt.cancel.is_none());
            *jobs = keep;
            gone.into_iter().map(|j| j.rec).collect()
        };
        for rec in removed.iter().filter(|r| r.state == DownloadState::Failed) {
            remove_partials(rec);
        }
        if !removed.is_empty() {
            self.persist();
        }
    }

    // -- internals ---------------------------------------------------------

    fn target_dir(&self, repo: &str, commit: &str) -> PathBuf {
        let mut dir = self.storage_dir.join("hf");
        for seg in repo.split('/') {
            dir.push(seg);
        }
        dir.join(commit)
    }

    fn persist(&self) {
        let _guard = self.persist_lock.lock();
        let snapshot = PersistedJobs {
            version: DOWNLOADS_VERSION,
            jobs: self.jobs.lock().iter().map(|j| j.rec.clone()).collect(),
        };
        let path = self.storage_dir.join("downloads.json");
        let result = serde_json::to_vec_pretty(&snapshot)
            .map_err(std::io::Error::other)
            .and_then(|bytes| util::write_atomic(&path, &bytes));
        if let Err(e) = result {
            tracing::warn!("could not persist download jobs: {e}");
        }
    }

    /// Start queued jobs while fewer than `max_concurrent` tasks are alive.
    fn schedule(&self) {
        let mut to_start = Vec::new();
        {
            let mut jobs = self.jobs.lock();
            let alive = jobs.iter().filter(|j| j.rt.cancel.is_some()).count();
            let mut free = self.max_concurrent.saturating_sub(alive);
            for job in jobs.iter_mut() {
                if free == 0 {
                    break;
                }
                if job.rec.state != DownloadState::Queued || job.rt.cancel.is_some() {
                    continue;
                }
                let token = CancellationToken::new();
                job.rec.state = DownloadState::Running;
                job.rec.error = None;
                job.rt.cancel = Some(token.clone());
                job.rt.stop = None;
                job.rt.speed_mark = None;
                job.rt.speed_bps = 0;
                to_start.push((job.rec.id.clone(), token, job.view()));
                free -= 1;
            }
        }
        if to_start.is_empty() {
            return;
        }
        self.persist();
        let Some(me) = self.me.upgrade() else { return };
        for (id, token, view) in to_start {
            self.events.on_update(&view);
            tokio::spawn(me.clone().run_job(id, token));
        }
    }

    /// Update the state from the worker, unless the user paused/cancelled.
    fn task_state(&self, id: &str, state: DownloadState) {
        let view = {
            let mut jobs = self.jobs.lock();
            let Some(job) = jobs.iter_mut().find(|j| j.rec.id == id) else {
                return;
            };
            if job.rt.stop.is_some()
                || !matches!(
                    job.rec.state,
                    DownloadState::Running | DownloadState::Verifying
                )
                || job.rec.state == state
            {
                return;
            }
            job.rec.state = state;
            job.view()
        };
        self.persist();
        self.events.on_update(&view);
    }

    fn set_current_file(&self, id: &str, file: Option<String>) {
        let view = {
            let mut jobs = self.jobs.lock();
            let Some(job) = jobs.iter_mut().find(|j| j.rec.id == id) else {
                return;
            };
            job.rt.current_file = file;
            job.rt.last_emit = Some(Instant::now());
            job.view()
        };
        self.events.on_update(&view);
    }

    /// Record progress; emits at most every 250 ms unless `force`.
    fn progress(&self, id: &str, bytes_done: u64, force: bool) {
        let view = {
            let mut jobs = self.jobs.lock();
            let Some(job) = jobs.iter_mut().find(|j| j.rec.id == id) else {
                return;
            };
            let now = Instant::now();
            job.rec.bytes_done = bytes_done;
            match job.rt.speed_mark {
                None => job.rt.speed_mark = Some((now, bytes_done)),
                Some((at, bytes)) => {
                    let elapsed = now.duration_since(at);
                    if bytes_done < bytes {
                        job.rt.speed_mark = Some((now, bytes_done));
                    } else if elapsed >= SPEED_WINDOW {
                        job.rt.speed_bps =
                            ((bytes_done - bytes) as f64 / elapsed.as_secs_f64()) as u64;
                        job.rt.speed_mark = Some((now, bytes_done));
                    }
                }
            }
            let due = job
                .rt
                .last_emit
                .is_none_or(|t| now.duration_since(t) >= PROGRESS_INTERVAL);
            if !(force || due) {
                return;
            }
            job.rt.last_emit = Some(now);
            job.view()
        };
        self.events.on_update(&view);
    }

    fn mark_file_done(&self, id: &str, index: usize, sha: Option<String>) {
        {
            let mut jobs = self.jobs.lock();
            if let Some(job) = jobs.iter_mut().find(|j| j.rec.id == id) {
                if let Some(f) = job.rec.files.get_mut(index) {
                    f.done = true;
                    f.verified_sha256 = sha;
                }
            }
        }
        self.persist();
    }

    async fn run_job(self: Arc<Self>, id: String, cancel: CancellationToken) {
        let result = self.run_job_inner(&id, &cancel).await;

        let mut completed = None;
        let mut cleanup = None;
        let view = {
            let mut jobs = self.jobs.lock();
            let Some(job) = jobs.iter_mut().find(|j| j.rec.id == id) else {
                return;
            };
            let stop = job.rt.stop.take();
            job.rt.cancel = None;
            job.rt.speed_bps = 0;
            match (result, stop) {
                (Ok(done), _) => {
                    // Finished before a pause/cancel took effect: keep the
                    // files. `Done` is set after the hooks ran.
                    job.rec.state = DownloadState::Verifying;
                    job.rec.error = None;
                    completed = Some(done);
                }
                (Err(_), Some(StopReason::Pause)) => {
                    // Either still Paused, or already re-queued by resume().
                }
                (Err(_), Some(StopReason::Cancel)) => {
                    job.rec.state = DownloadState::Cancelled;
                    cleanup = Some(job.rec.clone());
                }
                (Err(e), None) => {
                    tracing::warn!("download of {} failed: {e}", job.rec.repo);
                    job.rec.state = DownloadState::Failed;
                    job.rec.error = Some(e.to_string());
                }
            }
            job.rt.current_file = None;
            job.view()
        };
        if let Some(rec) = cleanup {
            remove_partials(&rec);
        }
        self.persist();
        self.events.on_update(&view);

        if let Some(done) = completed {
            let hooks: Vec<CompletionHook> = self.hooks.read().clone();
            if !hooks.is_empty() {
                let result = tokio::task::spawn_blocking(move || {
                    for hook in hooks {
                        hook(done.clone());
                    }
                })
                .await;
                if let Err(e) = result {
                    tracing::warn!("download completion hook panicked: {e}");
                }
            }
            let view = {
                let mut jobs = self.jobs.lock();
                jobs.iter_mut().find(|j| j.rec.id == id).map(|job| {
                    job.rec.state = DownloadState::Done;
                    job.rec.bytes_done = job.rec.bytes_done.max(job.rec.bytes_total);
                    job.view()
                })
            };
            self.persist();
            if let Some(view) = view {
                self.events.on_update(&view);
            }
        }
        self.schedule();
    }

    async fn run_job_inner(
        &self,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<CompletedDownload, DlError> {
        let rec = {
            let jobs = self.jobs.lock();
            jobs.iter()
                .find(|j| j.rec.id == id)
                .map(|j| j.rec.clone())
                .ok_or(DlError::Stopped)?
        };
        let mut completed_bytes = 0u64;
        let mut out = Vec::with_capacity(rec.files.len());
        for (index, file) in rec.files.iter().enumerate() {
            if cancel.is_cancelled() {
                return Err(DlError::Stopped);
            }
            let dest = rec.target_dir.join(&file.path);
            if file.done {
                if let Ok(meta) = tokio::fs::metadata(&dest).await {
                    if file.size.is_none_or(|s| s == meta.len()) {
                        completed_bytes += meta.len();
                        out.push((
                            file.path.clone(),
                            dest,
                            meta.len(),
                            file.verified_sha256.clone().or(file.sha256.clone()),
                        ));
                        continue;
                    }
                }
            }
            self.set_current_file(id, Some(file.path.clone()));
            let (len, sha) = self
                .download_file(id, &rec, file, completed_bytes, cancel)
                .await?;
            completed_bytes += len;
            self.mark_file_done(id, index, sha.clone());
            out.push((file.path.clone(), dest, len, sha));
        }
        self.set_current_file(id, None);
        self.progress(id, completed_bytes, true);
        Ok(CompletedDownload {
            repo: rec.repo.clone(),
            revision: rec.commit.clone(),
            files: out,
        })
    }

    async fn download_file(
        &self,
        id: &str,
        rec: &JobRecord,
        file: &FileRecord,
        base: u64,
        cancel: &CancellationToken,
    ) -> Result<(u64, Option<String>), DlError> {
        let dest = rec.target_dir.join(&file.path);
        let partial = util::with_suffix(&dest, ".partial");
        let mut checksum_retries = 0;
        let mut reresolves = 0;
        let mut restarts = 0;
        let mut interruptions = 0;
        loop {
            match self.attempt_file(id, rec, file, base, cancel).await {
                Ok(v) => return Ok(v),
                Err(DlError::Checksum) if checksum_retries < 1 => {
                    checksum_retries += 1;
                    tracing::warn!("checksum mismatch for {}; retrying once", file.path);
                }
                Err(DlError::CdnExpired) if reresolves < 2 => reresolves += 1,
                Err(DlError::Restart) if restarts < 2 => {
                    restarts += 1;
                    let _ = tokio::fs::remove_file(&partial).await;
                }
                Err(DlError::Interrupted(msg)) if interruptions < 3 => {
                    interruptions += 1;
                    tracing::debug!("download of {} interrupted ({msg}); resuming", file.path);
                    tokio::select! {
                        _ = cancel.cancelled() => return Err(DlError::Stopped),
                        _ = tokio::time::sleep(Duration::from_millis(500 * interruptions)) => {}
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn attempt_file(
        &self,
        id: &str,
        rec: &JobRecord,
        file: &FileRecord,
        base: u64,
        cancel: &CancellationToken,
    ) -> Result<(u64, Option<String>), DlError> {
        let dest = rec.target_dir.join(&file.path);
        let partial = util::with_suffix(&dest, ".partial");
        let meta_path = util::with_suffix(&dest, ".partial.json");
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Already in place (e.g. a crash after the rename)?
        if tokio::fs::metadata(&dest).await.is_ok() {
            match self.verify_existing(id, &dest, file, cancel).await? {
                Some(sha) => {
                    let len = tokio::fs::metadata(&dest).await?.len();
                    let _ = tokio::fs::remove_file(&meta_path).await;
                    return Ok((len, sha));
                }
                None => tokio::fs::remove_file(&dest).await?,
            }
        }

        // Only resume a partial whose sidecar matches this exact file.
        let wanted_meta = PartialMeta {
            sha256: file.sha256.clone(),
            size: file.size,
            commit: rec.commit.clone(),
        };
        let stored_meta: Option<PartialMeta> = tokio::fs::read(&meta_path)
            .await
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let mut existing = match tokio::fs::metadata(&partial).await {
            Ok(m) if stored_meta.as_ref() == Some(&wanted_meta) => m.len(),
            Ok(_) => {
                tokio::fs::remove_file(&partial).await?;
                0
            }
            Err(_) => 0,
        };
        if file.size.is_some_and(|size| existing > size) {
            tokio::fs::remove_file(&partial).await?;
            existing = 0;
        }
        let meta_bytes =
            serde_json::to_vec(&wanted_meta).map_err(|e| DlError::Io(e.to_string()))?;
        tokio::fs::write(&meta_path, meta_bytes).await?;

        let mut hasher = Sha256::new();
        if existing > 0 {
            self.task_state(id, DownloadState::Verifying);
            hasher = hash_prefix(partial.clone(), existing, cancel.clone()).await?;
            self.task_state(id, DownloadState::Running);
        }
        self.progress(id, base + existing, true);

        let mut linked = LinkedMeta::default();
        let complete_already = existing > 0 && file.size == Some(existing);
        if !complete_already {
            let url = Url::parse(&self.hub.resolve_url(&rec.repo, &rec.commit, &file.path))
                .map_err(|e| HubError::InvalidRequest(e.to_string()))?;
            let token = (self.token)();
            let mut headers = Vec::new();
            if existing > 0 {
                let v = HeaderValue::from_str(&format!("bytes={existing}-"))
                    .map_err(|e| DlError::Io(e.to_string()))?;
                headers.push((RANGE, v));
            }
            let followed = tokio::select! {
                _ = cancel.cancelled() => return Err(DlError::Stopped),
                r = http::get_following(
                    self.hub.http_client(),
                    url,
                    self.hub.endpoint_url(),
                    token.as_deref(),
                    &headers,
                    None,
                ) => r.map_err(|e| match e {
                    HubError::Network(m) => DlError::Interrupted(m),
                    other => DlError::Hub(other),
                })?,
            };
            linked = followed.linked;
            let resp = followed.response;
            let status = resp.status();
            let append = match status {
                StatusCode::PARTIAL_CONTENT => {
                    let start = content_range_start(&resp);
                    if existing > 0 && start == Some(existing) {
                        true
                    } else if existing == 0 && start.is_none_or(|s| s == 0) {
                        false
                    } else {
                        return Err(DlError::Restart);
                    }
                }
                StatusCode::OK => false,
                StatusCode::RANGE_NOT_SATISFIABLE if existing > 0 => return Err(DlError::Restart),
                StatusCode::FORBIDDEN if followed.off_origin => return Err(DlError::CdnExpired),
                _ => {
                    return Err(DlError::Hub(
                        http::error_from_response(resp, &rec.repo).await,
                    ))
                }
            };
            if !append && existing > 0 {
                tracing::debug!("server ignored the range request; restarting {}", file.path);
                existing = 0;
                hasher = Sha256::new();
                self.progress(id, base, true);
            }
            let expected_size = file.size.or(linked.size);

            let f = if append {
                tokio::fs::OpenOptions::new()
                    .append(true)
                    .open(&partial)
                    .await?
            } else {
                tokio::fs::File::create(&partial).await?
            };
            let mut writer = tokio::io::BufWriter::with_capacity(WRITE_BUFFER, f);
            let mut stream = resp.bytes_stream();
            loop {
                let next = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        writer.flush().await?;
                        return Err(DlError::Stopped);
                    }
                    next = stream.next() => next,
                };
                match next {
                    Some(Ok(chunk)) => {
                        if expected_size.is_some_and(|s| existing + chunk.len() as u64 > s) {
                            writer.flush().await?;
                            return Err(DlError::Restart);
                        }
                        writer.write_all(&chunk).await?;
                        hasher.update(&chunk);
                        existing += chunk.len() as u64;
                        self.progress(id, base + existing, false);
                    }
                    Some(Err(e)) => {
                        writer.flush().await?;
                        return Err(DlError::Interrupted(e.without_url().to_string()));
                    }
                    None => break,
                }
            }
            writer.flush().await?;
            writer.get_mut().sync_all().await?;
        }

        if let Some(size) = file.size.or(linked.size) {
            if existing < size {
                return Err(DlError::Interrupted(format!(
                    "received {existing} of {size} bytes"
                )));
            }
            if existing > size {
                return Err(DlError::Restart);
            }
        }
        let actual = hex::encode(hasher.finalize());
        let expected = file
            .sha256
            .clone()
            .or_else(|| linked.etag.clone().filter(|e| is_sha256_hex(e)));
        if let Some(expected) = expected {
            if !expected.eq_ignore_ascii_case(&actual) {
                let _ = tokio::fs::remove_file(&partial).await;
                let _ = tokio::fs::remove_file(&meta_path).await;
                return Err(DlError::Checksum);
            }
        }
        tokio::fs::rename(&partial, &dest).await?;
        let _ = tokio::fs::remove_file(&meta_path).await;
        Ok((existing, Some(actual)))
    }

    /// Check a file that is already at its destination. Returns
    /// `Some(sha256)` when it is valid (the SHA is `None` when there is
    /// nothing to verify against), `None` when it must be re-downloaded.
    async fn verify_existing(
        &self,
        id: &str,
        dest: &Path,
        file: &FileRecord,
        cancel: &CancellationToken,
    ) -> Result<Option<Option<String>>, DlError> {
        let len = tokio::fs::metadata(dest).await?.len();
        if file.size.is_some_and(|s| s != len) {
            return Ok(None);
        }
        let Some(expected) = &file.sha256 else {
            return Ok(Some(None));
        };
        self.task_state(id, DownloadState::Verifying);
        let hasher = hash_prefix(dest.to_path_buf(), len, cancel.clone()).await?;
        self.task_state(id, DownloadState::Running);
        let actual = hex::encode(hasher.finalize());
        Ok(expected
            .eq_ignore_ascii_case(&actual)
            .then_some(Some(actual)))
    }
}

/// All files are in place and the completion hooks are running; the job can
/// no longer be paused or cancelled.
fn is_finishing(job: &Job) -> bool {
    job.rt.cancel.is_none()
        && matches!(
            job.rec.state,
            DownloadState::Running | DownloadState::Verifying
        )
}

fn content_range_start(resp: &reqwest::Response) -> Option<u64> {
    let v = resp.headers().get(CONTENT_RANGE)?.to_str().ok()?;
    let rest = v.trim().strip_prefix("bytes")?.trim_start();
    let (start, _) = rest.split_once('-')?;
    start.trim().parse().ok()
}

/// SHA-256 of the first `len` bytes of `path`, on a blocking thread.
async fn hash_prefix(
    path: PathBuf,
    len: u64,
    cancel: CancellationToken,
) -> Result<Sha256, DlError> {
    tokio::task::spawn_blocking(move || -> Result<Sha256, DlError> {
        use std::io::Read;
        let mut f = std::fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; WRITE_BUFFER];
        let mut left = len;
        while left > 0 {
            if cancel.is_cancelled() {
                return Err(DlError::Stopped);
            }
            let want = left.min(buf.len() as u64) as usize;
            let n = f.read(&mut buf[..want])?;
            if n == 0 {
                return Err(DlError::Restart);
            }
            hasher.update(&buf[..n]);
            left -= n as u64;
        }
        Ok(hasher)
    })
    .await
    .map_err(|e| DlError::Io(e.to_string()))?
}

/// Delete `.partial` and `.partial.json` files of unfinished files of a job.
fn remove_partials(rec: &JobRecord) {
    for f in &rec.files {
        let dest = rec.target_dir.join(&f.path);
        let _ = std::fs::remove_file(util::with_suffix(&dest, ".partial"));
        let _ = std::fs::remove_file(util::with_suffix(&dest, ".partial.json"));
    }
}

/// Bytes still to be downloaded (sizes minus what is already on disk).
fn remaining_bytes(target_dir: &Path, files: &[FileRecord]) -> u64 {
    files
        .iter()
        .map(|f| {
            let Some(size) = f.size else { return 0 };
            let dest = target_dir.join(&f.path);
            if std::fs::metadata(&dest).is_ok_and(|m| m.len() == size) {
                return 0;
            }
            let have = std::fs::metadata(util::with_suffix(&dest, ".partial"))
                .map(|m| m.len())
                .unwrap_or(0);
            size.saturating_sub(have)
        })
        .sum()
}

/// Free space on the volume holding `dir` (via `sysinfo`), if it can be told.
fn available_space(dir: &Path) -> Option<u64> {
    let canonical = util::canonicalize_lenient(dir);
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|d| canonical.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

fn check_disk_space(dir: &Path, remaining: u64) -> Result<(), HubError> {
    ensure_space(available_space(dir), remaining, dir)
}

/// `remaining` plus 1 GB of headroom must be free. Unknown free space passes.
fn ensure_space(available: Option<u64>, remaining: u64, dir: &Path) -> Result<(), HubError> {
    let Some(available) = available else {
        tracing::debug!("free disk space unknown for {}", dir.display());
        return Ok(());
    };
    let needed = remaining.saturating_add(DISK_HEADROOM_BYTES);
    if available < needed {
        return Err(HubError::Storage(format!(
            "Not enough disk space: this download needs {} (plus 1 GB headroom) but only {} is free on the drive holding {}",
            util::gb(remaining),
            util::gb(available),
            dir.display()
        )));
    }
    Ok(())
}

/// Reject absolute paths, `..`, empty segments and backslashes in repo paths.
pub(crate) fn validate_repo_path(path: &str) -> Result<(), HubError> {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.contains('\0')
        || path
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..");
    if bad {
        return Err(HubError::InvalidRequest(format!(
            "unsafe file path in repository: {path:?}"
        )));
    }
    Ok(())
}

fn is_safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Load persisted jobs; unfinished ones become `Paused`. Records whose repo,
/// commit or paths fail validation are dropped.
fn load_jobs(storage_dir: &Path) -> Vec<Job> {
    let path = storage_dir.join("downloads.json");
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    let persisted: PersistedJobs = match serde_json::from_slice(&bytes) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("ignoring unreadable {}: {e}", path.display());
            return Vec::new();
        }
    };
    persisted
        .jobs
        .into_iter()
        .filter(|r| {
            validate_repo_id(&r.repo).is_ok()
                && is_safe_segment(&r.commit)
                && r.files.iter().all(|f| validate_repo_path(&f.path).is_ok())
        })
        .map(|mut rec| {
            // Never trust a persisted absolute path.
            let mut dir = storage_dir.join("hf");
            for seg in rec.repo.split('/') {
                dir.push(seg);
            }
            rec.target_dir = dir.join(&rec.commit);
            if matches!(
                rec.state,
                DownloadState::Queued | DownloadState::Running | DownloadState::Verifying
            ) {
                rec.state = DownloadState::Paused;
            }
            Job {
                rec,
                rt: Runtime::default(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::RangeResponder;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
    const REPO: &str = "org/repo";

    #[derive(Default)]
    struct Recorder(Mutex<Vec<DownloadJobView>>);

    impl DownloadEvents for Recorder {
        fn on_update(&self, job: &DownloadJobView) {
            self.0.lock().push(job.clone());
        }
    }

    impl Recorder {
        fn states(&self) -> Vec<DownloadState> {
            let mut out: Vec<DownloadState> = Vec::new();
            for v in self.0.lock().iter() {
                if out.last() != Some(&v.state) {
                    out.push(v.state);
                }
            }
            out
        }
    }

    struct Fixture {
        hub: MockServer,
        cdn: MockServer,
        dir: tempfile::TempDir,
        mgr: Arc<DownloadManager>,
        events: Arc<Recorder>,
    }

    fn data(n: usize, seed: u8) -> Vec<u8> {
        (0..n)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    fn sha(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    /// Hub with `model_info` listing `siblings` (path, size, sha256).
    async fn fixture(siblings: &[(&str, Option<u64>, Option<String>)]) -> Fixture {
        let hub = MockServer::start().await;
        let cdn = MockServer::start().await;
        let sib: Vec<serde_json::Value> = siblings
            .iter()
            .map(|(p, size, sha)| {
                let mut v = serde_json::json!({ "rfilename": p });
                if let Some(s) = size {
                    v["size"] = serde_json::json!(s);
                }
                if let Some(h) = sha {
                    v["lfs"] = serde_json::json!({ "sha256": h, "size": size });
                }
                v
            })
            .collect();
        Mock::given(method("GET"))
            .and(path(format!("/api/models/{REPO}/revision/main")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": REPO, "sha": COMMIT, "siblings": sib
            })))
            .mount(&hub)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let events = Arc::new(Recorder::default());
        let mgr = DownloadManager::new(
            dir.path().to_path_buf(),
            HubClient::new(&hub.uri()),
            2,
            events.clone(),
            Arc::new(|| Some("hf_secret".to_string())),
        );
        Fixture {
            hub,
            cdn,
            dir,
            mgr,
            events,
        }
    }

    impl Fixture {
        /// Hub resolve → 302 to the CDN, which serves `bytes` with Range support.
        async fn lfs_file(&self, name: &str, bytes: &[u8]) {
            Mock::given(method("GET"))
                .and(path(format!("/{REPO}/resolve/{COMMIT}/{name}")))
                .respond_with(
                    ResponseTemplate::new(302)
                        .insert_header(
                            "location",
                            format!("{}/blobs/{name}?sig=abc", self.cdn.uri()).as_str(),
                        )
                        .insert_header("x-linked-etag", format!("\"{}\"", sha(bytes)).as_str())
                        .insert_header("x-linked-size", bytes.len().to_string().as_str()),
                )
                .mount(&self.hub)
                .await;
            self.cdn_serves(name, RangeResponder::new(bytes.to_vec()))
                .await;
        }

        async fn cdn_serves(&self, name: &str, r: RangeResponder) {
            Mock::given(method("GET"))
                .and(path(format!("/blobs/{name}")))
                .respond_with(r)
                .with_priority(5)
                .mount(&self.cdn)
                .await;
        }

        fn dest(&self, name: &str) -> PathBuf {
            self.dir.path().join("hf/org/repo").join(COMMIT).join(name)
        }

        fn write_partial(&self, name: &str, bytes: &[u8], meta: &PartialMeta) {
            let dest = self.dest(name);
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::write(util::with_suffix(&dest, ".partial"), bytes).unwrap();
            std::fs::write(
                util::with_suffix(&dest, ".partial.json"),
                serde_json::to_vec(meta).unwrap(),
            )
            .unwrap();
        }

        async fn wait(&self, id: &str, pred: impl Fn(&DownloadJobView) -> bool) -> DownloadJobView {
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                let job = self.mgr.jobs().into_iter().find(|j| j.id == id).unwrap();
                if pred(&job) {
                    return job;
                }
                assert!(Instant::now() < deadline, "timed out; last state {job:?}");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        async fn wait_idle(&self, id: &str) {
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                let alive = self
                    .mgr
                    .jobs
                    .lock()
                    .iter()
                    .any(|j| j.rec.id == id && j.rt.cancel.is_some());
                if !alive {
                    return;
                }
                assert!(Instant::now() < deadline, "task did not stop");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }

    fn has_auth(r: &wiremock::Request) -> bool {
        r.headers.contains_key("authorization")
    }

    #[tokio::test]
    async fn downloads_via_cdn_and_relative_redirect_without_leaking_token() {
        let model = data(300_000, 1);
        let config = b"{\"small\":true}".to_vec();
        let fx = fixture(&[
            (
                "model-Q4_K_M.gguf",
                Some(model.len() as u64),
                Some(sha(&model)),
            ),
            ("config.json", Some(config.len() as u64), None),
        ])
        .await;
        fx.lfs_file("model-Q4_K_M.gguf", &model).await;
        // Small git file: relative 307 on the Hub itself.
        Mock::given(path(format!("/{REPO}/resolve/{COMMIT}/config.json")))
            .respond_with(ResponseTemplate::new(307).insert_header(
                "location",
                format!("/api/resolve-cache/models/{REPO}/{COMMIT}/config.json").as_str(),
            ))
            .mount(&fx.hub)
            .await;
        Mock::given(path(format!(
            "/api/resolve-cache/models/{REPO}/{COMMIT}/config.json"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(config.clone()))
        .mount(&fx.hub)
        .await;

        let completed = Arc::new(Mutex::new(Vec::<CompletedDownload>::new()));
        let c2 = completed.clone();
        fx.mgr.on_complete(Arc::new(move |d| c2.lock().push(d)));

        let id = fx
            .mgr
            .start(
                REPO,
                None,
                vec!["model-Q4_K_M.gguf".into(), "config.json".into()],
            )
            .await
            .unwrap();
        let job = fx.wait(&id, |j| j.state.is_finished()).await;
        assert_eq!(job.state, DownloadState::Done, "{job:?}");
        assert_eq!(job.revision, COMMIT);
        assert_eq!(job.bytes_total, (model.len() + config.len()) as u64);
        assert_eq!(job.bytes_done, job.bytes_total);

        assert_eq!(std::fs::read(fx.dest("model-Q4_K_M.gguf")).unwrap(), model);
        assert_eq!(std::fs::read(fx.dest("config.json")).unwrap(), config);
        assert!(!util::with_suffix(&fx.dest("model-Q4_K_M.gguf"), ".partial").exists());
        assert!(!util::with_suffix(&fx.dest("model-Q4_K_M.gguf"), ".partial.json").exists());

        // The token reached the Hub (resolve and the relative redirect) but
        // never the CDN.
        let hub_reqs = fx.hub.received_requests().await.unwrap();
        assert!(hub_reqs.iter().all(has_auth));
        assert!(hub_reqs
            .iter()
            .any(|r| r.url.path().starts_with("/api/resolve-cache/")));
        let cdn_reqs = fx.cdn.received_requests().await.unwrap();
        assert_eq!(cdn_reqs.len(), 1);
        assert!(!has_auth(&cdn_reqs[0]));

        let done = completed.lock().clone();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].repo, REPO);
        assert_eq!(done[0].revision, COMMIT);
        assert_eq!(done[0].files.len(), 2);
        assert_eq!(done[0].files[0].1, fx.dest("model-Q4_K_M.gguf"));
        assert_eq!(done[0].files[0].2, model.len() as u64);
        assert_eq!(done[0].files[0].3.as_deref(), Some(sha(&model).as_str()));
        assert_eq!(done[0].files[1].3.as_deref(), Some(sha(&config).as_str()));

        let states = fx.events.states();
        assert_eq!(states.first(), Some(&DownloadState::Queued));
        assert!(states.contains(&DownloadState::Running));
        assert_eq!(states.last(), Some(&DownloadState::Done));

        // Starting the same download again finds the files in place.
        let id2 = fx
            .mgr
            .start(REPO, None, vec!["model-Q4_K_M.gguf".into()])
            .await
            .unwrap();
        let job2 = fx.wait(&id2, |j| j.state.is_finished()).await;
        assert_eq!(job2.state, DownloadState::Done);
        assert_eq!(fx.cdn.received_requests().await.unwrap().len(), 1);

        fx.mgr.remove_finished();
        assert!(fx.mgr.jobs().is_empty());
    }

    #[tokio::test]
    async fn resumes_partial_with_range_and_hashes_whole_file() {
        let model = data(200_000, 2);
        let fx = fixture(&[("m.gguf", Some(model.len() as u64), Some(sha(&model)))]).await;
        fx.lfs_file("m.gguf", &model).await;
        fx.write_partial(
            "m.gguf",
            &model[..80_000],
            &PartialMeta {
                sha256: Some(sha(&model)),
                size: Some(model.len() as u64),
                commit: COMMIT.into(),
            },
        );
        let id = fx
            .mgr
            .start(REPO, None, vec!["m.gguf".into()])
            .await
            .unwrap();
        let job = fx.wait(&id, |j| j.state.is_finished()).await;
        assert_eq!(job.state, DownloadState::Done, "{job:?}");
        assert_eq!(std::fs::read(fx.dest("m.gguf")).unwrap(), model);
        let cdn_reqs = fx.cdn.received_requests().await.unwrap();
        assert_eq!(cdn_reqs.len(), 1);
        assert_eq!(
            cdn_reqs[0].headers.get("range").unwrap().to_str().unwrap(),
            "bytes=80000-"
        );
        assert!(fx.events.states().contains(&DownloadState::Verifying));
    }

    #[tokio::test]
    async fn stale_partial_with_other_sha_is_discarded() {
        let model = data(50_000, 3);
        let fx = fixture(&[("m.gguf", Some(model.len() as u64), Some(sha(&model)))]).await;
        fx.lfs_file("m.gguf", &model).await;
        fx.write_partial(
            "m.gguf",
            &[0u8; 1000],
            &PartialMeta {
                sha256: Some("ab".repeat(32)),
                size: Some(model.len() as u64),
                commit: COMMIT.into(),
            },
        );
        let id = fx
            .mgr
            .start(REPO, None, vec!["m.gguf".into()])
            .await
            .unwrap();
        let job = fx.wait(&id, |j| j.state.is_finished()).await;
        assert_eq!(job.state, DownloadState::Done, "{job:?}");
        let cdn_reqs = fx.cdn.received_requests().await.unwrap();
        assert!(cdn_reqs[0].headers.get("range").is_none());
    }

    #[tokio::test]
    async fn restarts_when_server_ignores_range() {
        let model = data(120_000, 4);
        let fx = fixture(&[("m.gguf", Some(model.len() as u64), Some(sha(&model)))]).await;
        Mock::given(path(format!("/{REPO}/resolve/{COMMIT}/m.gguf")))
            .respond_with(ResponseTemplate::new(302).insert_header(
                "location",
                format!("{}/blobs/m.gguf", fx.cdn.uri()).as_str(),
            ))
            .mount(&fx.hub)
            .await;
        fx.cdn_serves("m.gguf", RangeResponder::ignoring_range(model.clone()))
            .await;
        // Garbage partial: if it were kept the checksum would fail.
        fx.write_partial(
            "m.gguf",
            &[0xEEu8; 5000],
            &PartialMeta {
                sha256: Some(sha(&model)),
                size: Some(model.len() as u64),
                commit: COMMIT.into(),
            },
        );
        let id = fx
            .mgr
            .start(REPO, None, vec!["m.gguf".into()])
            .await
            .unwrap();
        let job = fx.wait(&id, |j| j.state.is_finished()).await;
        assert_eq!(job.state, DownloadState::Done, "{job:?}");
        assert_eq!(std::fs::read(fx.dest("m.gguf")).unwrap(), model);
        let cdn_reqs = fx.cdn.received_requests().await.unwrap();
        assert_eq!(cdn_reqs.len(), 1, "the 200 body is used directly");
    }

    #[tokio::test]
    async fn range_not_satisfiable_restarts_file() {
        let model = data(10_000, 5);
        // No size known up front, so the oversized partial is only detected
        // by the server's 416.
        let fx = fixture(&[("m.gguf", None, Some(sha(&model)))]).await;
        Mock::given(path(format!("/{REPO}/resolve/{COMMIT}/m.gguf")))
            .respond_with(ResponseTemplate::new(302).insert_header(
                "location",
                format!("{}/blobs/m.gguf", fx.cdn.uri()).as_str(),
            ))
            .mount(&fx.hub)
            .await;
        fx.cdn_serves("m.gguf", RangeResponder::new(model.clone()))
            .await;
        fx.write_partial(
            "m.gguf",
            &[1u8; 20_000],
            &PartialMeta {
                sha256: Some(sha(&model)),
                size: None,
                commit: COMMIT.into(),
            },
        );
        let id = fx
            .mgr
            .start(REPO, None, vec!["m.gguf".into()])
            .await
            .unwrap();
        let job = fx.wait(&id, |j| j.state.is_finished()).await;
        assert_eq!(job.state, DownloadState::Done, "{job:?}");
        assert_eq!(std::fs::read(fx.dest("m.gguf")).unwrap(), model);
        let cdn_reqs = fx.cdn.received_requests().await.unwrap();
        assert_eq!(cdn_reqs.len(), 2);
        assert!(cdn_reqs[1].headers.get("range").is_none());
    }

    #[tokio::test]
    async fn checksum_mismatch_retries_once_then_fails() {
        let model = data(40_000, 6);
        let wrong = "0".repeat(64);
        let fx = fixture(&[("m.gguf", Some(model.len() as u64), Some(wrong))]).await;
        fx.lfs_file("m.gguf", &model).await;
        let id = fx
            .mgr
            .start(REPO, None, vec!["m.gguf".into()])
            .await
            .unwrap();
        let job = fx.wait(&id, |j| j.state.is_finished()).await;
        assert_eq!(job.state, DownloadState::Failed);
        assert!(job.error.as_deref().unwrap().contains("checksum mismatch"));
        assert_eq!(fx.cdn.received_requests().await.unwrap().len(), 2);
        assert!(!fx.dest("m.gguf").exists());
        assert!(!util::with_suffix(&fx.dest("m.gguf"), ".partial").exists());
        // Persisted as failed.
        let persisted = std::fs::read_to_string(fx.dir.path().join("downloads.json")).unwrap();
        assert!(persisted.contains("\"failed\""));
    }

    #[tokio::test]
    async fn falls_back_to_linked_etag_when_listing_has_no_sha() {
        let model = data(30_000, 7);
        let fx = fixture(&[("m.gguf", Some(model.len() as u64), None)]).await;
        // x-linked-etag advertises a different hash → mismatch.
        Mock::given(path(format!("/{REPO}/resolve/{COMMIT}/m.gguf")))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header(
                        "location",
                        format!("{}/blobs/m.gguf", fx.cdn.uri()).as_str(),
                    )
                    .insert_header("x-linked-etag", format!("\"{}\"", "f".repeat(64)).as_str()),
            )
            .mount(&fx.hub)
            .await;
        fx.cdn_serves("m.gguf", RangeResponder::new(model.clone()))
            .await;
        let id = fx
            .mgr
            .start(REPO, None, vec!["m.gguf".into()])
            .await
            .unwrap();
        let job = fx.wait(&id, |j| j.state.is_finished()).await;
        assert_eq!(job.state, DownloadState::Failed);
        assert!(job.error.unwrap().contains("checksum mismatch"));
    }

    #[tokio::test]
    async fn expired_cdn_url_is_re_resolved() {
        let model = data(25_000, 8);
        let fx = fixture(&[("m.gguf", Some(model.len() as u64), Some(sha(&model)))]).await;
        fx.lfs_file("m.gguf", &model).await;
        Mock::given(path("/blobs/m.gguf"))
            .respond_with(ResponseTemplate::new(403))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&fx.cdn)
            .await;
        let id = fx
            .mgr
            .start(REPO, None, vec!["m.gguf".into()])
            .await
            .unwrap();
        let job = fx.wait(&id, |j| j.state.is_finished()).await;
        assert_eq!(job.state, DownloadState::Done, "{job:?}");
        let resolves = fx
            .hub
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().contains("/resolve/"))
            .count();
        assert_eq!(resolves, 2);
    }

    #[tokio::test]
    async fn pause_keeps_partial_and_resume_completes() {
        let model = data(60_000, 9);
        let fx = fixture(&[("m.gguf", Some(model.len() as u64), Some(sha(&model)))]).await;
        fx.lfs_file("m.gguf", &model).await;
        // The first CDN request hangs so the pause lands mid-download.
        Mock::given(path("/blobs/m.gguf"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&fx.cdn)
            .await;
        let meta = PartialMeta {
            sha256: Some(sha(&model)),
            size: Some(model.len() as u64),
            commit: COMMIT.into(),
        };
        fx.write_partial("m.gguf", &model[..10_000], &meta);

        let id = fx
            .mgr
            .start(REPO, None, vec!["m.gguf".into()])
            .await
            .unwrap();
        // Wait until the CDN request is in flight.
        let deadline = Instant::now() + Duration::from_secs(10);
        while fx.cdn.received_requests().await.unwrap().is_empty() {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        fx.mgr.pause(&id);
        fx.wait_idle(&id).await;
        let job = fx.mgr.jobs().into_iter().find(|j| j.id == id).unwrap();
        assert_eq!(job.state, DownloadState::Paused);
        assert!(util::with_suffix(&fx.dest("m.gguf"), ".partial").exists());
        let persisted = std::fs::read_to_string(fx.dir.path().join("downloads.json")).unwrap();
        assert!(persisted.contains("\"paused\""));

        fx.mgr.resume(&id).await;
        let job = fx.wait(&id, |j| j.state.is_finished()).await;
        assert_eq!(job.state, DownloadState::Done, "{job:?}");
        assert_eq!(std::fs::read(fx.dest("m.gguf")).unwrap(), model);
        let cdn_reqs = fx.cdn.received_requests().await.unwrap();
        assert_eq!(
            cdn_reqs.last().unwrap().headers.get("range").unwrap(),
            "bytes=10000-"
        );
    }

    #[tokio::test]
    async fn cancel_removes_partials() {
        let model = data(60_000, 10);
        let fx = fixture(&[("m.gguf", Some(model.len() as u64), Some(sha(&model)))]).await;
        fx.lfs_file("m.gguf", &model).await;
        Mock::given(path("/blobs/m.gguf"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
            .with_priority(1)
            .mount(&fx.cdn)
            .await;
        fx.write_partial(
            "m.gguf",
            &model[..5000],
            &PartialMeta {
                sha256: Some(sha(&model)),
                size: Some(model.len() as u64),
                commit: COMMIT.into(),
            },
        );
        let id = fx
            .mgr
            .start(REPO, None, vec!["m.gguf".into()])
            .await
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while fx.cdn.received_requests().await.unwrap().is_empty() {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        fx.mgr.cancel(&id);
        fx.wait_idle(&id).await;
        let job = fx.mgr.jobs().into_iter().find(|j| j.id == id).unwrap();
        assert_eq!(job.state, DownloadState::Cancelled);
        assert!(!util::with_suffix(&fx.dest("m.gguf"), ".partial").exists());
        assert!(!util::with_suffix(&fx.dest("m.gguf"), ".partial.json").exists());
        assert!(!fx.dest("m.gguf").exists());

        // Cancelling a paused (idle) job deletes partials synchronously.
        fx.write_partial(
            "m.gguf",
            &model[..5000],
            &PartialMeta {
                sha256: Some(sha(&model)),
                size: Some(model.len() as u64),
                commit: COMMIT.into(),
            },
        );
        fx.mgr.remove_finished();
        {
            let rec = JobRecord {
                id: "idle".into(),
                repo: REPO.into(),
                requested_revision: "main".into(),
                commit: COMMIT.into(),
                files: vec![FileRecord {
                    path: "m.gguf".into(),
                    size: Some(model.len() as u64),
                    sha256: None,
                    done: false,
                    verified_sha256: None,
                }],
                state: DownloadState::Paused,
                bytes_done: 0,
                bytes_total: 0,
                error: None,
                target_dir: fx.dest("m.gguf").parent().unwrap().to_path_buf(),
            };
            fx.mgr.jobs.lock().push(Job {
                rec,
                rt: Runtime::default(),
            });
        }
        fx.mgr.cancel("idle");
        assert!(!util::with_suffix(&fx.dest("m.gguf"), ".partial").exists());
    }

    #[tokio::test]
    async fn rejects_unsafe_paths_before_any_request() {
        let fx = fixture(&[("m.gguf", Some(1), None)]).await;
        for bad in [
            "../evil.gguf",
            "/etc/passwd",
            "a/../../b.gguf",
            "C:\\x.gguf",
            "a//b",
            "./a",
            "",
        ] {
            let err = fx
                .mgr
                .start(REPO, None, vec![bad.into()])
                .await
                .unwrap_err();
            assert!(matches!(err, HubError::InvalidRequest(_)), "{bad}: {err:?}");
        }
        assert!(matches!(
            fx.mgr.start("../x", None, vec!["m.gguf".into()]).await,
            Err(HubError::InvalidRequest(_))
        ));
        assert!(matches!(
            fx.mgr.start(REPO, None, vec![]).await,
            Err(HubError::InvalidRequest(_))
        ));
        assert!(fx.hub.received_requests().await.unwrap().is_empty());
        // A file that is not in the repo.
        assert!(matches!(
            fx.mgr.start(REPO, None, vec!["other.gguf".into()]).await,
            Err(HubError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn insufficient_disk_space_fails_fast() {
        let fx = fixture(&[("huge.gguf", Some(1_000_000_000_000_000_000), None)]).await;
        let err = fx
            .mgr
            .start(REPO, None, vec!["huge.gguf".into()])
            .await
            .unwrap_err();
        // Only meaningful where the volume's free space is known.
        if available_space(fx.dir.path()).is_some() {
            assert!(matches!(err, HubError::Storage(_)), "{err:?}");
            assert!(err.to_string().contains("disk space"));
        }
        assert!(fx.mgr.jobs().is_empty());

        assert!(ensure_space(Some(10), 5, Path::new("/x")).is_err());
        assert!(ensure_space(Some(2_000_000_000), 5, Path::new("/x")).is_ok());
        assert!(ensure_space(None, u64::MAX, Path::new("/x")).is_ok());
    }

    #[tokio::test]
    async fn gated_repo_error_is_returned_from_start() {
        let hub = MockServer::start().await;
        Mock::given(path("/api/models/org/gated/revision/main"))
            .respond_with(ResponseTemplate::new(403).insert_header("x-error-code", "GatedRepo"))
            .mount(&hub)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let mgr = DownloadManager::new(
            dir.path().to_path_buf(),
            HubClient::new(&hub.uri()),
            1,
            Arc::new(NoopDownloadEvents),
            Arc::new(|| None),
        );
        let err = mgr
            .start("org/gated", None, vec!["m.gguf".into()])
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            HubError::Gated {
                requires_login: false,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn persisted_jobs_reload_as_paused_without_network() {
        let model = data(10_000, 11);
        let fx = fixture(&[("m.gguf", Some(model.len() as u64), Some(sha(&model)))]).await;
        fx.lfs_file("m.gguf", &model).await;
        Mock::given(path("/blobs/m.gguf"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
            .with_priority(1)
            .mount(&fx.cdn)
            .await;
        let id = fx
            .mgr
            .start(REPO, None, vec!["m.gguf".into()])
            .await
            .unwrap();
        fx.wait(&id, |j| j.state == DownloadState::Running).await;
        // Let the first manager's requests settle (it is now waiting on the CDN).
        let deadline = Instant::now() + Duration::from_secs(10);
        while fx.cdn.received_requests().await.unwrap().is_empty() {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let count = Arc::new(AtomicUsize::new(0));
        struct Counter(Arc<AtomicUsize>);
        impl DownloadEvents for Counter {
            fn on_update(&self, _: &DownloadJobView) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let hub_before = fx.hub.received_requests().await.unwrap().len();
        let reloaded = DownloadManager::new(
            fx.dir.path().to_path_buf(),
            HubClient::new(&fx.hub.uri()),
            2,
            Arc::new(Counter(count.clone())),
            Arc::new(|| None),
        );
        let jobs = reloaded.jobs();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, id);
        assert_eq!(jobs[0].state, DownloadState::Paused);
        assert_eq!(jobs[0].files, vec!["m.gguf"]);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(reloaded.jobs()[0].state, DownloadState::Paused);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert_eq!(fx.hub.received_requests().await.unwrap().len(), hub_before);

        fx.mgr.cancel(&id);
        fx.wait_idle(&id).await;
    }

    #[tokio::test]
    async fn identical_start_reuses_job_and_overlap_is_rejected() {
        let a = data(1000, 12);
        let fx = fixture(&[
            ("a.gguf", Some(a.len() as u64), Some(sha(&a))),
            ("b.gguf", Some(10), None),
        ])
        .await;
        fx.lfs_file("a.gguf", &a).await;
        Mock::given(path("/blobs/a.gguf"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
            .with_priority(1)
            .mount(&fx.cdn)
            .await;
        let id = fx
            .mgr
            .start(REPO, None, vec!["a.gguf".into()])
            .await
            .unwrap();
        fx.mgr.pause(&id);
        let again = fx
            .mgr
            .start(REPO, None, vec!["a.gguf".into()])
            .await
            .unwrap();
        assert_eq!(again, id);
        let overlap = fx
            .mgr
            .start(REPO, None, vec!["a.gguf".into(), "b.gguf".into()])
            .await;
        assert!(matches!(overlap, Err(HubError::InvalidRequest(_))));
        fx.mgr.cancel(&id);
        fx.wait_idle(&id).await;
    }

    #[test]
    fn path_validation() {
        assert!(validate_repo_path("a/b/c.gguf").is_ok());
        assert!(validate_repo_path("Q4_K_M/model-00001-of-00002.gguf").is_ok());
        assert!(validate_repo_path("a/./b").is_err());
        assert!(validate_repo_path("a\\b").is_err());
        assert!(is_sha256_hex(&"a".repeat(64)));
        assert!(!is_sha256_hex("abc"));
    }
}
