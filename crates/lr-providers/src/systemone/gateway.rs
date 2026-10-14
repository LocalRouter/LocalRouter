//! System One on multi-model gateways (OpenRouter, LLM Gateway, Vercel AI
//! Gateway, Cloudflare Workers AI).
//!
//! These providers serve chat models and, alongside them, hosted decision
//! models (TypeSafe Jev). A [`SystemOneGateway`] attached to such a provider
//! knows which of its models are decision models (from the gateway's own
//! model listing, never from model names), answers System One requests for
//! them, and supplies their pricing. The provider's other models answer
//! System One questions through the chat translation layer.

use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};
use reqwest_middleware::ClientWithMiddleware;
use serde_json::{json, Value};
use tracing::debug;

use super::provider::{map_systemone_error, TYPESAFE_REQUEST_ID_HEADER};
use super::types::{SystemOneRequest, SystemOneResponse};
use crate::{Capability, ModelInfo, PricingInfo};
use lr_types::{AppError, AppResult};

/// How long a fetched model listing is trusted.
const LISTING_TTL: Duration = Duration::from_secs(10 * 60);
/// How long to wait before retrying a failed listing fetch.
const LISTING_RETRY: Duration = Duration::from_secs(60);

/// Request/response envelope the gateway uses for System One.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayWire {
    /// OpenAI Decisions arrays, including typed values and refusals.
    Decisions,
    VercelDecisions,
    OpenRouterDecisions,
    /// The TypeSafe body and response, unchanged.
    Plain,
    /// Cloudflare `POST /ai/run`: `{"model", "input": {state, questions}}`
    /// in, `{"result": {"state": "Completed", "result": <payload>},
    /// "success": true}` out.
    CloudflareRun,
}

/// How the gateway tells decision models apart from its other models.
#[derive(Debug, Clone)]
pub enum DecisionDiscovery {
    /// `GET url`; decision models list `modality` in
    /// `architecture.output_modalities` (OpenRouter: `decisions`, LLM
    /// Gateway: `decision`).
    OutputModality { url: String, modality: &'static str },
    /// `GET url`; decision models have `"type": model_type` (Vercel:
    /// `evaluation`).
    ModelType {
        url: String,
        model_type: &'static str,
    },
    /// No listing covers the decision models (Cloudflare partner models).
    Static(Vec<ListedModel>),
}

/// One model from a gateway listing.
#[derive(Debug, Clone)]
pub struct ListedModel {
    pub id: String,
    pub name: String,
    pub context_window: Option<u32>,
    pub pricing: Option<PricingInfo>,
    pub decision: bool,
    pub image_input: bool,
}

impl ListedModel {
    pub fn decision(id: &str, name: &str, context_window: u32, input_per_1k: f64) -> Self {
        Self {
            image_input: false,
            id: id.to_string(),
            name: name.to_string(),
            context_window: Some(context_window),
            pricing: Some(PricingInfo {
                input_cost_per_1k: input_per_1k,
                output_cost_per_1k: 0.0,
                reasoning_cost_per_1k: None,
                cache_read_cost_per_1k: None,
                cache_write_cost_per_1k: None,
                currency: "USD".to_string(),
            }),
            decision: true,
        }
    }
}

#[derive(Default)]
struct ListingCache {
    models: Option<Arc<Vec<ListedModel>>>,
    fetched_at: Option<Instant>,
    failed_at: Option<Instant>,
}

/// System One support for one gateway provider instance.
pub struct SystemOneGateway {
    display_name: &'static str,
    systemone_url: String,
    wire: GatewayWire,
    discovery: DecisionDiscovery,
    default_model: String,
    headers: HeaderMap,
    client: ClientWithMiddleware,
    cache: tokio::sync::Mutex<ListingCache>,
    gateway_preferences: crate::gateway::GatewayPreferences,
}

impl SystemOneGateway {
    /// `api_key` is sent as a Bearer token; `extra_headers` go with every
    /// request (e.g. OpenRouter's app attribution headers).
    pub fn new(
        display_name: &'static str,
        systemone_url: String,
        wire: GatewayWire,
        discovery: DecisionDiscovery,
        default_model: &str,
        api_key: Option<&str>,
        extra_headers: HeaderMap,
    ) -> AppResult<Self> {
        let mut headers = extra_headers;
        if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
            let value = HeaderValue::from_str(&format!("Bearer {}", key.trim()))
                .map_err(|_| AppError::Config("API key contains invalid characters".into()))?;
            headers.insert(AUTHORIZATION, value);
        }
        Ok(Self {
            display_name,
            systemone_url,
            wire,
            discovery,
            default_model: default_model.to_string(),
            headers,
            client: crate::http_client::extended_client()?,
            cache: tokio::sync::Mutex::new(ListingCache::default()),
            gateway_preferences: Default::default(),
        })
    }

    /// OpenRouter: alpha Decisions accepts System One maps; decision models are listed only
    /// by `GET {base}/models?output_modalities=decisions`.
    pub fn openrouter(base_url: &str, api_key: &str, extra_headers: HeaderMap) -> AppResult<Self> {
        let base = base_url.trim_end_matches('/');
        Self::new(
            "OpenRouter",
            format!(
                "{}/alpha/decisions",
                base.strip_suffix("/v1").unwrap_or(base)
            ),
            GatewayWire::OpenRouterDecisions,
            DecisionDiscovery::OutputModality {
                url: format!("{base}/models?output_modalities=decisions"),
                modality: "decisions",
            },
            "~typesafe/jev-latest",
            Some(api_key),
            extra_headers,
        )
    }

    /// LLM Gateway: `POST {base}/decisions`; decision models list
    /// `decision` among their output modalities in `GET {base}/models`.
    pub fn llmgateway(base_url: &str, api_key: &str) -> AppResult<Self> {
        let base = base_url.trim_end_matches('/');
        Self::new(
            "LLM Gateway",
            format!("{base}/decisions"),
            GatewayWire::Decisions,
            DecisionDiscovery::OutputModality {
                url: format!("{base}/models"),
                modality: "decision",
            },
            "jev-latest",
            Some(api_key),
            HeaderMap::new(),
        )
    }

    /// Vercel AI Gateway's OpenAI-compatible Decisions API. Decision models have
    /// `"type": "evaluation"` in `GET {base}/models`.
    pub fn vercel(base_url: &str, api_key: &str) -> AppResult<Self> {
        let base = base_url.trim_end_matches('/');
        Self::new(
            "Vercel AI Gateway",
            format!("{base}/decisions"),
            GatewayWire::VercelDecisions,
            DecisionDiscovery::ModelType {
                url: format!("{base}/models"),
                model_type: "evaluation",
            },
            "typesafe-ai/jev",
            Some(api_key),
            HeaderMap::new(),
        )
    }

    /// Cloudflare Workers AI: `POST .../accounts/{id}/ai/run` with the
    /// partner model `typesafe/jev`, which is not in the Workers AI catalog.
    /// Returns `None` when `base_url` does not identify an account.
    pub fn cloudflare(base_url: &str, api_key: &str) -> AppResult<Option<Self>> {
        let Some(target) = cloudflare_run_target(base_url) else {
            return Ok(None);
        };
        let mut headers = HeaderMap::new();
        if let Some(gateway) = &target.gateway_id {
            if let Ok(v) = HeaderValue::from_str(gateway) {
                headers.insert(HeaderName::from_static("cf-aig-gateway-id"), v);
            }
        }
        Self::new(
            "Cloudflare Workers AI",
            target.run_url,
            GatewayWire::CloudflareRun,
            DecisionDiscovery::Static(vec![ListedModel::decision(
                "typesafe/jev",
                "TypeSafe Jev",
                32_000,
                super::provider::TYPESAFE_INPUT_COST_PER_1K,
            )]),
            "typesafe/jev",
            Some(api_key),
            headers,
        )
        .map(Some)
    }

    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    /// The gateway's model listing (cached). Failures yield the last good
    /// listing, or an empty one.
    pub async fn listing(&self) -> Arc<Vec<ListedModel>> {
        let url = match &self.discovery {
            DecisionDiscovery::Static(models) => return Arc::new(models.clone()),
            DecisionDiscovery::OutputModality { url, .. }
            | DecisionDiscovery::ModelType { url, .. } => url,
        };

        let mut cache = self.cache.lock().await;
        let fresh = cache.fetched_at.is_some_and(|t| t.elapsed() < LISTING_TTL);
        let backing_off = cache.failed_at.is_some_and(|t| t.elapsed() < LISTING_RETRY);
        if let Some(models) = &cache.models {
            if fresh || backing_off {
                return models.clone();
            }
        } else if backing_off {
            return Arc::new(Vec::new());
        }

        match self.fetch_json(url).await {
            Ok(body) => {
                let models = Arc::new(self.parse_discovery(&body));
                cache.models = Some(models.clone());
                cache.fetched_at = Some(Instant::now());
                cache.failed_at = None;
                models
            }
            Err(e) => {
                debug!("{}: model listing unavailable: {}", self.display_name, e);
                cache.failed_at = Some(Instant::now());
                cache.models.clone().unwrap_or_default()
            }
        }
    }

    fn parse_discovery(&self, body: &Value) -> Vec<ListedModel> {
        match &self.discovery {
            DecisionDiscovery::Static(models) => models.clone(),
            DecisionDiscovery::OutputModality { modality, .. } => {
                parse_listing(body, |m| has_output_modality(m, modality))
            }
            DecisionDiscovery::ModelType { model_type, .. } => parse_listing(body, |m| {
                m.get("type").and_then(Value::as_str) == Some(*model_type)
                    || m.get("type").and_then(Value::as_str) == Some("decision")
                    || has_output_modality(m, "decision")
                    || has_output_modality(m, "decisions")
            }),
        }
    }

    async fn fetch_json(&self, url: &str) -> AppResult<Value> {
        let resp = self
            .client
            .get(url)
            .headers(self.headers.clone())
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("connection to {url} failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::Provider(format!(
                "{} returned {} for its model list",
                self.display_name,
                resp.status()
            )));
        }
        resp.json()
            .await
            .map_err(|e| AppError::Provider(format!("invalid model list: {e}")))
    }

    /// Whether `model` is one of the gateway's decision models.
    pub async fn is_decision_model(&self, model: &str) -> bool {
        model == self.default_model
            || self
                .listing()
                .await
                .iter()
                .any(|m| m.decision && m.id == model)
    }

    /// Pricing from the gateway's listing, when it has any for `model`.
    pub async fn pricing(&self, model: &str) -> Option<PricingInfo> {
        self.listing()
            .await
            .iter()
            .find(|m| m.id == model)
            .and_then(|m| m.pricing.clone())
    }

    /// Decision models as `ModelInfo`, for the provider's model list.
    pub async fn decision_model_infos(&self, provider: &str) -> Vec<ModelInfo> {
        self.listing()
            .await
            .iter()
            .filter(|m| m.decision)
            .map(|m| decision_model_info(m, provider))
            .collect()
    }

    /// Merge the gateway listing into a provider's chat model list: decision
    /// models replace any chat entry with the same id (or are appended), and
    /// chat models take the listing's context window when the list lacks one.
    pub async fn merge_into(&self, provider: &str, models: &mut Vec<ModelInfo>) {
        let listing = self.listing().await;
        models.retain(|m| !listing.iter().any(|l| l.decision && l.id == m.id));
        for m in models.iter_mut() {
            if let Some(ctx) = listing
                .iter()
                .find(|l| l.id == m.id)
                .and_then(|l| l.context_window)
            {
                m.context_window = ctx;
            }
        }
        models.extend(
            listing
                .iter()
                .filter(|l| l.decision)
                .map(|l| decision_model_info(l, provider)),
        );
    }

    pub fn validate_options(&self, request: &SystemOneRequest) -> AppResult<()> {
        if request
            .extra
            .get("service_tier")
            .is_some_and(|tier| !tier.is_string())
        {
            return Err(AppError::InvalidParams(
                "Gateway service tier must be a string".into(),
            ));
        }
        let extensions = request
            .extra
            .get("extensions")
            .and_then(Value::as_object)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
        let preferences = self.gateway_preferences.effective_with_service_tier(
            extensions.as_ref(),
            request.extra.get("service_tier").and_then(Value::as_str),
        )?;
        if preferences.reasoning_effort.is_some()
            || request.extra.contains_key("reasoning_effort")
            || request.extra.contains_key("reasoning")
        {
            return Err(AppError::InvalidParams(
                "Reasoning effort controls are not verified for native Decisions endpoints".into(),
            ));
        }
        if self.wire == GatewayWire::OpenRouterDecisions && preferences != Default::default() {
            return Err(AppError::InvalidParams(
                "OpenRouter Alpha Decisions does not expose verified routing controls".into(),
            ));
        }
        if let Some(kind) = self.gateway_kind() {
            preferences.apply(kind, &mut serde_json::json!({"model":request.model.as_deref().unwrap_or(&self.default_model)}))?;
        }
        Ok(())
    }

    /// Send a System One request. `request.model` is the gateway's model id
    /// (the default decision model when `None`).
    pub async fn systemone(&self, mut request: SystemOneRequest) -> AppResult<SystemOneResponse> {
        self.validate_options(&request)?;
        let extensions = request
            .extra
            .get("extensions")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<std::collections::HashMap<_, _>>()
            });
        let preferences = self.gateway_preferences.effective_with_service_tier(
            extensions.as_ref(),
            request.extra.get("service_tier").and_then(Value::as_str),
        )?;
        if self.wire == GatewayWire::OpenRouterDecisions && preferences != Default::default() {
            return Err(AppError::InvalidParams("OpenRouter alpha decisions does not expose verified per-request routing controls; use a separate instance or another gateway for constrained decisions".into()));
        }
        let model = request
            .model
            .take()
            .unwrap_or_else(|| self.default_model.clone());
        let mut body = match self.wire {
            GatewayWire::Decisions | GatewayWire::VercelDecisions => {
                request.model = Some(model);
                let decision = super::decisions::DecisionRequest::from_systemone(&request)
                    .map_err(AppError::InvalidParams)?;
                if self.wire == GatewayWire::VercelDecisions {
                    decision
                        .validate_vercel()
                        .map_err(AppError::InvalidParams)?;
                } else {
                    decision.validate().map_err(AppError::InvalidParams)?;
                }
                serde_json::to_value(decision)
                    .map_err(|e| AppError::Internal(format!("serialize decision: {e}")))?
            }
            GatewayWire::Plain => {
                request.model = Some(model);
                serde_json::to_value(&request)
                    .map_err(|e| AppError::Internal(format!("serialize request: {e}")))?
            }
            GatewayWire::OpenRouterDecisions => {
                request.model = Some(model);
                let adapted = super::decisions::DecisionRequest::openrouter_request(&request)
                    .map_err(AppError::InvalidParams)?;
                serde_json::to_value(adapted).map_err(|e| AppError::Internal(e.to_string()))?
            }
            // Cloudflare's input schema rejects any field besides these two.
            GatewayWire::CloudflareRun => json!({
                "model": model,
                "input": {"state": request.state.clone(), "questions": request.questions.clone()},
            }),
        };
        if let Some(extension) = body.get_mut("extensions").and_then(Value::as_object_mut) {
            extension.remove(crate::gateway::EXTENSION);
            if extension.is_empty() {
                body.as_object_mut()
                    .expect("request object")
                    .remove("extensions");
            }
        }
        let gateway_headers = self
            .gateway_kind()
            .map(|kind| preferences.apply(kind, &mut body))
            .transpose()?
            .unwrap_or_default();
        let resp = self
            .client
            .post(&self.systemone_url)
            .headers(self.headers.clone())
            .headers(gateway_headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!("connection to {} failed: {e}", self.systemone_url))
            })?;
        let status = resp.status().as_u16();
        let request_id = resp
            .headers()
            .get(TYPESAFE_REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let text = resp
            .text()
            .await
            .map_err(|e| AppError::Provider(format!("failed to read response: {e}")))?;
        if !(200..300).contains(&status) {
            return Err(map_systemone_error(status, text));
        }
        let mut payload = match self.wire {
            GatewayWire::Plain
            | GatewayWire::Decisions
            | GatewayWire::VercelDecisions
            | GatewayWire::OpenRouterDecisions => {
                serde_json::from_str::<Value>(&text).map_err(|e| {
                    AppError::Provider(format!(
                        "{} returned an invalid System One response: {e}",
                        self.display_name
                    ))
                })?
            }
            GatewayWire::CloudflareRun => unwrap_cloudflare(&text)?,
        };
        let attribution = self
            .gateway_kind()
            .map(|kind| crate::gateway::GatewayAttribution::from_wire(kind, &payload, preferences));
        // Native response extras otherwise preserve upstream metadata. Apply
        // the same credential-safe projection there, too.
        if let Some(attribution) = &attribution {
            if let Some(routing) = payload.pointer_mut("/metadata/routing") {
                *routing = serde_json::to_value(&attribution.routing)
                    .map_err(|e| AppError::Internal(e.to_string()))?;
            }
        }
        let mut parsed: SystemOneResponse = if matches!(
            self.wire,
            GatewayWire::Decisions | GatewayWire::VercelDecisions
        ) {
            serde_json::from_value::<super::decisions::DecisionResponse>(payload)
                .map_err(|e| AppError::Provider(format!("Invalid Decisions response: {e}")))?
                .into_systemone(&request)
                .map_err(AppError::Provider)?
        } else {
            serde_json::from_value(payload).map_err(|e| {
                AppError::Provider(format!(
                    "{} returned an invalid System One response: {e}",
                    self.display_name
                ))
            })?
        };
        parsed.request_id = request_id;
        if let Some(attribution) = attribution {
            let extensions = parsed
                .extra
                .entry("extensions")
                .or_insert_with(|| json!({}));
            if !extensions.is_object() {
                *extensions = json!({});
            }
            extensions[crate::gateway::EXTENSION] =
                serde_json::to_value(attribution).map_err(|e| AppError::Internal(e.to_string()))?;
        }
        Ok(parsed)
    }

    pub fn gateway_kind(&self) -> Option<crate::gateway::GatewayKind> {
        use crate::gateway::GatewayKind;
        match self.wire {
            GatewayWire::Decisions => Some(GatewayKind::Llmgateway),
            GatewayWire::VercelDecisions => Some(GatewayKind::Vercel),
            GatewayWire::OpenRouterDecisions => Some(GatewayKind::Openrouter),
            _ => None,
        }
    }

    pub fn set_gateway_preferences(&mut self, preferences: crate::gateway::GatewayPreferences) {
        self.gateway_preferences = preferences;
    }

    pub fn supports_decisions(&self) -> bool {
        matches!(
            self.wire,
            GatewayWire::Decisions
                | GatewayWire::VercelDecisions
                | GatewayWire::OpenRouterDecisions
        )
    }
}

fn decision_model_info(m: &ListedModel, provider: &str) -> ModelInfo {
    ModelInfo {
        id: m.id.clone(),
        name: m.name.clone(),
        provider: provider.to_string(),
        parameter_count: None,
        context_window: m.context_window.unwrap_or(32_000),
        supports_streaming: false,
        capabilities: if m.image_input {
            vec![Capability::Decision, Capability::Vision]
        } else {
            vec![Capability::Decision]
        },
        detailed_capabilities: None,
    }
}

fn has_output_modality(model: &Value, modality: &str) -> bool {
    model
        .pointer("/architecture/output_modalities")
        .and_then(Value::as_array)
        .is_some_and(|mods| mods.iter().any(|m| m.as_str() == Some(modality)))
}

/// Parse an OpenAI-style `{"data": [...]}` listing (OpenRouter, LLM Gateway
/// and Vercel all use it), marking entries `is_decision` accepts.
pub fn parse_listing(body: &Value, is_decision: impl Fn(&Value) -> bool) -> Vec<ListedModel> {
    let Some(items) = body
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| body.as_array())
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|m| {
            let id = m.get("id").and_then(Value::as_str)?.to_string();
            let name = m
                .get("name")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(&id)
                .to_string();
            let context_window = ["context_length", "context_window"]
                .iter()
                .find_map(|k| m.get(*k).and_then(Value::as_u64))
                .or_else(|| {
                    m.pointer("/top_provider/context_length")
                        .and_then(Value::as_u64)
                })
                .and_then(|c| u32::try_from(c).ok())
                .filter(|c| *c > 0);
            // OpenRouter/LLM Gateway: prompt/completion; Vercel: input/output.
            let pricing = crate::listing_metadata::listing_pricing(m);
            Some(ListedModel {
                image_input: m
                    .pointer("/architecture/input_modalities")
                    .or_else(|| m.get("input_modalities"))
                    .and_then(Value::as_array)
                    .is_some_and(|modalities| {
                        modalities.iter().any(|v| v.as_str() == Some("image"))
                    }),
                decision: is_decision(m),
                id,
                name,
                context_window,
                pricing,
            })
        })
        .collect()
}

/// Unwrap Cloudflare's `/ai/run` envelope around a TypeSafe payload.
fn unwrap_cloudflare(text: &str) -> AppResult<Value> {
    let v: Value = serde_json::from_str(text).map_err(|e| {
        AppError::Provider(format!(
            "Cloudflare Workers AI returned an invalid response: {e}"
        ))
    })?;
    if v.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(AppError::Provider(format!(
            "Cloudflare Workers AI error: {}",
            cloudflare_errors(&v)
        )));
    }
    let result = v.get("result").unwrap_or(&Value::Null);
    // Partner models wrap the payload once more with a job state.
    if let Some(inner) = result.get("result") {
        let state = result
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("Completed");
        if state != "Completed" {
            return Err(AppError::Provider(format!(
                "Cloudflare Workers AI returned an unfinished job (state '{state}')"
            )));
        }
        return Ok(inner.clone());
    }
    if result.get("answers").is_some() {
        return Ok(result.clone());
    }
    Err(AppError::Provider(
        "Cloudflare Workers AI returned no System One answers".to_string(),
    ))
}

fn cloudflare_errors(v: &Value) -> String {
    let msgs: Vec<String> = v
        .get("errors")
        .and_then(Value::as_array)
        .map(|errs| {
            errs.iter()
                .map(|e| {
                    let code = e.get("code").map(|c| c.to_string()).unwrap_or_default();
                    let msg = e.get("message").and_then(Value::as_str).unwrap_or("");
                    format!("{msg} ({code})")
                })
                .collect()
        })
        .unwrap_or_default();
    if msgs.is_empty() {
        "unknown error".to_string()
    } else {
        msgs.join("; ")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct CloudflareRunTarget {
    pub run_url: String,
    pub gateway_id: Option<String>,
}

/// The `/ai/run` URL for the account in a Cloudflare base URL. Accepts the
/// Workers AI URL (`https://api.cloudflare.com/client/v4/accounts/{id}/ai/v1`)
/// and AI Gateway URLs
/// (`https://gateway.ai.cloudflare.com/v1/{id}/{gateway}/workers-ai/v1`), in
/// which case requests name the gateway.
pub fn cloudflare_run_target(base_url: &str) -> Option<CloudflareRunTarget> {
    let url = reqwest::Url::parse(base_url.trim()).ok()?;
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let valid_id = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric());
    match url.host_str()? {
        "api.cloudflare.com" => {
            let pos = segments.iter().position(|s| *s == "accounts")?;
            let account = segments.get(pos + 1).copied().filter(|s| valid_id(s))?;
            Some(CloudflareRunTarget {
                run_url: format!("https://api.cloudflare.com/client/v4/accounts/{account}/ai/run"),
                gateway_id: None,
            })
        }
        "gateway.ai.cloudflare.com" => {
            // /v1/{account}/{gateway}/...
            let account = segments.get(1).copied().filter(|s| valid_id(s))?;
            let gateway = segments.get(2).map(|s| s.to_string());
            Some(CloudflareRunTarget {
                run_url: format!("https://api.cloudflare.com/client/v4/accounts/{account}/ai/run"),
                gateway_id: gateway,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn request(model: Option<&str>) -> SystemOneRequest {
        serde_json::from_value(json!({
            "model": model,
            "state": {"ticket": "charged twice"},
            "questions": {
                "team": {"type": "choice", "instructions": "Which team?",
                          "criteria": {"billing": "refunds", "tech": "bugs"}}
            }
        }))
        .unwrap()
    }

    fn answer_payload(model: &str) -> Value {
        json!({
            "model": model,
            "answers": {"team": {"type": "choice", "choice": "billing", "confidence": 0.8,
                                  "probabilities": {"billing": 0.9, "tech": 0.1}}},
            "usage": {"input_tokens": 40, "output_tokens": 1}
        })
    }

    #[test]
    fn listing_parses_openrouter_llmgateway_and_vercel_shapes() {
        let openrouter = json!({"data": [{
            "id": "typesafe/jev-1.13", "name": "TypeSafe: Jev 1.13", "context_length": 32000,
            "architecture": {"modality": "text->decisions", "output_modalities": ["decisions"]},
            "pricing": {"prompt": "0.000000042", "completion": "0"}
        }]});
        let m = parse_listing(&openrouter, |m| has_output_modality(m, "decisions"));
        assert_eq!(m.len(), 1);
        assert!(m[0].decision);
        assert_eq!(m[0].context_window, Some(32_000));
        let p = m[0].pricing.clone().unwrap();
        assert!((p.input_cost_per_1k - 0.000_042).abs() < 1e-12);

        let llmgateway = json!({"data": [
            {"id": "gpt-4o-mini", "architecture": {"output_modalities": ["text"]},
             "pricing": {"prompt": "0.15e-6", "completion": "0.6e-6"}, "context_length": 128000},
            {"id": "jev-1.13.0", "architecture": {"output_modalities": ["decision"]},
             "pricing": {"prompt": "0.042e-6", "completion": "0"}, "context_length": 64000}
        ]});
        let m = parse_listing(&llmgateway, |m| has_output_modality(m, "decision"));
        assert_eq!(
            m.iter().map(|x| x.decision).collect::<Vec<_>>(),
            vec![false, true]
        );
        assert!((m[0].pricing.clone().unwrap().output_cost_per_1k - 0.000_6).abs() < 1e-12);

        let vercel = json!({"object": "list", "data": [
            {"id": "openai/gpt-5", "type": "language", "context_window": 400000,
             "pricing": {"input": "0.00000125", "output": "0.00001"}},
            {"id": "typesafe-ai/jev", "type": "evaluation", "context_window": 32000,
             "pricing": {"input": "0.000000042", "output": "0"}}
        ]});
        let m = parse_listing(&vercel, |m| {
            m.get("type").and_then(Value::as_str) == Some("evaluation")
        });
        assert!(!m[0].decision && m[1].decision);
        assert_eq!(m[0].context_window, Some(400_000));
    }

    #[test]
    fn cloudflare_targets() {
        assert_eq!(
            cloudflare_run_target("https://api.cloudflare.com/client/v4/accounts/abc123/ai/v1"),
            Some(CloudflareRunTarget {
                run_url: "https://api.cloudflare.com/client/v4/accounts/abc123/ai/run".into(),
                gateway_id: None
            })
        );
        assert_eq!(
            cloudflare_run_target(
                "https://gateway.ai.cloudflare.com/v1/abc123/my-gw/workers-ai/v1"
            ),
            Some(CloudflareRunTarget {
                run_url: "https://api.cloudflare.com/client/v4/accounts/abc123/ai/run".into(),
                gateway_id: Some("my-gw".into())
            })
        );
        assert_eq!(cloudflare_run_target("https://example.com/v1"), None);
        assert_eq!(
            cloudflare_run_target("https://api.cloudflare.com/client/v4/accounts/../ai/v1"),
            None
        );
    }

    #[test]
    fn cloudflare_envelope() {
        let ok = json!({"result": {"state": "Completed", "result": answer_payload("jev-1.13.0")},
                        "success": true, "errors": []});
        assert_eq!(
            unwrap_cloudflare(&ok.to_string()).unwrap()["model"],
            "jev-1.13.0"
        );
        let flat = json!({"result": answer_payload("jev-1.13.0"), "success": true});
        assert!(unwrap_cloudflare(&flat.to_string()).is_ok());
        let queued = json!({"result": {"state": "Queued", "result": {}}, "success": true});
        assert!(unwrap_cloudflare(&queued.to_string()).is_err());
        let failed = json!({"result": null, "success": false,
                            "errors": [{"code": 2021, "message": "Insufficient balance"}]});
        match unwrap_cloudflare(&failed.to_string()) {
            Err(AppError::Provider(m)) => assert!(m.contains("Insufficient balance (2021)")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn plain_gateway_round_trip_and_discovery() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/models"))
            .and(query_param("output_modalities", "decisions"))
            .and(header("authorization", "Bearer k"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [
                {"id": "typesafe/jev-1.13", "context_length": 32000,
                 "architecture": {"output_modalities": ["decisions"]},
                 "pricing": {"prompt": "0.000000042", "completion": "0"}}
            ]})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/alpha/decisions"))
            .and(header("authorization", "Bearer k"))
            .respond_with(ResponseTemplate::new(200).set_body_json({
                let mut p = answer_payload("typesafe/jev-1.13-20260917");
                p["id"] = json!("gen-dec-1");
                p["provider"] = json!("TypeSafe");
                p
            }))
            .mount(&server)
            .await;

        let gw = SystemOneGateway::openrouter(
            &format!("{}/api/v1", server.uri()),
            "k",
            HeaderMap::new(),
        )
        .unwrap();
        assert!(gw.is_decision_model("typesafe/jev-1.13").await);
        assert!(gw.is_decision_model("~typesafe/jev-latest").await);
        assert!(!gw.is_decision_model("openai/gpt-4o").await);
        assert!(gw.pricing("typesafe/jev-1.13").await.is_some());
        let infos = gw.decision_model_infos("openrouter").await;
        assert_eq!(infos[0].capabilities, vec![Capability::Decision]);

        let resp = gw
            .systemone(request(Some("typesafe/jev-1.13")))
            .await
            .unwrap();
        assert_eq!(resp.model, "typesafe/jev-1.13-20260917");
        assert_eq!(resp.extra["id"], "gen-dec-1");
    }

    #[tokio::test]
    async fn merge_replaces_chat_entries_for_decision_models() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [
                {"id": "gpt-4o-mini", "context_length": 128000,
                 "architecture": {"output_modalities": ["text"]}},
                {"id": "jev-1.13.0", "context_length": 64000,
                 "architecture": {"output_modalities": ["decision"]}}
            ]})))
            .mount(&server)
            .await;
        let gw = SystemOneGateway::llmgateway(&format!("{}/v1", server.uri()), "k").unwrap();
        let chat = |id: &str| ModelInfo {
            id: id.into(),
            name: id.into(),
            provider: "llmgateway".into(),
            parameter_count: None,
            context_window: 4096,
            supports_streaming: true,
            capabilities: vec![Capability::Chat],
            detailed_capabilities: None,
        };
        let mut models = vec![chat("gpt-4o-mini"), chat("jev-1.13.0")];
        gw.merge_into("llmgateway", &mut models).await;
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].context_window, 128_000);
        assert_eq!(models[1].id, "jev-1.13.0");
        assert_eq!(models[1].capabilities, vec![Capability::Decision]);
        // Aliases the gateway accepts but does not list.
        assert!(gw.is_decision_model("jev-latest").await);
    }

    #[tokio::test]
    async fn vercel_uses_native_decisions_path() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/decisions"))
            .and(body_json(json!({"model":"typesafe-ai/jev","input":"{\"ticket\":\"charged twice\"}","questions":[{"type":"choice","name":"team","instructions":"Which team?","choices":[{"value":"billing","description":"refunds"},{"value":"tech","description":"bugs"}]}]})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"model":"jev-1.13.0","answers":[{"type":"choice","name":"team","choice":"billing","confidence":0.8,"probabilities":[{"value":"billing","probability":0.9},{"value":"tech","probability":0.1}]}],"usage":{"input_tokens":10,"output_tokens":0,"total_tokens":10}})))
            .expect(1).mount(&server).await;
        let gw = SystemOneGateway::vercel(&format!("{}/v1", server.uri()), "k").unwrap();
        assert!(gw.supports_decisions());
        let response = gw.systemone(request(None)).await.unwrap();
        assert_eq!(response.model, "jev-1.13.0");
        assert!(
            matches!(&response.answers["team"], super::super::SystemOneAnswer::Choice { choice, .. } if choice == "billing")
        );
    }

    #[tokio::test]
    async fn cloudflare_wraps_and_unwraps() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/run"))
            .and(body_json(json!({
                "model": "typesafe/jev",
                "input": {"state": {"ticket": "charged twice"},
                          "questions": {"team": {"type": "choice", "instructions": "Which team?",
                                                  "criteria": {"billing": "refunds", "tech": "bugs"}}}}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "result": {"state": "Completed", "result": answer_payload("jev-1.13.0")},
                "success": true, "errors": [], "messages": []
            })))
            .mount(&server)
            .await;
        let gw = SystemOneGateway::new(
            "Cloudflare Workers AI",
            format!("{}/run", server.uri()),
            GatewayWire::CloudflareRun,
            DecisionDiscovery::Static(vec![]),
            "typesafe/jev",
            Some("k"),
            HeaderMap::new(),
        )
        .unwrap();
        let resp = gw.systemone(request(Some("typesafe/jev"))).await.unwrap();
        assert_eq!(resp.usage.input_tokens, Some(40));
    }

    #[tokio::test]
    async fn upstream_client_errors_keep_their_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/decisions"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(json!({"error": {"code": 400, "message": "too many options"}})),
            )
            .mount(&server)
            .await;
        let gw = SystemOneGateway::llmgateway(&format!("{}/v1", server.uri()), "k").unwrap();
        match gw.systemone(request(None)).await {
            Err(AppError::ProviderStatus { status, message }) => {
                assert_eq!(status, 400);
                assert!(message.contains("too many options"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn failed_listing_is_empty_and_not_refetched_immediately() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;
        let gw = SystemOneGateway::llmgateway(&format!("{}/v1", server.uri()), "k").unwrap();
        assert!(!gw.is_decision_model("jev-1.13.0").await);
        assert!(!gw.is_decision_model("jev-1.13.0").await);
    }
}
