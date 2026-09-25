//! Von Local Embedded provider: LocalRouter runs `von serve` (from `uv tool install
//! von-sdk`) and warms it up. Von downloads its model (about 3.2 GB) from
//! Hugging Face the first time it answers, and its `/health` answers before
//! the model is loaded, so the first start sends a warm-up decision with a
//! long timeout before any client request goes through.

use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use serde_json::json;

use lr_config::FreeTierKind;
use lr_engines::{EngineCommand, EngineHandle, LaunchSpec, PortArg, RecipeId, Supervisor};
use lr_types::{AppError, AppResult};

use super::{
    engine_error, engine_missing, hf_env, parse_minutes, resolve_engine, SystemOneClientCache,
};
use crate::factory::{ParameterType, ProviderCategory, ProviderFactory, SetupParameter};
use crate::systemone::SystemOneFlavor;
use crate::{
    Capability, CompletionChunk, CompletionRequest, CompletionResponse, HealthStatus, ModelInfo,
    ModelProvider, PricingInfo, ProviderHealth, SupportLevel, SystemOneRequest, SystemOneResponse,
};

pub const PROVIDER_TYPE: &str = "von";

/// The model id clients use; Von serves one model per process.
pub const MODEL_ID: &str = "von-latest";

pub const DEVICES: &[&str] = &["auto", "cuda", "rocm", "mps", "openvino", "dml", "cpu"];

/// The port opens as soon as Python has imported the server.
const START_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// The warm-up covers the model download (about 3.2 GB) and load.
const WARMUP_TIMEOUT: Duration = Duration::from_secs(60 * 60);

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

    pub fn launch_spec(
        &self,
        instance: &str,
        command: &EngineCommand,
        hf: Vec<(String, String)>,
    ) -> LaunchSpec {
        let mut args = command.leading_args.clone();
        args.extend(["serve", "--host", "127.0.0.1"].map(String::from));
        if let Some(device) = &self.device {
            args.push("--device".to_string());
            args.push(device.clone());
        }
        LaunchSpec {
            key: format!("{PROVIDER_TYPE}:{instance}"),
            label: "Von".to_string(),
            program: command.program.clone(),
            args,
            // Von allows any CORS origin by default; keep browsers out.
            env: {
                let mut env = hf;
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
            idle_timeout: self.idle_timeout,
        }
    }
}

pub struct VonEmbeddedProvider {
    instance: String,
    settings: VonSettings,
    supervisor: Arc<Supervisor>,
    clients: SystemOneClientCache,
    /// Port of the engine process that finished its warm-up. Held across the
    /// warm-up so concurrent first requests wait for one download.
    warmed_port: tokio::sync::Mutex<Option<u16>>,
}

impl VonEmbeddedProvider {
    pub fn new(instance: String, settings: VonSettings, supervisor: Arc<Supervisor>) -> Self {
        Self {
            instance,
            settings,
            supervisor,
            clients: SystemOneClientCache::default(),
            warmed_port: tokio::sync::Mutex::new(None),
        }
    }

    fn key(&self) -> String {
        format!("{PROVIDER_TYPE}:{}", self.instance)
    }

    /// Start (or reuse) the engine and make sure its model is loaded.
    async fn ensure_engine(&self) -> AppResult<EngineHandle> {
        let command = resolve_engine(RecipeId::Von, self.settings.binary_path.clone())
            .await
            .ok_or_else(|| engine_missing(PROVIDER_TYPE, "von"))?;
        let spec = self
            .settings
            .launch_spec(&self.instance, &command, hf_env());
        let handle = self
            .supervisor
            .ensure(spec)
            .await
            .map_err(|e| engine_error(PROVIDER_TYPE, e))?;

        let mut warmed = self.warmed_port.lock().await;
        if *warmed != Some(handle.port) {
            let _lease = handle.lease();
            warm_up(&handle).await?;
            *warmed = Some(handle.port);
        }
        Ok(handle)
    }
}

/// Send one tiny decision so Von downloads and loads its model.
async fn warm_up(handle: &EngineHandle) -> AppResult<()> {
    let client = reqwest::Client::builder()
        .timeout(WARMUP_TIMEOUT)
        .build()
        .map_err(|e| AppError::Internal(format!("http client: {e}")))?;
    let resp = client
        .post(format!("{}/v1/systemone", handle.base_url()))
        .bearer_auth(handle.api_key())
        .json(&json!({
            "state": "LocalRouter warm-up",
            "questions": {"ready": {"type": "noul", "instructions": "Is this a warm-up request?"}}
        }))
        .send()
        .await
        .map_err(|e| {
            AppError::Provider(format!(
                "Provider '{PROVIDER_TYPE}' is unreachable: model warm-up failed: {e}"
            ))
        })?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Provider(format!(
            "Provider '{PROVIDER_TYPE}' is unreachable: model warm-up returned {status}: {}",
            body.chars().take(300).collect::<String>()
        )));
    }
    Ok(())
}

#[async_trait]
impl super::EmbeddedControl for VonEmbeddedProvider {
    async fn load(&self, model: &str) -> AppResult<()> {
        if model != MODEL_ID {
            return Err(AppError::ModelNotFound {
                model: model.to_string(),
            });
        }
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
        let found = resolve_engine(RecipeId::Von, self.settings.binary_path.clone())
            .await
            .is_some();
        ProviderHealth {
            status: if found {
                HealthStatus::Healthy
            } else {
                HealthStatus::Unhealthy
            },
            latency_ms: None,
            last_checked: Utc::now(),
            error_message: (!found).then(|| {
                "von was not found on PATH. Install it from the provider's Engine tab.".to_string()
            }),
        }
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
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
        if let Some(m) = request.model.as_deref() {
            if m != MODEL_ID {
                return Err(AppError::ModelNotFound {
                    model: m.to_string(),
                });
            }
        }
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
        let spec = s.launch_spec("Von", &command, vec![]);
        assert_eq!(spec.program, PathBuf::from("/bin/von"));
        assert_eq!(
            spec.args,
            vec!["serve", "--host", "127.0.0.1", "--device", "mps"]
        );
        assert_eq!(spec.api_key_env, "VON_API_KEY");
        assert_eq!(spec.ready_path, "/health");
        assert_eq!(spec.key, "von:Von");
        let auto = VonSettings::from_config(&cfg(&[("device", "auto")])).unwrap();
        assert!(!auto
            .launch_spec("Von", &command, vec![])
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
    async fn unknown_model_is_rejected_before_starting() {
        let dir = tempfile::tempdir().unwrap();
        let p = VonEmbeddedProvider::new(
            "Von".into(),
            VonSettings::from_config(&HashMap::new()).unwrap(),
            Supervisor::new(dir.path()),
        );
        let req: SystemOneRequest = serde_json::from_value(json!({
            "model": "von-9", "state": "s",
            "questions": {"q": {"type": "noul", "instructions": "?"}}
        }))
        .unwrap();
        assert!(matches!(
            p.systemone(req).await,
            Err(AppError::ModelNotFound { .. })
        ));
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
        for _ in 0..2 {
            let req: SystemOneRequest = serde_json::from_value(json!({
                "state": "s", "questions": {"q": {"type": "noul", "instructions": "?"}}
            }))
            .unwrap();
            p.systemone(req).await.unwrap();
        }
        let port = supervisor.processes()[0].port;
        assert_eq!(*p.warmed_port.lock().await, port);
        supervisor.stop_all().await;
    }
}
