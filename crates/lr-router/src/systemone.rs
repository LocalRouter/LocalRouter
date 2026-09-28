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

    async fn route_systemone(
        &self,
        client_id: &str,
        strategy: &lr_config::Strategy,
        request: SystemOneRequest,
    ) -> AppResult<SystemOneResponse> {
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
            let free_tier = self.get_effective_free_tier(&final_provider);
            if matches!(
                self.free_tier_manager
                    .classify_model(&final_provider, &final_model, &free_tier),
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
        let provider_instance = self
            .provider_registry
            .get_provider(provider)
            .ok_or_else(|| {
                AppError::Router(format!(
                    "Provider '{}' not found or disabled in registry",
                    provider
                ))
            })?;

        let native = match model {
            Some(m) => provider_instance.supports_systemone_model(m).await,
            None => provider_instance.supports_systemone(),
        };
        if native {
            request.model = model.map(str::to_string);
            let mut response = match provider_instance.systemone(request).await {
                Ok(resp) => resp,
                Err(e) => {
                    let label = model.unwrap_or("default");
                    if matches!(
                        RouterError::classify(&e, provider, label),
                        RouterError::Unreachable { .. }
                    ) {
                        self.report_provider_failure(provider, &e.to_string());
                    }
                    return Err(e);
                }
            };
            response.provider = provider.to_string();
            response.backend = SystemOneBackend::Native;

            let pricing_model = model.unwrap_or(response.model.as_str()).to_string();
            let pricing = provider_instance
                .get_pricing(&pricing_model)
                .await
                .unwrap_or_else(|_| PricingInfo::free());
            let input = response.usage.input_tokens.unwrap_or(0);
            let output = response.usage.output_tokens.unwrap_or(0);
            let cost = calculate_cost(input, output, None, &pricing);
            response.cost_usd = Some(cost);
            self.record_systemone_usage(client_id, provider, input, output, cost, free_tier_only)
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

    async fn record_systemone_usage(
        &self,
        client_id: &str,
        provider: &str,
        input_tokens: u64,
        output_tokens: u64,
        cost: f64,
        free_tier_only: bool,
    ) {
        let usage = UsageInfo {
            input_tokens,
            output_tokens,
            cost_usd: cost,
        };
        if let Err(e) = self
            .rate_limiter
            .record_api_key_usage(client_id, &usage)
            .await
        {
            warn!("Failed to record usage for client '{}': {}", client_id, e);
        }
        let free_tier = self.get_effective_free_tier(provider);
        self.free_tier_manager.record_usage(
            provider,
            &free_tier,
            input_tokens + output_tokens,
            cost,
        );
        if free_tier_only {
            if cost > 0.0 {
                self.free_tier_manager.record_cost_trigger(provider);
            } else {
                self.free_tier_manager.record_cost_free(provider);
            }
        }
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
        let pricing = provider_instance
            .get_pricing(model)
            .await
            .unwrap_or_else(|_| PricingInfo::free());

        let mut usage = SystemOneUsage {
            input_tokens: Some(0),
            output_tokens: Some(0),
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
                let chat = emulation::build_letter_request(model, &request.state, q, top);
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

        let chat = emulation::build_json_request(model, &request);
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
        if auto_config.prioritized_models.is_empty() {
            return Err(AppError::Router(
                "No prioritized models configured for auto-routing".into(),
            ));
        }
        let selected_models = &auto_config.prioritized_models;
        info!(
            "Auto-routing System One for client '{}' with {} prioritized models",
            client_id,
            selected_models.len()
        );

        let mut last_error = None;
        for (provider, model) in selected_models {
            if self.should_skip_for_endpoint(provider, model, EndpointType::SystemOne) {
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
                let free_tier = self.get_effective_free_tier(provider);
                if matches!(
                    self.free_tier_manager
                        .classify_model(provider, model, &free_tier),
                    free_tier::ModelFreeStatus::NotFree
                ) {
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

            match self
                .execute_systemone_request(
                    client_id,
                    provider,
                    Some(model),
                    request.clone(),
                    strategy.free_tier_only,
                )
                .await
            {
                Ok(response) => {
                    info!(
                        "Auto-routing System One succeeded with {}/{}",
                        provider, model
                    );
                    return Ok(response);
                }
                Err(e) => {
                    let router_error = RouterError::classify(&e, provider, model);
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
                        return Err(e);
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

        Err(AppError::Router(format!(
            "All auto-routing System One models failed. Last error: {}",
            last_error
                .map(|e| e.to_log_string())
                .unwrap_or_else(|| "no prioritized model can answer System One requests".into())
        )))
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
            self.kind == Kind::Native
        }
        async fn supports_systemone_model(&self, model: &str) -> bool {
            // A chat provider that also serves one native decision model.
            self.kind == Kind::Native || model == "jev-native"
        }
        async fn systemone(&self, request: SystemOneRequest) -> AppResult<SystemOneResponse> {
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
    }

    /// Router with the given provider instances (name, kind) and one client
    /// whose strategy allows everything. `prioritized` configures auto-routing.
    async fn harness(
        instances: &[(&str, Kind)],
        prioritized: &[(&str, &str)],
        mutate: impl FnOnce(&mut AppConfig),
    ) -> Harness {
        let chat_calls = Arc::new(AtomicUsize::new(0));
        let registry = Arc::new(ProviderRegistry::new());
        for (i, (name, kind)) in instances.iter().enumerate() {
            let type_name: &'static str = Box::leak(format!("mock{i}").into_boxed_str());
            registry.register_factory(Arc::new(MockFactory {
                kind: *kind,
                type_name,
                chat_calls: chat_calls.clone(),
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
}
