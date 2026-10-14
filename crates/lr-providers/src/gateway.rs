//! Typed gateway controls. Request preferences may narrow configured restrictions,
//! never remove them. Inference-provider identities differ from LocalRouter instances.
use lr_types::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use utoipa::ToSchema;

/// Preserve the typed serializer's f32 wire representation while decorating
/// a request (to_value widens f32 before serializing it as an f64).
pub(crate) fn wire_body<T: Serialize>(request: &T) -> AppResult<Value> {
    let encoded = serde_json::to_string(request).map_err(|e| AppError::Internal(e.to_string()))?;
    serde_json::from_str(&encoded).map_err(|e| AppError::Internal(e.to_string()))
}

/// Reject recognizable vendor controls before a typed ingress can discard them.
/// Otherwise a raw provider.zdr request could silently lose its privacy intent.
pub fn reject_raw_controls(body: &Value) -> AppResult<()> {
    for key in [
        "provider",
        "providerOptions",
        "provider_options",
        "routing",
        "models",
        "zdr",
        "data_collection",
        "max_price",
        "allow_fallbacks",
        "require_parameters",
        "only",
        "ignore",
        "order",
        "zeroDataRetention",
        "disallowPromptTraining",
    ] {
        if body.get(key).is_some() {
            return Err(AppError::InvalidParams(format!("Use extensions.localrouter.gateway for gateway routing controls (unsupported raw field: {key})")));
        }
    }
    Ok(())
}

pub const EXTENSION: &str = "localrouter.gateway";
pub const CONFIG_KEY: &str = "gateway_preferences";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GatewayKind {
    Openrouter,
    Llmgateway,
    Vercel,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GatewaySort {
    Price,
    Latency,
    Throughput,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DataCollection {
    Allow,
    Deny,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GatewayMaxPrice {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// USD per million input tokens.
    pub prompt: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// USD per million output tokens.
    pub completion: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// USD per request.
    pub request: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// USD per image.
    pub image: Option<f64>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GatewayPreferences {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub only: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignore: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<GatewaySort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_fallbacks: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_parameters: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_collection: Option<DataCollection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zdr: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_price: Option<GatewayMaxPrice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

fn covers(parent: &str, child: &str) -> bool {
    let parent = parent.to_ascii_lowercase();
    let child = child.to_ascii_lowercase();
    parent == child || child.starts_with(&format!("{parent}/"))
}
fn unique(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .filter(|s| seen.insert(s.to_ascii_lowercase()))
        .collect()
}
fn ceiling(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}
fn effort_rank(effort: &str) -> Option<usize> {
    ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
        .iter()
        .position(|value| *value == effort)
}
fn strict_bool(a: Option<bool>, b: Option<bool>, strict: bool) -> Option<bool> {
    if a == Some(strict) || b == Some(strict) {
        Some(strict)
    } else {
        b.or(a)
    }
}
impl GatewayPreferences {
    pub fn validate(&self) -> AppResult<()> {
        if self.only.as_ref().is_some_and(Vec::is_empty) {
            return Err(AppError::InvalidParams(
                "Gateway provider allowlist must not be empty".into(),
            ));
        }
        for value in self
            .only
            .iter()
            .flatten()
            .chain(&self.order)
            .chain(&self.ignore)
        {
            if value.trim().is_empty() || value != value.trim() || value.len() > 256 {
                return Err(AppError::InvalidParams(
                    "Gateway provider names must be nonempty, trimmed and at most 256 bytes".into(),
                ));
            }
        }
        if let Some(prices) = &self.max_price {
            let amounts = [
                prices.prompt,
                prices.completion,
                prices.request,
                prices.image,
            ];
            if amounts.iter().all(Option::is_none)
                || amounts
                    .into_iter()
                    .flatten()
                    .any(|v| !v.is_finite() || v < 0.0)
            {
                return Err(AppError::InvalidParams(
                    "Price ceilings must be finite nonnegative amounts".into(),
                ));
            }
        }
        if self.service_tier.as_ref().is_some_and(|s| {
            !["default", "flex", "priority", "fast", "ultrafast"].contains(&s.as_str())
        }) {
            return Err(AppError::InvalidParams(
                "Unsupported gateway service tier".into(),
            ));
        }
        if self
            .reasoning_effort
            .as_deref()
            .is_some_and(|effort| effort_rank(effort).is_none())
        {
            return Err(AppError::InvalidParams(
                "Unsupported gateway reasoning effort".into(),
            ));
        }
        Ok(())
    }
    pub fn from_config(config: &HashMap<String, String>) -> AppResult<Self> {
        let preferences = match config
            .get(CONFIG_KEY)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        {
            Some(raw) => serde_json::from_str(raw)
                .map_err(|e| AppError::Config(format!("Invalid gateway preferences: {e}")))?,
            None => Self::default(),
        };
        preferences.validate()?;
        Ok(preferences)
    }
    pub fn effective(&self, extensions: Option<&HashMap<String, Value>>) -> AppResult<Self> {
        let request = extensions
            .and_then(|e| e.get(EXTENSION))
            .map(|v| {
                serde_json::from_value::<Self>(v.clone())
                    .map_err(|e| AppError::InvalidParams(format!("Invalid {EXTENSION}: {e}")))
            })
            .transpose()?
            .unwrap_or_default();
        self.merge(&request)
    }
    /// The standard tier field is an alias; namespaced request controls and
    /// saved instance restrictions retain their normal precedence.
    pub fn effective_with_service_tier(
        &self,
        extensions: Option<&HashMap<String, Value>>,
        tier: Option<&str>,
    ) -> AppResult<Self> {
        let mut requested = Self::default().effective(extensions)?;
        if requested.service_tier.is_none() {
            requested.service_tier = tier.map(str::to_owned);
        }
        self.merge(&requested)
    }
    pub fn effective_for_chat(&self, request: &crate::CompletionRequest) -> AppResult<Self> {
        let mut requested = Self::default().effective_with_service_tier(
            request.extensions.as_ref(),
            request.service_tier.as_deref(),
        )?;
        if requested.reasoning_effort.is_none() {
            requested.reasoning_effort = request.reasoning_effort.clone();
        }
        self.merge(&requested)
    }
    pub fn merge(&self, request: &Self) -> AppResult<Self> {
        self.validate()?;
        request.validate()?;
        let only = match (&self.only, &request.only) {
            (Some(config), Some(wanted)) => Some(unique(
                config
                    .iter()
                    .flat_map(|c| {
                        wanted.iter().filter_map(move |w| {
                            if covers(c, w) {
                                Some(w.clone())
                            } else if covers(w, c) {
                                Some(c.clone())
                            } else {
                                None
                            }
                        })
                    })
                    .collect(),
            )),
            (a, b) => b.clone().or_else(|| a.clone()),
        };
        let ignore = unique(self.ignore.iter().chain(&request.ignore).cloned().collect());
        let only = only.map(|values| {
            values
                .into_iter()
                .filter(|v| !ignore.iter().any(|excluded| covers(excluded, v)))
                .collect::<Vec<_>>()
        });
        let order = if request.order.is_empty() {
            self.order.clone()
        } else {
            request.order.clone()
        };
        let order = unique(
            order
                .into_iter()
                .filter(|v| {
                    !ignore.iter().any(|excluded| covers(excluded, v))
                        && only.as_ref().is_none_or(|allowed| {
                            allowed.iter().any(|a| covers(a, v) || covers(v, a))
                        })
                })
                .collect(),
        );
        let max_price = match (&self.max_price, &request.max_price) {
            (Some(a), Some(b)) => Some(GatewayMaxPrice {
                prompt: ceiling(a.prompt, b.prompt),
                completion: ceiling(a.completion, b.completion),
                request: ceiling(a.request, b.request),
                image: ceiling(a.image, b.image),
            }),
            (a, b) => b.clone().or_else(|| a.clone()),
        };
        let merged = Self {
            only,
            order,
            ignore,
            sort: request.sort.or(self.sort),
            allow_fallbacks: strict_bool(self.allow_fallbacks, request.allow_fallbacks, false),
            require_parameters: strict_bool(
                self.require_parameters,
                request.require_parameters,
                true,
            ),
            data_collection: if self.data_collection == Some(DataCollection::Deny)
                || request.data_collection == Some(DataCollection::Deny)
            {
                Some(DataCollection::Deny)
            } else {
                request.data_collection.or(self.data_collection)
            },
            zdr: strict_bool(self.zdr, request.zdr, true),
            max_price,
            reasoning_effort: match (&self.reasoning_effort, &request.reasoning_effort) {
                (Some(configured), Some(requested)) => {
                    Some(if effort_rank(configured) <= effort_rank(requested) {
                        configured.clone()
                    } else {
                        requested.clone()
                    })
                }
                (a, b) => b.clone().or_else(|| a.clone()),
            },
            service_tier: self
                .service_tier
                .clone()
                .or_else(|| request.service_tier.clone()),
        };
        merged.validate()?;
        Ok(merged)
    }
    /// Return fixed gateway headers, modifying only documented routing fields.
    pub fn apply(
        &self,
        kind: GatewayKind,
        body: &mut Value,
    ) -> AppResult<reqwest::header::HeaderMap> {
        self.validate()?;
        if self.service_tier.is_some() && kind != GatewayKind::Openrouter {
            return Err(AppError::InvalidParams(
                "Service-tier controls are currently supported only for OpenRouter".into(),
            ));
        }
        reject_raw_controls(body)?;
        let mut headers = reqwest::header::HeaderMap::new();
        match kind {
            GatewayKind::Openrouter => {
                let mut provider =
                    serde_json::to_value(self).map_err(|e| AppError::Internal(e.to_string()))?;
                provider
                    .as_object_mut()
                    .expect("preferences object")
                    .remove("service_tier");
                provider
                    .as_object_mut()
                    .expect("preferences object")
                    .remove("reasoning_effort");
                if let Some(effort) = &self.reasoning_effort {
                    body["reasoning"] = json!({"effort":effort});
                }
                body.as_object_mut()
                    .expect("request object")
                    .remove("reasoning_effort");
                if provider.as_object().is_some_and(|v| !v.is_empty()) {
                    body["provider"] = provider;
                }
            }
            GatewayKind::Llmgateway => {
                if self.require_parameters == Some(true)
                    || self.data_collection == Some(DataCollection::Deny)
                    || self.zdr == Some(true)
                    || self.max_price.is_some()
                    || !self.ignore.is_empty()
                {
                    return Err(AppError::InvalidParams("LLM Gateway does not expose these strict per-request constraints; configure its project policy or choose a gateway that supports them".into()));
                }
                let pin = self
                    .only
                    .as_ref()
                    .or_else(|| (!self.order.is_empty()).then_some(&self.order));
                if let Some(pin) = pin {
                    if pin.len() != 1 {
                        return Err(AppError::InvalidParams(
                            "LLM Gateway routing requires exactly one pinned inference provider"
                                .into(),
                        ));
                    }
                    let model = body.get("model").and_then(Value::as_str).ok_or_else(|| {
                        AppError::InvalidParams("A model is required for gateway routing".into())
                    })?;
                    let prefix = format!("{}/", pin[0]);
                    if model.contains('/') && !model.starts_with(&prefix) {
                        return Err(AppError::InvalidParams(
                            "Model provider prefix conflicts with gateway restrictions".into(),
                        ));
                    }
                    if !model.starts_with(&prefix) {
                        body["model"] = json!(format!("{prefix}{model}"));
                    }
                }
                if self.allow_fallbacks == Some(false) || pin.is_some() {
                    headers.insert(
                        "x-no-fallback",
                        reqwest::header::HeaderValue::from_static("true"),
                    );
                }
                if let Some(sort) = self.sort {
                    body["routing"] = json!(sort);
                }
            }
            GatewayKind::Vercel => {
                if self.require_parameters == Some(true) || self.max_price.is_some() {
                    return Err(AppError::InvalidParams("Vercel does not expose parameter requirements or price ceilings through these provider options".into()));
                }
                let mut only = self.only.clone();
                if self.ignore.iter().any(|excluded| {
                    only.as_ref().is_none_or(|allowed| {
                        allowed
                            .iter()
                            .any(|a| covers(a, excluded) || covers(excluded, a))
                    })
                }) {
                    return Err(AppError::InvalidParams(
                        "Vercel cannot represent these exclusions with the selected provider allowlist".into(),
                    ));
                }
                if self.allow_fallbacks == Some(false) {
                    let pin = only.as_ref().filter(|v| v.len() == 1).cloned().or_else(|| {
                        self.order.first().and_then(|wanted| {
                            if let Some(allowed) = &only {
                                allowed.iter().find_map(|a| {
                                    if covers(a, wanted) {
                                        Some(vec![wanted.clone()])
                                    } else if covers(wanted, a) {
                                        Some(vec![a.clone()])
                                    } else {
                                        None
                                    }
                                })
                            } else {
                                Some(vec![wanted.clone()])
                            }
                        })
                    });
                    only = Some(pin.ok_or_else(|| {
                        AppError::InvalidParams(
                            "Disabling Vercel fallbacks requires one pinned inference provider"
                                .into(),
                        )
                    })?);
                }
                let mut options = json!({});
                if let Some(only) = only {
                    options["only"] = json!(only);
                }
                if !self.order.is_empty() {
                    options["order"] = json!(self.order);
                }
                if let Some(sort) = self.sort {
                    options["sort"] = json!(match sort {
                        GatewaySort::Price => "cost",
                        GatewaySort::Latency => "ttft",
                        GatewaySort::Throughput => "tps",
                    });
                }
                if self.zdr == Some(true) {
                    options["zeroDataRetention"] = json!(true);
                }
                if self.data_collection == Some(DataCollection::Deny) {
                    options["disallowPromptTraining"] = json!(true);
                    options["zeroDataRetention"] = json!(true);
                }
                if options.as_object().is_some_and(|v| !v.is_empty()) {
                    body["providerOptions"] = json!({"gateway":options});
                }
            }
        }
        if kind != GatewayKind::Openrouter {
            if let Some(effort) = &self.reasoning_effort {
                body["reasoning_effort"] = json!(effort);
            }
        }
        if let Some(tier) = &self.service_tier {
            body["service_tier"] = json!(tier);
        }
        Ok(headers)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GatewayCostSource {
    Reported,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GatewayCostScope {
    UpstreamCall,
    RequestTotal,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GatewayAttribution {
    pub gateway: GatewayKind,
    pub upstream_request_id: Option<String>,
    pub inference_provider: Option<String>,
    /// Initial resolution is not proof of which provider ultimately served the request.
    pub resolved_provider: Option<String>,
    pub region: Option<String>,
    pub service_tier: Option<String>,
    pub cost_usd: Option<f64>,
    pub cost_source: GatewayCostSource,
    pub cost_scope: GatewayCostScope,
    pub routing: Vec<GatewayRoutingAttempt>,
    pub applied_preferences: GatewayPreferences,
}

/// Deliberately omit credential fingerprints, key IDs, labels and raw errors.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GatewayRoutingAttempt {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub succeeded: Option<bool>,
    pub status_code: Option<u16>,
    pub region: Option<String>,
}
fn amount(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
}
impl GatewayAttribution {
    pub fn from_wire(kind: GatewayKind, body: &Value, preferences: GatewayPreferences) -> Self {
        let successful = body
            .pointer("/metadata/routing")
            .and_then(Value::as_array)
            .and_then(|attempts| {
                attempts
                    .iter()
                    .rev()
                    .find(|a| a.get("succeeded").and_then(Value::as_bool) == Some(true))
            });
        let string = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_string);
        let root_provider = string(body.get("provider"));
        let vercel_provider =
            string(body.pointer("/provider_metadata/gateway/routing/finalProvider"));
        let routed_provider = string(successful.and_then(|a| a.get("provider")));
        let inference_provider = match kind {
            GatewayKind::Vercel => vercel_provider,
            GatewayKind::Llmgateway => routed_provider,
            GatewayKind::Openrouter => root_provider,
        };
        let cost_usd = body
            .pointer("/usage/cost")
            .and_then(amount)
            .or_else(|| {
                body.pointer("/provider_metadata/gateway/cost")
                    .and_then(amount)
            })
            .or_else(|| body.pointer("/metadata/cost").and_then(amount));
        Self {
            gateway: kind,
            upstream_request_id: string(body.get("id")),
            inference_provider,
            resolved_provider: string(
                body.pointer("/provider_metadata/gateway/routing/resolvedProvider"),
            ),
            region: string(successful.and_then(|a| a.get("region")))
                .or_else(|| string(body.pointer("/metadata/region"))),
            service_tier: string(body.get("service_tier")),
            cost_usd,
            cost_source: if cost_usd.is_some() {
                GatewayCostSource::Reported
            } else {
                GatewayCostSource::Unknown
            },
            cost_scope: GatewayCostScope::UpstreamCall,
            routing: body
                .pointer("/metadata/routing")
                .and_then(Value::as_array)
                .map(|attempts| {
                    attempts
                        .iter()
                        .map(|a| GatewayRoutingAttempt {
                            provider: string(a.get("provider")),
                            model: string(a.get("model")),
                            succeeded: a.get("succeeded").and_then(Value::as_bool),
                            status_code: a
                                .get("status_code")
                                .and_then(Value::as_u64)
                                .and_then(|v| u16::try_from(v).ok()),
                            region: string(a.get("region")),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            applied_preferences: preferences,
        }
    }
    /// SSE fields can arrive before or after content; absent fields do not
    /// erase metadata already reported for this same upstream request.
    pub fn retain_stream_metadata(&mut self, previous: &Self) {
        if self.gateway != previous.gateway
            || self.upstream_request_id != previous.upstream_request_id
        {
            return;
        }
        self.inference_provider = self
            .inference_provider
            .take()
            .or_else(|| previous.inference_provider.clone());
        self.resolved_provider = self
            .resolved_provider
            .take()
            .or_else(|| previous.resolved_provider.clone());
        self.region = self.region.take().or_else(|| previous.region.clone());
        self.service_tier = self
            .service_tier
            .take()
            .or_else(|| previous.service_tier.clone());
        self.cost_usd = self.cost_usd.or(previous.cost_usd);
        self.cost_source = if self.cost_usd.is_some() {
            GatewayCostSource::Reported
        } else {
            GatewayCostSource::Unknown
        };
        if self.routing.is_empty() {
            self.routing = previous.routing.clone();
        }
    }
    pub fn extensions(self) -> HashMap<String, Value> {
        [(
            EXTENSION.into(),
            serde_json::to_value(self).expect("validated attribution serialization"),
        )]
        .into_iter()
        .collect()
    }
}
pub fn reported_cost(extensions: Option<&HashMap<String, Value>>) -> Option<f64> {
    extensions?.get(EXTENSION)?.get("cost_usd").and_then(amount)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prefs(v: Value) -> GatewayPreferences {
        serde_json::from_value(v).unwrap()
    }
    #[test]
    fn requests_cannot_relax_policies() {
        let configured = prefs(
            json!({"only":["azure/us"],"ignore":["bad"],"zdr":true,"data_collection":"deny","require_parameters":true,"allow_fallbacks":false,"max_price":{"prompt":1.0}}),
        );
        let request = prefs(
            json!({"only":["azure","other"],"ignore":["worse"],"zdr":false,"data_collection":"allow","require_parameters":false,"allow_fallbacks":true,"max_price":{"prompt":10.0,"completion":2.0}}),
        );
        let merged = configured.merge(&request).unwrap();
        assert_eq!(merged.only, Some(vec!["azure/us".into()]));
        assert_eq!(merged.zdr, Some(true));
        assert_eq!(merged.data_collection, Some(DataCollection::Deny));
        assert_eq!(merged.allow_fallbacks, Some(false));
        assert_eq!(merged.require_parameters, Some(true));
        assert_eq!(merged.max_price.unwrap().prompt, Some(1.0));
        assert!(configured.merge(&prefs(json!({"only":["other"]}))).is_err());
        assert!(configured
            .merge(&prefs(json!({"ignore":["azure"]})))
            .is_err());
    }
    #[test]
    fn gateway_shapes_and_unsupported_constraints_are_explicit() {
        let p = prefs(json!({"only":["azure"],"zdr":true,"sort":"latency"}));
        let mut body = json!({"model":"m"});
        p.apply(GatewayKind::Openrouter, &mut body).unwrap();
        assert_eq!(body["provider"]["zdr"], true);
        let mut body = json!({"model":"m"});
        p.apply(GatewayKind::Vercel, &mut body).unwrap();
        assert_eq!(body["providerOptions"]["gateway"]["sort"], "ttft");
        assert!(p
            .apply(GatewayKind::Llmgateway, &mut json!({"model":"m"}))
            .is_err());
        let p = prefs(json!({"only":["openai"],"allow_fallbacks":false,"sort":"price"}));
        let mut body = json!({"model":"m"});
        let headers = p.apply(GatewayKind::Llmgateway, &mut body).unwrap();
        assert_eq!(body["model"], "openai/m");
        assert_eq!(headers["x-no-fallback"], "true");
        assert!(p
            .apply(GatewayKind::Llmgateway, &mut json!({"model":"anthropic/m"}))
            .is_err());
    }
    #[test]
    fn hierarchical_fallback_pin_never_broadens_allowlist() {
        let configured = prefs(json!({"only":["azure/us"],"allow_fallbacks":false}));
        let merged = configured
            .merge(&prefs(json!({"order":["azure"]})))
            .unwrap();
        let mut body = json!({"model":"m"});
        merged.apply(GatewayKind::Vercel, &mut body).unwrap();
        assert_eq!(
            body["providerOptions"]["gateway"]["only"],
            json!(["azure/us"])
        );
        assert!(prefs(json!({"only":["azure"],"ignore":["azure/us"]}))
            .apply(GatewayKind::Vercel, &mut json!({"model":"m"}))
            .is_err());
    }
    #[test]
    fn conflicting_and_vendor_controls_fail_before_io() {
        for kind in [
            GatewayKind::Openrouter,
            GatewayKind::Vercel,
            GatewayKind::Llmgateway,
        ] {
            for key in ["provider", "providerOptions", "models", "routing"] {
                let mut body = json!({"model":"m"});
                body[key] = json!({});
                assert!(GatewayPreferences::default()
                    .apply(kind, &mut body)
                    .is_err());
            }
        }
        assert!(prefs(json!({"service_tier":"flex"}))
            .apply(GatewayKind::Vercel, &mut json!({"model":"m"}))
            .is_err());
        let configured = prefs(json!({"service_tier":"default"}));
        let merged = configured
            .merge(&prefs(json!({"service_tier":"ultrafast"})))
            .unwrap();
        assert_eq!(merged.service_tier.as_deref(), Some("default"));
        assert!(prefs(json!({"only":["azure"],"ignore":["azure"]}))
            .merge(&Default::default())
            .is_err());
    }
    #[test]
    fn attribution_omits_credentials_and_prefers_actual_final_provider() {
        let attribution = GatewayAttribution::from_wire(
            GatewayKind::Vercel,
            &json!({"id":"call1","provider":"gateway","usage":{"cost":null},"provider_metadata":{"gateway":{"cost":"0.002","routing":{"resolvedProvider":"first","finalProvider":"last"}}},"metadata":{"routing":[{"provider":"last","succeeded":true,"apiKeyHash":"secret","providerKeyLabel":"private","error":"sensitive"}]}}),
            Default::default(),
        );
        assert_eq!(attribution.inference_provider.as_deref(), Some("last"));
        assert_eq!(attribution.cost_usd, Some(0.002));
        let serialized = serde_json::to_string(&attribution).unwrap();
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains("private"));
        assert!(!serialized.contains("sensitive"));
    }
    #[test]
    fn initial_resolution_is_not_reported_as_final_inference() {
        let attribution = GatewayAttribution::from_wire(
            GatewayKind::Vercel,
            &json!({"provider":"gateway","provider_metadata":{"gateway":{"routing":{"resolvedProvider":"first"}}}}),
            Default::default(),
        );
        assert_eq!(attribution.resolved_provider.as_deref(), Some("first"));
        assert!(attribution.inference_provider.is_none());
        let attribution = GatewayAttribution::from_wire(
            GatewayKind::Llmgateway,
            &json!({"provider":"candidate","metadata":{"routing":[{"provider":"candidate","succeeded":false}]}}),
            Default::default(),
        );
        assert!(attribution.inference_provider.is_none());
    }
    #[test]
    fn stream_metadata_is_retained_only_within_the_same_upstream_call() {
        let first = GatewayAttribution::from_wire(
            GatewayKind::Openrouter,
            &json!({"id":"one","provider":"OpenAI","service_tier":"flex","usage":{"cost":0.01}}),
            Default::default(),
        );
        let mut last = GatewayAttribution::from_wire(
            GatewayKind::Openrouter,
            &json!({"id":"one"}),
            Default::default(),
        );
        last.retain_stream_metadata(&first);
        assert_eq!(last.inference_provider.as_deref(), Some("OpenAI"));
        assert_eq!(last.cost_usd, Some(0.01));
        let mut next = GatewayAttribution::from_wire(
            GatewayKind::Openrouter,
            &json!({"id":"two"}),
            Default::default(),
        );
        next.retain_stream_metadata(&last);
        assert!(next.inference_provider.is_none());
        assert!(next.cost_usd.is_none());
    }
    #[test]
    fn standard_tier_alias_cannot_override_saved_tier() {
        let mut request = crate::CompletionRequest::new("model", Vec::new());
        request.service_tier = Some("priority".into());
        assert_eq!(
            GatewayPreferences::default()
                .effective_for_chat(&request)
                .unwrap()
                .service_tier
                .as_deref(),
            Some("priority")
        );
        let configured = prefs(json!({"service_tier":"default"}));
        assert_eq!(
            configured
                .effective_for_chat(&request)
                .unwrap()
                .service_tier
                .as_deref(),
            Some("default")
        );
    }
    #[test]
    fn reasoning_effort_uses_unified_openrouter_shape_and_cannot_raise_saved_compute() {
        let configured = prefs(json!({"reasoning_effort":"low"}));
        let mut request = crate::CompletionRequest::new("model", Vec::new());
        request.reasoning_effort = Some("high".into());
        let default = GatewayPreferences::default()
            .effective_for_chat(&request)
            .unwrap();
        assert_eq!(default.reasoning_effort.as_deref(), Some("high"));
        let mut standard_body = json!({"model":"model","reasoning_effort":"high"});
        default
            .apply(GatewayKind::Openrouter, &mut standard_body)
            .unwrap();
        assert_eq!(standard_body["reasoning"]["effort"], "high");
        let effective = configured.effective_for_chat(&request).unwrap();
        let mut body = json!({"model":"model","reasoning_effort":"high"});
        effective.apply(GatewayKind::Openrouter, &mut body).unwrap();
        assert_eq!(body["reasoning"]["effort"], "low");
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("provider").is_none());
    }
    #[test]
    fn reported_cost_and_provider_are_not_fabricated() {
        let a = GatewayAttribution::from_wire(
            GatewayKind::Llmgateway,
            &json!({"usage":{"cost":0.0},"metadata":{"routing":[{"provider":"failed","succeeded":false},{"provider":"served","succeeded":true}]}}),
            Default::default(),
        );
        assert_eq!(a.inference_provider.as_deref(), Some("served"));
        assert_eq!(reported_cost(Some(&a.extensions())), Some(0.0));
        let a = GatewayAttribution::from_wire(GatewayKind::Vercel, &json!({}), Default::default());
        assert!(a.inference_provider.is_none());
        assert!(a.cost_usd.is_none());
        assert!(prefs(json!({"max_price":{"prompt":-1.0}}))
            .validate()
            .is_err());
        assert!(
            serde_json::from_value::<GatewayPreferences>(json!({"authorization":"secret"}))
                .is_err()
        );
    }
}
