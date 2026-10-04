//! Von Local Embedded provider: LocalRouter runs `von serve` (from `uv tool
//! install von-sdk`). Von loads its model (about 3.2 GB) on the first
//! decision, and its `/health` answers before that, so every start sends a
//! warm-up decision. The model is downloaded explicitly from the Models tab
//! (a network-enabled start plus warm-up); serving runs offline.

use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::Stream;

use lr_config::FreeTierKind;
use lr_engines::{EngineCommand, EngineHandle, LaunchSpec, PortArg, RecipeId, Supervisor};
use lr_types::{AppError, AppResult};

use super::{
    download_env, engine_error, engine_missing, not_downloaded, offline_env, parse_minutes,
    resolve_engine, warm_up, EmbeddedCatalogModel, EngineDownloads, SystemOneClientCache, Warmups,
};
use crate::factory::{ParameterType, ProviderCategory, ProviderFactory, SetupParameter};
use crate::systemone::SystemOneFlavor;
use crate::{
    Capability, CompletionChunk, CompletionRequest, CompletionResponse, ModelInfo, ModelProvider,
    PricingInfo, ProviderHealth, SupportLevel, SystemOneRequest, SystemOneResponse,
};

pub const PROVIDER_TYPE: &str = "von";

/// The model id clients use; Von serves one model per process.
pub const MODEL_ID: &str = "von-latest";

/// The Hugging Face repository Von loads.
const REPO: &str = "wfzyx/von";

pub const DEVICES: &[&str] = &["auto", "cuda", "rocm", "mps", "openvino", "dml", "cpu"];

/// The port opens as soon as Python has imported the server.
const START_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// The download warm-up covers fetching about 3.2 GB.
const DOWNLOAD_WARMUP_TIMEOUT: Duration = Duration::from_secs(3 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VonSettings {
    pub binary_path: Option<PathBuf>,
    /// `None` lets Von pick (CUDA, MPS, OpenVINO, then CPU).
    pub device: Option<String>,
    pub idle_timeout: Option<Duration>,
}

impl VonSettings {
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
            binary_path: config
                .get("binary_path")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
            device: device.filter(|d| d != "auto"),
            idle_timeout: parse_minutes(config, "idle_unload_minutes", 15)?,
        })
    }

    /// The serving launch (offline) or, with `download`, the launch whose
    /// warm-up fetches the model.
    pub fn launch_spec(
        &self,
        instance: &str,
        command: &EngineCommand,
        download: bool,
    ) -> LaunchSpec {
        let mut args = command.leading_args.clone();
        args.extend(["serve", "--host", "127.0.0.1"].map(String::from));
        if let Some(device) = &self.device {
            args.push("--device".to_string());
            args.push(device.clone());
        }
        LaunchSpec {
            key: if download {
                format!("{PROVIDER_TYPE}:{instance}:download:{MODEL_ID}")
            } else {
                format!("{PROVIDER_TYPE}:{instance}")
            },
            label: if download {
                "Von (downloading)".to_string()
            } else {
                "Von".to_string()
            },
            program: command.program.clone(),
            args,
            // Von allows any CORS origin by default; keep browsers out.
            env: {
                let mut env = if download {
                    download_env()
                } else {
                    offline_env()
                };
                env.push((
                    "VON_CORS_ORIGINS".to_string(),
                    "http://127.0.0.1".to_string(),
                ));
                env
            },
            port: PortArg::Flag("--port".to_string()),
            api_key_env: "VON_API_KEY".to_string(),
            ready_path: "/health".to_string(),
            start_timeout: START_TIMEOUT,
            idle_timeout: if download { None } else { self.idle_timeout },
        }
    }
}

pub struct VonEmbeddedProvider {
    instance: String,
    settings: VonSettings,
    supervisor: Arc<Supervisor>,
    clients: SystemOneClientCache,
    warmups: Warmups,
    downloads: Arc<EngineDownloads>,
}

impl VonEmbeddedProvider {
    pub fn new(instance: String, settings: VonSettings, supervisor: Arc<Supervisor>) -> Self {
        Self {
            downloads: EngineDownloads::new(PROVIDER_TYPE, supervisor.clone()),
            instance,
            settings,
            supervisor,
            clients: SystemOneClientCache::default(),
            warmups: Warmups::default(),
        }
    }

    fn key(&self) -> String {
        format!("{PROVIDER_TYPE}:{}", self.instance)
    }

    fn is_downloaded(&self) -> bool {
        self.downloads.is_downloaded(MODEL_ID, Some(REPO))
    }

    /// `model` must be Von's id (or absent) and downloaded.
    fn check_servable(&self, model: Option<&str>) -> AppResult<()> {
        match model {
            Some(m) if m != MODEL_ID => Err(AppError::ModelNotFound {
                model: m.to_string(),
            }),
            _ if !self.is_downloaded() => Err(not_downloaded(PROVIDER_TYPE, MODEL_ID)),
            _ => Ok(()),
        }
    }

    async fn command(&self) -> AppResult<EngineCommand> {
        resolve_engine(RecipeId::Von, self.settings.binary_path.clone())
            .await
            .ok_or_else(|| engine_missing(PROVIDER_TYPE, "von"))
    }

    /// Start (or reuse) the engine and make sure its model is loaded.
    async fn ensure_engine(&self) -> AppResult<EngineHandle> {
        let command = self.command().await?;
        let spec = self.settings.launch_spec(&self.instance, &command, false);
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
impl super::EmbeddedControl for VonEmbeddedProvider {
    async fn load(&self, model: &str) -> AppResult<()> {
        self.check_servable(Some(model))?;
        self.ensure_engine().await.map(|_| ())
    }

    async fn unload(&self, _model: &str) -> AppResult<()> {
        self.supervisor.stop(&self.key()).await;
        Ok(())
    }

    fn model_states(&self) -> Vec<super::EmbeddedModelState> {
        let key = self.key();
        super::states_from_supervisor(&self.supervisor, &key, |k| {
            if k == key {
                vec![MODEL_ID.to_string()]
            } else {
                vec![]
            }
        })
    }

    fn catalog(&self) -> Vec<EmbeddedCatalogModel> {
        vec![EmbeddedCatalogModel {
            id: MODEL_ID.to_string(),
            name: "Von (ModernBERT-large)".to_string(),
            download_size: "3.2 GB".to_string(),
            guidance: Some("Runs on CPU; faster with a GPU or Apple Silicon".to_string()),
            downloaded: self.is_downloaded(),
            downloading: self.downloads.is_downloading(MODEL_ID),
            download_error: self.downloads.error(MODEL_ID),
            progress: None,
            removable: false,
            unavailable: None,
        }]
    }

    async fn download(&self, model: &str) -> AppResult<()> {
        if model != MODEL_ID {
            return Err(AppError::ModelNotFound {
                model: model.to_string(),
            });
        }
        let command = self.command().await?;
        let spec = self.settings.launch_spec(&self.instance, &command, true);
        self.downloads.start(MODEL_ID, spec, |handle| async move {
            warm_up(&handle, PROVIDER_TYPE, DOWNLOAD_WARMUP_TIMEOUT).await
        })
    }

    async fn cancel_download(&self, model: &str) -> AppResult<()> {
        self.downloads.cancel(model).await;
        Ok(())
    }
}

impl Drop for VonEmbeddedProvider {
    fn drop(&mut self) {
        let supervisor = self.supervisor.clone();
        let key = self.key();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move { supervisor.stop(&key).await });
        }
    }
}

#[async_trait]
impl ModelProvider for VonEmbeddedProvider {
    fn name(&self) -> &str {
        PROVIDER_TYPE
    }

    async fn health_check(&self) -> ProviderHealth {
        // Never starts the engine.
        let found = self.command().await.is_ok();
        super::engine_health(
            (!found).then(|| {
                "von was not found on PATH. Install it from the provider's Engine tab.".to_string()
            }),
            self.is_downloaded(),
            "No model is downloaded yet. Download one in the Models tab.",
            &self.supervisor,
            &self.key(),
        )
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        if !self.is_downloaded() {
            return Ok(Vec::new());
        }
        Ok(vec![ModelInfo {
            id: MODEL_ID.to_string(),
            name: "Von (ModernBERT-large)".to_string(),
            provider: PROVIDER_TYPE.to_string(),
            parameter_count: None,
            context_window: 8_192,
            supports_streaming: false,
            capabilities: vec![Capability::Decision],
            detailed_capabilities: None,
        }])
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
        self.check_servable(request.model.as_deref())?;
        let handle = self.ensure_engine().await?;
        let _lease = handle.lease();
        let client = self.clients.get(SystemOneFlavor::Generic, &handle)?;
        // Von serves one model and names its exact version in the response.
        request.model = None;
        client.systemone(request).await
    }
}

/// Factory for the Von Local Embedded provider.
pub struct VonEmbeddedProviderFactory {
    supervisor: Arc<Supervisor>,
}

impl VonEmbeddedProviderFactory {
    pub fn new(supervisor: Arc<Supervisor>) -> Self {
        Self { supervisor }
    }
}

impl ProviderFactory for VonEmbeddedProviderFactory {
    fn provider_type(&self) -> &str {
        PROVIDER_TYPE
    }

    fn display_name(&self) -> &str {
        "Von"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::Embedded
    }

    fn description(&self) -> &str {
        "Von System One decision model (ModernBERT-large). LocalRouter runs von serve and it downloads the model from Hugging Face on first use"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::AlwaysFreeLocal
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::optional(
                "device",
                ParameterType::String,
                "Device: auto, cuda, rocm, mps, openvino, dml or cpu",
                Some("auto"),
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
                "binary_path",
                ParameterType::String,
                "Path to von (leave empty to find it on PATH)",
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
        let settings = VonSettings::from_config(&config)?;
        Ok(Arc::new(VonEmbeddedProvider::new(
            instance_name,
            settings,
            self.supervisor.clone(),
        )))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        VonSettings::from_config(config).map(|_| ())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None
    }

    fn model_list_source(&self) -> crate::factory::ModelListSource {
        crate::factory::ModelListSource::ApiOnly
    }

    fn docs_url(&self) -> Option<&str> {
        Some("https://github.com/wfzyx/von")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedded::EmbeddedControl;
    use serde_json::json;

    fn cfg(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn launch_spec_binds_localhost_with_a_key() {
        let s = VonSettings::from_config(&cfg(&[("device", "MPS")])).unwrap();
        let command = lr_engines::detect::command_for(RecipeId::Von, PathBuf::from("/bin/von"));
        let spec = s.launch_spec("Von", &command, false);
        assert_eq!(spec.program, PathBuf::from("/bin/von"));
        assert_eq!(
            spec.args,
            vec!["serve", "--host", "127.0.0.1", "--device", "mps"]
        );
        assert_eq!(spec.api_key_env, "VON_API_KEY");
        assert_eq!(spec.ready_path, "/health");
        assert_eq!(spec.key, "von:Von");
        assert!(spec
            .env
            .contains(&("HF_HUB_OFFLINE".to_string(), "1".to_string())));
        let dl = s.launch_spec("Von", &command, true);
        assert_eq!(dl.key, "von:Von:download:von-latest");
        assert!(!dl
            .env
            .contains(&("HF_HUB_OFFLINE".to_string(), "1".to_string())));
        let auto = VonSettings::from_config(&cfg(&[("device", "auto")])).unwrap();
        assert!(!auto
            .launch_spec("Von", &command, false)
            .args
            .contains(&"--device".to_string()));
    }

    #[test]
    fn settings_validation() {
        assert!(VonSettings::from_config(&cfg(&[("device", "tpu")])).is_err());
        assert!(VonSettings::from_config(&cfg(&[("idle_unload_minutes", "x")])).is_err());
        let d = VonSettings::from_config(&HashMap::new()).unwrap();
        assert_eq!(d.device, None);
        assert_eq!(d.idle_timeout, Some(Duration::from_secs(15 * 60)));
    }

    #[tokio::test]
    async fn requests_never_download() {
        let dir = tempfile::tempdir().unwrap();
        let p = VonEmbeddedProvider::new(
            "Von".into(),
            VonSettings::from_config(&HashMap::new()).unwrap(),
            Supervisor::new(dir.path()),
        );
        let req = |model: Option<&str>| -> SystemOneRequest {
            serde_json::from_value(json!({
                "model": model, "state": "s",
                "questions": {"q": {"type": "noul", "instructions": "?"}}
            }))
            .unwrap()
        };
        assert!(matches!(
            p.systemone(req(Some("von-9"))).await,
            Err(AppError::ModelNotFound { .. })
        ));
        assert!(matches!(
            p.systemone(req(None)).await,
            Err(AppError::InvalidParams(m)) if m.contains("not downloaded")
        ));
        assert!(p.list_models().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn warms_up_once_per_engine_process() {
        let Some(fake) = crate::embedded::fake_engine_path() else {
            eprintln!("skipping: lr-fake-engine not built (run the workspace tests)");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let supervisor = Supervisor::new(dir.path());
        let settings =
            VonSettings::from_config(&cfg(&[("binary_path", fake.to_str().unwrap())])).unwrap();
        let p = VonEmbeddedProvider::new("Von".into(), settings, supervisor.clone());
        p.downloads
            .fake_downloaded(&dir.path().join("hub"), MODEL_ID, REPO);
        assert_eq!(p.list_models().await.unwrap().len(), 1);
        for _ in 0..2 {
            let req: SystemOneRequest = serde_json::from_value(json!({
                "state": "s", "questions": {"q": {"type": "noul", "instructions": "?"}}
            }))
            .unwrap();
            p.systemone(req).await.unwrap();
        }
        let port = supervisor.processes()[0].port.unwrap();
        assert!(p.warmups.is_warm("von:Von", port).await);
        supervisor.stop_all().await;
    }

    #[tokio::test]
    async fn download_warms_up_and_records_it() {
        let Some(fake) = crate::embedded::fake_engine_path() else {
            eprintln!("skipping: lr-fake-engine not built (run the workspace tests)");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let supervisor = Supervisor::new(dir.path());
        let settings =
            VonSettings::from_config(&cfg(&[("binary_path", fake.to_str().unwrap())])).unwrap();
        let p = VonEmbeddedProvider::new("Von".into(), settings, supervisor.clone());
        p.download(MODEL_ID).await.unwrap();
        for _ in 0..400 {
            if !p.downloads.is_downloading(MODEL_ID) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert_eq!(p.downloads.error(MODEL_ID), None);
        assert!(p.downloads.marker(MODEL_ID).is_file());
        assert!(!supervisor.is_running("von:Von:download:von-latest"));
        supervisor.stop_all().await;
    }
}
