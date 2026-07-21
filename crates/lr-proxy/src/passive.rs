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

use crate::anthropic;
use crate::interceptor::{
    ClientCtx, ConnectDecision, InterceptAction, ObservedExchange, ProxyInterceptor,
};

/// Passive interceptor: MITM allow-listed LLM hosts, record what it sees, and
/// forward everything unchanged.
pub struct PassiveInterceptor {
    monitor: Arc<MonitorEventStore>,
}

impl PassiveInterceptor {
    pub fn new(monitor: Arc<MonitorEventStore>) -> Self {
        Self { monitor }
    }

    /// Record a fully-observed exchange as one combined LLM-call monitor event.
    fn record(&self, ex: &ObservedExchange) {
        let req_meta = ex
            .request_body
            .as_ref()
            .map(anthropic::parse_request)
            .unwrap_or_default();

        let resp_meta = ex
            .response_body
            .as_ref()
            .map(anthropic::parse_response)
            .unwrap_or_default();

        let tool_count = ex
            .request_body
            .as_ref()
            .and_then(|b| b.get("tools"))
            .and_then(|t| t.as_array())
            .map(Vec::len)
            .unwrap_or(0);

        let total_tokens = match (resp_meta.input_tokens, resp_meta.output_tokens) {
            (Some(i), Some(o)) => Some(i + o),
            _ => None,
        };

        let status = match ex.status {
            Some(code) if code >= 400 => EventStatus::Error,
            Some(_) => EventStatus::Complete,
            None => EventStatus::Error,
        };

        let data = MonitorEventData::LlmCall {
            endpoint: ex.path.clone(),
            model: req_meta.model.clone().unwrap_or_default(),
            stream: req_meta.stream,
            message_count: req_meta.message_count,
            has_tools: req_meta.has_tools,
            tool_count,
            request_body: ex.request_body.clone().unwrap_or(serde_json::Value::Null),
            source: LlmCallSource::Proxy,
            protocol: LlmProtocol::Anthropic,
            transformed_body: None,
            transformations_applied: None,
            provider: Some(ex.host.clone()),
            status_code: ex.status,
            input_tokens: resp_meta.input_tokens,
            output_tokens: resp_meta.output_tokens,
            total_tokens,
            reasoning_tokens: None,
            cost_usd: None,
            latency_ms: None,
            finish_reason: resp_meta.stop_reason,
            content_preview: resp_meta.content_preview,
            streamed: Some(req_meta.stream),
            response_body: ex.response_body.clone(),
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
            None,
        );
    }
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

    async fn on_request(&self, _exchange: &ObservedExchange) -> InterceptAction<()> {
        // Passive: nothing to rewrite. Recording happens once, on response,
        // so the monitor event carries both halves of the exchange.
        InterceptAction::Forward
    }

    async fn on_response(&self, exchange: &ObservedExchange) -> InterceptAction<()> {
        // Only record exchanges we actually parsed as Anthropic Messages calls.
        if anthropic::is_messages_path(&exchange.path) {
            self.record(exchange);
        }
        InterceptAction::Forward
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn exchange() -> ObservedExchange {
        ObservedExchange {
            client_id: "client-1".to_string(),
            host: "api.anthropic.com".to_string(),
            method: "POST".to_string(),
            path: "/v1/messages".to_string(),
            request_body: Some(json!({
                "model": "claude-sonnet-4-20250514",
                "messages": [{"role": "user", "content": "hi"}],
                "tools": [{"name": "t"}]
            })),
            status: Some(200),
            response_body: Some(json!({
                "content": [{"type": "text", "text": "hello"}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 5, "output_tokens": 3}
            })),
        }
    }

    #[test]
    fn on_connect_mitm_tunnel_reject() {
        let it = PassiveInterceptor::new(Arc::new(MonitorEventStore::new(16)));
        let enabled = ClientCtx {
            client_id: "c".into(),
            proxy_enabled: true,
        };
        let disabled = ClientCtx {
            client_id: "c".into(),
            proxy_enabled: false,
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
