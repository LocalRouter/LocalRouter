//! Bounded, private, process-local exact response retention.
use lr_config::ResponseCacheConfig;
use lr_providers::response_cache::CacheMode;
use lr_providers::{CompletionResponse, SystemOneResponse};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;
const GLOBAL_BYTES: usize = 64 * 1024 * 1024;
const GLOBAL_ENTRIES: usize = 4096;
const MAX_ENTRY_BYTES: usize = 1024 * 1024;
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "response")]
pub enum CachedResponse {
    Chat(CompletionResponse),
    Decision(SystemOneResponse),
}
#[derive(Clone)]
struct Entry {
    client_id: String,
    payload: CachedResponse,
    created: Instant,
    expires: Instant,
    bytes: usize,
    reference_cost: Option<f64>,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct CacheStats {
    pub entries: usize,
    pub bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub bypasses: u64,
    pub evictions: u64,
    pub upstream_requests_avoided: u64,
    pub avoided_cost_estimate_usd: Option<f64>,
}
#[derive(Default)]
struct Inner {
    pending: HashMap<String, PendingTicket>,
    entries: HashMap<String, Entry>,
    stats: HashMap<String, CacheStats>,
    generation: u64,
}
pub struct ResponseCache {
    inner: Arc<Mutex<Inner>>,
    salt: String,
}
struct PendingTicket {
    ticket: CacheTicket,
    reference: Option<f64>,
    expires: Instant,
}
pub struct CacheTicket {
    configuration_fingerprint: String,
    key: String,
    client_id: String,
    settings: ResponseCacheConfig,
    generation: u64,
    pub mode: CacheMode,
}
impl Default for ResponseCache {
    fn default() -> Self {
        let inner = Arc::new(Mutex::new(Inner::default()));
        if tokio::runtime::Handle::try_current().is_ok() {
            let weak = Arc::downgrade(&inner);
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(30));
                loop {
                    interval.tick().await;
                    let Some(inner) = weak.upgrade() else {
                        break;
                    };
                    let mut inner = inner.lock();
                    inner
                        .entries
                        .retain(|_, entry| entry.expires > Instant::now());
                    inner
                        .pending
                        .retain(|_, entry| entry.expires > Instant::now());
                }
            });
        }
        Self {
            inner,
            salt: Uuid::new_v4().to_string(),
        }
    }
}
impl ResponseCache {
    fn stats_mut<'a>(inner: &'a mut Inner, client: &str) -> &'a mut CacheStats {
        // Statistics are local counters, never a telemetry stream or payload log.
        if !inner.stats.contains_key(client) && inner.stats.len() >= 256 {
            if let Some(key) = inner.stats.keys().next().cloned() {
                inner.stats.remove(&key);
            }
        }
        inner.stats.entry(client.into()).or_default()
    }
    pub fn bypass(&self, client: &str) {
        Self::stats_mut(&mut self.inner.lock(), client).bypasses += 1;
    }
    pub fn ticket(
        &self,
        client: &str,
        credential_scope: &str,
        request: Value,
        configuration: Value,
        settings: &ResponseCacheConfig,
        mode: CacheMode,
    ) -> Option<CacheTicket> {
        if !settings.enabled || settings.validate().is_err() || mode == CacheMode::Bypass {
            self.bypass(client);
            return None;
        }
        let wire = serde_json::to_string(&request).ok()?;
        let configuration_fingerprint = crate::explanation::fingerprint(configuration);
        let key = crate::explanation::fingerprint(
            json!({"salt":self.salt,"client":client,"credential":credential_scope,"configuration":configuration_fingerprint,"request_wire":wire}),
        );
        Some(CacheTicket {
            configuration_fingerprint,
            key,
            client_id: client.into(),
            settings: settings.clone(),
            generation: self.inner.lock().generation,
            mode,
        })
    }
    pub fn get(&self, ticket: &CacheTicket) -> Option<(CachedResponse, Value)> {
        let mut inner = self.inner.lock();
        inner
            .entries
            .retain(|_, entry| entry.expires > Instant::now());
        let entry = if ticket.mode == CacheMode::Refresh {
            None
        } else {
            inner.entries.get(&ticket.key).cloned()
        };
        let stats = Self::stats_mut(&mut inner, &ticket.client_id);
        match entry {
            Some(entry) => {
                stats.hits += 1;
                stats.upstream_requests_avoided += 1;
                if let Some(cost) = entry.reference_cost {
                    let total = stats.avoided_cost_estimate_usd.unwrap_or(0.0) + cost;
                    if total.is_finite() {
                        stats.avoided_cost_estimate_usd = Some(total);
                    }
                }
                Some((
                    entry.payload,
                    json!({"hit":true,"scope":"resolved_model_call","age_ms":entry.created.elapsed().as_millis().min(u64::MAX as u128) as u64,"upstream_spend_usd":0.0,"billed_input_tokens":0,"billed_output_tokens":0,"avoided_cost_estimate_usd":entry.reference_cost}),
                ))
            }
            None => {
                stats.misses += 1;
                None
            }
        }
    }
    pub fn insert(
        &self,
        ticket: CacheTicket,
        payload: CachedResponse,
        reference_cost: Option<f64>,
    ) -> bool {
        let Ok(bytes) = serde_json::to_vec(&payload).map(|v| v.len()) else {
            return false;
        };
        if bytes > MAX_ENTRY_BYTES || bytes > ticket.settings.max_bytes {
            return false;
        }
        let mut inner = self.inner.lock();
        if ticket.generation != inner.generation {
            return false;
        }
        inner
            .entries
            .retain(|_, entry| entry.expires > Instant::now());
        inner.entries.remove(&ticket.key);
        loop {
            let (mut client_count, mut client_bytes, mut total_bytes) = (0, 0usize, 0usize);
            for entry in inner.entries.values() {
                total_bytes = total_bytes.saturating_add(entry.bytes);
                if entry.client_id == ticket.client_id {
                    client_count += 1;
                    client_bytes = client_bytes.saturating_add(entry.bytes);
                }
            }
            let client_full = client_count >= ticket.settings.max_entries
                || client_bytes.saturating_add(bytes) > ticket.settings.max_bytes;
            let global_full = inner.entries.len() >= GLOBAL_ENTRIES
                || total_bytes.saturating_add(bytes) > GLOBAL_BYTES;
            if !client_full && !global_full {
                break;
            }
            let oldest = inner
                .entries
                .iter()
                .filter(|(_, entry)| !client_full || entry.client_id == ticket.client_id)
                .min_by_key(|(_, entry)| entry.created)
                .map(|(key, entry)| (key.clone(), entry.client_id.clone()));
            let Some((key, client)) = oldest else {
                return false;
            };
            inner.entries.remove(&key);
            Self::stats_mut(&mut inner, &client).evictions += 1;
        }
        let now = Instant::now();
        inner.entries.insert(
            ticket.key,
            Entry {
                client_id: ticket.client_id,
                payload,
                created: now,
                expires: now + Duration::from_secs(ticket.settings.ttl_secs),
                bytes,
                reference_cost: reference_cost.filter(|c| c.is_finite() && *c >= 0.0),
            },
        );
        true
    }
    pub fn defer(&self, ticket: CacheTicket, reference: Option<f64>) -> Option<String> {
        let mut inner = self.inner.lock();
        inner
            .pending
            .retain(|_, pending| pending.expires > Instant::now());
        if inner.pending.len() >= 1024 || ticket.generation != inner.generation {
            return None;
        }
        let token = Uuid::new_v4().to_string();
        inner.pending.insert(
            token.clone(),
            PendingTicket {
                ticket,
                reference,
                expires: Instant::now() + Duration::from_secs(30),
            },
        );
        Some(token)
    }
    fn take_pending(&self, token: &str, client: &str) -> Option<(CacheTicket, Option<f64>)> {
        let pending = self.inner.lock().pending.remove(token)?;
        (pending.expires > Instant::now() && pending.ticket.client_id == client)
            .then_some((pending.ticket, pending.reference))
    }
    pub fn clear(&self, client: Option<&str>) {
        let mut inner = self.inner.lock();
        inner.generation = inner.generation.wrapping_add(1);
        inner
            .pending
            .retain(|_, pending| client.is_some_and(|client| pending.ticket.client_id != client));
        inner
            .entries
            .retain(|_, entry| client.is_some_and(|client| entry.client_id != client));
    }
    pub fn stats(&self, client: Option<&str>) -> CacheStats {
        let mut inner = self.inner.lock();
        inner
            .entries
            .retain(|_, entry| entry.expires > Instant::now());
        let mut result = CacheStats::default();
        for (id, stats) in &inner.stats {
            if client.is_none_or(|client| id == client) {
                result.hits += stats.hits;
                result.misses += stats.misses;
                result.bypasses += stats.bypasses;
                result.evictions += stats.evictions;
                result.upstream_requests_avoided += stats.upstream_requests_avoided;
                if let Some(cost) = stats.avoided_cost_estimate_usd {
                    result.avoided_cost_estimate_usd =
                        Some(result.avoided_cost_estimate_usd.unwrap_or(0.0) + cost);
                }
            }
        }
        for entry in inner.entries.values() {
            if client.is_none_or(|client| entry.client_id == client) {
                result.entries += 1;
                result.bytes += entry.bytes;
            }
        }
        result
    }
}

/// Exclude controls that can execute tools or bind a reply to a provider session.
fn unsafe_state(value: &Value, depth: usize) -> bool {
    if depth > 64 {
        return true;
    }
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            matches!(
                key.as_str(),
                "previous_response_id"
                    | "conversation"
                    | "conversation_id"
                    | "encrypted_content"
                    | "encryptedContent"
                    | "thought_signature"
                    | "thoughtSignature"
                    | "signature"
                    | "tool_calls"
                    | "tool_call_id"
                    | "web_search_options"
                    | "plugins"
                    | "server_tools"
            ) && !value.is_null()
                || (key == "type"
                    && value.as_str().is_some_and(|v| {
                        matches!(
                            v,
                            "image"
                                | "image_url"
                                | "input_image"
                                | "input_audio"
                                | "audio"
                                | "thinking"
                                | "redacted_thinking"
                                | "reasoning"
                                | "web_search"
                                | "web_search_preview"
                                | "computer"
                        )
                    }))
                || unsafe_state(value, depth + 1)
        }),
        Value::Array(items) => items.iter().any(|v| unsafe_state(v, depth + 1)),
        _ => false,
    }
}
impl crate::Router {
    pub fn response_cache_stats(&self, client: Option<&str>) -> CacheStats {
        self.response_cache.stats(client)
    }
    pub fn clear_response_cache(&self, client: Option<&str>) {
        self.response_cache.clear(client);
    }
    fn response_cache_ticket(
        &self,
        client: &str,
        provider: &str,
        model: &str,
        endpoint: &str,
        mut request: Value,
        native_decisions: Option<Value>,
    ) -> Option<CacheTicket> {
        if matches!(client, "internal-test" | "memory-service") || lr_types::is_duplicate_hop() {
            return None;
        }
        if self.client_privacy(client).no_retention {
            self.response_cache.bypass(client);
            return None;
        }
        let config = self.config_manager.get();
        let strategy = config
            .clients
            .iter()
            .find(|c| c.id == client && c.enabled)
            .and_then(|c| config.strategies.iter().find(|s| s.id == c.strategy_id))?;
        if !strategy.response_cache.enabled {
            return None;
        }
        let model_lower = model.to_ascii_lowercase();
        let kind = self
            .provider_registry
            .get_provider_type_for_instance(provider)?;
        if kind == "perplexity"
            || model_lower.contains(":online")
            || model_lower.contains("search")
            || model_lower.starts_with("sonar")
            || model_lower.contains("grounded")
        {
            self.response_cache.bypass(client);
            return None;
        }
        // Derived local routing state is stripped from the actual provider wire
        // too. It is not an opaque vendor action and must not disable caching.
        if let Some(extensions) = request.get_mut("extensions").and_then(Value::as_object_mut) {
            extensions.remove(lr_providers::privacy::ROUTE_CLIENT_EXTENSION);
            extensions.remove(lr_providers::privacy::INTERNAL_EXTENSION);
        }
        let extensions = request.get("extensions").and_then(Value::as_object)?;
        let scope_info = extensions.get(lr_providers::response_cache::SCOPE)?;
        let scope = scope_info.get("credential")?.as_str()?.to_string();
        let ingress = scope_info.get("endpoint")?.as_str()?.to_string();
        // Session state, replay and unknown vendor extensions are excluded rather
        // than assuming that opaque options cannot trigger external actions.
        if extensions.keys().any(|key| {
            !matches!(
                key.as_str(),
                lr_providers::response_cache::SCOPE
                    | lr_providers::response_cache::EXTENSION
                    | lr_providers::gateway::EXTENSION
                    | "json_mode"
                    | "structured_outputs"
                    | "logprobs"
                    | "prompt_caching"
            )
        }) {
            self.response_cache.bypass(client);
            return None;
        }
        let mode = extensions
            .get(lr_providers::response_cache::EXTENSION)
            .map(|v| {
                serde_json::from_value::<lr_providers::response_cache::CacheOptions>(v.clone())
            })
            .transpose()
            .ok()?
            .unwrap_or_default()
            .mode;
        let object = request.as_object_mut()?;
        let extensions = object.get_mut("extensions")?.as_object_mut()?;
        extensions.remove(lr_providers::response_cache::SCOPE);
        extensions.remove(lr_providers::response_cache::EXTENSION);
        if unsafe_state(&request, 0)
            || native_decisions
                .as_ref()
                .is_some_and(|value| unsafe_state(value, 0))
        {
            self.response_cache.bypass(client);
            return None;
        }
        self.response_cache.ticket(client,&scope,json!({"provider":provider,"model":model,"endpoint":format!("{endpoint}:{ingress}"),"body":request,"native_decisions":native_decisions}),json!({"configuration":config.routing_identity(),"provider_revision":self.provider_registry.routing_revision()}),&strategy.response_cache,mode)
    }
    pub(crate) fn chat_cache_ticket(
        &self,
        client: &str,
        provider: &str,
        model: &str,
        request: &lr_providers::CompletionRequest,
    ) -> Option<CacheTicket> {
        if request.has_document_context()
            || request.server_tools.is_some()
            || request.native_messages.is_some()
            || request.stream
            || request.tools.as_ref().is_some_and(|t| !t.is_empty())
            || request.tool_choice.is_some()
            || request.messages.iter().any(|m| {
                !matches!(m.content, lr_providers::ChatMessageContent::Text(_))
                    || !matches!(
                        m.role.as_str(),
                        "system" | "developer" | "user" | "assistant"
                    )
                    || m.tool_calls.is_some()
                    || m.tool_call_id.is_some()
                    || m.reasoning_content.is_some()
            })
        {
            self.response_cache.bypass(client);
            return None;
        }
        self.response_cache_ticket(
            client,
            provider,
            model,
            "chat",
            serde_json::to_value(request).ok()?,
            None,
        )
    }
    pub(crate) fn decision_cache_ticket(
        &self,
        client: &str,
        provider: &str,
        model: &str,
        request: &lr_providers::SystemOneRequest,
    ) -> Option<CacheTicket> {
        if request
            .decisions
            .as_ref()
            .is_some_and(|d| d.input.has_images())
        {
            self.response_cache.bypass(client);
            return None;
        }
        let native_decisions = if request.decisions.is_some() {
            let updated =
                lr_providers::systemone::decisions::DecisionRequest::from_systemone(request)
                    .ok()?;
            let mut value = serde_json::to_value(updated).ok()?;
            if let Some(extensions) = value.get_mut("extensions").and_then(Value::as_object_mut) {
                extensions.remove(lr_providers::response_cache::EXTENSION);
                extensions.remove(lr_providers::response_cache::SCOPE);
                extensions.remove(lr_providers::response_cache::PENDING);
            }
            Some(value)
        } else {
            None
        };
        self.response_cache_ticket(
            client,
            provider,
            model,
            "systemone",
            serde_json::to_value(request).ok()?,
            native_decisions,
        )
    }
    pub(crate) async fn chat_cache_hit(
        &self,
        ticket: &CacheTicket,
        client: &str,
        provider: &str,
        model: &str,
        free_tier_only: bool,
    ) -> Option<CompletionResponse> {
        let (CachedResponse::Chat(mut response), mut metadata) = self.response_cache.get(ticket)?
        else {
            return None;
        };
        metadata["original_response_id"] = json!(response.id);
        metadata["served_usage"] = serde_json::to_value(&response.usage).ok()?;
        let extensions = response.extensions.get_or_insert_with(Default::default);
        if let Some(original) = extensions.remove(lr_providers::gateway::EXTENSION) {
            metadata["original_gateway"] = original;
        }
        extensions.remove(lr_providers::session::ACCOUNTING);
        extensions.remove("localrouter.routing");
        extensions.insert(lr_providers::response_cache::EXTENSION.into(), metadata);
        response.id = format!("cache-{}", Uuid::new_v4());
        response.created = chrono::Utc::now().timestamp();
        response.usage.prompt_tokens_details = None;
        response.usage.completion_tokens_details = None;
        self.record_cached_request(client, provider, model, free_tier_only)
            .await;
        Some(response)
    }
    pub(crate) async fn decision_cache_hit(
        &self,
        ticket: &CacheTicket,
        client: &str,
        provider: &str,
        model: &str,
        free_tier_only: bool,
    ) -> Option<SystemOneResponse> {
        let (CachedResponse::Decision(mut response), mut metadata) =
            self.response_cache.get(ticket)?
        else {
            return None;
        };
        metadata["original_request_id"] = json!(response.request_id);
        metadata["served_usage"] = serde_json::to_value(&response.usage).ok()?;
        if let Some(value) = response.extra.remove("provider_metadata") {
            metadata["original_provider_metadata"] = value;
        }
        let extensions = response
            .extra
            .entry("extensions")
            .or_insert_with(|| json!({}))
            .as_object_mut()?;
        if let Some(original) = extensions.remove(lr_providers::gateway::EXTENSION) {
            metadata["original_gateway"] = original;
        }
        extensions.remove(lr_providers::session::ACCOUNTING);
        extensions.remove("localrouter.routing");
        extensions.insert(lr_providers::response_cache::EXTENSION.into(), metadata);
        response.request_id = None;
        response.cost_usd = Some(0.0);
        response.usage.extra.clear();
        self.record_cached_request(client, provider, model, free_tier_only)
            .await;
        Some(response)
    }
    async fn record_cached_request(
        &self,
        client: &str,
        provider: &str,
        model: &str,
        _free_tier_only: bool,
    ) {
        // Counts a client request, but consumes no provider token allowance,
        // paid quota, dispatch slot, latency sample or infrastructure result.
        let rate_limiter = self.rate_limiter.clone();
        let client = client.to_string();
        self.metrics_collector
            .record_feature_event("feature_response_cache_hit", 0, 0.0);
        let job = lr_types::spawn_traced(async move {
            if let Err(error) = rate_limiter
                .record_api_key_usage(
                    &client,
                    &crate::UsageInfo {
                        input_tokens: 0,
                        output_tokens: 0,
                        cost_usd: 0.0,
                    },
                )
                .await
            {
                tracing::warn!("Cached request accounting failed: {error}");
            }
        });
        if let Err(error) = job.await {
            tracing::warn!("Cached request accounting job failed: {error}");
        }
        let _ = (provider, model);
    }
}

pub(crate) fn cacheable_chat_response(response: &CompletionResponse) -> bool {
    // Native Responses envelopes carry provider item ids and continuation
    // semantics; leave those intact and outside this first text-cache release.
    !response.extensions.as_ref().is_some_and(|extensions| {
        extensions.contains_key(lr_providers::openai_responses::NATIVE_RESPONSES_API_EXT_KEY)
    }) && !response.choices.is_empty()
        && response.choices.iter().all(|choice| {
            choice.finish_reason.as_deref() == Some("stop")
                && choice.message.tool_calls.is_none()
                && choice.message.tool_call_id.is_none()
                && matches!(
                    choice.message.content,
                    lr_providers::ChatMessageContent::Text(_)
                )
        })
        && serde_json::to_value(response).is_ok_and(|value| !unsafe_state(&value, 0))
}

impl crate::Router {
    pub(crate) fn cache_reference_cost(
        &self,
        client: &str,
        provider: &str,
        model: &str,
        reported: Option<f64>,
        estimated: f64,
        cache_rates_known: bool,
    ) -> Option<f64> {
        if reported.is_some() {
            return reported;
        }
        if !cache_rates_known {
            return None;
        }
        let local = self
            .provider_registry
            .get_provider_type_for_instance(provider)
            .and_then(|kind| self.provider_registry.get_factory(&kind))
            .is_some_and(|f| {
                matches!(f.category(), lr_providers::factory::ProviderCategory::Local)
            });
        let cfg = self.config_manager.get();
        let strategy = cfg
            .clients
            .iter()
            .find(|c| c.id == client)
            .and_then(|c| cfg.strategies.iter().find(|s| s.id == c.strategy_id))?;
        (local
            || self
                .assess_candidate(
                    strategy,
                    provider,
                    model,
                    &lr_providers::CompletionRequest::new(model, vec![]),
                    0,
                    lr_providers::EndpointType::Chat,
                )
                .pricing
                .is_some())
        .then_some(estimated)
        .filter(|n| n.is_finite() && *n >= 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> ResponseCacheConfig {
        ResponseCacheConfig {
            enabled: true,
            ttl_secs: 60,
            max_entries: 2,
            max_bytes: 4096,
        }
    }
    fn response(text: &str) -> CachedResponse {
        CachedResponse::Chat(serde_json::from_value(json!({"id":"original","object":"chat.completion","created":1,"model":"m","provider":"p","choices":[{"index":0,"message":{"role":"assistant","content":text},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}})).unwrap())
    }
    fn ticket(
        cache: &ResponseCache,
        client: &str,
        scope: &str,
        body: Value,
        config: Value,
        mode: CacheMode,
    ) -> CacheTicket {
        cache
            .ticket(client, scope, body, config, &settings(), mode)
            .unwrap()
    }
    #[test]
    fn exact_identity_includes_client_credential_config_endpoint_and_all_inputs() {
        let cache = ResponseCache::default();
        let body = json!({"endpoint":"chat","model":"p/m","messages":["hello"],"temperature":0.2});
        let original = ticket(
            &cache,
            "one",
            "credential-one",
            body.clone(),
            json!(1),
            CacheMode::Use,
        );
        assert!(cache.get(&original).is_none());
        assert!(cache.insert(original, response("Answer"), Some(0.02)));
        for (client, scope, config, mut request) in [
            ("two", "credential-one", json!(1), body.clone()),
            ("one", "credential-two", json!(1), body.clone()),
            ("one", "credential-one", json!(2), body.clone()),
            (
                "one",
                "credential-one",
                json!(1),
                json!({"endpoint":"responses","model":"p/m","messages":["hello"],"temperature":0.2}),
            ),
            ("one", "credential-one", json!(1), body.clone()),
        ] {
            if client == "one" && scope == "credential-one" && config == json!(1) && request == body
            {
                request["temperature"] = json!(0.8);
            }
            assert!(cache
                .get(&ticket(
                    &cache,
                    client,
                    scope,
                    request,
                    config,
                    CacheMode::Use
                ))
                .is_none());
        }
        let (_, receipt) = cache
            .get(&ticket(
                &cache,
                "one",
                "credential-one",
                body,
                json!(1),
                CacheMode::Use,
            ))
            .unwrap();
        assert_eq!(receipt["upstream_spend_usd"], 0.0);
        let stats = cache.stats(Some("one"));
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.upstream_requests_avoided, 1);
        assert_eq!(stats.avoided_cost_estimate_usd, Some(0.02));
    }
    #[test]
    fn clearing_does_not_resurrect_inflight_entries_or_clear_other_clients() {
        let cache = ResponseCache::default();
        let stale = ticket(&cache, "one", "key", json!(1), json!(1), CacheMode::Use);
        let other = ticket(&cache, "two", "key", json!(1), json!(1), CacheMode::Use);
        cache.insert(other, response("Other"), None);
        cache.clear(Some("one"));
        assert!(!cache.insert(stale, response("Stale"), None));
        assert!(cache
            .get(&ticket(
                &cache,
                "two",
                "key",
                json!(1),
                json!(1),
                CacheMode::Use
            ))
            .is_some());
        cache.clear(None);
        assert_eq!(cache.stats(None).entries, 0);
    }
    #[test]
    fn bounded_eviction_and_refresh_preserve_unknown_costs() {
        let cache = ResponseCache::default();
        for n in 0..3 {
            let t = ticket(&cache, "one", "key", json!(n), json!(1), CacheMode::Use);
            assert!(cache.insert(t, response("A"), None));
        }
        assert_eq!(cache.stats(None).entries, 2);
        assert_eq!(cache.stats(None).evictions, 1);
        assert!(cache
            .get(&ticket(
                &cache,
                "one",
                "key",
                json!(0),
                json!(1),
                CacheMode::Use
            ))
            .is_none());
        let refresh = ticket(&cache, "one", "key", json!(2), json!(1), CacheMode::Refresh);
        assert!(cache.get(&refresh).is_none());
        assert!(cache.insert(refresh, response("New"), None));
        assert!(cache
            .get(&ticket(
                &cache,
                "one",
                "key",
                json!(2),
                json!(1),
                CacheMode::Use
            ))
            .is_some());
        assert_eq!(cache.stats(None).avoided_cost_estimate_usd, None);
        let oversized = ticket(&cache, "one", "key", json!(4), json!(1), CacheMode::Use);
        assert!(!cache.insert(oversized, response(&"x".repeat(5000)), None));
        assert!(cache.stats(None).bytes <= 4096);
        assert!(cache
            .ticket(
                "one",
                "key",
                json!(1),
                json!(1),
                &settings(),
                CacheMode::Bypass
            )
            .is_none());
    }
    #[test]
    fn ttl_releases_payloads_and_refuses_expired_hits() {
        let cache = ResponseCache::default();
        let mut settings = settings();
        settings.ttl_secs = 1;
        let t = cache
            .ticket("one", "key", json!(1), json!(1), &settings, CacheMode::Use)
            .unwrap();
        cache.insert(t, response("Expires"), Some(0.0));
        std::thread::sleep(Duration::from_millis(1100));
        assert_eq!(cache.stats(None).entries, 0);
        assert!(cache
            .get(
                &cache
                    .ticket("one", "key", json!(1), json!(1), &settings, CacheMode::Use)
                    .unwrap()
            )
            .is_none());
    }
    #[test]
    fn unsafe_bound_dynamic_or_incomplete_outputs_are_not_cacheable() {
        for value in [
            json!({"previous_response_id":"old"}),
            json!({"type":"redacted_thinking","data":"opaque"}),
            json!({"web_search_options":{}}),
            json!({"tool_calls":[{}]}),
            json!({"thoughtSignature":"opaque"}),
        ] {
            assert!(unsafe_state(&value, 0));
        }
        let CachedResponse::Chat(mut text) = response("Text") else {
            unreachable!()
        };
        assert!(cacheable_chat_response(&text));
        text.choices[0].finish_reason = Some("length".into());
        assert!(!cacheable_chat_response(&text));
    }
}

impl crate::Router {
    fn cache_ticket_current(&self, ticket: &CacheTicket) -> bool {
        ticket.configuration_fingerprint
            == crate::explanation::fingerprint(
                json!({"configuration":self.config_manager.get().routing_identity(),"provider_revision":self.provider_registry.routing_revision()}),
            )
    }
    /// Called only by the HTTP success path after approvals and final checks.
    pub fn commit_chat_cache(&self, client: &str, response: &mut CompletionResponse) {
        let token = response
            .extensions
            .as_mut()
            .and_then(|extensions| extensions.remove(lr_providers::response_cache::PENDING))
            .and_then(|v| v.as_str().map(str::to_owned));
        if let Some((ticket, reference)) =
            token.and_then(|token| self.response_cache.take_pending(&token, client))
        {
            if self.cache_ticket_current(&ticket) && cacheable_chat_response(response) {
                self.response_cache.insert(
                    ticket,
                    CachedResponse::Chat(response.clone()),
                    reference,
                );
            }
        }
    }
    pub fn commit_decision_cache(&self, client: &str, response: &mut SystemOneResponse) {
        let token = response
            .extra
            .get_mut("extensions")
            .and_then(Value::as_object_mut)
            .and_then(|extensions| extensions.remove(lr_providers::response_cache::PENDING))
            .and_then(|v| v.as_str().map(str::to_owned));
        if let Some((ticket, reference)) =
            token.and_then(|token| self.response_cache.take_pending(&token, client))
        {
            if self.cache_ticket_current(&ticket)
                && serde_json::to_value(&response).is_ok_and(|value| !unsafe_state(&value, 0))
            {
                self.response_cache.insert(
                    ticket,
                    CachedResponse::Decision(response.clone()),
                    reference,
                );
            }
        }
    }
}

#[cfg(test)]
mod pending_tests {
    use super::*;
    #[test]
    fn an_upstream_success_is_not_a_cache_entry_until_the_http_success_path_commits() {
        let cache = ResponseCache::default();
        let settings = ResponseCacheConfig {
            enabled: true,
            ..Default::default()
        };
        let ticket = cache
            .ticket(
                "client",
                "credential",
                json!({"messages":["private input"]}),
                json!(1),
                &settings,
                CacheMode::Use,
            )
            .unwrap();
        let token = cache.defer(ticket, Some(0.02)).unwrap();
        assert_eq!(cache.stats(None).entries, 0);
        assert_eq!(cache.stats(None).bytes, 0);
        assert!(cache.take_pending(&token, "other-client").is_none());
        let ticket = cache
            .ticket(
                "client",
                "credential",
                json!({"messages":["private input"]}),
                json!(1),
                &settings,
                CacheMode::Use,
            )
            .unwrap();
        let token = cache.defer(ticket, None).unwrap();
        cache.clear(Some("client"));
        assert!(cache.take_pending(&token, "client").is_none());
        assert_eq!(cache.stats(None).entries, 0);
    }
}

#[cfg(test)]
mod native_envelope_tests {
    use super::*;
    #[test]
    fn even_text_only_native_responses_keep_their_provider_bound_item_identity_outside_the_cache() {
        let mut response:CompletionResponse=serde_json::from_value(json!({"id":"original","object":"chat.completion","created":1,"model":"m","provider":"p","choices":[{"index":0,"message":{"role":"assistant","content":"Text"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}})).unwrap();
        assert!(cacheable_chat_response(&response));
        response.extensions.get_or_insert_with(Default::default).insert(lr_providers::openai_responses::NATIVE_RESPONSES_API_EXT_KEY.into(),json!({"id":"resp_original","output":[{"id":"msg_provider_bound","type":"message","content":[{"type":"output_text","text":"Text"}]}]}));
        assert!(!cacheable_chat_response(&response));
    }
}
