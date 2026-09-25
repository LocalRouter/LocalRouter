//! Laya Local Embedded provider: LocalRouter runs the official `laya-serve` (installed
//! with `uv tool install "laya[serve]"`) and serves System One decisions from
//! it. Laya downloads its checkpoints from Hugging Face on first start.

use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;

use lr_config::FreeTierKind;
use lr_engines::{LaunchSpec, PortArg, RecipeId, Supervisor};
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

pub const PROVIDER_TYPE: &str = "laya";

/// Laya checkpoints: (id, display name, context tokens, approximate download).
pub const CHECKPOINTS: &[(&str, &str, u32, &str)] = &[
    ("english", "Laya English", 512, "843 MB"),
    ("multilingual", "Laya Multilingual", 1_024, "678 MB"),
    ("typed-decisions", "Laya Typed Decisions", 1_024, "843 MB"),
];

/// First start downloads checkpoints before the port opens.
const START_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayaSettings {
    pub binary_path: Option<PathBuf>,
    pub checkpoints: Vec<String>,
    /// `None` lets Laya pick (CUDA, then MPS, then CPU).
    pub device: Option<String>,
    pub threads: Option<u32>,
    pub idle_timeout: Option<Duration>,
}

impl LayaSettings {
    pub fn from_config(config: &HashMap<String, String>) -> AppResult<Self> {
        let ids: Vec<&str> = CHECKPOINTS.iter().map(|c| c.0).collect();
        let device = config
            .get("device")
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty() && s != "auto");
        if let Some(d) = &device {
            if !matches!(d.as_str(), "cpu" | "cuda" | "mps") && !d.starts_with("cuda:") {
                return Err(AppError::Config(format!(
                    "device must be auto, cpu, cuda or mps (got '{d}')"
                )));
            }
        }
        let threads = match config
            .get("threads")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            Some(v) => Some(
                v.parse::<u32>()
                    .map_err(|_| AppError::Config("threads must be a whole number".into()))?,
            ),
            None => None,
        };
        Ok(Self {
            binary_path: config
                .get("binary_path")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
            checkpoints: parse_list(config, "checkpoints", &ids, &["english"])?,
            device,
            threads,
            idle_timeout: parse_minutes(config, "idle_unload_minutes", 15)?,
        })
    }

    /// The process launch for these settings. The API key and port are added
    /// by the supervisor.
    pub fn launch_spec(
        &self,
        instance: &str,
        program: PathBuf,
        hf: Vec<(String, String)>,
    ) -> LaunchSpec {
        let mut env = vec![
            // laya-serve binds 0.0.0.0 by default; never expose it.
            ("LAYA_HOST".to_string(), "127.0.0.1".to_string()),
            ("LAYA_MODELS".to_string(), self.checkpoints.join(",")),
            ("LAYA_PRELOAD".to_string(), "1".to_string()),
        ];
        if let Some(device) = &self.device {
            env.push(("LAYA_DEVICE".to_string(), device.clone()));
        }
        if let Some(threads) = self.threads {
            env.push(("LAYA_THREADS".to_string(), threads.to_string()));
        }
        env.extend(hf);
        LaunchSpec {
            key: format!("{PROVIDER_TYPE}:{instance}"),
            label: "Laya".to_string(),
            program,
            args: vec![],
            env,
            port: PortArg::Env("LAYA_PORT".to_string()),
            api_key_env: "LAYA_API_KEY".to_string(),
            ready_path: "/health".to_string(),
            start_timeout: START_TIMEOUT,
            idle_timeout: self.idle_timeout,
        }
    }
}

pub struct LayaEmbeddedProvider {
    instance: String,
    settings: LayaSettings,
    supervisor: Arc<Supervisor>,
    clients: SystemOneClientCache,
}

impl LayaEmbeddedProvider {
    pub fn new(instance: String, settings: LayaSettings, supervisor: Arc<Supervisor>) -> Self {
        Self {
            instance,
            settings,
            supervisor,
            clients: SystemOneClientCache::default(),
        }
    }

    fn spec_key(&self) -> String {
        format!("{PROVIDER_TYPE}:{}", self.instance)
    }

    /// Start (or reuse) the Laya engine.
    async fn ensure_engine(&self) -> AppResult<lr_engines::EngineHandle> {
        let command = resolve_engine(RecipeId::Laya, self.settings.binary_path.clone())
            .await
            .ok_or_else(|| engine_missing(PROVIDER_TYPE, "laya-serve"))?;
        let spec = self
            .settings
            .launch_spec(&self.instance, command.program, hf_env());
        self.supervisor.ensure(spec).await.map_err(|e| {
            let mut err = engine_error(PROVIDER_TYPE, e);
            if let AppError::Provider(msg) = &mut err {
                if msg.contains("did not become ready") {
                    msg.push_str(
                        "\nIf the unofficial PyPI package 'laya-serve' is installed, it ignores LocalRouter's port: remove it (uv tool uninstall laya-serve) and install the official \"laya[serve]\".",
                    );
                }
            }
            err
        })
    }
}

#[async_trait]
impl super::EmbeddedControl for LayaEmbeddedProvider {
    async fn load(&self, _model: &str) -> AppResult<()> {
        // One process serves every enabled checkpoint.
        self.ensure_engine().await.map(|_| ())
    }

    async fn unload(&self, _model: &str) -> AppResult<()> {
        self.supervisor.stop(&self.spec_key()).await;
        Ok(())
    }

    fn model_states(&self) -> Vec<super::EmbeddedModelState> {
        let key = self.spec_key();
        let checkpoints = self.settings.checkpoints.clone();
        super::states_from_supervisor(&self.supervisor, &key, |k| {
            if k == key {
                checkpoints.clone()
            } else {
                vec![]
            }
        })
    }
}

impl Drop for LayaEmbeddedProvider {
    fn drop(&mut self) {
        // The provider was removed or reconfigured: stop its engine.
        let supervisor = self.supervisor.clone();
        let key = self.spec_key();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move { supervisor.stop(&key).await });
        }
    }
}

#[async_trait]
impl ModelProvider for LayaEmbeddedProvider {
    fn name(&self) -> &str {
        PROVIDER_TYPE
    }

    async fn health_check(&self) -> ProviderHealth {
        // Never starts the engine: only checks it is installed.
        let found = resolve_engine(RecipeId::Laya, self.settings.binary_path.clone())
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
                "laya-serve was not found on PATH. Install it from the provider's Engine tab."
                    .to_string()
            }),
        }
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        Ok(self
            .settings
            .checkpoints
            .iter()
            .filter_map(|id| CHECKPOINTS.iter().find(|c| c.0 == id))
            .map(|(id, name, ctx, _)| ModelInfo {
                id: id.to_string(),
                name: name.to_string(),
                provider: PROVIDER_TYPE.to_string(),
                parameter_count: None,
                context_window: *ctx,
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

    async fn systemone(&self, request: SystemOneRequest) -> AppResult<SystemOneResponse> {
        if let Some(model) = request.model.as_deref() {
            if !self.settings.checkpoints.iter().any(|c| c == model) {
                return Err(AppError::ModelNotFound {
                    model: model.to_string(),
                });
            }
        }
        let handle = self.ensure_engine().await?;
        let _lease = handle.lease();
        let client = self.clients.get(SystemOneFlavor::Laya, &handle)?;
        client.systemone(request).await
    }
}

/// Factory for the Laya Local Embedded provider.
pub struct LayaEmbeddedProviderFactory {
    supervisor: Arc<Supervisor>,
}

impl LayaEmbeddedProviderFactory {
    pub fn new(supervisor: Arc<Supervisor>) -> Self {
        Self { supervisor }
    }
}

impl ProviderFactory for LayaEmbeddedProviderFactory {
    fn provider_type(&self) -> &str {
        PROVIDER_TYPE
    }

    fn display_name(&self) -> &str {
        "Laya"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::Embedded
    }

    fn description(&self) -> &str {
        "Laya System One decision models (typed choice, score and yes/no answers). LocalRouter runs laya-serve and downloads checkpoints from Hugging Face"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::AlwaysFreeLocal
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::optional(
                "checkpoints",
                ParameterType::String,
                "Checkpoints to serve, comma-separated: english, multilingual, typed-decisions",
                Some("english"),
                false,
            ),
            SetupParameter::optional(
                "device",
                ParameterType::String,
                "auto, cpu, cuda or mps",
                Some("auto"),
                false,
            ),
            SetupParameter::optional(
                "threads",
                ParameterType::Number,
                "CPU threads (default: PyTorch's choice)",
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
                "binary_path",
                ParameterType::String,
                "Path to laya-serve (leave empty to find it on PATH)",
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
        let settings = LayaSettings::from_config(&config)?;
        Ok(Arc::new(LayaEmbeddedProvider::new(
            instance_name,
            settings,
            self.supervisor.clone(),
        )))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        LayaSettings::from_config(config).map(|_| ())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None
    }

    fn model_list_source(&self) -> crate::factory::ModelListSource {
        crate::factory::ModelListSource::ApiOnly
    }

    fn docs_url(&self) -> Option<&str> {
        Some("https://github.com/NandhaKishorM/laya")
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
    fn launch_spec_is_local_only_and_keyed() {
        let s = LayaSettings::from_config(&cfg(&[
            ("checkpoints", "multilingual,english"),
            ("device", "CPU"),
            ("threads", "4"),
        ]))
        .unwrap();
        let spec = s.launch_spec(
            "Laya",
            PathBuf::from("/bin/laya-serve"),
            vec![("HF_TOKEN".into(), "hf_x".into())],
        );
        let env: HashMap<_, _> = spec.env.iter().cloned().collect();
        assert_eq!(env["LAYA_HOST"], "127.0.0.1");
        assert_eq!(env["LAYA_MODELS"], "multilingual,english");
        assert_eq!(env["LAYA_DEVICE"], "cpu");
        assert_eq!(env["LAYA_THREADS"], "4");
        assert_eq!(env["HF_TOKEN"], "hf_x");
        assert_eq!(spec.api_key_env, "LAYA_API_KEY");
        assert_eq!(spec.port, PortArg::Env("LAYA_PORT".into()));
        assert!(
            spec.args.is_empty(),
            "nothing secret or config goes in argv"
        );
    }

    #[test]
    fn settings_validation() {
        let d = LayaSettings::from_config(&HashMap::new()).unwrap();
        assert_eq!(d.checkpoints, vec!["english"]);
        assert_eq!(d.device, None);
        assert_eq!(d.idle_timeout, Some(Duration::from_secs(900)));
        assert!(LayaSettings::from_config(&cfg(&[("checkpoints", "klingon")])).is_err());
        assert!(LayaSettings::from_config(&cfg(&[("device", "tpu")])).is_err());
        assert!(LayaSettings::from_config(&cfg(&[("threads", "many")])).is_err());
        assert!(LayaSettings::from_config(&cfg(&[("device", "cuda:1")])).is_ok());
    }

    #[tokio::test]
    async fn lists_enabled_checkpoints_as_decision_models() {
        let dir = tempfile::tempdir().unwrap();
        let settings =
            LayaSettings::from_config(&cfg(&[("checkpoints", "english,typed-decisions")])).unwrap();
        let p = LayaEmbeddedProvider::new("Laya".into(), settings, Supervisor::new(dir.path()));
        let models = p.list_models().await.unwrap();
        let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["english", "typed-decisions"]);
        assert!(models
            .iter()
            .all(|m| m.capabilities == vec![Capability::Decision]));
        assert!(!p.supports_chat());
        // Unknown models are rejected before any engine starts.
        let req: SystemOneRequest = serde_json::from_value(serde_json::json!({
            "model": "multilingual", "state": "s",
            "questions": {"q": {"type": "noul", "instructions": "?"}}
        }))
        .unwrap();
        assert!(matches!(
            p.systemone(req).await,
            Err(AppError::ModelNotFound { .. })
        ));
    }

    #[test]
    fn factory_is_direct() {
        let dir = tempfile::tempdir().unwrap();
        let f = LayaEmbeddedProviderFactory::new(Supervisor::new(dir.path()));
        assert_eq!(f.category(), ProviderCategory::Embedded);
        assert!(f.listed());
        assert!(f.validate_config(&cfg(&[("checkpoints", "nope")])).is_err());
    }

    #[tokio::test]
    async fn serves_decisions_through_a_supervised_engine() {
        let Some(fake) = crate::embedded::fake_engine_path() else {
            eprintln!("skipping: lr-fake-engine not built (run the workspace tests)");
            return;
        };
        // The fake engine reads its port and key from the variables Laya uses.
        std::env::set_var("FAKE_PORT_VAR", "LAYA_PORT");
        std::env::set_var("FAKE_KEY_VAR", "LAYA_API_KEY");
        let dir = tempfile::tempdir().unwrap();
        let supervisor = Supervisor::new(dir.path());
        let settings =
            LayaSettings::from_config(&cfg(&[("binary_path", fake.to_str().unwrap())])).unwrap();
        let p = LayaEmbeddedProvider::new("Laya".into(), settings, supervisor.clone());
        let req: SystemOneRequest = serde_json::from_value(serde_json::json!({
            "model": "english", "state": "s",
            "questions": {"q": {"type": "noul", "instructions": "?"}}
        }))
        .unwrap();
        let resp = p.systemone(req).await.unwrap();
        assert!(resp.answers.contains_key("q"));
        assert!(supervisor.is_running("laya:Laya"));
        // Dropping the provider stops its engine.
        drop(p);
        for _ in 0..100 {
            if !supervisor.is_running("laya:Laya") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!supervisor.is_running("laya:Laya"));
    }
}
