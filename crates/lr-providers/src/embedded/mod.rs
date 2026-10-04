//! Local Embedded providers: LocalRouter launches and supervises the inference engine
//! itself (llama.cpp, stable-diffusion.cpp, Ollaya, and the older Laya, Kev, Von and
//! Decider providers), with models managed in-app. Engines are installed by the user
//! through their package manager or downloaded by LocalRouter (`lr_engines`).

pub mod decider;
pub mod kev;
pub mod laya;
pub mod llamacpp;
pub mod ollaya;
pub mod ollaya_registry;
pub mod sdcpp;
pub mod von;

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use lr_engines::EngineError;
use lr_types::AppError;

pub use decider::{DeciderEmbeddedProvider, DeciderEmbeddedProviderFactory};
pub use kev::{KevEmbeddedProvider, KevEmbeddedProviderFactory};
pub use laya::{LayaEmbeddedProvider, LayaEmbeddedProviderFactory};
pub use llamacpp::{LlamaCppEmbeddedProvider, LlamaCppEmbeddedProviderFactory};
pub use ollaya::{OllayaEmbeddedProvider, OllayaEmbeddedProviderFactory};
pub use sdcpp::{
    set_image_model_backend, ImageModelBackend, ImageModelStatus, SdCppEmbeddedProvider,
    SdCppEmbeddedProviderFactory,
};
pub use von::{VonEmbeddedProvider, VonEmbeddedProviderFactory};

/// State of one model served by a Local Embedded provider.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EmbeddedModelState {
    /// Model id as clients use it (without the provider prefix).
    pub model: String,
    /// `running`, `exited` or `failed` for models with a process; models
    /// without one are omitted (unloaded).
    pub state: lr_engines::EngineState,
    pub port: Option<u16>,
    pub idle_secs: Option<u64>,
    pub last_error: Option<String>,
}

/// Controls a Local Embedded provider exposes to the app (Load/Unload buttons).
#[async_trait::async_trait]
pub trait EmbeddedControl: Send + Sync {
    /// Start the engine for `model` and wait until it is ready.
    async fn load(&self, model: &str) -> Result<(), AppError>;
    /// Stop the engine serving `model` (it starts again on the next request).
    async fn unload(&self, model: &str) -> Result<(), AppError>;
    /// Models that currently have (or recently had) an engine process.
    fn model_states(&self) -> Vec<EmbeddedModelState>;
    /// Models the engine can serve once downloaded, with their download
    /// state. Empty for engines whose models live in the in-app library
    /// (llama.cpp).
    fn catalog(&self) -> Vec<EmbeddedCatalogModel> {
        Vec::new()
    }
    /// Start downloading `model` in the background (the engine fetches it
    /// from Hugging Face). Requests never download on demand.
    async fn download(&self, model: &str) -> Result<(), AppError> {
        Err(AppError::InvalidParams(format!(
            "'{model}' is managed in the model library, not downloaded by the engine"
        )))
    }
    /// Stop a running download.
    async fn cancel_download(&self, _model: &str) -> Result<(), AppError> {
        Ok(())
    }
    /// Delete a downloaded model (only where [`EmbeddedCatalogModel::removable`]).
    async fn remove_download(&self, model: &str) -> Result<(), AppError> {
        Err(AppError::InvalidParams(format!(
            "'{model}' cannot be removed here"
        )))
    }
}

/// A model a Local Embedded engine downloads itself, with its state.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EmbeddedCatalogModel {
    pub id: String,
    pub name: String,
    /// Approximate download, e.g. "3.8 GB".
    pub download_size: String,
    pub guidance: Option<String>,
    pub downloaded: bool,
    pub downloading: bool,
    /// Why the last download failed, if it did.
    pub download_error: Option<String>,
    /// Download progress 0.0–1.0 while downloading, when known.
    pub progress: Option<f64>,
    /// A downloaded model can be deleted from the Models tab.
    pub removable: bool,
    /// Why the model cannot be downloaded here (e.g. it needs a newer
    /// engine); the Models tab shows this instead of a Download button.
    pub unavailable: Option<String>,
}

/// Health of a Local Embedded provider without starting anything: the
/// engine must be installed, its last serving process must not have failed,
/// and at least one model must be downloaded.
pub(crate) fn engine_health(
    missing: Option<String>,
    has_models: bool,
    no_models: &str,
    supervisor: &lr_engines::Supervisor,
    key_prefix: &str,
) -> crate::ProviderHealth {
    use crate::HealthStatus;
    let (status, error_message) = if let Some(missing) = missing {
        (HealthStatus::Unhealthy, Some(missing))
    } else if let Some(failed) = supervisor.processes().into_iter().find(|p| {
        p.key.starts_with(key_prefix)
            && !p.key.contains(":download:")
            && p.state == lr_engines::EngineState::Failed
    }) {
        let detail = failed
            .last_error
            .unwrap_or_else(|| "it exited unexpectedly".to_string());
        (
            HealthStatus::Unhealthy,
            Some(format!("The engine failed to run: {detail}")),
        )
    } else if !has_models {
        (HealthStatus::Degraded, Some(no_models.to_string()))
    } else {
        (HealthStatus::Healthy, None)
    };
    crate::ProviderHealth {
        status,
        latency_ms: None,
        last_checked: chrono::Utc::now(),
        error_message,
    }
}

/// How long a serving engine may take to answer its warm-up decision.
pub(crate) const WARMUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Send one small decision covering every question type. System One engines
/// load lazily or compile kernels on their first decision (Decider's first
/// answer takes seconds, the next ones about one), so a started engine is
/// warmed up before it counts as loaded.
pub(crate) async fn warm_up(
    handle: &lr_engines::EngineHandle,
    provider: &str,
    timeout: std::time::Duration,
) -> Result<(), AppError> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .no_proxy()
        .build()
        .map_err(|e| AppError::Internal(format!("http client: {e}")))?;
    let resp = client
        .post(format!("{}/v1/systemone", handle.base_url()))
        .bearer_auth(handle.api_key())
        .json(&serde_json::json!({
            "state": "LocalRouter warm-up",
            "questions": {
                "ready": {"type": "noul", "instructions": "Is this a warm-up request?"},
                "kind": {"type": "choice", "instructions": "What kind of request is this?",
                         "criteria": {"warm_up": "a warm-up", "other": "anything else"}},
                "level": {"type": "score", "instructions": "How ready is the engine?",
                          "criteria": ["not ready", "ready"]}
            }
        }))
        .send()
        .await
        .map_err(|e| {
            AppError::Provider(format!(
                "Provider '{provider}' is unreachable: model warm-up failed: {e}"
            ))
        })?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Provider(format!(
            "Provider '{provider}' is unreachable: model warm-up returned {status}: {}",
            body.chars().take(300).collect::<String>()
        )));
    }
    Ok(())
}

/// Engine processes (by key and port) that finished their warm-up. The lock
/// is held during a warm-up so concurrent first requests wait for one.
#[derive(Default)]
pub(crate) struct Warmups {
    warmed: tokio::sync::Mutex<HashMap<String, u16>>,
}

impl Warmups {
    /// Warm up `handle`'s process unless it already was.
    pub(crate) async fn ensure(
        &self,
        handle: &lr_engines::EngineHandle,
        provider: &str,
    ) -> Result<(), AppError> {
        let mut warmed = self.warmed.lock().await;
        if warmed.get(&handle.key) == Some(&handle.port) {
            return Ok(());
        }
        let _lease = handle.lease();
        warm_up(handle, provider, WARMUP_TIMEOUT).await?;
        warmed.insert(handle.key.clone(), handle.port);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn is_warm(&self, key: &str, port: u16) -> bool {
        self.warmed.lock().await.get(key) == Some(&port)
    }
}

/// Environment that keeps a serving engine off the network: models must
/// already be in the Hugging Face cache, so a missing model fails at once
/// instead of downloading during a request.
pub(crate) fn offline_env() -> Vec<(String, String)> {
    [
        ("HF_HUB_OFFLINE", "1"),
        ("TRANSFORMERS_OFFLINE", "1"),
        ("HF_HUB_DISABLE_TELEMETRY", "1"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// Environment for a download run: the user's token, no telemetry.
pub(crate) fn download_env() -> Vec<(String, String)> {
    let mut env = hf_env();
    env.push(("HF_HUB_DISABLE_TELEMETRY".to_string(), "1".to_string()));
    env
}

/// The error for a request naming a model that has not been downloaded.
pub(crate) fn not_downloaded(provider: &str, model: &str) -> AppError {
    AppError::InvalidParams(format!(
        "Model '{model}' of provider '{provider}' is not downloaded. Download it in LocalRouter under Providers, {provider}, Models."
    ))
}

/// The Hugging Face hub cache the engines use: `HF_HUB_CACHE`, else
/// `HF_HOME/hub`, else `~/.cache/huggingface/hub` (from the user's shell
/// environment, which the engines inherit).
pub(crate) fn hf_hub_cache() -> Option<std::path::PathBuf> {
    let shell = lr_utils::binary::shell_env();
    let var = |k: &str| {
        shell
            .get(k)
            .cloned()
            .or_else(|| std::env::var(k).ok())
            .filter(|v| !v.trim().is_empty())
    };
    if let Some(dir) = var("HF_HUB_CACHE") {
        return Some(dir.into());
    }
    if let Some(home) = var("HF_HOME") {
        return Some(std::path::PathBuf::from(home).join("hub"));
    }
    let home = var("HOME").or_else(|| var("USERPROFILE"))?;
    Some(
        std::path::PathBuf::from(home)
            .join(".cache")
            .join("huggingface")
            .join("hub"),
    )
}

/// Whether the hub cache holds a snapshot of `repo` (`org/name`).
pub(crate) fn hf_repo_cached(cache: &std::path::Path, repo: &str) -> bool {
    let dir = cache
        .join(format!("models--{}", repo.replace('/', "--")))
        .join("snapshots");
    std::fs::read_dir(dir)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

/// Downloads run by Local Embedded engines: a download is a network-enabled
/// engine start whose readiness means the model is fetched and loaded; the
/// engine is stopped afterwards and a marker records the model as
/// downloaded. Markers live under `{state_dir}/engines/downloads/{type}/`.
pub(crate) struct EngineDownloads {
    provider_type: &'static str,
    supervisor: Arc<lr_engines::Supervisor>,
    marker_dir: std::path::PathBuf,
    active: parking_lot::Mutex<HashMap<String, String>>,
    errors: parking_lot::Mutex<HashMap<String, String>>,
    /// Tests point this at a scratch hub cache.
    hub_cache_override: parking_lot::Mutex<Option<std::path::PathBuf>>,
}

impl EngineDownloads {
    pub(crate) fn new(
        provider_type: &'static str,
        supervisor: Arc<lr_engines::Supervisor>,
    ) -> Arc<Self> {
        let marker_dir = supervisor
            .state_dir()
            .join("engines")
            .join("downloads")
            .join(provider_type);
        Arc::new(Self {
            provider_type,
            supervisor,
            marker_dir,
            active: parking_lot::Mutex::new(HashMap::new()),
            errors: parking_lot::Mutex::new(HashMap::new()),
            hub_cache_override: parking_lot::Mutex::new(None),
        })
    }

    fn marker(&self, model: &str) -> std::path::PathBuf {
        self.marker_dir.join(format!("{model}.done"))
    }

    /// Downloaded: the marker exists and, when the model's main repository
    /// is known, the Hugging Face cache still holds it.
    pub(crate) fn is_downloaded(&self, model: &str, repo: Option<&str>) -> bool {
        if !self.marker(model).is_file() {
            return false;
        }
        let cache = self.hub_cache_override.lock().clone().or_else(hf_hub_cache);
        match (repo, cache) {
            (Some(repo), Some(cache)) => hf_repo_cached(&cache, repo),
            _ => true,
        }
    }

    pub(crate) fn is_downloading(&self, model: &str) -> bool {
        self.active.lock().contains_key(model)
    }

    pub(crate) fn error(&self, model: &str) -> Option<String> {
        self.errors.lock().get(model).cloned()
    }

    /// Start downloading in the background. `spec` must not be offline;
    /// `after_ready` runs once the engine is up (e.g. a warm-up request for
    /// engines that load lazily).
    pub(crate) fn start<F, Fut>(
        self: &Arc<Self>,
        model: &str,
        spec: lr_engines::LaunchSpec,
        after_ready: F,
    ) -> Result<(), AppError>
    where
        F: FnOnce(lr_engines::EngineHandle) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), AppError>> + Send + 'static,
    {
        {
            let mut active = self.active.lock();
            if active.contains_key(model) {
                return Ok(());
            }
            active.insert(model.to_string(), spec.key.clone());
        }
        self.errors.lock().remove(model);
        let this = self.clone();
        let model = model.to_string();
        let key = spec.key.clone();
        tokio::spawn(async move {
            let result = match this.supervisor.ensure(spec).await {
                Ok(handle) => {
                    let lease = handle.lease();
                    let r = after_ready(handle).await;
                    drop(lease);
                    r
                }
                Err(e) => Err(engine_error(this.provider_type, e)),
            };
            this.supervisor.stop(&key).await;
            let cancelled = this.active.lock().remove(&model).is_none();
            match result {
                Ok(()) => {
                    let marker = this.marker(&model);
                    let written = std::fs::create_dir_all(&this.marker_dir)
                        .and_then(|_| std::fs::write(&marker, chrono::Utc::now().to_rfc3339()));
                    match written {
                        Ok(()) => models_changed(this.provider_type),
                        Err(e) => {
                            this.errors
                                .lock()
                                .insert(model, format!("could not record the download: {e}"));
                        }
                    }
                }
                Err(_) if cancelled => {}
                Err(e) => {
                    tracing::warn!("{} download of {} failed: {}", this.provider_type, model, e);
                    this.errors.lock().insert(model, e.to_string());
                }
            }
        });
        Ok(())
    }

    /// Test helper: record `model` as downloaded with `repo` present in a
    /// scratch hub cache under `dir`.
    #[cfg(test)]
    pub(crate) fn fake_downloaded(&self, dir: &std::path::Path, model: &str, repo: &str) {
        let snapshot = dir
            .join(format!("models--{}", repo.replace('/', "--")))
            .join("snapshots")
            .join("0");
        std::fs::create_dir_all(snapshot).unwrap();
        *self.hub_cache_override.lock() = Some(dir.to_path_buf());
        std::fs::create_dir_all(&self.marker_dir).unwrap();
        std::fs::write(self.marker(model), "test").unwrap();
    }

    /// Stop a running download (the partial files stay in the cache and the
    /// next download resumes them).
    pub(crate) async fn cancel(&self, model: &str) {
        let key = self.active.lock().remove(model);
        if let Some(key) = key {
            self.supervisor.stop(&key).await;
        }
    }
}

/// Model states from the supervisor's processes whose key starts with
/// `prefix`; `model_of` maps a process key to the model id.
pub(crate) fn states_from_supervisor(
    supervisor: &lr_engines::Supervisor,
    prefix: &str,
    model_of: impl Fn(&str) -> Vec<String>,
) -> Vec<EmbeddedModelState> {
    supervisor
        .processes()
        .into_iter()
        .filter(|p| p.key.starts_with(prefix))
        .flat_map(|p| {
            model_of(&p.key)
                .into_iter()
                .map(|model| EmbeddedModelState {
                    model,
                    state: p.state,
                    port: p.port,
                    idle_secs: p.idle_secs,
                    last_error: p.last_error.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

type TokenSource = Arc<dyn Fn() -> Option<String> + Send + Sync>;

type ModelsChangedHook = Arc<dyn Fn(&str) + Send + Sync>;

static MODELS_CHANGED_HOOK: RwLock<Option<ModelsChangedHook>> = RwLock::new(None);

/// Register what runs when a Local Embedded provider type's servable models
/// change (a download finished): the app refreshes its model lists.
pub fn set_models_changed_hook(hook: ModelsChangedHook) {
    *MODELS_CHANGED_HOOK.write() = Some(hook);
}

/// Tell the app that `provider_type`'s servable models changed (a download
/// finished), so it refreshes its model lists.
pub fn notify_models_changed(provider_type: &str) {
    models_changed(provider_type);
}

fn models_changed(provider_type: &str) {
    let hook = MODELS_CHANGED_HOOK.read().clone();
    if let Some(hook) = hook {
        hook(provider_type);
    }
}

static HF_TOKEN_SOURCE: RwLock<Option<TokenSource>> = RwLock::new(None);

/// Register where Local Embedded providers get the user's Hugging Face token (passed
/// to engines as `HF_TOKEN` for gated or private downloads).
pub fn set_hf_token_source(source: TokenSource) {
    *HF_TOKEN_SOURCE.write() = Some(source);
}

pub(crate) fn hf_token() -> Option<String> {
    HF_TOKEN_SOURCE.read().as_ref().and_then(|f| f())
}

/// Environment entries for the Hugging Face token, if the user signed in.
pub(crate) fn hf_env() -> Vec<(String, String)> {
    hf_token()
        .map(|t| vec![("HF_TOKEN".to_string(), t)])
        .unwrap_or_default()
}

/// Engine failures are reported as unreachable so auto-routing tries the
/// next model and the provider is marked unhealthy.
pub(crate) fn engine_error(provider: &str, e: EngineError) -> AppError {
    AppError::Provider(format!("Provider '{provider}' is unreachable: {e}"))
}

pub(crate) fn engine_missing(provider: &str, what: &str) -> AppError {
    AppError::Provider(format!(
        "Provider '{provider}' is unreachable: {what} was not found on PATH. Install it from the provider's Engine tab."
    ))
}

/// Parse a comma-separated list setting, keeping only allowed values in the
/// order the user gave. Unknown values are an error.
pub(crate) fn parse_list(
    config: &HashMap<String, String>,
    key: &str,
    allowed: &[&str],
    default: &[&str],
) -> Result<Vec<String>, AppError> {
    let raw = config.get(key).map(|s| s.trim()).unwrap_or_default();
    if raw.is_empty() {
        return Ok(default.iter().map(|s| s.to_string()).collect());
    }
    let mut out: Vec<String> = Vec::new();
    for item in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if !allowed.contains(&item) {
            return Err(AppError::Config(format!(
                "Unknown value '{item}' for {key}; expected one of: {}",
                allowed.join(", ")
            )));
        }
        if !out.iter().any(|x| x == item) {
            out.push(item.to_string());
        }
    }
    if out.is_empty() {
        return Err(AppError::Config(format!(
            "{key} must list at least one value"
        )));
    }
    Ok(out)
}

pub(crate) fn parse_minutes(
    config: &HashMap<String, String>,
    key: &str,
    default: u64,
) -> Result<Option<std::time::Duration>, AppError> {
    let minutes = match config.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(v) => v
            .parse::<u64>()
            .map_err(|_| AppError::Config(format!("{key} must be a whole number of minutes")))?,
        None => default,
    };
    Ok((minutes > 0).then(|| std::time::Duration::from_secs(minutes * 60)))
}

/// An HTTP client for one running System One engine, rebuilt when the engine
/// restarts on a new port or key.
#[derive(Default)]
pub(crate) struct SystemOneClientCache {
    inner: parking_lot::Mutex<HashMap<String, (u16, Arc<crate::systemone::SystemOneProvider>)>>,
}

impl SystemOneClientCache {
    pub(crate) fn get(
        &self,
        flavor: crate::systemone::SystemOneFlavor,
        handle: &lr_engines::EngineHandle,
    ) -> Result<Arc<crate::systemone::SystemOneProvider>, AppError> {
        let mut inner = self.inner.lock();
        if let Some((port, client)) = inner.get(&handle.key) {
            if *port == handle.port {
                return Ok(client.clone());
            }
        }
        let client = Arc::new(crate::systemone::SystemOneProvider::new(
            flavor,
            Some(handle.base_url()),
            Some(handle.api_key().to_string()),
        )?);
        inner.insert(handle.key.clone(), (handle.port, client.clone()));
        Ok(client)
    }
}

/// Look up an engine command off the async runtime (the first lookup may run
/// the user's login shell).
pub(crate) async fn resolve_engine(
    recipe: lr_engines::RecipeId,
    override_path: Option<std::path::PathBuf>,
) -> Option<lr_engines::EngineCommand> {
    tokio::task::spawn_blocking(move || lr_engines::resolve(recipe, override_path.as_deref()))
        .await
        .ok()
        .flatten()
}

/// The `lr-fake-engine` test binary from `lr-engines`, when it has been
/// built (it is during workspace test runs). Tests that need it skip
/// otherwise.
#[cfg(test)]
pub(crate) fn fake_engine_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.parent()?; // target/debug/deps -> target/debug
    let name = if cfg!(windows) {
        "lr-fake-engine.exe"
    } else {
        "lr-fake-engine"
    };
    let path = dir.join(name);
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(k: &str, v: &str) -> HashMap<String, String> {
        [(k.to_string(), v.to_string())].into_iter().collect()
    }

    #[test]
    fn list_parsing() {
        let allowed = ["a", "b", "c"];
        assert_eq!(
            parse_list(&HashMap::new(), "x", &allowed, &["a"]).unwrap(),
            vec!["a"]
        );
        assert_eq!(
            parse_list(&cfg("x", " c, a ,c"), "x", &allowed, &["a"]).unwrap(),
            vec!["c", "a"]
        );
        assert!(parse_list(&cfg("x", "a,z"), "x", &allowed, &["a"]).is_err());
        assert!(parse_list(&cfg("x", " , "), "x", &allowed, &["a"]).is_err());
    }

    #[test]
    fn minutes_parsing() {
        assert_eq!(
            parse_minutes(&HashMap::new(), "m", 15).unwrap(),
            Some(std::time::Duration::from_secs(900))
        );
        assert_eq!(parse_minutes(&cfg("m", "0"), "m", 15).unwrap(), None);
        assert!(parse_minutes(&cfg("m", "x"), "m", 15).is_err());
    }

    #[test]
    fn engine_errors_classify_as_unreachable() {
        let e = engine_missing("laya", "laya-serve");
        assert!(e.to_string().contains("unreachable"));
    }
}
