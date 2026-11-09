//! Active interceptor: applies the firewall to each proxied LLM request
//! (forward / rewrite / reject), then records the exchange like the passive one.
//!
//! The policy itself lives in the app (config-driven rules, model enforcement,
//! and interactive approval), reached through the [`Firewall`] trait so this
//! crate stays free of config/UI concerns.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::interceptor::{
    ClientCtx, ConnectDecision, ObservedExchange, PassthroughExchange, ProxyInterceptor,
    RequestAction,
};
use crate::passive::PassiveInterceptor;
use crate::wire;

/// A parsed proxied request handed to the firewall for a decision.
pub struct FirewallRequest {
    pub client_id: String,
    pub session_id: Option<String>,
    pub secret_session: Option<String>,
    pub host: String,
    pub method: String,
    pub path: String,
    /// Catalog/metrics provider name derived from the host ("anthropic", "openai").
    pub provider: String,
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
    async fn evaluate_uninspectable(
        &self,
        _req: &FirewallRequest,
        error: crate::request_body::DecodeError,
    ) -> RequestAction {
        RequestAction::reject_json(error.status(), error.message())
    }
    fn secret_inspection_warning(&self, _exchange: &ObservedExchange, _message: &str) {}
    fn protects_secrets(&self, _client_id: &str) -> bool {
        false
    }
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
        // A request an earlier LocalRouter hop already handled was already
        // judged (and possibly rewritten) there — pass it through verbatim.
        if ex.is_duplicate_hop() && !self.firewall.protects_secrets(&ex.client_id) {
            return RequestAction::Forward;
        }
        // Only recognized LLM API calls are subject to the firewall; anything
        // else (auth preflights, etc.) passes through untouched.
        let Some(format) = wire::detect(&ex.path) else {
            let path = ex.path.split('?').next().unwrap_or(&ex.path);
            let inference = [
                "/audio",
                "/images",
                "/realtime",
                "/videos",
                "/batches",
                "/embeddings",
                "/moderations",
                "/files",
                "/rerank",
                "/ocr",
                "/search",
                "/conversation",
                "/responses",
            ]
            .iter()
            .any(|part| path.contains(part));
            if self.firewall.protects_secrets(&ex.client_id)
                && inference
                && (ex.method == "POST" || path.contains("/realtime"))
            {
                self.firewall.secret_inspection_warning(ex,"Secret Scan has no inspection mapping for this endpoint; request forwarded unchanged");
            }
            return RequestAction::Forward;
        };
        let body: Value = ex
            .request_body
            .as_ref()
            .and_then(|b| serde_json::from_slice(b).ok())
            .unwrap_or(Value::Null);
        let meta = wire::parse_request(format, &body);
        let req = FirewallRequest {
            client_id: ex.client_id.clone(),
            session_id: ex.session_id.clone(),
            secret_session: ex.secret_session.clone(),
            host: ex.host.clone(),
            method: ex.method.clone(),
            path: ex.path.clone(),
            provider: ex
                .provider_override
                .clone()
                .unwrap_or_else(|| wire::provider_for_host(&ex.host).to_string()),
            model: meta.model,
            has_tools: meta.has_tools,
            message_count: meta.message_count,
            body,
        };
        if let Some(error) = ex.request_body_error {
            self.firewall.evaluate_uninspectable(&req, error).await
        } else {
            self.firewall.evaluate(&req).await
        }
    }

    fn begin(&self, ex: &ObservedExchange) -> Option<String> {
        // Open a pending event once the firewall allows the request through.
        self.recorder.emit_pending(ex)
    }

    async fn on_response(&self, ex: &ObservedExchange) {
        match &ex.event_id {
            Some(id) => self.recorder.complete(id, ex),
            // `record` ignores paths that aren't recognized LLM calls.
            None => self.recorder.record(ex),
        }
    }

    fn begin_passthrough(&self, ex: &PassthroughExchange) -> Option<String> {
        // Non-LLM traffic is never subject to the firewall — it is forwarded
        // untouched and only recorded so the user can see it happened.
        self.recorder.begin_passthrough(ex)
    }

    fn end_passthrough(&self, event_id: Option<String>, ex: &PassthroughExchange) {
        self.recorder.end_passthrough(event_id, ex);
    }
}

#[async_trait]
impl crate::reverse::ReverseRecorder for ActiveInterceptor {
    async fn on_request(&self, ex: &crate::reverse::ReverseExchange) -> RequestAction {
        ProxyInterceptor::on_request(self, &crate::passive::observed_from_reverse(ex)).await
    }
    fn begin(&self, ex: &crate::reverse::ReverseExchange) -> Option<String> {
        crate::reverse::ReverseRecorder::begin(&self.recorder, ex)
    }
    async fn record(&self, ex: crate::reverse::ReverseExchange) {
        crate::reverse::ReverseRecorder::record(&self.recorder, ex).await;
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

    struct RecordingFirewall(Arc<MonitorEventStore>);
    #[async_trait]
    impl Firewall for RecordingFirewall {
        async fn evaluate(&self, req: &FirewallRequest) -> RequestAction {
            self.0.push(
                lr_monitor::MonitorEventType::FirewallDecision,
                Some(req.client_id.clone()),
                None,
                req.session_id.clone(),
                lr_monitor::MonitorEventData::FirewallDecision {
                    firewall_type: "secret_scan".into(),
                    item_name: "synthetic".into(),
                    action: "AllowOnce".into(),
                    duration: None,
                },
                lr_monitor::EventStatus::Complete,
                None,
            );
            RequestAction::Forward
        }
    }

    #[tokio::test]
    async fn decisions_before_capture_receive_the_parent_conversation() {
        let store = Arc::new(MonitorEventStore::new(20));
        let it = ActiveInterceptor::new(
            PassiveInterceptor::new(store.clone()),
            Arc::new(RecordingFirewall(store.clone())),
        );
        for session in ["first", "second"] {
            let mut ex = messages_exchange();
            ex.client_id = "client".into();
            ex.session_id = Some(session.into());
            ex.request_body = Some(
                serde_json::to_vec(
                    &serde_json::json!({"messages":[{"role":"user","content":"Hello"}]}),
                )
                .unwrap(),
            );
            it.on_request(&ex).await;
            ex.event_id = it.begin(&ex);
            ex.status = Some(200);
            it.on_response(&ex).await;
        }
        let events = store.list(0, 20, None).events;
        assert_eq!(events.len(), 4);
        let first = events
            .iter()
            .find(|e| e.session_id.as_deref() == Some("first"))
            .unwrap()
            .conversation
            .as_ref()
            .unwrap();
        let second = events
            .iter()
            .find(|e| e.session_id.as_deref() == Some("second"))
            .unwrap()
            .conversation
            .as_ref()
            .unwrap();
        assert_ne!(first.id, second.id);
        for event in events.iter() {
            let expected = if event.session_id.as_deref() == Some("first") {
                first
            } else {
                second
            };
            assert_eq!(event.conversation.as_ref().unwrap().id, expected.id);
        }
    }

    #[tokio::test]
    async fn non_messages_paths_bypass_the_firewall() {
        // A firewall that would deny is never consulted for non-messages paths.
        let it = interceptor(|_| RequestAction::reject_json(403, "no"));
        let mut ex = messages_exchange();
        ex.path = "/v1/complete".to_string();
        assert!(matches!(it.on_request(&ex).await, RequestAction::Forward));
    }

    #[tokio::test]
    async fn forged_wire_trace_does_not_bypass_the_firewall() {
        let it = interceptor(|_| RequestAction::reject_json(403, "no"));
        let mut ex = messages_exchange();
        let mut headers = hyper::HeaderMap::new();
        headers.insert(lr_types::TRACE_HEADER, "forged;hop=99".parse().unwrap());
        ex.trace = crate::stamp_trace(&mut headers, true);
        assert!(matches!(
            it.on_request(&ex).await,
            RequestAction::Reject { status: 403, .. }
        ));
    }

    #[tokio::test]
    async fn duplicate_hops_bypass_the_firewall() {
        // A denying firewall is never consulted for a request an earlier
        // LocalRouter hop already handled.
        let it = interceptor(|_| RequestAction::reject_json(403, "no"));
        let mut ex = messages_exchange();
        ex.trace = Some(lr_types::RequestTrace::parse("abc;hop=2").unwrap());
        assert!(matches!(it.on_request(&ex).await, RequestAction::Forward));

        // The first hop is still judged.
        ex.trace = Some(lr_types::RequestTrace::parse("abc;hop=1").unwrap());
        assert!(matches!(
            it.on_request(&ex).await,
            RequestAction::Reject { status: 403, .. }
        ));
    }
}

#[cfg(test)]
mod protected_endpoint_tests {
    use super::*;
    struct Protected(std::sync::atomic::AtomicUsize);
    #[async_trait]
    impl Firewall for Protected {
        async fn evaluate(&self, _: &FirewallRequest) -> RequestAction {
            RequestAction::Forward
        }
        fn protects_secrets(&self, _: &str) -> bool {
            true
        }
        fn secret_inspection_warning(&self, _: &ObservedExchange, _: &str) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    #[tokio::test]
    async fn unsupported_inference_passes_with_warnings_while_auth_remains_untouched() {
        let firewall = Arc::new(Protected(Default::default()));
        let interceptor = ActiveInterceptor::new(
            PassiveInterceptor::new(Arc::new(lr_monitor::MonitorEventStore::new(4))),
            firewall.clone(),
        );
        for path in [
            "/v1/embeddings",
            "/v1/audio/speech",
            "/v1/images/generations",
            "/v1/files",
            "/v1/realtime",
            "/v1/batches",
            "/backend-api/conversation",
            "/backend-api/codex/responses/id/unknown-action",
        ] {
            let exchange = ObservedExchange {
                method: if path.contains("realtime") {
                    "GET"
                } else {
                    "POST"
                }
                .into(),
                path: path.into(),
                ..Default::default()
            };
            assert!(matches!(
                interceptor.on_request(&exchange).await,
                RequestAction::Forward
            ));
        }
        let auth = ObservedExchange {
            method: "POST".into(),
            path: "/oauth/token".into(),
            ..Default::default()
        };
        assert!(matches!(
            interceptor.on_request(&auth).await,
            RequestAction::Forward
        ));
        assert_eq!(firewall.0.load(std::sync::atomic::Ordering::Relaxed), 8);
        assert_eq!(
            wire::detect("/backend-api/codex/responses/compact"),
            Some(crate::wire::WireFormat::OpenAiResponses)
        );
    }
}
