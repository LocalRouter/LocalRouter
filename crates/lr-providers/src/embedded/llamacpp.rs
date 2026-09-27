//! llama.cpp Local Embedded provider: LocalRouter runs `llama-server` (installed with
//! the user's package manager) for models in the in-app library, one process
//! per loaded model, started on the first request and stopped when idle.

use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::{Stream, StreamExt};

use lr_config::FreeTierKind;
use lr_engines::{EngineCommand, LaunchSpec, LlamaCaps, PortArg, RecipeId, Supervisor};
use lr_local_models::{Library, LibraryEntry, ModelKind};
use lr_types::{AppError, AppResult};

use super::{engine_error, engine_missing, parse_minutes, resolve_engine};
use crate::factory::{ParameterType, ProviderCategory, ProviderFactory, SetupParameter};
use crate::openai_compatible::OpenAICompatibleProvider;
use crate::{
    Capability, CompletionChunk, CompletionRequest, CompletionResponse, EmbeddingRequest,
    EmbeddingResponse, ModelInfo, ModelProvider, PricingInfo, ProviderHealth,
};

pub const PROVIDER_TYPE: &str = "llamacpp_embedded";

/// Loading a large model from disk can take a while.
const START_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextSetting {
    /// Let llama.cpp fit the context to available memory (`--fit`).
    Auto,
    Fixed(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuLayers {
    Auto,
    All,
    Count(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlamaSettings {
    pub binary_path: Option<PathBuf>,
    pub context: ContextSetting,
    pub gpu_layers: GpuLayers,
    /// `auto`, `on` or `off`.
    pub flash_attention: String,
    /// `f16`, `q8_0` or `q4_0`.
    pub kv_cache: String,
    pub parallel: Option<u32>,
    pub threads: Option<u32>,
    pub idle_timeout: Option<Duration>,
    pub max_loaded_models: usize,
}

fn opt_u32(config: &HashMap<String, String>, key: &str) -> AppResult<Option<u32>> {
    match config.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(v) => v
            .parse::<u32>()
            .map(Some)
            .map_err(|_| AppError::Config(format!("{key} must be a whole number"))),
        None => Ok(None),
    }
}

impl LlamaSettings {
    pub fn from_config(config: &HashMap<String, String>) -> AppResult<Self> {
        let get = |k: &str| {
            config
                .get(k)
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
        };
        let context = match get("context").as_deref() {
            None | Some("auto") => ContextSetting::Auto,
            Some(v) => ContextSetting::Fixed(v.parse().map_err(|_| {
                AppError::Config("context must be auto or a number of tokens".into())
            })?),
        };
        let gpu_layers = match get("gpu_layers").as_deref() {
            None | Some("auto") => GpuLayers::Auto,
            Some("all") => GpuLayers::All,
            Some(v) => GpuLayers::Count(v.parse().map_err(|_| {
                AppError::Config("gpu_layers must be auto, all or a number".into())
            })?),
        };
        let flash_attention = get("flash_attention").unwrap_or_else(|| "auto".into());
        if !matches!(flash_attention.as_str(), "auto" | "on" | "off") {
            return Err(AppError::Config(
                "flash_attention must be auto, on or off".into(),
            ));
        }
        let kv_cache = get("kv_cache").unwrap_or_else(|| "f16".into());
        if !matches!(kv_cache.as_str(), "f16" | "q8_0" | "q4_0") {
            return Err(AppError::Config(
                "kv_cache must be f16, q8_0 or q4_0".into(),
            ));
        }
        if kv_cache != "f16" && flash_attention == "off" {
            return Err(AppError::Config(
                "a quantized KV cache needs flash attention (auto or on)".into(),
            ));
        }
        let max_loaded_models = opt_u32(config, "max_loaded_models")?.unwrap_or(1).max(1) as usize;
        Ok(Self {
            binary_path: config
                .get("binary_path")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
            context,
            gpu_layers,
            flash_attention,
            kv_cache,
            parallel: opt_u32(config, "parallel")?,
            threads: opt_u32(config, "threads")?,
            idle_timeout: parse_minutes(config, "idle_unload_minutes", 15)?,
            max_loaded_models,
        })
    }

    /// `llama-server` arguments for one library model. Flags the installed
    /// build does not know are left out (older distro builds).
    pub fn server_args(&self, entry: &LibraryEntry, caps: &LlamaCaps) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "--host".into(),
            "127.0.0.1".into(),
            "-m".into(),
            entry.model_path.display().to_string(),
        ];
        if caps.jinja {
            args.push("--jinja".into());
        }
        if caps.no_webui {
            args.push("--no-webui".into());
        }
        if caps.offline {
            args.push("--offline".into());
        }
        match self.context {
            ContextSetting::Fixed(n) => args.extend(["-c".into(), n.to_string()]),
            // With --fit (default on in current builds) llama.cpp picks the
            // largest context that fits; older builds get a safe default.
            ContextSetting::Auto if !caps.fit => args.extend(["-c".into(), "4096".into()]),
            ContextSetting::Auto => {}
        }
        match self.gpu_layers {
            GpuLayers::Auto if caps.gpu_layers_auto => {}
            GpuLayers::Auto | GpuLayers::All => args.extend(["-ngl".into(), "999".into()]),
            GpuLayers::Count(n) => args.extend(["-ngl".into(), n.to_string()]),
        }
        match (self.flash_attention.as_str(), caps.flash_attn_auto) {
            // Current builds default to auto.
            ("auto", _) => {}
            // Current builds take a value.
            (value, true) => args.extend(["-fa".into(), value.to_string()]),
            // Older builds: a bare flag that turns it on; off is the default.
            ("on", false) => args.push("-fa".into()),
            (_, false) => {}
        }
        if self.kv_cache != "f16" {
            args.extend([
                "-ctk".into(),
                self.kv_cache.clone(),
                "-ctv".into(),
                self.kv_cache.clone(),
            ]);
        }
        if let Some(n) = self.parallel {
            args.extend(["-np".into(), n.to_string()]);
        }
        if let Some(n) = self.threads {
            args.extend(["-t".into(), n.to_string()]);
        }
        if let Some(projector) = &entry.projector_path {
            args.extend(["--mmproj".into(), projector.display().to_string()]);
        }
        if entry.kind == ModelKind::Embedding {
            args.push("--embeddings".into());
            let pooling = match entry.pooling_type {
                Some(1) => "mean",
                Some(2) => "cls",
                Some(3) => "last",
                _ => "mean",
            };
            args.extend(["--pooling".into(), pooling.into()]);
        }
        args
    }
}

fn capabilities(entry: &LibraryEntry) -> Vec<Capability> {
    match entry.kind {
        ModelKind::Chat => {
            let mut caps = vec![Capability::Chat, Capability::Completion];
            if entry.has_tools {
                caps.push(Capability::FunctionCalling);
            }
            if entry.projector_path.is_some() {
                caps.push(Capability::Vision);
            }
            caps
        }
        ModelKind::Completion => vec![Capability::Completion],
        ModelKind::Embedding => vec![Capability::Embedding],
        // Not served yet (no rerank endpoint; projectors/adapters aren't models).
        ModelKind::Reranker
        | ModelKind::Projector
        | ModelKind::Adapter
        | ModelKind::Unsupported => {
            vec![]
        }
    }
}

fn servable(entry: &LibraryEntry) -> bool {
    matches!(
        entry.kind,
        ModelKind::Chat | ModelKind::Completion | ModelKind::Embedding
    )
}

pub struct LlamaCppEmbeddedProvider {
    instance: String,
    settings: LlamaSettings,
    library: Arc<Library>,
    supervisor: Arc<Supervisor>,
    clients: parking_lot::Mutex<HashMap<String, (u16, Arc<OpenAICompatibleProvider>)>>,
}

impl LlamaCppEmbeddedProvider {
    pub fn new(
        instance: String,
        settings: LlamaSettings,
        library: Arc<Library>,
        supervisor: Arc<Supervisor>,
    ) -> Self {
        Self {
            instance,
            settings,
            library,
            supervisor,
            clients: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    fn key_prefix(&self) -> String {
        format!("{PROVIDER_TYPE}:{}:", self.instance)
    }

    fn entry(&self, model: &str) -> AppResult<LibraryEntry> {
        self.library
            .get(model)
            .filter(servable)
            .ok_or_else(|| AppError::ModelNotFound {
                model: model.to_string(),
            })
    }

    async fn command(&self) -> AppResult<EngineCommand> {
        resolve_engine(RecipeId::LlamaCpp, self.settings.binary_path.clone())
            .await
            .ok_or_else(|| engine_missing(PROVIDER_TYPE, "llama-server"))
    }

    /// Stop least-recently-used idle models so at most
    /// `max_loaded_models` run once `model` starts.
    async fn make_room(&self, model_key: &str) {
        let prefix = self.key_prefix();
        let mut running: Vec<_> = self
            .supervisor
            .processes()
            .into_iter()
            .filter(|p| {
                p.key.starts_with(&prefix)
                    && p.key != model_key
                    && p.state == lr_engines::EngineState::Running
            })
            .collect();
        while running.len() >= self.settings.max_loaded_models {
            // Idle ones only; busy models are never evicted.
            let Some(pos) = running
                .iter()
                .enumerate()
                .filter(|(_, p)| p.in_flight == 0)
                .max_by_key(|(_, p)| p.idle_secs.unwrap_or(0))
                .map(|(i, _)| i)
            else {
                break;
            };
            let victim = running.remove(pos);
            tracing::info!("Unloading {} to make room for {}", victim.key, model_key);
            self.supervisor.stop(&victim.key).await;
        }
    }

    async fn ensure_engine(&self, model: &str) -> AppResult<lr_engines::EngineHandle> {
        let entry = self.entry(model)?;
        let command = self.command().await?;
        let caps = lr_engines::detect::llama_capabilities(&command).await;
        let mut args = command.leading_args.clone();
        args.extend(self.settings.server_args(&entry, &caps));
        let key = format!("{}{}", self.key_prefix(), entry.id);
        if !self.supervisor.is_running(&key) {
            self.make_room(&key).await;
        }
        let spec = LaunchSpec {
            key,
            label: entry.display_name.clone(),
            program: command.program,
            args,
            env: vec![],
            port: PortArg::Flag("--port".into()),
            api_key_env: "LLAMA_API_KEY".into(),
            ready_path: "/health".into(),
            start_timeout: START_TIMEOUT,
            idle_timeout: self.settings.idle_timeout,
        };
        self.supervisor
            .ensure(spec)
            .await
            .map_err(|e| engine_error(PROVIDER_TYPE, e))
    }

    fn client(&self, handle: &lr_engines::EngineHandle) -> Arc<OpenAICompatibleProvider> {
        let mut clients = self.clients.lock();
        if let Some((port, client)) = clients.get(&handle.key) {
            if *port == handle.port {
                return client.clone();
            }
        }
        let client = Arc::new(
            OpenAICompatibleProvider::new(
                PROVIDER_TYPE.to_string(),
                format!("{}/v1", handle.base_url()),
                Some(handle.api_key().to_string()),
            )
            .with_provider_type(PROVIDER_TYPE),
        );
        clients.insert(handle.key.clone(), (handle.port, client.clone()));
        client
    }
}

impl Drop for LlamaCppEmbeddedProvider {
    fn drop(&mut self) {
        let supervisor = self.supervisor.clone();
        let prefix = self.key_prefix();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move { supervisor.stop_prefix(&prefix).await });
        }
    }
}

#[async_trait]
impl super::EmbeddedControl for LlamaCppEmbeddedProvider {
    async fn load(&self, model: &str) -> AppResult<()> {
        self.ensure_engine(model).await.map(|_| ())
    }

    async fn unload(&self, model: &str) -> AppResult<()> {
        self.supervisor
            .stop(&format!("{}{model}", self.key_prefix()))
            .await;
        Ok(())
    }

    fn model_states(&self) -> Vec<super::EmbeddedModelState> {
        let prefix = self.key_prefix();
        super::states_from_supervisor(&self.supervisor, &prefix, |k| {
            vec![k.trim_start_matches(&prefix).to_string()]
        })
    }
}

#[async_trait]
impl ModelProvider for LlamaCppEmbeddedProvider {
    fn name(&self) -> &str {
        PROVIDER_TYPE
    }

    async fn health_check(&self) -> ProviderHealth {
        // Never starts the engine.
        let found = self.command().await.is_ok();
        let entries = self.library.list();
        let no_models = if entries.is_empty() {
            "No model is in the library yet. Download or import one in the Models tab."
        } else {
            "No model in the library can run in llama.cpp. Download or import a chat, completion or embedding model in the Models tab."
        };
        super::engine_health(
            (!found).then(|| "llama-server was not found on PATH. Install llama.cpp from the provider's Engine tab.".to_string()),
            entries.iter().any(servable),
            no_models,
            &self.supervisor,
            &self.key_prefix(),
        )
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        Ok(self
            .library
            .list()
            .into_iter()
            .filter(servable)
            .map(|e| ModelInfo {
                capabilities: capabilities(&e),
                id: e.id,
                name: e.display_name,
                provider: PROVIDER_TYPE.to_string(),
                parameter_count: None,
                context_window: e.context_length.unwrap_or(4096).min(u32::MAX as u64) as u32,
                supports_streaming: true,
                detailed_capabilities: None,
            })
            .collect())
    }

    async fn get_pricing(&self, _model: &str) -> AppResult<PricingInfo> {
        Ok(PricingInfo::free())
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let handle = self.ensure_engine(&request.model).await?;
        let _lease = handle.lease();
        self.client(&handle).complete(request).await
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        let handle = self.ensure_engine(&request.model).await?;
        let lease = handle.lease();
        let stream = self.client(&handle).stream_complete(request).await?;
        // Keep the model marked busy until the client stops reading.
        Ok(Box::pin(stream.map(move |item| {
            let _ = &lease;
            item
        })))
    }

    async fn embed(&self, request: EmbeddingRequest) -> AppResult<EmbeddingResponse> {
        let handle = self.ensure_engine(&request.model).await?;
        let _lease = handle.lease();
        self.client(&handle).embed(request).await
    }

    fn supports_embeddings(&self) -> bool {
        true
    }

    fn supports_feature(&self, feature: &str) -> bool {
        // llama-server returns OpenAI-format logprobs (System One letter mode).
        feature == "logprobs"
    }

    fn embedded_control(&self) -> Option<&dyn super::EmbeddedControl> {
        Some(self)
    }
}

/// Factory for the llama.cpp Local Embedded provider.
pub struct LlamaCppEmbeddedProviderFactory {
    library: Arc<Library>,
    supervisor: Arc<Supervisor>,
}

impl LlamaCppEmbeddedProviderFactory {
    pub fn new(library: Arc<Library>, supervisor: Arc<Supervisor>) -> Self {
        Self {
            library,
            supervisor,
        }
    }
}

impl ProviderFactory for LlamaCppEmbeddedProviderFactory {
    fn provider_type(&self) -> &str {
        PROVIDER_TYPE
    }

    fn display_name(&self) -> &str {
        "llama.cpp"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::Embedded
    }

    /// The general-purpose engine leads the Local Embedded list.
    fn list_priority(&self) -> u8 {
        0
    }

    fn description(&self) -> &str {
        "Run GGUF models from Hugging Face with llama.cpp. LocalRouter downloads models, starts llama-server on demand and unloads idle models"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::AlwaysFreeLocal
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        let opt = |key: &str, ty, desc: &str, default: Option<&str>| {
            SetupParameter::optional(key, ty, desc, default, false)
        };
        vec![
            opt(
                "context",
                ParameterType::String,
                "Context length in tokens, or auto (fit to memory)",
                Some("auto"),
            ),
            opt(
                "gpu_layers",
                ParameterType::String,
                "Layers on the GPU: auto, all, or a number",
                Some("auto"),
            ),
            opt(
                "flash_attention",
                ParameterType::String,
                "Flash attention: auto, on or off",
                Some("auto"),
            ),
            opt(
                "kv_cache",
                ParameterType::String,
                "KV cache type: f16, q8_0 or q4_0 (quantized saves memory)",
                Some("f16"),
            ),
            opt(
                "parallel",
                ParameterType::Number,
                "Parallel request slots (default: llama.cpp's choice)",
                None,
            ),
            opt(
                "threads",
                ParameterType::Number,
                "CPU threads (default: llama.cpp's choice)",
                None,
            ),
            opt(
                "max_loaded_models",
                ParameterType::Number,
                "Models kept loaded at once; the least recently used idle one is unloaded",
                Some("1"),
            ),
            opt(
                "idle_unload_minutes",
                ParameterType::Number,
                "Unload a model after this many idle minutes (0 = keep loaded)",
                Some("15"),
            ),
            opt(
                "binary_path",
                ParameterType::String,
                "Path to llama-server (leave empty to find it on PATH)",
                None,
            ),
        ]
    }

    fn create(
        &self,
        instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        let settings = LlamaSettings::from_config(&config)?;
        Ok(Arc::new(LlamaCppEmbeddedProvider::new(
            instance_name,
            settings,
            self.library.clone(),
            self.supervisor.clone(),
        )))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        LlamaSettings::from_config(config).map(|_| ())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None
    }

    fn model_list_source(&self) -> crate::factory::ModelListSource {
        crate::factory::ModelListSource::ApiOnly
    }

    fn docs_url(&self) -> Option<&str> {
        Some("https://github.com/ggml-org/llama.cpp")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn cfg(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn entry(kind: ModelKind) -> LibraryEntry {
        LibraryEntry {
            id: "qwen3-8b-q4_k_m".into(),
            display_name: "Qwen3 8B Q4_K_M".into(),
            source: lr_local_models::EntrySource::Imported,
            model_path: PathBuf::from("/models/qwen3-8b.gguf"),
            extra_parts: vec![],
            projector_path: None,
            kind,
            quant: Some("Q4_K_M".into()),
            architecture: Some("qwen3".into()),
            context_length: Some(40_960),
            pooling_type: None,
            has_tools: true,
            size_bytes: 5_000_000_000,
            installed_at: Utc::now(),
        }
    }

    fn modern() -> LlamaCaps {
        LlamaCaps {
            fit: true,
            flash_attn_auto: true,
            no_webui: true,
            offline: true,
            jinja: true,
            gpu_layers_auto: true,
            api_key_env: true,
        }
    }

    #[test]
    fn defaults_leave_tuning_to_current_llama_cpp() {
        let s = LlamaSettings::from_config(&HashMap::new()).unwrap();
        let args = s.server_args(&entry(ModelKind::Chat), &modern());
        assert_eq!(
            args,
            vec![
                "--host",
                "127.0.0.1",
                "-m",
                "/models/qwen3-8b.gguf",
                "--jinja",
                "--no-webui",
                "--offline"
            ]
        );
    }

    #[test]
    fn older_builds_get_explicit_flags() {
        let s = LlamaSettings::from_config(&cfg(&[("flash_attention", "on")])).unwrap();
        let args = s.server_args(&entry(ModelKind::Chat), &LlamaCaps::default());
        let joined = args.join(" ");
        assert!(joined.contains("-c 4096"));
        assert!(joined.contains("-ngl 999"));
        assert!(args.iter().any(|a| a == "-fa"));
        assert!(!args.iter().any(|a| a == "--jinja"));
    }

    #[test]
    fn explicit_settings_and_embeddings() {
        let s = LlamaSettings::from_config(&cfg(&[
            ("context", "8192"),
            ("gpu_layers", "20"),
            ("kv_cache", "q8_0"),
            ("parallel", "2"),
            ("threads", "6"),
        ]))
        .unwrap();
        let mut e = entry(ModelKind::Embedding);
        e.pooling_type = Some(2);
        let joined = s.server_args(&e, &modern()).join(" ");
        for part in [
            "-c 8192",
            "-ngl 20",
            "-ctk q8_0 -ctv q8_0",
            "-np 2",
            "-t 6",
            "--embeddings --pooling cls",
        ] {
            assert!(joined.contains(part), "missing {part} in {joined}");
        }
    }

    #[test]
    fn settings_validation() {
        assert!(LlamaSettings::from_config(&cfg(&[("context", "big")])).is_err());
        assert!(LlamaSettings::from_config(&cfg(&[("gpu_layers", "most")])).is_err());
        assert!(LlamaSettings::from_config(&cfg(&[("kv_cache", "q2")])).is_err());
        assert!(LlamaSettings::from_config(&cfg(&[
            ("kv_cache", "q4_0"),
            ("flash_attention", "off")
        ]))
        .is_err());
        let s = LlamaSettings::from_config(&cfg(&[("max_loaded_models", "0")])).unwrap();
        assert_eq!(s.max_loaded_models, 1);
    }

    #[test]
    fn capabilities_follow_the_library_entry() {
        let mut chat = entry(ModelKind::Chat);
        chat.projector_path = Some(PathBuf::from("/models/mmproj.gguf"));
        let caps = capabilities(&chat);
        assert!(caps.contains(&Capability::FunctionCalling));
        assert!(caps.contains(&Capability::Vision));
        assert_eq!(
            capabilities(&entry(ModelKind::Embedding)),
            vec![Capability::Embedding]
        );
        assert!(!servable(&entry(ModelKind::Projector)));
    }

    #[tokio::test]
    async fn lists_only_servable_library_models() {
        let dir = tempfile::tempdir().unwrap();
        let library = Arc::new(Library::open(dir.path().to_path_buf()));
        let p = LlamaCppEmbeddedProvider::new(
            "llama".into(),
            LlamaSettings::from_config(&HashMap::new()).unwrap(),
            library,
            Supervisor::new(dir.path()),
        );
        assert!(p.list_models().await.unwrap().is_empty());
        let req = CompletionRequest::new("missing", vec![]);
        assert!(matches!(
            p.complete(req).await,
            Err(AppError::ModelNotFound { .. })
        ));
    }
}
