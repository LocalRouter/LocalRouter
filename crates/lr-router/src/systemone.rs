//! Routing for `POST /v1/systemone` (System One typed decisions).
//!
//! A request is served natively by providers that speak the protocol
//! (`supports_systemone()`), or translated onto chat completions for any
//! chat-capable provider (see `lr_providers::systemone::emulation`).

use futures::future::join_all;
use tracing::{debug, error, info, warn};

use lr_config::SystemOneEmulation;
use lr_providers::systemone::emulation;
use lr_providers::systemone::types::option_count;
use lr_providers::{
    CompletionResponse, EndpointType, ModelProvider, PricingInfo, SystemOneBackend,
    SystemOneRequest, SystemOneResponse, SystemOneUsage,
};
use lr_types::{AppError, AppResult};

use crate::{calculate_cost, free_tier, Router, RouterError, UsageInfo};

/// Error message used when a provider can answer neither natively nor by
/// translation. `RouterError::classify` maps it to `EndpointNotSupported`.
fn not_supported(provider: &str) -> AppError {
    AppError::Provider(format!(
        "Provider '{}' does not support system one decisions",
        provider
    ))
}

fn chat_cost(resp: &CompletionResponse, pricing: &PricingInfo) -> f64 {
    let reasoning = resp
        .usage
        .completion_tokens_details
        .as_ref()
        .and_then(|d| d.reasoning_tokens.or(d.thinking_tokens))
        .map(|t| t as u64);
    if let Some(cost) = lr_providers::gateway::reported_cost(resp.extensions.as_ref()) {
        return cost;
    }
    calculate_cost(
        resp.usage.prompt_tokens as u64,
        resp.usage.completion_tokens as u64,
        reasoning,
        pricing,
    )
}

impl Router {
    /// Route a System One request.
    ///
    /// Model resolution:
    /// - `provider/model` → that provider instance
    /// - bare id (e.g. `jev-latest`) → resolved across allowed providers
    /// - `localrouter/auto` → the strategy's prioritized models
    /// - omitted → the only System One provider the client may use, else auto
    pub async fn systemone(
        &self,
        client_id: &str,
        request: SystemOneRequest,
    ) -> AppResult<SystemOneResponse> {
        debug!(
            "Routing System One request for client '{}', model {:?}",
            client_id, request.model
        );

        // Internal test token: direct provider access, no strategy.
        if client_id == "internal-test" || client_id == "memory-service" {
            let requested = request.model.clone().unwrap_or_default();
            let (provider, model) = Self::parse_model_string(&requested);
            if provider.is_empty() {
                return Err(AppError::Router(
                    "Internal test requires provider/model format".into(),
                ));
            }
            return self
                .execute_systemone_request(client_id, &provider, Some(&model), request, false)
                .await;
        }

        let (_client, strategy) = self.validate_client_and_strategy(client_id)?;
        self.check_client_rate_limits(client_id).await?;
        self.route_systemone(client_id, &strategy, request).await
    }

    /// Retry a System One request after the user approved paying once the
    /// free tier is exhausted: same routing with `free_tier_only` disabled.
    pub async fn systemone_with_paid_fallback(
        &self,
        client_id: &str,
        request: SystemOneRequest,
    ) -> AppResult<SystemOneResponse> {
        let (_client, mut strategy) = self.validate_client_and_strategy(client_id)?;
        strategy.free_tier_only = false;
        self.route_systemone(client_id, &strategy, request).await
    }

    pub(super) async fn route_systemone(
        &self,
        client_id: &str,
        strategy: &lr_config::Strategy,
        request: SystemOneRequest,
    ) -> AppResult<SystemOneResponse> {
        let extensions = request
            .extra
            .get("extensions")
            .and_then(serde_json::Value::as_object)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
        lr_providers::gateway::GatewayPreferences::default().effective(extensions.as_ref())?;
        let strategy = strategy.clone();
        let requested = match request.model.as_deref() {
            Some(m) => m.to_string(),
            None => match self.single_native_systemone_provider(&strategy) {
                Some(provider) => {
                    debug!(
                        "No model given; using the only System One provider '{}'",
                        provider
                    );
                    self.check_strategy_rate_limits(&strategy, "", "")?;
                    return self
                        .execute_systemone_request(
                            client_id,
                            &provider,
                            None,
                            request,
                            strategy.free_tier_only,
                        )
                        .await;
                }
                None => "localrouter/auto".to_string(),
            },
        };

        if requested == "localrouter/auto" {
            return self
                .systemone_with_auto_routing(client_id, &strategy, request)
                .await;
        }

        self.check_strategy_rate_limits(&strategy, "", "")?;

        let (provider, model) = Self::parse_model_string(&requested);
        let (final_provider, final_model) = if provider.is_empty() {
            self.find_provider_for_model(&model, &strategy).await?
        } else {
            if !strategy.is_model_allowed(&provider, &model) {
                return Err(AppError::Router(format!(
                    "Model '{}/{}' is not allowed by this strategy",
                    provider, model
                )));
            }
            (provider, model)
        };

        if strategy.free_tier_only {
            let exhausted = vec![(final_provider.clone(), final_model.clone())];
            if let Some(backoff) = self
                .free_tier_manager
                .is_in_backoff(&final_provider, &final_model)
            {
                return Err(Self::free_tier_exhausted_error(
                    &strategy,
                    backoff.retry_after_secs,
                    exhausted,
                ));
            }
            if matches!(
                self.classify_for_free_tier(&final_provider, &final_model)
                    .await,
                free_tier::ModelFreeStatus::NotFree
            ) {
                return Err(Self::free_tier_exhausted_error(&strategy, 0, exhausted));
            }
        }

        self.execute_systemone_request(
            client_id,
            &final_provider,
            Some(&final_model),
            request,
            strategy.free_tier_only,
        )
        .await
    }

    /// The single provider instance with native System One support that the
    /// strategy allows, if there is exactly one.
    fn single_native_systemone_provider(&self, strategy: &lr_config::Strategy) -> Option<String> {
        let mut found: Vec<String> = self
            .provider_registry
            .list_providers()
            .into_iter()
            .filter(|p| p.enabled)
            .map(|p| p.instance_name)
            .filter(|name| {
                strategy
                    .model_permissions
                    .has_any_enabled_for_provider(name)
            })
            .filter(|name| {
                self.provider_registry
                    .get_provider(name)
                    .map(|p| p.supports_systemone())
                    .unwrap_or(false)
            })
            .collect();
        found.sort();
        found.dedup();
        if found.len() == 1 {
            found.pop()
        } else {
            None
        }
    }

    /// Execute a System One request on one provider instance, natively or by
    /// translation. `model` is the bare model id (no provider prefix); `None`
    /// lets a native provider use its own default.
    pub(crate) async fn execute_systemone_request(
        &self,
        client_id: &str,
        provider: &str,
        model: Option<&str>,
        mut request: SystemOneRequest,
        free_tier_only: bool,
    ) -> AppResult<SystemOneResponse> {
        if let Some(model) = model {
            self.require_enabled_model(provider, model)?;
        }
        let provider_instance = self
            .provider_registry
            .get_provider(provider)
            .ok_or_else(|| {
                AppError::Router(format!(
                    "Provider '{}' not found or disabled in registry",
                    provider
                ))
            })?;

        if request
            .extra
            .get("extensions")
            .and_then(|v| v.get(lr_providers::gateway::EXTENSION))
            .is_some()
            && !provider_instance.supports_gateway_options()
        {
            return Err(AppError::InvalidParams(
                "Selected provider does not support gateway routing controls".into(),
            ));
        }
        if let Some(metadata) = request
            .extra
            .get_mut("metadata")
            .and_then(serde_json::Value::as_object_mut)
        {
            metadata.remove("localrouter.mode");
        }
        let native = match model {
            Some(m) => provider_instance.supports_systemone_model(m).await,
            None => provider_instance.supports_systemone(),
        };
        if request
            .decisions
            .as_ref()
            .is_some_and(|r| r.input.has_images())
        {
            if let Some(model) = model {
                if let Ok(models) = self
                    .provider_registry
                    .list_provider_models_cached(provider)
                    .await
                {
                    if models.iter().any(|m| {
                        m.id == model && !m.capabilities.contains(&lr_providers::Capability::Vision)
                    }) {
                        return Err(AppError::InvalidParams(format!(
                            "Model '{provider}/{model}' does not support image decisions"
                        )));
                    }
                }
            }
        }
        // Only a native Decisions transport may carry image input. Legacy
        // System One and chat emulation receive text, never image URLs as text.
        if request.decisions.is_some() && !(native && provider_instance.supports_decisions()) {
            request = lr_providers::systemone::decisions::DecisionRequest::legacy_request(&request)
                .map_err(AppError::InvalidParams)?;
        }
        if native {
            request.model = model.map(str::to_string);
            provider_instance
                .validate_systemone_gateway_options(&request)
                .await?;
            let prepared_pricing = if let Some(model) = model {
                Some(self.request_pricing(provider, model).await)
            } else {
                None
            };
            super::adaptive::reserve_inference_dispatch()?;
            let mut observation = self.adaptive_measurements.begin(
                provider,
                model.unwrap_or("default"),
                EndpointType::SystemOne,
                self.provider_registry.routing_revision(),
            );
            let mut response = match provider_instance.systemone(request).await {
                Ok(resp) => resp,
                Err(e) => {
                    self.record_retry_hint(provider, model.unwrap_or("default"), &e);
                    let label = model.unwrap_or("default");
                    if matches!(
                        RouterError::classify(&e, provider, label),
                        RouterError::Unreachable { .. }
                    ) {
                        observation.infrastructure_failure(
                            super::adaptive::current_infrastructure_cooldown(),
                        );
                        self.report_provider_failure(provider, &e.to_string());
                    }
                    return Err(e);
                }
            };
            observation.success();
            drop(observation);
            response.provider = provider.to_string();
            response.backend = SystemOneBackend::Native;

            let pricing_model = model.unwrap_or(response.model.as_str()).to_string();
            let pricing = if let Some(pricing) = prepared_pricing {
                pricing
            } else {
                self.request_pricing(provider, &pricing_model).await
            };
            let input = response.usage.input_tokens.unwrap_or(0);
            let output = response.usage.output_tokens.unwrap_or(0);
            let cached = response
                .usage
                .extra
                .get("input_tokens_details")
                .and_then(|v| v.get("cached_tokens"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
                .min(input);
            let cache_write = response
                .usage
                .extra
                .get("input_tokens_details")
                .and_then(|v| v.get("cache_write_tokens"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
                .min(input - cached);
            let estimated_cost =
                calculate_cost(input - cached - cache_write, output, None, &pricing)
                    + cached as f64 / 1000.0
                        * pricing
                            .cache_read_cost_per_1k
                            .unwrap_or(pricing.input_cost_per_1k)
                    + cache_write as f64 / 1000.0
                        * pricing
                            .cache_write_cost_per_1k
                            .unwrap_or(pricing.input_cost_per_1k);
            let gateway_cost =
                lr_providers::gateway::reported_cost_value(response.extra.get("extensions"));
            let native_cost = response
                .usage
                .extra
                .get("cost")
                .or_else(|| {
                    response
                        .extra
                        .get("provider_metadata")
                        .and_then(|v| v.pointer("/gateway/cost"))
                })
                .and_then(|v| {
                    v.as_f64()
                        .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
                })
                .filter(|v| v.is_finite() && *v >= 0.0);
            let reported = gateway_cost.or(native_cost);
            let cost = reported.unwrap_or(estimated_cost);
            super::adaptive::record_completed_cost(cost, reported.is_some());
            response.cost_usd = Some(cost);
            self.record_systemone_usage(
                client_id,
                provider,
                &pricing_model,
                input,
                output,
                cost,
                free_tier_only,
            )
            .await;
            return Ok(response);
        }

        if !provider_instance.supports_chat() {
            return Err(not_supported(provider));
        }
        if self.config_manager.get().systemone.emulation == SystemOneEmulation::Off {
            return Err(not_supported(provider));
        }
        let model = model.ok_or_else(|| {
            AppError::InvalidParams(format!(
                "A model is required to answer System One questions with chat provider '{}'",
                provider
            ))
        })?;
        self.emulate_systemone(
            client_id,
            provider,
            model,
            provider_instance.as_ref(),
            request,
            free_tier_only,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn record_systemone_usage(
        &self,
        client_id: &str,
        provider: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cost: f64,
        free_tier_only: bool,
    ) {
        self.completed_usage(
            client_id,
            provider,
            model,
            UsageInfo {
                input_tokens,
                output_tokens,
                cost_usd: cost,
            },
            free_tier_only,
        )
        .await;
    }

    /// Answer a System One request with a chat model.
    ///
    /// Letter mode (one call per question, probabilities from token
    /// logprobs) when the provider returns logprobs and every question has
    /// at most 26 options; otherwise, or if letter parsing fails, JSON mode
    /// (one call, self-reported probabilities).
    ///
    /// Each chat call goes through `execute_request`, so usage, free-tier
    /// accounting and provider health behave exactly like `/v1/chat/completions`.
    async fn emulate_systemone(
        &self,
        client_id: &str,
        provider: &str,
        model: &str,
        provider_instance: &dyn ModelProvider,
        request: SystemOneRequest,
        free_tier_only: bool,
    ) -> AppResult<SystemOneResponse> {
        let cfg = self.config_manager.get().systemone.clone();
        let pricing = self.request_pricing(provider, model).await;

        let mut usage = SystemOneUsage {
            input_tokens: Some(0),
            output_tokens: Some(0),
            extra: Default::default(),
        };
        let mut cost = 0.0;
        let add = |resp: &CompletionResponse, usage: &mut SystemOneUsage, cost: &mut f64| {
            usage.input_tokens =
                Some(usage.input_tokens.unwrap_or(0) + resp.usage.prompt_tokens as u64);
            usage.output_tokens =
                Some(usage.output_tokens.unwrap_or(0) + resp.usage.completion_tokens as u64);
            *cost += chat_cost(resp, &pricing);
        };

        if provider_instance.supports_feature("logprobs")
            && emulation::letter_mode_possible(&request)
        {
            let calls = request.questions.values().map(|q| {
                let k = option_count(q) as u32;
                let top = k.min(cfg.letter_top_logprobs).max(1);
                let mut chat = emulation::build_letter_request(model, &request.state, q, top);
                chat.service_tier = request
                    .extra
                    .get("service_tier")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                chat.reasoning_effort = request
                    .extra
                    .get("reasoning_effort")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                chat.extensions = request
                    .extra
                    .get("extensions")
                    .and_then(|v| v.as_object())
                    .map(|map| map.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
                self.execute_request(client_id, provider, model, chat, free_tier_only)
            });
            let results = join_all(calls).await;

            let mut distributions = Vec::with_capacity(results.len());
            let mut parsed_all = true;
            for (result, question) in results.into_iter().zip(request.questions.values()) {
                let resp = result?;
                add(&resp, &mut usage, &mut cost);
                match emulation::parse_letter_response(&resp, option_count(question)) {
                    Some(d) => distributions.push(d),
                    None => parsed_all = false,
                }
            }
            if parsed_all {
                return Ok(SystemOneResponse {
                    model: format!("{}/{}", provider, model),
                    answers: emulation::answers_from_letter_distributions(&request, &distributions),
                    usage,
                    extra: Default::default(),
                    provider: provider.to_string(),
                    request_id: None,
                    backend: SystemOneBackend::LetterLogprobs,
                    cost_usd: Some(cost),
                });
            }
            debug!(
                "System One letter mode returned no usable logprobs from {}/{}; using JSON mode",
                provider, model
            );
        }

        let mut chat = emulation::build_json_request(model, &request);
        chat.service_tier = request
            .extra
            .get("service_tier")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        chat.reasoning_effort = request
            .extra
            .get("reasoning_effort")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        chat.extensions = request
            .extra
            .get("extensions")
            .and_then(|v| v.as_object())
            .map(|map| map.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
        let resp = self
            .execute_request(client_id, provider, model, chat, free_tier_only)
            .await?;
        add(&resp, &mut usage, &mut cost);

        let text = resp
            .choices
            .first()
            .map(|c| c.message.content.as_text())
            .unwrap_or_default();
        let value = emulation::extract_json_object(&text).or_else(|| {
            let options = lr_json_repair::RepairOptions {
                syntax_repair: true,
                schema_coercion: false,
                strip_extra_fields: false,
                add_defaults: false,
                normalize_enums: false,
            };
            let repaired = lr_json_repair::repair_content(&text, None, &options);
            emulation::extract_json_object(&repaired.repaired)
        });
        let answers = value
            .ok_or_else(|| "no JSON object in the model output".to_string())
            .and_then(|v| emulation::parse_json_answers(&v, &request))
            .map_err(|e| {
                AppError::Provider(format!(
                    "systemone emulation: unparseable model output from {}/{} ({})",
                    provider, model, e
                ))
            })?;

        Ok(SystemOneResponse {
            model: format!("{}/{}", provider, model),
            answers,
            usage,
            extra: Default::default(),
            provider: provider.to_string(),
            request_id: None,
            backend: SystemOneBackend::Json,
            cost_usd: Some(cost),
        })
    }

    /// Textual classification context with image presence retained for eligibility.
    pub fn decision_routing_request(
        &self,
        request: &SystemOneRequest,
    ) -> lr_providers::CompletionRequest {
        use lr_providers::systemone::decisions::{
            DecisionContent, DecisionInput, DecisionInputPart,
        };
        use lr_providers::{ChatMessage, ChatMessageContent, ContentPart, ImageUrl};
        let mut parts = vec![ContentPart::Text {
            text: format!(
                "Decision questions: {}\nInput:",
                serde_json::to_string(&request.questions).unwrap_or_default()
            ),
        }];
        if let Some(decisions) = &request.decisions {
            match &decisions.input {
                DecisionInput::Text(text) => parts.push(ContentPart::Text { text: text.clone() }),
                DecisionInput::Messages(messages) => {
                    for message in messages {
                        match &message.content {
                            DecisionContent::Text(text) => {
                                parts.push(ContentPart::Text { text: text.clone() })
                            }
                            DecisionContent::Parts(input) => {
                                for part in input {
                                    parts.push(match part {
                                        DecisionInputPart::InputText { text } => {
                                            ContentPart::Text { text: text.clone() }
                                        }
                                        DecisionInputPart::InputImage { image_url, .. } => {
                                            ContentPart::ImageUrl {
                                                image_url: ImageUrl {
                                                    url: image_url.clone(),
                                                    detail: None,
                                                },
                                            }
                                        }
                                    });
                                }
                            }
                        }
                    }
                }
            }
        } else {
            parts.push(ContentPart::Text {
                text: request
                    .state
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| request.state.to_string()),
            });
        }
        let mut context = lr_providers::CompletionRequest::new(
            "localrouter/auto",
            vec![ChatMessage {
                role: "user".into(),
                content: ChatMessageContent::Parts(parts),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            }],
        );
        context.metadata = request
            .extra
            .get("metadata")
            .and_then(serde_json::Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(key, value)| {
                        value.as_str().map(|value| (key.clone(), value.to_string()))
                    })
                    .collect()
            });
        context.extensions = request
            .extra
            .get("extensions")
            .and_then(serde_json::Value::as_object)
            .map(|map| {
                map.iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            });
        context.service_tier = request
            .extra
            .get("service_tier")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        context.reasoning_effort = request
            .extra
            .get("reasoning_effort")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        context
    }

    /// Auto-routing for System One: tries the strategy's prioritized models
    /// in order, skipping models that can answer neither natively nor by
    /// translation.
    async fn systemone_with_auto_routing(
        &self,
        client_id: &str,
        strategy: &lr_config::Strategy,
        request: SystemOneRequest,
    ) -> AppResult<SystemOneResponse> {
        let auto_config = strategy.auto_config.as_ref().ok_or_else(|| {
            AppError::Router("localrouter/auto not configured for this strategy".into())
        })?;
        if !auto_config.permission.is_enabled() {
            return Err(AppError::Router(
                "localrouter/auto is disabled for this strategy".into(),
            ));
        }
        let routing_request = self.decision_routing_request(&request);
        let mut budget = super::adaptive::RoutingBudget::new(&auto_config.adaptive_routing);
        let (selected_models, routing_decision) = budget
            .preflight(self.select_models_for_auto_routing(
                client_id,
                strategy,
                auto_config,
                &routing_request,
            ))
            .await
            .map_err(|reason| {
                self.deadline_routing_error(
                    strategy,
                    &routing_request,
                    EndpointType::SystemOne,
                    reason,
                )
            })?;
        let (selected_models, scored_candidates) = self.order_auto_models(
            strategy,
            &routing_request,
            &selected_models,
            routing_decision.as_ref(),
            EndpointType::SystemOne,
        );
        if selected_models.is_empty() {
            return Err(AppError::Router(
                "No models configured for auto-routing".into(),
            ));
        }
        let selected_models = &selected_models;
        let candidate_models: Vec<String> = selected_models
            .iter()
            .map(|(provider, model)| format!("{provider}/{model}"))
            .collect();
        let policy = auto_config.routing_policy.clone().unwrap_or_default();
        let mut initial_explanation = serde_json::json!(self.routing_explanation_for_endpoint(
            strategy,
            &policy,
            &routing_request,
            routing_decision.as_ref(),
            EndpointType::SystemOne
        ));
        initial_explanation["routes"][0]["candidates"] = serde_json::json!(scored_candidates);
        let mut attempts = Vec::new();
        info!(
            "Auto-routing System One for client '{}' with {} prioritized models",
            client_id,
            selected_models.len()
        );

        let mut last_error = None;
        for (idx, (provider, model)) in selected_models.iter().enumerate() {
            if let Some(reason) = budget.exhausted() {
                attempts
                    .push(serde_json::json!({"provider":provider,"model":model,"outcome":reason}));
                break;
            }
            let assessment = self.assess_candidate(
                strategy,
                provider,
                model,
                &routing_request,
                idx,
                EndpointType::SystemOne,
            );
            if let Some(outcome) = assessment.exclusions.first() {
                attempts.push(serde_json::json!({"provider":provider,"model":model,"outcome":outcome,"exclusions":assessment.exclusions,"unknown_checks":assessment.unknown_checks}));
                continue;
            }
            if let Some(instance) = self.provider_registry.get_provider(provider) {
                let mut candidate_request = request.clone();
                candidate_request.model = Some(model.clone());
                if let Err(error) = budget
                    .preflight(instance.validate_systemone_gateway_options(&candidate_request))
                    .await
                    .map_err(|reason| {
                        self.deadline_routing_error(
                            strategy,
                            &routing_request,
                            EndpointType::SystemOne,
                            reason,
                        )
                    })?
                {
                    debug!("Skipping {provider}/{model}: {error}");
                    attempts.push(serde_json::json!({"provider":provider,"model":model,"outcome":"gateway_constraints_ineligible"}));
                    continue;
                }
            }
            if request
                .decisions
                .as_ref()
                .is_some_and(|r| r.input.has_images())
            {
                let Some(instance) = self.provider_registry.get_provider(provider) else {
                    continue;
                };
                if !instance.supports_decisions()
                    || !budget
                        .preflight(instance.supports_systemone_model(model))
                        .await
                        .map_err(|reason| {
                            self.deadline_routing_error(
                                strategy,
                                &routing_request,
                                EndpointType::SystemOne,
                                reason,
                            )
                        })?
                {
                    attempts.push(serde_json::json!({"provider":provider,"model":model,"outcome":"image_protocol_unsupported"}));
                    continue;
                }
                if let Ok(models) = budget
                    .preflight(self.provider_registry.list_provider_models_cached(provider))
                    .await
                    .map_err(|reason| {
                        self.deadline_routing_error(
                            strategy,
                            &routing_request,
                            EndpointType::SystemOne,
                            reason,
                        )
                    })?
                {
                    if models.iter().any(|m| {
                        m.id == *model
                            && !m.capabilities.contains(&lr_providers::Capability::Vision)
                    }) {
                        attempts.push(serde_json::json!({"provider":provider,"model":model,"outcome":"no_image_support"}));
                        continue;
                    }
                }
            }
            if self.should_skip_for_endpoint(provider, model, EndpointType::SystemOne)
                || self.provider_registry.is_model_disabled(provider, model)
            {
                continue;
            }
            if let Some(backoff) = self.free_tier_manager.is_in_backoff(provider, model) {
                debug!("Skipping {}/{}: {}", provider, model, backoff.reason);
                last_error = Some(RouterError::RateLimited {
                    provider: provider.clone(),
                    model: model.clone(),
                    retry_after_secs: backoff.retry_after_secs,
                });
                continue;
            }
            if strategy.free_tier_only {
                if matches!(
                    budget
                        .preflight(self.classify_for_free_tier(provider, model))
                        .await
                        .map_err(|reason| self.deadline_routing_error(
                            strategy,
                            &routing_request,
                            EndpointType::SystemOne,
                            reason
                        ))?,
                    free_tier::ModelFreeStatus::NotFree
                ) {
                    attempts.push(
                        serde_json::json!({"provider":provider,"model":model,"outcome":"not_free"}),
                    );
                    continue;
                }
            }
            if let Err(e) = self.check_strategy_rate_limits(strategy, provider, model) {
                warn!(
                    "Strategy rate limit exceeded for {}/{}, trying next model: {}",
                    provider, model, e
                );
                last_error = Some(RouterError::RateLimited {
                    provider: provider.clone(),
                    model: model.clone(),
                    retry_after_secs: 60,
                });
                continue;
            }

            let attempt_revision = self.provider_registry.routing_revision();
            match budget
                .dispatch(self.execute_systemone_request(
                    client_id,
                    provider,
                    Some(model),
                    request.clone(),
                    strategy.free_tier_only,
                ))
                .await
            {
                Ok(mut response) => {
                    info!(
                        "Auto-routing System One succeeded with {}/{}",
                        provider, model
                    );
                    attempts.push(
                        serde_json::json!({"provider":provider,"model":model,"outcome":"success"}),
                    );
                    let mut metadata = super::build_routing_metadata(
                        routing_decision.as_ref(),
                        &candidate_models,
                        attempts,
                        Some(idx),
                    );
                    metadata["attempt_costs"] = serde_json::json!(budget.costs());
                    metadata["inference_dispatches"] = serde_json::json!(budget.dispatch_count());
                    metadata["explanation"] = initial_explanation.clone();
                    if let Some(decision) = &routing_decision {
                        metadata["classifier_latency_ms"] = serde_json::json!(decision.latency_ms);
                        metadata["classifier_cost_usd"] =
                            serde_json::json!(decision.classifier_cost_usd);
                        metadata["classifier_cost_estimated"] =
                            serde_json::json!(decision.classifier_cost_estimated);
                    }
                    let extensions = response
                        .extra
                        .entry("extensions")
                        .or_insert_with(|| serde_json::json!({}));
                    if !extensions.is_object() {
                        *extensions = serde_json::json!({});
                    }
                    extensions["localrouter.routing"] = metadata;
                    return Ok(response);
                }
                Err(e) => {
                    if matches!(e, AppError::RoutingDeadlineExceeded { .. }) {
                        self.adaptive_measurements.record_infrastructure_failure(
                            provider,
                            model,
                            EndpointType::SystemOne,
                            attempt_revision,
                            auto_config.adaptive_routing.infrastructure_cooldown,
                        );
                    }
                    let router_error = RouterError::classify(&e, provider, model);
                    if let RouterError::RateLimited {
                        retry_after_secs, ..
                    } = &router_error
                    {
                        self.free_tier_manager.record_rate_limit_error(
                            provider,
                            model,
                            429,
                            Some(*retry_after_secs),
                            None,
                            false,
                        );
                    }
                    attempts.push(serde_json::json!({"provider":provider,"model":model,"outcome":router_error.outcome_str()}));
                    warn!(
                        "Auto-routing System One attempt failed: {}",
                        router_error.to_log_string()
                    );
                    if matches!(router_error, RouterError::EndpointNotSupported { .. }) {
                        self.endpoint_cache.record_unsupported(
                            provider,
                            model,
                            EndpointType::SystemOne,
                        );
                    }
                    let retry = router_error.should_retry() || Self::is_emulation_parse_error(&e);
                    last_error = Some(router_error);
                    if !retry {
                        error!("Non-retryable System One error, stopping auto-routing");
                        let mut metadata = super::build_routing_metadata(
                            routing_decision.as_ref(),
                            &candidate_models,
                            attempts,
                            None,
                        );
                        metadata["explanation"] = initial_explanation;
                        metadata["attempt_costs"] = serde_json::json!(budget.costs());
                        metadata["inference_dispatches"] =
                            serde_json::json!(budget.dispatch_count());
                        return Err(AppError::RoutingFailure {
                            error: Box::new(e),
                            explanation: Box::new(metadata),
                        });
                    }
                }
            }
        }

        if strategy.free_tier_only {
            let retry_after = self
                .free_tier_manager
                .get_min_retry_after(selected_models)
                .unwrap_or(60);
            return Err(Self::free_tier_exhausted_error(
                strategy,
                retry_after,
                selected_models.clone(),
            ));
        }

        let mut metadata = super::build_routing_metadata(
            routing_decision.as_ref(),
            &candidate_models,
            attempts,
            None,
        );
        metadata["explanation"] = initial_explanation;
        metadata["attempt_costs"] = serde_json::json!(budget.costs());
        metadata["inference_dispatches"] = serde_json::json!(budget.dispatch_count());
        metadata["failed_call_costs"] = serde_json::json!("unknown");
        if budget
            .deadline
            .is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
        {
            return Err(AppError::RoutingFailure {
                error: Box::new(AppError::RoutingDeadlineExceeded { scope: "total" }),
                explanation: Box::new(metadata),
            });
        }
        Err(AppError::RoutingExhausted {
            message: format!(
                "All auto-routing System One models failed. Last error: {}",
                last_error
                    .map(|error| error.outcome_str().to_string())
                    .unwrap_or_else(|| "no eligible candidate".into())
            ),
            explanation: Box::new(metadata),
        })
    }

    /// A chat model that produced output we could not turn into answers:
    /// worth trying the next model rather than failing the request.
    fn is_emulation_parse_error(e: &AppError) -> bool {
        matches!(e, AppError::Provider(msg) if msg.starts_with("systemone emulation:"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::Stream;
    use lr_config::{AppConfig, ConfigManager};
    use lr_providers::factory::{ParameterType, ProviderCategory, ProviderFactory, SetupParameter};
    use lr_providers::registry::ProviderRegistry;
    use lr_providers::{
        Capability, ChatMessage, ChatMessageContent, CompletionChoice, CompletionChunk,
        CompletionRequest, HealthStatus, Logprobs, ModelInfo, ProviderHealth, SystemOneAnswer,
        TokenLogprob, TokenUsage, TopLogprob,
    };
    use serde_json::json;
    use std::collections::HashMap;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        /// Speaks /v1/systemone natively.
        Native,
        SlowNative,
        /// Chat model that returns logprobs; always answers "B".
        ChatLogprobs,
        /// Chat model without logprobs; answers with JSON.
        ChatJson,
        /// Chat model without logprobs that answers with prose.
        ChatGarbage,
    }

    struct MockProvider {
        kind: Kind,
        chat_calls: Arc<AtomicUsize>,
        native_calls: Arc<AtomicUsize>,
    }

    fn msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".into(),
            content: ChatMessageContent::Text(text.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }
    }

    #[async_trait]
    impl ModelProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }
        async fn health_check(&self) -> ProviderHealth {
            ProviderHealth {
                status: HealthStatus::Healthy,
                latency_ms: Some(1),
                last_checked: chrono::Utc::now(),
                error_message: None,
            }
        }
        async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
            let caps = if self.kind == Kind::Native {
                vec![Capability::Decision]
            } else {
                vec![Capability::Chat]
            };
            Ok(vec![ModelInfo {
                id: "m1".into(),
                name: "m1".into(),
                provider: "mock".into(),
                parameter_count: None,
                context_window: 4096,
                supports_streaming: false,
                capabilities: caps,
                detailed_capabilities: None,
            }])
        }
        async fn list_model_listing(&self) -> AppResult<lr_providers::ModelListing> {
            Ok(lr_providers::ModelListing {
                models: self.list_models().await?,
                new_models_hidden: false,
                metadata: Arc::new(HashMap::from([(
                    "m1".to_string(),
                    json!({"id": "m1", "tier": "free", "pricing": {"prompt": "0", "completion": "0"}}),
                )])),
            })
        }
        async fn get_pricing(&self, _model: &str) -> AppResult<PricingInfo> {
            Ok(PricingInfo {
                input_cost_per_1k: 0.001,
                output_cost_per_1k: 0.002,
                reasoning_cost_per_1k: None,
                cache_read_cost_per_1k: None,
                cache_write_cost_per_1k: None,
                currency: "USD".into(),
            })
        }
        async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
            if self.kind == Kind::Native {
                return Err(AppError::Provider(
                    "Provider 'mock' does not support chat completions".into(),
                ));
            }
            self.chat_calls.fetch_add(1, Ordering::SeqCst);
            let (text, logprobs) = match self.kind {
                Kind::ChatLogprobs => {
                    assert_eq!(request.logprobs, Some(true));
                    (
                        "B".to_string(),
                        Some(Logprobs {
                            content: Some(vec![TokenLogprob {
                                token: "B".into(),
                                logprob: (0.75f64).ln(),
                                bytes: None,
                                top_logprobs: vec![
                                    TopLogprob { token: "B".into(), logprob: (0.75f64).ln(), bytes: None },
                                    TopLogprob { token: "A".into(), logprob: (0.25f64).ln(), bytes: None },
                                ],
                            }]),
                        }),
                    )
                }
                Kind::ChatJson => (
                    r#"{"answers": {"dept": {"probabilities": {"billing": 0.2, "tech": 0.8}}, "urgent": {"noul": 0.9}}}"#
                        .to_string(),
                    None,
                ),
                _ => ("I think it is probably billing.".to_string(), None),
            };
            Ok(CompletionResponse {
                id: "c".into(),
                object: "chat.completion".into(),
                created: 0,
                model: request.model,
                provider: "mock".into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    message: msg(&text),
                    finish_reason: Some("stop".into()),
                    logprobs,
                }],
                usage: TokenUsage {
                    prompt_tokens: 100,
                    completion_tokens: 10,
                    total_tokens: 110,
                    prompt_tokens_details: None,
                    completion_tokens_details: None,
                },
                system_fingerprint: None,
                service_tier: None,
                extensions: None,
                routellm_win_rate: None,
                request_usage_entries: None,
            })
        }
        async fn stream_complete(
            &self,
            _request: CompletionRequest,
        ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
            Err(AppError::Provider("no streaming".into()))
        }
        fn supports_feature(&self, feature: &str) -> bool {
            feature == "logprobs" && self.kind == Kind::ChatLogprobs
        }
        fn supports_chat(&self) -> bool {
            self.kind != Kind::Native
        }
        fn supports_systemone(&self) -> bool {
            matches!(self.kind, Kind::Native | Kind::SlowNative)
        }
        async fn supports_systemone_model(&self, model: &str) -> bool {
            // A chat provider that also serves one native decision model.
            self.supports_systemone() || model == "jev-native"
        }
        async fn systemone(&self, request: SystemOneRequest) -> AppResult<SystemOneResponse> {
            self.native_calls.fetch_add(1, Ordering::SeqCst);
            if self.kind == Kind::SlowNative {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            let answers = request
                .questions
                .iter()
                .map(|(id, q)| {
                    let n = option_count(q);
                    let mut w = vec![0.0; n];
                    w[0] = 1.0;
                    (
                        id.clone(),
                        lr_providers::systemone::answer_from_distribution(q, &w),
                    )
                })
                .collect();
            Ok(SystemOneResponse {
                model: request.model.unwrap_or_else(|| "native-default".into()),
                answers,
                usage: SystemOneUsage {
                    input_tokens: Some(1000),
                    output_tokens: Some(0),
                    extra: Default::default(),
                },
                extra: Default::default(),
                provider: String::new(),
                request_id: Some("req-1".into()),
                backend: SystemOneBackend::Native,
                cost_usd: None,
            })
        }
    }

    struct MockFactory {
        kind: Kind,
        type_name: &'static str,
        chat_calls: Arc<AtomicUsize>,
        native_calls: Arc<AtomicUsize>,
    }

    impl ProviderFactory for MockFactory {
        fn provider_type(&self) -> &str {
            self.type_name
        }
        fn display_name(&self) -> &str {
            self.type_name
        }
        fn category(&self) -> ProviderCategory {
            ProviderCategory::Generic
        }
        fn description(&self) -> &str {
            "mock"
        }
        fn setup_parameters(&self) -> Vec<SetupParameter> {
            vec![SetupParameter::optional(
                "base_url",
                ParameterType::BaseUrl,
                "unused",
                None::<String>,
                false,
            )]
        }
        fn create(
            &self,
            _instance_name: String,
            _config: HashMap<String, String>,
        ) -> AppResult<Arc<dyn ModelProvider>> {
            Ok(Arc::new(MockProvider {
                kind: self.kind,
                chat_calls: self.chat_calls.clone(),
                native_calls: self.native_calls.clone(),
            }))
        }
        fn validate_config(&self, _config: &HashMap<String, String>) -> AppResult<()> {
            Ok(())
        }
    }

    struct Harness {
        router: Router,
        client_id: String,
        chat_calls: Arc<AtomicUsize>,
        native_calls: Arc<AtomicUsize>,
    }

    /// Router with the given provider instances (name, kind) and one client
    /// whose strategy allows everything. `prioritized` configures auto-routing.
    async fn harness(
        instances: &[(&str, Kind)],
        prioritized: &[(&str, &str)],
        mutate: impl FnOnce(&mut AppConfig),
    ) -> Harness {
        let chat_calls = Arc::new(AtomicUsize::new(0));
        let native_calls = Arc::new(AtomicUsize::new(0));
        let registry = Arc::new(ProviderRegistry::new());
        for (i, (name, kind)) in instances.iter().enumerate() {
            let type_name: &'static str = Box::leak(format!("mock{i}").into_boxed_str());
            registry.register_factory(Arc::new(MockFactory {
                kind: *kind,
                type_name,
                chat_calls: chat_calls.clone(),
                native_calls: native_calls.clone(),
            }));
            registry
                .create_provider(name.to_string(), type_name.to_string(), HashMap::new())
                .await
                .unwrap();
        }

        let mut strategy = lr_config::Strategy::new("s".into());
        if let Some(auto) = strategy.auto_config.as_mut() {
            auto.prioritized_models = prioritized
                .iter()
                .map(|(p, m)| (p.to_string(), m.to_string()))
                .collect();
        }
        let client = lr_config::Client::new_with_strategy("c".into(), strategy.id.clone());
        let client_id = client.id.clone();
        let mut config = AppConfig::default();
        config.strategies.push(strategy);
        config.clients.push(client);
        mutate(&mut config);

        let config_manager = Arc::new(ConfigManager::new(
            config,
            std::path::PathBuf::from("/tmp/systemone-router-test.yaml"),
        ));
        let metrics_db_path =
            std::env::temp_dir().join(format!("test_metrics_{}.db", uuid::Uuid::new_v4()));
        let metrics_db =
            Arc::new(lr_monitoring::storage::MetricsDatabase::new(metrics_db_path).unwrap());
        let router = Router::new_without_free_tier(
            config_manager,
            registry,
            Arc::new(crate::RateLimiterManager::new(None)),
            Arc::new(lr_monitoring::metrics::MetricsCollector::new(metrics_db)),
        );
        Harness {
            router,
            client_id,
            chat_calls,
            native_calls,
        }
    }

    fn request(model: Option<&str>) -> SystemOneRequest {
        let mut v = json!({
            "state": {"body": "billed twice"},
            "questions": {
                "dept": {"type": "choice", "instructions": "which team?",
                    "criteria": {"billing": "refunds", "tech": "bugs"}},
                "urgent": {"type": "noul", "instructions": "urgent?"}
            }
        });
        if let Some(m) = model {
            v["model"] = json!(m);
        }
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn native_provider_with_prefix() {
        let h = harness(&[("laya", Kind::Native)], &[], |_| {}).await;
        let resp = h
            .router
            .systemone(&h.client_id, request(Some("laya/english")))
            .await
            .unwrap();
        assert_eq!(resp.provider, "laya");
        assert_eq!(
            resp.model, "english",
            "prefix is stripped before the provider sees it"
        );
        assert_eq!(resp.backend, SystemOneBackend::Native);
        assert_eq!(resp.request_id.as_deref(), Some("req-1"));
        // 1000 input tokens at $0.001/1k.
        assert!((resp.cost_usd.unwrap() - 0.001).abs() < 1e-12);
        assert_eq!(h.chat_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn missing_model_uses_single_native_provider() {
        let h = harness(
            &[("laya", Kind::Native), ("chat", Kind::ChatJson)],
            &[],
            |_| {},
        )
        .await;
        let resp = h
            .router
            .systemone(&h.client_id, request(None))
            .await
            .unwrap();
        assert_eq!(resp.provider, "laya");
        // The provider applies its own default when no model is sent.
        assert_eq!(resp.model, "native-default");
    }

    #[tokio::test]
    async fn missing_model_with_two_native_providers_falls_back_to_auto() {
        let h = harness(
            &[("laya", Kind::Native), ("kev", Kind::Native)],
            &[("kev", "m1")],
            |_| {},
        )
        .await;
        let resp = h
            .router
            .systemone(&h.client_id, request(None))
            .await
            .unwrap();
        assert_eq!(resp.provider, "kev");
    }

    #[tokio::test]
    async fn chat_provider_letter_mode() {
        let h = harness(&[("oa", Kind::ChatLogprobs)], &[], |_| {}).await;
        let resp = h
            .router
            .systemone(&h.client_id, request(Some("oa/gpt")))
            .await
            .unwrap();
        assert_eq!(resp.backend, SystemOneBackend::LetterLogprobs);
        assert_eq!(resp.model, "oa/gpt");
        // One chat call per question.
        assert_eq!(h.chat_calls.load(Ordering::SeqCst), 2);
        match &resp.answers["dept"] {
            SystemOneAnswer::Choice {
                choice,
                probabilities,
                ..
            } => {
                assert_eq!(choice, "tech");
                assert!((probabilities["tech"] - 0.75).abs() < 1e-9);
            }
            other => panic!("{other:?}"),
        }
        match &resp.answers["urgent"] {
            // B = no, so p(yes) = 0.25
            SystemOneAnswer::Noul { noul, .. } => assert!((noul - 0.25).abs() < 1e-9),
            other => panic!("{other:?}"),
        }
        assert_eq!(resp.usage.input_tokens, Some(200));
        assert_eq!(resp.usage.output_tokens, Some(20));
        // 2 × (100 in × 0.001/1k + 10 out × 0.002/1k)
        assert!((resp.cost_usd.unwrap() - 0.00024).abs() < 1e-12);
    }

    #[tokio::test]
    async fn chat_provider_json_mode() {
        let h = harness(&[("an", Kind::ChatJson)], &[], |_| {}).await;
        let resp = h
            .router
            .systemone(&h.client_id, request(Some("an/claude")))
            .await
            .unwrap();
        assert_eq!(resp.backend, SystemOneBackend::Json);
        assert_eq!(h.chat_calls.load(Ordering::SeqCst), 1);
        match &resp.answers["dept"] {
            SystemOneAnswer::Choice { choice, .. } => assert_eq!(choice, "tech"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn unparseable_chat_output_is_a_provider_error() {
        let h = harness(&[("g", Kind::ChatGarbage)], &[], |_| {}).await;
        let err = h
            .router
            .systemone(&h.client_id, request(Some("g/m1")))
            .await
            .unwrap_err();
        assert!(Router::is_emulation_parse_error(&err), "{err}");
    }

    #[tokio::test]
    async fn emulation_off_rejects_chat_providers() {
        let h = harness(&[("an", Kind::ChatJson)], &[], |c| {
            c.systemone.emulation = SystemOneEmulation::Off;
        })
        .await;
        let err = h
            .router
            .systemone(&h.client_id, request(Some("an/claude")))
            .await
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("does not support system one decisions"));
    }

    #[tokio::test]
    async fn auto_routing_falls_through_unparseable_models() {
        let h = harness(
            &[("g", Kind::ChatGarbage), ("an", Kind::ChatJson)],
            &[("g", "m1"), ("an", "m1")],
            |_| {},
        )
        .await;
        let resp = h
            .router
            .systemone(&h.client_id, request(Some("localrouter/auto")))
            .await
            .unwrap();
        assert_eq!(resp.provider, "an");
    }

    #[tokio::test]
    async fn internal_test_client_requires_prefixed_model() {
        let h = harness(&[("laya", Kind::Native)], &[], |_| {}).await;
        assert!(h
            .router
            .systemone("internal-test", request(Some("english")))
            .await
            .is_err());
        let resp = h
            .router
            .systemone("internal-test", request(Some("laya/english")))
            .await
            .unwrap();
        assert_eq!(resp.provider, "laya");
    }

    #[tokio::test]
    async fn unknown_client_is_unauthorized() {
        let h = harness(&[("laya", Kind::Native)], &[], |_| {}).await;
        let err = h
            .router
            .systemone("nobody", request(Some("laya/english")))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Unauthorized));
    }

    #[tokio::test]
    async fn mixed_provider_routes_per_model() {
        // Same chat provider: its decision model answers natively, its chat
        // models go through translation.
        let h = harness(&[("gw", Kind::ChatLogprobs)], &[], |_| {}).await;
        let native = h
            .router
            .systemone(&h.client_id, request(Some("gw/jev-native")))
            .await
            .unwrap();
        assert_eq!(native.backend, SystemOneBackend::Native);
        let translated = h
            .router
            .systemone(&h.client_id, request(Some("gw/some-chat-model")))
            .await
            .unwrap();
        assert_eq!(translated.backend, SystemOneBackend::LetterLogprobs);
    }

    #[test]
    fn classify_systemone_phrases() {
        for msg in [
            "Provider 'x' does not support system one decisions",
            "Provider 'laya' does not support chat completions",
        ] {
            assert!(matches!(
                RouterError::classify(&AppError::Provider(msg.into()), "p", "m"),
                RouterError::EndpointNotSupported { .. }
            ));
        }
    }
    fn routing_policy(provider: &str) -> lr_config::RoutingPolicy {
        let mut policy: lr_config::RoutingPolicy = serde_json::from_value(json!({
            "enabled":true,"weak_models":[["chat","m1"]]
        }))
        .unwrap();
        policy.decision_model = Some((provider.into(), "m1".into()));
        policy.default_route = "routine".into();
        policy.mode_rules = vec![lr_config::ModeRouteRule {
            mode: "plan".into(),
            route: "thorough".into(),
        }];
        policy
    }

    #[tokio::test]
    async fn cached_preview_leaves_semantic_routes_unknown_and_does_not_execute_models() {
        let h = harness(
            &[("decision", Kind::SlowNative), ("chat", Kind::ChatJson)],
            &[("chat", "m1")],
            |_| {},
        )
        .await;
        let (_, strategy) = h.router.validate_client_and_strategy(&h.client_id).unwrap();
        let policy = routing_policy("decision");
        let request = CompletionRequest::new("localrouter/auto", vec![msg("Implement")]);
        let decision = h
            .router
            .deterministic_routing_decision(&strategy, &policy, &request);
        assert!(decision.is_none());
        let explanation =
            h.router
                .routing_explanation(&strategy, &policy, &request, decision.as_ref());
        assert!(explanation.classifier_required);
        assert!(explanation.active_route.is_none());
        assert!(!explanation.no_route);
        assert_eq!(h.chat_calls.load(Ordering::SeqCst), 0);
        assert_eq!(h.router.decision_slots.available_permits(), 8);
        let again = h
            .router
            .routing_explanation(&strategy, &policy, &request, None);
        assert_eq!(explanation.config_fingerprint, again.config_fingerprint);
        assert_eq!(explanation.policy_fingerprint.len(), 64);
    }

    #[tokio::test]
    async fn preview_and_live_filters_agree_on_denied_candidates_and_no_route_details() {
        let h = harness(&[("chat", Kind::ChatJson)], &[("chat", "m1")], |config| {
            config
                .strategies
                .last_mut()
                .unwrap()
                .model_permissions
                .global = lr_config::PermissionState::Off;
        })
        .await;
        let (_, strategy) = h.router.validate_client_and_strategy(&h.client_id).unwrap();
        let request = CompletionRequest::new("localrouter/auto", vec![msg("hi")]);
        let assessment = h
            .router
            .assess_chat_candidate(&strategy, "chat", "m1", &request, 0);
        assert!(!assessment.eligible);
        assert!(assessment.exclusions.contains(&"model_not_allowed".into()));
        let explanation = h.router.routing_explanation(
            &strategy,
            &lr_config::RoutingPolicy::default(),
            &request,
            None,
        );
        assert!(explanation.no_route);
        let error = h.router.complete(&h.client_id, request).await.unwrap_err();
        let AppError::RoutingExhausted { explanation, .. } = error else {
            panic!("expected structured no-route error")
        };
        assert_eq!(explanation["attempts"][0]["outcome"], "model_not_allowed");
        assert_eq!(h.chat_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn native_decision_auto_uses_exact_policy_modes_and_retains_explanation() {
        let h = harness(
            &[("laya", Kind::Native), ("chat", Kind::ChatJson)],
            &[],
            |config| {
                let mut policy = routing_policy("missing");
                for option in &mut policy.options {
                    option.models = vec![("laya".into(), "m1".into())];
                }
                config
                    .strategies
                    .last_mut()
                    .unwrap()
                    .auto_config
                    .as_mut()
                    .unwrap()
                    .routing_policy = Some(policy);
            },
        )
        .await;
        let mut input = request(Some("localrouter/auto"));
        input
            .extra
            .insert("metadata".into(), json!({"localrouter.mode":"plan"}));
        let response = h.router.systemone(&h.client_id, input).await.unwrap();
        assert_eq!(
            response.extra["extensions"]["localrouter.routing"]["decision_routing"]["source"],
            "client_mode"
        );
        assert_eq!(
            response.extra["extensions"]["localrouter.routing"]["decision_routing"]["route"],
            "thorough"
        );
        assert_eq!(h.chat_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn policy_calls_selected_native_provider_and_routes_completion() {
        let h = harness(
            &[("decision", Kind::Native), ("chat", Kind::ChatJson)],
            &[("chat", "m1")],
            |cfg| {
                cfg.strategies
                    .last_mut()
                    .unwrap()
                    .auto_config
                    .as_mut()
                    .unwrap()
                    .routing_policy = Some(routing_policy("decision"));
            },
        )
        .await;
        let (_, strategy) = h.router.validate_client_and_strategy(&h.client_id).unwrap();
        let req = CompletionRequest::new("localrouter/auto", vec![msg("Implement the feature")]);
        let (response, meta) = h
            .router
            .complete_with_auto_routing(&h.client_id, &strategy, req)
            .await
            .unwrap();
        assert_eq!(response.provider, "chat");
        let meta = meta.unwrap();
        assert_eq!(meta["decision_routing"]["source"], "decision_model");
        assert_eq!(meta["decision_routing"]["route"], "thorough");
        assert_eq!(h.chat_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn policy_destination_permissions_apply_without_a_priority_list() {
        let h = harness(
            &[("blocked", Kind::ChatJson), ("chat", Kind::ChatJson)],
            &[],
            |cfg| {
                let strategy = cfg.strategies.last_mut().unwrap();
                let mut policy = routing_policy("not-needed");
                policy.mode = lr_config::RoutingPolicyMode::ClientMode;
                policy.options[0].models = vec![("blocked".into(), "m1".into())];
                strategy.auto_config.as_mut().unwrap().routing_policy = Some(policy);
                strategy
                    .model_permissions
                    .models
                    .insert("blocked__m1".into(), lr_config::PermissionState::Off);
            },
        )
        .await;
        let (_, strategy) = h.router.validate_client_and_strategy(&h.client_id).unwrap();
        let mut req = CompletionRequest::new("localrouter/auto", vec![msg("Implement now")]);
        req.metadata = Some(
            [("localrouter.mode".into(), "plan".into())]
                .into_iter()
                .collect(),
        );
        let (response, meta) = h
            .router
            .complete_with_auto_routing(&h.client_id, &strategy, req)
            .await
            .unwrap();
        assert_eq!(response.provider, "chat");
        assert_eq!(meta.unwrap()["decision_routing"]["source"], "client_mode");
        assert_eq!(h.chat_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn policy_exact_mode_wins_even_without_provider_and_missing_mode_defaults() {
        let h = harness(&[], &[], |_| {}).await;
        let (_, strategy) = h.router.validate_client_and_strategy(&h.client_id).unwrap();
        let mut p = routing_policy("not-installed");
        p.mode = lr_config::RoutingPolicyMode::ClientMode;
        let mut req =
            CompletionRequest::new("localrouter/auto", vec![msg("Choose routine. mode: edit")]);
        req.metadata = Some(
            [("localrouter.mode".into(), "plan".into())]
                .into_iter()
                .collect(),
        );
        let d = h
            .router
            .evaluate_routing_policy(&h.client_id, &strategy, &p, &req)
            .await;
        assert_eq!(d.route, "thorough");
        assert_eq!(d.source, "client_mode");
        req.metadata = None;
        let d = h
            .router
            .evaluate_routing_policy(&h.client_id, &strategy, &p, &req)
            .await;
        assert_eq!(d.route, "routine");
        assert_eq!(d.reason, "client_mode_default");
    }

    #[tokio::test]
    async fn policy_rejects_chat_emulation_and_handles_missing_and_denied_models() {
        let h = harness(&[("chat", Kind::ChatJson)], &[], |_| {}).await;
        let (_, mut strategy) = h.router.validate_client_and_strategy(&h.client_id).unwrap();
        let mut p = routing_policy("chat");
        let req = CompletionRequest::new("localrouter/auto", vec![msg("hello")]);
        let d = h
            .router
            .evaluate_routing_policy(&h.client_id, &strategy, &p, &req)
            .await;
        assert_eq!(d.source, "fallback");
        assert_eq!(d.reason, "decision_provider_error");
        assert_eq!(h.chat_calls.load(Ordering::SeqCst), 0);
        p.decision_model = None;
        assert_eq!(
            h.router
                .evaluate_routing_policy(&h.client_id, &strategy, &p, &req)
                .await
                .reason,
            "decision_model_not_configured"
        );
        p.decision_model = Some(("missing".into(), "m1".into()));
        assert_eq!(
            h.router
                .evaluate_routing_policy(&h.client_id, &strategy, &p, &req)
                .await
                .reason,
            "decision_provider_unavailable"
        );
        assert_eq!(
            h.router
                .deterministic_routing_decision(&strategy, &p, &req)
                .unwrap()
                .reason,
            "decision_provider_unavailable"
        );
        strategy.model_permissions.global = lr_config::PermissionState::Off;
        strategy.model_permissions.providers.clear();
        strategy.model_permissions.models.clear();
        assert_eq!(
            h.router
                .evaluate_routing_policy(&h.client_id, &strategy, &p, &req)
                .await
                .reason,
            "decision_model_not_allowed"
        );
    }

    #[tokio::test]
    async fn policy_timeout_and_busy_classifier_use_default() {
        let h = harness(&[("decision", Kind::SlowNative)], &[], |_| {}).await;
        let (_, strategy) = h.router.validate_client_and_strategy(&h.client_id).unwrap();
        let mut p = routing_policy("decision");
        p.timeout_ms = 100;
        let req = CompletionRequest::new("localrouter/auto", vec![]);
        let d = h
            .router
            .evaluate_routing_policy(&h.client_id, &strategy, &p, &req)
            .await;
        assert_eq!(d.reason, "decision_timeout");
        assert_eq!(d.route, "routine");
        let _slots = h.router.decision_slots.acquire_many(8).await.unwrap();
        assert_eq!(
            h.router
                .evaluate_routing_policy(&h.client_id, &strategy, &p, &req)
                .await
                .reason,
            "classifier_busy"
        );
    }

    #[tokio::test]
    async fn policy_empty_route_inherits_priority_before_default_and_deduplicates() {
        let h = harness(&[], &[("primary", "a")], |_| {}).await;
        let (_, mut strategy) = h.router.validate_client_and_strategy(&h.client_id).unwrap();
        let mut p = routing_policy("missing");
        p.mode = lr_config::RoutingPolicyMode::ClientMode;
        p.options[1].models = vec![
            ("backup".into(), "b".into()),
            ("primary".into(), "a".into()),
        ];
        strategy.auto_config.as_mut().unwrap().routing_policy = Some(p);
        let mut req = CompletionRequest::new("localrouter/auto", vec![]);
        req.metadata = Some(
            [("localrouter.mode".into(), "plan".into())]
                .into_iter()
                .collect(),
        );
        let (models, _) = h
            .router
            .select_models_for_auto_routing(
                &h.client_id,
                &strategy,
                strategy.auto_config.as_ref().unwrap(),
                &req,
            )
            .await;
        assert_eq!(
            models,
            vec![
                ("primary".into(), "a".into()),
                ("backup".into(), "b".into())
            ]
        );
    }

    /// `localrouter/auto` is listed and accepted exactly when
    /// `has_chat_candidates()`; requests that fall back (here: no decision
    /// model) must then still find a model, and a config without one must not
    /// pretend otherwise.
    #[tokio::test]
    async fn listed_auto_model_always_selects_a_candidate_on_fallback() {
        let h = harness(&[], &[], |_| {}).await;
        let (_, mut strategy) = h.router.validate_client_and_strategy(&h.client_id).unwrap();
        let req = CompletionRequest::new("localrouter/auto", vec![msg("Implement it")]);
        let select = |strategy: lr_config::Strategy| {
            let router = &h.router;
            let client_id = h.client_id.clone();
            let req = req.clone();
            async move {
                router
                    .select_models_for_auto_routing(
                        &client_id,
                        &strategy,
                        strategy.auto_config.as_ref().unwrap(),
                        &req,
                    )
                    .await
            }
        };

        // Migrated 0.0.153 strong/weak policy with an empty priority list.
        let migrated: lr_config::RoutingPolicy = serde_json::from_value(json!({
            "enabled": true, "weak_models": [["local", "small"]]
        }))
        .unwrap();
        strategy.auto_config.as_mut().unwrap().routing_policy = Some(migrated.clone());
        assert!(!strategy.auto_config.as_ref().unwrap().has_chat_candidates());
        let (models, decision) = select(strategy.clone()).await;
        assert!(models.is_empty());
        assert_eq!(decision.unwrap().reason, "decision_model_not_configured");

        // A policy-only strategy whose default route has its own models.
        let mut policy = migrated;
        policy.default_route = "routine".into();
        strategy.auto_config.as_mut().unwrap().routing_policy = Some(policy);
        assert!(strategy.auto_config.as_ref().unwrap().has_chat_candidates());
        let (models, decision) = select(strategy).await;
        assert_eq!(models, vec![("local".into(), "small".into())]);
        assert_eq!(decision.unwrap().route, "routine");
    }

    // ----- Per-model settings (manual models, visibility, overrides) -----

    fn hide(router: &Router, provider: &str, model: &str) {
        router
            .provider_registry
            .set_model_settings(
                provider,
                lr_config::ProviderModelSettings {
                    show_new_models: None,
                    entries: vec![lr_config::ModelEntry {
                        id: model.into(),
                        enabled: Some(false),
                        ..Default::default()
                    }],
                },
            )
            .unwrap();
    }

    fn chat_request(model: &str) -> CompletionRequest {
        serde_json::from_value(json!({
            "model": model,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn automatic_decisions_never_call_a_model_revoked_by_client_permissions() {
        let h = harness(&[("chat", Kind::ChatJson)], &[("chat", "m1")], |config| {
            let strategy = config.strategies.last_mut().unwrap();
            strategy.model_permissions.global = lr_config::PermissionState::Off;
            strategy.model_permissions.providers.clear();
            strategy.model_permissions.models.clear();
        })
        .await;
        let result = h
            .router
            .systemone(&h.client_id, request(Some("localrouter/auto")))
            .await;
        assert!(result.is_err());
        assert_eq!(h.chat_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn explicit_request_for_a_hidden_model_is_refused() {
        let h = harness(&[("chat", Kind::ChatJson)], &[], |_| {}).await;
        hide(&h.router, "chat", "m1");
        let err = h
            .router
            .complete(&h.client_id, chat_request("chat/m1"))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidParams(_)), "{err:?}");
        assert_eq!(h.chat_calls.load(Ordering::SeqCst), 0);
        // Ids the provider does not list still pass through.
        h.router
            .complete(&h.client_id, chat_request("chat/unlisted"))
            .await
            .unwrap();
        // So does the hidden model once the cache knows it is listed and
        // the user shows it again.
        h.router
            .provider_registry
            .set_model_settings("chat", Default::default())
            .unwrap();
        h.router
            .complete(&h.client_id, chat_request("chat/m1"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn unprefixed_requests_skip_providers_hiding_the_model() {
        let h = harness(
            &[("first", Kind::ChatJson), ("second", Kind::ChatJson)],
            &[],
            |_| {},
        )
        .await;
        hide(&h.router, "first", "m1");
        let strategy = lr_config::Strategy::new("s".into());
        for _ in 0..3 {
            let (provider, model) = h
                .router
                .find_provider_for_model("m1", &strategy)
                .await
                .unwrap();
            assert_eq!((provider.as_str(), model.as_str()), ("second", "m1"));
        }
        // A pair the strategy selected by hand is skipped too.
        let mut picked = lr_config::Strategy::new("p".into());
        picked.allowed_models.selected_all = false;
        picked.allowed_models.selected_models = vec![("first".into(), "m1".into())];
        picked.allowed_models.selected_providers = vec!["second".into()];
        let (provider, _) = h
            .router
            .find_provider_for_model("m1", &picked)
            .await
            .unwrap();
        assert_eq!(provider, "second");
    }

    #[tokio::test]
    async fn auto_routing_skips_hidden_models() {
        let h = harness(
            &[("first", Kind::ChatJson), ("second", Kind::ChatJson)],
            &[("first", "m1"), ("second", "m1")],
            |_| {},
        )
        .await;
        hide(&h.router, "first", "m1");
        let (resp, meta) = h
            .router
            .complete(&h.client_id, chat_request("localrouter/auto"))
            .await
            .unwrap();
        assert_eq!(resp.provider, "second");
        let attempts = meta.unwrap()["attempts"].clone();
        assert_eq!(attempts[0]["outcome"], "model_disabled");
    }

    #[tokio::test]
    async fn price_override_sets_the_cost() {
        let h = harness(&[("laya", Kind::Native)], &[], |_| {}).await;
        h.router
            .provider_registry
            .set_model_settings(
                "laya",
                lr_config::ProviderModelSettings {
                    show_new_models: None,
                    entries: vec![lr_config::ModelEntry {
                        id: "english".into(),
                        pricing: Some(lr_config::ModelPricingOverride {
                            input_per_million: 5.0,
                            output_per_million: 0.0,
                        }),
                        ..Default::default()
                    }],
                },
            )
            .unwrap();
        let resp = h
            .router
            .systemone(&h.client_id, request(Some("laya/english")))
            .await
            .unwrap();
        // 1000 input tokens at $5 per million.
        assert!((resp.cost_usd.unwrap() - 0.005).abs() < 1e-12);
    }

    #[tokio::test]
    async fn priced_model_on_a_local_provider_is_not_free() {
        let h = harness(&[("local", Kind::ChatJson)], &[], |_| {}).await;
        let always_free = lr_config::FreeTierKind::AlwaysFreeLocal;
        assert!(!matches!(
            h.router
                .classify_with_free_tier("local", "m1", &always_free)
                .await,
            free_tier::ModelFreeStatus::NotFree
        ));
        h.router
            .provider_registry
            .set_model_settings(
                "local",
                lr_config::ProviderModelSettings {
                    show_new_models: None,
                    entries: vec![lr_config::ModelEntry {
                        id: "m1".into(),
                        pricing: Some(lr_config::ModelPricingOverride {
                            input_per_million: 0.0,
                            output_per_million: 0.5,
                        }),
                        ..Default::default()
                    }],
                },
            )
            .unwrap();
        assert!(matches!(
            h.router
                .classify_with_free_tier("local", "m1", &always_free)
                .await,
            free_tier::ModelFreeStatus::NotFree
        ));
        // Other models of the provider stay free; a $0 price changes nothing.
        assert!(!matches!(
            h.router
                .classify_with_free_tier("local", "m2", &always_free)
                .await,
            free_tier::ModelFreeStatus::NotFree
        ));
    }

    fn free_models(rules: Vec<lr_config::FreeModelRule>) -> lr_config::FreeTierKind {
        lr_config::FreeTierKind::FreeModelsOnly {
            free_model_patterns: vec![],
            excluded_models: vec![],
            rules,
            max_rpm: 0,
        }
    }

    fn include(when: lr_config::FreeModelCondition) -> lr_config::FreeModelRule {
        lr_config::FreeModelRule {
            enabled: true,
            effect: lr_config::FreeModelRuleEffect::Include,
            when,
            label: None,
        }
    }

    #[tokio::test]
    async fn listing_rules_fetch_the_model_list_once() {
        let h = harness(&[("p", Kind::ChatJson)], &[], |_| {}).await;
        let kind = free_models(vec![include(lr_config::FreeModelCondition::Field {
            pointer: "/tier".into(),
            op: lr_config::FieldOp::Equals,
            value: json!("free"),
        })]);
        assert!(!h.router.provider_registry.has_cached_models("p"));
        assert_eq!(
            h.router.classify_with_free_tier("p", "m1", &kind).await,
            free_tier::ModelFreeStatus::FreeModel
        );
        assert!(h.router.provider_registry.has_cached_models("p"));
        // Not listed: no data, so not free.
        assert_eq!(
            h.router.classify_with_free_tier("p", "m2", &kind).await,
            free_tier::ModelFreeStatus::NotFree
        );
    }

    #[tokio::test]
    async fn zero_price_in_the_listing_makes_a_model_free() {
        let h = harness(&[("p", Kind::ChatJson)], &[], |_| {}).await;
        let kind = free_models(vec![include(lr_config::FreeModelCondition::ZeroPrice {
            sources: vec![lr_config::PriceSource::ProviderListing],
            input_pointer: None,
            output_pointer: None,
        })]);
        let decision = h.router.free_model_decision("p", "m1", &kind).await;
        assert!(decision.is_free);
        assert_eq!(decision.rule_index, Some(0));
        assert!(decision.reason.contains("$0"), "{}", decision.reason);
        // A price set on the Models tab is not part of a listing-only rule.
        assert!(
            !h.router
                .free_model_decision("p", "other", &kind)
                .await
                .is_free
        );
    }

    #[tokio::test]
    async fn free_only_strategy_refuses_models_the_rules_do_not_free() {
        let kind = free_models(vec![include(lr_config::FreeModelCondition::ModelId {
            pattern: "m1".into(),
            syntax: lr_config::ModelIdPatternSyntax::Exact,
            case_insensitive: false,
        })]);
        let h = harness(&[("p", Kind::ChatJson)], &[], |config| {
            config.strategies[0].free_tier_only = true;
            config.providers.push(lr_config::ProviderConfig {
                name: "p".into(),
                free_tier: Some(kind.clone()),
                ..lr_config::ProviderConfig::default_ollama()
            });
        })
        .await;
        assert!(h
            .router
            .systemone(&h.client_id, request(Some("p/m1")))
            .await
            .is_ok());
        let refused = h
            .router
            .systemone(&h.client_id, request(Some("p/m2")))
            .await
            .unwrap_err();
        assert!(
            matches!(refused, AppError::FreeTierExhausted { .. }),
            "{refused:?}"
        );
        assert!(h.router.free_tier_usage("p", "m1").model_is_free);
        assert!(!h.router.free_tier_usage("p", "m2").model_is_free);
    }
    #[tokio::test]
    async fn adaptive_cost_sorting_keeps_selected_default_and_baseline_groups_separate() {
        let h = harness(
            &[
                ("selected_unknown", Kind::ChatJson),
                ("selected_known", Kind::ChatJson),
                ("default", Kind::ChatJson),
                ("baseline", Kind::ChatJson),
            ],
            &[("baseline", "m1")],
            |config| {
                let mut policy = routing_policy("missing");
                policy.mode = lr_config::RoutingPolicyMode::ClientMode;
                policy
                    .options
                    .iter_mut()
                    .find(|option| option.id == "thorough")
                    .unwrap()
                    .models = vec![
                    ("selected_unknown".into(), "m1".into()),
                    ("selected_known".into(), "m1".into()),
                ];
                policy
                    .options
                    .iter_mut()
                    .find(|option| option.id == "routine")
                    .unwrap()
                    .models = vec![("default".into(), "m1".into())];
                let auto = config
                    .strategies
                    .last_mut()
                    .unwrap()
                    .auto_config
                    .as_mut()
                    .unwrap();
                auto.routing_policy = Some(policy);
                auto.adaptive_routing.ordering = lr_config::CandidateOrdering::EstimatedCost;
                for (provider, price) in [("mock1", 20.0), ("mock2", 1.0), ("mock3", 0.0)] {
                    config.pricing_overrides.insert(
                        provider.into(),
                        [(
                            "m1".into(),
                            lr_config::ModelPricingOverride {
                                input_per_million: price,
                                output_per_million: price,
                            },
                        )]
                        .into_iter()
                        .collect(),
                    );
                }
            },
        )
        .await;
        let mut request = CompletionRequest::new("localrouter/auto", vec![msg("explain")]);
        request.metadata = Some(
            [("localrouter.mode".into(), "plan".into())]
                .into_iter()
                .collect(),
        );
        let (response, metadata) = h.router.complete(&h.client_id, request).await.unwrap();
        assert_eq!(response.provider, "selected_known");
        let metadata = metadata.unwrap();
        let candidates = metadata["explanation"]["routes"][0]["candidates"]
            .as_array()
            .unwrap();
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate["provider"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["selected_known", "selected_unknown", "default", "baseline"]
        );
        assert!(candidates[1]["score"].is_null());
        assert_eq!(candidates[0]["priority"], 1);
        assert_eq!(metadata["inference_dispatches"], 1);
    }

    #[tokio::test]
    async fn least_load_ordering_counts_other_endpoint_work_for_the_same_model() {
        let h = harness(
            &[("busy", Kind::ChatJson), ("available", Kind::ChatJson)],
            &[("busy", "m1"), ("available", "m1")],
            |config| {
                config
                    .strategies
                    .last_mut()
                    .unwrap()
                    .auto_config
                    .as_mut()
                    .unwrap()
                    .adaptive_routing
                    .ordering = lr_config::CandidateOrdering::LeastInFlight;
            },
        )
        .await;
        let busy = h.router.adaptive_measurements.begin(
            "busy",
            "m1",
            EndpointType::SystemOne,
            h.router.provider_registry.routing_revision(),
        );
        let (response, _) = h
            .router
            .complete(
                &h.client_id,
                CompletionRequest::new("localrouter/auto", vec![msg("hello")]),
            )
            .await
            .unwrap();
        assert_eq!(response.provider, "available");
        drop(busy);
    }

    #[tokio::test]
    async fn native_timeout_consumes_one_dispatch_and_does_not_escape_attempt_limit() {
        let h = harness(
            &[("slow", Kind::SlowNative), ("fast", Kind::Native)],
            &[("slow", "m1"), ("fast", "m1")],
            |config| {
                let settings = &mut config
                    .strategies
                    .last_mut()
                    .unwrap()
                    .auto_config
                    .as_mut()
                    .unwrap()
                    .adaptive_routing;
                settings.max_attempts = Some(1);
                settings.attempt_timeout_ms = Some(100);
            },
        )
        .await;
        let error = h
            .router
            .systemone(&h.client_id, request(Some("localrouter/auto")))
            .await
            .unwrap_err();
        let AppError::RoutingExhausted { explanation, .. } = error else {
            panic!("{error:?}")
        };
        assert_eq!(h.native_calls.load(Ordering::SeqCst), 1);
        assert_eq!(explanation["inference_dispatches"], 1);
        assert_eq!(explanation["attempt_costs"]["unknown_calls"], 1);
        assert!(explanation["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|attempt| attempt["outcome"] == "max_attempts_exceeded"));
    }

    #[tokio::test]
    async fn total_deadline_stops_native_fallback_and_retains_explanation() {
        let h = harness(
            &[("slow", Kind::SlowNative), ("fast", Kind::Native)],
            &[("slow", "m1"), ("fast", "m1")],
            |config| {
                config
                    .strategies
                    .last_mut()
                    .unwrap()
                    .auto_config
                    .as_mut()
                    .unwrap()
                    .adaptive_routing
                    .total_timeout_ms = Some(100);
            },
        )
        .await;
        let error = h
            .router
            .systemone(&h.client_id, request(Some("localrouter/auto")))
            .await
            .unwrap_err();
        let AppError::RoutingFailure { error, explanation } = error else {
            panic!("{error:?}")
        };
        assert!(matches!(
            *error,
            AppError::RoutingDeadlineExceeded { scope: "total" }
        ));
        assert!(h.native_calls.load(Ordering::SeqCst) <= 1);
        assert_eq!(
            explanation["inference_dispatches"],
            h.native_calls.load(Ordering::SeqCst)
        );
        assert!(explanation["explanation"]["config_fingerprint"].is_string());
    }

    #[tokio::test]
    async fn translated_questions_cannot_multiply_the_actual_dispatch_limit() {
        let h = harness(
            &[("chat", Kind::ChatLogprobs)],
            &[("chat", "m1")],
            |config| {
                config
                    .strategies
                    .last_mut()
                    .unwrap()
                    .auto_config
                    .as_mut()
                    .unwrap()
                    .adaptive_routing
                    .max_attempts = Some(1);
            },
        )
        .await;
        let error = h
            .router
            .systemone(&h.client_id, request(Some("localrouter/auto")))
            .await
            .unwrap_err();
        let AppError::RoutingFailure { explanation, .. } = error else {
            panic!("{error:?}")
        };
        assert_eq!(h.chat_calls.load(Ordering::SeqCst), 1);
        assert_eq!(explanation["inference_dispatches"], 1);
        assert_eq!(explanation["attempt_costs"]["unknown_calls"], 0);
        assert!(
            explanation["attempt_costs"]["estimated_usd"]
                .as_f64()
                .unwrap()
                > 0.0
        );
    }
    #[tokio::test]
    async fn classifier_time_already_spent_cannot_reset_the_total_routing_deadline() {
        let h = harness(&[("chat", Kind::ChatJson)], &[("chat", "m1")], |config| {
            config
                .strategies
                .last_mut()
                .unwrap()
                .auto_config
                .as_mut()
                .unwrap()
                .adaptive_routing
                .total_timeout_ms = Some(100);
        })
        .await;
        let mut request = CompletionRequest::new("localrouter/auto", vec![msg("hello")]);
        request.pre_computed_routing=Some(serde_json::from_value(json!({"route":"routine","source":"decision_model","reason":"policy_match","probabilities":{},"latency_ms":1000,"policy_version":1,"context_omitted":0,"classifier_cost_usd":0.02,"classifier_cost_estimated":false})).unwrap());
        let error = h.router.complete(&h.client_id, request).await.unwrap_err();
        let AppError::RoutingFailure { error, explanation } = error else {
            panic!("{error:?}")
        };
        assert!(matches!(
            *error,
            AppError::RoutingDeadlineExceeded { scope: "total" }
        ));
        assert_eq!(h.chat_calls.load(Ordering::SeqCst), 0);
        assert_eq!(explanation["classifier_cost_usd"], 0.02);
    }
    #[tokio::test]
    async fn completed_response_accounting_survives_a_cancelled_await() {
        let h = harness(&[("chat", Kind::ChatJson)], &[], |_| {}).await;
        h.router.rate_limiter.add_api_key_limiters(
            h.client_id.clone(),
            vec![crate::rate_limit::RateLimiter::new(
                crate::rate_limit::RateLimitType::Cost,
                100.0,
                3600,
            )],
        );
        let work = h.router.completed_usage(
            &h.client_id,
            "chat",
            "m1",
            UsageInfo {
                input_tokens: 5,
                output_tokens: 3,
                cost_usd: 0.02,
            },
            false,
        );
        drop(work);
        let recorded = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if let Some(usage) = h
                    .router
                    .rate_limiter
                    .get_api_key_usage(&h.client_id, crate::rate_limit::RateLimitType::Cost)
                    .await
                {
                    break usage.0;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!((recorded - 0.02).abs() < 1e-12);
    }
    #[tokio::test]
    async fn owned_accounting_does_not_charge_a_duplicate_hop() {
        let h = harness(&[("chat", Kind::ChatJson)], &[], |_| {}).await;
        h.router.rate_limiter.add_api_key_limiters(
            h.client_id.clone(),
            vec![crate::rate_limit::RateLimiter::new(
                crate::rate_limit::RateLimitType::Cost,
                100.0,
                3600,
            )],
        );
        lr_types::with_outbound_trace(Some(lr_types::RequestTrace::new().next_hop()), async {
            h.router
                .completed_usage(
                    &h.client_id,
                    "chat",
                    "m1",
                    UsageInfo {
                        input_tokens: 5,
                        output_tokens: 3,
                        cost_usd: 0.02,
                    },
                    false,
                )
                .await;
        })
        .await;
        assert!(h
            .router
            .rate_limiter
            .get_api_key_usage(&h.client_id, crate::rate_limit::RateLimitType::Cost)
            .await
            .is_none());
    }
}
