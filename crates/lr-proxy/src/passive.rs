//! The v1 inspect-only interceptor.
//!
//! Decides which hosts to MITM, and records each decrypted LLM exchange to the
//! monitor as a single combined event — without ever modifying the traffic.
//! Active rewriting is a future implementation of the same
//! [`ProxyInterceptor`](crate::interceptor::ProxyInterceptor) trait.

use std::sync::Arc;

use async_trait::async_trait;
use lr_monitor::{
    EventStatus, LlmCallSource, LlmProtocol, MonitorEventData, MonitorEventStore, MonitorEventType,
};
use lr_monitoring::metrics::{MetricsCollector, RequestMetrics};

use crate::interceptor::{
    ClientCtx, ClientNameResolver, ConnectDecision, ObservedExchange, PricingResolver,
    ProxyInterceptor, RequestAction, TokenUsage,
};
use crate::wire::{self, RequestMeta, ResponseMeta, WireFormat};

/// Passive interceptor: MITM allow-listed LLM hosts, record what it sees, and
/// forward everything unchanged.
pub struct PassiveInterceptor {
    monitor: Arc<MonitorEventStore>,
    /// Aggregate metrics sink (per-key/provider/model/strategy). Optional so the
    /// interceptor is still usable in tests without the metrics stack.
    metrics: Option<Arc<MetricsCollector>>,
    /// Resolves USD cost from model + token usage (via the catalog).
    pricing: Option<Arc<dyn PricingResolver>>,
    /// Resolves the client's display name so Monitor shows a name, not a UUID.
    client_names: Option<Arc<dyn ClientNameResolver>>,
}

impl PassiveInterceptor {
    pub fn new(monitor: Arc<MonitorEventStore>) -> Self {
        Self {
            monitor,
            metrics: None,
            pricing: None,
            client_names: None,
        }
    }

    /// Attach the metrics collector so proxied calls feed the dashboards.
    pub fn with_metrics(mut self, metrics: Arc<MetricsCollector>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Attach a pricing resolver so proxied calls get a cost.
    pub fn with_pricing(mut self, pricing: Arc<dyn PricingResolver>) -> Self {
        self.pricing = Some(pricing);
        self
    }

    /// Attach a client-name resolver so proxied events are attributed by name.
    pub fn with_client_names(mut self, names: Arc<dyn ClientNameResolver>) -> Self {
        self.client_names = Some(names);
        self
    }

    /// The client's display name for monitor attribution, if resolvable.
    fn client_name(&self, client_id: &str) -> Option<String> {
        self.client_names.as_ref()?.name_for(client_id)
    }

    /// Parse the request half into the pieces a monitor event needs.
    fn request_parts(
        format: WireFormat,
        ex: &ObservedExchange,
    ) -> (
        RequestMeta,
        Option<serde_json::Value>,
        Option<String>,
        usize,
    ) {
        let request_json = ex
            .request_body
            .as_ref()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok());
        let req_meta = request_json
            .as_ref()
            .map(|b| wire::parse_request(format, b))
            .unwrap_or_default();
        // Raw wire payload, capped, so the exact bytes are always inspectable.
        let raw_request = ex
            .request_body
            .as_ref()
            .map(|b| cap_raw(&String::from_utf8_lossy(b)));
        let tool_count = request_json
            .as_ref()
            .and_then(|b| b.get("tools"))
            .and_then(|t| t.as_array())
            .map(Vec::len)
            .unwrap_or(0);
        (req_meta, request_json, raw_request, tool_count)
    }

    /// Parse the response half (single JSON object, or a reconstructed SSE
    /// stream) into the pieces a monitor event needs.
    fn response_parts(
        format: WireFormat,
        ex: &ObservedExchange,
    ) -> (ResponseMeta, Option<serde_json::Value>, Option<String>) {
        // For SSE we reconstruct a full message body so it's captured like a
        // plain one.
        let (resp_meta, response_json) = match &ex.response_body {
            Some(bytes) if ex.response_is_sse => {
                let raw = String::from_utf8_lossy(bytes);
                let (meta, body) = wire::reconstruct_sse(format, &raw);
                (meta, Some(body))
            }
            Some(bytes) => {
                let json = serde_json::from_slice::<serde_json::Value>(bytes).ok();
                let meta = json
                    .as_ref()
                    .map(|b| wire::parse_response(format, b))
                    .unwrap_or_default();
                (meta, json)
            }
            None => (Default::default(), None),
        };
        let raw_response = ex
            .response_body
            .as_ref()
            .map(|b| cap_raw(&String::from_utf8_lossy(b)));
        (resp_meta, response_json, raw_response)
    }

    /// Compute model/usage/cost/status for a fully-observed exchange and feed
    /// aggregate metrics (per key/provider/model/strategy) so proxied traffic
    /// shows in the dashboards, just like native calls.
    fn finalize(
        &self,
        ex: &ObservedExchange,
        req_meta: &RequestMeta,
        resp_meta: &ResponseMeta,
    ) -> (String, Option<f64>, Option<u64>, EventStatus) {
        let total_tokens = match (resp_meta.input_tokens, resp_meta.output_tokens) {
            (Some(i), Some(o)) => Some(i + o),
            _ => None,
        };
        let model = req_meta
            .model
            .clone()
            .or_else(|| resp_meta.model.clone())
            .unwrap_or_default();
        let provider = wire::provider_for_host(&ex.host);

        // Cost from the catalog, including cache-write/cache-read/reasoning tokens.
        let usage = TokenUsage {
            input: resp_meta.input_tokens.unwrap_or(0),
            output: resp_meta.output_tokens.unwrap_or(0),
            cache_write: resp_meta.cache_creation_tokens.unwrap_or(0),
            cache_read: resp_meta.cache_read_tokens.unwrap_or(0),
            reasoning: resp_meta.reasoning_tokens.unwrap_or(0),
        };
        let cost_usd = self
            .pricing
            .as_ref()
            .and_then(|p| p.cost_usd(provider, &model, usage));

        let status = match ex.status {
            Some(code) if code >= 400 => EventStatus::Error,
            Some(_) => EventStatus::Complete,
            None => EventStatus::Error,
        };

        if let Some(metrics) = &self.metrics {
            if status == EventStatus::Complete {
                metrics.record_success(&RequestMetrics {
                    api_key_name: &ex.client_id,
                    provider,
                    model: &model,
                    strategy_id: &ex.strategy_id,
                    input_tokens: usage.input,
                    output_tokens: usage.output,
                    cost_usd: cost_usd.unwrap_or(0.0),
                    latency_ms: ex.latency_ms.unwrap_or(0),
                });
            } else {
                metrics.record_failure(
                    &ex.client_id,
                    provider,
                    &model,
                    &ex.strategy_id,
                    ex.latency_ms.unwrap_or(0),
                );
            }
        }

        (model, cost_usd, total_tokens, status)
    }

    /// Open a Pending monitor event at request time so the exchange is visible
    /// while the upstream call is in flight. Returns the event id, which the
    /// transport threads back so [`complete`](Self::complete) fills in the
    /// response half. Only recognized LLM API calls are recorded.
    pub(crate) fn emit_pending(&self, ex: &ObservedExchange) -> Option<String> {
        let format = wire::detect(&ex.path)?;
        let (req_meta, request_json, raw_request, tool_count) = Self::request_parts(format, ex);
        let model = req_meta.model.clone().unwrap_or_default();

        let data = MonitorEventData::LlmCall {
            endpoint: ex.path.clone(),
            model,
            stream: req_meta.stream,
            message_count: req_meta.message_count,
            has_tools: req_meta.has_tools,
            tool_count,
            request_body: request_json.unwrap_or(serde_json::Value::Null),
            source: LlmCallSource::Proxy,
            protocol: protocol_for(format),
            transformed_body: None,
            transformations_applied: None,
            provider: Some(ex.host.clone()),
            status_code: None,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            reasoning_tokens: None,
            cost_usd: None,
            latency_ms: None,
            finish_reason: None,
            content_preview: None,
            streamed: Some(req_meta.stream),
            response_body: None,
            raw_request,
            raw_response: None,
            error: None,
            routing_info: None,
        };

        Some(self.monitor.push(
            MonitorEventType::LlmCall,
            Some(ex.client_id.clone()),
            self.client_name(&ex.client_id),
            None,
            data,
            EventStatus::Pending,
            None,
        ))
    }

    /// Fill in the response half of a previously-opened Pending event
    /// (pending → complete/error), recording cost + metrics.
    pub(crate) fn complete(&self, event_id: &str, ex: &ObservedExchange) {
        let Some(format) = wire::detect(&ex.path) else {
            return;
        };
        let (req_meta, _request_json, _raw_request, _tool_count) = Self::request_parts(format, ex);
        let (resp_meta, response_json, raw_response) = Self::response_parts(format, ex);
        let (model, cost_usd, total_tokens, status) = self.finalize(ex, &req_meta, &resp_meta);

        self.monitor.update(event_id, |event| {
            event.status = status;
            event.duration_ms = ex.latency_ms;
            if let MonitorEventData::LlmCall {
                model: m,
                status_code: sc,
                input_tokens: it,
                output_tokens: ot,
                total_tokens: tt,
                reasoning_tokens: rt,
                cost_usd: cu,
                latency_ms: lm,
                finish_reason: fr,
                content_preview: cp,
                response_body: rb,
                raw_response: rr,
                ..
            } = &mut event.data
            {
                *m = model;
                *sc = ex.status;
                *it = resp_meta.input_tokens;
                *ot = resp_meta.output_tokens;
                *tt = total_tokens;
                *rt = resp_meta.reasoning_tokens;
                *cu = cost_usd;
                *lm = ex.latency_ms;
                *fr = resp_meta.stop_reason.clone();
                *cp = resp_meta.content_preview.clone();
                *rb = response_json;
                *rr = raw_response;
            }
        });
    }

    /// Record a fully-observed exchange as one combined event in a single push.
    /// Used when there is no pre-opened pending event (e.g. a firewall reject
    /// synthesizes its response without ever contacting the upstream).
    pub(crate) fn record(&self, ex: &ObservedExchange) {
        let Some(format) = wire::detect(&ex.path) else {
            return;
        };
        let (req_meta, request_json, raw_request, tool_count) = Self::request_parts(format, ex);
        let (resp_meta, response_json, raw_response) = Self::response_parts(format, ex);
        let (model, cost_usd, total_tokens, status) = self.finalize(ex, &req_meta, &resp_meta);

        let data = MonitorEventData::LlmCall {
            endpoint: ex.path.clone(),
            model,
            stream: req_meta.stream,
            message_count: req_meta.message_count,
            has_tools: req_meta.has_tools,
            tool_count,
            request_body: request_json.unwrap_or(serde_json::Value::Null),
            source: LlmCallSource::Proxy,
            protocol: protocol_for(format),
            transformed_body: None,
            transformations_applied: None,
            provider: Some(ex.host.clone()),
            status_code: ex.status,
            input_tokens: resp_meta.input_tokens,
            output_tokens: resp_meta.output_tokens,
            total_tokens,
            reasoning_tokens: resp_meta.reasoning_tokens,
            cost_usd,
            latency_ms: ex.latency_ms,
            finish_reason: resp_meta.stop_reason.clone(),
            content_preview: resp_meta.content_preview.clone(),
            streamed: Some(req_meta.stream),
            response_body: response_json,
            raw_request,
            raw_response,
            error: None,
            routing_info: None,
        };

        self.monitor.push(
            MonitorEventType::LlmCall,
            Some(ex.client_id.clone()),
            self.client_name(&ex.client_id),
            None,
            data,
            status,
            ex.latency_ms,
        );
    }
}

/// Monitor protocol marker for a wire format.
fn protocol_for(format: WireFormat) -> LlmProtocol {
    match format {
        WireFormat::AnthropicMessages => LlmProtocol::Anthropic,
        WireFormat::OpenAiChat | WireFormat::OpenAiResponses => LlmProtocol::Openai,
    }
}

/// Cap on the raw payload stored per event, so the in-memory monitor ring
/// buffer stays bounded even for large exchanges.
const RAW_CAP: usize = 256 * 1024;

/// Truncate a raw payload to the cap (on a char boundary), appending a marker.
fn cap_raw(s: &str) -> String {
    if s.len() <= RAW_CAP {
        return s.to_string();
    }
    let mut end = RAW_CAP;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… [truncated {} bytes]", &s[..end], s.len() - end)
}

#[async_trait]
impl ProxyInterceptor for PassiveInterceptor {
    fn on_connect(&self, host: &str, client: &ClientCtx) -> ConnectDecision {
        if !client.proxy_enabled {
            return ConnectDecision::Reject("client is not in a proxy mode");
        }
        if crate::should_mitm_host(host) {
            ConnectDecision::Mitm
        } else {
            // Non-LLM hosts (auth, telemetry, everything else) are never decrypted.
            ConnectDecision::Tunnel
        }
    }

    async fn on_request(&self, _exchange: &ObservedExchange) -> RequestAction {
        // Passive: nothing to rewrite.
        RequestAction::Forward
    }

    fn begin(&self, exchange: &ObservedExchange) -> Option<String> {
        // Open a pending event as the request is forwarded, so it's visible in
        // the monitor while the upstream call is in flight.
        self.emit_pending(exchange)
    }

    async fn on_response(&self, exchange: &ObservedExchange) {
        match &exchange.event_id {
            // Complete the pending event opened in `begin`.
            Some(id) => self.complete(id, exchange),
            // No pending event (e.g. a firewall reject) — record in one push.
            // (`record` itself ignores paths that aren't recognized LLM calls.)
            None => self.record(exchange),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn exchange() -> ObservedExchange {
        let req = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"name": "t"}]
        });
        let resp = json!({
            "content": [{"type": "text", "text": "hello"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 5, "output_tokens": 3}
        });
        ObservedExchange {
            client_id: "client-1".to_string(),
            host: "api.anthropic.com".to_string(),
            method: "POST".to_string(),
            path: "/v1/messages".to_string(),
            request_body: Some(serde_json::to_vec(&req).unwrap()),
            status: Some(200),
            response_body: Some(serde_json::to_vec(&resp).unwrap()),
            response_is_sse: false,
            ..Default::default()
        }
    }

    struct StaticNames;

    impl ClientNameResolver for StaticNames {
        fn name_for(&self, client_id: &str) -> Option<String> {
            (client_id == "client-1").then(|| "Claude Code".to_string())
        }
    }

    #[test]
    fn recorded_events_carry_the_client_name() {
        // Monitor attributes proxy traffic by name, not the raw client UUID.
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone()).with_client_names(Arc::new(StaticNames));

        it.record(&exchange());
        let pending_id = it.emit_pending(&exchange()).expect("pending event");

        let events = store.list(0, 10, None);
        assert_eq!(events.events.len(), 2);
        for e in &events.events {
            assert_eq!(e.client_name.as_deref(), Some("Claude Code"));
        }
        assert!(events.events.iter().any(|e| e.id == pending_id));
    }

    #[test]
    fn client_name_is_none_without_a_resolver() {
        // No resolver attached (tests, and any embedder that doesn't wire one).
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone());

        it.record(&exchange());

        let events = store.list(0, 10, None);
        assert_eq!(events.events.len(), 1);
        assert_eq!(events.events[0].client_name, None);
    }

    #[test]
    fn unknown_client_id_falls_back_to_no_name() {
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone()).with_client_names(Arc::new(StaticNames));

        let mut ex = exchange();
        ex.client_id = "not-a-known-client".to_string();
        it.record(&ex);

        let events = store.list(0, 10, None);
        assert_eq!(events.events.len(), 1);
        assert_eq!(events.events[0].client_name, None);
    }

    #[test]
    fn on_connect_mitm_tunnel_reject() {
        let it = PassiveInterceptor::new(Arc::new(MonitorEventStore::new(16)));
        let enabled = ClientCtx {
            client_id: "c".into(),
            proxy_enabled: true,
            ..Default::default()
        };
        let disabled = ClientCtx {
            client_id: "c".into(),
            proxy_enabled: false,
            ..Default::default()
        };
        assert_eq!(
            it.on_connect("api.anthropic.com", &enabled),
            ConnectDecision::Mitm
        );
        assert_eq!(
            it.on_connect("claude.ai", &enabled),
            ConnectDecision::Tunnel
        );
        assert!(matches!(
            it.on_connect("api.anthropic.com", &disabled),
            ConnectDecision::Reject(_)
        ));
    }

    #[tokio::test]
    async fn records_messages_exchange_to_monitor() {
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone());

        let ex = exchange();
        let _ = it.on_request(&ex).await;
        let _ = it.on_response(&ex).await;

        let resp = store.list(0, 100, None);
        assert_eq!(
            resp.events.len(),
            1,
            "one combined event should be recorded"
        );
    }

    #[tokio::test]
    async fn begin_opens_pending_then_response_completes_it() {
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone());

        // begin() opens exactly one Pending event.
        let base = ObservedExchange {
            path: "/v1/messages".to_string(),
            request_body: exchange().request_body,
            ..Default::default()
        };
        let id = it
            .begin(&base)
            .expect("messages path opens a pending event");
        let pending = store.get(&id).expect("pending event exists");
        assert_eq!(pending.status, EventStatus::Pending);

        // The response half completes that same event (no second event).
        let mut done = exchange();
        done.event_id = Some(id.clone());
        it.on_response(&done).await;

        let resp = store.list(0, 100, None);
        assert_eq!(resp.events.len(), 1, "pending event is completed in place");
        let completed = store.get(&id).expect("event still present");
        assert_eq!(completed.status, EventStatus::Complete);
    }

    #[tokio::test]
    async fn begin_ignores_non_messages_paths() {
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone());
        let mut ex = exchange();
        ex.path = "/v1/complete".to_string();
        assert!(it.begin(&ex).is_none(), "non-messages path opens no event");
        assert!(store.list(0, 100, None).events.is_empty());
    }

    #[tokio::test]
    async fn ignores_non_messages_paths() {
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone());

        let mut ex = exchange();
        ex.path = "/v1/complete".to_string();
        let _ = it.on_response(&ex).await;

        let resp = store.list(0, 100, None);
        assert!(
            resp.events.is_empty(),
            "non-messages paths must not be recorded"
        );
    }
}
