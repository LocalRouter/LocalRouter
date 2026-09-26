//! stable-diffusion.cpp Local Embedded provider: LocalRouter runs
//! `sd-server` (chosen by the user, downloaded from the latest release on
//! request, or found on PATH) for image models downloaded in the Models tab,
//! one image model loaded at a time.
//!
//! The app supplies the image models through [`ImageModelBackend`] (it owns
//! the download manager and the model library).

use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::Stream;
use parking_lot::RwLock;
use serde_json::{json, Value};

use lr_config::FreeTierKind;
use lr_engines::{EngineCommand, EngineHandle, LaunchSpec, PortArg, RecipeId, Supervisor};
use lr_local_models::ImageModelLaunch;
use lr_types::{AppError, AppResult};

use super::{engine_error, engine_missing, not_downloaded, parse_minutes, resolve_engine};
use crate::factory::{ParameterType, ProviderCategory, ProviderFactory, SetupParameter};
use crate::{
    Capability, CompletionChunk, CompletionRequest, CompletionResponse, GeneratedImage,
    ImageEditRequest, ImageGenerationRequest, ImageGenerationResponse, ImageInput, ModelInfo,
    ModelProvider, PricingInfo, ProviderHealth, SupportLevel,
};

pub const PROVIDER_TYPE: &str = "sdcpp_embedded";

/// Loading a large image model (diffusion + text encoder) from disk.
const START_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// One generation on a slow machine (CPU, large model, several images).
const GENERATE_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// An image model as the provider sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageModelStatus {
    pub id: String,
    pub name: String,
    pub description: String,
    pub total_bytes: u64,
    pub downloaded: bool,
    pub downloading: bool,
    /// 0.0–1.0 while downloading.
    pub progress: Option<f64>,
    pub error: Option<String>,
}

/// Image models for the stable-diffusion.cpp provider, supplied by the app.
#[async_trait]
pub trait ImageModelBackend: Send + Sync {
    fn models(&self) -> Vec<ImageModelStatus>;
    /// Files and arguments for a downloaded model.
    fn launch(&self, id: &str) -> Option<ImageModelLaunch>;
    async fn start_download(&self, id: &str) -> Result<(), String>;
    fn cancel_download(&self, id: &str);
    fn remove(&self, id: &str) -> Result<(), String>;
}

static BACKEND: RwLock<Option<Arc<dyn ImageModelBackend>>> = RwLock::new(None);

/// Register the app's image model backend.
pub fn set_image_model_backend(backend: Arc<dyn ImageModelBackend>) {
    *BACKEND.write() = Some(backend);
}

fn backend() -> AppResult<Arc<dyn ImageModelBackend>> {
    BACKEND
        .read()
        .clone()
        .ok_or_else(|| AppError::Internal("image models are not available".into()))
}

fn format_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdSettings {
    pub binary_path: Option<PathBuf>,
    /// Keep weights in RAM and move them to the GPU as needed (less VRAM).
    pub offload_to_cpu: bool,
    pub flash_attention: bool,
    pub idle_timeout: Option<Duration>,
}

fn parse_bool(config: &HashMap<String, String>, key: &str, default: bool) -> AppResult<bool> {
    match config
        .get(key)
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .as_deref()
    {
        None => Ok(default),
        Some("true" | "on" | "yes" | "1") => Ok(true),
        Some("false" | "off" | "no" | "0") => Ok(false),
        Some(v) => Err(AppError::Config(format!(
            "{key} must be on or off (got '{v}')"
        ))),
    }
}

impl SdSettings {
    pub fn from_config(config: &HashMap<String, String>) -> AppResult<Self> {
        Ok(Self {
            binary_path: config
                .get("binary_path")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
            offload_to_cpu: parse_bool(config, "offload_to_cpu", true)?,
            flash_attention: parse_bool(config, "flash_attention", true)?,
            idle_timeout: parse_minutes(config, "idle_unload_minutes", 15)?,
        })
    }

    /// The `sd-server` launch for one model. sd-server has no API key
    /// option: it only ever listens on 127.0.0.1, behind LocalRouter's auth.
    pub fn launch_spec(
        &self,
        instance: &str,
        model: &str,
        command: &EngineCommand,
        launch: &ImageModelLaunch,
    ) -> LaunchSpec {
        let mut args = command.leading_args.clone();
        args.extend(["--listen-ip".to_string(), "127.0.0.1".to_string()]);
        args.extend(launch.args());
        if self.offload_to_cpu {
            args.push("--offload-to-cpu".into());
        }
        if self.flash_attention {
            args.push("--diffusion-fa".into());
        }
        LaunchSpec {
            key: format!("{PROVIDER_TYPE}:{instance}:{model}"),
            label: format!("stable-diffusion.cpp {model}"),
            program: command.program.clone(),
            args,
            env: Vec::new(),
            port: PortArg::Flag("--listen-port".into()),
            // Unused by sd-server; the supervisor always sets one.
            api_key_env: "SD_SERVER_API_KEY".into(),
            ready_path: "/v1/models".into(),
            start_timeout: START_TIMEOUT,
            idle_timeout: self.idle_timeout,
        }
    }
}

/// The OpenAI-style body sent to `sd-server`.
fn generation_body(request: &ImageGenerationRequest, default_size: &str) -> Value {
    json!({
        "prompt": request.prompt,
        "n": request.n.unwrap_or(1).clamp(1, 10),
        "size": request.size.as_deref().filter(|s| !s.is_empty()).unwrap_or(default_size),
        "output_format": "png",
    })
}

/// The multipart form for `sd-server`'s `POST /v1/images/edits`: every
/// reference image as `image[]`, the optional mask, and the prompt. Without
/// a size, sd-server takes the first image's dimensions.
fn edit_form(request: &ImageEditRequest) -> AppResult<reqwest::multipart::Form> {
    let part = |img: &ImageInput| -> AppResult<reqwest::multipart::Part> {
        reqwest::multipart::Part::bytes(img.data.clone())
            .file_name(img.file_name.clone())
            .mime_str(&img.content_type)
            .map_err(|e| AppError::InvalidParams(format!("invalid image type: {e}")))
    };
    let mut form = reqwest::multipart::Form::new()
        .text("prompt", request.prompt.clone())
        .text("n", request.n.unwrap_or(1).clamp(1, 10).to_string())
        .text("output_format", "png");
    if let Some(size) = request
        .size
        .as_deref()
        .filter(|s| !s.is_empty() && *s != "auto")
    {
        form = form.text("size", size.to_string());
    }
    for img in &request.images {
        form = form.part("image[]", part(img)?);
    }
    if let Some(mask) = &request.mask {
        form = form.part("mask", part(mask)?);
    }
    Ok(form)
}

/// Map `sd-server`'s response (base64 only) to ours; `url` requests get a
/// `data:` URL.
fn map_response(body: &Value, want_url: bool) -> AppResult<ImageGenerationResponse> {
    let format = body
        .get("output_format")
        .and_then(Value::as_str)
        .unwrap_or("png");
    let data = body
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::Provider("stable-diffusion.cpp returned no images".into()))?
        .iter()
        .filter_map(|item| item.get("b64_json").and_then(Value::as_str))
        .map(|b64| GeneratedImage {
            url: want_url.then(|| format!("data:image/{format};base64,{b64}")),
            b64_json: (!want_url).then(|| b64.to_string()),
            revised_prompt: None,
        })
        .collect::<Vec<_>>();
    if data.is_empty() {
        return Err(AppError::Provider(
            "stable-diffusion.cpp returned no images".into(),
        ));
    }
    Ok(ImageGenerationResponse {
        created: body
            .get("created")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| chrono::Utc::now().timestamp()),
        data,
    })
}

pub struct SdCppEmbeddedProvider {
    instance: String,
    settings: SdSettings,
    supervisor: Arc<Supervisor>,
    client: reqwest::Client,
    /// Serializes starts so only one image model is loaded at a time.
    start_lock: tokio::sync::Mutex<()>,
}

impl SdCppEmbeddedProvider {
    pub fn new(instance: String, settings: SdSettings, supervisor: Arc<Supervisor>) -> Self {
        Self {
            instance,
            settings,
            supervisor,
            client: reqwest::Client::builder()
                .timeout(GENERATE_TIMEOUT)
                .no_proxy()
                .build()
                .unwrap_or_default(),
            start_lock: tokio::sync::Mutex::new(()),
        }
    }

    fn key_prefix(&self) -> String {
        format!("{PROVIDER_TYPE}:{}:", self.instance)
    }

    async fn command(&self) -> AppResult<EngineCommand> {
        resolve_engine(RecipeId::SdCpp, self.settings.binary_path.clone())
            .await
            .ok_or_else(|| engine_missing(PROVIDER_TYPE, "sd-server"))
    }

    fn launch_for(&self, model: &str) -> AppResult<ImageModelLaunch> {
        let backend = backend()?;
        if !backend.models().iter().any(|m| m.id == model) {
            return Err(AppError::ModelNotFound {
                model: model.to_string(),
            });
        }
        backend
            .launch(model)
            .ok_or_else(|| not_downloaded(PROVIDER_TYPE, model))
    }

    /// Start (or reuse) the engine for `model`, stopping any other image
    /// model of this provider first (image models are large).
    async fn ensure_engine(&self, model: &str) -> AppResult<(EngineHandle, ImageModelLaunch)> {
        let launch = self.launch_for(model)?;
        let command = self.command().await?;
        let spec = self
            .settings
            .launch_spec(&self.instance, model, &command, &launch);
        let _guard = self.start_lock.lock().await;
        if !self.supervisor.is_running(&spec.key) {
            let prefix = self.key_prefix();
            for p in self.supervisor.processes() {
                if p.key.starts_with(&prefix) && p.key != spec.key && p.in_flight == 0 {
                    tracing::info!("Unloading {} to load {}", p.key, spec.key);
                    self.supervisor.stop(&p.key).await;
                }
            }
        }
        let handle = self
            .supervisor
            .ensure(spec)
            .await
            .map_err(|e| engine_error(PROVIDER_TYPE, e))?;
        Ok((handle, launch))
    }
}

#[async_trait]
impl super::EmbeddedControl for SdCppEmbeddedProvider {
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

    fn catalog(&self) -> Vec<super::EmbeddedCatalogModel> {
        backend()
            .map(|b| b.models())
            .unwrap_or_default()
            .into_iter()
            .map(|m| super::EmbeddedCatalogModel {
                download_size: format_gb(m.total_bytes),
                guidance: Some(m.description),
                removable: m.downloaded,
                id: m.id,
                name: m.name,
                downloaded: m.downloaded,
                downloading: m.downloading,
                download_error: m.error,
                progress: m.progress,
            })
            .collect()
    }

    async fn download(&self, model: &str) -> AppResult<()> {
        backend()?
            .start_download(model)
            .await
            .map_err(AppError::InvalidParams)
    }

    async fn cancel_download(&self, model: &str) -> AppResult<()> {
        backend()?.cancel_download(model);
        Ok(())
    }

    async fn remove_download(&self, model: &str) -> AppResult<()> {
        self.supervisor
            .stop(&format!("{}{model}", self.key_prefix()))
            .await;
        backend()?.remove(model).map_err(AppError::InvalidParams)
    }
}

impl Drop for SdCppEmbeddedProvider {
    fn drop(&mut self) {
        let supervisor = self.supervisor.clone();
        let prefix = self.key_prefix();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move { supervisor.stop_prefix(&prefix).await });
        }
    }
}

#[async_trait]
impl ModelProvider for SdCppEmbeddedProvider {
    fn name(&self) -> &str {
        PROVIDER_TYPE
    }

    async fn health_check(&self) -> ProviderHealth {
        // Never starts the engine.
        let found = self.command().await.is_ok();
        let any = backend()
            .map(|b| b.models().iter().any(|m| m.downloaded))
            .unwrap_or(false);
        super::engine_health(
            (!found).then(|| {
                "sd-server was not found. Choose it or download it in the provider's Engine tab."
                    .to_string()
            }),
            any,
            "No image model is downloaded yet. Download one in the Models tab.",
            &self.supervisor,
            &self.key_prefix(),
        )
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        Ok(backend()
            .map(|b| b.models())
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.downloaded)
            .map(|m| ModelInfo {
                id: m.id,
                name: m.name,
                provider: PROVIDER_TYPE.to_string(),
                parameter_count: None,
                context_window: 0,
                supports_streaming: false,
                capabilities: vec![Capability::ImageGeneration],
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

    fn supports_image_generation(&self) -> bool {
        true
    }

    fn api_path_support(&self, _path: &str) -> SupportLevel {
        SupportLevel::NotSupported
    }

    fn embedded_control(&self) -> Option<&dyn super::EmbeddedControl> {
        Some(self)
    }

    fn supports_image_edits(&self) -> bool {
        true
    }

    async fn generate_image(
        &self,
        request: ImageGenerationRequest,
    ) -> AppResult<ImageGenerationResponse> {
        let (handle, launch) = self.ensure_engine(&request.model).await?;
        let _lease = handle.lease();
        let resp = self
            .client
            .post(format!("{}/v1/images/generations", handle.base_url()))
            .json(&generation_body(&request, &launch.default_size))
            .send()
            .await;
        read_images(resp, request.response_format.as_deref() == Some("url")).await
    }

    async fn edit_image(&self, request: ImageEditRequest) -> AppResult<ImageGenerationResponse> {
        if request.images.is_empty() {
            return Err(AppError::InvalidParams(
                "An image edit needs at least one image".into(),
            ));
        }
        let (handle, _launch) = self.ensure_engine(&request.model).await?;
        let _lease = handle.lease();
        let resp = self
            .client
            .post(format!("{}/v1/images/edits", handle.base_url()))
            .multipart(edit_form(&request)?)
            .send()
            .await;
        read_images(resp, request.response_format.as_deref() == Some("url")).await
    }
}

/// Turn an `sd-server` images reply into our response or error.
async fn read_images(
    resp: Result<reqwest::Response, reqwest::Error>,
    want_url: bool,
) -> AppResult<ImageGenerationResponse> {
    let resp = resp.map_err(|e| {
        AppError::Provider(format!(
            "Provider '{PROVIDER_TYPE}' is unreachable: image request failed: {e}"
        ))
    })?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| AppError::Provider(format!("failed to read the image response: {e}")))?;
    if !status.is_success() {
        let message: String = body.chars().take(500).collect();
        return Err(if status.is_client_error() {
            AppError::InvalidParams(format!(
                "stable-diffusion.cpp rejected the request: {message}"
            ))
        } else {
            AppError::Provider(format!("stable-diffusion.cpp error {status}: {message}"))
        });
    }
    let json: Value = serde_json::from_str(&body).map_err(|e| {
        AppError::Provider(format!("stable-diffusion.cpp returned invalid JSON: {e}"))
    })?;
    map_response(&json, want_url)
}

/// Factory for the stable-diffusion.cpp Local Embedded provider.
pub struct SdCppEmbeddedProviderFactory {
    supervisor: Arc<Supervisor>,
}

impl SdCppEmbeddedProviderFactory {
    pub fn new(supervisor: Arc<Supervisor>) -> Self {
        Self { supervisor }
    }
}

impl ProviderFactory for SdCppEmbeddedProviderFactory {
    fn provider_type(&self) -> &str {
        PROVIDER_TYPE
    }

    fn display_name(&self) -> &str {
        "stable-diffusion.cpp"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::Embedded
    }

    fn list_priority(&self) -> u8 {
        1
    }

    fn description(&self) -> &str {
        "Generate images locally (Qwen-Image, FLUX.2, Z-Image) with stable-diffusion.cpp. LocalRouter runs sd-server and downloads image models from Hugging Face"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::AlwaysFreeLocal
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::optional(
                "offload_to_cpu",
                ParameterType::String,
                "Keep weights in RAM and move them to the GPU as needed (uses less GPU memory): on or off",
                Some("on"),
                false,
            ),
            SetupParameter::optional(
                "flash_attention",
                ParameterType::String,
                "Flash attention in the diffusion model (faster, less memory): on or off",
                Some("on"),
                false,
            ),
            SetupParameter::optional(
                "idle_unload_minutes",
                ParameterType::Number,
                "Unload the image model after this many idle minutes (0 = keep loaded)",
                Some("15"),
                false,
            ),
            SetupParameter::optional(
                "binary_path",
                ParameterType::String,
                "Path to sd-server (leave empty to use the downloaded one or find it on PATH)",
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
        let settings = SdSettings::from_config(&config)?;
        Ok(Arc::new(SdCppEmbeddedProvider::new(
            instance_name,
            settings,
            self.supervisor.clone(),
        )))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        SdSettings::from_config(config).map(|_| ())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None
    }

    fn model_list_source(&self) -> crate::factory::ModelListSource {
        crate::factory::ModelListSource::ApiOnly
    }

    fn docs_url(&self) -> Option<&str> {
        Some("https://github.com/leejet/stable-diffusion.cpp")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_local_models::ImageRole;

    fn cfg(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn launch() -> ImageModelLaunch {
        ImageModelLaunch {
            files: vec![
                (ImageRole::Diffusion, PathBuf::from("/m/model dir/d.gguf")),
                (ImageRole::Vae, PathBuf::from("/m/vae.safetensors")),
                (ImageRole::Llm, PathBuf::from("/m/llm.gguf")),
            ],
            server_args: vec!["--cfg-scale".into(), "1.0".into()],
            default_size: "1024x1024".into(),
        }
    }

    #[test]
    fn launch_spec_binds_loopback_with_model_files() {
        let s = SdSettings::from_config(&HashMap::new()).unwrap();
        let command = EngineCommand {
            program: PathBuf::from("/bin/sd-server"),
            leading_args: vec![],
            binary: "sd-server".into(),
        };
        let spec = s.launch_spec("SD", "z-image-turbo", &command, &launch());
        assert_eq!(spec.key, "sdcpp_embedded:SD:z-image-turbo");
        assert_eq!(spec.port, PortArg::Flag("--listen-port".into()));
        let a = spec.args;
        assert_eq!(&a[..2], &["--listen-ip", "127.0.0.1"]);
        let at = |flag: &str| a.iter().position(|x| x == flag).map(|i| a[i + 1].clone());
        // Paths with spaces stay one argument.
        assert_eq!(
            at("--diffusion-model").as_deref(),
            Some("/m/model dir/d.gguf")
        );
        assert_eq!(at("--vae").as_deref(), Some("/m/vae.safetensors"));
        assert_eq!(at("--llm").as_deref(), Some("/m/llm.gguf"));
        assert_eq!(at("--cfg-scale").as_deref(), Some("1.0"));
        assert!(a.contains(&"--offload-to-cpu".to_string()));
        assert!(a.contains(&"--diffusion-fa".to_string()));

        let off = SdSettings::from_config(&cfg(&[
            ("offload_to_cpu", "off"),
            ("flash_attention", "no"),
        ]))
        .unwrap();
        let a = off.launch_spec("SD", "m", &command, &launch()).args;
        assert!(!a.contains(&"--offload-to-cpu".to_string()));
        assert!(!a.contains(&"--diffusion-fa".to_string()));
        assert!(SdSettings::from_config(&cfg(&[("offload_to_cpu", "maybe")])).is_err());
    }

    #[test]
    fn request_and_response_mapping() {
        let req = ImageGenerationRequest {
            model: "m".into(),
            prompt: "a cat".into(),
            n: Some(40),
            size: None,
            quality: None,
            style: None,
            response_format: None,
            user: None,
        };
        let body = generation_body(&req, "1024x768");
        assert_eq!(body["n"], 10);
        assert_eq!(body["size"], "1024x768");
        assert_eq!(body["output_format"], "png");

        let reply = json!({"created": 5, "output_format": "png", "data": [{"b64_json": "QUJD"}]});
        let r = map_response(&reply, false).unwrap();
        assert_eq!(r.created, 5);
        assert_eq!(r.data[0].b64_json.as_deref(), Some("QUJD"));
        let r = map_response(&reply, true).unwrap();
        assert_eq!(r.data[0].url.as_deref(), Some("data:image/png;base64,QUJD"));
        assert!(map_response(&json!({"data": []}), false).is_err());
    }
}

#[cfg(test)]
mod engine_tests {
    use super::*;
    use crate::embedded::EmbeddedControl;
    use lr_local_models::ImageRole;

    /// Two downloaded models backed by scratch files.
    struct FakeBackend {
        dir: PathBuf,
    }

    #[async_trait]
    impl ImageModelBackend for FakeBackend {
        fn models(&self) -> Vec<ImageModelStatus> {
            ["one", "two", "missing"]
                .iter()
                .map(|id| ImageModelStatus {
                    id: id.to_string(),
                    name: id.to_string(),
                    description: String::new(),
                    total_bytes: 1,
                    downloaded: *id != "missing",
                    downloading: false,
                    progress: None,
                    error: None,
                })
                .collect()
        }
        fn launch(&self, id: &str) -> Option<ImageModelLaunch> {
            (id != "missing").then(|| ImageModelLaunch {
                files: vec![(ImageRole::Diffusion, self.dir.join(format!("{id}.gguf")))],
                server_args: vec!["--steps".into(), "4".into()],
                default_size: "512x512".into(),
            })
        }
        async fn start_download(&self, _id: &str) -> Result<(), String> {
            Ok(())
        }
        fn cancel_download(&self, _id: &str) {}
        fn remove(&self, _id: &str) -> Result<(), String> {
            Ok(())
        }
    }

    fn request(model: &str) -> ImageGenerationRequest {
        ImageGenerationRequest {
            model: model.into(),
            prompt: "a lovely cat".into(),
            n: Some(2),
            size: None,
            quality: None,
            style: None,
            response_format: None,
            user: None,
        }
    }

    #[tokio::test]
    async fn generates_through_sd_server_and_keeps_one_model_loaded() {
        let Some(fake) = crate::embedded::fake_engine_path() else {
            eprintln!("skipping: lr-fake-engine not built (run the workspace tests)");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        set_image_model_backend(Arc::new(FakeBackend {
            dir: dir.path().to_path_buf(),
        }));
        let supervisor = Supervisor::new(dir.path());
        let settings = SdSettings::from_config(
            &[("binary_path".to_string(), fake.display().to_string())]
                .into_iter()
                .collect(),
        )
        .unwrap();
        let p = SdCppEmbeddedProvider::new("SD".into(), settings, supervisor.clone());

        assert_eq!(
            p.list_models()
                .await
                .unwrap()
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["one", "two"]
        );
        assert!(matches!(
            p.generate_image(request("missing")).await,
            Err(AppError::InvalidParams(m)) if m.contains("not downloaded")
        ));
        assert!(matches!(
            p.generate_image(request("nope")).await,
            Err(AppError::ModelNotFound { .. })
        ));

        let resp = p.generate_image(request("one")).await.unwrap();
        assert_eq!(resp.data.len(), 2);
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(resp.data[0].b64_json.as_ref().unwrap())
            .unwrap();
        assert_eq!(bytes, b"a lovely cat");

        // The launch carried the model files and the family defaults.
        let port = supervisor
            .processes()
            .into_iter()
            .find(|x| x.key == "sdcpp_embedded:SD:one")
            .and_then(|x| x.port)
            .unwrap();
        let args: Vec<String> = reqwest::get(format!("http://127.0.0.1:{port}/fake/args"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let at = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .map(|i| args[i + 1].clone())
        };
        assert_eq!(at("--listen-ip").as_deref(), Some("127.0.0.1"));
        assert_eq!(
            at("--diffusion-model"),
            Some(dir.path().join("one.gguf").display().to_string())
        );
        assert_eq!(at("--steps").as_deref(), Some("4"));

        // An edit sends the images, the mask and the prompt as multipart.
        let img = |name: &str| crate::ImageInput {
            data: b"png".to_vec(),
            file_name: name.into(),
            content_type: "image/png".into(),
        };
        let edited = p
            .edit_image(crate::ImageEditRequest {
                model: "one".into(),
                prompt: "make it blue".into(),
                images: vec![img("a.png"), img("b.png")],
                mask: Some(img("mask.png")),
                n: Some(1),
                size: Some("768x512".into()),
                response_format: Some("url".into()),
                user: None,
            })
            .await
            .unwrap();
        let url = edited.data[0].url.as_deref().unwrap();
        let b64 = url.strip_prefix("data:image/png;base64,").unwrap();
        let summary = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        assert_eq!(
            String::from_utf8(summary).unwrap(),
            "make it blue|images=2|mask=yes|size=768x512"
        );
        assert!(matches!(
            p.edit_image(crate::ImageEditRequest {
                model: "one".into(),
                prompt: "x".into(),
                images: vec![],
                mask: None,
                n: None,
                size: None,
                response_format: None,
                user: None,
            })
            .await,
            Err(AppError::InvalidParams(_))
        ));

        // Loading another image model unloads the first.
        p.load("two").await.unwrap();
        assert!(supervisor.is_running("sdcpp_embedded:SD:two"));
        assert!(!supervisor.is_running("sdcpp_embedded:SD:one"));
        let states: Vec<String> = p.model_states().into_iter().map(|s| s.model).collect();
        assert!(states.contains(&"two".to_string()));
        supervisor.stop_all().await;
    }
}
