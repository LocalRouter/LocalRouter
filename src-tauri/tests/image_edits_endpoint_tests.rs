//! End-to-end tests for `POST /v1/images/edits` through the real server:
//! multipart parsing (several images, mask, uploads above axum's 2 MB
//! default), validation, provider capability checks and response formats.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;
use localrouter::clients::{ClientManager, TokenStore};
use localrouter::config::{AppConfig, Client, ConfigManager, Strategy};
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::factory::{ProviderCategory, ProviderFactory, SetupParameter};
use localrouter::providers::registry::ProviderRegistry;
use localrouter::providers::{
    CompletionChunk, CompletionRequest, CompletionResponse, GeneratedImage, HealthStatus,
    ImageEditRequest, ImageGenerationResponse, ModelInfo, ModelProvider, PricingInfo,
    ProviderHealth,
};
use localrouter::router::{RateLimiterManager, Router};
use localrouter::server;
use lr_types::{AppError, AppResult};
use serde_json::Value;
use tokio::time::{sleep, Duration};

/// Answers edits with a description of what it received.
struct EchoEditor {
    edits: bool,
}

#[async_trait]
impl ModelProvider for EchoEditor {
    fn name(&self) -> &str {
        "echo"
    }
    async fn health_check(&self) -> ProviderHealth {
        ProviderHealth {
            status: HealthStatus::Healthy,
            latency_ms: None,
            last_checked: chrono::Utc::now(),
            error_message: None,
        }
    }
    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        Ok(vec![])
    }
    async fn get_pricing(&self, _model: &str) -> AppResult<PricingInfo> {
        Ok(PricingInfo::free())
    }
    async fn complete(&self, _r: CompletionRequest) -> AppResult<CompletionResponse> {
        Err(AppError::Provider("no chat".into()))
    }
    async fn stream_complete(
        &self,
        _r: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        Err(AppError::Provider("no chat".into()))
    }
    fn supports_image_edits(&self) -> bool {
        self.edits
    }
    async fn edit_image(&self, r: ImageEditRequest) -> AppResult<ImageGenerationResponse> {
        let summary = format!(
            "model={} prompt={} images={} sizes={} mask={} n={} size={}",
            r.model,
            r.prompt,
            r.images.len(),
            r.images
                .iter()
                .map(|i| format!("{}:{}:{}", i.file_name, i.content_type, i.data.len()))
                .collect::<Vec<_>>()
                .join(","),
            r.mask.as_ref().map(|m| m.data.len()).unwrap_or(0),
            r.n.unwrap_or(1),
            r.size.as_deref().unwrap_or("-"),
        );
        let url = r.response_format.as_deref() == Some("url");
        Ok(ImageGenerationResponse {
            created: 7,
            data: vec![GeneratedImage {
                url: url.then(|| format!("data:text/plain,{summary}")),
                b64_json: (!url).then(|| summary.clone()),
                revised_prompt: None,
            }],
        })
    }
}

struct EchoFactory;

impl ProviderFactory for EchoFactory {
    fn provider_type(&self) -> &str {
        "echo"
    }
    fn display_name(&self) -> &str {
        "Echo"
    }
    fn category(&self) -> ProviderCategory {
        ProviderCategory::Generic
    }
    fn description(&self) -> &str {
        "test"
    }
    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![]
    }
    fn create(
        &self,
        _name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        Ok(Arc::new(EchoEditor {
            edits: config.get("edits").map(String::as_str) != Some("no"),
        }))
    }
    fn validate_config(&self, _config: &HashMap<String, String>) -> AppResult<()> {
        Ok(())
    }
}

async fn start_server() -> (String, String) {
    let mut client = Client::new_with_strategy("Test".to_string(), "default".to_string());
    client.id = "test-api-key".to_string();
    let config = AppConfig {
        clients: vec![client.clone()],
        strategies: vec![Strategy::new("Default".to_string())],
        ..Default::default()
    };
    let config_manager = Arc::new(ConfigManager::new(
        config,
        std::env::temp_dir().join(format!("test_edits_{}.yaml", uuid::Uuid::new_v4())),
    ));
    let registry = Arc::new(ProviderRegistry::new());
    registry.register_factory(Arc::new(EchoFactory));
    registry
        .create_provider("editor".into(), "echo".into(), HashMap::new())
        .await
        .unwrap();
    let mut no = HashMap::new();
    no.insert("edits".to_string(), "no".to_string());
    registry
        .create_provider("plain".into(), "echo".into(), no)
        .await
        .unwrap();
    let metrics = Arc::new(MetricsCollector::new(Arc::new(
        MetricsDatabase::new(
            std::env::temp_dir().join(format!("test_edits_{}.db", uuid::Uuid::new_v4())),
        )
        .unwrap(),
    )));
    let rate_limiter = Arc::new(RateLimiterManager::new(None));
    let router = Arc::new(Router::new(
        config_manager.clone(),
        registry.clone(),
        rate_limiter.clone(),
        metrics.clone(),
        Arc::new(lr_router::FreeTierManager::new(None)),
    ));
    let (state, _handle, port, _shutdown) = server::start_server(
        server::ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 44000 + (std::process::id() % 10000) as u16,
            enable_cors: true,
        },
        router,
        Arc::new(McpServerManager::new()),
        rate_limiter,
        registry,
        config_manager,
        Arc::new(ClientManager::new(vec![client])),
        Arc::new(TokenStore::new()),
        metrics,
        None,
    )
    .await
    .expect("server");
    sleep(Duration::from_millis(200)).await;
    (
        format!("http://127.0.0.1:{port}"),
        state.get_internal_test_secret(),
    )
}

fn png(bytes: usize, name: &str) -> reqwest::multipart::Part {
    reqwest::multipart::Part::bytes(vec![7u8; bytes])
        .file_name(name.to_string())
        .mime_str("image/png")
        .unwrap()
}

async fn post(base: &str, key: &str, path: &str, form: reqwest::multipart::Form) -> (u16, Value) {
    let resp = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .bearer_auth(key)
        .multipart(form)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    (status, resp.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn image_edits_end_to_end() {
    let (base, key) = start_server().await;

    // Two images (one above axum's 2 MB multipart default), a mask, options.
    let form = reqwest::multipart::Form::new()
        .text("model", "editor/model-a")
        .text("prompt", "make it blue")
        .part("image[]", png(3 * 1024 * 1024, "a.png"))
        .part("image[]", png(10, "b.png"))
        .part("mask", png(5, "mask.png"))
        .text("n", "2")
        .text("size", "1024x768")
        .text("quality", "high");
    let (status, body) = post(&base, &key, "/v1/images/edits", form).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["data"][0]["b64_json"],
        "model=model-a prompt=make it blue images=2 sizes=a.png:image/png:3145728,b.png:image/png:10 mask=5 n=2 size=1024x768"
    );

    // Unprefixed path, single legacy `image` field, url format.
    let form = reqwest::multipart::Form::new()
        .text("model", "editor/m")
        .text("prompt", "p")
        .part("image", png(4, "x.png"))
        .text("response_format", "url");
    let (status, body) = post(&base, &key, "/images/edits", form).await;
    assert_eq!(status, 200, "{body}");
    assert!(body["data"][0]["url"]
        .as_str()
        .unwrap()
        .starts_with("data:"));

    // Validation.
    let cases = [
        (
            reqwest::multipart::Form::new()
                .text("model", "editor/m")
                .text("prompt", "p"),
            "image",
        ),
        (
            reqwest::multipart::Form::new()
                .text("prompt", "p")
                .part("image", png(4, "x.png")),
            "model",
        ),
        (
            reqwest::multipart::Form::new()
                .text("model", "editor/m")
                .text("prompt", "p")
                .text("size", "huge")
                .part("image", png(4, "x.png")),
            "size",
        ),
        (
            reqwest::multipart::Form::new()
                .text("model", "editor/m")
                .text("prompt", "p")
                .text("n", "11")
                .part("image", png(4, "x.png")),
            "n",
        ),
    ];
    for (form, param) in cases {
        let (status, body) = post(&base, &key, "/v1/images/edits", form).await;
        assert_eq!(status, 400, "{param}: {body}");
        assert_eq!(body["error"]["param"], param, "{body}");
    }

    // A provider without edits.
    let form = reqwest::multipart::Form::new()
        .text("model", "plain/m")
        .text("prompt", "p")
        .part("image", png(4, "x.png"));
    let (status, body) = post(&base, &key, "/v1/images/edits", form).await;
    assert_eq!(status, 400);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("does not support image edits"));

    // Authentication is required.
    let form = reqwest::multipart::Form::new()
        .text("model", "editor/m")
        .text("prompt", "p")
        .part("image", png(4, "x.png"));
    let (status, _) = post(&base, "wrong-key", "/v1/images/edits", form).await;
    assert_eq!(status, 401);

    // OpenAPI documents the route.
    let spec: Value = reqwest::get(format!("{base}/openapi.json"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(spec["paths"]["/v1/images/edits"]["post"].is_object());
    assert!(spec["paths"]["/v1/images/generations"]["post"].is_object());
}
