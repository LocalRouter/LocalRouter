//! Provider for servers that speak the System One protocol natively.
//!
//! One implementation covers four flavors that differ only in defaults,
//! model discovery and health probing:
//! - TypeSafe (hosted Jev, `https://api.typesafe.ai`)
//! - Laya (`laya-serve`, default `http://localhost:8000`)
//! - Kev (`python -m kev.serve`, default `http://127.0.0.1:8009`)
//! - Generic (any other Jev-compatible server at a user-supplied URL)

use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use reqwest_middleware::ClientWithMiddleware;
use serde_json::Value;
use std::pin::Pin;
use std::time::Instant;
use tracing::debug;

use super::types::{SystemOneRequest, SystemOneResponse};
use crate::{
    Capability, CompletionChunk, CompletionRequest, CompletionResponse, HealthStatus, ModelInfo,
    ModelProvider, PricingInfo, ProviderHealth, SupportLevel,
};
use lr_types::{AppError, AppResult};

/// Response header TypeSafe uses for request ids (Kev mirrors it).
pub const TYPESAFE_REQUEST_ID_HEADER: &str = "x-typesafe-request-id";

/// TypeSafe input price: $0.042 per million tokens.
pub const TYPESAFE_INPUT_COST_PER_1K: f64 = 0.000_042;

/// Which System One server family a provider instance talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemOneFlavor {
    TypeSafe,
    Laya,
    Kev,
    Generic,
}

impl SystemOneFlavor {
    /// Provider type identifier, also returned by `ModelProvider::name`.
    pub fn provider_type(&self) -> &'static str {
        match self {
            SystemOneFlavor::TypeSafe => "typesafe",
            SystemOneFlavor::Laya => "laya",
            SystemOneFlavor::Kev => "kev",
            SystemOneFlavor::Generic => "systemone_compatible",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            SystemOneFlavor::TypeSafe => "TypeSafe (Jev)",
            SystemOneFlavor::Laya => "Laya",
            SystemOneFlavor::Kev => "Kev",
            SystemOneFlavor::Generic => "System One compatible",
        }
    }

    /// Default base URL, without a trailing `/v1`.
    pub fn default_base_url(&self) -> Option<&'static str> {
        match self {
            SystemOneFlavor::TypeSafe => Some("https://api.typesafe.ai"),
            SystemOneFlavor::Laya => Some("http://localhost:8000"),
            SystemOneFlavor::Kev => Some("http://127.0.0.1:8009"),
            SystemOneFlavor::Generic => None,
        }
    }

    /// Model to send when the client did not name one. `None` means "omit
    /// the field" (laya-serve then routes by language on its own).
    pub fn default_model(&self) -> Option<&'static str> {
        match self {
            SystemOneFlavor::TypeSafe => Some("jev-latest"),
            SystemOneFlavor::Kev => Some("kev-latest"),
            SystemOneFlavor::Laya | SystemOneFlavor::Generic => None,
        }
    }

    /// Models known without asking the server.
    pub fn static_models(&self) -> Vec<(&'static str, &'static str, u32)> {
        match self {
            SystemOneFlavor::TypeSafe => vec![
                ("jev-latest", "Jev (latest)", 65_536),
                ("jev-preview", "Jev (preview)", 65_536),
                ("jev-1.13.0", "Jev 1.13.0", 65_536),
            ],
            SystemOneFlavor::Laya => vec![
                ("english", "Laya English (ModernBERT-large)", 512),
                ("multilingual", "Laya Multilingual (mmBERT-base)", 1_024),
                ("typed-decisions", "Laya Typed Decisions", 1_024),
            ],
            SystemOneFlavor::Kev => vec![("kev-latest", "Kev (loaded checkpoint)", 4_096)],
            SystemOneFlavor::Generic => vec![],
        }
    }
}

/// Provider for native System One servers.
pub struct SystemOneProvider {
    flavor: SystemOneFlavor,
    base_url: String,
    api_key: Option<String>,
    client: ClientWithMiddleware,
}

impl SystemOneProvider {
    /// Create a provider. `base_url` falls back to the flavor default; a
    /// trailing `/` or `/v1` is stripped so both `http://host:8000` and
    /// `http://host:8000/v1` work.
    pub fn new(
        flavor: SystemOneFlavor,
        base_url: Option<String>,
        api_key: Option<String>,
    ) -> AppResult<Self> {
        let base = base_url
            .filter(|u| !u.trim().is_empty())
            .or_else(|| flavor.default_base_url().map(str::to_string))
            .ok_or_else(|| {
                AppError::Config(format!("{} requires a base_url", flavor.display_name()))
            })?;
        Ok(Self {
            flavor,
            base_url: normalize_base_url(&base),
            api_key: api_key.filter(|k| !k.trim().is_empty()),
            client: crate::http_client::extended_client()?,
        })
    }

    pub fn flavor(&self) -> SystemOneFlavor {
        self.flavor
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn with_auth(
        &self,
        rb: reqwest_middleware::RequestBuilder,
    ) -> reqwest_middleware::RequestBuilder {
        match &self.api_key {
            Some(key) => rb.bearer_auth(key),
            None => rb,
        }
    }

    fn model_info(&self, id: &str, name: &str, context_window: u32) -> ModelInfo {
        ModelInfo {
            id: id.to_string(),
            name: name.to_string(),
            provider: self.flavor.provider_type().to_string(),
            parameter_count: None,
            context_window,
            supports_streaming: false,
            capabilities: vec![Capability::Decision],
            detailed_capabilities: None,
        }
    }

    fn static_model_infos(&self) -> Vec<ModelInfo> {
        self.flavor
            .static_models()
            .into_iter()
            .map(|(id, name, ctx)| self.model_info(id, name, ctx))
            .collect()
    }

    async fn fetch_models(&self) -> AppResult<Vec<ModelInfo>> {
        let url = format!("{}/v1/models", self.base_url);
        let resp = self
            .with_auth(self.client.get(&url))
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("connection to {url} failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::Provider(format!(
                "{} returned {} for /v1/models",
                self.flavor.display_name(),
                resp.status()
            )));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("invalid /v1/models response: {e}")))?;
        Ok(parse_model_ids(&body)
            .into_iter()
            .map(|id| {
                let ctx = self
                    .flavor
                    .static_models()
                    .into_iter()
                    .find(|(sid, _, _)| *sid == id)
                    .map(|(_, _, c)| c)
                    .unwrap_or(4_096);
                self.model_info(&id, &id, ctx)
            })
            .collect())
    }

    async fn probe(&self, rb: reqwest_middleware::RequestBuilder) -> ProviderHealth {
        let start = Instant::now();
        let result = self.with_auth(rb).send().await;
        let latency_ms = start.elapsed().as_millis() as u64;
        let (status, error_message, latency_ms) = match result {
            Ok(resp) => {
                let (status, msg) = health_from_status(self.flavor, resp.status().as_u16());
                (status, msg, Some(latency_ms))
            }
            Err(e) => (
                HealthStatus::Unhealthy,
                Some(format!(
                    "Failed to connect to {}: {}",
                    self.flavor.display_name(),
                    e
                )),
                None,
            ),
        };
        ProviderHealth {
            latency_ms,
            status,
            last_checked: Utc::now(),
            error_message,
        }
    }
}

/// Strip a trailing `/` and `/v1` from a base URL.
pub fn normalize_base_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    trimmed
        .strip_suffix("/v1")
        .unwrap_or(trimmed)
        .trim_end_matches('/')
        .to_string()
}

/// Extract model ids from the model-list shapes used by System One servers:
/// OpenAI style `{"data": [{"id"}]}`, `{"models": [{"name"|"id"}]}`, or a
/// top-level array of objects or strings.
pub fn parse_model_ids(body: &Value) -> Vec<String> {
    let list = body
        .get("data")
        .or_else(|| body.get("models"))
        .unwrap_or(body)
        .as_array();
    let Some(list) = list else {
        return vec![];
    };
    let mut ids: Vec<String> = Vec::new();
    for item in list {
        let id = match item {
            Value::String(s) => Some(s.clone()),
            Value::Object(o) => o
                .get("id")
                .or_else(|| o.get("name"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            _ => None,
        };
        if let Some(id) = id {
            if !id.is_empty() && !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

/// Map a probe's HTTP status to a health status for the given flavor.
///
/// TypeSafe is probed with an empty `POST /v1/systemone`: a validation error
/// (400/422) proves the host is up and the key was accepted, without spending
/// any tokens.
pub fn health_from_status(flavor: SystemOneFlavor, status: u16) -> (HealthStatus, Option<String>) {
    match status {
        200..=299 => (HealthStatus::Healthy, None),
        400 | 422 if flavor == SystemOneFlavor::TypeSafe => (HealthStatus::Healthy, None),
        401 | 403 => (
            HealthStatus::Unhealthy,
            Some(format!(
                "{} rejected the API key ({status})",
                flavor.display_name()
            )),
        ),
        429 => (
            HealthStatus::Degraded,
            Some(format!(
                "{} is rate limiting requests",
                flavor.display_name()
            )),
        ),
        _ => (
            HealthStatus::Unhealthy,
            Some(format!(
                "{} returned status {status}",
                flavor.display_name()
            )),
        ),
    }
}

/// Map a non-success System One response to an `AppError`.
///
/// Client errors other than auth and rate limiting keep the upstream body
/// verbatim in `ProviderStatus.message`, so the gateway can return it to the
/// caller unchanged (SDKs parse TypeSafe's 422 detail).
pub fn map_systemone_error(status: u16, body: String) -> AppError {
    match status {
        401 | 403 => AppError::Unauthorized,
        429 => AppError::RateLimitExceeded,
        400..=499 => AppError::ProviderStatus {
            status,
            message: body,
        },
        _ => AppError::Provider(format!("API error ({status}): {body}")),
    }
}

#[async_trait]
impl ModelProvider for SystemOneProvider {
    fn name(&self) -> &str {
        self.flavor.provider_type()
    }

    async fn health_check(&self) -> ProviderHealth {
        match self.flavor {
            SystemOneFlavor::TypeSafe => {
                self.probe(
                    self.client
                        .post(format!("{}/v1/systemone", self.base_url))
                        .json(&serde_json::json!({})),
                )
                .await
            }
            SystemOneFlavor::Laya => {
                self.probe(self.client.get(format!("{}/health", self.base_url)))
                    .await
            }
            SystemOneFlavor::Kev | SystemOneFlavor::Generic => {
                self.probe(self.client.get(format!("{}/v1/models", self.base_url)))
                    .await
            }
        }
    }

    fn health_check_interval_multiplier(&self) -> u32 {
        // The hosted API rate limits; probe it every sixth cycle.
        if self.flavor == SystemOneFlavor::TypeSafe {
            6
        } else {
            1
        }
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        if self.flavor == SystemOneFlavor::TypeSafe {
            // TypeSafe has no model-list endpoint.
            return Ok(self.static_model_infos());
        }
        let mut models = match self.fetch_models().await {
            Ok(models) if !models.is_empty() => models,
            Ok(_) => self.static_model_infos(),
            Err(e) => {
                debug!(
                    "{}: model list unavailable ({}), using known models",
                    self.flavor.display_name(),
                    e
                );
                if self.flavor == SystemOneFlavor::Generic {
                    return Err(e);
                }
                self.static_model_infos()
            }
        };
        // Kev serves one checkpoint and accepts `kev-latest` for it.
        if self.flavor == SystemOneFlavor::Kev && !models.iter().any(|m| m.id == "kev-latest") {
            models.insert(
                0,
                self.model_info("kev-latest", "Kev (loaded checkpoint)", 4_096),
            );
        }
        Ok(models)
    }

    async fn get_pricing(&self, _model: &str) -> AppResult<PricingInfo> {
        Ok(match self.flavor {
            SystemOneFlavor::TypeSafe => PricingInfo {
                input_cost_per_1k: TYPESAFE_INPUT_COST_PER_1K,
                output_cost_per_1k: 0.0,
                reasoning_cost_per_1k: None,
                cache_read_cost_per_1k: None,
                cache_write_cost_per_1k: None,
                currency: "USD".to_string(),
            },
            _ => PricingInfo::free(),
        })
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

    async fn systemone(&self, mut request: SystemOneRequest) -> AppResult<SystemOneResponse> {
        if request.model.is_none() {
            request.model = self.flavor.default_model().map(str::to_string);
        }
        let url = format!("{}/v1/systemone", self.base_url);
        let resp = self
            .with_auth(self.client.post(&url).json(&request))
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("connection to {url} failed: {e}")))?;

        let status = resp.status().as_u16();
        let request_id = resp
            .headers()
            .get(TYPESAFE_REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = resp
            .text()
            .await
            .map_err(|e| AppError::Provider(format!("failed to read response: {e}")))?;

        if !(200..300).contains(&status) {
            return Err(map_systemone_error(status, body));
        }

        let mut parsed: SystemOneResponse = serde_json::from_str(&body).map_err(|e| {
            AppError::Provider(format!(
                "{} returned an invalid System One response: {e}",
                self.flavor.display_name()
            ))
        })?;
        parsed.request_id = request_id;
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn base_url_normalization() {
        assert_eq!(normalize_base_url("http://h:8000/"), "http://h:8000");
        assert_eq!(normalize_base_url("http://h:8000/v1"), "http://h:8000");
        assert_eq!(normalize_base_url("http://h:8000/v1/"), "http://h:8000");
        assert_eq!(
            normalize_base_url("https://jev-agent.com/api"),
            "https://jev-agent.com/api"
        );
    }

    #[test]
    fn defaults_per_flavor() {
        let laya = SystemOneProvider::new(SystemOneFlavor::Laya, None, None).unwrap();
        assert_eq!(laya.base_url(), "http://localhost:8000");
        let kev = SystemOneProvider::new(SystemOneFlavor::Kev, Some("".into()), None).unwrap();
        assert_eq!(kev.base_url(), "http://127.0.0.1:8009");
        assert!(SystemOneProvider::new(SystemOneFlavor::Generic, None, None).is_err());
        assert_eq!(
            SystemOneFlavor::TypeSafe.default_model(),
            Some("jev-latest")
        );
        assert_eq!(SystemOneFlavor::Laya.default_model(), None);
    }

    #[test]
    fn model_id_parsing_shapes() {
        assert_eq!(
            parse_model_ids(&json!({"data": [{"id": "a"}, {"id": "b"}]})),
            vec!["a", "b"]
        );
        assert_eq!(
            parse_model_ids(&json!({"models": [{"name": "english"}, {"id": "multilingual"}]})),
            vec!["english", "multilingual"]
        );
        assert_eq!(
            parse_model_ids(&json!(["x", {"name": "y"}, "x"])),
            vec!["x", "y"]
        );
        assert!(parse_model_ids(&json!({"status": "ok"})).is_empty());
        assert!(parse_model_ids(&json!("garbage")).is_empty());
    }

    #[test]
    fn health_status_mapping() {
        use HealthStatus::*;
        assert_eq!(
            health_from_status(SystemOneFlavor::TypeSafe, 422).0,
            Healthy
        );
        assert_eq!(
            health_from_status(SystemOneFlavor::TypeSafe, 400).0,
            Healthy
        );
        assert_eq!(health_from_status(SystemOneFlavor::Laya, 422).0, Unhealthy);
        assert_eq!(
            health_from_status(SystemOneFlavor::TypeSafe, 401).0,
            Unhealthy
        );
        assert_eq!(health_from_status(SystemOneFlavor::Kev, 429).0, Degraded);
        assert_eq!(health_from_status(SystemOneFlavor::Kev, 200).0, Healthy);
        assert_eq!(health_from_status(SystemOneFlavor::Kev, 503).0, Unhealthy);
    }

    #[test]
    fn error_mapping_keeps_client_error_body() {
        assert!(matches!(
            map_systemone_error(401, String::new()),
            AppError::Unauthorized
        ));
        assert!(matches!(
            map_systemone_error(429, String::new()),
            AppError::RateLimitExceeded
        ));
        match map_systemone_error(422, r#"{"detail":[{"msg":"bad"}]}"#.into()) {
            AppError::ProviderStatus { status, message } => {
                assert_eq!(status, 422);
                assert_eq!(message, r#"{"detail":[{"msg":"bad"}]}"#);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            map_systemone_error(529, "overloaded".into()),
            AppError::Provider(_)
        ));
    }

    #[tokio::test]
    async fn pricing_per_flavor() {
        let ts = SystemOneProvider::new(SystemOneFlavor::TypeSafe, None, Some("k".into())).unwrap();
        let p = ts.get_pricing("jev-latest").await.unwrap();
        assert_eq!(p.input_cost_per_1k, TYPESAFE_INPUT_COST_PER_1K);
        assert_eq!(p.output_cost_per_1k, 0.0);
        let laya = SystemOneProvider::new(SystemOneFlavor::Laya, None, None).unwrap();
        assert_eq!(
            laya.get_pricing("english").await.unwrap().input_cost_per_1k,
            0.0
        );
    }

    #[test]
    fn feature_support_reflects_decision_only_provider() {
        let laya = SystemOneProvider::new(SystemOneFlavor::Laya, None, None).unwrap();
        let fs = laya.get_feature_support("laya");
        let endpoint = |name: &str| {
            fs.endpoints
                .iter()
                .find(|e| e.name == name)
                .map(|e| e.support.clone())
                .unwrap()
        };
        assert_eq!(endpoint("Chat Completions"), SupportLevel::NotSupported);
        assert_eq!(endpoint("Streaming"), SupportLevel::NotSupported);
        assert_eq!(endpoint("System One Decisions"), SupportLevel::Supported);
        let feature = |name: &str| {
            fs.optimization_features
                .iter()
                .find(|f| f.name == name)
                .map(|f| f.support.clone())
                .unwrap()
        };
        assert_eq!(feature("Guardrails"), SupportLevel::Supported);
        assert_eq!(feature("Secret Scanning"), SupportLevel::Supported);
        assert_eq!(feature("Prompt Compression"), SupportLevel::Supported);
        assert_eq!(feature("JSON Repair"), SupportLevel::NotSupported);
        assert_eq!(feature("Decision Routing"), SupportLevel::NotSupported);
    }

    #[tokio::test]
    async fn typesafe_lists_static_models_and_no_chat() {
        let ts = SystemOneProvider::new(SystemOneFlavor::TypeSafe, None, Some("k".into())).unwrap();
        let models = ts.list_models().await.unwrap();
        assert!(models.iter().any(|m| m.id == "jev-latest"));
        assert!(models
            .iter()
            .all(|m| m.capabilities == vec![Capability::Decision]));
        assert!(!ts.supports_chat());
        assert!(ts.supports_systemone());
        let err = ts
            .complete(CompletionRequest::new("jev-latest", vec![]))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not support chat completions"));
    }
}
