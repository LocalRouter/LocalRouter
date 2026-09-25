//! Decider Local Embedded provider: LocalRouter runs Decider's server
//! (`decider-ai` has no console script) through `uv tool run … uvicorn
//! decider.serve:app`, one process per checkpoint. Checkpoints are
//! downloaded explicitly from the Models tab; serving runs offline, so a
//! checkpoint that is not downloaded fails at once.

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

pub const PROVIDER_TYPE: &str = "decider";

/// Decider checkpoints: (id, Hugging Face repo, download size, guidance).
pub const CHECKPOINTS: &[(&str, &str, &str, &str)] = &[
    (
        "decider-0.8b",
        "Mapika/decider-0.8b",
        "1.5 GB",
        "Runs on any Apple Silicon Mac or a modest GPU",
    ),
    (
        "decider-2b",
        "Mapika/decider-2b",
        "3.8 GB",
        "About 4 GB of GPU memory",
    ),
    (
        "decider-4b",
        "Mapika/decider-4b",
        "8.4 GB",
        "16 GB Mac, or a GPU with about 9 GB of memory",
    ),
];

pub const DEVICES: &[&str] = &["auto", "cuda", "mps", "cpu"];

/// A download fetches PyTorch and the weights before the port opens.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(3 * 60 * 60);
/// Loading a downloaded checkpoint (the port opens once it is loaded).
const START_TIMEOUT: Duration = Duration::from_secs(15 * 60);

fn repo_of(checkpoint: &str) -> Option<&'static str> {
    CHECKPOINTS.iter().find(|c| c.0 == checkpoint).map(|c| c.1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeciderSettings {
    /// Path to `uv` (Decider runs through it); `None` finds it on PATH.
    pub uv_path: Option<PathBuf>,
    pub device: Option<String>,
    pub idle_timeout: Option<Duration>,
}

impl DeciderSettings {
    pub fn from_config(config: &HashMap<String, String>) -> AppResult<Self> {
        let device = config
            .get("device")
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty());
        if let Some(d) = &device {
            if !DEVICES.contains(&d.as_str()) {
                return Err(AppError::Config(format!(
                    "device must be one of {} (got '{d}')",
                    DEVICES.join(", ")
                )));
            }
        }
        Ok(Self {
            uv_path: config
                .get("binary_path")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
            device: device.filter(|d| d != "auto"),
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
        let mut env = if download {
            download_env()
        } else {
            offline_env()
        };
        env.push(("DECIDER_MODEL".to_string(), repo.to_string()));
        if let Some(device) = &self.device {
            env.push(("DECIDER_DEVICE".to_string(), device.clone()));
        }
        let size = checkpoint.trim_start_matches("decider-");
        Some(LaunchSpec {
            key: if download {
                format!("{PROVIDER_TYPE}:{instance}:download:{checkpoint}")
            } else {
                format!("{PROVIDER_TYPE}:{instance}:{checkpoint}")
            },
            label: if download {
                format!("Decider {size} (downloading)")
            } else {
                format!("Decider {size}")
            },
            program: command.program.clone(),
            // `--host 127.0.0.1` is part of the leading args: Decider has no
            // authentication, so it must never listen beyond localhost.
            args: command.leading_args.clone(),
            env,
            port: PortArg::Flag("--port".to_string()),
            // Decider ignores it; the supervisor always sets one.
            api_key_env: "DECIDER_API_KEY".to_string(),
            ready_path: "/health".to_string(),
            start_timeout: if download {
                DOWNLOAD_TIMEOUT
            } else {
                START_TIMEOUT
            },
            idle_timeout: if download { None } else { self.idle_timeout },
        })
    }
}

pub struct DeciderEmbeddedProvider {
    instance: String,
    settings: DeciderSettings,
    supervisor: Arc<Supervisor>,
    clients: SystemOneClientCache,
    downloads: Arc<EngineDownloads>,
    warmups: Warmups,
}

impl DeciderEmbeddedProvider {
    pub fn new(instance: String, settings: DeciderSettings, supervisor: Arc<Supervisor>) -> Self {
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
        resolve_engine(RecipeId::Decider, self.settings.uv_path.clone())
            .await
            .ok_or_else(|| engine_missing(PROVIDER_TYPE, "uv"))
    }

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
impl super::EmbeddedControl for DeciderEmbeddedProvider {
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
                name: format!("Decider {}", id.trim_start_matches("decider-")),
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

impl Drop for DeciderEmbeddedProvider {
    fn drop(&mut self) {
        let supervisor = self.supervisor.clone();
        let prefix = self.key_prefix();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move { supervisor.stop_prefix(&prefix).await });
        }
    }
}

#[async_trait]
impl ModelProvider for DeciderEmbeddedProvider {
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
                name: format!("Decider {}", id.trim_start_matches("decider-")),
                provider: PROVIDER_TYPE.to_string(),
                parameter_count: None,
                context_window: 32_768,
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
        let client = self.clients.get(SystemOneFlavor::Generic, &handle)?;
        // One checkpoint per process; Decider ignores the model field.
        request.model = None;
        let mut response = client.systemone(request).await?;
        response.model = checkpoint;
        Ok(response)
    }
}

/// Factory for the Decider Local Embedded provider.
pub struct DeciderEmbeddedProviderFactory {
    supervisor: Arc<Supervisor>,
}

impl DeciderEmbeddedProviderFactory {
    pub fn new(supervisor: Arc<Supervisor>) -> Self {
        Self { supervisor }
    }
}

impl ProviderFactory for DeciderEmbeddedProviderFactory {
    fn provider_type(&self) -> &str {
        PROVIDER_TYPE
    }

    fn display_name(&self) -> &str {
        "Decider"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::Embedded
    }

    fn description(&self) -> &str {
        "Decider System One decision models (Qwen-based). LocalRouter runs Decider through uv; download checkpoints from Hugging Face in the Models tab"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::AlwaysFreeLocal
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::optional(
                "device",
                ParameterType::String,
                "Device: auto, cuda, mps or cpu",
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
        let settings = DeciderSettings::from_config(&config)?;
        Ok(Arc::new(DeciderEmbeddedProvider::new(
            instance_name,
            settings,
            self.supervisor.clone(),
        )))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        DeciderSettings::from_config(config).map(|_| ())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None
    }

    fn model_list_source(&self) -> crate::factory::ModelListSource {
        crate::factory::ModelListSource::ApiOnly
    }

    fn docs_url(&self) -> Option<&str> {
        Some("https://github.com/Mapika/decider")
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
        lr_engines::detect::command_for(RecipeId::Decider, PathBuf::from("/bin/uv"))
    }

    fn has_env(spec: &LaunchSpec, key: &str, value: &str) -> bool {
        spec.env.contains(&(key.to_string(), value.to_string()))
    }

    #[test]
    fn launch_spec_runs_uvicorn_on_localhost() {
        let s = DeciderSettings::from_config(&cfg(&[("device", "CPU")])).unwrap();
        let spec = s
            .launch_spec("Decider", "decider-2b", &uv_command(), false)
            .unwrap();
        assert_eq!(spec.program, PathBuf::from("/bin/uv"));
        assert!(spec.args.windows(2).any(|w| w == ["--host", "127.0.0.1"]));
        assert!(spec.args.contains(&"decider.serve:app".to_string()));
        assert!(has_env(&spec, "DECIDER_MODEL", "Mapika/decider-2b"));
        assert!(has_env(&spec, "DECIDER_DEVICE", "cpu"));
        assert!(has_env(&spec, "HF_HUB_OFFLINE", "1"));
        assert_eq!(spec.key, "decider:Decider:decider-2b");
        let dl = s
            .launch_spec("Decider", "decider-2b", &uv_command(), true)
            .unwrap();
        assert!(!has_env(&dl, "HF_HUB_OFFLINE", "1"));
        assert_eq!(dl.key, "decider:Decider:download:decider-2b");
        assert!(s
            .launch_spec("Decider", "decider-35b", &uv_command(), false)
            .is_none());
    }

    #[test]
    fn settings_validation() {
        let d = DeciderSettings::from_config(&HashMap::new()).unwrap();
        assert_eq!(d.device, None);
        assert!(DeciderSettings::from_config(&cfg(&[("device", "rocm")])).is_err());
    }

    #[tokio::test]
    async fn requests_never_download() {
        let dir = tempfile::tempdir().unwrap();
        let p = DeciderEmbeddedProvider::new(
            "Decider".into(),
            DeciderSettings::from_config(&HashMap::new()).unwrap(),
            Supervisor::new(dir.path()),
        );
        assert!(p.list_models().await.unwrap().is_empty());
        let req: SystemOneRequest = serde_json::from_value(serde_json::json!({
            "state": "s", "questions": {"q": {"type": "noul", "instructions": "?"}}
        }))
        .unwrap();
        assert!(matches!(
            p.systemone(req).await,
            Err(AppError::InvalidParams(m)) if m.contains("not downloaded")
        ));
        assert_eq!(p.catalog().len(), 3);
    }

    #[tokio::test]
    async fn download_runs_the_engine_once_and_records_it() {
        let Some(fake) = crate::embedded::fake_engine_path() else {
            eprintln!("skipping: lr-fake-engine not built (run the workspace tests)");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let supervisor = Supervisor::new(dir.path());
        let settings =
            DeciderSettings::from_config(&cfg(&[("binary_path", fake.to_str().unwrap())])).unwrap();
        let p = DeciderEmbeddedProvider::new("Decider".into(), settings, supervisor.clone());
        p.download("decider-2b").await.unwrap();
        for _ in 0..400 {
            if !p.downloads.is_downloading("decider-2b") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert_eq!(p.downloads.error("decider-2b"), None);
        assert!(p.downloads.marker("decider-2b").is_file());
        assert!(!supervisor.is_running("decider:Decider:download:decider-2b"));
        supervisor.stop_all().await;
    }
}
