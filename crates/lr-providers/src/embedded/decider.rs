//! Decider Local Embedded provider: LocalRouter runs Decider's server (`decider-ai`
//! has no console script) through `uv tool run … uvicorn decider.serve:app`,
//! one process per enabled checkpoint. Decider downloads its checkpoint from
//! Hugging Face and loads it before the port opens.

use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;

use lr_config::FreeTierKind;
use lr_engines::{EngineCommand, LaunchSpec, PortArg, RecipeId, Supervisor};
use lr_types::{AppError, AppResult};

use super::{
    engine_error, engine_missing, hf_env, parse_list, parse_minutes, resolve_engine,
    SystemOneClientCache,
};
use crate::factory::{ParameterType, ProviderCategory, ProviderFactory, SetupParameter};
use crate::systemone::SystemOneFlavor;
use crate::{
    Capability, CompletionChunk, CompletionRequest, CompletionResponse, HealthStatus, ModelInfo,
    ModelProvider, PricingInfo, ProviderHealth, SupportLevel, SystemOneRequest, SystemOneResponse,
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

/// The port opens only after the download and model load finish.
const START_TIMEOUT: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeciderSettings {
    /// Path to `uv` (Decider runs through it); `None` finds it on PATH.
    pub uv_path: Option<PathBuf>,
    pub checkpoints: Vec<String>,
    pub device: Option<String>,
    pub idle_timeout: Option<Duration>,
}

impl DeciderSettings {
    pub fn from_config(config: &HashMap<String, String>) -> AppResult<Self> {
        let ids: Vec<&str> = CHECKPOINTS.iter().map(|c| c.0).collect();
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
            checkpoints: parse_list(config, "checkpoints", &ids, &["decider-0.8b"])?,
            device: device.filter(|d| d != "auto"),
            idle_timeout: parse_minutes(config, "idle_unload_minutes", 15)?,
        })
    }

    pub fn launch_spec(
        &self,
        instance: &str,
        checkpoint: &str,
        command: &EngineCommand,
        hf: Vec<(String, String)>,
    ) -> Option<LaunchSpec> {
        let repo = CHECKPOINTS.iter().find(|c| c.0 == checkpoint)?.1;
        let mut env = hf;
        env.push(("DECIDER_MODEL".to_string(), repo.to_string()));
        if let Some(device) = &self.device {
            env.push(("DECIDER_DEVICE".to_string(), device.clone()));
        }
        Some(LaunchSpec {
            key: format!("{PROVIDER_TYPE}:{instance}:{checkpoint}"),
            label: format!("Decider {}", checkpoint.trim_start_matches("decider-")),
            program: command.program.clone(),
            // `--host 127.0.0.1` is part of the leading args: Decider has no
            // authentication, so it must never listen beyond localhost.
            args: command.leading_args.clone(),
            env,
            port: PortArg::Flag("--port".to_string()),
            // Decider ignores it; the supervisor always sets one.
            api_key_env: "DECIDER_API_KEY".to_string(),
            ready_path: "/health".to_string(),
            start_timeout: START_TIMEOUT,
            idle_timeout: self.idle_timeout,
        })
    }
}

pub struct DeciderEmbeddedProvider {
    instance: String,
    settings: DeciderSettings,
    supervisor: Arc<Supervisor>,
    clients: SystemOneClientCache,
}

impl DeciderEmbeddedProvider {
    pub fn new(instance: String, settings: DeciderSettings, supervisor: Arc<Supervisor>) -> Self {
        Self {
            instance,
            settings,
            supervisor,
            clients: SystemOneClientCache::default(),
        }
    }

    fn key_prefix(&self) -> String {
        format!("{PROVIDER_TYPE}:{}:", self.instance)
    }

    async fn ensure_engine(&self, checkpoint: &str) -> AppResult<lr_engines::EngineHandle> {
        if !self.settings.checkpoints.iter().any(|c| c == checkpoint) {
            return Err(AppError::ModelNotFound {
                model: checkpoint.to_string(),
            });
        }
        let command = resolve_engine(RecipeId::Decider, self.settings.uv_path.clone())
            .await
            .ok_or_else(|| engine_missing(PROVIDER_TYPE, "uv"))?;
        let spec = self
            .settings
            .launch_spec(&self.instance, checkpoint, &command, hf_env())
            .ok_or_else(|| AppError::ModelNotFound {
                model: checkpoint.to_string(),
            })?;
        self.supervisor
            .ensure(spec)
            .await
            .map_err(|e| engine_error(PROVIDER_TYPE, e))
    }
}

#[async_trait]
impl super::EmbeddedControl for DeciderEmbeddedProvider {
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
        let found = resolve_engine(RecipeId::Decider, self.settings.uv_path.clone())
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
                "uv was not found on PATH. Install it from the provider's Engine tab.".to_string()
            }),
        }
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        Ok(self
            .settings
            .checkpoints
            .iter()
            .map(|id| ModelInfo {
                id: id.clone(),
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
        let checkpoint = match request.model.as_deref() {
            Some(m) if self.settings.checkpoints.iter().any(|c| c == m) => m.to_string(),
            Some(m) => {
                return Err(AppError::ModelNotFound {
                    model: m.to_string(),
                })
            }
            None => self.settings.checkpoints[0].clone(),
        };
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
        "Decider System One decision models (Qwen-based). LocalRouter runs Decider through uv and downloads checkpoints from Hugging Face"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::AlwaysFreeLocal
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::optional(
                "checkpoints",
                ParameterType::String,
                "Checkpoints to serve, comma-separated: decider-0.8b, decider-2b, decider-4b",
                Some("decider-0.8b"),
                false,
            ),
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

    fn cfg(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn launch_spec_runs_uvicorn_on_localhost() {
        let s =
            DeciderSettings::from_config(&cfg(&[("checkpoints", "decider-2b"), ("device", "CPU")]))
                .unwrap();
        let command = lr_engines::detect::command_for(RecipeId::Decider, PathBuf::from("/bin/uv"));
        let spec = s
            .launch_spec("Decider", "decider-2b", &command, vec![])
            .unwrap();
        assert_eq!(spec.program, PathBuf::from("/bin/uv"));
        assert!(spec.args.windows(2).any(|w| w == ["--host", "127.0.0.1"]));
        assert!(spec.args.contains(&"decider.serve:app".to_string()));
        assert!(spec
            .env
            .contains(&("DECIDER_MODEL".to_string(), "Mapika/decider-2b".to_string())));
        assert!(spec
            .env
            .contains(&("DECIDER_DEVICE".to_string(), "cpu".to_string())));
        assert_eq!(spec.key, "decider:Decider:decider-2b");
        assert!(s
            .launch_spec("Decider", "decider-35b", &command, vec![])
            .is_none());
    }

    #[test]
    fn settings_validation() {
        let d = DeciderSettings::from_config(&HashMap::new()).unwrap();
        assert_eq!(d.checkpoints, vec!["decider-0.8b"]);
        assert_eq!(d.device, None);
        assert!(DeciderSettings::from_config(&cfg(&[("checkpoints", "decider-35b-a3b")])).is_err());
        assert!(DeciderSettings::from_config(&cfg(&[("device", "rocm")])).is_err());
    }

    #[tokio::test]
    async fn one_engine_per_checkpoint() {
        let Some(fake) = crate::embedded::fake_engine_path() else {
            eprintln!("skipping: lr-fake-engine not built (run the workspace tests)");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let supervisor = Supervisor::new(dir.path());
        let settings = DeciderSettings::from_config(&cfg(&[
            ("checkpoints", "decider-0.8b,decider-2b"),
            ("binary_path", fake.to_str().unwrap()),
        ]))
        .unwrap();
        let p = DeciderEmbeddedProvider::new("Decider".into(), settings, supervisor.clone());
        for model in ["decider-0.8b", "decider-2b"] {
            let req: SystemOneRequest = serde_json::from_value(serde_json::json!({
                "model": model, "state": "s",
                "questions": {"q": {"type": "noul", "instructions": "?"}}
            }))
            .unwrap();
            assert_eq!(p.systemone(req).await.unwrap().model, model);
        }
        assert!(supervisor.is_running("decider:Decider:decider-0.8b"));
        assert!(supervisor.is_running("decider:Decider:decider-2b"));
        supervisor.stop_all().await;
    }
}
