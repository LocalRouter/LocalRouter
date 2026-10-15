//! Cached-only route inspection. No model discovery, classifier or pricing IO.
use crate::{decision_routing, Router};
use lr_config::{RoutingPolicy, Strategy};
use lr_providers::gateway::GatewayPreferences;
use lr_providers::{Capability, CompletionRequest, EndpointType, PreComputedRouting};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CandidateExplanation {
    pub provider: String,
    pub model: String,
    pub priority: usize,
    /// Eligible under known filters; runtime checks may still reject it.
    pub eligible: bool,
    pub exclusions: Vec<String>,
    pub unknown_checks: Vec<String>,
    pub retry_after_secs: Option<u64>,
    pub gateway_preferences: Option<GatewayPreferences>,
    pub pricing: Option<PriceEstimate>,
    pub ordering: String,
    pub score: Option<f64>,
    pub estimated_request_cost_usd: Option<f64>,
    pub estimated_output_tokens: u32,
    pub execution_priority: usize,
    pub observed_performance: crate::adaptive::ObservedPerformance,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PriceEstimate {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub source: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RouteExplanation {
    pub route: String,
    pub candidates: Vec<CandidateExplanation>,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoutingExplanation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<Value>,
    pub policy_fingerprint: String,
    pub config_fingerprint: String,
    pub active_route: Option<String>,
    pub classifier_required: bool,
    pub reason: String,
    pub routes: Vec<RouteExplanation>,
    pub no_route: bool,
    pub runtime_checks: Vec<String>,
}

/// Canonical object ordering, retaining order in arrays such as model priority.
fn canonical(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: std::collections::BTreeMap<_, _> = map
                .into_iter()
                .map(|(key, value)| (key, canonical(value)))
                .collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonical).collect()),
        value => value,
    }
}
pub fn fingerprint(value: Value) -> String {
    format!(
        "{:x}",
        Sha256::digest(canonical(value).to_string().as_bytes())
    )
}

type AutoModelSelection = (
    Vec<(String, String)>,
    Vec<CandidateExplanation>,
    Option<crate::session_routing::SessionHandle>,
);

impl Router {
    /// Same cached endpoint rules as execution, without writing negative entries.
    fn cached_endpoint_exclusion(
        &self,
        provider: &str,
        model: &str,
        endpoint: EndpointType,
    ) -> Option<&'static str> {
        if self
            .endpoint_cache
            .is_known_unsupported(provider, model, endpoint)
        {
            return Some("endpoint_cached_unsupported");
        }
        if let Some(instance) = self.provider_registry.get_provider(provider) {
            if !endpoint.is_supported_by_provider(instance.as_ref()) {
                return Some("provider_endpoint_unsupported");
            }
        }
        if let Some(capabilities) = self
            .provider_registry
            .get_model_capabilities(provider, model)
        {
            if !endpoint.is_compatible_with(&capabilities) {
                return Some("model_endpoint_unsupported");
            }
        }
        None
    }

    pub fn assess_chat_candidate(
        &self,
        strategy: &Strategy,
        provider: &str,
        model: &str,
        request: &CompletionRequest,
        priority: usize,
    ) -> CandidateExplanation {
        self.assess_candidate(
            strategy,
            provider,
            model,
            request,
            priority,
            request.endpoint_type(),
        )
    }

    /// Shared static filters. Do not poll quotas or fetch missing metadata here.
    pub fn assess_candidate(
        &self,
        strategy: &Strategy,
        provider: &str,
        model: &str,
        request: &CompletionRequest,
        priority: usize,
        endpoint: EndpointType,
    ) -> CandidateExplanation {
        let input_tokens = lr_providers::usage_estimate::prompt_tokens(
            &request.messages,
            request.tools.as_deref(),
        );
        self.assess_candidate_with_input_tokens(
            strategy,
            provider,
            model,
            request,
            priority,
            endpoint,
            input_tokens,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn assess_candidate_with_input_tokens(
        &self,
        strategy: &Strategy,
        provider: &str,
        model: &str,
        request: &CompletionRequest,
        priority: usize,
        endpoint: EndpointType,
        input_tokens: u64,
    ) -> CandidateExplanation {
        let adaptive = strategy
            .auto_config
            .as_ref()
            .map(|auto| auto.adaptive_routing.clone())
            .unwrap_or_default();
        let observed = self.adaptive_measurements.snapshot(
            provider,
            model,
            endpoint,
            &adaptive,
            self.provider_registry.routing_revision(),
        );
        let mut result = CandidateExplanation {
            provider: provider.into(),
            model: model.into(),
            priority,
            eligible: true,
            exclusions: Vec::new(),
            unknown_checks: vec!["runtime_readiness".into(), "provider_quota".into()],
            retry_after_secs: None,
            gateway_preferences: None,
            pricing: None,
            ordering: serde_json::to_value(adaptive.ordering)
                .unwrap()
                .as_str()
                .unwrap()
                .into(),
            score: None,
            estimated_request_cost_usd: None,
            estimated_output_tokens: request.max_tokens.unwrap_or(1024),
            execution_priority: priority,
            observed_performance: observed,
        };
        let mut force_privacy_zdr = false;
        if let Some(value) = request
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.get(lr_providers::privacy::INTERNAL_EXTENSION))
        {
            match serde_json::from_value::<lr_config::ClientPrivacyConfig>(value.clone()) {
                Ok(policy) => {
                    let assessment = self.provider_registry.privacy_assessment(provider, &policy);
                    force_privacy_zdr = assessment.requires_gateway_zdr;
                    if !assessment.allowed {
                        result.exclusions.push("privacy_policy_unverified".into());
                    }
                    if assessment.requires_gateway_zdr
                        && !matches!(endpoint, EndpointType::Chat | EndpointType::Messages)
                    {
                        result
                            .exclusions
                            .push("privacy_controls_unsupported".into());
                    }
                }
                Err(_) => result.exclusions.push("invalid_privacy_policy".into()),
            }
        }
        if !strategy.is_model_allowed(provider, model) {
            result.exclusions.push("model_not_allowed".into());
        }
        if self.provider_registry.is_model_disabled(provider, model) {
            result.exclusions.push("model_disabled".into());
        }
        if strategy
            .auto_config
            .as_ref()
            .is_none_or(|auto| !auto.permission.is_enabled())
        {
            result.exclusions.push("auto_router_disabled".into());
        }
        let instance = self.provider_registry.get_provider(provider);
        if let Some(instance) = instance.as_ref() {
            if lr_providers::messages::validate_dispatch(instance.as_ref(), request).is_err() {
                result
                    .exclusions
                    .push("messages_transport_unsupported".into());
            }
        }
        if instance.is_none() {
            result.exclusions.push(
                if self.provider_registry.is_provider_enabled(provider) == Some(false) {
                    "provider_disabled"
                } else {
                    "provider_not_found"
                }
                .into(),
            );
        }
        // Permissions are resolved before inspecting a provider's controls.
        if result.exclusions.is_empty() {
            if let Some(instance) = instance {
                // No prompt copy: routing-control preflight needs scalar options only.
                let mut controls = CompletionRequest::new(model, Vec::new());
                controls.extensions = request.extensions.clone();
                if force_privacy_zdr {
                    let required = GatewayPreferences {
                        zdr: Some(true),
                        data_collection: Some(lr_providers::gateway::DataCollection::Deny),
                        ..Default::default()
                    };
                    match required.effective(controls.extensions.as_ref()) {
                        Ok(effective) => {
                            controls
                                .extensions
                                .get_or_insert_with(Default::default)
                                .insert(
                                    lr_providers::gateway::EXTENSION.into(),
                                    serde_json::to_value(effective).unwrap(),
                                );
                        }
                        Err(_) => result.exclusions.push("privacy_controls_conflict".into()),
                    }
                }
                controls.service_tier = request.service_tier.clone();
                controls.reasoning_effort = request.reasoning_effort.clone();
                let known_native = self.provider_registry.model_has_capability(
                    provider,
                    model,
                    &Capability::Decision,
                ) == Some(true)
                    || !instance.supports_chat();
                let preferences = if endpoint == EndpointType::SystemOne && known_native {
                    instance.effective_native_gateway_preferences(&controls)
                } else {
                    instance.effective_gateway_preferences(&controls)
                };
                match preferences {
                    Ok(preferences) => result.gateway_preferences = preferences,
                    Err(_) => result.exclusions.push(
                        if instance.supports_gateway_options() {
                            "gateway_constraints_ineligible"
                        } else {
                            "gateway_controls_unsupported"
                        }
                        .into(),
                    ),
                }
            }
        }
        if let Some(reason) = self.cached_endpoint_exclusion(provider, model, endpoint) {
            result.exclusions.push(reason.into());
        }
        let capabilities = self
            .provider_registry
            .get_model_capabilities(provider, model);
        if capabilities.is_none() {
            result.unknown_checks.push("model_capabilities".into());
        }
        if request
            .tools
            .as_ref()
            .is_some_and(|tools| !tools.is_empty())
        {
            match self.provider_registry.model_has_capability(
                provider,
                model,
                &Capability::FunctionCalling,
            ) {
                Some(false) => result.exclusions.push("no_tool_support".into()),
                None => result.unknown_checks.push("tool_support".into()),
                _ => {}
            }
        }
        if endpoint == EndpointType::SystemOne {
            result
                .unknown_checks
                .push("native_protocol_and_gateway_options".into());
        }
        if request
            .messages
            .iter()
            .any(|message| message.content.has_images())
        {
            if endpoint == EndpointType::SystemOne
                && self
                    .provider_registry
                    .get_provider(provider)
                    .is_some_and(|instance| !instance.supports_decisions())
            {
                result.exclusions.push("image_protocol_unsupported".into());
            }
            match capabilities
                .as_ref()
                .map(|caps| caps.contains(&Capability::Vision))
            {
                Some(false) => result.exclusions.push("no_image_support".into()),
                None => result.unknown_checks.push("image_support".into()),
                _ => {}
            }
        }
        if let Some(backoff) = self.free_tier_manager.is_in_backoff(provider, model) {
            result.exclusions.push("backoff".into());
            result.retry_after_secs = Some(backoff.retry_after_secs);
        }
        if self
            .check_strategy_rate_limits(strategy, provider, model)
            .is_err()
        {
            result.exclusions.push("strategy_rate_limit".into());
        }
        if strategy.free_tier_only {
            let kind = self.get_effective_free_tier(provider);
            let evaluator = lr_providers::free_models::FreeTierEvaluator::new(&kind);
            if evaluator.needs_metadata() && !self.provider_registry.has_cached_models(provider) {
                result.unknown_checks.push("free_model_status".into());
            } else {
                let free = self.free_model_decision_cached(provider, model, &evaluator);
                if !free.is_free {
                    result.exclusions.push("not_free".into());
                }
            }
            if let Some(seconds) = self.free_tier_manager.check_cost_backoff(provider) {
                result.exclusions.push("cost_backoff".into());
                result.retry_after_secs = Some(seconds);
            }
        }
        let config = self.config_manager.get();
        result.pricing = self
            .provider_registry
            .effective_pricing_override(provider, model, &config.pricing_overrides)
            .map(|price| PriceEstimate {
                input_per_million: price.input_per_million,
                output_per_million: price.output_per_million,
                source: "configured_estimate".into(),
            })
            .or_else(|| {
                self.provider_registry
                    .model_metadata(provider, model)
                    .as_ref()
                    .and_then(lr_providers::listing_metadata::listing_pricing)
                    .map(|price| PriceEstimate {
                        input_per_million: price.input_cost_per_1k * 1000.0,
                        output_per_million: price.output_cost_per_1k * 1000.0,
                        source: "provider_listing_estimate".into(),
                    })
            })
            .filter(|price| {
                price.input_per_million.is_finite()
                    && price.output_per_million.is_finite()
                    && price.input_per_million >= 0.0
                    && price.output_per_million >= 0.0
            });
        let tier = result
            .gateway_preferences
            .as_ref()
            .and_then(|prefs| prefs.service_tier.as_deref())
            .or(request.service_tier.as_deref());
        if tier.is_some_and(|tier| !matches!(tier, "default" | "auto")) {
            result.pricing = None;
            result.unknown_checks.push("service_tier_pricing".into());
        }
        if let Some(pricing) = &result.pricing {
            let input = input_tokens;
            result.estimated_request_cost_usd = Some(
                (pricing.input_per_million * input as f64
                    + pricing.output_per_million * result.estimated_output_tokens as f64)
                    / 1_000_000.0,
            )
            .filter(|value| value.is_finite());
        }
        if adaptive.infrastructure_cooldown && result.observed_performance.retry_after_ms.is_some()
        {
            result.exclusions.push("infrastructure_cooldown".into());
            result.retry_after_secs = result
                .observed_performance
                .retry_after_ms
                .map(|ms| ms.div_ceil(1000));
        }
        if result.pricing.is_none() {
            result.unknown_checks.push("pricing".into());
        }
        result.eligible = result.exclusions.is_empty();
        if result.eligible {
            result.score = crate::adaptive::ordering_score(
                adaptive.ordering,
                &result.observed_performance,
                result.estimated_request_cost_usd,
                priority,
            );
        }
        result
    }

    pub(crate) fn deadline_routing_error(
        &self,
        strategy: &Strategy,
        request: &CompletionRequest,
        endpoint: EndpointType,
        reason: &str,
    ) -> lr_types::AppError {
        let policy = strategy
            .auto_config
            .as_ref()
            .and_then(|auto| auto.routing_policy.as_ref())
            .cloned()
            .unwrap_or_default();
        let decision = self.deterministic_routing_decision(strategy, &policy, request);
        let mut metadata = super::build_routing_metadata(decision.as_ref(), &[], Vec::new(), None);
        let mut explanation = self.routing_explanation_for_endpoint(
            strategy,
            &policy,
            request,
            decision.as_ref(),
            endpoint,
        );
        explanation.reason = reason.into();
        metadata["explanation"] = json!(explanation);
        metadata["outcome"] = json!(reason);
        if let Some(actual) = &request.pre_computed_routing {
            metadata["classifier_latency_ms"] = json!(actual.latency_ms);
            metadata["classifier_cost_usd"] = json!(actual.classifier_cost_usd);
            metadata["classifier_cost_estimated"] = json!(actual.classifier_cost_estimated);
        }
        lr_types::AppError::RoutingFailure {
            error: Box::new(lr_types::AppError::RoutingDeadlineExceeded { scope: "total" }),
            explanation: Box::new(metadata),
        }
    }

    pub(crate) fn ranked_candidates(
        &self,
        strategy: &Strategy,
        policy: &RoutingPolicy,
        route: &str,
        models: &[(String, String)],
        request: &CompletionRequest,
        endpoint: EndpointType,
    ) -> Vec<CandidateExplanation> {
        let baseline = strategy
            .auto_config
            .as_ref()
            .map(|auto| auto.prioritized_models.as_slice())
            .unwrap_or_default();
        let groups: Vec<&[(String, String)]> = if policy.enabled {
            [route, policy.default_route.as_str()]
                .into_iter()
                .filter_map(|id| policy.options.iter().find(|option| option.id == id))
                .map(|option| {
                    if option.models.is_empty() {
                        baseline
                    } else {
                        option.models.as_slice()
                    }
                })
                .chain(std::iter::once(baseline))
                .collect()
        } else {
            vec![baseline]
        };
        let group = |candidate: &CandidateExplanation| {
            groups
                .iter()
                .position(|models| {
                    models.iter().any(|(provider, model)| {
                        provider == &candidate.provider && model == &candidate.model
                    })
                })
                .unwrap_or(groups.len())
        };
        let input_tokens = lr_providers::usage_estimate::prompt_tokens(
            &request.messages,
            request.tools.as_deref(),
        );
        let mut candidates: Vec<_> = models
            .iter()
            .enumerate()
            .map(|(priority, (provider, model))| {
                self.assess_candidate_with_input_tokens(
                    strategy,
                    provider,
                    model,
                    request,
                    priority,
                    endpoint,
                    input_tokens,
                )
            })
            .collect();
        if strategy.auto_config.as_ref().is_none_or(|auto| {
            auto.adaptive_routing.ordering == lr_config::CandidateOrdering::Priority
        }) {
            return candidates;
        }
        candidates.sort_by(|a, b| {
            group(a)
                .cmp(&group(b))
                .then_with(|| b.eligible.cmp(&a.eligible))
                .then_with(|| match (a.score, b.score) {
                    (Some(a), Some(b)) => a.total_cmp(&b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    _ => std::cmp::Ordering::Equal,
                })
                .then(a.priority.cmp(&b.priority))
        });
        for (execution_priority, candidate) in candidates.iter_mut().enumerate() {
            candidate.execution_priority = execution_priority;
        }
        candidates
    }
    /// Apply only a cached session preference; never create/reset a live session.
    pub fn apply_session_preview(
        &self,
        client_id: &str,
        strategy: &Strategy,
        request: &CompletionRequest,
        endpoint: EndpointType,
        explanation: &mut RoutingExplanation,
    ) -> lr_types::AppResult<()> {
        let mut session = self.preview_session(client_id, strategy, request, endpoint)?;
        let pin = session
            .as_ref()
            .and_then(|s| s.get("pin"))
            .filter(|pin| pin["eligible"] == true)
            .and_then(|pin| {
                Some((
                    pin["provider"].as_str()?.to_string(),
                    pin["model"].as_str()?.to_string(),
                    pin["route"].as_str()?.to_string(),
                ))
            });
        let bound = lr_providers::session::SessionOptions::from_request(request)?
            .is_some_and(|o| o.require_continuity);
        let pinned_route_name = pin.as_ref().map(|(_, _, route)| route.clone());
        if let Some((provider, model, pinned_route)) = pin {
            let policy = strategy
                .auto_config
                .as_ref()
                .and_then(|a| a.routing_policy.as_ref())
                .cloned()
                .unwrap_or_default();
            let baseline = strategy
                .auto_config
                .as_ref()
                .map(|a| a.prioritized_models.as_slice())
                .unwrap_or_default();
            if explanation
                .active_route
                .as_ref()
                .is_some_and(|route| route != &pinned_route)
            {
                if let Some(session) = session.as_mut() {
                    session["status"] = json!("route_would_change");
                }
            }
            for route in &mut explanation.routes {
                if route.route != pinned_route {
                    continue;
                }
                let groups: Vec<_> = if policy.enabled {
                    [route.route.as_str(), policy.default_route.as_str()]
                        .into_iter()
                        .filter_map(|id| policy.options.iter().find(|o| o.id == id))
                        .map(|o| {
                            if o.models.is_empty() {
                                baseline
                            } else {
                                o.models.as_slice()
                            }
                        })
                        .chain(std::iter::once(baseline))
                        .collect()
                } else {
                    vec![baseline]
                };
                let group = |p: &str, m: &str| {
                    groups
                        .iter()
                        .position(|g| g.iter().any(|(gp, gm)| gp == p && gm == m))
                        .unwrap_or(groups.len())
                };
                let Some(index) = route
                    .candidates
                    .iter()
                    .position(|c| c.provider == provider && c.model == model)
                else {
                    continue;
                };
                let pin_group = group(&provider, &model);
                let unsupported_gateway = self
                    .provider_registry
                    .get_provider(&provider)
                    .is_some_and(|p| p.supports_gateway_options());
                if bound
                    && (unsupported_gateway
                        || route
                            .candidates
                            .iter()
                            .any(|c| c.eligible && group(&c.provider, &c.model) < pin_group))
                {
                    explanation.no_route = true;
                    if let Some(session) = session.as_mut() {
                        session["continuity_status"] =
                            json!("reset_required_or_unsupported_gateway");
                    }
                    continue;
                }
                if bound {
                    route
                        .candidates
                        .retain(|c| c.provider == provider && c.model == model);
                } else {
                    let first = route
                        .candidates
                        .iter()
                        .position(|c| group(&c.provider, &c.model) == pin_group)
                        .unwrap_or(index);
                    let candidate = route.candidates.remove(index);
                    route.candidates.insert(first, candidate);
                }
                for (index, candidate) in route.candidates.iter_mut().enumerate() {
                    candidate.execution_priority = index;
                }
            }
        }
        if bound {
            explanation.classifier_required = false;
            if let Some(route) = pinned_route_name.filter(|route| {
                !explanation.no_route && explanation.routes.iter().any(|r| &r.route == route)
            }) {
                explanation
                    .routes
                    .retain(|candidate| candidate.route == route);
                explanation.active_route = Some(route);
                explanation.reason = "provider_bound_session".into();
            } else {
                explanation.no_route = true;
                explanation.reason = "session_reset_required".into();
            }
        }
        explanation.session = session;
        Ok(())
    }

    pub(crate) fn order_auto_models(
        &self,
        client_id: &str,
        strategy: &Strategy,
        request: &CompletionRequest,
        models: &[(String, String)],
        decision: Option<&PreComputedRouting>,
        endpoint: EndpointType,
    ) -> lr_types::AppResult<AutoModelSelection> {
        let policy = strategy
            .auto_config
            .as_ref()
            .and_then(|auto| auto.routing_policy.as_ref())
            .cloned()
            .unwrap_or_default();
        let route = decision
            .map(|decision| decision.route.as_str())
            .unwrap_or("baseline");
        let mut reports =
            self.ranked_candidates(strategy, &policy, route, models, request, endpoint);
        let mut models: Vec<(String, String)> = reports
            .iter()
            .map(|candidate| (candidate.provider.clone(), candidate.model.clone()))
            .collect();
        let handle = self.session_handle(client_id, strategy, request, endpoint, route)?;
        if let Some(handle) = &handle {
            let baseline = strategy
                .auto_config
                .as_ref()
                .map(|a| a.prioritized_models.as_slice())
                .unwrap_or_default();
            let groups: Vec<&[(String, String)]> = if policy.enabled {
                [route, policy.default_route.as_str()]
                    .into_iter()
                    .filter_map(|id| policy.options.iter().find(|o| o.id == id))
                    .map(|o| {
                        if o.models.is_empty() {
                            baseline
                        } else {
                            o.models.as_slice()
                        }
                    })
                    .chain(std::iter::once(baseline))
                    .collect()
            } else {
                vec![baseline]
            };
            handle.prioritize(
                &mut models,
                |p, m| {
                    reports
                        .iter()
                        .any(|r| r.provider == p && r.model == m && r.eligible)
                },
                |p, m| {
                    groups
                        .iter()
                        .position(|g| g.iter().any(|(gp, gm)| gp == p && gm == m))
                        .unwrap_or(groups.len())
                },
            )?;
            reports.retain(|r| {
                models
                    .iter()
                    .any(|(p, m)| p == &r.provider && m == &r.model)
            });
            reports.sort_by_key(|r| {
                models
                    .iter()
                    .position(|(p, m)| p == &r.provider && m == &r.model)
                    .unwrap_or(usize::MAX)
            });
            for (index, report) in reports.iter_mut().enumerate() {
                report.execution_priority = index;
            }
        }
        Ok((models, reports, handle))
    }

    pub(crate) fn attach_routing_explanation(
        &self,
        metadata: &mut Value,
        strategy: &Strategy,
        request: &CompletionRequest,
        decision: Option<&PreComputedRouting>,
    ) {
        let default_policy = RoutingPolicy::default();
        let policy = strategy
            .auto_config
            .as_ref()
            .and_then(|auto| auto.routing_policy.as_ref())
            .unwrap_or(&default_policy);
        metadata["explanation"] =
            json!(self.routing_explanation(strategy, policy, request, decision));
        if let Some(decision) = decision.or(request.pre_computed_routing.as_ref()) {
            metadata["classifier_latency_ms"] = json!(decision.latency_ms);
            metadata["classifier_cost_usd"] = json!(decision.classifier_cost_usd);
            metadata["classifier_cost_estimated"] = json!(decision.classifier_cost_estimated);
        }
    }

    /// Determine exact client rules; leave semantic results unknown without IO.
    pub fn deterministic_routing_decision(
        &self,
        strategy: &Strategy,
        policy: &RoutingPolicy,
        request: &CompletionRequest,
    ) -> Option<PreComputedRouting> {
        if let Some(decision) =
            decision_routing::deterministic_decision(policy, request.metadata.as_ref())
        {
            return Some(decision);
        }
        let (provider, model) = policy.decision_model.as_ref()?;
        if !strategy.is_model_allowed(provider, model) {
            return Some(decision_routing::fallback(
                policy,
                "decision_model_not_allowed",
            ));
        }
        if self.provider_registry.is_model_disabled(provider, model) {
            return Some(decision_routing::fallback(
                policy,
                "decision_model_disabled",
            ));
        }
        if self.provider_registry.get_provider(provider).is_none() {
            return Some(decision_routing::fallback(
                policy,
                "decision_provider_unavailable",
            ));
        }
        None
    }

    pub fn routing_explanation(
        &self,
        strategy: &Strategy,
        policy: &RoutingPolicy,
        request: &CompletionRequest,
        decision: Option<&PreComputedRouting>,
    ) -> RoutingExplanation {
        self.routing_explanation_for_endpoint(
            strategy,
            policy,
            request,
            decision,
            EndpointType::Chat,
        )
    }

    pub fn routing_explanation_for_endpoint(
        &self,
        strategy: &Strategy,
        policy: &RoutingPolicy,
        request: &CompletionRequest,
        decision: Option<&PreComputedRouting>,
        endpoint: EndpointType,
    ) -> RoutingExplanation {
        let baseline = strategy
            .auto_config
            .as_ref()
            .map(|auto| auto.prioritized_models.as_slice())
            .unwrap_or_default();
        let classifier_required = policy.enabled && decision.is_none();
        let routes = if !policy.enabled {
            vec![("baseline".to_string(), baseline.to_vec())]
        } else if let Some(decision) = decision {
            vec![(
                decision.route.clone(),
                decision_routing::route_models(policy, baseline, &decision.route),
            )]
        } else {
            policy
                .options
                .iter()
                .map(|option| {
                    (
                        option.id.clone(),
                        decision_routing::route_models(policy, baseline, &option.id),
                    )
                })
                .collect()
        };
        let routes: Vec<RouteExplanation> = routes
            .into_iter()
            .map(|(route, models)| {
                let candidates =
                    self.ranked_candidates(strategy, policy, &route, &models, request, endpoint);
                RouteExplanation { route, candidates }
            })
            .collect();
        // No credentials, prompts, raw headers or transient cooldown data enter this digest.
        let projection: Vec<Value> = routes.iter().map(|route| json!({"route":route.route,"models":route.candidates.iter().map(|candidate| json!({"provider":candidate.provider,"model":candidate.model,"provider_type":self.provider_registry.get_provider_type_for_instance(&candidate.provider),"provider_enabled":self.provider_registry.is_provider_enabled(&candidate.provider),"gateway_preferences":candidate.gateway_preferences,"pricing":candidate.pricing,"capabilities":self.provider_registry.get_model_capabilities(&candidate.provider,&candidate.model),"disabled":self.provider_registry.is_model_disabled(&candidate.provider,&candidate.model)})).collect::<Vec<_>>()})).collect();
        let no_route = routes
            .iter()
            .all(|route| route.candidates.iter().all(|candidate| !candidate.eligible));
        RoutingExplanation {
            session: None,
            policy_fingerprint: fingerprint(json!(policy)),
            config_fingerprint: fingerprint(
                json!({"provider_revision":self.provider_registry.routing_revision(),"adaptive":strategy.auto_config.as_ref().map(|auto| &auto.adaptive_routing),"session_routing":strategy.auto_config.as_ref().map(|auto| &auto.session_routing),"policy":policy,"permissions":strategy.model_permissions,"free_tier_only":strategy.free_tier_only,"limits":strategy.rate_limits,"routes":projection}),
            ),
            active_route: policy
                .enabled
                .then(|| decision.map(|decision| decision.route.clone()))
                .flatten(),
            classifier_required,
            reason: if !policy.enabled {
                "policy_disabled".into()
            } else {
                decision
                    .map(|decision| decision.reason.clone())
                    .unwrap_or_else(|| "classifier_not_executed".into())
            },
            routes,
            no_route,
            runtime_checks: vec![
                "client_quota".into(),
                "provider_quota".into(),
                "runtime_readiness".into(),
                "actual_pricing_and_upstream_identity".into(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fingerprints_ignore_map_insertion_order_but_preserve_model_priority() {
        assert_eq!(
            fingerprint(json!({"a":1,"nested":{"x":1,"y":2}})),
            fingerprint(json!({"nested":{"y":2,"x":1},"a":1}))
        );
        assert_ne!(
            fingerprint(json!({"models":["a","b"]})),
            fingerprint(json!({"models":["b","a"]}))
        );
    }
}
