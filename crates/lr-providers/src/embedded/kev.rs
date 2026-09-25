//! Kev Local Embedded provider: LocalRouter runs Kev (from its Git
//! repository, pinned to a commit) through uv, one process per checkpoint.
//! Checkpoints are downloaded explicitly from the Models tab; serving runs
//! offline, so a checkpoint that is not downloaded fails at once.

use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::Stream;

use lr_config::FreeTierKind;
use lr_engines::{EngineCommand, LaunchSpec, PortArg, RecipeId, Supervisor};
use lr_types::{AppError, AppResult};

use super::{
    download_env, engine_error, engine_missing, not_downloaded, offline_env, parse_minutes,
    resolve_engine, EmbeddedCatalogModel, EngineDownloads, SystemOneClientCache, Warmups,
};
use crate::factory::{ParameterType, ProviderCategory, ProviderFactory, SetupParameter};
use crate::systemone::SystemOneFlavor;
use crate::{
    Capability, CompletionChunk, CompletionRequest, CompletionResponse, ModelInfo, ModelProvider,
    PricingInfo, ProviderHealth, SupportLevel, SystemOneRequest, SystemOneResponse,
};

pub const PROVIDER_TYPE: &str = "kev";

/// Kev checkpoints: (id, Hugging Face repo, approximate total download, guidance).
pub const CHECKPOINTS: &[(&str, &str, &str, &str)] = &[
    (
        "kev-0.8b",
        "jaredpalmer/kev-0.8b",
        "1.8 GB",
        "Runs on any Apple Silicon Mac or a modest GPU",
    ),
    (
        "kev-4b",
        "jaredpalmer/kev-4b",
        "9.5 GB",
        "32 GB Mac, or a GPU with 9-14 GB VRAM",
    ),
    (
        "kev-9b",
        "jaredpalmer/kev-9b",
        "19.5 GB",
        "GPU with about 17 GB VRAM",
    ),
];

/// A download fetches PyTorch and the weights before the port opens.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(3 * 60 * 60);
/// Loading a downloaded checkpoint.
const START_TIMEOUT: Duration = Duration::from_secs(15 * 60);

fn repo_of(checkpoint: &str) -> Option<&'static str> {
    CHECKPOINTS.iter().find(|c| c.0 == checkpoint).map(|c| c.1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KevSettings {
    /// Path to `uv` (Kev runs through it); `None` finds it on PATH.
    pub uv_path: Option<PathBuf>,
    pub dtype: Option<String>,
    pub idle_timeout: Option<Duration>,
}

impl KevSettings {
    pub fn from_config(config: &HashMap<String, String>) -> AppResult<Self> {
        let dtype = config
            .get("dtype")
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty() && s != "auto");
        if let Some(d) = &dtype {
            if !matches!(d.as_str(), "bf16" | "fp16" | "fp32") {
                return Err(AppError::Config(format!(
                    "dtype must be auto, bf16, fp16 or fp32 (got '{d}')"
                )));
            }
        }
        Ok(Self {
            uv_path: config
                .get("binary_path")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
            dtype,
            idle_timeout: parse_minutes(config, "idle_unload_minutes", 15)?,
        })
    }

    /// The serving launch (offline) or, with `download`, the launch that
    /// fetches the checkpoint.
    pub fn launch_spec(
        &self,
        instance: &str,
        checkpoint: &str,
        command: &EngineCommand,
        download: bool,
    ) -> Option<LaunchSpec> {
        let repo = repo_of(checkpoint)?;
        let mut args = command.leading_args.clone();
        args.push("--run".to_string());
        args.push(repo.to_string());
        let mut env = if download {
            download_env()
        } else {
            offline_env()
        };
        if let Some(dtype) = &self.dtype {
            env.push(("KEV_DTYPE".to_string(), dtype.clone()));
        }
        let size = checkpoint.trim_start_matches("kev-");
        Some(LaunchSpec {
            key: if download {
                format!("{PROVIDER_TYPE}:{instance}:download:{checkpoint}")
            } else {
                format!("{PROVIDER_TYPE}:{instance}:{checkpoint}")
            },
            label: if download {
                format!("Kev {size} (downloading)")
            } else {
                format!("Kev {size}")
            },
            program: command.program.clone(),
            args,
            env,
            // Kev binds 127.0.0.1 itself and has no --host flag.
            port: PortArg::Flag("--port".to_string()),
            // Always set: Kev's CORS policy allows any origin.
            api_key_env: "KEV_API_KEY".to_string(),
            // Kev has no /health; its OpenAPI document is served without a key.
            ready_path: "/openapi.json".to_string(),
            start_timeout: if download {
                DOWNLOAD_TIMEOUT
            } else {
                START_TIMEOUT
            },
            idle_timeout: if download { None } else { self.idle_timeout },
        })
    }
}

pub struct KevEmbeddedProvider {
    instance: String,
    settings: KevSettings,
    supervisor: Arc<Supervisor>,
    clients: SystemOneClientCache,
    downloads: Arc<EngineDownloads>,
    warmups: Warmups,
}

impl KevEmbeddedProvider {
    pub fn new(instance: String, settings: KevSettings, supervisor: Arc<Supervisor>) -> Self {
        Self {
            downloads: EngineDownloads::new(PROVIDER_TYPE, supervisor.clone()),
            instance,
            settings,
            supervisor,
            clients: SystemOneClientCache::default(),
            warmups: Warmups::default(),
        }
    }

    fn key_prefix(&self) -> String {
        format!("{PROVIDER_TYPE}:{}:", self.instance)
    }

    fn is_downloaded(&self, checkpoint: &str) -> bool {
        self.downloads
            .is_downloaded(checkpoint, repo_of(checkpoint))
    }

    fn downloaded(&self) -> Vec<&'static str> {
        CHECKPOINTS
            .iter()
            .map(|c| c.0)
            .filter(|c| self.is_downloaded(c))
            .collect()
    }

    /// A known, downloaded checkpoint (`None` picks the first downloaded).
    fn servable(&self, model: Option<&str>) -> AppResult<String> {
        match model {
            Some(m) if repo_of(m).is_none() => Err(AppError::ModelNotFound {
                model: m.to_string(),
            }),
            Some(m) if !self.is_downloaded(m) => Err(not_downloaded(PROVIDER_TYPE, m)),
            Some(m) => Ok(m.to_string()),
            None => self
                .downloaded()
                .first()
                .map(|c| c.to_string())
                .ok_or_else(|| not_downloaded(PROVIDER_TYPE, CHECKPOINTS[0].0)),
        }
    }

    async fn command(&self) -> AppResult<EngineCommand> {
        resolve_engine(RecipeId::Kev, self.settings.uv_path.clone())
            .await
            .ok_or_else(|| engine_missing(PROVIDER_TYPE, "uv"))
    }

    /// Start (or reuse) the engine for a downloaded checkpoint.
    async fn ensure_engine(&self, checkpoint: &str) -> AppResult<lr_engines::EngineHandle> {
        let command = self.command().await?;
        let spec = self
            .settings
            .launch_spec(&self.instance, checkpoint, &command, false)
            .ok_or_else(|| AppError::ModelNotFound {
                model: checkpoint.to_string(),
            })?;
        let handle = self
            .supervisor
            .ensure(spec)
            .await
            .map_err(|e| engine_error(PROVIDER_TYPE, e))?;
        self.warmups.ensure(&handle, PROVIDER_TYPE).await?;
        Ok(handle)
    }
}

#[async_trait]
impl super::EmbeddedControl for KevEmbeddedProvider {
    async fn load(&self, model: &str) -> AppResult<()> {
        let checkpoint = self.servable(Some(model))?;
        self.ensure_engine(&checkpoint).await.map(|_| ())
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
            let rest = k.trim_start_matches(&prefix);
            if rest.starts_with("download:") {
                vec![]
            } else {
                vec![rest.to_string()]
            }
        })
    }

    fn catalog(&self) -> Vec<EmbeddedCatalogModel> {
        CHECKPOINTS
            .iter()
            .map(|(id, _, size, guidance)| EmbeddedCatalogModel {
                id: id.to_string(),
                name: format!("Kev {}", id.trim_start_matches("kev-")),
                download_size: size.to_string(),
                guidance: Some(guidance.to_string()),
                downloaded: self.is_downloaded(id),
                downloading: self.downloads.is_downloading(id),
                download_error: self.downloads.error(id),
            })
            .collect()
    }

    async fn download(&self, model: &str) -> AppResult<()> {
        let command = self.command().await?;
        let spec = self
            .settings
            .launch_spec(&self.instance, model, &command, true)
            .ok_or_else(|| AppError::ModelNotFound {
                model: model.to_string(),
            })?;
        self.downloads.start(model, spec, |_| async { Ok(()) })
    }

    async fn cancel_download(&self, model: &str) -> AppResult<()> {
        self.downloads.cancel(model).await;
        Ok(())
    }
}

impl Drop for KevEmbeddedProvider {
    fn drop(&mut self) {
        let supervisor = self.supervisor.clone();
        let prefix = format!("{PROVIDER_TYPE}:{}:", self.instance);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move { supervisor.stop_prefix(&prefix).await });
        }
    }
}

#[async_trait]
impl ModelProvider for KevEmbeddedProvider {
    fn name(&self) -> &str {
        PROVIDER_TYPE
    }

    async fn health_check(&self) -> ProviderHealth {
        // Never starts the engine.
        let found = self.command().await.is_ok();
        super::engine_health(
            (!found).then(|| {
                "uv was not found on PATH. Install it from the provider's Engine tab.".to_string()
            }),
            !self.downloaded().is_empty(),
            "No model is downloaded yet. Download one in the Models tab.",
            &self.supervisor,
            &self.key_prefix(),
        )
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        Ok(self
            .downloaded()
            .into_iter()
            .map(|id| ModelInfo {
                id: id.to_string(),
                name: format!("Kev {}", id.trim_start_matches("kev-")),
                provider: PROVIDER_TYPE.to_string(),
                parameter_count: None,
                context_window: 4_096,
                supports_streaming: false,
                capabilities: vec![Capability::Decision],
                detailed_capabilities: None,
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
        let checkpoint = self.servable(request.model.as_deref())?;
        let handle = self.ensure_engine(&checkpoint).await?;
        let _lease = handle.lease();
        let client = self.clients.get(SystemOneFlavor::Kev, &handle)?;
        // One checkpoint per process; the Kev flavor sends its default model.
        request.model = None;
        let mut response = client.systemone(request).await?;
        response.model = checkpoint;
        Ok(response)
    }
}

/// Factory for the Kev Local Embedded provider.
pub struct KevEmbeddedProviderFactory {
    supervisor: Arc<Supervisor>,
}

impl KevEmbeddedProviderFactory {
    pub fn new(supervisor: Arc<Supervisor>) -> Self {
        Self { supervisor }
    }
}

impl ProviderFactory for KevEmbeddedProviderFactory {
    fn provider_type(&self) -> &str {
        PROVIDER_TYPE
    }

    fn display_name(&self) -> &str {
        "Kev"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::Embedded
    }

    fn description(&self) -> &str {
        "Kev System One decision models (Qwen-based). LocalRouter runs Kev through uv; download checkpoints from Hugging Face in the Models tab"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::AlwaysFreeLocal
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::optional(
                "dtype",
                ParameterType::String,
                "Weights precision: auto, bf16, fp16 or fp32",
                Some("auto"),
                false,
            ),
            SetupParameter::optional(
                "idle_unload_minutes",
                ParameterType::Number,
                "Stop a checkpoint's engine after this many idle minutes (0 = keep running)",
                Some("15"),
                false,
            ),
            SetupParameter::optional(
                "binary_path",
                ParameterType::String,
                "Path to uv (leave empty to find it on PATH)",
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
        let settings = KevSettings::from_config(&config)?;
        Ok(Arc::new(KevEmbeddedProvider::new(
            instance_name,
            settings,
            self.supervisor.clone(),
        )))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        KevSettings::from_config(config).map(|_| ())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None
    }

    fn model_list_source(&self) -> crate::factory::ModelListSource {
        crate::factory::ModelListSource::ApiOnly
    }

    fn docs_url(&self) -> Option<&str> {
        Some("https://github.com/jaredpalmer/kev")
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

    fn uv_command() -> EngineCommand {
        lr_engines::detect::command_for(RecipeId::Kev, PathBuf::from("/bin/uv"))
    }

    fn has_env(spec: &LaunchSpec, key: &str, value: &str) -> bool {
        spec.env.contains(&(key.to_string(), value.to_string()))
    }

    #[test]
    fn serving_is_offline_and_downloads_are_not() {
        let s = KevSettings::from_config(&cfg(&[("dtype", "BF16")])).unwrap();
        let serve = s
            .launch_spec("Kev", "kev-4b", &uv_command(), false)
            .unwrap();
        assert_eq!(serve.program, PathBuf::from("/bin/uv"));
        assert!(serve
            .args
            .iter()
            .any(|a| a.contains(lr_engines::KEV_GIT_REV)));
        let run = serve.args.iter().position(|a| a == "--run").unwrap();
        assert_eq!(serve.args[run + 1], "jaredpalmer/kev-4b");
        assert_eq!(serve.api_key_env, "KEV_API_KEY");
        assert_eq!(serve.ready_path, "/openapi.json");
        assert!(has_env(&serve, "KEV_DTYPE", "bf16"));
        assert!(has_env(&serve, "HF_HUB_OFFLINE", "1"));
        assert_eq!(serve.key, "kev:Kev:kev-4b");

        let dl = s.launch_spec("Kev", "kev-4b", &uv_command(), true).unwrap();
        assert!(!has_env(&dl, "HF_HUB_OFFLINE", "1"));
        assert!(has_env(&dl, "HF_HUB_DISABLE_TELEMETRY", "1"));
        assert_eq!(dl.key, "kev:Kev:download:kev-4b");
        assert_eq!(dl.idle_timeout, None);
        assert!(s
            .launch_spec("Kev", "kev-99b", &uv_command(), false)
            .is_none());
    }

    #[test]
    fn settings_validation() {
        assert!(KevSettings::from_config(&cfg(&[("dtype", "int4")])).is_err());
        // Configs from before downloads were explicit still load.
        assert!(KevSettings::from_config(&cfg(&[("checkpoints", "kev-4b")])).is_ok());
    }

    #[tokio::test]
    async fn requests_never_download() {
        let dir = tempfile::tempdir().unwrap();
        let p = KevEmbeddedProvider::new(
            "Kev".into(),
            KevSettings::from_config(&HashMap::new()).unwrap(),
            Supervisor::new(dir.path()),
        );
        assert!(p.list_models().await.unwrap().is_empty());
        let req = |model: &str| -> SystemOneRequest {
            serde_json::from_value(serde_json::json!({
                "model": model, "state": "s",
                "questions": {"q": {"type": "noul", "instructions": "?"}}
            }))
            .unwrap()
        };
        assert!(matches!(
            p.systemone(req("kev-4b")).await,
            Err(AppError::InvalidParams(m)) if m.contains("not downloaded")
        ));
        assert!(matches!(
            p.systemone(req("kev-27b")).await,
            Err(AppError::ModelNotFound { .. })
        ));
        let catalog = p.catalog();
        assert_eq!(catalog.len(), 3);
        assert!(catalog.iter().all(|m| !m.downloaded && !m.downloading));
    }

    #[tokio::test]
    async fn health_reflects_engine_and_downloads() {
        use crate::HealthStatus;
        let dir = tempfile::tempdir().unwrap();
        let Some(fake) = crate::embedded::fake_engine_path() else {
            eprintln!("skipping: lr-fake-engine not built (run the workspace tests)");
            return;
        };
        let p = KevEmbeddedProvider::new(
            "Kev".into(),
            KevSettings::from_config(&cfg(&[("binary_path", fake.to_str().unwrap())])).unwrap(),
            Supervisor::new(dir.path()),
        );
        let h = p.health_check().await;
        assert_eq!(h.status, HealthStatus::Degraded);
        assert!(h.error_message.unwrap().contains("Models tab"));
        p.downloads
            .fake_downloaded(&dir.path().join("hub"), "kev-4b", "jaredpalmer/kev-4b");
        assert_eq!(p.health_check().await.status, HealthStatus::Healthy);
    }

    #[tokio::test]
    async fn download_runs_the_engine_once_and_records_it() {
        let Some(fake) = crate::embedded::fake_engine_path() else {
            eprintln!("skipping: lr-fake-engine not built (run the workspace tests)");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let supervisor = Supervisor::new(dir.path());
        // The fake engine stands in for uv; it reads --port from the args.
        let settings =
            KevSettings::from_config(&cfg(&[("binary_path", fake.to_str().unwrap())])).unwrap();
        let p = KevEmbeddedProvider::new("Kev".into(), settings, supervisor.clone());
        static CHANGED: parking_lot::Mutex<Vec<String>> = parking_lot::Mutex::new(Vec::new());
        crate::embedded::set_models_changed_hook(Arc::new(|t| CHANGED.lock().push(t.to_string())));
        p.download("kev-0.8b").await.unwrap();
        for _ in 0..400 {
            if !p.downloads.is_downloading("kev-0.8b") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(!p.downloads.is_downloading("kev-0.8b"));
        assert_eq!(p.downloads.error("kev-0.8b"), None);
        assert!(p.downloads.marker("kev-0.8b").is_file());
        assert!(!supervisor.is_running("kev:Kev:download:kev-0.8b"));
        // The app is told so it refreshes model lists.
        assert!(CHANGED.lock().iter().any(|t| t == "kev"));
        supervisor.stop_all().await;
    }
}
