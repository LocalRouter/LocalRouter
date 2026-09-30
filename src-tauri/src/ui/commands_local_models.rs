//! Tauri commands for local models: Hugging Face search and downloads, the
//! installed-model library, Load/Unload for Local Embedded providers, and the
//! Hugging Face account shared by every Local Embedded provider.
//!
//! Every command is a thin layer over `lr_local_models`. Network requests only
//! happen inside commands the user triggers (search, repo details, header
//! inspection, downloads, sign-in); nothing here polls in the background.

use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use lr_api_keys::{CachedKeychain, KeychainStorage};
use lr_local_models::auth::{HF_AUTHORIZE_URL, HF_OAUTH_CLIENT_ID, HF_OAUTH_SCOPES, HF_TOKEN_URL};
use lr_local_models::{
    CompletedDownload, DownloadEvents, DownloadJobView, DownloadManager, DownloadState,
    FitEstimate, GgufSummary, GgufVariant, HardwareInfo, HfAccount, HfCredentials, HubClient,
    HubFile, HubPage, HubSearch, KvCacheType, Library, LibraryEntry, LibraryError, ModelKind,
    SecretStore,
};
use lr_oauth::browser::{FlowId, OAuthFlowConfig, OAuthFlowManager, OAuthFlowResult};
use lr_providers::embedded::{EmbeddedCatalogModel, EmbeddedModelState};
use lr_providers::registry::ProviderRegistry;

/// Keychain service holding the Hugging Face credentials.
pub const HF_KEYCHAIN_SERVICE: &str = "LocalRouter-HuggingFace";
/// Account prefix the OAuth flow manager files tokens under
/// (`{prefix}_access_token`, `{prefix}_refresh_token`, `{prefix}_expires_at`),
/// which are exactly the keys `HfCredentials` reads.
const HF_OAUTH_ACCOUNT: &str = "hf_oauth";

/// Every download job update (throttled progress and each state change).
/// Payload: [`DownloadJobView`].
pub const EVENT_DOWNLOAD_PROGRESS: &str = "local-model-download-progress";
/// A job reached done, failed or cancelled. Payload: [`DownloadFinishedEvent`].
pub const EVENT_DOWNLOAD_FINISHED: &str = "local-model-download-finished";
/// The library gained entries from a download. Payload: [`LibraryChangedEvent`].
pub const EVENT_LIBRARY_CHANGED: &str = "local-models-library-changed";

/// Parallel downloads.
const MAX_CONCURRENT_DOWNLOADS: usize = 2;
const MAX_QUERY_LEN: usize = 200;
const MAX_FILTERS: usize = 10;
const MAX_NAME_LEN: usize = 200;
const SORT_KEYS: &[&str] = &["downloads", "likes", "trendingScore", "lastModified"];

// ---------------------------------------------------------------------------
// Keychain-backed secret store
// ---------------------------------------------------------------------------

/// [`SecretStore`] over the app keychain (the OS keychain, or the dev file
/// store with `LOCALROUTER_KEYCHAIN=file`), one entry per key under
/// [`HF_KEYCHAIN_SERVICE`].
pub struct KeychainSecretStore {
    keychain: Arc<dyn KeychainStorage>,
    service: String,
}

impl KeychainSecretStore {
    pub fn new(keychain: Arc<dyn KeychainStorage>, service: &str) -> Self {
        Self {
            keychain,
            service: service.to_string(),
        }
    }
}

impl SecretStore for KeychainSecretStore {
    fn get(&self, key: &str) -> Option<String> {
        match self.keychain.get(&self.service, key) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("could not read {key} from the keychain: {e}");
                None
            }
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.keychain
            .store(&self.service, key, value)
            .map_err(|e| e.to_string())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        self.keychain
            .delete(&self.service, key)
            .map_err(|e| e.to_string())
    }
}

/// Hugging Face credentials kept in the app keychain.
pub fn hf_credentials(keychain: CachedKeychain, hub: HubClient) -> Arc<HfCredentials> {
    let store = KeychainSecretStore::new(Arc::new(keychain), HF_KEYCHAIN_SERVICE);
    Arc::new(HfCredentials::new(Arc::new(store), hub))
}

// ---------------------------------------------------------------------------
// Download events
// ---------------------------------------------------------------------------

/// Where events go (the Tauri app, or a recorder in tests).
pub trait EventSink: Send + Sync {
    fn emit_json(&self, event: &str, payload: serde_json::Value);
}

/// Emits to every window of the app.
pub struct TauriEventSink(pub AppHandle);

impl EventSink for TauriEventSink {
    fn emit_json(&self, event: &str, payload: serde_json::Value) {
        if let Err(e) = self.0.emit(event, payload) {
            tracing::debug!("could not emit {event}: {e}");
        }
    }
}

/// Payload of [`EVENT_DOWNLOAD_FINISHED`].
#[derive(Serialize, Clone, Debug)]
pub struct DownloadFinishedEvent {
    pub job: DownloadJobView,
    /// Library ids added or updated by a finished download.
    pub added_models: Vec<String>,
    /// Set when the files downloaded but could not be added to the library.
    pub library_error: Option<String>,
}

/// Payload of [`EVENT_LIBRARY_CHANGED`].
#[derive(Serialize, Clone, Debug)]
pub struct LibraryChangedEvent {
    pub added_models: Vec<String>,
}

/// `(repo, commit, files)`: identifies a job from both a
/// [`DownloadJobView`] and a [`CompletedDownload`].
type JobKey = (String, String, BTreeSet<String>);

fn job_key<'a>(repo: &str, revision: &str, files: impl Iterator<Item = &'a String>) -> JobKey {
    (
        repo.to_string(),
        revision.to_string(),
        files.cloned().collect(),
    )
}

/// Turns download-manager updates into app events and remembers what the
/// completion hook added to the library, so the finished event can say so.
pub struct DownloadEventBridge {
    sink: Arc<dyn EventSink>,
    last_state: Mutex<HashMap<String, DownloadState>>,
    outcomes: Mutex<HashMap<JobKey, Result<Vec<String>, String>>>,
}

impl DownloadEventBridge {
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        Self {
            sink,
            last_state: Mutex::new(HashMap::new()),
            outcomes: Mutex::new(HashMap::new()),
        }
    }

    /// Tell listeners the library changed (the app refreshes model lists).
    pub fn library_changed(&self, added_models: Vec<String>) {
        self.emit(EVENT_LIBRARY_CHANGED, &LibraryChangedEvent { added_models });
    }

    /// Record the result of adding a completed download to the library
    /// (called from the completion hook, before the job is reported done).
    pub fn record_completion(
        &self,
        done: &CompletedDownload,
        result: Result<Vec<LibraryEntry>, LibraryError>,
    ) {
        let key = job_key(&done.repo, &done.revision, done.files.iter().map(|f| &f.0));
        let outcome = match result {
            Ok(entries) => {
                let ids: Vec<String> = entries.into_iter().map(|e| e.id).collect();
                if !ids.is_empty() {
                    self.emit(
                        EVENT_LIBRARY_CHANGED,
                        &LibraryChangedEvent {
                            added_models: ids.clone(),
                        },
                    );
                }
                Ok(ids)
            }
            Err(e) => {
                tracing::warn!("could not add {} to the model library: {e}", done.repo);
                Err(e.to_string())
            }
        };
        self.outcomes.lock().insert(key, outcome);
    }

    /// Forget finished jobs the manager no longer lists.
    pub fn retain_jobs(&self, ids: &[String]) {
        self.last_state.lock().retain(|id, _| ids.contains(id));
    }

    fn emit<T: Serialize>(&self, event: &str, payload: &T) {
        match serde_json::to_value(payload) {
            Ok(v) => self.sink.emit_json(event, v),
            Err(e) => tracing::warn!("could not serialize {event}: {e}"),
        }
    }
}

impl DownloadEvents for DownloadEventBridge {
    fn on_update(&self, job: &DownloadJobView) {
        self.emit(EVENT_DOWNLOAD_PROGRESS, job);
        let previous = self.last_state.lock().insert(job.id.clone(), job.state);
        if !job.state.is_finished() || previous == Some(job.state) {
            return;
        }
        let (added_models, library_error) = if job.state == DownloadState::Done {
            let key = job_key(&job.repo, &job.revision, job.files.iter());
            match self.outcomes.lock().remove(&key) {
                Some(Ok(ids)) => (ids, None),
                Some(Err(e)) => (Vec::new(), Some(e)),
                None => (Vec::new(), None),
            }
        } else {
            (Vec::new(), None)
        };
        self.emit(
            EVENT_DOWNLOAD_FINISHED,
            &DownloadFinishedEvent {
                job: job.clone(),
                added_models,
                library_error,
            },
        );
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Managed state behind the local-model commands.
pub struct LocalModels {
    hub: HubClient,
    credentials: Arc<HfCredentials>,
    downloads: Arc<DownloadManager>,
    library: Arc<Library>,
    events: Arc<DownloadEventBridge>,
    sign_in_flow: Mutex<Option<FlowId>>,
    images: Arc<ImageModels>,
}

/// Image models for the stable-diffusion.cpp provider: bundle files are
/// downloaded with the shared download manager (tagged with an image
/// purpose) and tracked by the image model store.
pub struct ImageModels {
    store: Arc<lr_local_models::ImageModelStore>,
    library: Arc<Library>,
    downloads: Arc<DownloadManager>,
}

impl ImageModels {
    fn jobs_for(&self, id: &str) -> Vec<DownloadJobView> {
        let purpose = lr_local_models::image_models::purpose_for(id);
        self.downloads
            .jobs()
            .into_iter()
            .filter(|j| j.purpose.as_deref() == Some(purpose.as_str()))
            .collect()
    }
}

#[async_trait::async_trait]
impl lr_providers::embedded::ImageModelBackend for ImageModels {
    fn models(&self) -> Vec<lr_providers::embedded::ImageModelStatus> {
        self.store
            .catalog(&self.library)
            .into_iter()
            .map(|m| {
                let jobs = self.jobs_for(&m.id);
                let active: Vec<&DownloadJobView> =
                    jobs.iter().filter(|j| !j.state.is_finished()).collect();
                let downloading = !m.downloaded && !active.is_empty();
                let progress = downloading.then(|| {
                    let done: u64 = active.iter().map(|j| j.bytes_done).sum();
                    let total: u64 = active.iter().map(|j| j.bytes_total).sum();
                    if total == 0 {
                        0.0
                    } else {
                        done as f64 / total as f64
                    }
                });
                let error = (!m.downloaded && active.is_empty())
                    .then(|| {
                        jobs.iter()
                            .rev()
                            .find(|j| j.state == DownloadState::Failed)
                            .and_then(|j| j.error.clone())
                    })
                    .flatten();
                lr_providers::embedded::ImageModelStatus {
                    id: m.id,
                    name: m.name,
                    description: m.description,
                    total_bytes: m.total_bytes,
                    downloaded: m.downloaded,
                    downloading,
                    progress,
                    error,
                }
            })
            .collect()
    }

    fn launch(&self, id: &str) -> Option<lr_local_models::ImageModelLaunch> {
        self.store.launch(id, &self.library)
    }

    async fn start_download(&self, id: &str) -> Result<(), String> {
        let plan = self
            .store
            .download_plan(id, &self.library)
            .ok_or_else(|| format!("Unknown image model '{id}'"))?;
        let purpose = lr_local_models::image_models::purpose_for(id);
        for (repo, files) in plan {
            self.downloads
                .start_for(&repo, None, files, Some(&purpose))
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn cancel_download(&self, id: &str) {
        for job in self.jobs_for(id) {
            if !job.state.is_finished() {
                self.downloads.cancel(&job.id);
            }
        }
    }

    fn remove(&self, id: &str) -> Result<(), String> {
        if self.store.remove(id, &self.library, true) {
            lr_providers::embedded::notify_models_changed(
                lr_providers::embedded::sdcpp::PROVIDER_TYPE,
            );
            Ok(())
        } else {
            Err(format!("Unknown image model '{id}'"))
        }
    }
}

impl LocalModels {
    /// Create the download manager (storage = the library's directory) with
    /// a completion hook that adds finished downloads to the library.
    /// Persisted jobs come back paused; nothing starts on its own.
    pub fn new(
        library: Arc<Library>,
        credentials: Arc<HfCredentials>,
        hub: HubClient,
        sink: Arc<dyn EventSink>,
    ) -> Arc<Self> {
        let events = Arc::new(DownloadEventBridge::new(sink));
        let token_creds = credentials.clone();
        let downloads = DownloadManager::new(
            library.storage_dir().to_path_buf(),
            hub.clone(),
            MAX_CONCURRENT_DOWNLOADS,
            events.clone(),
            Arc::new(move || token_creds.token()),
        );
        let image_store = Arc::new(lr_local_models::ImageModelStore::open(
            library.storage_dir(),
        ));
        {
            let library = library.clone();
            let events = events.clone();
            let image_store = image_store.clone();
            downloads.on_complete(Arc::new(move |done: CompletedDownload| {
                // Image model files belong to the image store, not the
                // llama.cpp library.
                if image_store.record_completed(&done) {
                    lr_providers::embedded::notify_models_changed(
                        lr_providers::embedded::sdcpp::PROVIDER_TYPE,
                    );
                    return;
                }
                let result = library.add_downloaded(&done);
                events.record_completion(&done, result);
            }));
        }
        let images = Arc::new(ImageModels {
            store: image_store,
            library: library.clone(),
            downloads: downloads.clone(),
        });
        Arc::new(Self {
            hub,
            credentials,
            downloads,
            library,
            events,
            sign_in_flow: Mutex::new(None),
            images,
        })
    }

    /// The image model backend for the stable-diffusion.cpp provider.
    pub fn image_backend(&self) -> Arc<dyn lr_providers::embedded::ImageModelBackend> {
        self.images.clone()
    }

    /// The token to send to the Hub (refreshing an expiring OAuth token).
    async fn token(&self) -> Option<String> {
        self.credentials.refresh_if_needed().await
    }
}

// ---------------------------------------------------------------------------
// Input validation
// ---------------------------------------------------------------------------

fn validate_filter(f: &str) -> Result<(), String> {
    let ok = !f.is_empty()
        && f.len() <= 64
        && f.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/'));
    if ok {
        Ok(())
    } else {
        Err(format!("Invalid search filter {f:?}"))
    }
}

fn validate_sort(sort: Option<&str>) -> Result<(), String> {
    match sort {
        None | Some("") => Ok(()),
        Some(s) if SORT_KEYS.contains(&s) => Ok(()),
        Some(s) => Err(format!(
            "Unknown sort {s:?}; expected one of {}",
            SORT_KEYS.join(", ")
        )),
    }
}

/// A file path inside a repository: relative, no `.`/`..` segments.
fn validate_repo_file(path: &str) -> Result<(), String> {
    let bad = path.is_empty()
        || path.len() > 1024
        || path.starts_with('/')
        || path.contains(['\\', ':', '\0'])
        || path
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..");
    if bad {
        Err(format!("Invalid file path {path:?}"))
    } else {
        Ok(())
    }
}

/// A model id an engine names itself: a library id, or an Ollaya name
/// such as `laya:en` or `acme/triage:v1` (`/`-separated segments of
/// `[a-z0-9._:-]`, none empty, `.` or `..`).
fn validate_engine_model_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= 200
        && id.split('/').all(|seg| {
            !seg.is_empty()
                && seg != "."
                && seg != ".."
                && seg.chars().all(|c| {
                    c.is_ascii_lowercase()
                        || c.is_ascii_digit()
                        || matches!(c, '.' | '_' | '-' | ':')
                })
        });
    if ok {
        Ok(())
    } else {
        Err(format!("Invalid model id {id:?}"))
    }
}

/// A library id (`[a-z0-9._-]`, as the library generates them).
fn validate_model_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= 200
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(format!("Invalid model id {id:?}"))
    }
}

fn validate_display_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("The name must not be empty".into());
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(format!(
            "The name must be at most {MAX_NAME_LEN} characters"
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("The name must not contain control characters".into());
    }
    Ok(name.to_string())
}

/// A local GGUF to import: an absolute path to an existing `.gguf` file with
/// no `..` components. (The library then checks the GGUF header.)
fn validate_import_path(raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains('\0') {
        return Err("Choose a GGUF file to import".into());
    }
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err("The path must be absolute".into());
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("The path must not contain '..'".into());
    }
    let is_gguf = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gguf"));
    if !is_gguf {
        return Err("Only .gguf files can be imported".into());
    }
    if !path.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    Ok(path.to_path_buf())
}

// ---------------------------------------------------------------------------
// Hugging Face Hub
// ---------------------------------------------------------------------------

/// Search the Hugging Face Hub. `filters` are Hub tags (e.g. `gguf`); `sort`
/// is `downloads`, `likes`, `trendingScore` or `lastModified`; `cursor` is a
/// previous page's `next_cursor`.
#[tauri::command]
pub async fn local_models_search(
    query: Option<String>,
    filters: Option<Vec<String>>,
    sort: Option<String>,
    cursor: Option<String>,
    limit: Option<u32>,
    state: State<'_, Arc<LocalModels>>,
) -> Result<HubPage, String> {
    let query = query
        .map(|q| q.trim().to_string())
        .filter(|q| !q.is_empty());
    if query
        .as_ref()
        .is_some_and(|q| q.chars().count() > MAX_QUERY_LEN)
    {
        return Err(format!(
            "The search text must be at most {MAX_QUERY_LEN} characters"
        ));
    }
    let filters = filters.unwrap_or_default();
    if filters.len() > MAX_FILTERS {
        return Err("Too many search filters".into());
    }
    for f in &filters {
        validate_filter(f)?;
    }
    validate_sort(sort.as_deref())?;
    let search = HubSearch {
        query,
        filters,
        pipeline_tag: None,
        sort: sort.filter(|s| !s.is_empty()),
        limit,
        cursor,
    };
    let token = state.token().await;
    state
        .hub
        .search(&search, token.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// Repository details for the model drawer.
#[derive(Serialize, Clone, Debug)]
pub struct LocalRepoDetails {
    pub id: String,
    /// Commit SHA of the revision; pass it back as `revision` so inspection
    /// and downloads use the same files.
    pub sha: Option<String>,
    /// `None` when not gated, else `auto` / `manual`.
    pub gated: Option<String>,
    pub gate_prompt: Option<String>,
    pub license: Option<String>,
    pub pipeline_tag: Option<String>,
    /// The repository page on the Hub (for "Request access").
    pub repo_url: String,
    /// GGUF models (split parts grouped), smallest first.
    pub variants: Vec<GgufVariant>,
    /// Every file in the repository.
    pub files: Vec<HubFile>,
}

/// Files, GGUF variants and gating of a repository.
#[tauri::command]
pub async fn local_models_repo(
    repo: String,
    revision: Option<String>,
    state: State<'_, Arc<LocalModels>>,
) -> Result<LocalRepoDetails, String> {
    let token = state.token().await;
    let info = state
        .hub
        .model_info(repo.trim(), revision.as_deref(), token.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    Ok(LocalRepoDetails {
        repo_url: state.hub.repo_url(&info.id),
        variants: lr_local_models::gguf_variants(&info.siblings),
        id: info.id,
        sha: info.sha,
        gated: info.gated,
        gate_prompt: info.gate_prompt,
        license: info.license,
        pipeline_tag: info.pipeline_tag,
        files: info.siblings,
    })
}

/// A remote GGUF's header facts and whether it fits this machine.
#[derive(Serialize, Clone, Debug)]
pub struct RemoteModelInspection {
    pub summary: GgufSummary,
    pub kind: ModelKind,
    pub fit: FitEstimate,
    /// Largest standard context (4K…128K) that fits; `None` when unknown or
    /// nothing fits.
    pub max_context: Option<u64>,
    pub hardware: HardwareInfo,
}

async fn detect_hardware() -> Result<HardwareInfo, String> {
    tokio::task::spawn_blocking(lr_local_models::hardware::detect)
        .await
        .map_err(|e| format!("Hardware detection failed: {e}"))
}

/// Read the header of `path` in `repo` (Range requests, a few MB at most)
/// and estimate memory use. `size_bytes` is the total size of the variant
/// (all split parts); `context_length` 0/None = the trained context.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn local_models_inspect_remote(
    repo: String,
    revision: Option<String>,
    path: String,
    size_bytes: u64,
    context_length: Option<u64>,
    kv_cache: Option<KvCacheType>,
    state: State<'_, Arc<LocalModels>>,
) -> Result<RemoteModelInspection, String> {
    validate_repo_file(&path)?;
    let token = state.token().await;
    let header = state
        .hub
        .gguf_header(repo.trim(), revision.as_deref(), &path, token.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    let summary = GgufSummary::from_header(&header);
    let kind = lr_local_models::classify(&summary, header.general_type());
    let hardware = detect_hardware().await?;
    let kv = kv_cache.unwrap_or(KvCacheType::F16);
    let fit = lr_local_models::estimate(
        size_bytes,
        Some(&summary),
        context_length.unwrap_or(0),
        kv,
        1,
        &hardware,
    );
    let max_context =
        lr_local_models::max_context_that_fits(size_bytes, Some(&summary), kv, 1, &hardware);
    Ok(RemoteModelInspection {
        summary,
        kind,
        fit,
        max_context,
        hardware,
    })
}

/// This machine's memory and CPU (no network).
#[tauri::command]
pub async fn local_models_hardware() -> Result<HardwareInfo, String> {
    detect_hardware().await
}

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------

/// Download `files` (one model, optionally with its projector) from `repo`
/// at `revision` (default `main`). Returns the job id. Progress arrives as
/// `local-model-download-progress` events; `local-model-download-finished`
/// ends the job, and finished models are added to the library.
#[tauri::command]
pub async fn local_models_download_start(
    repo: String,
    revision: Option<String>,
    files: Vec<String>,
    state: State<'_, Arc<LocalModels>>,
) -> Result<String, String> {
    for f in &files {
        validate_repo_file(f)?;
    }
    // Refresh an expiring OAuth token before the manager reads it.
    state.token().await;
    state
        .downloads
        .start(repo.trim(), revision.as_deref(), files)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn local_models_download_pause(
    id: String,
    state: State<'_, Arc<LocalModels>>,
) -> Result<(), String> {
    state.downloads.pause(&id);
    Ok(())
}

#[tauri::command]
pub async fn local_models_download_resume(
    id: String,
    state: State<'_, Arc<LocalModels>>,
) -> Result<(), String> {
    state.token().await;
    state.downloads.resume(&id).await;
    Ok(())
}

/// Cancel a download and delete its partial files.
#[tauri::command]
pub async fn local_models_download_cancel(
    id: String,
    state: State<'_, Arc<LocalModels>>,
) -> Result<(), String> {
    state.downloads.cancel(&id);
    Ok(())
}

/// All download jobs (unfinished ones from earlier sessions are paused).
#[tauri::command]
pub async fn local_models_downloads(
    state: State<'_, Arc<LocalModels>>,
) -> Result<Vec<DownloadJobView>, String> {
    // Image model downloads show in the stable-diffusion.cpp Models tab.
    Ok(state
        .downloads
        .jobs()
        .into_iter()
        .filter(|j| j.purpose.is_none())
        .collect())
}

/// Forget finished download jobs.
#[tauri::command]
pub async fn local_models_downloads_clear(
    state: State<'_, Arc<LocalModels>>,
) -> Result<(), String> {
    state.downloads.remove_finished();
    let ids: Vec<String> = state.downloads.jobs().into_iter().map(|j| j.id).collect();
    state.events.retain_jobs(&ids);
    Ok(())
}

// ---------------------------------------------------------------------------
// Library
// ---------------------------------------------------------------------------

/// The installed models and their disk usage.
#[derive(Serialize, Clone, Debug)]
pub struct LocalLibraryView {
    pub entries: Vec<LibraryEntry>,
    /// Bytes used by library files inside the storage directory.
    pub disk_usage_bytes: u64,
    pub storage_dir: String,
}

#[tauri::command]
pub async fn local_models_library(
    state: State<'_, Arc<LocalModels>>,
) -> Result<LocalLibraryView, String> {
    let library = state.library.clone();
    tokio::task::spawn_blocking(move || LocalLibraryView {
        entries: library.list(),
        disk_usage_bytes: library.disk_usage(),
        storage_dir: library.storage_dir().display().to_string(),
    })
    .await
    .map_err(|e| e.to_string())
}

/// Add a local GGUF file to the library in place (never copied or moved).
#[tauri::command]
pub async fn local_models_import(
    path: String,
    state: State<'_, Arc<LocalModels>>,
) -> Result<LibraryEntry, String> {
    let path = validate_import_path(&path)?;
    let library = state.library.clone();
    let entry = tokio::task::spawn_blocking(move || library.import_file(&path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    state.events.library_changed(vec![entry.id.clone()]);
    Ok(entry)
}

#[tauri::command]
pub async fn local_models_rename(
    id: String,
    display_name: String,
    state: State<'_, Arc<LocalModels>>,
) -> Result<(), String> {
    validate_model_id(&id)?;
    let name = validate_display_name(&display_name)?;
    state
        .library
        .rename(&id, &name)
        .map_err(|e| e.to_string())?;
    state.events.library_changed(Vec::new());
    Ok(())
}

/// Remove a model from the library, unloading it first. With `delete_files`
/// its downloaded files are deleted too (imported files never are).
#[tauri::command]
pub async fn local_models_remove(
    id: String,
    delete_files: bool,
    state: State<'_, Arc<LocalModels>>,
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<(), String> {
    validate_model_id(&id)?;
    for instance in registry.list_providers() {
        if instance.provider_type != lr_providers::embedded::llamacpp::PROVIDER_TYPE {
            continue;
        }
        let Some(provider) = registry.get_provider_unchecked(&instance.instance_name) else {
            continue;
        };
        if let Some(control) = provider.embedded_control() {
            if let Err(e) = control.unload(&id).await {
                tracing::warn!("could not unload {id} from {}: {e}", instance.instance_name);
            }
        }
    }
    let library = state.library.clone();
    tokio::task::spawn_blocking(move || library.remove(&id, delete_files))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    state.events.library_changed(Vec::new());
    Ok(())
}

// ---------------------------------------------------------------------------
// Load / unload (Local Embedded providers)
// ---------------------------------------------------------------------------

fn embedded_provider(
    registry: &ProviderRegistry,
    instance_name: &str,
    enabled_only: bool,
) -> Result<Arc<dyn lr_providers::ModelProvider>, String> {
    let provider = if enabled_only {
        registry.get_provider(instance_name)
    } else {
        registry.get_provider_unchecked(instance_name)
    }
    .ok_or_else(|| {
        if enabled_only && registry.get_provider_unchecked(instance_name).is_some() {
            format!("Provider '{instance_name}' is disabled")
        } else {
            format!("Provider '{instance_name}' not found")
        }
    })?;
    if provider.embedded_control().is_none() {
        return Err(format!(
            "Provider '{instance_name}' is not a Local Embedded provider"
        ));
    }
    Ok(provider)
}

/// Start the engine for `model` and wait until it is ready (can take minutes
/// for a large model).
#[tauri::command]
pub async fn local_models_load(
    instance_name: String,
    model: String,
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<(), String> {
    validate_engine_model_id(&model)?;
    let provider = embedded_provider(&registry, &instance_name, true)?;
    let control = provider
        .embedded_control()
        .ok_or("not a Local Embedded provider")?;
    control.load(&model).await.map_err(|e| e.to_string())
}

/// Stop the engine serving `model` (it starts again on the next request).
#[tauri::command]
pub async fn local_models_unload(
    instance_name: String,
    model: String,
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<(), String> {
    validate_engine_model_id(&model)?;
    let provider = embedded_provider(&registry, &instance_name, false)?;
    let control = provider
        .embedded_control()
        .ok_or("not a Local Embedded provider")?;
    control.unload(&model).await.map_err(|e| e.to_string())
}

/// Models of a provider instance that have (or recently had) an engine
/// process; models not listed are unloaded.
#[tauri::command]
pub async fn local_models_states(
    instance_name: String,
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<Vec<EmbeddedModelState>, String> {
    let provider = embedded_provider(&registry, &instance_name, false)?;
    Ok(provider
        .embedded_control()
        .map(|c| c.model_states())
        .unwrap_or_default())
}

/// Models an engine provider (Ollaya, Laya, Kev, Von, Decider,
/// stable-diffusion.cpp) can download, with their download state (empty for
/// llama.cpp, whose models live in the library).
#[tauri::command]
pub async fn local_models_engine_catalog(
    instance_name: String,
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<Vec<EmbeddedCatalogModel>, String> {
    let provider = embedded_provider(&registry, &instance_name, false)?;
    Ok(provider
        .embedded_control()
        .map(|c| c.catalog())
        .unwrap_or_default())
}

/// Start downloading one of the engine's models in the background; poll
/// [`local_models_engine_catalog`] for progress.
#[tauri::command]
pub async fn local_models_engine_download(
    instance_name: String,
    model: String,
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<(), String> {
    validate_engine_model_id(&model)?;
    let provider = embedded_provider(&registry, &instance_name, true)?;
    let control = provider
        .embedded_control()
        .ok_or_else(|| format!("Provider '{instance_name}' is not a Local Embedded provider"))?;
    control.download(&model).await.map_err(|e| e.to_string())
}

/// Delete a downloaded engine model (image models of stable-diffusion.cpp).
#[tauri::command]
pub async fn local_models_engine_remove(
    instance_name: String,
    model: String,
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<(), String> {
    validate_engine_model_id(&model)?;
    let provider = embedded_provider(&registry, &instance_name, false)?;
    let control = provider
        .embedded_control()
        .ok_or_else(|| format!("Provider '{instance_name}' is not a Local Embedded provider"))?;
    control
        .remove_download(&model)
        .await
        .map_err(|e| e.to_string())
}

/// Stop a running engine download.
#[tauri::command]
pub async fn local_models_engine_download_cancel(
    instance_name: String,
    model: String,
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<(), String> {
    validate_engine_model_id(&model)?;
    let provider = embedded_provider(&registry, &instance_name, false)?;
    if let Some(control) = provider.embedded_control() {
        control
            .cancel_download(&model)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Hugging Face account
// ---------------------------------------------------------------------------

/// Current sign-in status (asks the Hub who the token belongs to when one is
/// stored).
#[tauri::command]
pub async fn local_models_hf_account(
    state: State<'_, Arc<LocalModels>>,
) -> Result<HfAccount, String> {
    Ok(state.credentials.account().await)
}

/// Validate and store a pasted access token (replaces an OAuth sign-in).
#[tauri::command]
pub async fn local_models_hf_set_token(
    token: String,
    state: State<'_, Arc<LocalModels>>,
) -> Result<HfAccount, String> {
    state
        .credentials
        .set_token(&token)
        .await
        .map_err(|e| e.to_string())
}

/// Delete the stored Hugging Face credentials.
#[tauri::command]
pub async fn local_models_hf_sign_out(state: State<'_, Arc<LocalModels>>) -> Result<(), String> {
    state.credentials.sign_out();
    Ok(())
}

/// OAuth configuration for "Sign in with Hugging Face": authorization code +
/// PKCE (S256) as a public CIMD client, loopback redirect on `port`.
fn hf_oauth_config(port: u16) -> OAuthFlowConfig {
    OAuthFlowConfig {
        client_id: HF_OAUTH_CLIENT_ID.to_string(),
        client_secret: None,
        auth_url: HF_AUTHORIZE_URL.to_string(),
        token_url: HF_TOKEN_URL.to_string(),
        scopes: HF_OAUTH_SCOPES.iter().map(|s| s.to_string()).collect(),
        redirect_uri: format!("http://127.0.0.1:{port}/callback"),
        callback_port: port,
        keychain_service: HF_KEYCHAIN_SERVICE.to_string(),
        account_id: HF_OAUTH_ACCOUNT.to_string(),
        extra_auth_params: HashMap::new(),
        extra_token_params: HashMap::new(),
        expected_issuer: None,
    }
}

fn free_loopback_port() -> Result<u16, String> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("Could not open a local port for the sign-in callback: {e}"))
}

/// A started browser sign-in.
#[derive(Serialize, Clone, Debug)]
pub struct HfSignInStart {
    pub flow_id: String,
    /// Open this in the browser.
    pub auth_url: String,
}

/// Start "Sign in with Hugging Face". The frontend opens `auth_url` and polls
/// [`local_models_hf_sign_in_poll`]. Starting again cancels an earlier flow.
#[tauri::command]
pub async fn local_models_hf_sign_in(
    state: State<'_, Arc<LocalModels>>,
    flows: State<'_, Arc<OAuthFlowManager>>,
) -> Result<HfSignInStart, String> {
    let previous = state.sign_in_flow.lock().take();
    if let Some(previous) = previous {
        let _ = flows.cancel_flow(previous);
    }
    let port = free_loopback_port()?;
    let start = flows
        .start_flow(hf_oauth_config(port))
        .await
        .map_err(|e| e.to_string())?;
    *state.sign_in_flow.lock() = Some(start.flow_id);
    Ok(HfSignInStart {
        flow_id: start.flow_id.to_string(),
        auth_url: start.auth_url,
    })
}

/// Sign-in progress.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HfSignInState {
    Pending,
    Success,
    Error,
    Timeout,
    Cancelled,
}

#[derive(Serialize, Clone, Debug)]
pub struct HfSignInStatus {
    pub state: HfSignInState,
    pub message: Option<String>,
    /// Set on success.
    pub account: Option<HfAccount>,
}

fn parse_flow_id(flow_id: &str) -> Result<FlowId, String> {
    FlowId::parse(flow_id).map_err(|_| "Invalid sign-in id".to_string())
}

#[tauri::command]
pub async fn local_models_hf_sign_in_poll(
    flow_id: String,
    state: State<'_, Arc<LocalModels>>,
    flows: State<'_, Arc<OAuthFlowManager>>,
) -> Result<HfSignInStatus, String> {
    let id = parse_flow_id(&flow_id)?;
    let result = flows.poll_status(id).map_err(|e| e.to_string())?;
    let status = |state, message: Option<String>| HfSignInStatus {
        state,
        message,
        account: None,
    };
    let finished = |s: &LocalModels| {
        let mut current = s.sign_in_flow.lock();
        if *current == Some(id) {
            *current = None;
        }
    };
    Ok(match result {
        OAuthFlowResult::Pending { .. } | OAuthFlowResult::ExchangingToken => {
            status(HfSignInState::Pending, None)
        }
        OAuthFlowResult::Success { tokens } => {
            finished(&state);
            // The flow manager already filed the tokens under the keys the
            // credentials read; this also drops a pasted token.
            state.credentials.store_oauth_tokens(
                &tokens.access_token,
                tokens.refresh_token.as_deref(),
                tokens.expires_at.map(|t| t.timestamp()),
            );
            HfSignInStatus {
                state: HfSignInState::Success,
                message: None,
                account: Some(state.credentials.account().await),
            }
        }
        OAuthFlowResult::Error { message } => {
            finished(&state);
            status(HfSignInState::Error, Some(message))
        }
        OAuthFlowResult::Timeout => {
            finished(&state);
            status(HfSignInState::Timeout, None)
        }
        OAuthFlowResult::Cancelled => {
            finished(&state);
            status(HfSignInState::Cancelled, None)
        }
    })
}

#[tauri::command]
pub async fn local_models_hf_sign_in_cancel(
    flow_id: String,
    state: State<'_, Arc<LocalModels>>,
    flows: State<'_, Arc<OAuthFlowManager>>,
) -> Result<(), String> {
    let id = parse_flow_id(&flow_id)?;
    {
        let mut current = state.sign_in_flow.lock();
        if *current == Some(id) {
            *current = None;
        }
    }
    flows.cancel_flow(id).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_api_keys::MockKeychain;
    use lr_local_models::auth::{
        KEY_OAUTH_ACCESS, KEY_OAUTH_EXPIRES_AT, KEY_OAUTH_REFRESH, KEY_TOKEN,
    };
    use lr_local_models::HubModelSummary;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<(String, serde_json::Value)>>);

    impl EventSink for Recorder {
        fn emit_json(&self, event: &str, payload: serde_json::Value) {
            self.0.lock().push((event.to_string(), payload));
        }
    }

    impl Recorder {
        fn events(&self, name: &str) -> Vec<serde_json::Value> {
            self.0
                .lock()
                .iter()
                .filter(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
                .collect()
        }
    }

    fn job(id: &str, state: DownloadState) -> DownloadJobView {
        DownloadJobView {
            id: id.into(),
            repo: "org/model-GGUF".into(),
            revision: "abc123".into(),
            files: vec!["m-Q4_K_M.gguf".into(), "mmproj-F16.gguf".into()],
            state,
            bytes_done: 10,
            bytes_total: 100,
            speed_bps: 5,
            current_file: None,
            error: None,
            target_dir: "/models/hf/org/model-GGUF/abc123".into(),
            purpose: None,
        }
    }

    fn entry(id: &str) -> LibraryEntry {
        LibraryEntry {
            id: id.into(),
            display_name: id.into(),
            source: lr_local_models::EntrySource::Imported,
            model_path: PathBuf::from("/m.gguf"),
            extra_parts: vec![],
            projector_path: None,
            kind: ModelKind::Chat,
            quant: None,
            architecture: None,
            context_length: None,
            pooling_type: None,
            has_tools: false,
            size_bytes: 1,
            installed_at: chrono::Utc::now(),
        }
    }

    fn completed() -> CompletedDownload {
        CompletedDownload {
            repo: "org/model-GGUF".into(),
            revision: "abc123".into(),
            purpose: None,
            // Reverse order: matching must not depend on it.
            files: vec![
                (
                    "mmproj-F16.gguf".into(),
                    PathBuf::from("/x/mmproj-F16.gguf"),
                    1,
                    None,
                ),
                ("m-Q4_K_M.gguf".into(), PathBuf::from("/x/m.gguf"), 1, None),
            ],
        }
    }

    #[test]
    fn progress_events_for_every_update_and_finished_once() {
        let rec = Arc::new(Recorder::default());
        let bridge = DownloadEventBridge::new(rec.clone());
        bridge.on_update(&job("j1", DownloadState::Queued));
        bridge.on_update(&job("j1", DownloadState::Running));
        bridge.on_update(&job("j1", DownloadState::Running));
        assert_eq!(rec.events(EVENT_DOWNLOAD_PROGRESS).len(), 3);
        assert!(rec.events(EVENT_DOWNLOAD_FINISHED).is_empty());

        bridge.on_update(&job("j1", DownloadState::Failed));
        bridge.on_update(&job("j1", DownloadState::Failed));
        let finished = rec.events(EVENT_DOWNLOAD_FINISHED);
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0]["job"]["state"], "failed");
        assert_eq!(finished[0]["job"]["id"], "j1");
        assert_eq!(finished[0]["added_models"], serde_json::json!([]));
        assert!(finished[0]["library_error"].is_null());

        // Retried after a failure: finishes again.
        bridge.on_update(&job("j1", DownloadState::Queued));
        bridge.on_update(&job("j1", DownloadState::Cancelled));
        assert_eq!(rec.events(EVENT_DOWNLOAD_FINISHED).len(), 2);
    }

    #[test]
    fn finished_event_reports_library_outcome() {
        let rec = Arc::new(Recorder::default());
        let bridge = DownloadEventBridge::new(rec.clone());
        bridge.on_update(&job("j1", DownloadState::Verifying));
        bridge.record_completion(&completed(), Ok(vec![entry("m-q4_k_m")]));
        let changed = rec.events(EVENT_LIBRARY_CHANGED);
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0]["added_models"], serde_json::json!(["m-q4_k_m"]));

        bridge.on_update(&job("j1", DownloadState::Done));
        let finished = rec.events(EVENT_DOWNLOAD_FINISHED);
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0]["job"]["state"], "done");
        assert_eq!(finished[0]["added_models"], serde_json::json!(["m-q4_k_m"]));
        // The outcome is consumed.
        assert!(bridge.outcomes.lock().is_empty());

        // A library failure is reported on the finished event.
        bridge.on_update(&job("j2", DownloadState::Verifying));
        bridge.record_completion(
            &completed(),
            Err(LibraryError::InvalidModel("bad header".into())),
        );
        bridge.on_update(&job("j2", DownloadState::Done));
        let finished = rec.events(EVENT_DOWNLOAD_FINISHED);
        assert_eq!(finished.len(), 2);
        assert!(finished[1]["library_error"]
            .as_str()
            .unwrap()
            .contains("bad header"));
        assert_eq!(rec.events(EVENT_LIBRARY_CHANGED).len(), 1);

        bridge.retain_jobs(&["j2".to_string()]);
        assert_eq!(bridge.last_state.lock().len(), 1);
    }

    #[test]
    fn keychain_store_uses_the_huggingface_service() {
        let keychain = MockKeychain::new();
        let store = KeychainSecretStore::new(Arc::new(keychain.clone()), HF_KEYCHAIN_SERVICE);
        assert_eq!(store.get(KEY_TOKEN), None);
        store.set(KEY_TOKEN, "hf_abc").unwrap();
        assert_eq!(
            keychain.get(HF_KEYCHAIN_SERVICE, KEY_TOKEN).unwrap(),
            Some("hf_abc".into())
        );
        assert_eq!(store.get(KEY_TOKEN).as_deref(), Some("hf_abc"));
        store.delete(KEY_TOKEN).unwrap();
        assert_eq!(store.get(KEY_TOKEN), None);
        // Deleting a missing key is fine.
        store.delete(KEY_TOKEN).unwrap();
    }

    #[test]
    fn oauth_flow_files_tokens_under_the_credential_keys() {
        // The flow manager stores `{account_id}_access_token` etc.
        for (suffix, key) in [
            ("access_token", KEY_OAUTH_ACCESS),
            ("refresh_token", KEY_OAUTH_REFRESH),
            ("expires_at", KEY_OAUTH_EXPIRES_AT),
        ] {
            assert_eq!(format!("{HF_OAUTH_ACCOUNT}_{suffix}"), key);
        }
        let cfg = hf_oauth_config(51234);
        assert_eq!(cfg.redirect_uri, "http://127.0.0.1:51234/callback");
        assert_eq!(cfg.callback_port, 51234);
        assert_eq!(cfg.client_id, HF_OAUTH_CLIENT_ID);
        assert!(cfg.client_secret.is_none());
        assert_eq!(
            cfg.scopes,
            vec!["openid", "profile", "read-repos", "gated-repos"]
        );
        assert_eq!(cfg.keychain_service, HF_KEYCHAIN_SERVICE);
        assert!(free_loopback_port().unwrap() > 0);
    }

    #[test]
    fn credentials_read_tokens_from_the_keychain() {
        let keychain = CachedKeychain::new(Arc::new(MockKeychain::new()));
        let creds = hf_credentials(keychain.clone(), HubClient::default());
        assert_eq!(creds.token(), None);
        // What the OAuth flow manager writes after a successful sign-in.
        keychain
            .store(HF_KEYCHAIN_SERVICE, KEY_OAUTH_ACCESS, "oauth_tok")
            .unwrap();
        assert_eq!(creds.token().as_deref(), Some("oauth_tok"));
        creds.sign_out();
        assert_eq!(creds.token(), None);
        assert_eq!(
            keychain.get(HF_KEYCHAIN_SERVICE, KEY_OAUTH_ACCESS).unwrap(),
            None
        );
    }

    #[test]
    fn search_inputs_are_validated() {
        assert!(validate_filter("gguf").is_ok());
        assert!(validate_filter("license:mit").is_ok());
        assert!(validate_filter("").is_err());
        assert!(validate_filter("a b").is_err());
        assert!(validate_filter("x&sort=likes").is_err());
        assert!(validate_sort(None).is_ok());
        assert!(validate_sort(Some("trendingScore")).is_ok());
        assert!(validate_sort(Some("random")).is_err());
    }

    #[test]
    fn repo_file_paths_are_validated() {
        assert!(validate_repo_file("m-Q4_K_M.gguf").is_ok());
        assert!(validate_repo_file("sub/dir/m.gguf").is_ok());
        for bad in [
            "",
            "/etc/passwd",
            "../x.gguf",
            "a/../../b.gguf",
            "a//b.gguf",
            "./m.gguf",
            "a\\b.gguf",
            "C:x.gguf",
        ] {
            assert!(validate_repo_file(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn model_ids_and_names_are_validated() {
        assert!(validate_model_id("qwen3-8b-q4_k_m").is_ok());
        assert!(validate_model_id("model.v2").is_ok());
        for bad in ["", "..", "Qwen", "a/b", "a:b", "a b"] {
            assert!(validate_model_id(bad).is_err(), "{bad}");
        }
        for good in ["qwen3-8b-q4_k_m", "laya:en", "kev:0.8b", "acme/triage:v1"] {
            assert!(validate_engine_model_id(good).is_ok(), "{good}");
        }
        for bad in ["", "..", "a/../b", "/abs", "a//b", "Laya:en", "a b", "a\\b"] {
            assert!(validate_engine_model_id(bad).is_err(), "{bad}");
        }
        assert_eq!(validate_display_name("  My model ").unwrap(), "My model");
        assert!(validate_display_name("   ").is_err());
        assert!(validate_display_name("a\nb").is_err());
        assert!(validate_display_name(&"x".repeat(MAX_NAME_LEN + 1)).is_err());
    }

    #[test]
    fn import_paths_are_validated() {
        let dir = tempfile::tempdir().unwrap();
        let gguf = dir.path().join("model.GGUF");
        std::fs::write(&gguf, b"GGUF").unwrap();
        let txt = dir.path().join("notes.txt");
        std::fs::write(&txt, b"x").unwrap();
        let sub = dir.path().join("sub.gguf");
        std::fs::create_dir(&sub).unwrap();

        assert_eq!(
            validate_import_path(&format!("  {}  ", gguf.display())).unwrap(),
            gguf
        );
        assert!(validate_import_path("").is_err());
        assert!(validate_import_path("model.gguf").is_err());
        assert!(validate_import_path(&txt.display().to_string()).is_err());
        assert!(validate_import_path(&sub.display().to_string()).is_err());
        assert!(
            validate_import_path(&dir.path().join("missing.gguf").display().to_string()).is_err()
        );
        let traversal = dir.path().join("sub.gguf").join("..").join("model.GGUF");
        assert!(validate_import_path(&traversal.display().to_string())
            .unwrap_err()
            .contains(".."));
    }

    #[test]
    fn import_rejects_files_that_are_not_gguf_models() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().join("lib"));
        let fake = dir.path().join("fake.gguf");
        std::fs::write(&fake, b"not a gguf file at all").unwrap();
        let path = validate_import_path(&fake.display().to_string()).unwrap();
        assert!(library.import_file(&path).is_err());
        assert!(library.list().is_empty());
    }

    #[test]
    fn summaries_serialize_for_the_ui() {
        // Types the UI relies on keep their snake_case wire names.
        let page = HubPage {
            models: vec![HubModelSummary {
                id: "org/m".into(),
                author: Some("org".into()),
                downloads: 1,
                likes: 2,
                last_modified: None,
                gated: Some("auto".into()),
                pipeline_tag: None,
                library_name: None,
                tags: vec![],
                parameters: None,
                architecture: None,
                context_length: None,
            }],
            next_cursor: None,
        };
        let v = serde_json::to_value(&page).unwrap();
        assert_eq!(v["models"][0]["gated"], "auto");
        assert!(v["next_cursor"].is_null());
        let status = HfSignInStatus {
            state: HfSignInState::Timeout,
            message: None,
            account: None,
        };
        assert_eq!(serde_json::to_value(status).unwrap()["state"], "timeout");
    }
}
