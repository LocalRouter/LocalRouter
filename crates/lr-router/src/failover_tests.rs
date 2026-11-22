//! Router integration tests against scripted mock providers: failover across
//! a strategy's prioritized models, and rate-limit admission (in-flight
//! reservations) through the same paths.
//!
//! Every provider instance serves one model, `m`, and is registered through
//! a `ProviderFactory` like a real provider, so requests travel the whole
//! router: client and strategy lookup, admission, candidate assessment,
//! dispatch, usage accounting.

use super::*;
use crate::rate_limit::{RateLimitType, RateLimiter};
use async_trait::async_trait;
use lr_config::{AppConfig, RateLimitTimeWindow, StrategyRateLimit};
use lr_providers::factory::{ParameterType, ProviderCategory, ProviderFactory, SetupParameter};
use lr_providers::{
    Capability, ChatMessage, ChatMessageContent, ChunkChoice, ChunkDelta, CompletionChoice,
    HealthStatus, ModelInfo, ModelProvider, PricingInfo, ProviderHealth, TokenUsage,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Tokens every scripted completion reports.
const PROMPT_TOKENS: u32 = 100;
const COMPLETION_TOKENS: u32 = 10;

/// A failure a scripted provider answers with.
#[derive(Clone, Copy)]
enum Fail {
    /// 503 from the upstream: retryable.
    Unavailable,
    /// 429 with a `Retry-After`: retryable, and backs the provider off.
    RateLimited(u64),
    /// The model's context window is too small: retryable.
    ContextLength,
    /// The provider does not know the model: retryable.
    ModelNotFound,
    /// The upstream rejected the request itself (400): not retryable.
    BadRequest,
    /// LocalRouter rejected the request's parameters: not retryable.
    InvalidParams,
}

impl Fail {
    fn error(self) -> AppError {
        match self {
            Fail::Unavailable => AppError::ProviderUnavailable {
                status: 503,
                message: "upstream overloaded".into(),
                retry_after_secs: None,
            },
            Fail::RateLimited(retry_after_secs) => {
                AppError::RateLimitWithRetryAfter { retry_after_secs }
            }
            Fail::ContextLength => AppError::ContextLengthExceeded {
                max: Some(8),
                requested: Some(9),
            },
            Fail::ModelNotFound => AppError::ModelNotFound { model: "m".into() },
            Fail::BadRequest => AppError::ProviderStatus {
                status: 400,
                message: "malformed request".into(),
            },
            Fail::InvalidParams => AppError::InvalidParams("unsupported parameter".into()),
        }
    }
}

#[derive(Clone, Copy)]
enum Behavior {
    /// Answer (or stream two chunks, the last carrying usage).
    Succeed,
    /// Fail the dispatch before anything is produced.
    Fail(Fail),
    /// Streaming only: produce one chunk, then fail mid-stream.
    BreakAfterFirstChunk,
}

/// A hook run inside a provider call, to look at router state mid-request.
type Probe = Box<dyn Fn() + Send + Sync>;

/// One provider instance's script and what it observed.
struct Script {
    name: String,
    behavior: parking_lot::Mutex<Behavior>,
    healthy: bool,
    calls: AtomicUsize,
    /// Dispatches that entered the provider and have not returned.
    entered: AtomicUsize,
    /// When set, a call waits for a permit before answering.
    gate: parking_lot::Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    probe: parking_lot::Mutex<Option<Probe>>,
    /// Call order across every provider of the harness.
    order: Arc<parking_lot::Mutex<Vec<String>>>,
}

impl Script {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn set(&self, behavior: Behavior) {
        *self.behavior.lock() = behavior;
    }

    /// Hold every call inside the provider until the returned gate gets
    /// permits.
    fn hold(&self) -> Arc<tokio::sync::Semaphore> {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *self.gate.lock() = Some(gate.clone());
        gate
    }

    fn probe(&self, probe: impl Fn() + Send + Sync + 'static) {
        *self.probe.lock() = Some(Box::new(probe));
    }

    /// Common start of a call: count it, run the probe, wait at the gate.
    async fn begin(&self) -> Behavior {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.order.lock().push(self.name.clone());
        if let Some(probe) = self.probe.lock().as_ref() {
            probe();
        }
        let gate = self.gate.lock().clone();
        if let Some(gate) = gate {
            self.entered.fetch_add(1, Ordering::SeqCst);
            let permit = gate.acquire().await;
            self.entered.fetch_sub(1, Ordering::SeqCst);
            permit.expect("gate closed").forget();
        }
        *self.behavior.lock()
    }
}

struct MockProvider(Arc<Script>);

fn usage() -> TokenUsage {
    TokenUsage {
        prompt_tokens: PROMPT_TOKENS,
        completion_tokens: COMPLETION_TOKENS,
        total_tokens: PROMPT_TOKENS + COMPLETION_TOKENS,
        prompt_tokens_details: None,
        completion_tokens_details: None,
    }
}

fn message(role: &str, text: &str) -> ChatMessage {
    ChatMessage {
        annotations: None,
        role: role.into(),
        content: ChatMessageContent::Text(text.into()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
        reasoning_content: None,
    }
}

fn chunk(content: &str, usage: Option<TokenUsage>) -> CompletionChunk {
    CompletionChunk {
        id: "chunk".into(),
        object: "chat.completion.chunk".into(),
        created: 0,
        model: "m".into(),
        choices: vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta {
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

#[async_trait]
impl ModelProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }
    async fn health_check(&self) -> ProviderHealth {
        ProviderHealth {
            status: if self.0.healthy {
                HealthStatus::Healthy
            } else {
                HealthStatus::Unhealthy
            },
            latency_ms: Some(1),
            last_checked: chrono::Utc::now(),
            error_message: (!self.0.healthy).then(|| "health probe failed".into()),
        }
    }
    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            id: "m".into(),
            name: "m".into(),
            provider: "mock".into(),
            parameter_count: None,
            context_window: 4096,
            supports_streaming: true,
            capabilities: vec![Capability::Chat],
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
    async fn complete(&self, _request: CompletionRequest) -> AppResult<CompletionResponse> {
        match self.0.begin().await {
            Behavior::Fail(fail) => Err(fail.error()),
            _ => Ok(CompletionResponse {
                id: "response".into(),
                object: "chat.completion".into(),
                created: 0,
                model: "m".into(),
                provider: "mock".into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    message: message("assistant", &format!("answer from {}", self.0.name)),
                    finish_reason: Some("stop".into()),
                    logprobs: None,
                }],
                usage: usage(),
                system_fingerprint: None,
                service_tier: None,
                extensions: None,
                routellm_win_rate: None,
                request_usage_entries: None,
            }),
        }
    }
    async fn stream_complete(
        &self,
        _request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        let name = &self.0.name;
        let items: Vec<AppResult<CompletionChunk>> = match self.0.begin().await {
            Behavior::Fail(fail) => return Err(fail.error()),
            Behavior::Succeed => vec![
                Ok(chunk(&format!("first from {name}"), None)),
                Ok(chunk(&format!("last from {name}"), Some(usage()))),
            ],
            Behavior::BreakAfterFirstChunk => vec![
                Ok(chunk(&format!("first from {name}"), None)),
                Err(Fail::Unavailable.error()),
            ],
        };
        Ok(Box::pin(futures::stream::iter(items)))
    }
}

struct MockFactory {
    type_name: String,
    script: Arc<Script>,
}

impl ProviderFactory for MockFactory {
    fn provider_type(&self) -> &str {
        &self.type_name
    }
    fn display_name(&self) -> &str {
        &self.type_name
    }
    fn category(&self) -> ProviderCategory {
        ProviderCategory::Generic
    }
    fn description(&self) -> &str {
        "scripted mock"
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
        Ok(Arc::new(MockProvider(self.script.clone())))
    }
    fn validate_config(&self, _config: &HashMap<String, String>) -> AppResult<()> {
        Ok(())
    }
}

/// A provider instance to register: its name, script, and health.
struct Instance {
    name: &'static str,
    behavior: Behavior,
    healthy: bool,
}

fn ok(name: &'static str) -> Instance {
    Instance {
        name,
        behavior: Behavior::Succeed,
        healthy: true,
    }
}

fn failing(name: &'static str, fail: Fail) -> Instance {
    Instance {
        name,
        behavior: Behavior::Fail(fail),
        healthy: true,
    }
}

struct Harness {
    router: Arc<Router>,
    client_id: String,
    strategy_id: String,
    scripts: HashMap<&'static str, Arc<Script>>,
    order: Arc<parking_lot::Mutex<Vec<String>>>,
    health: Arc<lr_providers::health_cache::HealthCacheManager>,
}

impl Harness {
    fn script(&self, name: &str) -> &Arc<Script> {
        &self.scripts[name]
    }

    fn order(&self) -> Vec<String> {
        self.order.lock().clone()
    }

    fn limiter(&self) -> &Arc<RateLimiterManager> {
        &self.router.rate_limiter
    }

    /// Limit the harness client itself (the per-client limiter).
    fn limit_client(&self, limits: Vec<RateLimiter>) {
        self.limiter()
            .add_api_key_limiters(self.client_id.clone(), limits);
    }

    async fn recorded(&self, limit_type: RateLimitType) -> f64 {
        self.limiter()
            .get_api_key_usage(&self.client_id, limit_type)
            .await
            .map_or(0.0, |(used, _, _)| used)
    }

    fn in_flight_requests(&self) -> u64 {
        self.limiter().api_key_in_flight(&self.client_id).requests
    }

    async fn complete(
        &self,
        model: &str,
    ) -> AppResult<(CompletionResponse, Option<serde_json::Value>)> {
        self.router.complete(&self.client_id, request(model)).await
    }
}

fn request(model: &str) -> CompletionRequest {
    CompletionRequest::new(model, vec![message("user", "hello there")])
}

const AUTO: &str = "localrouter/auto";

/// A router over the given provider instances with one client whose strategy
/// allows every model and auto-routes over all instances in the given order.
async fn harness(instances: Vec<Instance>) -> Harness {
    harness_with(instances, |_| {}).await
}

async fn harness_with(
    instances: Vec<Instance>,
    mutate: impl FnOnce(&mut lr_config::Strategy),
) -> Harness {
    let order = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let registry = Arc::new(ProviderRegistry::new());
    let mut scripts = HashMap::new();
    let mut prioritized = Vec::new();
    for instance in instances {
        let script = Arc::new(Script {
            name: instance.name.into(),
            behavior: parking_lot::Mutex::new(instance.behavior),
            healthy: instance.healthy,
            calls: AtomicUsize::new(0),
            entered: AtomicUsize::new(0),
            gate: parking_lot::Mutex::new(None),
            probe: parking_lot::Mutex::new(None),
            order: order.clone(),
        });
        let type_name = format!("mock-{}", instance.name);
        registry.register_factory(Arc::new(MockFactory {
            type_name: type_name.clone(),
            script: script.clone(),
        }));
        registry
            .create_provider(instance.name.to_string(), type_name, HashMap::new())
            .await
            .unwrap();
        prioritized.push((instance.name.to_string(), "m".to_string()));
        scripts.insert(instance.name, script);
    }

    let mut strategy = lr_config::Strategy::new("failover".into());
    strategy.auto_config.as_mut().unwrap().prioritized_models = prioritized;
    mutate(&mut strategy);
    let client = lr_config::Client::new_with_strategy("client".into(), strategy.id.clone());
    let client_id = client.id.clone();
    let strategy_id = strategy.id.clone();
    let mut config = AppConfig::default();
    config.strategies.push(strategy);
    config.clients.push(client);

    let config_manager = Arc::new(ConfigManager::new(
        config,
        std::env::temp_dir().join(format!("failover-{}.yaml", uuid::Uuid::new_v4())),
    ));
    let metrics_db = Arc::new(
        lr_monitoring::storage::MetricsDatabase::new(
            std::env::temp_dir().join(format!("failover-metrics-{}.db", uuid::Uuid::new_v4())),
        )
        .unwrap(),
    );
    let health = Arc::new(lr_providers::health_cache::HealthCacheManager::new());
    let router = Router::new_without_free_tier(
        config_manager,
        registry,
        Arc::new(RateLimiterManager::new(None)),
        Arc::new(lr_monitoring::metrics::MetricsCollector::new(metrics_db)),
    )
    .with_health_cache(health.clone());
    Harness {
        router: Arc::new(router),
        client_id,
        strategy_id,
        scripts,
        order,
        health,
    }
}

/// The `attempts` a routing explanation lists, as `(provider, outcome)`.
fn attempts(metadata: &serde_json::Value) -> Vec<(String, String)> {
    metadata["attempts"]
        .as_array()
        .expect("routing metadata lists attempts")
        .iter()
        .map(|attempt| {
            (
                attempt["provider"].as_str().unwrap_or_default().to_string(),
                attempt["outcome"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

/// Poll until `condition` holds; usage accounting of a dropped stream runs
/// on a spawned task.
async fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..400 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("timed out waiting for: {what}");
}

// ============================================================================
// Failover across the strategy's prioritized models
// ============================================================================

#[tokio::test]
async fn first_model_succeeding_means_no_other_model_is_called() {
    let h = harness(vec![ok("a"), ok("b"), ok("c")]).await;

    let (response, metadata) = h.complete(AUTO).await.unwrap();

    assert_eq!(response.provider, "a");
    assert_eq!(response.model, "m");
    assert_eq!(h.order(), ["a"]);
    assert_eq!(h.script("b").calls(), 0);
    assert_eq!(h.script("c").calls(), 0);
    let metadata = metadata.unwrap();
    assert_eq!(attempts(&metadata), pairs(&[("a", "success")]));
    assert_eq!(metadata["successful_attempt"], 0);
}

#[tokio::test]
async fn retryable_failures_fail_over_in_priority_order() {
    let h = harness(vec![
        failing("a", Fail::Unavailable),
        failing("b", Fail::RateLimited(30)),
        failing("c", Fail::ContextLength),
        failing("d", Fail::ModelNotFound),
        ok("e"),
        ok("f"),
    ])
    .await;

    let (response, metadata) = h.complete(AUTO).await.unwrap();

    assert_eq!(response.provider, "e");
    assert_eq!(h.order(), ["a", "b", "c", "d", "e"]);
    assert_eq!(h.script("f").calls(), 0, "nothing after the first success");
    let metadata = metadata.unwrap();
    assert_eq!(
        attempts(&metadata),
        pairs(&[
            ("a", "unreachable"),
            ("b", "rate_limited"),
            ("c", "context_length"),
            ("d", "endpoint_not_supported"),
            ("e", "success"),
        ])
    );
    assert_eq!(metadata["successful_attempt"], 4);
    assert_eq!(
        metadata["candidate_models"],
        serde_json::json!(["a/m", "b/m", "c/m", "d/m", "e/m", "f/m"])
    );
}

#[tokio::test]
async fn non_retryable_error_stops_without_trying_the_next_model() {
    for fail in [Fail::BadRequest, Fail::InvalidParams] {
        let h = harness(vec![failing("a", fail), ok("b")]).await;

        let error = h.complete(AUTO).await.unwrap_err();

        assert_eq!(h.order(), ["a"], "the next model must not be tried");
        let AppError::RoutingFailure { error, explanation } = error else {
            panic!("expected the provider's own error with a routing explanation, got {error:?}");
        };
        match fail {
            Fail::BadRequest => assert!(
                matches!(*error, AppError::ProviderStatus { status: 400, .. }),
                "the upstream status must survive: {error:?}"
            ),
            _ => assert!(matches!(*error, AppError::InvalidParams(_)), "{error:?}"),
        }
        assert_eq!(attempts(&explanation), pairs(&[("a", "error")]));
    }
}

#[tokio::test]
async fn rate_limited_provider_is_skipped_while_it_backs_off() {
    let h = harness(vec![failing("a", Fail::RateLimited(120)), ok("b")]).await;

    let (first, _) = h.complete(AUTO).await.unwrap();
    assert_eq!(first.provider, "b");
    assert_eq!(h.order(), ["a", "b"]);

    // `a` answered 429 with a Retry-After: it is not called again while that
    // lasts, even though it would now succeed.
    h.script("a").set(Behavior::Succeed);
    let (second, metadata) = h.complete(AUTO).await.unwrap();

    assert_eq!(second.provider, "b");
    assert_eq!(h.script("a").calls(), 1, "a backed-off provider is skipped");
    assert_eq!(h.order(), ["a", "b", "b"]);
    assert_eq!(
        attempts(&metadata.unwrap()),
        pairs(&[("a", "backoff"), ("b", "success")])
    );
}

#[tokio::test]
async fn every_model_failing_returns_routing_exhausted_with_all_attempts() {
    let h = harness(vec![
        failing("a", Fail::Unavailable),
        failing("b", Fail::Unavailable),
    ])
    .await;

    let error = h.complete(AUTO).await.unwrap_err();

    assert_eq!(h.order(), ["a", "b"]);
    let AppError::RoutingExhausted {
        message,
        explanation,
    } = error
    else {
        panic!("expected RoutingExhausted, got {error:?}");
    };
    assert_eq!(
        message,
        "All auto-routing models failed. Last error: unreachable"
    );
    assert_eq!(
        attempts(&explanation),
        pairs(&[("a", "unreachable"), ("b", "unreachable")])
    );
    assert!(explanation.get("successful_attempt").is_none());
}

#[tokio::test]
async fn every_model_backing_off_is_exhausted_without_calling_any() {
    let h = harness(vec![ok("a"), ok("b")]).await;
    for provider in ["a", "b"] {
        h.router.free_tier_manager.record_rate_limit_error(
            provider,
            "m",
            429,
            Some(90),
            None,
            false,
        );
    }

    let error = h.complete(AUTO).await.unwrap_err();

    assert!(h.order().is_empty(), "no provider may be called");
    let AppError::RoutingExhausted {
        message,
        explanation,
    } = error
    else {
        panic!("expected RoutingExhausted, got {error:?}");
    };
    assert_eq!(
        message,
        "All auto-routing models failed. Last error: rate_limited"
    );
    assert_eq!(
        attempts(&explanation),
        pairs(&[("a", "backoff"), ("b", "backoff")])
    );
}

#[tokio::test]
async fn models_the_strategy_cannot_use_are_skipped_before_dispatch() {
    // `ghost` is prioritized but no such provider exists; `b` is configured
    // but the strategy does not allow it.
    let h = harness_with(vec![ok("a"), ok("b"), ok("c")], |strategy| {
        let auto = strategy.auto_config.as_mut().unwrap();
        auto.prioritized_models
            .insert(0, ("ghost".into(), "m".into()));
        strategy
            .model_permissions
            .providers
            .insert("b".into(), lr_config::PermissionState::Off);
    })
    .await;
    h.script("a").set(Behavior::Fail(Fail::Unavailable));

    let (response, metadata) = h.complete(AUTO).await.unwrap();

    assert_eq!(response.provider, "c");
    assert_eq!(h.order(), ["a", "c"]);
    assert_eq!(
        attempts(&metadata.unwrap()),
        pairs(&[
            ("ghost", "provider_not_found"),
            ("a", "unreachable"),
            ("b", "model_not_allowed"),
            ("c", "success"),
        ])
    );
}

#[tokio::test]
async fn a_request_for_one_model_is_not_failed_over() {
    let h = harness(vec![failing("a", Fail::Unavailable), ok("b")]).await;

    let error = h.complete("a/m").await.unwrap_err();

    assert!(
        matches!(error, AppError::ProviderUnavailable { status: 503, .. }),
        "the provider's error is returned as is: {error:?}"
    );
    assert_eq!(h.order(), ["a"]);
}

#[tokio::test]
async fn failing_health_check_does_not_keep_a_working_provider_from_serving() {
    let h = harness(vec![Instance {
        name: "a",
        behavior: Behavior::Succeed,
        healthy: false,
    }])
    .await;

    let (response, _) = h.complete(AUTO).await.unwrap();

    assert_eq!(response.provider, "a");
    assert_eq!(h.script("a").calls(), 1);
}

#[tokio::test]
async fn unreachable_provider_is_reported_unhealthy_and_others_are_not() {
    let h = harness(vec![
        failing("a", Fail::Unavailable),
        failing("b", Fail::RateLimited(30)),
        ok("c"),
    ])
    .await;

    h.complete(AUTO).await.unwrap();

    let health = h.health.get();
    assert_eq!(
        health.providers.get("a").map(|item| item.status),
        Some(lr_providers::health_cache::ItemHealthStatus::Unhealthy)
    );
    assert!(
        !health.providers.contains_key("b"),
        "a rate limit is not an outage"
    );
    assert!(!health.providers.contains_key("c"));
}

// ============================================================================
// Streaming failover
// ============================================================================

async fn collect(
    stream: Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>,
) -> Vec<AppResult<CompletionChunk>> {
    stream.collect().await
}

fn text(chunk: &CompletionChunk) -> &str {
    chunk.choices[0].delta.content.as_deref().unwrap_or("")
}

#[tokio::test]
async fn stream_fails_over_when_dispatch_fails_before_the_first_chunk() {
    let mut request = request(AUTO);
    request.stream = true;
    let h = harness(vec![failing("a", Fail::Unavailable), ok("b"), ok("c")]).await;

    let (stream, metadata) = h
        .router
        .stream_complete(&h.client_id, request)
        .await
        .unwrap();
    let chunks = collect(stream).await;

    assert_eq!(h.order(), ["a", "b"]);
    assert_eq!(h.script("c").calls(), 0);
    let mut chunks: Vec<CompletionChunk> = chunks.into_iter().map(Result::unwrap).collect();
    // Auto-routing closes the stream with a routing-metadata chunk.
    let trailer = chunks.pop().expect("a routing trailer");
    assert!(trailer.choices.is_empty());
    assert!(trailer
        .extensions
        .as_ref()
        .is_some_and(|extensions| extensions.contains_key("localrouter.routing")));
    assert_eq!(
        chunks.iter().map(text).collect::<Vec<_>>(),
        ["first from b", "last from b"]
    );
    for chunk in &chunks {
        assert_eq!(chunk.provider.as_deref(), Some("b"));
        assert_eq!(chunk.model, "b/m");
    }
    assert_eq!(
        attempts(&metadata.unwrap()),
        pairs(&[("a", "unreachable"), ("b", "success")])
    );
}

#[tokio::test]
async fn stream_is_not_failed_over_once_chunks_were_produced() {
    let mut request = request(AUTO);
    request.stream = true;
    let h = harness(vec![ok("a"), ok("b")]).await;
    h.script("a").set(Behavior::BreakAfterFirstChunk);

    let (stream, _) = h
        .router
        .stream_complete(&h.client_id, request)
        .await
        .unwrap();
    let chunks = collect(stream).await;

    // The client already holds `a`'s first chunk: splicing `b`'s answer onto
    // it would corrupt the response, so the failure is passed on.
    assert_eq!(h.order(), ["a"], "no second dispatch after output began");
    assert_eq!(
        chunks.len(),
        3,
        "first chunk, the failure, the routing trailer"
    );
    assert_eq!(text(chunks[0].as_ref().unwrap()), "first from a");
    let Err(AppError::RoutingFailure { error, explanation }) = &chunks[1] else {
        panic!("the mid-stream failure reaches the client: {:?}", chunks[1]);
    };
    assert!(
        matches!(**error, AppError::ProviderUnavailable { status: 503, .. }),
        "the provider's own error is kept: {error:?}"
    );
    assert_eq!(attempts(explanation), pairs(&[("a", "success")]));
    assert!(chunks[2].as_ref().unwrap().choices.is_empty());
}

#[tokio::test]
async fn stream_non_retryable_dispatch_error_stops_failover() {
    let mut request = request(AUTO);
    request.stream = true;
    let h = harness(vec![failing("a", Fail::BadRequest), ok("b")]).await;

    let error = h
        .router
        .stream_complete(&h.client_id, request)
        .await
        .err()
        .expect("dispatch must fail");

    assert_eq!(h.order(), ["a"]);
    let AppError::RoutingFailure { error, .. } = error else {
        panic!("expected RoutingFailure, got {error:?}");
    };
    assert!(matches!(
        *error,
        AppError::ProviderStatus { status: 400, .. }
    ));
}

#[tokio::test]
async fn stream_every_model_failing_returns_routing_exhausted() {
    let mut request = request(AUTO);
    request.stream = true;
    let h = harness(vec![
        failing("a", Fail::Unavailable),
        failing("b", Fail::RateLimited(30)),
    ])
    .await;

    let error = h
        .router
        .stream_complete(&h.client_id, request)
        .await
        .err()
        .expect("dispatch must fail");

    assert_eq!(h.order(), ["a", "b"]);
    let AppError::RoutingExhausted {
        message,
        explanation,
    } = error
    else {
        panic!("expected RoutingExhausted, got {error:?}");
    };
    assert_eq!(
        message,
        "All auto-routing streaming models failed. Last error: rate_limited"
    );
    assert_eq!(
        attempts(&explanation),
        pairs(&[("a", "unreachable"), ("b", "rate_limited")])
    );
}

// ============================================================================
// Admission: in-flight reservations
// ============================================================================

fn requests_per_minute(limit: f64) -> Vec<RateLimiter> {
    vec![RateLimiter::new(RateLimitType::Requests, limit, 60)]
}

/// Run `total` concurrent requests against a provider that holds every call,
/// and return how many were admitted (reached the provider) and how many
/// were turned away, before any of them finished.
async fn race(h: &Harness, total: usize) -> (usize, usize) {
    let gate = h.script("a").hold();
    let rejected = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for _ in 0..total {
        let router = h.router.clone();
        let client_id = h.client_id.clone();
        let rejected = rejected.clone();
        tasks.push(tokio::spawn(async move {
            let result = router.complete(&client_id, request("a/m")).await;
            if matches!(result, Err(AppError::RateLimitExceeded)) {
                rejected.fetch_add(1, Ordering::SeqCst);
            }
            result.map(|_| ())
        }));
    }
    let script = h.script("a").clone();
    let seen = rejected.clone();
    eventually("every request to be admitted or rejected", move || {
        script.entered.load(Ordering::SeqCst) + seen.load(Ordering::SeqCst) == total
    })
    .await;
    let admitted = h.script("a").entered.load(Ordering::SeqCst);
    let turned_away = rejected.load(Ordering::SeqCst);

    gate.add_permits(total);
    let mut succeeded = 0;
    for task in tasks {
        if task.await.unwrap().is_ok() {
            succeeded += 1;
        }
    }
    assert_eq!(succeeded, admitted, "every admitted request completes");
    (admitted, turned_away)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_requests_admit_exactly_the_client_request_limit() {
    let h = harness(vec![ok("a")]).await;
    h.limit_client(requests_per_minute(3.0));

    let (admitted, rejected) = race(&h, 10).await;

    assert_eq!(admitted, 3, "exactly the limit is admitted while in flight");
    assert_eq!(rejected, 7);
    assert_eq!(
        h.script("a").calls(),
        3,
        "a rejected request never reaches a provider"
    );
    assert_eq!(h.recorded(RateLimitType::Requests).await, 3.0);
    assert_eq!(h.in_flight_requests(), 0, "finished requests hold nothing");
    // The window is now full of completed requests.
    assert!(matches!(
        h.complete("a/m").await,
        Err(AppError::RateLimitExceeded)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_requests_admit_exactly_the_strategy_request_limit() {
    let h = harness_with(vec![ok("a")], |strategy| {
        strategy.rate_limits = vec![StrategyRateLimit {
            limit_type: lr_config::RateLimitType::Requests,
            value: 2.0,
            time_window: RateLimitTimeWindow::Minute,
            enabled: true,
        }];
    })
    .await;

    let (admitted, rejected) = race(&h, 8).await;

    assert_eq!(
        admitted, 2,
        "metrics show no in-flight request; reservations do"
    );
    assert_eq!(rejected, 6);
    assert_eq!(
        h.limiter().strategy_in_flight(&h.strategy_id).requests,
        0,
        "finished requests hold nothing"
    );
}

#[tokio::test]
async fn failed_request_releases_its_reservation() {
    let h = harness(vec![failing("a", Fail::Unavailable)]).await;
    h.limit_client(requests_per_minute(1.0));
    let limiter = h.limiter().clone();
    let client_id = h.client_id.clone();
    let held_during_call = Arc::new(AtomicUsize::new(0));
    let seen = held_during_call.clone();
    h.script("a").probe(move || {
        seen.store(
            limiter.api_key_in_flight(&client_id).requests as usize,
            Ordering::SeqCst,
        );
    });

    assert!(h.complete("a/m").await.is_err());

    assert_eq!(held_during_call.load(Ordering::SeqCst), 1);
    assert_eq!(h.in_flight_requests(), 0, "the failure gave the slot back");
    assert_eq!(h.recorded(RateLimitType::Requests).await, 0.0);
    // The only slot is free again for a request that succeeds.
    h.script("a").set(Behavior::Succeed);
    h.complete("a/m").await.unwrap();
    assert_eq!(h.recorded(RateLimitType::Requests).await, 1.0);
}

#[tokio::test]
async fn request_rejected_by_every_model_releases_its_reservation() {
    let h = harness(vec![
        failing("a", Fail::Unavailable),
        failing("b", Fail::Unavailable),
    ])
    .await;
    h.limit_client(requests_per_minute(1.0));

    assert!(matches!(
        h.complete(AUTO).await,
        Err(AppError::RoutingExhausted { .. })
    ));

    assert_eq!(h.in_flight_requests(), 0);
    assert_eq!(h.recorded(RateLimitType::Requests).await, 0.0);
}

#[tokio::test]
async fn cancelled_request_releases_its_reservation() {
    let h = harness(vec![ok("a")]).await;
    h.limit_client(requests_per_minute(1.0));
    let gate = h.script("a").hold();

    let router = h.router.clone();
    let client_id = h.client_id.clone();
    let task = tokio::spawn(async move { router.complete(&client_id, request("a/m")).await });
    let script = h.script("a").clone();
    eventually("the request to reach the provider", move || {
        script.entered.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(h.in_flight_requests(), 1);

    // The client goes away while the upstream is still working.
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());

    assert_eq!(h.in_flight_requests(), 0, "cancellation gave the slot back");
    assert_eq!(h.recorded(RateLimitType::Requests).await, 0.0);
    drop(gate);
}

#[tokio::test]
async fn failover_attempts_share_one_reservation() {
    let h = harness_with(
        vec![
            failing("a", Fail::Unavailable),
            failing("b", Fail::RateLimited(30)),
            ok("c"),
        ],
        |strategy| {
            strategy.rate_limits = vec![StrategyRateLimit {
                limit_type: lr_config::RateLimitType::Requests,
                value: 1.0,
                time_window: RateLimitTimeWindow::Minute,
                enabled: true,
            }];
        },
    )
    .await;
    h.limit_client(requests_per_minute(1.0));
    // Every attempt looks at what the request holds at that moment.
    let held: Arc<parking_lot::Mutex<Vec<(u64, u64)>>> = Default::default();
    for name in ["a", "b", "c"] {
        let limiter = h.limiter().clone();
        let client_id = h.client_id.clone();
        let strategy_id = h.strategy_id.clone();
        let held = held.clone();
        h.script(name).probe(move || {
            held.lock().push((
                limiter.api_key_in_flight(&client_id).requests,
                limiter.strategy_in_flight(&strategy_id).requests,
            ));
        });
    }

    // With a limit of one request, a second reservation for the same request
    // would turn its own failover attempts away.
    let (response, _) = h.complete(AUTO).await.unwrap();

    assert_eq!(response.provider, "c");
    assert_eq!(h.order(), ["a", "b", "c"]);
    assert_eq!(
        *held.lock(),
        [(1, 1), (1, 1), (1, 1)],
        "one request holds one slot on every attempt"
    );
    assert_eq!(
        h.recorded(RateLimitType::Requests).await,
        1.0,
        "three attempts are one request"
    );
    assert_eq!(h.in_flight_requests(), 0);
    assert_eq!(h.limiter().strategy_in_flight(&h.strategy_id).requests, 0);
}

#[tokio::test]
async fn reservation_is_reconciled_to_actual_usage() {
    let h = harness(vec![ok("a")]).await;
    h.limit_client(vec![
        RateLimiter::new(RateLimitType::InputTokens, 100_000.0, 60),
        RateLimiter::new(RateLimitType::OutputTokens, 100_000.0, 60),
        RateLimiter::new(RateLimitType::TotalTokens, 100_000.0, 60),
    ]);
    let mut request = request("a/m");
    request.max_tokens = Some(5_000);
    let prompt_estimate =
        lr_providers::usage_estimate::prompt_tokens(&request.messages, request.tools.as_deref());
    let limiter = h.limiter().clone();
    let client_id = h.client_id.clone();
    let held: Arc<parking_lot::Mutex<Option<rate_limit::ReservedUsage>>> = Default::default();
    let seen = held.clone();
    h.script("a").probe(move || {
        *seen.lock() = Some(limiter.api_key_in_flight(&client_id));
    });

    h.router.complete(&h.client_id, request).await.unwrap();

    // In flight: the estimate (the prompt, and the output the request caps
    // itself at).
    let held = held.lock().clone().unwrap();
    assert_eq!(held.requests, 1);
    assert_eq!(held.input_tokens, prompt_estimate);
    assert_eq!(held.output_tokens, 5_000);
    // Afterwards: what the provider reported, and nothing held.
    assert_eq!(
        h.recorded(RateLimitType::InputTokens).await,
        PROMPT_TOKENS as f64
    );
    assert_eq!(
        h.recorded(RateLimitType::OutputTokens).await,
        COMPLETION_TOKENS as f64
    );
    assert_eq!(
        h.recorded(RateLimitType::TotalTokens).await,
        (PROMPT_TOKENS + COMPLETION_TOKENS) as f64
    );
    let after = h.limiter().api_key_in_flight(&h.client_id);
    assert_eq!(
        (after.requests, after.input_tokens, after.output_tokens),
        (0, 0, 0)
    );
}

#[tokio::test]
async fn in_flight_token_estimates_count_against_the_limit() {
    let h = harness(vec![ok("a")]).await;
    h.limit_client(vec![RateLimiter::new(
        RateLimitType::OutputTokens,
        1_000.0,
        60,
    )]);
    let gate = h.script("a").hold();

    // Two requests in flight, each capped at 600 output tokens.
    let mut tasks = Vec::new();
    for expected in 1..=2 {
        let router = h.router.clone();
        let client_id = h.client_id.clone();
        tasks.push(tokio::spawn(async move {
            let mut request = request("a/m");
            request.max_tokens = Some(600);
            router.complete(&client_id, request).await
        }));
        let script = h.script("a").clone();
        eventually("the request to reach the provider", move || {
            script.entered.load(Ordering::SeqCst) == expected
        })
        .await;
    }
    assert_eq!(
        h.limiter().api_key_in_flight(&h.client_id).output_tokens,
        1_200
    );
    // Nothing has completed, but the budget is spoken for.
    assert!(matches!(
        h.complete("a/m").await,
        Err(AppError::RateLimitExceeded)
    ));

    gate.add_permits(2);
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    // Each used 10 output tokens: the estimates are gone and there is room.
    assert_eq!(
        h.recorded(RateLimitType::OutputTokens).await,
        2.0 * COMPLETION_TOKENS as f64
    );
    gate.add_permits(1);
    h.complete("a/m").await.unwrap();
}

#[tokio::test]
async fn completed_stream_holds_its_reservation_until_usage_is_recorded() {
    let mut request = request(AUTO);
    request.stream = true;
    let h = harness(vec![failing("a", Fail::Unavailable), ok("b")]).await;
    h.limit_client(requests_per_minute(1.0));

    let (mut stream, _) = h
        .router
        .stream_complete(&h.client_id, request)
        .await
        .unwrap();

    // The dispatch returned, but the request is still in flight.
    assert_eq!(h.in_flight_requests(), 1, "failover reserved once");
    assert!(matches!(
        h.complete("b/m").await,
        Err(AppError::RateLimitExceeded)
    ));
    while let Some(chunk) = stream.next().await {
        chunk.unwrap();
    }
    drop(stream);

    let limiter = h.limiter().clone();
    let client_id = h.client_id.clone();
    eventually("the stream's reservation to be released", move || {
        limiter.api_key_in_flight(&client_id).requests == 0
    })
    .await;
    assert_eq!(h.recorded(RateLimitType::Requests).await, 1.0);
}

#[tokio::test]
async fn stream_cancelled_mid_flight_releases_its_reservation() {
    let mut request = request("a/m");
    request.stream = true;
    let h = harness(vec![ok("a")]).await;
    h.limit_client(vec![
        RateLimiter::new(RateLimitType::Requests, 5.0, 60),
        RateLimiter::new(RateLimitType::OutputTokens, 100_000.0, 60),
    ]);

    let (mut stream, _) = h
        .router
        .stream_complete(&h.client_id, request)
        .await
        .unwrap();
    let first = stream.next().await.unwrap().unwrap();
    assert_eq!(text(&first), "first from a");
    assert_eq!(h.in_flight_requests(), 1);

    // The client disconnects after the first chunk.
    drop(stream);

    let limiter = h.limiter().clone();
    let client_id = h.client_id.clone();
    eventually(
        "the cancelled stream's reservation to be released",
        move || limiter.api_key_in_flight(&client_id).requests == 0,
    )
    .await;
    // The request happened and what it produced is charged; only the
    // estimate is gone.
    assert_eq!(h.recorded(RateLimitType::Requests).await, 1.0);
    assert!(h.recorded(RateLimitType::OutputTokens).await > 0.0);
}

#[tokio::test]
async fn stream_failing_before_the_first_chunk_releases_its_reservation() {
    let mut request = request(AUTO);
    request.stream = true;
    let h = harness(vec![failing("a", Fail::Unavailable)]).await;
    h.limit_client(requests_per_minute(1.0));

    assert!(h
        .router
        .stream_complete(&h.client_id, request)
        .await
        .is_err());

    assert_eq!(h.in_flight_requests(), 0);
    assert_eq!(h.recorded(RateLimitType::Requests).await, 0.0);
}

#[tokio::test]
async fn paid_fallback_retries_are_admitted_like_any_request() {
    let h = harness(vec![ok("a")]).await;
    h.limit_client(requests_per_minute(2.0));

    h.router
        .complete_with_paid_fallback(&h.client_id, request("a/m"))
        .await
        .unwrap();
    let mut streaming = request(AUTO);
    streaming.stream = true;
    let (stream, _) = h
        .router
        .stream_complete_with_paid_fallback(&h.client_id, streaming)
        .await
        .unwrap();
    assert_eq!(h.in_flight_requests(), 1, "the stream holds its admission");
    collect(stream).await;

    let limiter = h.limiter().clone();
    let client_id = h.client_id.clone();
    eventually("the paid stream's reservation to be released", move || {
        limiter.api_key_in_flight(&client_id).requests == 0
    })
    .await;
    assert_eq!(h.recorded(RateLimitType::Requests).await, 2.0);
    assert!(matches!(
        h.router
            .complete_with_paid_fallback(&h.client_id, request("a/m"))
            .await,
        Err(AppError::RateLimitExceeded)
    ));
}
