//! Kev Local Embedded provider: LocalRouter runs Kev (from its Git repository, pinned
//! to a commit) through uv, one process per enabled checkpoint. Kev downloads
//! its adapter and Qwen base model from Hugging Face on first start.

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

/// First start downloads PyTorch and model weights before the port opens.
const START_TIMEOUT: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KevSettings {
    /// Path to `uv` (Kev runs through it); `None` finds it on PATH.
    pub uv_path: Option<PathBuf>,
    pub checkpoints: Vec<String>,
    pub dtype: Option<String>,
    pub idle_timeout: Option<Duration>,
}

impl KevSettings {
    pub fn from_config(config: &HashMap<String, String>) -> AppResult<Self> {
        let ids: Vec<&str> = CHECKPOINTS.iter().map(|c| c.0).collect();
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
            checkpoints: parse_list(config, "checkpoints", &ids, &["kev-0.8b"])?,
            dtype,
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
        let mut args = command.leading_args.clone();
        args.push("--run".to_string());
        args.push(repo.to_string());
        let mut env = hf;
        if let Some(dtype) = &self.dtype {
            env.push(("KEV_DTYPE".to_string(), dtype.clone()));
        }
        Some(LaunchSpec {
            key: format!("{PROVIDER_TYPE}:{instance}:{checkpoint}"),
            label: format!("Kev {}", checkpoint.trim_start_matches("kev-")),
            program: command.program.clone(),
            args,
            env,
            // Kev binds 127.0.0.1 itself and has no --host flag.
            port: PortArg::Flag("--port".to_string()),
            // Always set: Kev's CORS policy allows any origin.
            api_key_env: "KEV_API_KEY".to_string(),
            // Kev has no /health; its OpenAPI document is served without a key.
            ready_path: "/openapi.json".to_string(),
            start_timeout: START_TIMEOUT,
            idle_timeout: self.idle_timeout,
        })
    }
}

pub struct KevEmbeddedProvider {
    instance: String,
    settings: KevSettings,
    supervisor: Arc<Supervisor>,
    clients: SystemOneClientCache,
}

impl KevEmbeddedProvider {
    pub fn new(instance: String, settings: KevSettings, supervisor: Arc<Supervisor>) -> Self {
        Self {
            instance,
            settings,
            supervisor,
            clients: SystemOneClientCache::default(),
        }
    }
}

impl KevEmbeddedProvider {
    fn key_prefix(&self) -> String {
        format!("{PROVIDER_TYPE}:{}:", self.instance)
    }

    /// Start (or reuse) the engine for one checkpoint.
    async fn ensure_engine(&self, checkpoint: &str) -> AppResult<lr_engines::EngineHandle> {
        if !self.settings.checkpoints.iter().any(|c| c == checkpoint) {
            return Err(AppError::ModelNotFound {
                model: checkpoint.to_string(),
            });
        }
        let command = resolve_engine(RecipeId::Kev, self.settings.uv_path.clone())
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
impl super::EmbeddedControl for KevEmbeddedProvider {
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
        let found = resolve_engine(RecipeId::Kev, self.settings.uv_path.clone())
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
        "Kev System One decision models (Qwen-based). LocalRouter runs Kev through uv and downloads checkpoints from Hugging Face"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::AlwaysFreeLocal
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::optional(
                "checkpoints",
                ParameterType::String,
                "Checkpoints to serve, comma-separated: kev-0.8b, kev-4b, kev-9b",
                Some("kev-0.8b"),
                false,
            ),
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

    fn cfg(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn uv_command() -> EngineCommand {
        lr_engines::detect::command_for(RecipeId::Kev, PathBuf::from("/bin/uv"))
    }

    #[test]
    fn launch_spec_runs_pinned_kev_through_uv() {
        let s = KevSettings::from_config(&cfg(&[("checkpoints", "kev-4b"), ("dtype", "BF16")]))
            .unwrap();
        let spec = s
            .launch_spec("Kev", "kev-4b", &uv_command(), vec![])
            .unwrap();
        assert_eq!(spec.program, PathBuf::from("/bin/uv"));
        assert!(spec
            .args
            .iter()
            .any(|a| a.contains(lr_engines::KEV_GIT_REV)));
        let run = spec.args.iter().position(|a| a == "--run").unwrap();
        assert_eq!(spec.args[run + 1], "jaredpalmer/kev-4b");
        assert_eq!(spec.api_key_env, "KEV_API_KEY");
        assert_eq!(spec.ready_path, "/openapi.json");
        assert!(spec
            .env
            .contains(&("KEV_DTYPE".to_string(), "bf16".to_string())));
        assert_eq!(spec.key, "kev:Kev:kev-4b");
        assert!(s
            .launch_spec("Kev", "kev-99b", &uv_command(), vec![])
            .is_none());
    }

    #[test]
    fn settings_validation() {
        let d = KevSettings::from_config(&HashMap::new()).unwrap();
        assert_eq!(d.checkpoints, vec!["kev-0.8b"]);
        assert!(KevSettings::from_config(&cfg(&[("checkpoints", "kev-27b")])).is_err());
        assert!(KevSettings::from_config(&cfg(&[("dtype", "int4")])).is_err());
    }

    #[tokio::test]
    async fn models_and_routing_checks() {
        let dir = tempfile::tempdir().unwrap();
        let settings =
            KevSettings::from_config(&cfg(&[("checkpoints", "kev-0.8b,kev-4b")])).unwrap();
        let p = KevEmbeddedProvider::new("Kev".into(), settings, Supervisor::new(dir.path()));
        let ids: Vec<_> = p
            .list_models()
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, vec!["kev-0.8b", "kev-4b"]);
        let req: SystemOneRequest = serde_json::from_value(serde_json::json!({
            "model": "kev-9b", "state": "s",
            "questions": {"q": {"type": "noul", "instructions": "?"}}
        }))
        .unwrap();
        assert!(matches!(
            p.systemone(req).await,
            Err(AppError::ModelNotFound { .. })
        ));
    }

    #[tokio::test]
    async fn one_engine_per_checkpoint() {
        let Some(fake) = crate::embedded::fake_engine_path() else {
            eprintln!("skipping: lr-fake-engine not built (run the workspace tests)");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let supervisor = Supervisor::new(dir.path());
        // The fake engine stands in for uv; it reads --port from the args.
        let settings = KevSettings::from_config(&cfg(&[
            ("checkpoints", "kev-0.8b,kev-4b"),
            ("binary_path", fake.to_str().unwrap()),
        ]))
        .unwrap();
        let p = KevEmbeddedProvider::new("Kev".into(), settings, supervisor.clone());
        for model in ["kev-0.8b", "kev-4b"] {
            let req: SystemOneRequest = serde_json::from_value(serde_json::json!({
                "model": model, "state": "s",
                "questions": {"q": {"type": "noul", "instructions": "?"}}
            }))
            .unwrap();
            let resp = p.systemone(req).await.unwrap();
            assert_eq!(resp.model, model);
        }
        assert!(supervisor.is_running("kev:Kev:kev-0.8b"));
        assert!(supervisor.is_running("kev:Kev:kev-4b"));
        supervisor.stop_all().await;
    }
}
