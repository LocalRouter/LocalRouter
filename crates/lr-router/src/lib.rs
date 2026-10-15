//! Smart routing system
//!
//! Routes incoming requests to appropriate model providers based on API key configuration.

use std::sync::Arc;

use futures::{Stream, StreamExt};
use std::pin::Pin;
use tracing::{debug, error, info, warn};

use lr_config::{ConfigManager, FreeTierFallback, FreeTierKind};
use lr_providers::registry::ProviderRegistry;
use lr_providers::{
    AudioTranscriptionRequest, AudioTranscriptionResponse, AudioTranslationRequest,
    AudioTranslationResponse, CompletionChunk, CompletionRequest, CompletionResponse,
    EmbeddingRequest, EmbeddingResponse, SpeechRequest, SpeechResponse,
};
use lr_types::{AppError, AppResult};

pub mod adaptive;
pub mod decision_routing;
pub mod endpoint_cache;
pub mod explanation;
pub mod free_tier;
mod messages;
mod privacy;
pub mod rate_limit;
pub mod response_cache;
mod route_rules;
mod search;
mod systemone;
mod upstream_tools;

// Re-export commonly used types
pub use endpoint_cache::EndpointCapabilityCache;
pub use free_tier::FreeTierManager;
pub use rate_limit::{RateLimiterManager, UsageInfo};

/// Router error classification for auto-routing fallback decisions
#[derive(Debug, Clone)]
pub enum RouterError {
    /// Provider rate limited - should retry with different model
    RateLimited {
        provider: String,
        model: String,
        retry_after_secs: u64,
    },
    /// Content policy violation - should retry with different model
    PolicyViolation {
        provider: String,
        model: String,
        reason: String,
    },
    /// Context length exceeded - should retry with different model
    ContextLengthExceeded {
        provider: String,
        model: String,
        /// The model's max context (tokens) if the provider surfaced it.
        /// `None` when we couldn't parse it out of the error body — prefer
        /// `None` over `0` so the log line doesn't read `"max: 0"`.
        max_tokens: Option<u64>,
        requested_tokens: Option<u64>,
    },
    /// Provider unreachable - should retry with different model
    Unreachable { provider: String, model: String },
    /// Endpoint not supported by this model/provider - should retry with different model
    EndpointNotSupported {
        provider: String,
        model: String,
        endpoint: String,
    },
    /// Other error - should not retry
    Other {
        provider: String,
        model: String,
        error: String,
    },
}

impl RouterError {
    /// Whether this error should trigger fallback to next model
    pub fn should_retry(&self) -> bool {
        matches!(
            self,
            RouterError::RateLimited { .. }
                | RouterError::Unreachable { .. }
                | RouterError::ContextLengthExceeded { .. }
                | RouterError::PolicyViolation { .. }
                | RouterError::EndpointNotSupported { .. }
        )
    }

    /// Classify an AppError into a RouterError
    pub fn classify(error: &AppError, provider: &str, model: &str) -> Self {
        match error {
            AppError::RoutingDeadlineExceeded { .. } => RouterError::Unreachable {
                provider: provider.into(),
                model: model.into(),
            },
            AppError::RoutingFailure { error, .. } => Self::classify(error, provider, model),
            // Prefer typed variants. Providers that parse their own error
            // bodies (OpenAI via `classify_openai_style_error`) produce these
            // so the router doesn't have to reverse-engineer the string.
            AppError::ContextLengthExceeded { max, requested } => {
                RouterError::ContextLengthExceeded {
                    provider: provider.to_string(),
                    model: model.to_string(),
                    max_tokens: *max,
                    requested_tokens: *requested,
                }
            }
            AppError::ModelNotFound { .. } => RouterError::EndpointNotSupported {
                provider: provider.to_string(),
                model: model.to_string(),
                endpoint: "model_not_found".to_string(),
            },
            AppError::RateLimitWithRetryAfter { retry_after_secs } => RouterError::RateLimited {
                provider: provider.into(),
                model: model.into(),
                retry_after_secs: *retry_after_secs,
            },
            AppError::ProviderUnavailable { .. } => RouterError::Unreachable {
                provider: provider.into(),
                model: model.into(),
            },
            AppError::RateLimitExceeded => RouterError::RateLimited {
                provider: provider.to_string(),
                model: model.to_string(),
                retry_after_secs: 60,
            },
            AppError::Provider(msg) if msg.contains("policy") || msg.contains("content_policy") => {
                RouterError::PolicyViolation {
                    provider: provider.to_string(),
                    model: model.to_string(),
                    reason: msg.clone(),
                }
            }
            // Backward-compat substring fallback: tightened to full phrases
            // rather than individual words so unrelated errors mentioning
            // "token" or "length" (e.g. "Bearer token invalid",
            // "Content-Length") don't get mis-classified as context-length.
            AppError::Provider(msg)
                if msg.contains("maximum context length")
                    || msg.contains("context_length_exceeded")
                    || msg.contains("context length exceeded") =>
            {
                RouterError::ContextLengthExceeded {
                    provider: provider.to_string(),
                    model: model.to_string(),
                    max_tokens: None,
                    requested_tokens: None,
                }
            }
            AppError::Provider(msg)
                if msg.contains("unreachable")
                    || msg.contains("timeout")
                    || msg.contains("connection") =>
            {
                RouterError::Unreachable {
                    provider: provider.to_string(),
                    model: model.to_string(),
                }
            }
            // Match our own trait default error messages from lr-providers (lines 192-299)
            AppError::Provider(msg)
                if msg.contains("does not support embeddings")
                    || msg.contains("does not support image generation")
                    || msg.contains("does not support audio transcription")
                    || msg.contains("does not support audio translation")
                    || msg.contains("does not support text-to-speech")
                    || msg.contains("does not support system one decisions")
                    || msg.contains("does not support chat completions") =>
            {
                RouterError::EndpointNotSupported {
                    provider: provider.to_string(),
                    model: model.to_string(),
                    endpoint: msg.clone(),
                }
            }
            AppError::Io(_) => RouterError::Unreachable {
                provider: provider.to_string(),
                model: model.to_string(),
            },
            _ => RouterError::Other {
                provider: provider.to_string(),
                model: model.to_string(),
                error: error.to_string(),
            },
        }
    }

    /// Short outcome string for monitor events
    pub fn outcome_str(&self) -> &'static str {
        match self {
            RouterError::RateLimited { .. } => "rate_limited",
            RouterError::PolicyViolation { .. } => "policy_violation",
            RouterError::ContextLengthExceeded { .. } => "context_length",
            RouterError::Unreachable { .. } => "unreachable",
            RouterError::EndpointNotSupported { .. } => "endpoint_not_supported",
            RouterError::Other { .. } => "error",
        }
    }

    /// Get a log-friendly string representation
    pub fn to_log_string(&self) -> String {
        match self {
            RouterError::RateLimited {
                provider,
                model,
                retry_after_secs,
            } => {
                format!(
                    "RATE_LIMITED: {}/{} (retry after {}s)",
                    provider, model, retry_after_secs
                )
            }
            RouterError::PolicyViolation {
                provider,
                model,
                reason,
            } => {
                format!("POLICY_VIOLATION: {}/{} - {}", provider, model, reason)
            }
            RouterError::ContextLengthExceeded {
                provider,
                model,
                max_tokens,
                requested_tokens,
            } => {
                // Only include "(max: N)" / "(requested: N)" when we
                // actually know the numbers — otherwise the message used to
                // read "(max: 0)", which was misleading.
                let mut extra = String::new();
                if let Some(m) = max_tokens {
                    extra.push_str(&format!(" (max: {})", m));
                }
                if let Some(r) = requested_tokens {
                    extra.push_str(&format!(" (requested: {})", r));
                }
                format!("CONTEXT_LENGTH_EXCEEDED: {}/{}{}", provider, model, extra)
            }
            RouterError::Unreachable { provider, model } => {
                format!("UNREACHABLE: {}/{}", provider, model)
            }
            RouterError::EndpointNotSupported {
                provider,
                model,
                endpoint,
            } => {
                format!(
                    "ENDPOINT_NOT_SUPPORTED: {}/{} - {}",
                    provider, model, endpoint
                )
            }
            RouterError::Other {
                provider,
                model,
                error,
            } => {
                format!("ERROR: {}/{} - {}", provider, model, error)
            }
        }
    }
}

/// Wraps a completion stream to set the resolved model and serving provider
/// on every chunk and to record usage (rate limits, free tier) once the
/// stream ends or is dropped.
///
/// Usage is the upstream's own (`CompletionChunk::usage`, usually on the last
/// chunk) when it reports one; otherwise an estimate from the whole prompt
/// (`prompt_estimate`) and every generated character, including reasoning
/// and tool-call arguments.
///
/// The `resolved_model` parameter should be in `provider/model` format (e.g., "openai/gpt-4o")
/// and will be set on each chunk to ensure clients know which model actually processed the request.
#[allow(clippy::too_many_arguments)]
async fn wrap_stream_with_usage_tracking(
    stream: Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>,
    client_id: String,
    resolved_model: String,
    prompt_estimate: u64,
    rate_limiter: Arc<RateLimiterManager>,
    free_tier_manager: Arc<FreeTierManager>,
    free_tier: FreeTierUsage,
    pricing: lr_providers::PricingInfo,
    free_tier_only: bool,
) -> Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>> {
    // Decide now, while the request's trace is still in scope (the stream is
    // polled later, outside it): a duplicate hop must not charge usage.
    let count_usage = !lr_types::is_duplicate_hop();
    let provider = resolved_model
        .split_once('/')
        .map(|(p, _)| p.to_string())
        .unwrap_or_default();

    let cache_pricing = pricing.clone();
    let recorder = Arc::new(StreamUsageRecorder {
        state: parking_lot::Mutex::new(StreamUsageState::default()),
        record: parking_lot::Mutex::new(count_usage.then(|| StreamUsageSink {
            client_id,
            provider: provider.clone(),
            prompt_estimate,
            rate_limiter,
            free_tier_manager,
            free_tier,
            pricing,
            free_tier_only,
        })),
    });

    let recorder_map = recorder.clone();
    let chunks = stream.map(move |chunk_result| {
        chunk_result.map(|mut chunk| {
            chunk.model = resolved_model.clone();
            chunk.provider = Some(provider.clone());
            if let Some(accounting) = chunk.usage.as_ref().and_then(|usage| {
                lr_providers::session::prompt_cache_accounting(&cache_pricing, usage)
            }) {
                chunk
                    .extensions
                    .get_or_insert_with(Default::default)
                    .insert(
                        lr_providers::session::ACCOUNTING.into(),
                        serde_json::json!(accounting),
                    );
            }
            recorder_map.observe(&chunk);
            chunk
        })
    });
    // After the last chunk: record now (a dropped stream records on drop).
    let end = futures::stream::once(async move {
        recorder.finish();
        None
    });
    Box::pin(
        chunks
            .map(Some)
            .chain(end)
            .filter_map(futures::future::ready),
    )
}

#[derive(Default)]
struct StreamUsageState {
    reported_cost: Option<f64>,
    output_chars: u64,
    upstream: Option<lr_providers::TokenUsage>,
}

/// Where and how to record a stream's usage.
struct StreamUsageSink {
    client_id: String,
    provider: String,
    prompt_estimate: u64,
    rate_limiter: Arc<RateLimiterManager>,
    free_tier_manager: Arc<FreeTierManager>,
    free_tier: FreeTierUsage,
    pricing: lr_providers::PricingInfo,
    free_tier_only: bool,
}

/// A provider's free tier and whether the model used counts as one of its
/// free models, for recording usage.
pub(crate) struct FreeTierUsage {
    pub(crate) kind: FreeTierKind,
    pub(crate) model_is_free: bool,
}

/// Collects a stream's output and records its usage exactly once: at the
/// end of the stream, or when the stream is dropped early (client gone).
struct StreamUsageRecorder {
    state: parking_lot::Mutex<StreamUsageState>,
    record: parking_lot::Mutex<Option<StreamUsageSink>>,
}

impl StreamUsageRecorder {
    fn observe(&self, chunk: &CompletionChunk) {
        let mut st = self.state.lock();
        if let Some(cost) = lr_providers::gateway::reported_cost(chunk.extensions.as_ref()) {
            st.reported_cost = Some(cost);
        }
        st.output_chars += lr_providers::usage_estimate::chunk_output_chars(chunk);
        if let Some(u) = &chunk.usage {
            st.upstream = Some(u.clone());
        }
    }

    fn finish(&self) {
        let Some(sink) = self.record.lock().take() else {
            return;
        };
        let st = std::mem::take(&mut *self.state.lock());
        let totals = stream_usage_totals(&st, sink.prompt_estimate);
        let cost = st
            .reported_cost
            .unwrap_or_else(|| sink.pricing.cost(&totals));
        let input = totals.prompt_tokens as u64;
        let output = totals.completion_tokens as u64;
        let usage = UsageInfo {
            input_tokens: input,
            output_tokens: output,
            cost_usd: cost,
        };
        let task = async move {
            if let Err(e) = sink
                .rate_limiter
                .record_api_key_usage(&sink.client_id, &usage)
                .await
            {
                warn!(
                    "Failed to record streaming usage for API key '{}': {}",
                    sink.client_id, e
                );
            }
            if !sink.provider.is_empty() {
                sink.free_tier_manager.record_usage(
                    &sink.provider,
                    &sink.free_tier.kind,
                    sink.free_tier.model_is_free,
                    input + output,
                    cost,
                );
                // Cost watchdog: track whether this stream cost money
                if sink.free_tier_only {
                    if cost > 0.0 {
                        sink.free_tier_manager.record_cost_trigger(&sink.provider);
                    } else {
                        sink.free_tier_manager.record_cost_free(&sink.provider);
                    }
                }
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(task);
            }
            Err(_) => warn!("No async runtime to record streaming usage"),
        }
    }
}

impl Drop for StreamUsageRecorder {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Token usage for a finished stream: the upstream's when reported, else
/// estimates (whole prompt, generated characters / 4).
fn stream_usage_totals(st: &StreamUsageState, prompt_estimate: u64) -> lr_providers::TokenUsage {
    match &st.upstream {
        Some(u) => u.clone(),
        None => {
            let prompt = prompt_estimate.min(u32::MAX as u64) as u32;
            let completion = lr_providers::usage_estimate::chars_to_tokens(st.output_chars)
                .min(u32::MAX as u64) as u32;
            lr_providers::TokenUsage {
                prompt_tokens: prompt,
                completion_tokens: completion,
                total_tokens: prompt.saturating_add(completion),
                prompt_tokens_details: None,
                completion_tokens_details: None,
            }
        }
    }
}

/// Calculate cost in USD from token usage and pricing info.
/// Reasoning tokens are typically included in `output_tokens` by providers,
/// so we subtract them to avoid double-counting, then charge reasoning tokens
/// at the reasoning rate (or output rate if no separate reasoning pricing).
fn calculate_cost(
    input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: Option<u64>,
    pricing: &lr_providers::PricingInfo,
) -> f64 {
    let input_cost = (input_tokens as f64 / 1000.0) * pricing.input_cost_per_1k;

    let reasoning_tokens = reasoning_tokens.unwrap_or(0);
    let non_reasoning_output = output_tokens.saturating_sub(reasoning_tokens);
    let reasoning_rate = pricing
        .reasoning_cost_per_1k
        .unwrap_or(pricing.output_cost_per_1k);
    let reasoning_cost = (reasoning_tokens as f64 / 1000.0) * reasoning_rate;
    let output_cost = (non_reasoning_output as f64 / 1000.0) * pricing.output_cost_per_1k;

    input_cost + output_cost + reasoning_cost
}

/// How a requested model matches a provider's model list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchKind {
    /// Provider exposes the model id verbatim (case-insensitive).
    Exact,
    /// Provider exposes the model only after stripping a `prefix/` and/or `:tag`.
    Normalized,
}

/// Router for handling completion requests with API key-based model selection
pub struct Router {
    response_cache: response_cache::ResponseCache,
    config_manager: Arc<ConfigManager>,
    provider_registry: Arc<ProviderRegistry>,
    rate_limiter: Arc<RateLimiterManager>,
    metrics_collector: Arc<lr_monitoring::metrics::MetricsCollector>,
    sessions: session_routing::SessionStore,
    decision_slots: tokio::sync::Semaphore,
    adaptive_measurements: Arc<adaptive::AdaptiveMeasurements>,
    free_tier_manager: Arc<FreeTierManager>,
    health_cache: Option<Arc<lr_providers::health_cache::HealthCacheManager>>,
    endpoint_cache: Arc<EndpointCapabilityCache>,
}

/// Summarize skip reasons from the `attempts` log as a human-readable string
/// like `"endpoint_not_supported: 1, no_tool_support: 2"`.
///
/// Used to produce a specific error when every prioritized model was skipped
/// by a pre-request filter (e.g. no function-calling support for a tools
/// request) so callers see the real reason instead of a generic "Unknown".
fn summarize_attempt_outcomes(attempts: &[serde_json::Value]) -> String {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for a in attempts {
        if let Some(outcome) = a.get("outcome").and_then(|v| v.as_str()) {
            *counts.entry(outcome).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .map(|(k, v)| format!("{}: {}", k, v))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Build routing metadata JSON for monitor events.
fn build_routing_metadata(
    decision: Option<&lr_providers::PreComputedRouting>,
    candidate_models: &[String],
    attempts: Vec<serde_json::Value>,
    successful_idx: Option<usize>,
) -> serde_json::Value {
    let attempts: Vec<serde_json::Value> = attempts
        .into_iter()
        .map(|attempt| {
            let mut safe = serde_json::json!({});
            for key in [
                "provider",
                "model",
                "outcome",
                "duration_ms",
                "exclusions",
                "unknown_checks",
                "retry_after_secs",
                "cost_usd",
                "cost_source",
            ] {
                if let Some(value) = attempt.get(key) {
                    safe[key] = value.clone();
                }
            }
            safe
        })
        .collect();
    let total = attempts.len();
    let mut info = serde_json::json!({
        "candidate_models": candidate_models,
        "attempts": attempts,
        "total_attempts": total,
    });
    if let Some(idx) = successful_idx {
        info["successful_attempt"] = serde_json::json!(idx);
    }
    if let Some(decision) = decision {
        info["decision_routing"] = serde_json::to_value(decision).unwrap_or_default();
        info["classifier_latency_ms"] = serde_json::json!(decision.latency_ms);
        info["classifier_cost_usd"] = serde_json::json!(decision.classifier_cost_usd);
        info["classifier_cost_estimated"] = serde_json::json!(decision.classifier_cost_estimated);
    }
    info
}

impl Router {
    /// The wire API LocalRouter speaks to a provider instance, by name.
    pub fn upstream_api(&self, provider: &str) -> Option<lr_types::LlmApi> {
        self.provider_registry
            .get_provider(provider)
            .map(|p| p.upstream_api())
    }

    /// Create a new router without free tier manager (for tests)
    #[cfg(test)]
    pub fn new_without_free_tier(
        config_manager: Arc<ConfigManager>,
        provider_registry: Arc<ProviderRegistry>,
        rate_limiter: Arc<RateLimiterManager>,
        metrics_collector: Arc<lr_monitoring::metrics::MetricsCollector>,
    ) -> Self {
        Self {
            response_cache: Default::default(),
            config_manager,
            provider_registry,
            rate_limiter,
            metrics_collector,
            sessions: Default::default(),
            decision_slots: tokio::sync::Semaphore::new(8),
            adaptive_measurements: Arc::new(adaptive::AdaptiveMeasurements::default()),
            free_tier_manager: Arc::new(FreeTierManager::new(None)),
            health_cache: None,
            endpoint_cache: Arc::new(EndpointCapabilityCache::new(
                std::time::Duration::from_secs(3600),
            )),
        }
    }

    /// Create a new router
    pub fn new(
        config_manager: Arc<ConfigManager>,
        provider_registry: Arc<ProviderRegistry>,
        rate_limiter: Arc<RateLimiterManager>,
        metrics_collector: Arc<lr_monitoring::metrics::MetricsCollector>,
        free_tier_manager: Arc<FreeTierManager>,
    ) -> Self {
        Self {
            response_cache: Default::default(),
            config_manager,
            provider_registry,
            rate_limiter,
            metrics_collector,
            sessions: Default::default(),
            decision_slots: tokio::sync::Semaphore::new(8),
            adaptive_measurements: Arc::new(adaptive::AdaptiveMeasurements::default()),
            free_tier_manager,
            health_cache: None,
            endpoint_cache: Arc::new(EndpointCapabilityCache::new(
                std::time::Duration::from_secs(3600),
            )),
        }
    }

    /// Set the health cache for on-failure health marking
    pub fn with_health_cache(
        mut self,
        health_cache: Arc<lr_providers::health_cache::HealthCacheManager>,
    ) -> Self {
        self.health_cache = Some(health_cache);
        self
    }

    /// Report a provider failure to the health cache (debounced)
    fn report_provider_failure(&self, provider: &str, error: &str) {
        if let Some(ref health_cache) = self.health_cache {
            let cooldown_secs = self.config_manager.get().health_check.failure_cooldown_secs;
            let cooldown = std::time::Duration::from_secs(cooldown_secs);
            if health_cache.report_provider_failure(provider, error.to_string(), cooldown) {
                warn!(
                    "Provider '{}' marked unhealthy due to request failure: {}",
                    provider, error
                );
            }
        }
    }

    /// Get the free tier manager
    pub fn free_tier_manager(&self) -> &Arc<FreeTierManager> {
        &self.free_tier_manager
    }

    /// Check if a model should be skipped for a given endpoint type.
    ///
    /// Uses three layers of filtering:
    /// 1. Negative capability cache (cached previous failures)
    /// 2. Provider-level support flags (supports_embeddings, etc.)
    /// 3. Model capability lookup (registry cache → catalog fallback)
    ///
    /// Returns true if the model should be skipped (incompatible).
    fn should_skip_for_endpoint(
        &self,
        provider: &str,
        model: &str,
        endpoint_type: lr_providers::EndpointType,
    ) -> bool {
        // Layer 1: Negative cache (cached previous runtime failures)
        if self
            .endpoint_cache
            .is_known_unsupported(provider, model, endpoint_type)
        {
            debug!(
                "Skipping {}/{} for {:?}: cached as unsupported",
                provider, model, endpoint_type
            );
            return true;
        }

        // Layer 2: Provider-level support
        if let Some(provider_instance) = self.provider_registry.get_provider(provider) {
            if !endpoint_type.is_supported_by_provider(provider_instance.as_ref()) {
                debug!(
                    "Skipping {}/{} for {:?}: provider does not support endpoint",
                    provider, model, endpoint_type
                );
                self.endpoint_cache
                    .record_unsupported(provider, model, endpoint_type);
                return true;
            }
        }

        // Layer 3: Model capability lookup (registry cache → catalog → None)
        if let Some(capabilities) = self
            .provider_registry
            .get_model_capabilities(provider, model)
        {
            if !endpoint_type.is_compatible_with(&capabilities) {
                debug!(
                    "Skipping {}/{} for {:?}: model capabilities {:?} incompatible",
                    provider, model, endpoint_type, capabilities
                );
                return true; // Known incompatible; don't cache since capabilities may refresh
            }
        }

        // Model not found or capabilities unknown → try it
        false
    }

    /// Get the effective free tier config for a provider instance.
    /// User override on provider config takes priority, then the factory
    /// default for the instance (which may depend on its configuration).
    fn get_effective_free_tier(&self, provider_instance: &str) -> lr_config::FreeTierKind {
        let config = self.config_manager.get();
        // User override on provider config takes priority
        if let Some(provider_config) = config
            .providers
            .iter()
            .find(|p| p.name == provider_instance)
        {
            if let Some(ref free_tier) = provider_config.free_tier {
                return free_tier.clone();
            }
            return self
                .provider_registry
                .default_free_tier_for_instance(provider_instance, &provider_config.provider_type);
        }
        lr_config::FreeTierKind::None
    }

    /// Whether `model` is free on `provider` under `free_tier`, and why.
    /// Fetches the provider's model list first when a rule needs listing
    /// data and the list was never fetched.
    pub(crate) async fn free_model_decision(
        &self,
        provider: &str,
        model: &str,
        free_tier: &lr_config::FreeTierKind,
    ) -> lr_providers::free_models::FreeModelDecision {
        let evaluator = lr_providers::free_models::FreeTierEvaluator::new(free_tier);
        if evaluator.needs_metadata() && !self.provider_registry.has_cached_models(provider) {
            let _ = self
                .provider_registry
                .list_provider_models_cached(provider)
                .await;
        }
        self.free_model_decision_cached(provider, model, &evaluator)
    }

    /// [`Self::free_model_decision`] from what is cached, without fetching.
    fn free_model_decision_cached(
        &self,
        provider: &str,
        model: &str,
        evaluator: &lr_providers::free_models::FreeTierEvaluator,
    ) -> lr_providers::free_models::FreeModelDecision {
        let config = self.config_manager.get();
        self.provider_registry.decide_free_model(
            provider,
            model,
            evaluator,
            &config.pricing_overrides,
        )
    }

    /// Whether a model counts as free for free-tier-only strategies: the
    /// model's own free decision (rules, lists, prices) combined with the
    /// provider's limits.
    pub(crate) async fn classify_for_free_tier(
        &self,
        provider: &str,
        model: &str,
    ) -> free_tier::ModelFreeStatus {
        let free_tier = self.get_effective_free_tier(provider);
        self.classify_with_free_tier(provider, model, &free_tier)
            .await
    }

    pub(crate) async fn classify_with_free_tier(
        &self,
        provider: &str,
        model: &str,
        free_tier: &lr_config::FreeTierKind,
    ) -> free_tier::ModelFreeStatus {
        let decision = self.free_model_decision(provider, model, free_tier).await;
        self.free_tier_manager
            .classify_model(provider, free_tier, decision.is_free)
    }

    /// The free tier to record usage under, with whether the model counts
    /// as free (only free models count against a free-models-only limit).
    pub(crate) fn free_tier_usage(&self, provider: &str, model: &str) -> FreeTierUsage {
        let kind = self.get_effective_free_tier(provider);
        let model_is_free = match kind {
            lr_config::FreeTierKind::FreeModelsOnly { .. } => {
                let evaluator = lr_providers::free_models::FreeTierEvaluator::new(&kind);
                self.free_model_decision_cached(provider, model, &evaluator)
                    .is_free
            }
            _ => true,
        };
        FreeTierUsage {
            kind,
            model_is_free,
        }
    }

    /// Refuse a model the user turned off on its provider. Models the
    /// provider does not know of pass through.
    pub(crate) fn require_enabled_model(&self, provider: &str, model: &str) -> AppResult<()> {
        if self.provider_registry.is_model_disabled(provider, model) {
            return Err(AppError::InvalidParams(format!(
                "Model '{}/{}' is turned off in LocalRouter (provider '{}', Models tab)",
                provider, model, provider
            )));
        }
        Ok(())
    }

    /// Build the appropriate error when free-tier models are exhausted.
    /// Returns `FreeTierFallbackAvailable` if fallback is Ask or Allow,
    /// otherwise returns `FreeTierExhausted`.
    fn free_tier_exhausted_error(
        strategy: &lr_config::Strategy,
        retry_after_secs: u64,
        exhausted_models: Vec<(String, String)>,
    ) -> AppError {
        match strategy.free_tier_fallback {
            FreeTierFallback::Off => AppError::FreeTierExhausted { retry_after_secs },
            FreeTierFallback::Ask | FreeTierFallback::Allow => {
                AppError::FreeTierFallbackAvailable {
                    retry_after_secs,
                    exhausted_models,
                }
            }
        }
    }

    /// Complete with paid fallback (free-tier disabled, uses all models)
    pub async fn complete_with_paid_fallback(
        &self,
        client_id: &str,
        request: CompletionRequest,
    ) -> AppResult<(CompletionResponse, Option<serde_json::Value>)> {
        let mut request = request;
        self.prepare_upstream_tools(client_id, &mut request)?;
        let (_client, mut strategy) = self.validate_client_and_strategy(client_id)?;
        strategy.free_tier_only = false;

        if request.model == "localrouter/auto" {
            self.complete_with_auto_routing(client_id, &strategy, request)
                .await
        } else {
            let (provider, model) = Self::parse_model_string(&request.model);
            let (final_provider, final_model) = if provider.is_empty() {
                self.find_provider_for_model(&model, &strategy).await?
            } else {
                (provider, model)
            };
            self.execute_request(client_id, &final_provider, &final_model, request, false)
                .await
                .map(|r| (r, None))
        }
    }

    /// Stream complete with paid fallback (free-tier disabled, uses all models)
    pub async fn stream_complete_with_paid_fallback(
        &self,
        client_id: &str,
        request: CompletionRequest,
    ) -> AppResult<(
        Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>,
        Option<serde_json::Value>,
    )> {
        let mut request = request;
        self.prepare_upstream_tools(client_id, &mut request)?;
        let (_client, mut strategy) = self.validate_client_and_strategy(client_id)?;
        strategy.free_tier_only = false;

        if request.model == "localrouter/auto" {
            self.stream_complete_with_auto_routing(client_id, &strategy, request)
                .await
        } else {
            let (provider, model) = Self::parse_model_string(&request.model);
            let (final_provider, final_model) = if provider.is_empty() {
                self.find_provider_for_model(&model, &strategy).await?
            } else {
                (provider, model)
            };
            self.require_enabled_model(&final_provider, &final_model)?;
            let provider_instance = self
                .provider_registry
                .get_provider(&final_provider)
                .ok_or_else(|| {
                    AppError::Router(format!("Provider '{}' not found", final_provider))
                })?;
            let pricing = self.request_pricing(&final_provider, &final_model).await;
            let free_tier = self.free_tier_usage(&final_provider, &final_model);
            let (modified_request, explicit_session_handle) = self.provider_session_request(
                client_id,
                &final_provider,
                &request,
                request.endpoint_type(),
            )?;
            let mut modified_request = modified_request.without_routing_metadata();
            modified_request.model = final_model.clone();
            let prompt_estimate = lr_providers::usage_estimate::prompt_tokens(
                &modified_request.messages,
                modified_request.tools.as_deref(),
            );
            let observation = self.adaptive_measurements.begin(
                &final_provider,
                &final_model,
                request.endpoint_type(),
                self.provider_registry.routing_revision(),
            );
            let first_output_deadline =
                adaptive::current_turn().and_then(|turn| turn.attempt_deadline());
            let stream = match adaptive::dispatch_in_turn(async {
                adaptive::reserve_inference_dispatch()?;
                lr_providers::messages_stream::dispatch_stream(
                    provider_instance.as_ref(),
                    modified_request,
                )
                .await
            })
            .await
            {
                Ok(s) => s,
                Err(e) => {
                    self.record_retry_hint(&final_provider, &final_model, &e);
                    let router_error = RouterError::classify(&e, &final_provider, &final_model);
                    if matches!(router_error, RouterError::Unreachable { .. }) {
                        self.report_provider_failure(&final_provider, &e.to_string());
                    }
                    return Err(e);
                }
            };
            if let Some(handle) = &explicit_session_handle {
                handle.record(&final_provider, &final_model);
            }
            let stream = adaptive::observe_stream(
                stream,
                observation,
                adaptive::current_turn().and_then(|turn| turn.deadline()),
                first_output_deadline,
                false,
            );
            let stream = if let Some(turn) = adaptive::current_turn() {
                adaptive::routing_stream(
                    stream,
                    turn.metadata().unwrap_or_default(),
                    turn.cost_handle(),
                    pricing.clone(),
                )
            } else {
                stream
            };
            let resolved_model = format!("{}/{}", final_provider, final_model);
            Ok((
                wrap_stream_with_usage_tracking(
                    stream,
                    client_id.to_string(),
                    resolved_model,
                    prompt_estimate,
                    self.rate_limiter.clone(),
                    self.free_tier_manager.clone(),
                    free_tier,
                    pricing,
                    strategy.free_tier_only,
                )
                .await,
                None,
            ))
        }
    }

    /// Parse model string into (provider, model) tuple
    /// Supports formats: "provider/model" or just "model" (returns empty provider)
    fn parse_model_string(model: &str) -> (String, String) {
        if let Some((provider, model_name)) = model.split_once('/') {
            (provider.to_string(), model_name.to_string())
        } else {
            (String::new(), model.to_string())
        }
    }

    /// Validate client exists and is enabled, then get their strategy
    /// Returns 403 (via AppError::Unauthorized) if client is invalid or disabled
    fn validate_client_and_strategy(
        &self,
        client_id: &str,
    ) -> AppResult<(lr_config::Client, lr_config::Strategy)> {
        let config = self.config_manager.get();

        let client = config
            .clients
            .iter()
            .find(|c| c.id == client_id)
            .ok_or_else(|| {
                warn!("Client '{}' not found", client_id);
                AppError::Unauthorized
            })?
            .clone();

        if !client.credentials_active() {
            warn!("Client '{}' is disabled", client_id);
            return Err(AppError::Unauthorized);
        }

        let strategy = config
            .strategies
            .iter()
            .find(|s| s.id == client.strategy_id)
            .ok_or_else(|| {
                warn!(
                    "Strategy '{}' not found for client '{}'",
                    client.strategy_id, client_id
                );
                AppError::Router(format!("Strategy '{}' not found", client.strategy_id))
            })?
            .clone();

        Ok((client, strategy))
    }

    /// Check client-level rate limits
    async fn check_client_rate_limits(&self, client_id: &str) -> AppResult<()> {
        let usage_estimate = UsageInfo {
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
        };

        let rate_check = self
            .rate_limiter
            .check_api_key(client_id, &usage_estimate)
            .await?;

        if !rate_check.allowed {
            warn!(
                "Client '{}' rate limited. Retry after {} seconds",
                client_id,
                rate_check.retry_after_secs.unwrap_or(0)
            );
            return Err(AppError::RateLimitExceeded);
        }

        Ok(())
    }

    /// Find provider for a model when no provider is specified in model string.
    ///
    /// Match priority: exact id match (case-insensitive) wins over normalized
    /// match. Without this, a request for `gpt-5.5` would tie between ChatGPT
    /// Plus (which exposes id `gpt-5.5`) and OpenRouter (which exposes
    /// `openai/gpt-5.5`, normalizing to the same value), and iteration order
    /// would silently pick the wrong provider.
    async fn find_provider_for_model(
        &self,
        model: &str,
        strategy: &lr_config::Strategy,
    ) -> AppResult<(String, String)> {
        let normalized_requested = Self::normalize_model_id(model);
        let allowed = &strategy.allowed_models;

        let mut exact_match: Option<String> = None;
        let mut normalized_match: Option<String> = None;

        // selected_models: explicit (provider, model) pairs already chosen by the user.
        for (prov, mod_name) in &allowed.selected_models {
            if self.provider_registry.is_model_disabled(prov, mod_name) {
                continue;
            }
            if mod_name.eq_ignore_ascii_case(model) {
                if exact_match.is_none() {
                    exact_match = Some(prov.clone());
                }
            } else if normalized_match.is_none()
                && Self::normalize_model_id(mod_name) == normalized_requested
            {
                normalized_match = Some(prov.clone());
            }
        }

        // Build the ordered list of provider instances to scan.
        let mut candidates: Vec<String> = allowed.selected_providers.clone();
        if allowed.selected_all {
            for inst in self.provider_registry.list_providers() {
                if !candidates.iter().any(|c| c == &inst.instance_name) {
                    candidates.push(inst.instance_name);
                }
            }
        }

        for prov in candidates {
            if exact_match.is_some() {
                break;
            }
            if self.provider_registry.get_provider(&prov).is_none() {
                continue;
            }
            // The offered models (hidden ones excluded), from the model
            // cache: fetched only when it is empty or expired.
            let Ok(models) = self
                .provider_registry
                .list_provider_models_cached(&prov)
                .await
            else {
                continue;
            };
            let model_ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
            match Self::classify_match(model, &model_ids) {
                Some(MatchKind::Exact) => exact_match = Some(prov),
                Some(MatchKind::Normalized) if normalized_match.is_none() => {
                    normalized_match = Some(prov);
                }
                _ => {}
            }
        }

        if let Some(prov) = exact_match.or(normalized_match) {
            return Ok((prov, model.to_string()));
        }

        Err(AppError::Router(format!(
            "Model '{}' is not allowed by this strategy",
            model
        )))
    }

    /// Classify how a list of provider-supplied model ids matches a requested model.
    /// Exact id match (case-insensitive) is preferred over normalized match.
    fn classify_match(requested: &str, available_ids: &[&str]) -> Option<MatchKind> {
        let normalized_requested = Self::normalize_model_id(requested);
        let mut found_normalized = false;
        for id in available_ids {
            if id.eq_ignore_ascii_case(requested) {
                return Some(MatchKind::Exact);
            }
            if !found_normalized && Self::normalize_model_id(id) == normalized_requested {
                found_normalized = true;
            }
        }
        if found_normalized {
            Some(MatchKind::Normalized)
        } else {
            None
        }
    }

    /// Normalize a model ID for comparison
    ///
    /// Different providers return model IDs in different formats:
    /// - Ollama: "llama2" or "llama2:latest"
    /// - OpenAI-compatible: "provider/model" or just "model"
    /// - LMStudio: "model-name"
    ///
    /// This function normalizes by:
    /// 1. Stripping provider prefix (e.g., "openai/gpt-4" -> "gpt-4")
    /// 2. Stripping tag suffix (e.g., "llama2:latest" -> "llama2")
    ///
    /// Returns the normalized model name for case-insensitive comparison
    fn normalize_model_id(model_id: &str) -> String {
        // Strip provider prefix if present
        let without_provider = if let Some((_provider, model)) = model_id.split_once('/') {
            model
        } else {
            model_id
        };

        // Strip tag suffix if present (e.g., ":latest", ":7b")
        let without_tag = if let Some((base, _tag)) = without_provider.split_once(':') {
            base
        } else {
            without_provider
        };

        without_tag.to_lowercase()
    }

    /// Check strategy rate limits using metrics-based approach
    /// Returns error if projected usage would exceed any configured limits
    fn check_strategy_rate_limits(
        &self,
        strategy: &lr_config::Strategy,
        _provider: &str,
        _model: &str,
    ) -> AppResult<()> {
        if strategy.rate_limits.is_empty() {
            return Ok(());
        }

        // Get pre-estimates from recent usage (last 10 minutes)
        let (avg_tokens, avg_cost) = self
            .metrics_collector
            .get_pre_estimate_for_strategy(&strategy.id, 10);

        for limit in &strategy.rate_limits {
            if !limit.enabled {
                continue;
            }
            let window_secs = limit.time_window.to_seconds();

            let (current_requests, current_tokens, current_cost) = self
                .metrics_collector
                .get_recent_usage_for_strategy(&strategy.id, window_secs);

            let (projected_usage, limit_value) = match limit.limit_type {
                lr_config::RateLimitType::Requests => (current_requests as f64 + 1.0, limit.value),
                lr_config::RateLimitType::TotalTokens => {
                    (current_tokens as f64 + avg_tokens, limit.value)
                }
                lr_config::RateLimitType::Cost => {
                    // Special case: if avg_cost is 0 (free models), don't count against cost limit
                    if avg_cost == 0.0 {
                        continue;
                    }
                    (current_cost + avg_cost, limit.value)
                }
                _ => continue, // InputTokens/OutputTokens not supported for pre-check
            };

            if projected_usage > limit_value {
                warn!(
                    "Strategy rate limit exceeded: {} (current: {:.2}, projected: {:.2}, limit: {:.2})",
                    match limit.limit_type {
                        lr_config::RateLimitType::Requests => "requests",
                        lr_config::RateLimitType::TotalTokens => "tokens",
                        lr_config::RateLimitType::Cost => "cost",
                        _ => "unknown",
                    },
                    if matches!(limit.limit_type, lr_config::RateLimitType::Requests) {
                        current_requests as f64
                    } else if matches!(limit.limit_type, lr_config::RateLimitType::TotalTokens) {
                        current_tokens as f64
                    } else {
                        current_cost
                    },
                    projected_usage,
                    limit_value
                );
                return Err(AppError::RateLimitExceeded);
            }
        }

        Ok(())
    }

    pub fn auto_turn_budget(
        &self,
        client_id: &str,
        request: &CompletionRequest,
    ) -> Option<Arc<adaptive::TurnBudget>> {
        if request.model != "localrouter/auto" {
            return None;
        }
        let (_, strategy) = self.validate_client_and_strategy(client_id).ok()?;
        let config = &strategy.auto_config.as_ref()?.adaptive_routing;
        Some(adaptive::TurnBudget::new(
            config,
            request
                .pre_computed_routing
                .as_ref()
                .map_or(0, |decision| decision.latency_ms),
        ))
    }
    pub fn turn_deadline_error(&self, client_id: &str, request: &CompletionRequest) -> AppError {
        match self.validate_client_and_strategy(client_id) {
            Ok((_, strategy)) => self.deadline_routing_error(
                &strategy,
                request,
                request.endpoint_type(),
                "total_deadline_exceeded",
            ),
            Err(error) => error,
        }
    }

    fn record_retry_hint(&self, provider: &str, _model: &str, error: &AppError) {
        match error {
            AppError::RateLimitWithRetryAfter { retry_after_secs } => self
                .free_tier_manager
                .record_server_retry_after(provider, *retry_after_secs),
            AppError::ProviderUnavailable {
                retry_after_secs: Some(seconds),
                ..
            } => self
                .free_tier_manager
                .record_server_retry_after(provider, *seconds),
            AppError::RoutingFailure { error, .. } => {
                self.record_retry_hint(provider, _model, error)
            }
            _ => {}
        }
    }

    async fn request_pricing(&self, provider: &str, model: &str) -> lr_providers::PricingInfo {
        let config = self.config_manager.get();
        if let Some(pricing) = self.provider_registry.effective_pricing_override(
            provider,
            model,
            &config.pricing_overrides,
        ) {
            return lr_providers::PricingInfo {
                input_cost_per_1k: pricing.input_per_million / 1000.0,
                output_cost_per_1k: pricing.output_per_million / 1000.0,
                ..lr_providers::PricingInfo::free()
            };
        }
        self.provider_registry.get_pricing(provider, model).await
    }

    /// Start accounting synchronously after a completed upstream response.
    /// Awaiting this task preserves ordinary ordering; dropping the await leaves
    /// the owned local accounting job running instead of losing known charges.
    fn completed_usage(
        &self,
        client_id: &str,
        provider: &str,
        model: &str,
        usage: UsageInfo,
        free_tier_only: bool,
    ) -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        if lr_types::is_duplicate_hop() {
            return Box::pin(async {});
        }
        let free_tier = self.free_tier_usage(provider, model);
        self.free_tier_manager.record_usage(
            provider,
            &free_tier.kind,
            free_tier.model_is_free,
            usage.input_tokens.saturating_add(usage.output_tokens),
            usage.cost_usd,
        );
        if free_tier_only {
            if usage.cost_usd > 0.0 {
                self.free_tier_manager.record_cost_trigger(provider);
            } else {
                self.free_tier_manager.record_cost_free(provider);
            }
        }
        let rate_limiter = self.rate_limiter.clone();
        let client_id = client_id.to_string();
        let work = async move {
            if let Err(error) = rate_limiter.record_api_key_usage(&client_id, &usage).await {
                warn!(
                    "Failed to record usage for client '{}': {}",
                    client_id, error
                );
            }
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            let job = lr_types::spawn_traced(work);
            Box::pin(async move {
                if let Err(error) = job.await {
                    warn!("Usage accounting job failed: {error}");
                }
            })
        } else {
            Box::pin(work)
        }
    }

    /// Execute a completion request on a specific provider/model
    /// This is the core execution logic extracted from the old complete() method
    async fn execute_request(
        &self,
        client_id: &str,
        provider: &str,
        model: &str,
        request: CompletionRequest,
        free_tier_only: bool,
    ) -> AppResult<CompletionResponse> {
        adaptive::dispatch_in_turn(Box::pin(self.execute_request_inner(
            client_id,
            provider,
            model,
            request,
            free_tier_only,
        )))
        .await
    }

    async fn execute_request_inner(
        &self,
        client_id: &str,
        provider: &str,
        model: &str,
        request: CompletionRequest,
        free_tier_only: bool,
    ) -> AppResult<CompletionResponse> {
        self.require_enabled_model(provider, model)?;
        // Get provider instance from registry
        let provider_instance = self
            .provider_registry
            .get_provider(provider)
            .ok_or_else(|| {
                AppError::Router(format!(
                    "Provider '{}' not found or disabled in registry",
                    provider
                ))
            })?;

        // Modify the request to use just the model name (without provider prefix)
        if request
            .extensions
            .as_ref()
            .is_some_and(|e| e.contains_key(lr_providers::gateway::EXTENSION))
            && !provider_instance.supports_gateway_options()
        {
            return Err(AppError::InvalidParams(
                "Selected provider does not support gateway routing controls".into(),
            ));
        }
        let (modified_request, explicit_session_handle) =
            self.provider_session_request(client_id, provider, &request, request.endpoint_type())?;
        let mut modified_request = modified_request.without_routing_metadata();
        modified_request.model = model.to_string();

        // Apply feature adapters if extensions are present
        if let Some(ref extensions) = request.extensions {
            for (feature_name, feature_params) in extensions {
                if matches!(
                    feature_name.as_str(),
                    lr_providers::gateway::EXTENSION
                        | lr_providers::session::EXTENSION
                        | lr_providers::session::CACHE_KEY
                        | lr_providers::session::CONTROL_TOKEN
                        | lr_providers::replay::EXTENSION
                        | lr_providers::response_cache::EXTENSION
                        | lr_providers::response_cache::SCOPE
                        | lr_providers::privacy::INTERNAL_EXTENSION
                        | lr_providers::privacy::ROUTE_CLIENT_EXTENSION
                        | lr_providers::server_tools::INGRESS_EXTENSION
                ) {
                    continue;
                }
                // Check if provider supports this feature
                if provider_instance.supports_feature(feature_name) {
                    debug!(
                        "Provider '{}' supports feature '{}'",
                        provider, feature_name
                    );

                    // Get the feature adapter
                    if let Some(adapter) = provider_instance.get_feature_adapter(feature_name) {
                        // Validate parameters
                        let mut params: lr_providers::features::FeatureParams =
                            std::collections::HashMap::new();
                        if let serde_json::Value::Object(map) = feature_params {
                            for (k, v) in map {
                                params.insert(k.clone(), v.clone());
                            }
                        }
                        adapter.validate_params(&params).map_err(|e| {
                            warn!(
                                "Feature '{}' parameter validation failed: {}",
                                feature_name, e
                            );
                            e
                        })?;

                        // Adapt the request
                        adapter
                            .adapt_request(&mut modified_request, &params)
                            .map_err(|e| {
                                warn!(
                                    "Feature '{}' request adaptation failed: {}",
                                    feature_name, e
                                );
                                e
                            })?;

                        debug!(
                            "Successfully applied feature adapter for '{}'",
                            feature_name
                        );
                    }
                } else {
                    warn!(
                        "Provider '{}' does not support feature '{}', ignoring",
                        provider, feature_name
                    );
                }
            }
        }

        provider_instance.validate_gateway_options(&request)?;
        let cache_ticket = self.chat_cache_ticket(client_id, provider, model, &request);
        if let Some(ticket) = &cache_ticket {
            if let Some(response) = self
                .chat_cache_hit(ticket, client_id, provider, model, free_tier_only)
                .await
            {
                return Ok(response);
            }
        }
        let pricing = self.request_pricing(provider, model).await;
        adaptive::reserve_inference_dispatch()?;
        let mut observation = self.adaptive_measurements.begin(
            provider,
            model,
            request.endpoint_type(),
            self.provider_registry.routing_revision(),
        );
        // Execute the completion
        let mut response = match lr_providers::messages::dispatch_complete(
            provider_instance.as_ref(),
            modified_request,
        )
        .await
        {
            Ok(resp) => resp,
            Err(e) => {
                // Mark provider unhealthy for infrastructure errors (not content/policy/rate-limit)
                self.record_retry_hint(provider, model, &e);
                let router_error = RouterError::classify(&e, provider, model);
                if matches!(router_error, RouterError::Unreachable { .. }) {
                    observation.infrastructure_failure(adaptive::current_infrastructure_cooldown());
                    self.report_provider_failure(provider, &e.to_string());
                }
                return Err(e);
            }
        };

        observation.success();
        drop(observation);
        if let Some(handle) = &explicit_session_handle {
            handle.record(provider, model);
        }
        if let Some(accounting) =
            lr_providers::session::prompt_cache_accounting(&pricing, &response.usage)
        {
            response
                .extensions
                .get_or_insert_with(Default::default)
                .insert(
                    lr_providers::session::ACCOUNTING.into(),
                    serde_json::json!(accounting),
                );
        }

        let reported_cost = lr_providers::gateway::reported_cost(response.extensions.as_ref());
        let cost = reported_cost.unwrap_or_else(|| pricing.cost(&response.usage));
        adaptive::record_completed_cost(cost, reported_cost.is_some());
        let accounting = self.completed_usage(
            client_id,
            provider,
            model,
            UsageInfo {
                input_tokens: response.usage.prompt_tokens as u64,
                output_tokens: response.usage.completion_tokens as u64,
                cost_usd: cost,
            },
            free_tier_only,
        );
        // Apply feature adapters to response if needed
        if let Some(ref extensions) = request.extensions {
            for feature_name in extensions.keys() {
                if provider_instance.supports_feature(feature_name) {
                    if let Some(adapter) = provider_instance.get_feature_adapter(feature_name) {
                        adapter.adapt_response(&mut response)?;
                    }
                }
            }
        }

        // Add provider and model information to response
        response.provider = provider.to_string();
        response.model = model.to_string();

        accounting.await;
        if let Some(ticket) = cache_ticket {
            if response_cache::cacheable_chat_response(&response) {
                let details = response.usage.prompt_tokens_details.as_ref();
                let rates_known = details.is_none_or(|d| {
                    (d.cache_read_tokens.or(d.cached_tokens).unwrap_or(0) == 0
                        || pricing.cache_read_cost_per_1k.is_some())
                        && (d.cache_creation_tokens.unwrap_or(0) == 0
                            || pricing.cache_write_cost_per_1k.is_some())
                });
                let reference = self.cache_reference_cost(
                    client_id,
                    provider,
                    model,
                    reported_cost,
                    cost,
                    rates_known,
                );
                if let Some(token) = self.response_cache.defer(ticket, reference) {
                    response
                        .extensions
                        .get_or_insert_with(Default::default)
                        .insert(
                            lr_providers::response_cache::PENDING.into(),
                            serde_json::json!(token),
                        );
                }
            }
        }
        Ok(response)
    }

    /// Select a policy route, then its configured fallback and ordinary priority list.
    async fn select_models_for_auto_routing(
        &self,
        client_id: &str,
        strategy: &lr_config::Strategy,
        auto_config: &lr_config::AutoModelConfig,
        request: &CompletionRequest,
        endpoint: lr_providers::EndpointType,
    ) -> AppResult<(
        Vec<(String, String)>,
        Option<lr_providers::PreComputedRouting>,
    )> {
        if request
            .pre_computed_routing
            .as_ref()
            .and_then(|decision| decision.route_config_fingerprint.as_ref())
            .is_some_and(|saved| {
                saved != &explanation::fingerprint(serde_json::json!(strategy.routing_scope()))
            })
        {
            return Err(AppError::InvalidParams(
                "Routing configuration changed during preflight; retry with a fresh turn".into(),
            ));
        }
        let bound = self.bound_routing_decision(client_id, strategy, request, endpoint)?;
        let Some(policy) = auto_config.routing_policy.as_ref().filter(|p| p.enabled) else {
            return Ok((auto_config.prioritized_models.clone(), bound));
        };
        let decision = match bound.as_ref().or(request.pre_computed_routing.as_ref()) {
            Some(d) => d.clone(),
            None => {
                self.evaluate_routing_policy_for_endpoint(
                    client_id, strategy, policy, request, endpoint,
                )
                .await
            }
        };
        let models = decision_routing::route_models(
            policy,
            &auto_config.prioritized_models,
            &decision.route,
        );
        Ok((models, Some(decision)))
    }

    /// Complete with auto-routing (localrouter/auto virtual model)
    /// Tries models in prioritized order with intelligent fallback.
    /// Returns (response, optional routing metadata as JSON).
    async fn complete_with_auto_routing(
        &self,
        client_id: &str,
        strategy: &lr_config::Strategy,
        request: CompletionRequest,
    ) -> AppResult<(CompletionResponse, Option<serde_json::Value>)> {
        let auto_config = strategy.auto_config.as_ref().ok_or_else(|| {
            AppError::Router("localrouter/auto not configured for this strategy".into())
        })?;

        if !auto_config.permission.is_enabled() {
            return Err(AppError::Router(
                "localrouter/auto is disabled for this strategy".into(),
            ));
        }

        let mut budget = adaptive::RoutingBudget::new(&auto_config.adaptive_routing);
        if let Some(decision) = &request.pre_computed_routing {
            budget.account_classifier_latency(decision.latency_ms);
        }
        let (selected_models, routing_decision) = budget
            .preflight(self.select_models_for_auto_routing(
                client_id,
                strategy,
                auto_config,
                &request,
                request.endpoint_type(),
            ))
            .await
            .map_err(|reason| {
                self.deadline_routing_error(strategy, &request, request.endpoint_type(), reason)
            })??;
        let (selected_models, scored_candidates, session_handle) = self.order_auto_models(
            client_id,
            strategy,
            &request,
            &selected_models,
            routing_decision.as_ref(),
            request.endpoint_type(),
        )?;

        if selected_models.is_empty() {
            return Err(AppError::Router(
                "No models available for auto-routing".into(),
            ));
        }

        info!(
            "Auto-routing for client '{}' with {} selected models",
            client_id,
            selected_models.len()
        );

        let mut initial_metadata = serde_json::json!({});
        if let Some(handle) = &session_handle {
            initial_metadata["session"] = handle.metadata();
        }
        self.attach_routing_explanation(
            &mut initial_metadata,
            strategy,
            &request,
            routing_decision.as_ref(),
        );
        initial_metadata["explanation"]["routes"][0]["candidates"] =
            serde_json::json!(scored_candidates);
        let mut last_error = None;
        let mut min_retry_after: u64 = u64::MAX;
        let candidate_models: Vec<String> = selected_models
            .iter()
            .map(|(p, m)| format!("{}/{}", p, m))
            .collect();
        let mut attempts: Vec<serde_json::Value> = Vec::new();

        for (idx, (provider, model)) in selected_models.iter().enumerate() {
            if let Some(reason) = budget.exhausted() {
                attempts
                    .push(serde_json::json!({"provider":provider,"model":model,"outcome":reason}));
                break;
            }
            let assessment = self.assess_chat_candidate(strategy, provider, model, &request, idx);
            if let Some(outcome) = assessment.exclusions.first() {
                if let Some(seconds) = assessment.retry_after_secs {
                    min_retry_after = min_retry_after.min(seconds);
                    last_error = Some(RouterError::RateLimited {
                        provider: provider.clone(),
                        model: model.clone(),
                        retry_after_secs: seconds,
                    });
                }
                attempts.push(serde_json::json!({"provider":provider,"model":model,"outcome":outcome,"exclusions":assessment.exclusions,"unknown_checks":assessment.unknown_checks}));
                continue;
            }
            // Check backoff state (skip providers with recent 429/402)
            if let Some(backoff) = self.free_tier_manager.is_in_backoff(provider, model) {
                debug!("Skipping {}/{}: {}", provider, model, backoff.reason);
                min_retry_after = min_retry_after.min(backoff.retry_after_secs);
                last_error = Some(RouterError::RateLimited {
                    provider: provider.clone(),
                    model: model.clone(),
                    retry_after_secs: backoff.retry_after_secs,
                });
                attempts.push(serde_json::json!({"provider": provider, "model": model, "outcome": "backoff", "error": &backoff.reason}));
                continue;
            }

            // Check free tier constraints (when free_tier_only is enabled)
            if strategy.free_tier_only {
                let status = budget
                    .preflight(self.classify_for_free_tier(provider, model))
                    .await
                    .map_err(|reason| {
                        self.deadline_routing_error(
                            strategy,
                            &request,
                            request.endpoint_type(),
                            reason,
                        )
                    })?;
                match status {
                    free_tier::ModelFreeStatus::AlwaysFree
                    | free_tier::ModelFreeStatus::FreeWithinLimits
                    | free_tier::ModelFreeStatus::FreeModel => { /* allow */ }
                    free_tier::ModelFreeStatus::NotFree => {
                        debug!("Skipping {}/{} in free-tier-only mode", provider, model);
                        attempts.push(serde_json::json!({"provider": provider, "model": model, "outcome": "not_free"}));
                        continue;
                    }
                }

                // Cost watchdog: skip provider if in cost-based backoff
                if let Some(retry_secs) = self.free_tier_manager.check_cost_backoff(provider) {
                    debug!(
                        "Skipping {}/{}: cost backoff ({}s remaining)",
                        provider, model, retry_secs
                    );
                    last_error = Some(RouterError::RateLimited {
                        provider: provider.clone(),
                        model: model.clone(),
                        retry_after_secs: retry_secs,
                    });
                    attempts.push(serde_json::json!({"provider": provider, "model": model, "outcome": "cost_backoff", "error": format!("Cost backoff ({}s)", retry_secs)}));
                    continue;
                }
            }

            // Check strategy rate limits before trying this model
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
                attempts.push(serde_json::json!({"provider": provider, "model": model, "outcome": "rate_limited", "error": e.to_string()}));
                continue;
            }

            let attempt_revision = self.provider_registry.routing_revision();
            let attempt_start = std::time::Instant::now();
            match budget
                .dispatch(self.execute_request(
                    client_id,
                    provider,
                    model,
                    request.clone(),
                    strategy.free_tier_only,
                ))
                .await
            {
                Ok(response) => {
                    let attempt_ms = attempt_start.elapsed().as_millis() as u64;
                    info!("Auto-routing succeeded with {}/{}", provider, model);
                    if let Some(handle) = &session_handle {
                        handle.record(provider, model);
                    }
                    if !lr_providers::response_cache::is_hit(response.extensions.as_ref()) {
                        self.free_tier_manager.clear_backoff(provider, model);
                    }
                    attempts.push(serde_json::json!({"provider": provider, "model": model, "outcome": if lr_providers::response_cache::is_hit(response.extensions.as_ref()) {"cache_hit"} else {"success"}, "duration_ms": attempt_ms,"cost_usd":lr_providers::gateway::reported_cost(response.extensions.as_ref()),"cost_source":if lr_providers::response_cache::is_hit(response.extensions.as_ref()) {"local_cache"} else if lr_providers::gateway::reported_cost(response.extensions.as_ref()).is_some(){"reported"}else{"unknown"}}));
                    let mut routing_meta = build_routing_metadata(
                        routing_decision.as_ref(),
                        &candidate_models,
                        attempts,
                        Some(idx),
                    );
                    routing_meta["attempt_costs"] = serde_json::json!(budget.costs());
                    routing_meta["inference_dispatches"] =
                        serde_json::json!(budget.dispatch_count());
                    routing_meta
                        .as_object_mut()
                        .unwrap()
                        .extend(initial_metadata.as_object().unwrap().clone());
                    adaptive::remember_turn_metadata(&routing_meta);
                    let mut response = response;
                    response
                        .extensions
                        .get_or_insert_with(Default::default)
                        .insert("localrouter.routing".into(), routing_meta.clone());
                    return Ok((response, Some(routing_meta)));
                }
                Err(e) => {
                    let attempt_ms = attempt_start.elapsed().as_millis() as u64;
                    if matches!(e, AppError::RoutingDeadlineExceeded { .. }) {
                        self.adaptive_measurements.record_infrastructure_failure(
                            provider,
                            model,
                            request.endpoint_type(),
                            attempt_revision,
                            auto_config.adaptive_routing.infrastructure_cooldown,
                        );
                    }
                    self.record_retry_hint(provider, model, &e);
                    let router_error = RouterError::classify(&e, provider, model);
                    warn!(
                        "Auto-routing attempt failed: {}",
                        router_error.to_log_string()
                    );

                    // Cache endpoint incompatibility for future requests
                    if matches!(router_error, RouterError::EndpointNotSupported { .. }) {
                        self.endpoint_cache.record_unsupported(
                            provider,
                            model,
                            request.endpoint_type(),
                        );
                    }

                    // Record backoff for rate-limited errors
                    if router_error.should_retry() {
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
                    }

                    attempts.push(serde_json::json!({"provider": provider, "model": model, "outcome": router_error.outcome_str(), "error": router_error.to_log_string(), "duration_ms": attempt_ms}));
                    last_error = Some(router_error.clone());

                    // Continue to next model on retryable errors
                    if request.server_tools.is_some() || !router_error.should_retry() {
                        error!(
                            "Non-retryable error encountered: {}",
                            router_error.to_log_string()
                        );
                        let mut metadata = build_routing_metadata(
                            routing_decision.as_ref(),
                            &candidate_models,
                            attempts,
                            None,
                        );
                        metadata
                            .as_object_mut()
                            .unwrap()
                            .extend(initial_metadata.as_object().unwrap().clone());
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

        // All models failed
        if strategy.free_tier_only {
            let retry_after = self
                .free_tier_manager
                .get_min_retry_after(&selected_models)
                .unwrap_or(60);
            return Err(Self::free_tier_exhausted_error(
                strategy,
                retry_after,
                selected_models,
            ));
        }

        // No model produced an error because every candidate was filtered
        // out before any request was attempted. Surface the specific reason
        // instead of a generic "Unknown" so the user can fix the strategy.
        let detail = match last_error {
            Some(e) => e.outcome_str().to_string(),
            None => {
                let summary = summarize_attempt_outcomes(&attempts);
                if summary.is_empty() {
                    format!(
                        "all {} prioritized models skipped by pre-request filters",
                        selected_models.len()
                    )
                } else {
                    format!(
                        "all {} prioritized models skipped by pre-request filters ({})",
                        selected_models.len(),
                        summary
                    )
                }
            }
        };
        let mut metadata =
            build_routing_metadata(routing_decision.as_ref(), &candidate_models, attempts, None);
        metadata
            .as_object_mut()
            .unwrap()
            .extend(initial_metadata.as_object().unwrap().clone());
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
            message: format!("All auto-routing models failed. Last error: {detail}"),
            explanation: Box::new(metadata),
        })
    }

    /// Stream complete with auto-routing (localrouter/auto virtual model for streaming)
    /// Tries models in prioritized order with intelligent fallback
    async fn stream_complete_with_auto_routing(
        &self,
        client_id: &str,
        strategy: &lr_config::Strategy,
        request: CompletionRequest,
    ) -> AppResult<(
        Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>,
        Option<serde_json::Value>,
    )> {
        let auto_config = strategy.auto_config.as_ref().ok_or_else(|| {
            AppError::Router("localrouter/auto not configured for this strategy".into())
        })?;

        if !auto_config.permission.is_enabled() {
            return Err(AppError::Router(
                "localrouter/auto is disabled for this strategy".into(),
            ));
        }

        let mut budget = adaptive::RoutingBudget::new(&auto_config.adaptive_routing);
        if let Some(decision) = &request.pre_computed_routing {
            budget.account_classifier_latency(decision.latency_ms);
        }
        let (selected_models, routing_decision) = budget
            .preflight(self.select_models_for_auto_routing(
                client_id,
                strategy,
                auto_config,
                &request,
                request.endpoint_type(),
            ))
            .await
            .map_err(|reason| {
                self.deadline_routing_error(strategy, &request, request.endpoint_type(), reason)
            })??;
        let (selected_models, scored_candidates, session_handle) = self.order_auto_models(
            client_id,
            strategy,
            &request,
            &selected_models,
            routing_decision.as_ref(),
            request.endpoint_type(),
        )?;

        if selected_models.is_empty() {
            return Err(AppError::Router(
                "No models available for auto-routing".into(),
            ));
        }

        info!(
            "Auto-routing streaming for client '{}' with {} selected models",
            client_id,
            selected_models.len()
        );

        let mut initial_metadata = serde_json::json!({});
        if let Some(handle) = &session_handle {
            initial_metadata["session"] = handle.metadata();
        }
        self.attach_routing_explanation(
            &mut initial_metadata,
            strategy,
            &request,
            routing_decision.as_ref(),
        );
        initial_metadata["explanation"]["routes"][0]["candidates"] =
            serde_json::json!(scored_candidates);
        let mut last_error = None;
        let candidate_models: Vec<String> = selected_models
            .iter()
            .map(|(p, m)| format!("{}/{}", p, m))
            .collect();
        let mut attempts: Vec<serde_json::Value> = Vec::new();

        for (idx, (provider, model)) in selected_models.iter().enumerate() {
            if let Some(reason) = budget.exhausted() {
                attempts
                    .push(serde_json::json!({"provider":provider,"model":model,"outcome":reason}));
                break;
            }
            let assessment = self.assess_chat_candidate(strategy, provider, model, &request, idx);
            if let Some(outcome) = assessment.exclusions.first() {
                if let Some(seconds) = assessment.retry_after_secs {
                    last_error = Some(RouterError::RateLimited {
                        provider: provider.clone(),
                        model: model.clone(),
                        retry_after_secs: seconds,
                    });
                }
                attempts.push(serde_json::json!({"provider":provider,"model":model,"outcome":outcome,"exclusions":assessment.exclusions,"unknown_checks":assessment.unknown_checks}));
                continue;
            }
            // Check backoff state (skip providers with recent 429/402)
            if let Some(backoff) = self.free_tier_manager.is_in_backoff(provider, model) {
                debug!("Skipping {}/{}: {}", provider, model, backoff.reason);
                last_error = Some(RouterError::RateLimited {
                    provider: provider.clone(),
                    model: model.clone(),
                    retry_after_secs: backoff.retry_after_secs,
                });
                attempts.push(serde_json::json!({"provider": provider, "model": model, "outcome": "backoff", "error": &backoff.reason}));
                continue;
            }

            // Check free tier constraints (when free_tier_only is enabled)
            if strategy.free_tier_only {
                let status = budget
                    .preflight(self.classify_for_free_tier(provider, model))
                    .await
                    .map_err(|reason| {
                        self.deadline_routing_error(
                            strategy,
                            &request,
                            request.endpoint_type(),
                            reason,
                        )
                    })?;
                match status {
                    free_tier::ModelFreeStatus::AlwaysFree
                    | free_tier::ModelFreeStatus::FreeWithinLimits
                    | free_tier::ModelFreeStatus::FreeModel => { /* allow */ }
                    free_tier::ModelFreeStatus::NotFree => {
                        debug!("Skipping {}/{} in free-tier-only mode", provider, model);
                        attempts.push(serde_json::json!({"provider": provider, "model": model, "outcome": "not_free"}));
                        continue;
                    }
                }

                // Cost watchdog: skip provider if in cost-based backoff
                if let Some(retry_secs) = self.free_tier_manager.check_cost_backoff(provider) {
                    debug!(
                        "Skipping {}/{}: cost backoff ({}s remaining)",
                        provider, model, retry_secs
                    );
                    last_error = Some(RouterError::RateLimited {
                        provider: provider.clone(),
                        model: model.clone(),
                        retry_after_secs: retry_secs,
                    });
                    attempts.push(serde_json::json!({"provider": provider, "model": model, "outcome": "cost_backoff", "error": format!("Cost backoff ({}s)", retry_secs)}));
                    continue;
                }
            }

            // Check strategy rate limits before trying this model
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
                attempts.push(serde_json::json!({"provider": provider, "model": model, "outcome": "rate_limited", "error": e.to_string()}));
                continue;
            }

            // Get provider instance
            let provider_instance = match self.provider_registry.get_provider(provider) {
                Some(p) => p,
                None => {
                    warn!("Provider '{}' not found, trying next model", provider);
                    last_error = Some(RouterError::Unreachable {
                        provider: provider.clone(),
                        model: model.clone(),
                    });
                    attempts.push(serde_json::json!({"provider": provider, "model": model, "outcome": "provider_not_found"}));
                    continue;
                }
            };

            // Execute streaming request
            if request
                .extensions
                .as_ref()
                .is_some_and(|e| e.contains_key(lr_providers::gateway::EXTENSION))
                && !provider_instance.supports_gateway_options()
            {
                return Err(AppError::InvalidParams(
                    "Selected provider does not support gateway routing controls".into(),
                ));
            }
            let (modified_request, _explicit_session_handle) = self.provider_session_request(
                client_id,
                provider,
                &request,
                request.endpoint_type(),
            )?;
            let mut modified_request = modified_request.without_routing_metadata();
            modified_request.model = model.clone();

            let prompt_estimate = lr_providers::usage_estimate::prompt_tokens(
                &modified_request.messages,
                modified_request.tools.as_deref(),
            );
            let pricing = budget
                .preflight(self.request_pricing(provider, model))
                .await
                .map_err(|reason| {
                    self.deadline_routing_error(strategy, &request, request.endpoint_type(), reason)
                })?;
            let observation = self.adaptive_measurements.begin(
                provider,
                model,
                request.endpoint_type(),
                self.provider_registry.routing_revision(),
            );
            let first_output_deadline = budget.attempt_deadline();
            match budget
                .dispatch(async {
                    adaptive::reserve_inference_dispatch()?;
                    lr_providers::messages_stream::dispatch_stream(
                        provider_instance.as_ref(),
                        modified_request,
                    )
                    .await
                })
                .await
            {
                Ok(stream) => {
                    let stream = adaptive::observe_stream(
                        stream,
                        observation,
                        budget.deadline,
                        first_output_deadline,
                        auto_config.adaptive_routing.infrastructure_cooldown,
                    );
                    info!(
                        "Auto-routing streaming succeeded with {}/{}",
                        provider, model
                    );
                    if let Some(handle) = &session_handle {
                        handle.record(provider, model);
                    }
                    self.free_tier_manager.clear_backoff(provider, model);
                    attempts.push(serde_json::json!({"provider": provider, "model": model, "outcome": "success"}));
                    let mut routing_meta = build_routing_metadata(
                        routing_decision.as_ref(),
                        &candidate_models,
                        attempts,
                        Some(idx),
                    );
                    routing_meta["attempt_costs"] = serde_json::json!(budget.costs());
                    routing_meta["inference_dispatches"] =
                        serde_json::json!(budget.dispatch_count());
                    routing_meta
                        .as_object_mut()
                        .unwrap()
                        .extend(initial_metadata.as_object().unwrap().clone());
                    adaptive::remember_turn_metadata(&routing_meta);
                    let stream = adaptive::routing_stream(
                        stream,
                        routing_meta.clone(),
                        budget.cost_handle(),
                        pricing.clone(),
                    );
                    let resolved_model = format!("{}/{}", provider, model);
                    let free_tier = self.free_tier_usage(provider, model);
                    return Ok((
                        wrap_stream_with_usage_tracking(
                            stream,
                            client_id.to_string(),
                            resolved_model,
                            prompt_estimate,
                            self.rate_limiter.clone(),
                            self.free_tier_manager.clone(),
                            free_tier,
                            pricing,
                            strategy.free_tier_only,
                        )
                        .await,
                        Some(routing_meta),
                    ));
                }
                Err(e) => {
                    let mut observation = observation;
                    self.record_retry_hint(provider, model, &e);
                    let router_error = RouterError::classify(&e, provider, model);
                    warn!(
                        "Auto-routing streaming attempt failed: {}",
                        router_error.to_log_string()
                    );

                    if matches!(router_error, RouterError::Unreachable { .. }) {
                        observation.infrastructure_failure(
                            auto_config.adaptive_routing.infrastructure_cooldown,
                        );
                    }
                    drop(observation);
                    // Mark provider unhealthy for infrastructure errors
                    if matches!(router_error, RouterError::Unreachable { .. }) {
                        self.report_provider_failure(provider, &e.to_string());
                    }

                    // Cache endpoint incompatibility for future requests
                    if matches!(router_error, RouterError::EndpointNotSupported { .. }) {
                        self.endpoint_cache.record_unsupported(
                            provider,
                            model,
                            request.endpoint_type(),
                        );
                    }

                    // Record backoff for rate-limited errors
                    if router_error.should_retry() {
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
                    }

                    attempts.push(serde_json::json!({"provider":provider,"model":model,"outcome":router_error.outcome_str()}));
                    last_error = Some(router_error.clone());

                    // Continue to next model on retryable errors
                    if request.server_tools.is_some() || !router_error.should_retry() {
                        error!(
                            "Non-retryable error encountered: {}",
                            router_error.to_log_string()
                        );
                        let mut metadata = build_routing_metadata(
                            routing_decision.as_ref(),
                            &candidate_models,
                            attempts,
                            None,
                        );
                        metadata
                            .as_object_mut()
                            .unwrap()
                            .extend(initial_metadata.as_object().unwrap().clone());
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

        // All models failed
        if strategy.free_tier_only {
            let retry_after = self
                .free_tier_manager
                .get_min_retry_after(&selected_models)
                .unwrap_or(60);
            return Err(Self::free_tier_exhausted_error(
                strategy,
                retry_after,
                selected_models,
            ));
        }

        // Same "all filtered out" reporting as the non-streaming path: when
        // no request was ever attempted, say *why* models were skipped.
        let detail = match last_error {
            Some(e) => e.outcome_str().to_string(),
            None => {
                let summary = summarize_attempt_outcomes(&attempts);
                if summary.is_empty() {
                    format!(
                        "all {} prioritized models skipped by pre-request filters",
                        selected_models.len()
                    )
                } else {
                    format!(
                        "all {} prioritized models skipped by pre-request filters ({})",
                        selected_models.len(),
                        summary
                    )
                }
            }
        };
        let mut metadata =
            build_routing_metadata(routing_decision.as_ref(), &candidate_models, attempts, None);
        metadata
            .as_object_mut()
            .unwrap()
            .extend(initial_metadata.as_object().unwrap().clone());
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
            message: format!("All auto-routing streaming models failed. Last error: {detail}"),
            explanation: Box::new(metadata),
        })
    }

    /// Try completion with prioritized list of models (with automatic retry on failure)
    ///
    /// Tries each model in the prioritized list in order until one succeeds.
    /// Retries on specific errors like provider unavailable, rate limit, or model not found.
    /// Records usage for rate limiting on success.
    #[deprecated(note = "Use complete_with_auto_routing instead")]
    #[allow(dead_code)]
    async fn complete_with_prioritized_list(
        &self,
        client_id: &str,
        prioritized_models: &[(String, String)],
        mut request: CompletionRequest,
    ) -> AppResult<CompletionResponse> {
        let mut last_error = None;

        for (idx, (provider_name, model_name)) in prioritized_models.iter().enumerate() {
            debug!(
                "Trying prioritized model {}/{}: provider='{}', model='{}'",
                idx + 1,
                prioritized_models.len(),
                provider_name,
                model_name
            );

            // Get provider instance
            let provider_instance = match self.provider_registry.get_provider(provider_name) {
                Some(p) => p,
                None => {
                    warn!(
                        "Provider '{}' not found or disabled, trying next model",
                        provider_name
                    );
                    last_error = Some(AppError::Router(format!(
                        "Provider '{}' not found or disabled",
                        provider_name
                    )));
                    continue;
                }
            };

            // Update request with this model
            request.model = model_name.clone();

            // Apply the same live tool/privacy checks as direct dispatch.
            let (mut request, _) = self.provider_session_request(
                client_id,
                provider_name,
                &request,
                request.endpoint_type(),
            )?;
            request.model = model_name.clone();
            let request = request.without_routing_metadata();
            // Try the completion
            match lr_providers::messages::dispatch_complete(
                provider_instance.as_ref(),
                request.clone(),
            )
            .await
            {
                Ok(response) => {
                    info!(
                        "Prioritized model succeeded: provider='{}', model='{}'",
                        provider_name, model_name
                    );

                    // Calculate cost and record usage for rate limiting
                    let pricing = self
                        .provider_registry
                        .get_pricing(provider_name, model_name)
                        .await;

                    let cost = lr_providers::gateway::reported_cost(response.extensions.as_ref())
                        .unwrap_or_else(|| pricing.cost(&response.usage));

                    let usage = UsageInfo {
                        input_tokens: response.usage.prompt_tokens as u64,
                        output_tokens: response.usage.completion_tokens as u64,
                        cost_usd: cost,
                    };

                    // Log error but don't fail the request if usage recording fails
                    // The provider already succeeded and consumed tokens/cost
                    if let Err(e) = self
                        .rate_limiter
                        .record_api_key_usage(client_id, &usage)
                        .await
                    {
                        warn!(
                            "Failed to record usage for API key '{}': {}. Request succeeded but usage not tracked.",
                            client_id, e
                        );
                    }

                    return Ok(response);
                }
                Err(e) => {
                    warn!(
                        "Model failed: provider='{}', model='{}', error='{}'",
                        provider_name, model_name, e
                    );

                    // Determine if we should retry with next model. An
                    // upstream 4xx (ProviderStatus) is still worth trying on
                    // the next model — another model may accept the request —
                    // but if it turns out to be the final error we preserve
                    // its status rather than flattening it into a 502 (below).
                    let should_retry = matches!(
                        e,
                        AppError::Provider(_)
                            | AppError::ProviderStatus { .. }
                            | AppError::RateLimitExceeded
                            | AppError::Router(_)
                            | AppError::Internal(_)
                    );

                    if request.server_tools.is_some() || !should_retry {
                        // Non-retryable error (e.g., validation error) - fail immediately
                        warn!("Non-retryable error, stopping retry attempts");
                        return Err(e);
                    }

                    // Store error message for later use. Preserve an upstream
                    // 4xx status through the wrap so a permanent request error
                    // surfaces as a 4xx even after exhausting the model list;
                    // other errors collapse to a generic Router error (502).
                    last_error = Some(match e {
                        AppError::ProviderStatus { status, message } => AppError::ProviderStatus {
                            status,
                            message: format!(
                                "Model '{}' from provider '{}' failed: {}",
                                model_name, provider_name, message
                            ),
                        },
                        other => AppError::Router(format!(
                            "Model '{}' from provider '{}' failed: {}",
                            model_name, provider_name, other
                        )),
                    });

                    // Continue to next model
                    if idx < prioritized_models.len() - 1 {
                        debug!("Retrying with next prioritized model...");
                    }
                }
            }
        }

        // All models failed
        Err(last_error.unwrap_or_else(|| {
            AppError::Router("All prioritized models failed or no models configured".to_string())
        }))
    }

    /// Route a completion request based on client configuration
    ///
    /// This method:
    /// 1. Validates the client exists and is enabled
    /// 2. Gets the client's routing strategy
    /// 3. Routes based on requested model (auto vs specific)
    /// 4. Executes the request with appropriate fallback behavior
    ///
    /// Returns 403 (via AppError::Unauthorized) if client is invalid or disabled
    pub async fn complete(
        &self,
        client_id: &str,
        request: CompletionRequest,
    ) -> AppResult<(CompletionResponse, Option<serde_json::Value>)> {
        let mut request = request;
        self.prepare_privacy_request(client_id, &mut request)?;
        self.prepare_upstream_tools(client_id, &mut request)?;
        lr_providers::session::prepare_turn(&mut request)?;
        debug!(
            "Routing completion request for client '{}', model '{}'",
            client_id, request.model
        );

        lr_providers::gateway::GatewayPreferences::default()
            .effective(request.extensions.as_ref())?;

        // Special handling for internal test token (bypasses all routing config)
        if client_id == "internal-test" || client_id == "memory-service" {
            debug!("Internal test token detected - bypassing routing config (test mode only)");
            let (provider, model) = Self::parse_model_string(&request.model);
            if provider.is_empty() {
                return Err(AppError::Router(
                    "Internal test requires provider/model format".into(),
                ));
            }
            return self
                .execute_request(client_id, &provider, &model, request, false)
                .await
                .map(|r| (r, None));
        }

        // 1. Validate client and get strategy
        let (_client, strategy) = self.validate_client_and_strategy(client_id)?;

        // 2. Check client-level rate limits
        self.check_client_rate_limits(client_id).await?;

        // 3. Route based on requested model
        if request.model == "localrouter/auto" {
            debug!("Using auto-routing for client '{}'", client_id);
            return self
                .complete_with_auto_routing(client_id, &strategy, request)
                .await;
        }

        // 4. For specific model requests, check strategy rate limits
        self.check_strategy_rate_limits(&strategy, "", "")?;

        // 5. Parse and validate model
        let (provider, model) = Self::parse_model_string(&request.model);

        let (final_provider, final_model) = if provider.is_empty() {
            self.find_provider_for_model(&model, &strategy).await?
        } else {
            // Provider specified - validate it's allowed
            if !strategy.is_model_allowed(&provider, &model) {
                return Err(AppError::Router(format!(
                    "Model '{}/{}' is not allowed by this strategy",
                    provider, model
                )));
            }
            (provider, model)
        };

        // 6. Check free tier constraints for specific model requests
        if strategy.free_tier_only {
            let exhausted = vec![(final_provider.clone(), final_model.clone())];
            // Check backoff first
            if let Some(backoff) = self
                .free_tier_manager
                .is_in_backoff(&final_provider, &final_model)
            {
                debug!(
                    "Specific model {}/{} is in backoff: {}",
                    final_provider, final_model, backoff.reason
                );
                return Err(Self::free_tier_exhausted_error(
                    &strategy,
                    backoff.retry_after_secs,
                    exhausted,
                ));
            }

            let status = self
                .classify_for_free_tier(&final_provider, &final_model)
                .await;
            if matches!(status, free_tier::ModelFreeStatus::NotFree) {
                debug!(
                    "Model {}/{} is not free, rejecting in free-tier-only mode",
                    final_provider, final_model
                );
                return Err(Self::free_tier_exhausted_error(&strategy, 0, exhausted));
            }
        }

        // 7. Execute the request
        debug!(
            "Executing request for client '{}' on {}/{}",
            client_id, final_provider, final_model
        );

        self.execute_request(
            client_id,
            &final_provider,
            &final_model,
            request,
            strategy.free_tier_only,
        )
        .await
        .map(|r| (r, None))
    }

    /// Route a streaming completion request based on API key configuration
    pub async fn stream_complete(
        &self,
        client_id: &str,
        request: CompletionRequest,
    ) -> AppResult<(
        Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>,
        Option<serde_json::Value>,
    )> {
        let mut request = request;
        self.prepare_privacy_request(client_id, &mut request)?;
        self.prepare_upstream_tools(client_id, &mut request)?;
        lr_providers::session::prepare_turn(&mut request)?;
        debug!(
            "Routing streaming completion request for client '{}', model '{}'",
            client_id, request.model
        );

        lr_providers::gateway::GatewayPreferences::default()
            .effective(request.extensions.as_ref())?;

        // Special handling for internal test token
        if client_id == "internal-test" || client_id == "memory-service" {
            debug!("Internal test token detected - bypassing routing config for streaming (test mode only)");
            let (provider, model) = Self::parse_model_string(&request.model);
            if provider.is_empty() {
                return Err(AppError::Router(
                    "Internal test requires provider/model format".into(),
                ));
            }
            self.require_enabled_model(&provider, &model)?;
            let provider_instance = self
                .provider_registry
                .get_provider(&provider)
                .ok_or_else(|| AppError::Router(format!("Provider '{}' not found", provider)))?;
            let pricing = self.provider_registry.get_pricing(&provider, &model).await;
            let free_tier = self.free_tier_usage(&provider, &model);
            if request
                .extensions
                .as_ref()
                .is_some_and(|e| e.contains_key(lr_providers::gateway::EXTENSION))
                && !provider_instance.supports_gateway_options()
            {
                return Err(AppError::InvalidParams(
                    "Selected provider does not support gateway routing controls".into(),
                ));
            }
            let (modified_request, explicit_session_handle) = self.provider_session_request(
                client_id,
                &provider,
                &request,
                request.endpoint_type(),
            )?;
            let mut modified_request = modified_request.without_routing_metadata();
            modified_request.model = model.clone();
            let prompt_estimate = lr_providers::usage_estimate::prompt_tokens(
                &modified_request.messages,
                modified_request.tools.as_deref(),
            );
            let observation = self.adaptive_measurements.begin(
                &provider,
                &model,
                request.endpoint_type(),
                self.provider_registry.routing_revision(),
            );
            let first_output_deadline =
                adaptive::current_turn().and_then(|turn| turn.attempt_deadline());
            let stream = match adaptive::dispatch_in_turn(async {
                adaptive::reserve_inference_dispatch()?;
                lr_providers::messages_stream::dispatch_stream(
                    provider_instance.as_ref(),
                    modified_request,
                )
                .await
            })
            .await
            {
                Ok(s) => s,
                Err(e) => {
                    self.record_retry_hint(&provider, &model, &e);
                    let router_error = RouterError::classify(&e, &provider, &model);
                    if matches!(router_error, RouterError::Unreachable { .. }) {
                        self.report_provider_failure(&provider, &e.to_string());
                    }
                    return Err(e);
                }
            };
            if let Some(handle) = &explicit_session_handle {
                handle.record(&provider, &model);
            }
            let stream = adaptive::observe_stream(
                stream,
                observation,
                adaptive::current_turn().and_then(|turn| turn.deadline()),
                first_output_deadline,
                false,
            );
            let stream = if let Some(turn) = adaptive::current_turn() {
                adaptive::routing_stream(
                    stream,
                    turn.metadata().unwrap_or_default(),
                    turn.cost_handle(),
                    pricing.clone(),
                )
            } else {
                stream
            };
            let resolved_model = format!("{}/{}", provider, model);
            return Ok((
                wrap_stream_with_usage_tracking(
                    stream,
                    client_id.to_string(),
                    resolved_model,
                    prompt_estimate,
                    self.rate_limiter.clone(),
                    self.free_tier_manager.clone(),
                    free_tier,
                    pricing,
                    false, // internal test — not free tier mode
                )
                .await,
                None,
            ));
        }

        // 1. Validate client and get strategy
        let (_client, strategy) = self.validate_client_and_strategy(client_id)?;

        // 2. Check rate limits
        self.check_client_rate_limits(client_id).await?;

        // 3. Route - handle auto-routing for streaming
        if request.model == "localrouter/auto" {
            debug!("Using auto-routing for streaming client '{}'", client_id);
            return self
                .stream_complete_with_auto_routing(client_id, &strategy, request)
                .await;
        }

        // 4. Parse and validate model
        let (provider, model) = Self::parse_model_string(&request.model);

        let (final_provider, final_model) = if provider.is_empty() {
            self.find_provider_for_model(&model, &strategy).await?
        } else {
            if !strategy.is_model_allowed(&provider, &model) {
                return Err(AppError::Router(format!(
                    "Model '{}/{}' not allowed",
                    provider, model
                )));
            }
            (provider, model)
        };

        // 5. Check strategy rate limits
        self.check_strategy_rate_limits(&strategy, &final_provider, &final_model)?;

        // 6. Check free tier constraints for specific model requests
        if strategy.free_tier_only {
            let exhausted = vec![(final_provider.clone(), final_model.clone())];
            if let Some(backoff) = self
                .free_tier_manager
                .is_in_backoff(&final_provider, &final_model)
            {
                debug!(
                    "Specific model {}/{} is in backoff: {}",
                    final_provider, final_model, backoff.reason
                );
                return Err(Self::free_tier_exhausted_error(
                    &strategy,
                    backoff.retry_after_secs,
                    exhausted,
                ));
            }

            let status = self
                .classify_for_free_tier(&final_provider, &final_model)
                .await;
            if matches!(status, free_tier::ModelFreeStatus::NotFree) {
                debug!(
                    "Model {}/{} is not free, rejecting in free-tier-only mode (streaming)",
                    final_provider, final_model
                );
                return Err(Self::free_tier_exhausted_error(&strategy, 0, exhausted));
            }
        }

        // 7. Execute streaming request
        self.require_enabled_model(&final_provider, &final_model)?;
        let provider_instance = self
            .provider_registry
            .get_provider(&final_provider)
            .ok_or_else(|| AppError::Router(format!("Provider '{}' not found", final_provider)))?;

        let pricing = self
            .provider_registry
            .get_pricing(&final_provider, &final_model)
            .await;
        let free_tier = self.free_tier_usage(&final_provider, &final_model);
        if request
            .extensions
            .as_ref()
            .is_some_and(|e| e.contains_key(lr_providers::gateway::EXTENSION))
            && !provider_instance.supports_gateway_options()
        {
            return Err(AppError::InvalidParams(
                "Selected provider does not support gateway routing controls".into(),
            ));
        }
        let (modified_request, explicit_session_handle) = self.provider_session_request(
            client_id,
            &final_provider,
            &request,
            request.endpoint_type(),
        )?;
        let mut modified_request = modified_request.without_routing_metadata();
        modified_request.model = final_model.clone();
        let prompt_estimate = lr_providers::usage_estimate::prompt_tokens(
            &modified_request.messages,
            modified_request.tools.as_deref(),
        );
        let observation = self.adaptive_measurements.begin(
            &final_provider,
            &final_model,
            request.endpoint_type(),
            self.provider_registry.routing_revision(),
        );
        let first_output_deadline =
            adaptive::current_turn().and_then(|turn| turn.attempt_deadline());
        let stream = match adaptive::dispatch_in_turn(async {
            adaptive::reserve_inference_dispatch()?;
            lr_providers::messages_stream::dispatch_stream(
                provider_instance.as_ref(),
                modified_request,
            )
            .await
        })
        .await
        {
            Ok(s) => s,
            Err(e) => {
                self.record_retry_hint(&final_provider, &final_model, &e);
                let router_error = RouterError::classify(&e, &final_provider, &final_model);
                if matches!(router_error, RouterError::Unreachable { .. }) {
                    self.report_provider_failure(&final_provider, &e.to_string());
                }
                return Err(e);
            }
        };
        if let Some(handle) = &explicit_session_handle {
            handle.record(&final_provider, &final_model);
        }
        let stream = adaptive::observe_stream(
            stream,
            observation,
            adaptive::current_turn().and_then(|turn| turn.deadline()),
            first_output_deadline,
            false,
        );
        let stream = if let Some(turn) = adaptive::current_turn() {
            adaptive::routing_stream(
                stream,
                turn.metadata().unwrap_or_default(),
                turn.cost_handle(),
                pricing.clone(),
            )
        } else {
            stream
        };
        let resolved_model = format!("{}/{}", final_provider, final_model);
        Ok((
            wrap_stream_with_usage_tracking(
                stream,
                client_id.to_string(),
                resolved_model,
                prompt_estimate,
                self.rate_limiter.clone(),
                self.free_tier_manager.clone(),
                free_tier,
                pricing,
                strategy.free_tier_only,
            )
            .await,
            None,
        ))
    }

    /// Execute an embedding request on a specific provider/model
    async fn execute_embedding_request(
        &self,
        client_id: &str,
        provider: &str,
        model: &str,
        request: EmbeddingRequest,
    ) -> AppResult<EmbeddingResponse> {
        self.require_enabled_model(provider, model)?;
        self.check_provider_privacy(client_id, provider, false)?;
        // Get provider instance from registry
        let provider_instance = self
            .provider_registry
            .get_provider(provider)
            .ok_or_else(|| {
                AppError::Router(format!(
                    "Provider '{}' not found or disabled in registry",
                    provider
                ))
            })?;

        // Modify the request to use just the model name (without provider prefix)
        let modified_request = EmbeddingRequest {
            model: model.to_string(),
            input: request.input,
            encoding_format: request.encoding_format,
            dimensions: request.dimensions,
            user: request.user,
        };

        // Execute the embedding
        let response = match provider_instance.embed(modified_request).await {
            Ok(resp) => resp,
            Err(e) => {
                self.record_retry_hint(provider, model, &e);
                let router_error = RouterError::classify(&e, provider, model);
                if matches!(router_error, RouterError::Unreachable { .. }) {
                    self.report_provider_failure(provider, &e.to_string());
                }
                return Err(e);
            }
        };

        // Embeddings only consume input tokens
        let total_tokens = response.usage.prompt_tokens as u64;
        let pricing = self.request_pricing(provider, model).await;
        let cost = calculate_cost(total_tokens, 0, None, &pricing);

        // Record usage for rate limiting (embeddings have simpler usage)
        let usage = UsageInfo {
            input_tokens: total_tokens,
            output_tokens: 0, // Embeddings don't have output tokens
            cost_usd: cost,
        };

        // Record usage for rate limiting
        if let Err(e) = self
            .rate_limiter
            .record_api_key_usage(client_id, &usage)
            .await
        {
            warn!("Failed to record usage for client '{}': {}", client_id, e);
        }

        // Record free tier usage
        let free_tier = self.free_tier_usage(provider, model);
        self.free_tier_manager.record_usage(
            provider,
            &free_tier.kind,
            free_tier.model_is_free,
            total_tokens,
            cost,
        );

        Ok(response)
    }

    /// Embed with auto-routing (localrouter/auto virtual model)
    /// Tries models in prioritized order with intelligent fallback
    async fn embed_with_auto_routing(
        &self,
        client_id: &str,
        strategy: &lr_config::Strategy,
        request: EmbeddingRequest,
    ) -> AppResult<EmbeddingResponse> {
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

        // Decision policies apply to chat; embeddings use the priority list.
        // We just use the prioritized_models list directly
        let selected_models = &auto_config.prioritized_models;

        info!(
            "Auto-routing embeddings for client '{}' with {} prioritized models",
            client_id,
            selected_models.len()
        );

        let mut last_error = None;

        for (idx, (provider, model)) in selected_models.iter().enumerate() {
            if !strategy.is_model_allowed(provider, model)
                || self.provider_registry.is_model_disabled(provider, model)
            {
                continue;
            }
            debug!(
                "Auto-routing embeddings attempt {}/{}: {}/{}",
                idx + 1,
                selected_models.len(),
                provider,
                model
            );

            // Check endpoint compatibility (skip models that don't support embeddings)
            if self.should_skip_for_endpoint(provider, model, lr_providers::EndpointType::Embedding)
            {
                continue;
            }

            // Check backoff state (skip providers with recent 429/402)
            if let Some(backoff) = self.free_tier_manager.is_in_backoff(provider, model) {
                debug!("Skipping {}/{}: {}", provider, model, backoff.reason);
                last_error = Some(RouterError::RateLimited {
                    provider: provider.clone(),
                    model: model.clone(),
                    retry_after_secs: backoff.retry_after_secs,
                });
                continue;
            }

            // Check free tier constraints (when free_tier_only is enabled)
            if strategy.free_tier_only {
                let status = self.classify_for_free_tier(provider, model).await;
                match status {
                    free_tier::ModelFreeStatus::AlwaysFree
                    | free_tier::ModelFreeStatus::FreeWithinLimits
                    | free_tier::ModelFreeStatus::FreeModel => { /* allow */ }
                    free_tier::ModelFreeStatus::NotFree => {
                        debug!(
                            "Skipping {}/{} in free-tier-only mode (embeddings)",
                            provider, model
                        );
                        continue;
                    }
                }
            }

            // Check strategy rate limits before trying this model
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
                .execute_embedding_request(client_id, provider, model, request.clone())
                .await
            {
                Ok(response) => {
                    info!(
                        "Auto-routing embeddings succeeded with {}/{}",
                        provider, model
                    );
                    return Ok(response);
                }
                Err(e) => {
                    self.record_retry_hint(provider, model, &e);
                    let router_error = RouterError::classify(&e, provider, model);
                    warn!(
                        "Auto-routing embeddings attempt failed: {}",
                        router_error.to_log_string()
                    );

                    // Cache endpoint incompatibility for future requests
                    if matches!(router_error, RouterError::EndpointNotSupported { .. }) {
                        self.endpoint_cache.record_unsupported(
                            provider,
                            model,
                            lr_providers::EndpointType::Embedding,
                        );
                    }

                    last_error = Some(router_error.clone());

                    // Continue to next model on retryable errors
                    if !router_error.should_retry() {
                        error!(
                            "Non-retryable error encountered: {}",
                            router_error.to_log_string()
                        );
                        return Err(e);
                    }
                }
            }
        }

        // All models failed
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
            "All auto-routing embedding models failed. Last error: {}",
            last_error
                .map(|e| e.to_log_string())
                .unwrap_or_else(|| "Unknown".to_string())
        )))
    }

    /// Route an embedding request to the appropriate provider
    ///
    /// This method:
    /// 1. Validates the client exists and is enabled
    /// 2. Gets the client's routing strategy
    /// 3. Routes based on requested model (auto vs specific)
    /// 4. Executes the request with appropriate fallback behavior
    ///
    /// Returns 403 (via AppError::Unauthorized) if client is invalid or disabled
    pub async fn embed(
        &self,
        client_id: &str,
        request: EmbeddingRequest,
    ) -> AppResult<EmbeddingResponse> {
        debug!(
            "Routing embedding request for client '{}', model '{}'",
            client_id, request.model
        );

        // Special handling for internal test token (bypasses all routing config)
        if client_id == "internal-test" || client_id == "memory-service" {
            debug!("Internal test token detected - bypassing routing config for embeddings (test mode only)");
            let (provider, model) = Self::parse_model_string(&request.model);
            if provider.is_empty() {
                return Err(AppError::Router(
                    "Internal test requires provider/model format".into(),
                ));
            }
            return self
                .execute_embedding_request(client_id, &provider, &model, request)
                .await;
        }

        // 1. Validate client and get strategy
        let (_client, strategy) = self.validate_client_and_strategy(client_id)?;

        // 2. Check client-level rate limits
        self.check_client_rate_limits(client_id).await?;

        // 3. Route based on requested model
        if request.model == "localrouter/auto" {
            debug!("Using auto-routing for embeddings client '{}'", client_id);
            return self
                .embed_with_auto_routing(client_id, &strategy, request)
                .await;
        }

        // 4. For specific model requests, check strategy rate limits
        self.check_strategy_rate_limits(&strategy, "", "")?;

        // 5. Parse and validate model
        let (provider, model) = Self::parse_model_string(&request.model);

        let (final_provider, final_model) = if provider.is_empty() {
            self.find_provider_for_model(&model, &strategy).await?
        } else {
            // Provider specified - validate it's allowed
            if !strategy.is_model_allowed(&provider, &model) {
                return Err(AppError::Router(format!(
                    "Model '{}/{}' is not allowed by this strategy",
                    provider, model
                )));
            }
            (provider, model)
        };

        // 6. Check free tier constraints for specific model requests
        if strategy.free_tier_only {
            let exhausted = vec![(final_provider.clone(), final_model.clone())];
            // Check backoff first
            if let Some(backoff) = self
                .free_tier_manager
                .is_in_backoff(&final_provider, &final_model)
            {
                debug!(
                    "Specific embedding model {}/{} is in backoff: {}",
                    final_provider, final_model, backoff.reason
                );
                return Err(Self::free_tier_exhausted_error(
                    &strategy,
                    backoff.retry_after_secs,
                    exhausted,
                ));
            }

            let status = self
                .classify_for_free_tier(&final_provider, &final_model)
                .await;
            if matches!(status, free_tier::ModelFreeStatus::NotFree) {
                debug!(
                    "Embedding model {}/{} is not free, rejecting in free-tier-only mode",
                    final_provider, final_model
                );
                return Err(Self::free_tier_exhausted_error(&strategy, 0, exhausted));
            }
        }

        // 7. Execute the request
        debug!(
            "Executing embedding request for client '{}' on {}/{}",
            client_id, final_provider, final_model
        );

        self.execute_embedding_request(client_id, &final_provider, &final_model, request)
            .await
    }

    // ==================== Audio Routing ====================

    /// Execute a transcription request on a specific provider
    async fn execute_transcription_request(
        &self,
        client_id: &str,
        provider: &str,
        model: &str,
        request: AudioTranscriptionRequest,
    ) -> AppResult<AudioTranscriptionResponse> {
        self.require_enabled_model(provider, model)?;
        self.check_provider_privacy(client_id, provider, false)?;
        let provider_instance = self
            .provider_registry
            .get_provider(provider)
            .ok_or_else(|| {
                AppError::Router(format!(
                    "Provider '{}' not found or disabled in registry",
                    provider
                ))
            })?;

        // Estimate tokens from audio file size (rough: 1 sec audio ~ 25 tokens, ~16KB/sec for common formats)
        let estimated_tokens = (request.file.len() as u64 / 16_000).max(1) * 25;

        let modified_request = AudioTranscriptionRequest {
            model: model.to_string(),
            ..request
        };

        let response = match provider_instance.transcribe(modified_request).await {
            Ok(resp) => resp,
            Err(e) => {
                self.record_retry_hint(provider, model, &e);
                let router_error = RouterError::classify(&e, provider, model);
                if matches!(router_error, RouterError::Unreachable { .. }) {
                    self.report_provider_failure(provider, &e.to_string());
                }
                return Err(e);
            }
        };

        // Record estimated usage for rate limiting
        let usage = UsageInfo {
            input_tokens: estimated_tokens,
            output_tokens: 0,
            cost_usd: 0.0,
        };
        if let Err(e) = self
            .rate_limiter
            .record_api_key_usage(client_id, &usage)
            .await
        {
            warn!("Failed to record usage for client '{}': {}", client_id, e);
        }

        Ok(response)
    }

    /// Route a transcription request to the appropriate provider
    pub async fn transcribe(
        &self,
        client_id: &str,
        request: AudioTranscriptionRequest,
    ) -> AppResult<AudioTranscriptionResponse> {
        debug!(
            "Routing transcription request for client '{}', model '{}'",
            client_id, request.model
        );

        if client_id == "internal-test" || client_id == "memory-service" {
            let (provider, model) = Self::parse_model_string(&request.model);
            if provider.is_empty() {
                return Err(AppError::Router(
                    "Internal test requires provider/model format".into(),
                ));
            }
            return self
                .execute_transcription_request(client_id, &provider, &model, request)
                .await;
        }

        let (_client, strategy) = self.validate_client_and_strategy(client_id)?;
        self.check_client_rate_limits(client_id).await?;

        if request.model == "localrouter/auto" {
            debug!(
                "Using auto-routing for transcription client '{}'",
                client_id
            );
            return self
                .transcribe_with_auto_routing(client_id, &strategy, request)
                .await;
        }

        self.check_strategy_rate_limits(&strategy, "", "")?;

        let (provider, model) = Self::parse_model_string(&request.model);
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

        self.execute_transcription_request(client_id, &final_provider, &final_model, request)
            .await
    }

    /// Execute a translation request on a specific provider
    async fn execute_translation_request(
        &self,
        client_id: &str,
        provider: &str,
        model: &str,
        request: AudioTranslationRequest,
    ) -> AppResult<AudioTranslationResponse> {
        self.require_enabled_model(provider, model)?;
        self.check_provider_privacy(client_id, provider, false)?;
        let provider_instance = self
            .provider_registry
            .get_provider(provider)
            .ok_or_else(|| {
                AppError::Router(format!(
                    "Provider '{}' not found or disabled in registry",
                    provider
                ))
            })?;

        // Estimate tokens from audio file size (rough: 1 sec audio ~ 25 tokens, ~16KB/sec)
        let estimated_tokens = (request.file.len() as u64 / 16_000).max(1) * 25;

        let modified_request = AudioTranslationRequest {
            model: model.to_string(),
            ..request
        };

        let response = match provider_instance.translate_audio(modified_request).await {
            Ok(resp) => resp,
            Err(e) => {
                self.record_retry_hint(provider, model, &e);
                let router_error = RouterError::classify(&e, provider, model);
                if matches!(router_error, RouterError::Unreachable { .. }) {
                    self.report_provider_failure(provider, &e.to_string());
                }
                return Err(e);
            }
        };

        let usage = UsageInfo {
            input_tokens: estimated_tokens,
            output_tokens: 0,
            cost_usd: 0.0,
        };
        if let Err(e) = self
            .rate_limiter
            .record_api_key_usage(client_id, &usage)
            .await
        {
            warn!("Failed to record usage for client '{}': {}", client_id, e);
        }

        Ok(response)
    }

    /// Route an audio translation request to the appropriate provider
    pub async fn translate_audio(
        &self,
        client_id: &str,
        request: AudioTranslationRequest,
    ) -> AppResult<AudioTranslationResponse> {
        debug!(
            "Routing audio translation request for client '{}', model '{}'",
            client_id, request.model
        );

        if client_id == "internal-test" || client_id == "memory-service" {
            let (provider, model) = Self::parse_model_string(&request.model);
            if provider.is_empty() {
                return Err(AppError::Router(
                    "Internal test requires provider/model format".into(),
                ));
            }
            return self
                .execute_translation_request(client_id, &provider, &model, request)
                .await;
        }

        let (_client, strategy) = self.validate_client_and_strategy(client_id)?;
        self.check_client_rate_limits(client_id).await?;

        if request.model == "localrouter/auto" {
            debug!(
                "Using auto-routing for audio translation client '{}'",
                client_id
            );
            return self
                .translate_with_auto_routing(client_id, &strategy, request)
                .await;
        }

        self.check_strategy_rate_limits(&strategy, "", "")?;

        let (provider, model) = Self::parse_model_string(&request.model);
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

        self.execute_translation_request(client_id, &final_provider, &final_model, request)
            .await
    }

    /// Execute a speech request on a specific provider
    async fn execute_speech_request(
        &self,
        client_id: &str,
        provider: &str,
        model: &str,
        request: SpeechRequest,
    ) -> AppResult<SpeechResponse> {
        self.require_enabled_model(provider, model)?;
        self.check_provider_privacy(client_id, provider, false)?;
        let provider_instance = self
            .provider_registry
            .get_provider(provider)
            .ok_or_else(|| {
                AppError::Router(format!(
                    "Provider '{}' not found or disabled in registry",
                    provider
                ))
            })?;

        // Estimate tokens from text length (rough: 4 chars per token)
        let estimated_tokens = (request.input.len() as u64 / 4).max(1);

        let modified_request = SpeechRequest {
            model: model.to_string(),
            input: request.input,
            voice: request.voice,
            response_format: request.response_format,
            speed: request.speed,
        };

        let response = match provider_instance.speech(modified_request).await {
            Ok(resp) => resp,
            Err(e) => {
                self.record_retry_hint(provider, model, &e);
                let router_error = RouterError::classify(&e, provider, model);
                if matches!(router_error, RouterError::Unreachable { .. }) {
                    self.report_provider_failure(provider, &e.to_string());
                }
                return Err(e);
            }
        };

        let usage = UsageInfo {
            input_tokens: estimated_tokens,
            output_tokens: 0,
            cost_usd: 0.0,
        };
        if let Err(e) = self
            .rate_limiter
            .record_api_key_usage(client_id, &usage)
            .await
        {
            warn!("Failed to record usage for client '{}': {}", client_id, e);
        }

        Ok(response)
    }

    /// Route a speech (TTS) request to the appropriate provider
    pub async fn speech(
        &self,
        client_id: &str,
        request: SpeechRequest,
    ) -> AppResult<SpeechResponse> {
        debug!(
            "Routing speech request for client '{}', model '{}'",
            client_id, request.model
        );

        if client_id == "internal-test" || client_id == "memory-service" {
            let (provider, model) = Self::parse_model_string(&request.model);
            if provider.is_empty() {
                return Err(AppError::Router(
                    "Internal test requires provider/model format".into(),
                ));
            }
            return self
                .execute_speech_request(client_id, &provider, &model, request)
                .await;
        }

        let (_client, strategy) = self.validate_client_and_strategy(client_id)?;
        self.check_client_rate_limits(client_id).await?;

        if request.model == "localrouter/auto" {
            debug!("Using auto-routing for speech client '{}'", client_id);
            return self
                .speech_with_auto_routing(client_id, &strategy, request)
                .await;
        }

        self.check_strategy_rate_limits(&strategy, "", "")?;

        let (provider, model) = Self::parse_model_string(&request.model);
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

        self.execute_speech_request(client_id, &final_provider, &final_model, request)
            .await
    }

    /// Auto-routing for audio transcription requests.
    /// Tries prioritized models in order with fallback (decision policies apply to chat).
    async fn transcribe_with_auto_routing(
        &self,
        client_id: &str,
        strategy: &lr_config::Strategy,
        mut request: AudioTranscriptionRequest,
    ) -> AppResult<AudioTranscriptionResponse> {
        let auto_config = strategy.auto_config.as_ref().ok_or_else(|| {
            AppError::Router("localrouter/auto not configured for this strategy".into())
        })?;
        if !auto_config.permission.is_enabled() {
            return Err(AppError::Router("localrouter/auto is disabled".into()));
        }
        if auto_config.prioritized_models.is_empty() {
            return Err(AppError::Router(
                "No prioritized models configured for auto-routing".into(),
            ));
        }

        let selected_models = &auto_config.prioritized_models;
        let mut last_error = None;

        for (provider, model) in selected_models {
            if self.provider_registry.is_model_disabled(provider, model) {
                continue;
            }
            if let Some(backoff) = self.free_tier_manager.is_in_backoff(provider, model) {
                debug!("Skipping {}/{}: {}", provider, model, backoff.reason);
                continue;
            }
            if let Err(e) = self.check_strategy_rate_limits(strategy, provider, model) {
                warn!("Rate limit for {}/{}: {}", provider, model, e);
                continue;
            }

            request.model = format!("{}/{}", provider, model);
            match self
                .execute_transcription_request(client_id, provider, model, request.clone())
                .await
            {
                Ok(response) => return Ok(response),
                Err(e) => {
                    warn!(
                        "Auto-routing transcription {}/{} failed: {}",
                        provider, model, e
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| AppError::Router("All auto-routing audio models failed".into())))
    }

    /// Auto-routing for audio translation requests.
    async fn translate_with_auto_routing(
        &self,
        client_id: &str,
        strategy: &lr_config::Strategy,
        mut request: AudioTranslationRequest,
    ) -> AppResult<AudioTranslationResponse> {
        let auto_config = strategy.auto_config.as_ref().ok_or_else(|| {
            AppError::Router("localrouter/auto not configured for this strategy".into())
        })?;
        if !auto_config.permission.is_enabled() {
            return Err(AppError::Router("localrouter/auto is disabled".into()));
        }
        if auto_config.prioritized_models.is_empty() {
            return Err(AppError::Router(
                "No prioritized models configured for auto-routing".into(),
            ));
        }

        let selected_models = &auto_config.prioritized_models;
        let mut last_error = None;

        for (provider, model) in selected_models {
            if self.provider_registry.is_model_disabled(provider, model) {
                continue;
            }
            if let Some(backoff) = self.free_tier_manager.is_in_backoff(provider, model) {
                debug!("Skipping {}/{}: {}", provider, model, backoff.reason);
                continue;
            }
            if let Err(e) = self.check_strategy_rate_limits(strategy, provider, model) {
                warn!("Rate limit for {}/{}: {}", provider, model, e);
                continue;
            }

            request.model = format!("{}/{}", provider, model);
            match self
                .execute_translation_request(client_id, provider, model, request.clone())
                .await
            {
                Ok(response) => return Ok(response),
                Err(e) => {
                    warn!(
                        "Auto-routing translation {}/{} failed: {}",
                        provider, model, e
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| AppError::Router("All auto-routing audio models failed".into())))
    }

    /// Auto-routing for speech (TTS) requests.
    async fn speech_with_auto_routing(
        &self,
        client_id: &str,
        strategy: &lr_config::Strategy,
        mut request: SpeechRequest,
    ) -> AppResult<SpeechResponse> {
        let auto_config = strategy.auto_config.as_ref().ok_or_else(|| {
            AppError::Router("localrouter/auto not configured for this strategy".into())
        })?;
        if !auto_config.permission.is_enabled() {
            return Err(AppError::Router("localrouter/auto is disabled".into()));
        }
        if auto_config.prioritized_models.is_empty() {
            return Err(AppError::Router(
                "No prioritized models configured for auto-routing".into(),
            ));
        }

        let selected_models = &auto_config.prioritized_models;
        let mut last_error = None;

        for (provider, model) in selected_models {
            if self.provider_registry.is_model_disabled(provider, model) {
                continue;
            }
            if let Some(backoff) = self.free_tier_manager.is_in_backoff(provider, model) {
                debug!("Skipping {}/{}: {}", provider, model, backoff.reason);
                continue;
            }
            if let Err(e) = self.check_strategy_rate_limits(strategy, provider, model) {
                warn!("Rate limit for {}/{}: {}", provider, model, e);
                continue;
            }

            request.model = format!("{}/{}", provider, model);
            match self
                .execute_speech_request(client_id, provider, model, request.clone())
                .await
            {
                Ok(response) => return Ok(response),
                Err(e) => {
                    warn!("Auto-routing speech {}/{} failed: {}", provider, model, e);
                    last_error = Some(e);
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| AppError::Router("All auto-routing speech models failed".into())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_config::AppConfig;

    fn text_chunk(content: &str, usage: Option<lr_providers::TokenUsage>) -> CompletionChunk {
        CompletionChunk {
            id: "c".into(),
            object: "chat.completion.chunk".into(),
            created: 0,
            model: "upstream-name".into(),
            choices: vec![lr_providers::ChunkChoice {
                index: 0,
                delta: lr_providers::ChunkDelta {
                    annotations: None,
                    role: None,
                    content: Some(content.into()),
                    tool_calls: None,
                    reasoning_content: None,
                },
                finish_reason: None,
            }],
            extensions: None,
            usage,
            provider: None,
        }
    }

    #[test]
    fn stream_usage_totals_prefer_the_upstream_usage() {
        let st = StreamUsageState {
            reported_cost: None,
            output_chars: 400,
            upstream: Some(lr_providers::TokenUsage {
                prompt_tokens: 900,
                completion_tokens: 30,
                total_tokens: 930,
                prompt_tokens_details: None,
                completion_tokens_details: Some(lr_providers::CompletionTokensDetails {
                    reasoning_tokens: Some(12),
                    thinking_tokens: None,
                    audio_tokens: None,
                }),
            }),
        };
        let totals = stream_usage_totals(&st, 5);
        assert_eq!((totals.prompt_tokens, totals.completion_tokens), (900, 30));
        assert_eq!(
            totals
                .completion_tokens_details
                .and_then(|d| d.reasoning_tokens),
            Some(12)
        );
    }

    #[test]
    fn stream_usage_totals_estimate_without_upstream_usage() {
        let st = StreamUsageState {
            reported_cost: None,
            output_chars: 401,
            upstream: None,
        };
        // Whole-prompt estimate, output chars / 4 rounded up
        let totals = stream_usage_totals(&st, 777);
        assert_eq!((totals.prompt_tokens, totals.completion_tokens), (777, 101));
    }

    #[tokio::test]
    async fn usage_wrapper_stamps_the_serving_provider_and_model() {
        let upstream: Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>> =
            Box::pin(futures::stream::iter(vec![
                Ok(text_chunk("hello", None)),
                Ok(text_chunk("", None)),
            ]));
        let wrapped = wrap_stream_with_usage_tracking(
            upstream,
            "client".into(),
            "chatgpt/gpt-5.5".into(),
            10,
            Arc::new(RateLimiterManager::new(None)),
            Arc::new(FreeTierManager::new(None)),
            FreeTierUsage {
                kind: lr_config::FreeTierKind::None,
                model_is_free: true,
            },
            lr_providers::PricingInfo::free(),
            false,
        )
        .await;
        let chunks: Vec<_> = wrapped.collect().await;
        assert_eq!(chunks.len(), 2);
        for chunk in chunks {
            let chunk = chunk.expect("chunk");
            assert_eq!(chunk.model, "chatgpt/gpt-5.5");
            assert_eq!(chunk.provider.as_deref(), Some("chatgpt"));
        }
    }

    #[test]
    fn test_summarize_attempt_outcomes_counts_and_sorts() {
        let attempts = vec![
            serde_json::json!({"provider": "Ollama", "model": "m1", "outcome": "no_tool_support"}),
            serde_json::json!({"provider": "Ollama", "model": "m2", "outcome": "no_tool_support"}),
            serde_json::json!({"provider": "Ollama", "model": "m3", "outcome": "endpoint_not_supported"}),
        ];
        // BTreeMap ordering → alphabetical by outcome.
        assert_eq!(
            summarize_attempt_outcomes(&attempts),
            "endpoint_not_supported: 1, no_tool_support: 2"
        );
    }

    #[test]
    fn test_summarize_attempt_outcomes_empty() {
        assert_eq!(summarize_attempt_outcomes(&[]), "");
    }

    #[test]
    fn test_summarize_attempt_outcomes_ignores_entries_without_outcome() {
        let attempts = vec![
            serde_json::json!({"provider": "p", "model": "m"}),
            serde_json::json!({"outcome": "no_tool_support"}),
        ];
        assert_eq!(summarize_attempt_outcomes(&attempts), "no_tool_support: 1");
    }

    #[test]
    fn test_routing_metadata_records_all_skips_alongside_success() {
        // Regression: the streaming auto-router used to silently skip
        // backoff'd / not_free / cost_backoff / rate_limited candidates
        // without pushing to `attempts`, making the routing UI claim
        // "1 attempt across 2 candidate models" when in reality two
        // candidates were evaluated. Both code paths (non-streaming +
        // streaming) now push skip outcomes — `build_routing_metadata`
        // reflects every entry in total_attempts and the attempts array.
        let candidates = vec![
            "Minimax Starter/MiniMax-M3".to_string(),
            "MiniMax Max/MiniMax-M3".to_string(),
        ];
        let attempts = vec![
            serde_json::json!({
                "provider": "Minimax Starter",
                "model": "MiniMax-M3",
                "outcome": "backoff",
                "error": "rate limited (available in 60s)",
            }),
            serde_json::json!({
                "provider": "MiniMax Max",
                "model": "MiniMax-M3",
                "outcome": "success",
                "duration_ms": 4165,
            }),
        ];

        let meta = build_routing_metadata(None, &candidates, attempts, Some(1));

        assert_eq!(meta["candidate_models"].as_array().unwrap().len(), 2);
        assert_eq!(meta["total_attempts"].as_u64().unwrap(), 2);
        assert_eq!(meta["attempts"].as_array().unwrap().len(), 2);
        assert_eq!(meta["successful_attempt"].as_u64().unwrap(), 1);
        assert_eq!(
            meta["attempts"][0]["provider"].as_str().unwrap(),
            "Minimax Starter"
        );
        assert_eq!(meta["attempts"][0]["outcome"].as_str().unwrap(), "backoff");
        assert_eq!(
            meta["attempts"][1]["provider"].as_str().unwrap(),
            "MiniMax Max"
        );
        assert_eq!(meta["attempts"][1]["outcome"].as_str().unwrap(), "success");
    }

    #[test]
    fn test_classify_prefers_typed_context_length_error() {
        // Structured provider error (from classify_openai_error) carries the
        // real max — verify the router preserves it instead of zeroing it out.
        let err = AppError::ContextLengthExceeded {
            max: Some(8192),
            requested: Some(9100),
        };
        match RouterError::classify(&err, "OpenAI", "gpt-4") {
            RouterError::ContextLengthExceeded {
                max_tokens,
                requested_tokens,
                ..
            } => {
                assert_eq!(max_tokens, Some(8192));
                assert_eq!(requested_tokens, Some(9100));
            }
            other => panic!("expected ContextLengthExceeded, got {:?}", other),
        }
    }

    #[test]
    fn test_classify_model_not_found_maps_to_endpoint_not_supported() {
        let err = AppError::ModelNotFound {
            model: "gpt-99".into(),
        };
        assert!(matches!(
            RouterError::classify(&err, "OpenAI", "gpt-99"),
            RouterError::EndpointNotSupported { .. }
        ));
    }

    #[test]
    fn test_classify_token_auth_error_not_context_length() {
        // Regression: the old classifier caught any AppError::Provider
        // containing the word "token" and mis-labeled it as context-length.
        // With the tightened substring match this should fall through to
        // `Other` (not retried as context-length fallback).
        let err = AppError::Provider("Bearer token invalid".into());
        assert!(matches!(
            RouterError::classify(&err, "OpenAI", "gpt-4"),
            RouterError::Other { .. }
        ));
    }

    #[test]
    fn test_context_length_log_string_omits_unknown_max() {
        let e = RouterError::ContextLengthExceeded {
            provider: "OpenAI".into(),
            model: "gpt-4".into(),
            max_tokens: None,
            requested_tokens: None,
        };
        let s = e.to_log_string();
        assert!(
            !s.contains("max:"),
            "should not show max when unknown: {}",
            s
        );
        assert!(s.contains("OpenAI/gpt-4"));
    }

    #[test]
    fn test_context_length_log_string_includes_known_max() {
        let e = RouterError::ContextLengthExceeded {
            provider: "OpenAI".into(),
            model: "gpt-4".into(),
            max_tokens: Some(8192),
            requested_tokens: Some(9100),
        };
        let s = e.to_log_string();
        assert!(s.contains("(max: 8192)"));
        assert!(s.contains("(requested: 9100)"));
    }

    #[tokio::test]
    async fn test_router_creation() {
        let config_manager = Arc::new(ConfigManager::new(
            AppConfig::default(),
            std::path::PathBuf::from("/tmp/test.yaml"),
        ));

        let provider_registry = Arc::new(ProviderRegistry::new());
        let rate_limiter = Arc::new(RateLimiterManager::new(None));

        // Create test metrics collector
        let metrics_db_path =
            std::env::temp_dir().join(format!("test_metrics_{}.db", uuid::Uuid::new_v4()));
        let metrics_db =
            Arc::new(lr_monitoring::storage::MetricsDatabase::new(metrics_db_path).unwrap());
        let metrics_collector = Arc::new(lr_monitoring::metrics::MetricsCollector::new(metrics_db));

        let router = Router::new_without_free_tier(
            config_manager,
            provider_registry,
            rate_limiter,
            metrics_collector,
        );

        // Just verify it compiles and constructs
        assert!(Arc::strong_count(&router.config_manager) >= 1);
    }

    #[tokio::test]
    async fn test_router_unauthorized_api_key() {
        let config_manager = Arc::new(ConfigManager::new(
            AppConfig::default(),
            std::path::PathBuf::from("/tmp/test.yaml"),
        ));

        let provider_registry = Arc::new(ProviderRegistry::new());
        let rate_limiter = Arc::new(RateLimiterManager::new(None));

        // Create test metrics collector
        let metrics_db_path =
            std::env::temp_dir().join(format!("test_metrics_{}.db", uuid::Uuid::new_v4()));
        let metrics_db =
            Arc::new(lr_monitoring::storage::MetricsDatabase::new(metrics_db_path).unwrap());
        let metrics_collector = Arc::new(lr_monitoring::metrics::MetricsCollector::new(metrics_db));

        let router = Router::new_without_free_tier(
            config_manager,
            provider_registry,
            rate_limiter,
            metrics_collector,
        );

        let request = CompletionRequest {
            model: "test-model".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            extensions: None,
            logprobs: None,
            top_logprobs: None,
            response_format: None,
            tool_choice: None,
            tools: None,
            n: None,
            logit_bias: None,
            parallel_tool_calls: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
            pre_computed_routing: None,
            native_messages: None,
            server_tools: None,
        };

        let result = router.complete("invalid-key-id", request).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::Unauthorized));
    }

    // ============================================================================
    // Routing Strategy Tests - REMOVED (tests used obsolete ApiKeyConfig)
    // ============================================================================
    // Tests were removed during migration to unified Client architecture.
    // New tests should be written using the Client structure.

    // ============================================================================
    // TODO: Provider Retry Integration Tests
    // ============================================================================
    //
    // Future work: Add integration tests that test actual provider retry logic with mock
    // provider implementations. These tests should cover:
    //
    // 1. **Provider Failures with Retry**:
    //    - First provider fails with retryable error (Provider, RateLimitExceeded, etc.)
    //    - Router retries with next provider in prioritized list
    //    - Verify call counts and retry order
    //
    // 2. **Non-Retryable Errors**:
    //    - Provider fails with Validation error
    //    - Router stops immediately without retrying
    //
    // 3. **All Providers Fail**:
    //    - All providers in prioritized list fail
    //    - Router returns last error
    //
    // 4. **Health Check Failures**:
    //    - Provider health check fails
    //    - Router logs warning but continues with request
    //    - Completion still works if provider is functional
    //
    // 5. **Success on First Try**:
    //    - First provider succeeds
    //    - No retries attempted
    //    - Other providers never called
    //
    // **Implementation Requirements**:
    // - Create MockProviderFactory that implements ProviderFactory trait
    // - MockProvider with configurable failure modes (count, error type)
    // - Track call counts to verify retry logic
    // - Test with ProviderRegistry.create_provider() for proper architecture
    //
    // **Current Status**: Unit tests in routing_strategy_tests provide comprehensive
    // coverage of configuration logic and is_model_allowed() behavior. Integration
    // tests require ProviderRegistry architecture support.
    //
    // ============================================================================

    // Placeholder for future integration tests - see TODO comment above

    // ============================================================================
    // Test normalize_model_id
    // ============================================================================

    #[test]
    fn test_normalize_model_id_plain() {
        assert_eq!(Router::normalize_model_id("llama2"), "llama2");
        assert_eq!(Router::normalize_model_id("gpt-4"), "gpt-4");
        assert_eq!(Router::normalize_model_id("claude-3-opus"), "claude-3-opus");
    }

    #[test]
    fn test_normalize_model_id_with_tag() {
        assert_eq!(Router::normalize_model_id("llama2:latest"), "llama2");
        assert_eq!(Router::normalize_model_id("llama2:7b"), "llama2");
        assert_eq!(Router::normalize_model_id("mistral:instruct"), "mistral");
    }

    #[test]
    fn test_normalize_model_id_with_provider_prefix() {
        assert_eq!(Router::normalize_model_id("openai/gpt-4"), "gpt-4");
        assert_eq!(Router::normalize_model_id("anthropic/claude-3"), "claude-3");
        assert_eq!(Router::normalize_model_id("ollama/llama2"), "llama2");
    }

    #[test]
    fn test_normalize_model_id_with_both() {
        assert_eq!(Router::normalize_model_id("openai/gpt-4:turbo"), "gpt-4");
        assert_eq!(Router::normalize_model_id("ollama/llama2:latest"), "llama2");
        assert_eq!(Router::normalize_model_id("mistral/mistral:7b"), "mistral");
    }

    #[test]
    fn test_normalize_model_id_case_insensitive() {
        assert_eq!(Router::normalize_model_id("GPT-4"), "gpt-4");
        assert_eq!(Router::normalize_model_id("LLaMA2"), "llama2");
        assert_eq!(Router::normalize_model_id("Claude-3-Opus"), "claude-3-opus");
    }

    #[test]
    fn test_normalize_model_id_all_together() {
        // Case: "OpenAI/GPT-4:Turbo" -> "gpt-4"
        assert_eq!(Router::normalize_model_id("OpenAI/GPT-4:Turbo"), "gpt-4");
        // Case: "OLLAMA/LLaMA2:Latest" -> "llama2"
        assert_eq!(Router::normalize_model_id("OLLAMA/LLaMA2:Latest"), "llama2");
    }

    // ============================================================================
    // Test classify_match — exact id match preferred over normalized
    // ============================================================================

    #[test]
    fn test_classify_match_exact() {
        assert_eq!(
            Router::classify_match("gpt-5.5", &["gpt-5.5", "gpt-5.5-mini"]),
            Some(MatchKind::Exact)
        );
    }

    #[test]
    fn test_classify_match_exact_case_insensitive() {
        assert_eq!(
            Router::classify_match("GPT-5.5", &["gpt-5.5"]),
            Some(MatchKind::Exact)
        );
    }

    #[test]
    fn test_classify_match_normalized_only() {
        // OpenRouter exposes models as `vendor/model`; bare request matches only via normalization.
        assert_eq!(
            Router::classify_match("gpt-5.5", &["openai/gpt-5.5", "anthropic/claude-3"]),
            Some(MatchKind::Normalized)
        );
    }

    #[test]
    fn test_classify_match_no_match() {
        assert_eq!(
            Router::classify_match("gpt-5.5", &["gpt-4", "claude-3"]),
            None
        );
    }

    #[test]
    fn test_classify_match_exact_wins_over_normalized_in_same_list() {
        // If both an exact and a normalized-only candidate live in the same
        // provider's list, exact still wins.
        assert_eq!(
            Router::classify_match("gpt-5.5", &["openai/gpt-5.5", "gpt-5.5"]),
            Some(MatchKind::Exact)
        );
    }

    #[test]
    fn test_classify_match_normalized_handles_tags() {
        // Ollama-style tags should still match via normalization.
        assert_eq!(
            Router::classify_match("llama2", &["llama2:latest"]),
            Some(MatchKind::Normalized)
        );
    }
}

mod session_routing;
