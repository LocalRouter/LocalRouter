//! Active interceptor: applies the firewall to each proxied LLM request
//! (forward / rewrite / reject), then records the exchange like the passive one.
//!
//! The policy itself lives in the app (config-driven rules, model enforcement,
//! and interactive approval), reached through the [`Firewall`] trait so this
//! crate stays free of config/UI concerns.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::anthropic;
use crate::interceptor::{
    ClientCtx, ConnectDecision, ObservedExchange, ProxyInterceptor, RequestAction,
};
use crate::passive::PassiveInterceptor;

/// A parsed proxied request handed to the firewall for a decision.
pub struct FirewallRequest {
    pub client_id: String,
    pub host: String,
    pub path: String,
    /// Requested model, if present in the body.
    pub model: Option<String>,
    pub has_tools: bool,
    pub message_count: usize,
    /// Parsed request JSON — for content matching and for rewriting (return a
    /// `RequestAction::Replace` with a mutated body).
    pub body: Value,
}

/// The firewall decides what happens to a proxied LLM request. Implemented by
/// the app: config-driven rules, model allow-list enforcement, model rewrites,
/// and interactive approval.
#[async_trait]
pub trait Firewall: Send + Sync {
    async fn evaluate(&self, req: &FirewallRequest) -> RequestAction;
}

/// Interceptor that runs the firewall on requests and records on responses.
pub struct ActiveInterceptor {
    recorder: PassiveInterceptor,
    firewall: Arc<dyn Firewall>,
}

impl ActiveInterceptor {
    pub fn new(recorder: PassiveInterceptor, firewall: Arc<dyn Firewall>) -> Self {
        Self { recorder, firewall }
    }
}

#[async_trait]
impl ProxyInterceptor for ActiveInterceptor {
    fn on_connect(&self, host: &str, client: &ClientCtx) -> ConnectDecision {
        if !client.proxy_enabled {
            return ConnectDecision::Reject("client is not in a proxy mode");
        }
        if crate::should_mitm_host(host) {
            ConnectDecision::Mitm
        } else {
            ConnectDecision::Tunnel
        }
    }

    async fn on_request(&self, ex: &ObservedExchange) -> RequestAction {
        // Only the Anthropic Messages endpoint is subject to the firewall;
        // anything else (auth preflights, etc.) passes through untouched.
        if !anthropic::is_messages_path(&ex.path) {
            return RequestAction::Forward;
        }
        let body: Value = ex
            .request_body
            .as_ref()
            .and_then(|b| serde_json::from_slice(b).ok())
            .unwrap_or(Value::Null);
        let meta = anthropic::parse_request(&body);
        let req = FirewallRequest {
            client_id: ex.client_id.clone(),
            host: ex.host.clone(),
            path: ex.path.clone(),
            model: meta.model,
            has_tools: meta.has_tools,
            message_count: meta.message_count,
            body,
        };
        self.firewall.evaluate(&req).await
    }

    async fn on_response(&self, ex: &ObservedExchange) {
        if anthropic::is_messages_path(&ex.path) {
            self.recorder.record(ex);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_monitor::MonitorEventStore;

    struct MockFirewall(fn(&FirewallRequest) -> RequestAction);
    #[async_trait]
    impl Firewall for MockFirewall {
        async fn evaluate(&self, req: &FirewallRequest) -> RequestAction {
            (self.0)(req)
        }
    }

    fn interceptor(f: fn(&FirewallRequest) -> RequestAction) -> ActiveInterceptor {
        let recorder = PassiveInterceptor::new(Arc::new(MonitorEventStore::new(8)));
        ActiveInterceptor::new(recorder, Arc::new(MockFirewall(f)))
    }

    fn messages_exchange() -> ObservedExchange {
        ObservedExchange {
            path: "/v1/messages".to_string(),
            request_body: Some(
                serde_json::to_vec(&serde_json::json!({"model": "claude-x", "messages": []}))
                    .unwrap(),
            ),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn firewall_decides_messages_requests() {
        // Deny → Reject.
        let it = interceptor(|_| RequestAction::reject_json(403, "no"));
        assert!(matches!(
            it.on_request(&messages_exchange()).await,
            RequestAction::Reject { status: 403, .. }
        ));

        // Rewrite → Replace, and the firewall saw the parsed model.
        let it = interceptor(|req| {
            assert_eq!(req.model.as_deref(), Some("claude-x"));
            RequestAction::Replace(b"rewritten".to_vec())
        });
        assert!(matches!(
            it.on_request(&messages_exchange()).await,
            RequestAction::Replace(_)
        ));
    }

    #[tokio::test]
    async fn non_messages_paths_bypass_the_firewall() {
        // A firewall that would deny is never consulted for non-messages paths.
        let it = interceptor(|_| RequestAction::reject_json(403, "no"));
        let mut ex = messages_exchange();
        ex.path = "/v1/complete".to_string();
        assert!(matches!(it.on_request(&ex).await, RequestAction::Forward));
    }
}
