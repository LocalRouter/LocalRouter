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

use crate::anthropic;
use crate::interceptor::{
    ClientCtx, ConnectDecision, ObservedExchange, PricingResolver, ProxyInterceptor, RequestAction,
    TokenUsage,
};

/// Passive interceptor: MITM allow-listed LLM hosts, record what it sees, and
/// forward everything unchanged.
pub struct PassiveInterceptor {
    monitor: Arc<MonitorEventStore>,
    /// Aggregate metrics sink (per-key/provider/model/strategy). Optional so the
    /// interceptor is still usable in tests without the metrics stack.
    metrics: Option<Arc<MetricsCollector>>,
    /// Resolves USD cost from model + token usage (via the catalog).
    pricing: Option<Arc<dyn PricingResolver>>,
}

impl PassiveInterceptor {
    pub fn new(monitor: Arc<MonitorEventStore>) -> Self {
        Self {
            monitor,
            metrics: None,
            pricing: None,
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

    /// Record a fully-observed exchange as one combined LLM-call monitor event.
    /// Shared with the active interceptor, which reuses this for recording.
    pub(crate) fn record(&self, ex: &ObservedExchange) {
        // Parse the raw request body as Anthropic JSON (best-effort).
        let request_json = ex
            .request_body
            .as_ref()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok());
        let req_meta = request_json
            .as_ref()
            .map(anthropic::parse_request)
            .unwrap_or_default();

        // The response is either a single JSON object or an SSE stream. For SSE
        // we reconstruct a full message body so it's captured like a plain one.
        let (resp_meta, response_json) = match &ex.response_body {
            Some(bytes) if ex.response_is_sse => {
                let raw = String::from_utf8_lossy(bytes);
                let (meta, body) = anthropic::reconstruct_sse(&raw);
                (meta, Some(body))
            }
            Some(bytes) => {
                let json = serde_json::from_slice::<serde_json::Value>(bytes).ok();
                let meta = json
                    .as_ref()
                    .map(anthropic::parse_response)
                    .unwrap_or_default();
                (meta, json)
            }
            None => (Default::default(), None),
        };

        // Raw wire payloads, capped, so the exact bytes are always inspectable
        // (this is what "captures everything" for streamed responses).
        let raw_request = ex
            .request_body
            .as_ref()
            .map(|b| cap_raw(&String::from_utf8_lossy(b)));
        let raw_response = ex
            .response_body
            .as_ref()
            .map(|b| cap_raw(&String::from_utf8_lossy(b)));

        let tool_count = request_json
            .as_ref()
            .and_then(|b| b.get("tools"))
            .and_then(|t| t.as_array())
            .map(Vec::len)
            .unwrap_or(0);

        let total_tokens = match (resp_meta.input_tokens, resp_meta.output_tokens) {
            (Some(i), Some(o)) => Some(i + o),
            _ => None,
        };

        let model = req_meta
            .model
            .clone()
            .or_else(|| resp_meta.model.clone())
            .unwrap_or_default();

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
            .and_then(|p| p.cost_usd(&model, usage));

        let status = match ex.status {
            Some(code) if code >= 400 => EventStatus::Error,
            Some(_) => EventStatus::Complete,
            None => EventStatus::Error,
        };

        // Feed aggregate metrics (per key/provider/model/strategy) so proxied
        // traffic shows in the dashboards, just like native calls.
        if let Some(metrics) = &self.metrics {
            if status == EventStatus::Complete {
                metrics.record_success(&RequestMetrics {
                    api_key_name: &ex.client_id,
                    provider: "anthropic",
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
                    "anthropic",
                    &model,
                    &ex.strategy_id,
                    ex.latency_ms.unwrap_or(0),
                );
            }
        }

        let data = MonitorEventData::LlmCall {
            endpoint: ex.path.clone(),
            model,
            stream: req_meta.stream,
            message_count: req_meta.message_count,
            has_tools: req_meta.has_tools,
            tool_count,
            request_body: request_json.clone().unwrap_or(serde_json::Value::Null),
            source: LlmCallSource::Proxy,
            protocol: LlmProtocol::Anthropic,
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
            finish_reason: resp_meta.stop_reason,
            content_preview: resp_meta.content_preview,
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
            None,
            None,
            data,
            status,
            ex.latency_ms,
        );
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
        // Passive: nothing to rewrite. Recording happens once, on response,
        // so the monitor event carries both halves of the exchange.
        RequestAction::Forward
    }

    async fn on_response(&self, exchange: &ObservedExchange) {
        // Only record exchanges we actually parsed as Anthropic Messages calls.
        if anthropic::is_messages_path(&exchange.path) {
            self.record(exchange);
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
