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
    BodyPolicy, ClientCtx, ConnectDecision, ObservedExchange, PassthroughExchange,
    ProxyInterceptor, RequestAction, RequestHead,
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
    /// Decide from the head of a recognized LLM request how its body is
    /// handled, before any of it is read. Return
    /// [`BodyPolicy::Reject`] only for a refusal that
    /// [`evaluate`](Self::evaluate) would reach whatever the body holds. The
    /// default buffers, so `evaluate` sees every request.
    ///
    /// Not asked about requests the interceptor forwards without consulting
    /// the firewall at all (unrecognized paths, duplicate hops when secrets
    /// are not protected), nor about image requests and websocket upgrades.
    fn body_policy(&self, _head: &RequestHead<'_>) -> BodyPolicy {
        BodyPolicy::Buffered
    }
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

    fn body_policy(&self, head: &RequestHead<'_>) -> BodyPolicy {
        // Mirrors the two early returns of `on_request`: a request it
        // forwards without reading the body does not need one buffered.
        if head.is_duplicate_hop() && !self.firewall.protects_secrets(head.client_id) {
            return BodyPolicy::PassThrough;
        }
        match wire::detect(head.path) {
            None => BodyPolicy::PassThrough,
            // Whether an image request is inspected depends on its body.
            Some(wire::WireFormat::Images) => BodyPolicy::Buffered,
            Some(_) => match self.firewall.body_policy(head) {
                // The firewall evaluates every recognized LLM request, so
                // none of them can skip buffering yet.
                BodyPolicy::PassThrough => BodyPolicy::Buffered,
                policy => policy,
            },
        }
    }

    async fn on_request(&self, ex: &ObservedExchange) -> RequestAction {
        // A request an earlier LocalRouter hop already handled was already
        // judged (and possibly rewritten) there — pass it through verbatim.
        if ex.is_duplicate_hop() && !self.firewall.protects_secrets(&ex.client_id) {
            return RequestAction::Forward;
        }
        // A pass-through request reaches the firewall only when the policy
        // that let its body skip buffering changed before this call. Its body
        // was never read, so it cannot be judged: refuse it.
        if ex.request_capture.is_some() && wire::detect(&ex.path).is_some() {
            return RequestAction::reject_json(
                503,
                "Request policy changed while the request was being forwarded; retry",
            );
        }
        // Only recognized LLM API calls are subject to the firewall; anything
        // else (auth preflights, etc.) passes through untouched.
        let format = wire::detect(&ex.path);
        // Image edits may be multipart (OpenAI's public API); only the JSON
        // shape has a prompt we can scan, so treat the rest as unsupported.
        let format = format.filter(|format| {
            *format != wire::WireFormat::Images
                || ex
                    .request_body
                    .as_ref()
                    .and_then(|b| serde_json::from_slice::<Value>(b).ok())
                    .is_some_and(|body| crate::images::is_json_request(&body))
        });
        let Some(format) = format else {
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
                let path = path.split('#').next().unwrap_or(path);
                self.firewall.secret_inspection_warning(
                    ex,
                    &format!(
                        "Unsupported endpoint for secret scanning: {} {}{}. LocalRouter does not know how to scan this request. The request was sent without scanning for secrets.",
                        ex.method, ex.host, path
                    ),
                );
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
    fn body_policy(&self, head: &RequestHead<'_>) -> BodyPolicy {
        ProxyInterceptor::body_policy(self, head)
    }
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
mod body_policy_tests {
    use super::*;
    use crate::interceptor::{ExchangeSource, StreamMasking, StreamPolicy, DEFAULT_STREAM_LAG};

    /// A firewall with a fixed header-time answer.
    struct Fixed {
        policy: BodyPolicy,
        protects: bool,
    }
    #[async_trait]
    impl Firewall for Fixed {
        fn body_policy(&self, _: &RequestHead<'_>) -> BodyPolicy {
            self.policy.clone()
        }
        async fn evaluate(&self, _: &FirewallRequest) -> RequestAction {
            RequestAction::reject_json(403, "evaluated")
        }
        fn protects_secrets(&self, _: &str) -> bool {
            self.protects
        }
    }

    fn interceptor(policy: BodyPolicy, protects: bool) -> ActiveInterceptor {
        ActiveInterceptor::new(
            PassiveInterceptor::new(Arc::new(lr_monitor::MonitorEventStore::new(4))),
            Arc::new(Fixed { policy, protects }),
        )
    }

    fn head<'a>(
        path: &'a str,
        headers: &'a http::HeaderMap,
        trace: Option<&'a lr_types::RequestTrace>,
    ) -> RequestHead<'a> {
        RequestHead {
            client_id: "client",
            strategy_id: "strategy",
            host: "api.openai.com",
            port: 443,
            method: "POST",
            path,
            headers,
            trace,
            provider_override: None,
            source: ExchangeSource::Proxy,
        }
    }

    fn policy(
        it: &ActiveInterceptor,
        path: &str,
        trace: Option<&lr_types::RequestTrace>,
    ) -> BodyPolicy {
        ProxyInterceptor::body_policy(it, &head(path, &http::HeaderMap::new(), trace))
    }

    #[test]
    fn unrecognized_paths_pass_through_whatever_the_firewall_says() {
        for protects in [false, true] {
            let it = interceptor(BodyPolicy::Reject(403, "no".into()), protects);
            for path in [
                "/v1/files",
                "/api/blobs/sha256:abc",
                "/oauth/token?x=1",
                "/",
            ] {
                assert_eq!(policy(&it, path, None), BodyPolicy::PassThrough, "{path}");
            }
        }
    }

    #[test]
    fn llm_paths_never_pass_through() {
        // Even a firewall that asks for it: every LLM body is evaluated.
        let it = interceptor(BodyPolicy::PassThrough, false);
        for path in [
            "/v1/messages",
            "/v1/chat/completions",
            "/v1/responses",
            "/backend-api/codex/responses",
            "/backend-api/codex/responses/compact",
            "/api/chat",
            "/api/generate",
            "/v1/systemone",
        ] {
            assert_eq!(policy(&it, path, None), BodyPolicy::Buffered, "{path}");
        }
        // The default firewall policy buffers too.
        struct Plain;
        #[async_trait]
        impl Firewall for Plain {
            async fn evaluate(&self, _: &FirewallRequest) -> RequestAction {
                RequestAction::Forward
            }
        }
        let it = ActiveInterceptor::new(
            PassiveInterceptor::new(Arc::new(lr_monitor::MonitorEventStore::new(4))),
            Arc::new(Plain),
        );
        assert_eq!(policy(&it, "/v1/messages", None), BodyPolicy::Buffered);
    }

    #[test]
    fn firewall_rejects_and_stream_policies_are_passed_on_for_llm_paths() {
        let it = interceptor(BodyPolicy::Reject(429, "slow down".into()), true);
        assert_eq!(
            policy(&it, "/v1/messages", None),
            BodyPolicy::Reject(429, "slow down".into())
        );
        let stream = BodyPolicy::Stream(StreamPolicy {
            masking: StreamMasking::Redact,
            needs_model: true,
            whole_document: false,
            lag: DEFAULT_STREAM_LAG,
            restore_only: false,
        });
        let it = interceptor(stream.clone(), true);
        assert_eq!(policy(&it, "/v1/responses", None), stream);
    }

    #[test]
    fn image_requests_are_buffered_without_asking_the_firewall() {
        let it = interceptor(BodyPolicy::Reject(403, "no".into()), true);
        assert_eq!(
            policy(&it, "/v1/images/generations", None),
            BodyPolicy::Buffered
        );
    }

    #[test]
    fn duplicate_hops_pass_through_only_when_secrets_are_not_protected() {
        let duplicate = lr_types::RequestTrace::parse("abc;hop=2").unwrap();
        let first = lr_types::RequestTrace::parse("abc;hop=1").unwrap();
        let reject = BodyPolicy::Reject(403, "no".into());

        let it = interceptor(reject.clone(), false);
        assert_eq!(
            policy(&it, "/v1/messages", Some(&duplicate)),
            BodyPolicy::PassThrough
        );
        assert_eq!(policy(&it, "/v1/messages", Some(&first)), reject);

        let it = interceptor(reject.clone(), true);
        assert_eq!(policy(&it, "/v1/messages", Some(&duplicate)), reject);
    }

    /// `on_request` forwards exactly the requests `body_policy` lets through
    /// unbuffered, and refuses a pass-through request it would have judged.
    #[tokio::test]
    async fn pass_through_requests_are_never_evaluated_without_their_body() {
        let duplicate = lr_types::RequestTrace::parse("abc;hop=2").unwrap();
        let capture = Some(crate::interceptor::BoundedCapture::default());

        let it = interceptor(BodyPolicy::Buffered, false);
        for (path, trace) in [
            ("/v1/files", None),
            ("/v1/messages", Some(duplicate.clone())),
        ] {
            let ex = ObservedExchange {
                method: "POST".into(),
                path: path.into(),
                trace,
                request_capture: capture.clone(),
                ..Default::default()
            };
            assert!(matches!(it.on_request(&ex).await, RequestAction::Forward));
        }

        // Secrets became protected after the duplicate hop was let through.
        let it = interceptor(BodyPolicy::Buffered, true);
        let ex = ObservedExchange {
            method: "POST".into(),
            path: "/v1/messages".into(),
            trace: Some(duplicate),
            request_capture: capture,
            ..Default::default()
        };
        assert!(matches!(
            it.on_request(&ex).await,
            RequestAction::Reject { status: 503, .. }
        ));
    }

    #[test]
    fn reverse_recorder_shares_the_policy() {
        let it = interceptor(BodyPolicy::Reject(403, "no".into()), true);
        let headers = http::HeaderMap::new();
        let mut blob = head("/api/blobs/sha256:abc", &headers, None);
        blob.source = ExchangeSource::ReverseProxy;
        assert_eq!(
            crate::reverse::ReverseRecorder::body_policy(&it, &blob),
            BodyPolicy::PassThrough
        );
        let mut chat = head("/api/chat", &headers, None);
        chat.source = ExchangeSource::ReverseProxy;
        assert_eq!(
            crate::reverse::ReverseRecorder::body_policy(&it, &chat),
            BodyPolicy::Reject(403, "no".into())
        );
    }

    /// The header-time verdict on Content-Encoding is the decoder's verdict.
    #[tokio::test]
    async fn header_time_encoding_errors_match_the_decoder() {
        for value in [
            "zstd",
            "gzip, br",
            "identity",
            "deflate,gzip,br,zstd",
            "ZSTD",
            "",
            "gzip,",
            "compress",
            "gzip, snappy",
            "identity, identity, identity, identity, identity",
            "gzip, gzip, gzip, gzip, gzip",
        ] {
            let mut headers = http::HeaderMap::new();
            headers.insert(http::header::CONTENT_ENCODING, value.parse().unwrap());
            let decoded =
                crate::request_body::inspect("/v1/responses", &headers, bytes::Bytes::new()).await;
            let unsupported = decoded == Err(crate::request_body::DecodeError::Unsupported);
            assert_eq!(
                head("/v1/responses", &headers, None)
                    .content_encoding_error()
                    .is_some(),
                unsupported,
                "{value:?}"
            );
            // Paths that are not inspected are never decoded.
            assert!(head("/v1/files", &headers, None)
                .content_encoding_error()
                .is_none());
        }
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::CONTENT_ENCODING,
            http::HeaderValue::from_bytes(b"\xff").unwrap(),
        );
        assert!(head("/v1/responses", &headers, None)
            .content_encoding_error()
            .is_some());
        assert!(head("/v1/responses", &http::HeaderMap::new(), None)
            .content_encoding_error()
            .is_none());
        // Split across header lines, the count still applies.
        let mut headers = http::HeaderMap::new();
        for _ in 0..5 {
            headers.append(http::header::CONTENT_ENCODING, "gzip".parse().unwrap());
        }
        assert!(head("/v1/responses", &headers, None)
            .content_encoding_error()
            .is_some());
    }
}

#[cfg(test)]
mod protected_endpoint_tests {
    use super::*;
    struct Protected(std::sync::Mutex<Vec<String>>);
    #[async_trait]
    impl Firewall for Protected {
        async fn evaluate(&self, _: &FirewallRequest) -> RequestAction {
            RequestAction::Forward
        }
        fn protects_secrets(&self, _: &str) -> bool {
            true
        }
        fn secret_inspection_warning(&self, _: &ObservedExchange, message: &str) {
            self.0.lock().unwrap().push(message.into());
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
        assert_eq!(firewall.0.lock().unwrap().len(), 8);
        assert_eq!(
            wire::detect("/backend-api/codex/responses/compact"),
            Some(crate::wire::WireFormat::OpenAiResponses)
        );
    }

    #[tokio::test]
    async fn unsupported_endpoint_warning_identifies_request_without_query_or_fragment() {
        let firewall = Arc::new(Protected(Default::default()));
        let interceptor = ActiveInterceptor::new(
            PassiveInterceptor::new(Arc::new(lr_monitor::MonitorEventStore::new(4))),
            firewall.clone(),
        );
        for path in [
            "/backend-api/conversation?token=private-query#private-fragment",
            "/backend-api/conversation#private-fragment",
        ] {
            let exchange = ObservedExchange {
                method: "POST".into(),
                host: "chatgpt.com".into(),
                path: path.into(),
                ..Default::default()
            };
            assert!(matches!(
                interceptor.on_request(&exchange).await,
                RequestAction::Forward
            ));
        }
        let messages = firewall.0.lock().unwrap();
        assert_eq!(messages.len(), 2);
        for message in messages.iter() {
            assert_eq!(message, "Unsupported endpoint for secret scanning: POST chatgpt.com/backend-api/conversation. LocalRouter does not know how to scan this request. The request was sent without scanning for secrets.");
        }
    }
}
