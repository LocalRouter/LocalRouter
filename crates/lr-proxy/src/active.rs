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
    BodyPolicy, ClientCtx, ConnectDecision, ExchangeAdmission, ObservedExchange,
    PassthroughExchange, ProxyInterceptor, RequestAction, RequestHead, StreamOpen, StreamPolicy,
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
    /// Cross-hop trace of the request; a duplicate hop was already admitted
    /// by its first LocalRouter hop.
    pub trace: Option<lr_types::RequestTrace>,
    /// Where an admitted request's rate-limit reservation is kept: it stays
    /// with the exchange until its usage is recorded.
    pub admission: ExchangeAdmission,
}

/// A recognized LLM request whose body outgrew its stream lag, handed to
/// [`Firewall::open_stream`] before any of it is forwarded.
pub struct StreamRequest {
    pub client_id: String,
    pub session_id: Option<String>,
    pub secret_session: Option<String>,
    pub host: String,
    pub method: String,
    pub path: String,
    pub provider: String,
    /// The request's `model` (a streamed body always has one).
    pub model: Option<String>,
    pub format: wire::WireFormat,
    /// The first decoded bytes, for a read-only preview in an approval
    /// popup (the body is too large to edit there).
    pub preview: Option<StreamPreview>,
    /// As [`FirewallRequest::trace`].
    pub trace: Option<lr_types::RequestTrace>,
    /// As [`FirewallRequest::admission`]; kept only when the body streams.
    pub admission: ExchangeAdmission,
}

/// The beginning of a body that is still uploading.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamPreview {
    /// The first decoded bytes as text (invalid UTF-8 replaced).
    pub text: String,
    /// Decoded bytes received when the preview was taken.
    pub received: u64,
    /// `text` covers less than `received`.
    pub truncated: bool,
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
    /// Each client message of an LLM websocket is asked about as a `POST` of
    /// that message to the connection's path, without headers.
    fn body_policy(&self, _head: &RequestHead<'_>) -> BodyPolicy {
        BodyPolicy::Buffered
    }
    async fn evaluate(&self, req: &FirewallRequest) -> RequestAction;
    /// Decide how a body that [`body_policy`](Self::body_policy) let stream
    /// is handled once it outgrew the lag: stream it through a session, read
    /// it whole for [`evaluate`](Self::evaluate), or refuse it. The default
    /// reads it whole.
    async fn open_stream(&self, _req: &StreamRequest, _policy: &StreamPolicy) -> StreamOpen {
        StreamOpen::Buffer
    }
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
            trace: ex.trace.clone(),
            admission: ex.admission.clone(),
        };
        // One reservation per exchange: this decision replaces any earlier
        // one (a stream offer that fell back to buffering).
        ex.admission.release();
        let action = if let Some(error) = ex.request_body_error {
            self.firewall.evaluate_uninspectable(&req, error).await
        } else {
            self.firewall.evaluate(&req).await
        };
        if matches!(action, RequestAction::Reject { .. }) {
            ex.admission.release();
        }
        action
    }

    async fn open_stream(
        &self,
        ex: &ObservedExchange,
        metadata: &lr_secret_scanner::request_stream::RequestMetadata,
        policy: &StreamPolicy,
    ) -> StreamOpen {
        if ex.is_duplicate_hop() && !self.firewall.protects_secrets(&ex.client_id) {
            return StreamOpen::Buffer;
        }
        // Only these formats have the conversation layout the pipeline cuts
        // (Ollama's native chat and generate: `messages` / `prompt`).
        let Some(format) = wire::detect(&ex.path).filter(|format| {
            matches!(
                format,
                wire::WireFormat::AnthropicMessages
                    | wire::WireFormat::OpenAiChat
                    | wire::WireFormat::OpenAiResponses
                    | wire::WireFormat::Ollama(_)
            )
        }) else {
            return StreamOpen::Buffer;
        };
        let req = StreamRequest {
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
            model: metadata.model.value().cloned(),
            format,
            preview: ex.request_capture.as_ref().map(|capture| StreamPreview {
                text: String::from_utf8_lossy(&capture.head).into_owned(),
                received: capture.total_len,
                truncated: capture.truncated,
            }),
            trace: ex.trace.clone(),
            admission: ex.admission.clone(),
        };
        ex.admission.release();
        let open = self.firewall.open_stream(&req, policy).await;
        // Only a streaming body keeps its admission; a buffered one is
        // admitted again by `on_request`.
        if !matches!(open, StreamOpen::Stream { .. }) {
            ex.admission.release();
        }
        open
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
        // Recording settled the admission with the usage; anything still
        // held (an exchange the recorder does not count) ends here.
        ex.admission.release();
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
    async fn open_stream(
        &self,
        ex: &crate::reverse::ReverseExchange,
        metadata: &lr_secret_scanner::request_stream::RequestMetadata,
        policy: &StreamPolicy,
    ) -> StreamOpen {
        ProxyInterceptor::open_stream(
            self,
            &crate::passive::observed_from_reverse(ex),
            metadata,
            policy,
        )
        .await
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
            strict: false,
            incremental: false,
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

/// One admission per exchange, released on every refusal.
#[cfg(test)]
mod admission_tests {
    use super::*;
    use crate::interceptor::{AdmissionGuard, RecordedUsage};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts the guards alive.
    struct Held(Arc<AtomicUsize>);
    impl Held {
        fn new(live: &Arc<AtomicUsize>) -> Box<Self> {
            live.fetch_add(1, Ordering::SeqCst);
            Box::new(Self(live.clone()))
        }
    }
    impl Drop for Held {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    impl AdmissionGuard for Held {
        fn settle(self: Box<Self>, _: &RecordedUsage) {}
    }

    /// Reserves in every decision, then answers as configured.
    struct Reserving {
        live: Arc<AtomicUsize>,
        reject: bool,
        open: fn() -> StreamOpen,
    }
    #[async_trait]
    impl Firewall for Reserving {
        async fn evaluate(&self, req: &FirewallRequest) -> RequestAction {
            // An earlier decision's share is gone before a new one is taken.
            assert!(!req.admission.is_held(), "one admission per exchange");
            req.admission.hold(Held::new(&self.live));
            if self.reject {
                RequestAction::reject_json(403, "no")
            } else {
                RequestAction::Forward
            }
        }
        async fn open_stream(&self, req: &StreamRequest, _: &StreamPolicy) -> StreamOpen {
            assert!(!req.admission.is_held(), "one admission per exchange");
            req.admission.hold(Held::new(&self.live));
            (self.open)()
        }
    }

    fn interceptor(
        live: &Arc<AtomicUsize>,
        reject: bool,
        open: fn() -> StreamOpen,
    ) -> ActiveInterceptor {
        ActiveInterceptor::new(
            PassiveInterceptor::new(Arc::new(lr_monitor::MonitorEventStore::new(8))),
            Arc::new(Reserving {
                live: live.clone(),
                reject,
                open,
            }),
        )
    }

    fn exchange() -> ObservedExchange {
        ObservedExchange {
            method: "POST".into(),
            path: "/v1/messages".into(),
            request_body: Some(br#"{"model":"m","messages":[]}"#.to_vec()),
            ..Default::default()
        }
    }

    fn policy() -> StreamPolicy {
        StreamPolicy {
            masking: crate::interceptor::StreamMasking::Off,
            needs_model: false,
            whole_document: false,
            lag: 0,
            restore_only: false,
            strict: false,
            incremental: false,
        }
    }

    #[tokio::test]
    async fn an_admitted_request_keeps_its_share_until_recorded() {
        let live = Arc::new(AtomicUsize::new(0));
        let it = interceptor(&live, false, || StreamOpen::Buffer);
        let mut ex = exchange();
        assert!(matches!(it.on_request(&ex).await, RequestAction::Forward));
        assert_eq!(live.load(Ordering::SeqCst), 1);
        // A clone (the transport's response half) shares it.
        let recorded = ex.clone();
        // Deciding again replaces the share rather than adding one.
        assert!(matches!(it.on_request(&ex).await, RequestAction::Forward));
        assert_eq!(live.load(Ordering::SeqCst), 1);
        ex.status = Some(200);
        it.on_response(&recorded).await;
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_refusal_releases_the_share() {
        let live = Arc::new(AtomicUsize::new(0));
        let it = interceptor(&live, true, || StreamOpen::Buffer);
        let ex = exchange();
        assert!(matches!(
            it.on_request(&ex).await,
            RequestAction::Reject { status: 403, .. }
        ));
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn dropping_the_exchange_releases_the_share() {
        let live = Arc::new(AtomicUsize::new(0));
        let it = interceptor(&live, false, || StreamOpen::Buffer);
        let ex = exchange();
        it.on_request(&ex).await;
        let clone = ex.clone();
        drop(ex);
        assert_eq!(live.load(Ordering::SeqCst), 1, "a clone still holds it");
        drop(clone);
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn only_a_stream_keeps_its_share() {
        let metadata = lr_secret_scanner::request_stream::RequestMetadata::default();
        let live = Arc::new(AtomicUsize::new(0));
        for open in [(|| StreamOpen::Buffer) as fn() -> StreamOpen, || {
            StreamOpen::Reject(RequestAction::reject_json(429, "slow down"))
        }] {
            let it = interceptor(&live, false, open);
            let ex = exchange();
            let decided = it.open_stream(&ex, &metadata, &policy()).await;
            assert!(!matches!(decided, StreamOpen::Stream { .. }));
            assert!(!ex.admission.is_held());
            assert_eq!(live.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn duplicate_hops_reach_no_admission() {
        let live = Arc::new(AtomicUsize::new(0));
        let it = interceptor(&live, false, || StreamOpen::Buffer);
        let mut ex = exchange();
        ex.trace = Some(lr_types::RequestTrace::parse("abc;hop=2").unwrap());
        assert!(matches!(it.on_request(&ex).await, RequestAction::Forward));
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }
}
