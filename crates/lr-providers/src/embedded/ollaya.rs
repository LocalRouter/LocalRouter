//! Ollaya Local Embedded provider: LocalRouter runs Ollaya
//! (<https://github.com/ollaya-dev/ollaya>), one server that serves many open
//! decision models (Laya, Kev, Decider, Von, Winnow, …) behind a
//! TypeSafe-compatible `/v1/systemone`.
//!
//! Models live in Ollaya's own store (`~/.ollaya/models` unless
//! `OLLAYA_MODELS` or the `models_dir` setting says otherwise), shared with
//! the Ollaya CLI and app. What is downloaded is read from the store's
//! manifests on disk, so listing models never starts the engine. Downloads
//! run through Ollaya's `/api/pull` and happen only from the Models tab.
//!
//! The Models tab lists the built-in [`LIBRARY`] plus whatever Ollaya has
//! published since, read from its repository ([`super::ollaya_registry`])
//! at most once a day. A model that needs a newer Ollaya than the installed
//! engine is listed with the reason and cannot be downloaded.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use parking_lot::Mutex;
use serde_json::{json, Value};

use lr_config::FreeTierKind;
use lr_engines::{EngineHandle, EngineState, LaunchSpec, PortArg, RecipeId, Supervisor};
use lr_types::{AppError, AppResult};

use super::ollaya_registry::{self, RegistrySource, RegistryView};
use super::{
    engine_error, engine_missing, not_downloaded, parse_minutes, resolve_engine,
    EmbeddedCatalogModel, SystemOneClientCache,
};
use crate::factory::{ParameterType, ProviderCategory, ProviderFactory, SetupParameter};
use crate::systemone::SystemOneFlavor;
use crate::{
    Capability, CompletionChunk, CompletionRequest, CompletionResponse, ModelInfo, ModelProvider,
    PricingInfo, ProviderHealth, SupportLevel, SystemOneRequest, SystemOneResponse,
};

pub const PROVIDER_TYPE: &str = "ollaya";

/// A model in Ollaya's library.
pub struct LibraryModel {
    /// Ollaya name, `model:tag`.
    pub id: &'static str,
    pub name: &'static str,
    /// Approximate download.
    pub size: &'static str,
    /// Tokens of state the model reads.
    pub context: u32,
    pub guidance: &'static str,
}

const fn m(
    id: &'static str,
    name: &'static str,
    size: &'static str,
    context: u32,
    guidance: &'static str,
) -> LibraryModel {
    LibraryModel {
        id,
        name,
        size,
        context,
        guidance,
    }
}

/// Ollaya's library at the pinned release (registry `ollaya.dev`, v0.9.0),
/// smallest and most general first. The first downloaded one answers
/// requests that name no model. Models Ollaya adds later come from its
/// repository ([`super::ollaya_registry`]); this list is what is shown
/// offline and describes the models it covers.
pub const LIBRARY: &[LibraryModel] = &[
    m(
        "laya:latest",
        "Laya (routes by language)",
        "1.5 GB",
        512,
        "Sends English to laya:en and other languages to laya:multilingual (downloads both). Small and fast; uses the Apple GPU on Apple Silicon.",
    ),
    m(
        "laya:en",
        "Laya English",
        "853 MB",
        512,
        "421M-parameter ModernBERT encoder, up to 512 tokens of state.",
    ),
    m(
        "laya:multilingual",
        "Laya Multilingual",
        "683 MB",
        1_024,
        "322M-parameter mmBERT encoder, up to 1,024 tokens of state.",
    ),
    m(
        "laya:typed-decisions",
        "Laya Typed Decisions",
        "853 MB",
        1_024,
        "Laya fine-tuned on typed decision workflows, up to 1,024 tokens of state.",
    ),
    m(
        "von:1.1",
        "Von 1.1",
        "1.6 GB",
        8_192,
        "395M-parameter ModernBERT, up to 8K tokens of state. Runs on the CPU; the Von provider serves the newer Von 1.3, on the GPU where there is one.",
    ),
    m(
        "decider:0.8b",
        "Decider 0.8B",
        "1.5 GB",
        32_768,
        "Qwen3.5 decoder, up to 32K tokens of state. On Macs Ollaya runs it on the CPU; the Decider provider runs it on the Apple GPU, much faster.",
    ),
    m(
        "decider:2b",
        "Decider 2B",
        "3.8 GB",
        32_768,
        "Qwen3.5 decoder, up to 32K tokens of state. Needs about 4 GB of memory. On Macs Ollaya runs it on the CPU; the Decider provider runs it on the Apple GPU, much faster.",
    ),
    m(
        "decider:4b",
        "Decider 4B",
        "8.4 GB",
        32_768,
        "Qwen3.5 decoder, up to 32K tokens of state. Needs about 9 GB of memory. On Macs Ollaya runs it on the CPU; the Decider provider runs it on the Apple GPU, much faster.",
    ),
    m(
        "decider:2b-vision",
        "Decider 2B Vision",
        "4.5 GB",
        32_768,
        "Decider that can also read one image per request. LocalRouter's System One requests are text only, so it answers like Decider 2B here.",
    ),
    m(
        "kev:0.8b",
        "Kev 0.8B",
        "1.8 GB",
        8_192,
        "Qwen3.5 decoder, up to 8K tokens of state. On Macs Ollaya runs it on the CPU; the Kev provider runs it on the Apple GPU, much faster.",
    ),
    m(
        "kev:4b",
        "Kev 4B",
        "9.5 GB",
        8_192,
        "Qwen3.5 decoder, up to 8K tokens of state. Needs about 10 GB of memory. On Macs Ollaya runs it on the CPU; the Kev provider runs it on the Apple GPU, much faster.",
    ),
    m(
        "kev:9b",
        "Kev 9B",
        "19.5 GB",
        8_192,
        "Qwen3.5 decoder, up to 8K tokens of state. Needs about 20 GB of memory. On Macs Ollaya runs it on the CPU; the Kev provider runs it on the Apple GPU, much faster.",
    ),
    m(
        "decision:eos",
        "Decision EOS",
        "1.5 GB",
        16_384,
        "0.75B decoder for English and Chinese, up to 16K tokens of state.",
    ),
    m(
        "jevk5:4b",
        "JEVK5 4B",
        "4.5 GB",
        16_384,
        "4B GGUF model (llama.cpp), up to 16 options per question and 16K tokens of state.",
    ),
    m(
        "jeb:4b",
        "Jeb 4B",
        "4.6 GB",
        4_096,
        "Jebadiah 4B (Qwen3.5, AINode), GGUF on llama.cpp with the authors' temperatures, up to 4K tokens of state.",
    ),
    m(
        "jeb:9b",
        "Jeb 9B",
        "9.8 GB",
        4_096,
        "Jebadiah 9B (Qwen3.5, AINode), GGUF on llama.cpp: the authors' recommended local model. Up to 4K tokens of state.",
    ),
    m(
        "jeb:27b",
        "Jeb 27B",
        "16.8 GB",
        4_096,
        "Jebadiah 27B (Qwen3.8, AINode), Q4_K_M GGUF that fits a 24 GB GPU: the most accurate Jebadiah. Up to 4K tokens of state.",
    ),
    m(
        "winnow:e4b",
        "Winnow E4B",
        "8.0 GB",
        8_192,
        "Multilingual Gemma fine-tune (GGUF, llama.cpp), up to 8K tokens of state.",
    ),
    m(
        "winnow:12b",
        "Winnow 12B",
        "12.7 GB",
        8_192,
        "Multilingual Gemma fine-tune (GGUF, llama.cpp), up to 8K tokens of state.",
    ),
    m(
        "cygnet:12b",
        "Cygnet 12B",
        "12.7 GB",
        16_384,
        "Gemma 4 12B IT (GGUF, llama.cpp) with Cygnet's prompt and calibration (blockbrain-ai). Multilingual, up to 20 options and 16K tokens of state.",
    ),
    m(
        "nimble:9b",
        "Nimble 9B",
        "19.5 GB",
        8_192,
        "Bespoke Labs' Nimble v2 (Qwen3.5-9B LoRA), calibrated, up to 255 options and 8K tokens of state. Needs about 18 GB of memory; best on a 24 GB GPU.",
    ),
    m(
        "jeeves:9b",
        "Jeeves 9B",
        "17.9 GB",
        8_192,
        "PostHog's Jeeves-9B (Qwen3.5-9B with a pointer head), calibrated, up to 8K tokens of state. Needs about 18 GB of memory.",
    ),
    m(
        "clef:flash",
        "Clef Flash",
        "19.1 GB",
        4_096,
        "Cloudflare's Clef-Flash (Qwen3.5-9B with a joint schema head): every option of every question in one pass, up to 4K tokens of state. Needs about 19 GB of memory.",
    ),
    m(
        "nli:modernbert-large",
        "NLI ModernBERT-large",
        "799 MB",
        512,
        "Natural-language inference (does the state support a statement?), up to 512 tokens.",
    ),
    m(
        "nli:deberta-v3-large",
        "NLI DeBERTa-v3-large",
        "884 MB",
        512,
        "Natural-language inference, up to 512 tokens. MIT license.",
    ),
    m(
        "gliclass:large",
        "GLiClass Large",
        "1.8 GB",
        1_024,
        "Zero-shot classification, up to 1,024 tokens of state.",
    ),
    m(
        "qwen3guard:0.6b",
        "Qwen3Guard 0.6B",
        "1.5 GB",
        32_768,
        "Multilingual safety moderation with built-in questions, up to 32K tokens of state.",
    ),
    m(
        "clm:8b",
        "CLM 8B",
        "16.5 GB",
        2_048,
        "8B decoder, up to 2K tokens of state. Needs about 17 GB of memory.",
    ),
];

/// Ollaya's registry, whose library models are listed without the host.
const REGISTRY: &str = "ollaya.dev";

/// Starting the server is quick; models load on first use.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the cached list of loaded models is trusted.
const LOADED_TTL: Duration = Duration::from_millis(1500);
/// How long the library read from Ollaya's repository is trusted.
const REGISTRY_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// How often the installed engine's release is checked (a different one
/// refreshes the library at once), and how soon a failed read is retried.
const REGISTRY_CHECK: Duration = Duration::from_secs(5 * 60);

pub const DEVICES: &[&str] = &["auto", "cpu", "cuda", "metal"];

/// `name`, `name:tag`, `ns/name[:tag]` → the canonical form Ollaya lists
/// (`latest` when no tag is given; lowercase).
pub fn canonical(model: &str) -> String {
    let model = model.trim().to_lowercase();
    let last = model.rsplit('/').next().unwrap_or(&model);
    if last.contains(':') {
        model
    } else {
        format!("{model}:latest")
    }
}

fn library(id: &str) -> Option<&'static LibraryModel> {
    LIBRARY.iter().find(|m| m.id == id)
}

/// Models in Ollaya's store: `(name, bytes)` from
/// `manifests/<host>/<namespace>/<model>/<tag>`. Library models of the
/// default registry are named `model:tag`, like Ollaya lists them.
pub fn stored_models(models_dir: &Path) -> Vec<(String, u64)> {
    let root = models_dir.join("manifests");
    let mut out = Vec::new();
    let dirs = |p: &Path| -> Vec<(String, PathBuf)> {
        std::fs::read_dir(p)
            .map(|rd| {
                rd.flatten()
                    .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
                    .collect()
            })
            .unwrap_or_default()
    };
    for (host, host_dir) in dirs(&root) {
        for (ns, ns_dir) in dirs(&host_dir) {
            for (model, model_dir) in dirs(&ns_dir) {
                for (tag, file) in dirs(&model_dir) {
                    if !file.is_file() || tag.starts_with('.') {
                        continue;
                    }
                    let name = match (host.as_str(), ns.as_str()) {
                        (REGISTRY, "library") => format!("{model}:{tag}"),
                        (REGISTRY, _) => format!("{ns}/{model}:{tag}"),
                        _ => format!("{host}/{ns}/{model}:{tag}"),
                    };
                    out.push((name.to_lowercase(), manifest_size(&file)));
                }
            }
        }
    }
    out.sort();
    out
}

fn manifest_size(file: &Path) -> u64 {
    let Ok(v) = std::fs::read(file).map(|b| serde_json::from_slice::<Value>(&b)) else {
        return 0;
    };
    let Ok(v) = v else { return 0 };
    let size = |x: &Value| x.get("size").and_then(Value::as_u64).unwrap_or(0);
    v.get("config").map(size).unwrap_or(0)
        + v.get("layers")
            .and_then(Value::as_array)
            .map(|l| l.iter().map(size).sum())
            .unwrap_or(0)
}

fn human_size(bytes: u64) -> String {
    const GB: f64 = 1_000_000_000.0;
    const MB: f64 = 1_000_000.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{:.0} MB", (b / MB).max(1.0))
    }
}

/// Ollaya's default store, as the engine (which inherits the user's shell
/// environment) would resolve it.
fn default_models_dir() -> Option<PathBuf> {
    let shell = lr_utils::binary::shell_env();
    let var = |k: &str| {
        shell
            .get(k)
            .cloned()
            .or_else(|| std::env::var(k).ok())
            .filter(|v| !v.trim().is_empty())
    };
    if let Some(dir) = var("OLLAYA_MODELS") {
        return Some(dir.into());
    }
    let home = var("HOME").or_else(|| var("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".ollaya").join("models"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllayaSettings {
    pub binary_path: Option<PathBuf>,
    /// `None` lets Ollaya pick (Apple GPU, CUDA when installed, else CPU).
    pub device: Option<String>,
    pub max_loaded_models: Option<u32>,
    /// `None` uses Ollaya's own store.
    pub models_dir: Option<PathBuf>,
    pub idle_timeout: Option<Duration>,
}

impl OllayaSettings {
    pub fn from_config(config: &HashMap<String, String>) -> AppResult<Self> {
        let text = |k: &str| {
            config
                .get(k)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let device = text("device")
            .map(|s| s.to_lowercase())
            .filter(|s| s != "auto");
        if let Some(d) = &device {
            let gpu_index = d
                .strip_prefix("cuda:")
                .is_some_and(|n| n.parse::<u32>().is_ok());
            if !DEVICES.contains(&d.as_str()) && !gpu_index {
                return Err(AppError::Config(format!(
                    "device must be auto, cpu, cuda, cuda:<n> or metal (got '{d}')"
                )));
            }
        }
        let max_loaded_models = match text("max_loaded_models") {
            Some(v) => match v.parse::<u32>() {
                Ok(n) if n > 0 => Some(n),
                _ => {
                    return Err(AppError::Config(
                        "max_loaded_models must be a whole number of at least 1".into(),
                    ))
                }
            },
            None => None,
        };
        Ok(Self {
            binary_path: text("binary_path").map(PathBuf::from),
            device,
            max_loaded_models,
            models_dir: text("models_dir").map(PathBuf::from),
            idle_timeout: parse_minutes(config, "idle_unload_minutes", 15)?,
        })
    }

    /// The store the engine uses.
    pub fn resolved_models_dir(&self) -> Option<PathBuf> {
        self.models_dir.clone().or_else(default_models_dir)
    }

    /// The serving launch. The API key and `host:port` are added by the
    /// supervisor.
    pub fn launch_spec(&self, instance: &str, program: PathBuf) -> LaunchSpec {
        let mut env = Vec::new();
        // Always explicit, so the engine and the manifests we read agree.
        if let Some(dir) = self.resolved_models_dir() {
            env.push((
                "OLLAYA_MODELS".to_string(),
                dir.to_string_lossy().to_string(),
            ));
        }
        if let Some(device) = &self.device {
            env.push(("OLLAYA_DEVICE".to_string(), device.clone()));
        }
        if let Some(n) = self.max_loaded_models {
            env.push(("OLLAYA_MAX_LOADED_MODELS".to_string(), n.to_string()));
        }
        LaunchSpec {
            key: format!("{PROVIDER_TYPE}:{instance}"),
            label: "Ollaya".to_string(),
            program,
            args: vec!["serve".to_string()],
            env,
            port: PortArg::Addr {
                var: "OLLAYA_HOST".to_string(),
                host: "127.0.0.1".to_string(),
            },
            api_key_env: "OLLAYA_API_KEY".to_string(),
            // `GET /` answers 200 without the key once the server is up.
            ready_path: "/".to_string(),
            start_timeout: START_TIMEOUT,
            idle_timeout: self.idle_timeout,
        }
    }
}

/// A download in progress (or the last failed one).
#[derive(Default)]
struct Pull {
    running: bool,
    progress: Option<f64>,
    error: Option<String>,
    task: Option<tokio::task::AbortHandle>,
}

/// The library read from Ollaya's repository, refreshed in the background.
#[derive(Default)]
struct RegistryCache {
    view: Option<RegistryView>,
    fetched: Option<Instant>,
    checked: Option<Instant>,
    refreshing: bool,
}

/// Name and context window of a model the catalog knows (built in or read
/// from the repository).
struct Known {
    name: String,
    context: u32,
}

/// Models Ollaya has loaded, from `/api/ps`, refreshed in the background.
#[derive(Default)]
struct LoadedCache {
    models: HashSet<String>,
    refreshed: Option<Instant>,
    refreshing: bool,
}

pub struct OllayaEmbeddedProvider {
    instance: String,
    settings: OllayaSettings,
    supervisor: Arc<Supervisor>,
    clients: SystemOneClientCache,
    http: reqwest::Client,
    pulls: Arc<Mutex<HashMap<String, Pull>>>,
    loaded: Arc<Mutex<LoadedCache>>,
    last_handle: Mutex<Option<EngineHandle>>,
    /// Where Ollaya's library is read from; `None` keeps to the built-in
    /// list.
    registry_source: Option<RegistrySource>,
    registry: Arc<Mutex<RegistryCache>>,
    /// The engine release to assume instead of detecting it (tests).
    fixed_engine_tag: Option<String>,
    /// Extra launch environment (tests point the fake engine at our
    /// variables).
    extra_env: Vec<(String, String)>,
}

impl OllayaEmbeddedProvider {
    pub fn new(instance: String, settings: OllayaSettings, supervisor: Arc<Supervisor>) -> Self {
        Self {
            instance,
            settings,
            supervisor,
            clients: SystemOneClientCache::default(),
            // No overall timeout: pulls run for as long as the download takes.
            // GitHub's API refuses requests without a User-Agent.
            http: reqwest::Client::builder()
                .user_agent(concat!("LocalRouter/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(300))
                .build()
                .unwrap_or_default(),
            pulls: Arc::default(),
            loaded: Arc::default(),
            last_handle: Mutex::new(None),
            registry_source: Some(RegistrySource::default()),
            registry: Arc::default(),
            fixed_engine_tag: None,
            extra_env: Vec::new(),
        }
    }

    /// The library read from Ollaya's repository, if it has been read.
    fn registry_view(&self) -> Option<RegistryView> {
        self.registry.lock().view.clone()
    }

    /// Name and context of a model the catalog lists.
    fn known(&self, id: &str) -> Option<Known> {
        if let Some(m) = library(id) {
            return Some(Known {
                name: m.name.to_string(),
                context: m.context,
            });
        }
        self.registry_view()?
            .added
            .into_iter()
            .find(|m| m.id == id)
            .map(|m| Known {
                name: m.name,
                context: m.context.unwrap_or(4_096),
            })
    }

    /// Why `id` cannot be downloaded with the installed engine, if it needs
    /// a newer Ollaya.
    fn needs_newer(&self, id: &str) -> Option<String> {
        let view = self.registry_view()?;
        let tag = view.needs_newer.get(id)?;
        Some(format!(
            "Needs Ollaya {tag} or newer; the engine here is {}. Update LocalRouter, or install a newer Ollaya and choose it in the Engine tab.",
            view.engine_tag
        ))
    }

    /// Read Ollaya's library in the background when it is stale: once a
    /// day, or as soon as the installed engine's release changes (checked
    /// every few minutes). Never blocks; the catalog uses what is cached.
    fn refresh_registry(&self) {
        let Some(source) = self.registry_source.clone() else {
            return;
        };
        {
            let mut cache = self.registry.lock();
            if cache.refreshing || cache.checked.is_some_and(|t| t.elapsed() < REGISTRY_CHECK) {
                return;
            }
            cache.refreshing = true;
        }
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            self.registry.lock().refreshing = false;
            return;
        };
        let http = self.http.clone();
        let cache = self.registry.clone();
        let binary_path = self.settings.binary_path.clone();
        let fixed_tag = self.fixed_engine_tag.clone();
        rt.spawn(async move {
            let engine_tag = match fixed_tag {
                Some(tag) => tag,
                None => engine_release(binary_path).await,
            };
            let stale = {
                let c = cache.lock();
                match &c.view {
                    Some(view) => {
                        view.engine_tag != engine_tag
                            || c.fetched.is_none_or(|t| t.elapsed() >= REGISTRY_TTL)
                    }
                    None => true,
                }
            };
            let result = if stale {
                Some(
                    ollaya_registry::fetch(&http, &source, &engine_tag, &|id| {
                        library(id).is_some()
                    })
                    .await,
                )
            } else {
                None
            };
            let mut c = cache.lock();
            match result {
                Some(Ok(view)) => {
                    c.view = Some(view);
                    c.fetched = Some(Instant::now());
                }
                Some(Err(e)) => {
                    tracing::debug!("Ollaya: could not read the model library: {e}");
                }
                None => {}
            }
            c.checked = Some(Instant::now());
            c.refreshing = false;
        });
    }

    fn spec_key(&self) -> String {
        format!("{PROVIDER_TYPE}:{}", self.instance)
    }

    /// Downloaded models, `(name, bytes)`.
    fn stored(&self) -> Vec<(String, u64)> {
        self.settings
            .resolved_models_dir()
            .map(|d| stored_models(&d))
            .unwrap_or_default()
    }

    fn is_downloaded(&self, model: &str) -> bool {
        self.stored().iter().any(|(n, _)| n == model)
    }

    /// The model a request that names none uses: the first downloaded
    /// library model, else any downloaded model.
    fn default_model(&self) -> Option<String> {
        let stored = self.stored();
        LIBRARY
            .iter()
            .find(|m| stored.iter().any(|(n, _)| n == m.id))
            .map(|m| m.id.to_string())
            .or_else(|| stored.first().map(|(n, _)| n.clone()))
    }

    /// The canonical model to serve: `model` must be downloaded; without a
    /// model, something must be.
    fn servable(&self, model: Option<&str>) -> AppResult<String> {
        match model {
            Some(m) => {
                let id = canonical(m);
                if self.is_downloaded(&id) {
                    Ok(id)
                } else if self.known(&id).is_some() {
                    Err(not_downloaded(PROVIDER_TYPE, &id))
                } else {
                    Err(AppError::ModelNotFound {
                        model: m.to_string(),
                    })
                }
            }
            None => self
                .default_model()
                .ok_or_else(|| not_downloaded(PROVIDER_TYPE, LIBRARY[0].id)),
        }
    }

    async fn program(&self) -> AppResult<PathBuf> {
        resolve_engine(RecipeId::Ollaya, self.settings.binary_path.clone())
            .await
            .map(|c| c.program)
            .ok_or_else(|| engine_missing(PROVIDER_TYPE, "Ollaya"))
    }

    async fn ensure_engine(&self) -> AppResult<EngineHandle> {
        let program = self.program().await?;
        let mut spec = self.settings.launch_spec(&self.instance, program);
        spec.env.extend(self.extra_env.iter().cloned());
        let handle = self
            .supervisor
            .ensure(spec)
            .await
            .map_err(|e| engine_error(PROVIDER_TYPE, e))?;
        *self.last_handle.lock() = Some(handle.clone());
        Ok(handle)
    }

    /// The running engine's handle, without starting it.
    fn running_handle(&self) -> Option<EngineHandle> {
        let handle = self.last_handle.lock().clone()?;
        self.supervisor
            .processes()
            .into_iter()
            .any(|p| {
                p.key == handle.key
                    && p.state == EngineState::Running
                    && p.port == Some(handle.port)
            })
            .then_some(handle)
    }

    /// Ollaya's error message from a failed response body.
    async fn error_of(resp: reqwest::Response) -> String {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| format!("HTTP {status}: {body}"))
    }

    /// Load (`keep_alive` -1: until unloaded) or unload (0) a model.
    async fn set_loaded(&self, handle: &EngineHandle, model: &str, load: bool) -> AppResult<()> {
        let resp = self
            .http
            .post(format!("{}/api/decide", handle.base_url()))
            .bearer_auth(handle.api_key())
            .json(&json!({"model": model, "keep_alive": if load { -1 } else { 0 }}))
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Ollaya: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::Provider(format!(
                "Ollaya could not {} {model}: {}",
                if load { "load" } else { "unload" },
                Self::error_of(resp).await
            )));
        }
        let mut loaded = self.loaded.lock();
        if load {
            loaded.models.insert(model.to_string());
        } else {
            loaded.models.remove(model);
        }
        Ok(())
    }

    /// Refresh the loaded-models cache from `/api/ps` in the background when
    /// it is stale.
    fn refresh_loaded(&self, handle: EngineHandle) {
        {
            let mut cache = self.loaded.lock();
            if cache.refreshing || cache.refreshed.is_some_and(|t| t.elapsed() < LOADED_TTL) {
                return;
            }
            cache.refreshing = true;
        }
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            self.loaded.lock().refreshing = false;
            return;
        };
        let http = self.http.clone();
        let cache = self.loaded.clone();
        rt.spawn(async move {
            let names = async {
                let v: Value = http
                    .get(format!("{}/api/ps", handle.base_url()))
                    .bearer_auth(handle.api_key())
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await
                    .ok()?
                    .error_for_status()
                    .ok()?
                    .json()
                    .await
                    .ok()?;
                Some(
                    v.get("models")?
                        .as_array()?
                        .iter()
                        .filter_map(|m| m.get("name").and_then(Value::as_str))
                        .map(canonical)
                        .collect::<HashSet<_>>(),
                )
            }
            .await;
            let mut c = cache.lock();
            if let Some(names) = names {
                c.models = names;
            }
            c.refreshed = Some(Instant::now());
            c.refreshing = false;
        });
    }
}

/// The release of the Ollaya LocalRouter would run, e.g. `v0.9.0`: the
/// downloaded engine's tag, else the version the binary reports, else the
/// pinned release (what a download would install).
async fn engine_release(binary_path: Option<PathBuf>) -> String {
    let status = lr_engines::detect(RecipeId::Ollaya, binary_path, false).await;
    let version = status
        .found
        .then(|| status.managed_tag.or(status.version))
        .flatten();
    match version {
        Some(v) if v.starts_with('v') => v,
        Some(v) => format!("v{v}"),
        None => lr_engines::OLLAYA_VERSION.to_string(),
    }
}

/// Run one `/api/pull` to completion, reporting progress into `pulls`.
async fn pull(
    http: reqwest::Client,
    handle: EngineHandle,
    model: String,
    pulls: Arc<Mutex<HashMap<String, Pull>>>,
) -> Result<(), String> {
    let _lease = handle.lease();
    let resp = http
        .post(format!("{}/api/pull", handle.base_url()))
        .bearer_auth(handle.api_key())
        .json(&json!({"model": model, "stream": true}))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(OllayaEmbeddedProvider::error_of(resp).await);
    }
    // digest → (total, completed)
    let mut blobs: HashMap<String, (u64, u64)> = HashMap::new();
    let mut buf: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    let mut succeeded = false;
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk.map_err(|e| format!("the download was interrupted: {e}"))?);
        while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = buf.drain(..=pos).collect();
            let Ok(v) = serde_json::from_slice::<Value>(&line) else {
                continue;
            };
            if let Some(err) = v.get("error").and_then(Value::as_str) {
                return Err(err.to_string());
            }
            if v.get("status").and_then(Value::as_str) == Some("success") {
                succeeded = true;
            }
            if let (Some(digest), Some(total)) = (
                v.get("digest").and_then(Value::as_str),
                v.get("total").and_then(Value::as_u64),
            ) {
                let completed = v.get("completed").and_then(Value::as_u64).unwrap_or(0);
                blobs.insert(digest.to_string(), (total, completed.min(total)));
                let (total, done) = blobs
                    .values()
                    .fold((0u64, 0u64), |(t, d), (bt, bd)| (t + bt, d + bd));
                if total > 0 {
                    if let Some(p) = pulls.lock().get_mut(&model) {
                        p.progress = Some(done as f64 / total as f64);
                    }
                }
            }
        }
    }
    if succeeded {
        Ok(())
    } else {
        Err("the download ended before it finished".to_string())
    }
}

#[async_trait]
impl super::EmbeddedControl for OllayaEmbeddedProvider {
    async fn load(&self, model: &str) -> AppResult<()> {
        let id = self.servable(Some(model))?;
        let handle = self.ensure_engine().await?;
        let _lease = handle.lease();
        self.set_loaded(&handle, &id, true).await
    }

    async fn unload(&self, model: &str) -> AppResult<()> {
        let Some(handle) = self.running_handle() else {
            return Ok(());
        };
        self.set_loaded(&handle, &canonical(model), false).await
    }

    fn model_states(&self) -> Vec<super::EmbeddedModelState> {
        let key = self.spec_key();
        let Some(p) = self
            .supervisor
            .processes()
            .into_iter()
            .find(|p| p.key == key)
        else {
            return Vec::new();
        };
        let state = |model: String| super::EmbeddedModelState {
            model,
            state: p.state,
            port: p.port,
            idle_secs: p.idle_secs,
            last_error: p.last_error.clone(),
        };
        if p.state != EngineState::Running {
            // The engine stopped or failed: every model shares its fate.
            return self.stored().into_iter().map(|(n, _)| state(n)).collect();
        }
        if let Some(handle) = self.running_handle() {
            self.refresh_loaded(handle);
        }
        let mut loaded: Vec<String> = self.loaded.lock().models.iter().cloned().collect();
        loaded.sort();
        loaded.into_iter().map(state).collect()
    }

    fn catalog(&self) -> Vec<EmbeddedCatalogModel> {
        self.refresh_registry();
        let view = self.registry_view();
        let stored = self.stored();
        let pulls = self.pulls.lock();
        let entry = |id: &str, name: String, size: String, guidance: Option<String>| {
            let downloaded = stored.iter().any(|(n, _)| n == id);
            let pull = pulls.get(id);
            EmbeddedCatalogModel {
                id: id.to_string(),
                name,
                download_size: size,
                guidance,
                downloaded,
                downloading: pull.is_some_and(|p| p.running),
                download_error: pull.and_then(|p| p.error.clone()),
                progress: pull.filter(|p| p.running).and_then(|p| p.progress),
                removable: downloaded,
                // A model already in the store can still be removed.
                unavailable: (!downloaded).then(|| self.needs_newer(id)).flatten(),
            }
        };
        let mut out: Vec<EmbeddedCatalogModel> = LIBRARY
            .iter()
            .map(|m| {
                entry(
                    m.id,
                    m.name.to_string(),
                    m.size.to_string(),
                    Some(m.guidance.to_string()),
                )
            })
            .collect();
        // Models Ollaya published after the built-in list.
        let added = view.map(|v| v.added).unwrap_or_default();
        for m in &added {
            if library(&m.id).is_none() {
                out.push(entry(
                    &m.id,
                    m.name.clone(),
                    human_size(m.size_bytes),
                    m.description.clone(),
                ));
            }
        }
        // Models pulled another way (the Ollaya CLI, `ollaya create`, a
        // router's targets) are listed too.
        for (name, bytes) in &stored {
            if library(name).is_none() && !added.iter().any(|m| &m.id == name) {
                out.push(entry(name, name.clone(), human_size(*bytes), None));
            }
        }
        out
    }

    async fn download(&self, model: &str) -> AppResult<()> {
        let id = canonical(model);
        if let Some(reason) = self.needs_newer(&id) {
            return Err(AppError::InvalidParams(format!("{id}: {reason}")));
        }
        if self.pulls.lock().get(&id).is_some_and(|p| p.running) {
            return Ok(());
        }
        let handle = self.ensure_engine().await?;
        let mut pulls = self.pulls.lock();
        if pulls.get(&id).is_some_and(|p| p.running) {
            return Ok(());
        }
        let task = {
            let (http, pulls_ref, id) = (self.http.clone(), self.pulls.clone(), id.clone());
            tokio::spawn(async move {
                let result = pull(http, handle, id.clone(), pulls_ref.clone()).await;
                let mut pulls = pulls_ref.lock();
                match result {
                    Ok(()) => {
                        pulls.remove(&id);
                        drop(pulls);
                        super::notify_models_changed(PROVIDER_TYPE);
                    }
                    Err(e) => {
                        let p = pulls.entry(id).or_default();
                        p.running = false;
                        p.progress = None;
                        p.error = Some(e);
                    }
                }
            })
        };
        pulls.insert(
            id,
            Pull {
                running: true,
                progress: Some(0.0),
                error: None,
                task: Some(task.abort_handle()),
            },
        );
        Ok(())
    }

    async fn cancel_download(&self, model: &str) -> AppResult<()> {
        if let Some(p) = self.pulls.lock().remove(&canonical(model)) {
            if let Some(task) = p.task {
                task.abort();
            }
        }
        Ok(())
    }

    async fn remove_download(&self, model: &str) -> AppResult<()> {
        let id = canonical(model);
        if !self.is_downloaded(&id) {
            return Ok(());
        }
        let handle = self.ensure_engine().await?;
        let _lease = handle.lease();
        let resp = self
            .http
            .delete(format!("{}/api/delete", handle.base_url()))
            .bearer_auth(handle.api_key())
            .json(&json!({ "model": id }))
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Ollaya: {e}")))?;
        // 404: already gone.
        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NOT_FOUND {
            return Err(AppError::Provider(format!(
                "Ollaya could not remove {id}: {}",
                Self::error_of(resp).await
            )));
        }
        self.loaded.lock().models.remove(&id);
        self.pulls.lock().remove(&id);
        super::notify_models_changed(PROVIDER_TYPE);
        Ok(())
    }
}

impl Drop for OllayaEmbeddedProvider {
    fn drop(&mut self) {
        for p in self.pulls.lock().values() {
            if let Some(task) = &p.task {
                task.abort();
            }
        }
        // The provider was removed or reconfigured: stop its engine.
        let supervisor = self.supervisor.clone();
        let key = self.spec_key();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move { supervisor.stop(&key).await });
        }
    }
}

#[async_trait]
impl ModelProvider for OllayaEmbeddedProvider {
    fn upstream_api(&self) -> lr_types::LlmApi {
        lr_types::LlmApi::SystemOne
    }

    fn name(&self) -> &str {
        PROVIDER_TYPE
    }

    async fn health_check(&self) -> ProviderHealth {
        // Never starts the engine.
        let found = self.program().await.is_ok();
        super::engine_health(
            (!found).then(|| {
                "Ollaya is not installed. Download it in the provider's Engine tab.".to_string()
            }),
            !self.stored().is_empty(),
            "No model is downloaded yet. Download one in the Models tab.",
            &self.supervisor,
            &self.spec_key(),
        )
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        let stored = self.stored();
        // Library order first, then anything else in the store.
        let mut ids: Vec<String> = LIBRARY
            .iter()
            .filter(|m| stored.iter().any(|(n, _)| n == m.id))
            .map(|m| m.id.to_string())
            .collect();
        ids.extend(
            stored
                .into_iter()
                .map(|(n, _)| n)
                .filter(|n| library(n).is_none()),
        );
        Ok(ids
            .into_iter()
            .map(|id| {
                let known = self.known(&id);
                ModelInfo {
                    name: known
                        .as_ref()
                        .map(|k| k.name.clone())
                        .unwrap_or_else(|| id.clone()),
                    context_window: known.map(|k| k.context).unwrap_or(4_096),
                    id,
                    provider: PROVIDER_TYPE.to_string(),
                    parameter_count: None,
                    supports_streaming: false,
                    capabilities: vec![Capability::Decision],
                    detailed_capabilities: None,
                }
            })
            .collect())
    }

    async fn get_pricing(&self, _model: &str) -> AppResult<PricingInfo> {
        Ok(PricingInfo::free())
    }

    async fn complete(&self, _request: CompletionRequest) -> AppResult<CompletionResponse> {
        Err(AppError::Provider(format!(
            "Provider '{}' does not support chat completions",
            self.name()
        )))
    }

    async fn stream_complete(
        &self,
        _request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        Err(AppError::Provider(format!(
            "Provider '{}' does not support chat completions",
            self.name()
        )))
    }

    fn supports_chat(&self) -> bool {
        false
    }

    fn supports_systemone(&self) -> bool {
        true
    }

    fn api_path_support(&self, _path: &str) -> SupportLevel {
        SupportLevel::NotSupported
    }

    fn embedded_control(&self) -> Option<&dyn super::EmbeddedControl> {
        Some(self)
    }

    async fn systemone(&self, mut request: SystemOneRequest) -> AppResult<SystemOneResponse> {
        let model = self.servable(request.model.as_deref())?;
        let handle = self.ensure_engine().await?;
        let _lease = handle.lease();
        let client = self.clients.get(SystemOneFlavor::Generic, &handle)?;
        request.model = Some(model);
        client.systemone(request).await
    }
}

/// Factory for the Ollaya Local Embedded provider.
pub struct OllayaEmbeddedProviderFactory {
    supervisor: Arc<Supervisor>,
}

impl OllayaEmbeddedProviderFactory {
    pub fn new(supervisor: Arc<Supervisor>) -> Self {
        Self { supervisor }
    }
}

impl ProviderFactory for OllayaEmbeddedProviderFactory {
    fn provider_type(&self) -> &str {
        PROVIDER_TYPE
    }

    fn display_name(&self) -> &str {
        "Ollaya"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::Embedded
    }

    fn description(&self) -> &str {
        "Open System One decision models (typed choice, score and yes/no answers): Laya, Kev, Decider, Von, Winnow and more. LocalRouter downloads and runs the Ollaya engine; download models in the Models tab"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::AlwaysFreeLocal
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::optional(
                "device",
                ParameterType::String,
                "auto, cpu, cuda, cuda:<n> or metal",
                Some("auto"),
                false,
            ),
            SetupParameter::optional(
                "max_loaded_models",
                ParameterType::Number,
                "Models kept in memory at once (Ollaya's default: 3)",
                None::<String>,
                false,
            ),
            SetupParameter::optional(
                "idle_unload_minutes",
                ParameterType::Number,
                "Stop the engine after this many idle minutes (0 = keep running)",
                Some("15"),
                false,
            ),
            SetupParameter::optional(
                "models_dir",
                ParameterType::String,
                "Where models are stored (leave empty for Ollaya's own ~/.ollaya/models)",
                None::<String>,
                false,
            ),
            SetupParameter::optional(
                "binary_path",
                ParameterType::String,
                "Path to ollaya (leave empty to use the downloaded engine or find it on PATH)",
                None::<String>,
                false,
            ),
        ]
    }

    fn create(
        &self,
        instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        let settings = OllayaSettings::from_config(&config)?;
        Ok(Arc::new(OllayaEmbeddedProvider::new(
            instance_name,
            settings,
            self.supervisor.clone(),
        )))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        OllayaSettings::from_config(config).map(|_| ())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None
    }

    fn model_list_source(&self) -> crate::factory::ModelListSource {
        crate::factory::ModelListSource::ApiOnly
    }

    fn docs_url(&self) -> Option<&str> {
        Some("https://github.com/ollaya-dev/ollaya")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedded::EmbeddedControl;

    fn cfg(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn write_manifest(dir: &Path, host: &str, ns: &str, model: &str, tag: &str) {
        let d = dir.join("manifests").join(host).join(ns).join(model);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join(tag),
            r#"{"config":{"size":10},"layers":[{"size":1000000},{"size":990}]}"#,
        )
        .unwrap();
    }

    fn req(model: Option<&str>) -> SystemOneRequest {
        let mut v = serde_json::json!({
            "state": "s",
            "questions": {"q": {"type": "noul", "instructions": "?"}}
        });
        if let Some(m) = model {
            v["model"] = m.into();
        }
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn canonical_names() {
        assert_eq!(canonical("laya"), "laya:latest");
        assert_eq!(canonical(" Kev:0.8B "), "kev:0.8b");
        assert_eq!(canonical("acme/triage"), "acme/triage:latest");
        assert_eq!(
            canonical("host:5000/acme/triage:v1"),
            "host:5000/acme/triage:v1"
        );
        assert!(LIBRARY.iter().all(|m| canonical(m.id) == m.id));
    }

    #[test]
    fn settings_validation_and_launch_spec() {
        assert!(OllayaSettings::from_config(&cfg(&[("device", "tpu")])).is_err());
        assert!(OllayaSettings::from_config(&cfg(&[("device", "cuda:x")])).is_err());
        assert!(OllayaSettings::from_config(&cfg(&[("max_loaded_models", "0")])).is_err());
        let auto = OllayaSettings::from_config(&cfg(&[("device", "Auto")])).unwrap();
        assert_eq!(auto.device, None);

        let s = OllayaSettings::from_config(&cfg(&[
            ("device", "cuda:1"),
            ("max_loaded_models", "2"),
            ("models_dir", "/data/ollaya"),
            ("idle_unload_minutes", "0"),
        ]))
        .unwrap();
        let spec = s.launch_spec("Ollaya", PathBuf::from("/opt/ollaya/bin/ollaya"));
        assert_eq!(spec.key, "ollaya:Ollaya");
        assert_eq!(spec.args, vec!["serve"]);
        assert_eq!(
            spec.port,
            PortArg::Addr {
                var: "OLLAYA_HOST".into(),
                host: "127.0.0.1".into()
            }
        );
        assert_eq!(spec.api_key_env, "OLLAYA_API_KEY");
        assert_eq!(spec.ready_path, "/");
        assert_eq!(spec.idle_timeout, None);
        let env: HashMap<_, _> = spec.env.into_iter().collect();
        assert_eq!(env["OLLAYA_MODELS"], "/data/ollaya");
        assert_eq!(env["OLLAYA_DEVICE"], "cuda:1");
        assert_eq!(env["OLLAYA_MAX_LOADED_MODELS"], "2");
        // The key and address come from the supervisor, never from us.
        assert!(!env.contains_key("OLLAYA_API_KEY") && !env.contains_key("OLLAYA_HOST"));
    }

    #[test]
    fn stored_models_are_named_like_ollaya_lists_them() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(dir.path(), "ollaya.dev", "library", "laya", "en");
        write_manifest(dir.path(), "ollaya.dev", "acme", "triage", "v1");
        write_manifest(dir.path(), "registry.example.com", "team", "gate", "latest");
        let stored = stored_models(dir.path());
        assert_eq!(
            stored,
            vec![
                ("acme/triage:v1".to_string(), 1_001_000),
                ("laya:en".to_string(), 1_001_000),
                (
                    "registry.example.com/team/gate:latest".to_string(),
                    1_001_000
                ),
            ]
        );
        assert!(stored_models(&dir.path().join("missing")).is_empty());
    }

    #[tokio::test]
    async fn serves_only_downloaded_models_and_picks_a_default() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("models");
        let settings =
            OllayaSettings::from_config(&cfg(&[("models_dir", store.to_str().unwrap())])).unwrap();
        let mut p =
            OllayaEmbeddedProvider::new("Ollaya".into(), settings, Supervisor::new(dir.path()));
        p.registry_source = None;
        assert!(p.list_models().await.unwrap().is_empty());
        assert!(matches!(
            p.systemone(req(None)).await,
            Err(AppError::InvalidParams(m)) if m.contains("not downloaded")
        ));
        assert!(matches!(
            p.systemone(req(Some("kev:4b"))).await,
            Err(AppError::InvalidParams(m)) if m.contains("not downloaded")
        ));
        assert!(matches!(
            p.systemone(req(Some("klingon:1b"))).await,
            Err(AppError::ModelNotFound { .. })
        ));

        write_manifest(&store, "ollaya.dev", "library", "kev", "0.8b");
        write_manifest(&store, "ollaya.dev", "library", "laya", "en");
        write_manifest(&store, "ollaya.dev", "acme", "triage", "v1");
        // Library order, then the rest.
        let ids: Vec<String> = p
            .list_models()
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, vec!["laya:en", "kev:0.8b", "acme/triage:v1"]);
        let models = p.list_models().await.unwrap();
        assert!(models
            .iter()
            .all(|m| m.capabilities == vec![Capability::Decision]));
        assert_eq!(models[0].context_window, 512);
        assert_eq!(p.servable(None).unwrap(), "laya:en");
        assert_eq!(p.servable(Some("KEV:0.8b")).unwrap(), "kev:0.8b");

        let catalog = p.catalog();
        assert_eq!(catalog.len(), LIBRARY.len() + 1);
        let laya = catalog.iter().find(|m| m.id == "laya:en").unwrap();
        assert!(laya.downloaded && laya.removable);
        let custom = catalog.last().unwrap();
        assert_eq!(custom.id, "acme/triage:v1");
        assert_eq!(custom.download_size, "1 MB");
        assert!(!p.supports_chat());
        assert!(p.supports_systemone());
    }

    #[tokio::test]
    async fn the_catalog_adds_published_models_and_says_which_need_a_newer_engine() {
        let server = crate::embedded::ollaya_registry::tests::mock_registry().await;
        let dir = tempfile::tempdir().unwrap();
        let settings = OllayaSettings::from_config(&cfg(&[(
            "models_dir",
            dir.path().join("models").to_str().unwrap(),
        )]))
        .unwrap();
        let mut p =
            OllayaEmbeddedProvider::new("Ollaya".into(), settings, Supervisor::new(dir.path()));
        p.registry_source = Some(crate::embedded::ollaya_registry::tests::source(&server));
        p.fixed_engine_tag = Some("v0.9.0".into());

        // The first look shows the built-in list and starts the read.
        assert_eq!(p.catalog().len(), LIBRARY.len());
        wait_for(|| p.registry_view().is_some()).await;
        let catalog = p.catalog();
        assert_eq!(catalog.len(), LIBRARY.len() + 2);
        let acme = catalog.iter().find(|m| m.id == "acme:2b").unwrap();
        assert_eq!(acme.name, "Acme 2B");
        assert_eq!(acme.download_size, "2.0 GB");
        assert_eq!(acme.guidance.as_deref(), Some("The acme model."));
        assert!(acme.unavailable.is_none());
        let zeta = catalog.iter().find(|m| m.id == "zeta:1b").unwrap();
        assert!(zeta
            .unavailable
            .as_deref()
            .unwrap()
            .contains("Needs Ollaya v0.10.0 or newer; the engine here is v0.9.0"));
        assert!(catalog
            .iter()
            .filter(|m| library(&m.id).is_some())
            .all(|m| m.unavailable.is_none()));

        // A published model is known: asking for it says to download it.
        assert!(matches!(
            p.servable(Some("acme:2b")),
            Err(AppError::InvalidParams(m)) if m.contains("not downloaded")
        ));
        // One that needs a newer engine is refused before anything starts.
        assert!(matches!(
            p.download("zeta:1b").await,
            Err(AppError::InvalidParams(m)) if m.contains("v0.10.0")
        ));
        // Downloaded, it is listed with the registry's name and context.
        write_manifest(
            &dir.path().join("models"),
            "ollaya.dev",
            "library",
            "acme",
            "2b",
        );
        let models = p.list_models().await.unwrap();
        assert_eq!(models[0].id, "acme:2b");
        assert_eq!(models[0].name, "Acme 2B");
        assert_eq!(models[0].context_window, 2048);
    }

    #[test]
    fn factory_is_embedded_listed_and_asks_for_nothing_required() {
        let dir = tempfile::tempdir().unwrap();
        let f = OllayaEmbeddedProviderFactory::new(Supervisor::new(dir.path()));
        assert_eq!(f.category(), ProviderCategory::Embedded);
        assert!(f.listed());
        assert!(f.setup_parameters().iter().all(|p| !p.required));
        assert!(f.validate_config(&HashMap::new()).is_ok());
    }

    async fn wait_for(mut done: impl FnMut() -> bool) {
        for _ in 0..200 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("timed out");
    }

    #[tokio::test]
    async fn downloads_serves_loads_and_removes_through_the_engine() {
        let Some(fake) = crate::embedded::fake_engine_path() else {
            eprintln!("skipping: lr-fake-engine not built (run the workspace tests)");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("models");
        let supervisor = Supervisor::new(dir.path());
        let settings = OllayaSettings::from_config(&cfg(&[
            ("binary_path", fake.to_str().unwrap()),
            ("models_dir", store.to_str().unwrap()),
        ]))
        .unwrap();
        let mut p = OllayaEmbeddedProvider::new("Ollaya".into(), settings, supervisor.clone());
        p.registry_source = None;
        p.extra_env = vec![
            ("FAKE_ADDR_VAR".into(), "OLLAYA_HOST".into()),
            ("FAKE_KEY_VAR".into(), "OLLAYA_API_KEY".into()),
        ];

        // A failed pull is reported on the model.
        p.download("missing:1b").await.unwrap();
        wait_for(|| {
            p.pulls
                .lock()
                .get("missing:1b")
                .is_some_and(|x| !x.running && x.error.is_some())
        })
        .await;
        assert!(p.pulls.lock()["missing:1b"]
            .error
            .as_deref()
            .unwrap()
            .contains("not found in registry"));

        p.download("laya:en").await.unwrap();
        assert!(p
            .catalog()
            .iter()
            .any(|m| m.id == "laya:en" && m.downloading));
        wait_for(|| {
            p.catalog()
                .iter()
                .any(|m| m.id == "laya:en" && m.downloaded && !m.downloading)
        })
        .await;
        assert!(supervisor.is_running("ollaya:Ollaya"));

        // Requests name the model to Ollaya; no model means the default.
        let resp = p.systemone(req(None)).await.unwrap();
        assert!(resp.answers.contains_key("q"));
        assert_eq!(resp.model, "laya:en");

        p.load("laya:en").await.unwrap();
        wait_for(|| p.model_states().iter().any(|s| s.model == "laya:en")).await;
        p.unload("laya:en").await.unwrap();
        assert!(!p.loaded.lock().models.contains("laya:en"));

        p.remove_download("laya:en").await.unwrap();
        assert!(!p.is_downloaded("laya:en"));
        assert!(p.list_models().await.unwrap().is_empty());

        drop(p);
        wait_for(|| !supervisor.is_running("ollaya:Ollaya")).await;
    }

    /// Against a real Ollaya: `OLLAYA_E2E_BINARY=/path/to/bin/ollaya
    /// OLLAYA_E2E_MODELS=/path/to/store cargo test -p lr-providers --
    /// --ignored real_ollaya`. Pulls `nli:modernbert-large` (about 800 MB)
    /// unless the store already has it.
    #[tokio::test]
    #[ignore = "needs a real Ollaya binary and network"]
    async fn real_ollaya_pulls_and_decides() {
        let (Ok(binary), Ok(models)) = (
            std::env::var("OLLAYA_E2E_BINARY"),
            std::env::var("OLLAYA_E2E_MODELS"),
        ) else {
            eprintln!("skipping: set OLLAYA_E2E_BINARY and OLLAYA_E2E_MODELS");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let supervisor = Supervisor::new(dir.path());
        let settings =
            OllayaSettings::from_config(&cfg(&[("binary_path", &binary), ("models_dir", &models)]))
                .unwrap();
        let p = OllayaEmbeddedProvider::new("Ollaya".into(), settings, supervisor.clone());
        let model = "nli:modernbert-large";
        if !p.is_downloaded(model) {
            p.download(model).await.unwrap();
            let mut saw_progress = false;
            for _ in 0..3_600 {
                let entry = p.catalog().into_iter().find(|m| m.id == model).unwrap();
                saw_progress |= entry.progress.is_some_and(|x| x > 0.0 && x < 1.0);
                if entry.downloaded && !entry.downloading {
                    break;
                }
                assert!(entry.download_error.is_none(), "{:?}", entry.download_error);
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            assert!(p.is_downloaded(model));
            assert!(saw_progress, "no download progress was reported");
        }
        let mut request = req(Some(model));
        request.state = serde_json::json!("The invoice was paid twice.");
        let resp = p.systemone(request).await.unwrap();
        let answer = serde_json::to_value(&resp.answers["q"]).unwrap();
        let yes = answer["noul"].as_f64().unwrap_or(-1.0);
        assert!((0.0..=1.0).contains(&yes), "{answer}");
        p.load(model).await.unwrap();
        wait_for(|| p.model_states().iter().any(|s| s.model == model)).await;
        p.unload(model).await.unwrap();
        drop(p);
        wait_for(|| !supervisor.is_running("ollaya:Ollaya")).await;
    }
}
