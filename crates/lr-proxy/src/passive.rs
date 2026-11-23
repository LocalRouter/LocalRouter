//! The v1 inspect-only interceptor.
//!
//! Decides which hosts to MITM, and records each decrypted LLM exchange to the
//! monitor as a single combined event — without ever modifying the traffic.
//! Active rewriting is a future implementation of the same
//! [`ProxyInterceptor`](crate::interceptor::ProxyInterceptor) trait.

use std::sync::Arc;

use async_trait::async_trait;
use lr_monitor::{
    truncate_json, truncate_json_owned, EventStatus, LlmApi, LlmCallSource, LlmProtocol,
    MonitorEventData, MonitorEventStore, MonitorEventType, PassthroughMode,
};
use lr_monitoring::metrics::{MetricsCollector, RequestMetrics};

use crate::interceptor::{
    BodyPolicy, BoundedCapture, ClientCtx, ClientNameResolver, ConnectDecision, ExchangeSource,
    ObservedExchange, PassthroughExchange, PricingResolver, ProxyInterceptor, RequestAction,
    RequestHead, TokenUsage,
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
    /// Per-account usage ledger (subscription value, API spend).
    usage: Option<Arc<lr_usage::UsageTracker>>,
}

impl PassiveInterceptor {
    pub fn new(monitor: Arc<MonitorEventStore>) -> Self {
        Self {
            monitor,
            metrics: None,
            pricing: None,
            client_names: None,
            usage: None,
        }
    }

    /// Attach the usage tracker so proxied calls feed the per-account ledger.
    pub fn with_usage(mut self, usage: Arc<lr_usage::UsageTracker>) -> Self {
        self.usage = Some(usage);
        self
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

    /// Parse the request half into the pieces a monitor event needs. The
    /// body is parsed once; `full` carries the parse on for correlation.
    fn request_parts(format: WireFormat, ex: &ObservedExchange) -> RequestParts {
        if let Some(streamed) = &ex.request_stream {
            // A streamed body is never held whole: its metadata was sniffed,
            // and only a capture that holds all of it is shown parsed.
            let display = ex
                .request_capture
                .as_ref()
                .and_then(BoundedCapture::inspectable)
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok())
                .map(|b| truncate_json_owned(b, PARSED_CAP));
            return RequestParts {
                meta: streamed.meta.clone(),
                display,
                raw: ex.request_capture.as_ref().and_then(raw_capture),
                tool_count: streamed.tool_count,
                full: None,
            };
        }
        let full =
            request_bytes(ex).and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok());
        let meta = full
            .as_ref()
            .map(|b| wire::parse_request(format, b))
            .unwrap_or_default();
        // Raw wire payload, capped, so the exact bytes are always inspectable.
        let raw = match (&ex.request_body, &ex.request_capture) {
            (Some(body), _) => Some(cap_raw(&String::from_utf8_lossy(body))),
            (None, Some(capture)) => raw_capture(capture),
            (None, None) => None,
        };
        let tool_count = full
            .as_ref()
            .and_then(|b| b.get("tools"))
            .and_then(|t| t.as_array())
            .map(Vec::len)
            .unwrap_or(0);
        // Capped only after the metadata above is read off the full body.
        let display = full.as_ref().map(|b| truncate_json(b, PARSED_CAP));
        RequestParts {
            meta,
            display,
            raw,
            tool_count,
            full,
        }
    }

    /// The request metadata a pending event recorded when it opened, so
    /// completing it does not parse the request again.
    fn recorded_request_meta(&self, event_id: &str) -> Option<RequestMeta> {
        match self.monitor.get(event_id)?.data {
            MonitorEventData::LlmCall {
                requested_model,
                stream,
                message_count,
                has_tools,
                ..
            } => Some(RequestMeta {
                model: requested_model,
                stream,
                message_count,
                has_tools,
            }),
            _ => None,
        }
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
            Some(bytes)
                if lr_secret_scanner::wire::detect_sse(bytes).unwrap_or(ex.response_is_sse) =>
            {
                let raw = String::from_utf8_lossy(bytes);
                let (meta, body) = wire::reconstruct_sse(format, &raw);
                (meta, Some(body))
            }
            // Ollama's native streaming: one JSON object per line.
            Some(bytes) if ex.response_is_ndjson => {
                let raw = String::from_utf8_lossy(bytes);
                let (meta, body) = wire::reconstruct_ndjson(format, &raw);
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
        // Capped only after `resp_meta` is read off the full body.
        let response_json = response_json.map(|b| truncate_json_owned(b, PARSED_CAP));
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
        let provider = ex
            .provider_override
            .clone()
            .unwrap_or_else(|| wire::provider_for_host(&ex.host).to_string());
        let provider = provider.as_str();

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
            _ if ex.error.is_some() => EventStatus::Error,
            Some(code) if code >= 400 => EventStatus::Error,
            Some(_) => EventStatus::Complete,
            None => EventStatus::Error,
        };

        // A duplicate hop was counted by the first hop; recording it again
        // would double every tier.
        let metrics = self.metrics.as_ref().filter(|_| !ex.is_duplicate_hop());
        let ledger = self.usage.as_ref().zip(ex.usage_account.as_ref());
        if let Some((tracker, account)) =
            ledger.filter(|_| status == EventStatus::Complete && !ex.is_duplicate_hop())
        {
            tracker.record_request(
                account,
                lr_usage::LedgerEntry {
                    requests: 1,
                    input_tokens: usage.input,
                    output_tokens: usage.output,
                    cache_read_tokens: usage.cache_read,
                    cache_write_tokens: usage.cache_write,
                    cost_usd: cost_usd.unwrap_or(0.0),
                },
            );
        }
        if let Some(metrics) = metrics {
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
        let Some(format) = wire::detect(&ex.path) else {
            // Not an LLM API call: forwarded untouched, recorded as a
            // passthrough so the stray traffic is at least visible.
            return self.begin_passthrough(&passthrough_from_observed(ex)?);
        };
        let RequestParts {
            meta: req_meta,
            display: request_json,
            raw: raw_request,
            tool_count,
            full,
        } = Self::request_parts(format, ex);
        let model = req_meta.model.clone().unwrap_or_default();

        let data = MonitorEventData::LlmCall {
            endpoint: ex.path.clone(),
            model,
            stream: req_meta.stream,
            message_count: req_meta.message_count,
            has_tools: req_meta.has_tools,
            tool_count,
            request_body: request_json.unwrap_or(serde_json::Value::Null),
            source: source_for(ex.source),
            protocol: protocol_for(format),
            // The proxy forwards the call as-is: both sides speak one API.
            client_api: Some(api_for(format)),
            upstream_api: Some(api_for(format)),
            requested_model: req_meta.model.clone(),
            cached_input_tokens: None,
            transformed_body: None,
            transformations_applied: duplicate_label(ex),
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
            error: ex.error.clone(),
            routing_info: None,
            trace_id: ex
                .trace
                .as_ref()
                .filter(|_| !ex.connection_trace)
                .map(|t| t.trace_id.clone()),
            duplicate_hop: duplicate_hop(ex),
        };

        let event_id = self.monitor.push(
            MonitorEventType::LlmCall,
            Some(ex.client_id.clone()),
            self.client_name(&ex.client_id),
            ex.session_id.clone(),
            data,
            EventStatus::Pending,
            None,
        );
        self.track_conversation(&event_id, ex, full);
        Some(event_id)
    }

    /// Fill in the response half of a previously-opened Pending event
    /// (pending → complete/error), recording cost + metrics.
    pub(crate) fn complete(&self, event_id: &str, ex: &ObservedExchange) {
        let Some(format) = wire::detect(&ex.path) else {
            if let Some(pt) = passthrough_from_observed(ex) {
                self.end_passthrough(Some(event_id.to_string()), &pt);
            }
            return;
        };
        // A pass-through body had not been read when the pending event was
        // opened, so its request half is only known now. A buffered one was
        // parsed when the event opened and is not parsed again.
        let late_request = ex.request_body.is_none() && ex.request_capture.is_some();
        let parts = if late_request {
            Self::request_parts(format, ex)
        } else {
            RequestParts {
                meta: match self.recorded_request_meta(event_id) {
                    Some(meta) => meta,
                    None => Self::request_parts(format, ex).meta,
                },
                ..Default::default()
            }
        };
        let RequestParts {
            meta: req_meta,
            display: request_json,
            raw: raw_request,
            tool_count,
            full,
        } = parts;
        let (resp_meta, response_json, raw_response) = Self::response_parts(format, ex);
        let (model, cost_usd, total_tokens, status) = self.finalize(ex, &req_meta, &resp_meta);

        self.monitor.update(event_id, |event| {
            event.status = status;
            event.duration_ms = ex.latency_ms;
            if late_request {
                if let MonitorEventData::LlmCall {
                    stream,
                    message_count,
                    has_tools,
                    tool_count: tools,
                    request_body,
                    requested_model,
                    streamed,
                    raw_request: raw,
                    ..
                } = &mut event.data
                {
                    *stream = req_meta.stream;
                    *message_count = req_meta.message_count;
                    *has_tools = req_meta.has_tools;
                    *tools = tool_count;
                    *request_body = request_json.unwrap_or(serde_json::Value::Null);
                    *requested_model = req_meta.model.clone();
                    *streamed = Some(req_meta.stream);
                    *raw = raw_request;
                }
            }
            if let MonitorEventData::LlmCall {
                model: m,
                status_code: sc,
                input_tokens: it,
                output_tokens: ot,
                total_tokens: tt,
                reasoning_tokens: rt,
                cached_input_tokens: ct,
                cost_usd: cu,
                latency_ms: lm,
                finish_reason: fr,
                content_preview: cp,
                response_body: rb,
                raw_response: rr,
                error: err,
                ..
            } = &mut event.data
            {
                *m = model;
                *ct = resp_meta.cache_read_tokens;
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
                if ex.error.is_some() {
                    *err = ex.error.clone();
                }
            }
        });
        if late_request {
            self.track_conversation(event_id, ex, full);
        }
    }

    /// Record a fully-observed exchange as one combined event in a single push.
    /// Used when there is no pre-opened pending event (e.g. a firewall reject
    /// synthesizes its response without ever contacting the upstream).
    pub(crate) fn record(&self, ex: &ObservedExchange) {
        let Some(format) = wire::detect(&ex.path) else {
            if let Some(pt) = passthrough_from_observed(ex) {
                self.end_passthrough(None, &pt);
            }
            return;
        };
        let RequestParts {
            meta: req_meta,
            display: request_json,
            raw: raw_request,
            tool_count,
            full,
        } = Self::request_parts(format, ex);
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
            source: source_for(ex.source),
            protocol: protocol_for(format),
            // The proxy forwards the call as-is: both sides speak one API.
            client_api: Some(api_for(format)),
            upstream_api: Some(api_for(format)),
            requested_model: req_meta.model.clone(),
            cached_input_tokens: resp_meta.cache_read_tokens,
            transformed_body: None,
            transformations_applied: duplicate_label(ex),
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
            error: ex.error.clone(),
            routing_info: None,
            trace_id: ex
                .trace
                .as_ref()
                .filter(|_| !ex.connection_trace)
                .map(|t| t.trace_id.clone()),
            duplicate_hop: duplicate_hop(ex),
        };

        let event_id = self.monitor.push(
            MonitorEventType::LlmCall,
            Some(ex.client_id.clone()),
            self.client_name(&ex.client_id),
            ex.session_id.clone(),
            data,
            status,
            ex.latency_ms,
        );
        self.track_conversation(&event_id, ex, full);
    }

    /// Correlate the request into a conversation, from its parsed body or,
    /// for a streamed body, from the parts collected while it was forwarded.
    fn track_conversation(
        &self,
        event_id: &str,
        ex: &ObservedExchange,
        body: Option<serde_json::Value>,
    ) {
        if let Some(streamed) = &ex.request_stream {
            let Some(mut parts) = streamed.correlation.clone() else {
                return;
            };
            let caller_session = ex
                .secret_session
                .clone()
                .or_else(|| streamed.conversation_id.clone());
            if let Some(session) = caller_session {
                // As below: the caller's session stands in for a missing id.
                if parts.session.as_ref().and_then(|s| s.get("id")).is_none() {
                    parts.session = Some(serde_json::json!({ "id": session }));
                }
            }
            self.monitor.track_conversation_with(event_id, parts);
            return;
        }
        if let Some(mut body) = body {
            let caller_session = ex
                .secret_session
                .clone()
                .or_else(|| wire::conversation_id(&body));
            if let Some(session) = &caller_session {
                if body.pointer("/extensions/localrouter.session/id").is_none() {
                    if let Some(object) = body.as_object_mut() {
                        let extensions = object
                            .entry("extensions")
                            .or_insert_with(|| serde_json::json!({}));
                        if let Some(extensions) = extensions.as_object_mut() {
                            extensions.insert(
                                "localrouter.session".into(),
                                serde_json::json!({"id":session}),
                            );
                        }
                    }
                }
            }
            self.monitor.track_conversation(event_id, &body);
        }
    }
}

// ---------------------------------------------------------------------------
// Passthrough (non-LLM) traffic
// ---------------------------------------------------------------------------

/// The warning every passthrough event carries: why this is here, and the
/// promise that nothing about its content was kept.
pub const PASSTHROUGH_WARNING: &str =
    "`HTTPS_PROXY` applies to a whole process, so a tool pointed at \
     LocalRouter also sends its unrelated traffic (git, package managers, \
     telemetry, update checks) here. It is forwarded untouched — no request or \
     response content is inspected, recorded, or logged. If you did not expect \
     this destination, narrow where the proxy setting is applied.";

/// The per-mode explanation shown above [`PASSTHROUGH_WARNING`].
fn passthrough_note(ex: &PassthroughExchange) -> String {
    let dest = format!("{}:{}", ex.host, ex.port);
    let what = match ex.mode {
        PassthroughMode::Tunnel => format!(
            "Not an LLM call. This connection to {dest} was tunneled through \
             LocalRouter byte-for-byte; its TLS was never decrypted, so only \
             the destination is known."
        ),
        PassthroughMode::Http => format!(
            "Not an LLM call. This plain HTTP request was forwarded to {dest} \
             unchanged."
        ),
        PassthroughMode::Inspected => format!(
            "Not an LLM call. {} is inspected for LLM traffic, but this path is \
             not an LLM API endpoint, so the request was forwarded unchanged \
             and its content was not captured.",
            ex.host
        ),
        PassthroughMode::Websocket => format!(
            "Not an LLM call. This websocket connection to {dest} was relayed \
             frame-for-frame without inspection."
        ),
    };
    format!("{what}\n\n{PASSTHROUGH_WARNING}")
}

/// Build the monitor payload for a passthrough exchange. Deliberately carries
/// destination + volume only — never a body, header, or query string.
fn passthrough_data(ex: &PassthroughExchange, closed: bool) -> MonitorEventData {
    MonitorEventData::ProxyPassthrough {
        mode: ex.mode,
        host: ex.host.clone(),
        port: ex.port,
        method: ex.method.clone(),
        path: ex.path.clone(),
        status_code: ex.status,
        bytes_sent: closed.then_some(ex.bytes_sent),
        bytes_received: closed.then_some(ex.bytes_received),
        note: passthrough_note(ex),
        error: ex.error.clone(),
    }
}

/// Project a decrypted-but-unrecognized exchange onto a passthrough record.
///
/// Only the MITM proxy produces these: the reverse proxy deliberately wraps a
/// provider's own port, so its non-LLM paths (`/api/tags`, health checks) are
/// expected traffic, not accidental egress.
fn passthrough_from_observed(ex: &ObservedExchange) -> Option<PassthroughExchange> {
    if ex.source != ExchangeSource::Proxy {
        return None;
    }
    Some(PassthroughExchange {
        client_id: ex.client_id.clone(),
        mode: PassthroughMode::Inspected,
        host: ex.host.clone(),
        // The MITM path always knows its port; fall back to https.
        port: if ex.port == 0 { 443 } else { ex.port },
        method: (!ex.method.is_empty()).then(|| ex.method.clone()),
        path: Some(PassthroughExchange::strip_query(&ex.path)),
        status: ex.status,
        // Known only for a body forwarded without buffering.
        bytes_sent: ex.request_capture.as_ref().map_or(0, |c| c.total_len),
        latency_ms: ex.latency_ms,
        error: ex.error.clone(),
        ..Default::default()
    })
}

impl PassiveInterceptor {
    /// Open a Pending passthrough event while the connection is in flight.
    pub(crate) fn open_passthrough(&self, ex: &PassthroughExchange) -> String {
        self.monitor.push(
            MonitorEventType::ProxyPassthrough,
            Some(ex.client_id.clone()),
            self.client_name(&ex.client_id),
            None,
            passthrough_data(ex, false),
            EventStatus::Pending,
            None,
        )
    }

    /// Close out a passthrough exchange: complete the pending event if one was
    /// opened, otherwise push a combined one.
    pub(crate) fn close_passthrough(&self, event_id: Option<String>, ex: &PassthroughExchange) {
        let status = passthrough_status(ex);
        let Some(id) = event_id else {
            self.monitor.push(
                MonitorEventType::ProxyPassthrough,
                Some(ex.client_id.clone()),
                self.client_name(&ex.client_id),
                None,
                passthrough_data(ex, true),
                status,
                ex.latency_ms,
            );
            return;
        };
        self.monitor.update(&id, |event| {
            event.status = status;
            event.duration_ms = ex.latency_ms;
            event.data = passthrough_data(ex, true);
        });
    }
}

/// A passthrough failed only if the forward itself broke, or the upstream
/// answered with an error status we happened to see.
fn passthrough_status(ex: &PassthroughExchange) -> EventStatus {
    if ex.error.is_some() || ex.status.is_some_and(|s| s >= 400) {
        EventStatus::Error
    } else {
        EventStatus::Complete
    }
}

// ---------------------------------------------------------------------------
// Reverse-proxy bridge
// ---------------------------------------------------------------------------

/// Project a reverse-proxy exchange onto the shared observation type, so both
/// proxies produce identical monitor events and metrics.
pub(crate) fn observed_from_reverse(ex: &crate::reverse::ReverseExchange) -> ObservedExchange {
    ObservedExchange {
        client_id: ex.client_id.clone(),
        session_id: ex.session_id.clone(),
        secret_session: ex.secret_session.clone(),
        event_id: ex.event_id.clone(),
        strategy_id: ex.strategy_id.clone(),
        latency_ms: ex.latency_ms,
        // The wrapped provider stands in for the "host" — monitor events show
        // it as the provider, and it reads better than `127.0.0.1:11435`.
        host: ex
            .provider_instance
            .clone()
            .unwrap_or_else(|| ex.upstream.clone()),
        // The reverse proxy identifies its upstream by provider, not host:port.
        port: 0,
        method: ex.method.clone(),
        path: ex.path.clone(),
        request_body: ex.request_body.clone(),
        request_capture: ex.request_capture.clone(),
        // The reverse proxy buffers every inspected body.
        request_stream: None,
        request_body_error: ex.request_body_error,
        status: ex.status,
        response_body: ex.response_body.clone(),
        response_is_sse: ex.response_is_sse,
        response_is_ndjson: ex.response_is_ndjson,
        provider_override: ex.provider_instance.clone(),
        source: ExchangeSource::ReverseProxy,
        error: ex.error.clone(),
        trace: ex.trace.clone(),
        connection_trace: false,
        // Local providers behind the reverse proxy have no usage limits.
        usage_account: None,
    }
}

#[async_trait]
impl crate::reverse::ReverseRecorder for PassiveInterceptor {
    fn body_policy(&self, _head: &RequestHead<'_>) -> BodyPolicy {
        // Observe-only: the body is never needed to decide anything.
        BodyPolicy::PassThrough
    }

    fn begin(&self, ex: &crate::reverse::ReverseExchange) -> Option<String> {
        self.emit_pending(&observed_from_reverse(ex))
    }

    async fn record(&self, ex: crate::reverse::ReverseExchange) {
        let observed = observed_from_reverse(&ex);
        match &observed.event_id {
            // Complete the pending event opened at request time.
            Some(id) => self.complete(id, &observed),
            // No pending event (an unrecognized path, or `begin` declined):
            // push a combined one. `PassiveInterceptor::record` is the
            // inherent method, disambiguated from this trait method.
            None => PassiveInterceptor::record(self, &observed),
        }
    }
}

/// Map the capture site onto the monitor's source enum.
fn source_for(source: ExchangeSource) -> LlmCallSource {
    match source {
        ExchangeSource::Proxy => LlmCallSource::Proxy,
        ExchangeSource::ReverseProxy => LlmCallSource::ReverseProxy,
    }
}

/// Hop number when an earlier LocalRouter hop already handled the request.
fn duplicate_hop(ex: &ObservedExchange) -> Option<u32> {
    ex.trace
        .as_ref()
        .filter(|t| t.is_duplicate())
        .map(|t| t.hop)
}

/// Transformation label shown on duplicate-hop events.
pub const DUPLICATE_HOP_LABEL: &str = "duplicate hop (passthrough, not counted)";

fn duplicate_label(ex: &ObservedExchange) -> Option<Vec<String>> {
    duplicate_hop(ex).map(|_| vec![DUPLICATE_HOP_LABEL.to_string()])
}

/// Monitor protocol marker for a wire format.
fn protocol_for(format: WireFormat) -> LlmProtocol {
    match format {
        WireFormat::AnthropicMessages => LlmProtocol::Anthropic,
        WireFormat::OpenAiChat
        | WireFormat::OpenAiResponses
        | WireFormat::CodexSearch
        | WireFormat::Images => LlmProtocol::Openai,
        // Ollama's native dialect is closest to the OpenAI shape for the
        // monitor's purposes (messages + content + token counts).
        WireFormat::Ollama(_) => LlmProtocol::Openai,
        WireFormat::SystemOne => LlmProtocol::SystemOne,
        WireFormat::Decisions => LlmProtocol::Openai,
    }
}

/// The API a recognized wire format belongs to.
fn api_for(format: WireFormat) -> LlmApi {
    match format {
        WireFormat::AnthropicMessages => LlmApi::AnthropicMessages,
        WireFormat::OpenAiChat => LlmApi::ChatCompletions,
        WireFormat::OpenAiResponses => LlmApi::Responses,
        WireFormat::CodexSearch => LlmApi::Search,
        WireFormat::Images => LlmApi::Images,
        WireFormat::Ollama(_) => LlmApi::OllamaChat,
        WireFormat::SystemOne => LlmApi::SystemOne,
        WireFormat::Decisions => LlmApi::Decisions,
    }
}

/// Cap on the raw payload stored per event, so the in-memory monitor ring
/// buffer stays bounded even for large exchanges. Also the most the
/// transports capture of a request body they forward without buffering.
pub(crate) const RAW_CAP: usize = 256 * 1024;

/// Cap on the *parsed* body stored per event.
///
/// Each half of an exchange is kept twice: as raw wire text and as a parsed
/// JSON tree for structured display. A `serde_json::Value` costs several times
/// its serialized length in heap, so an uncapped parsed body is by far the
/// largest thing an event holds — a long-context call runs to megabytes.
///
/// This is deliberately far below [`RAW_CAP`]: a body under the cap is kept
/// whole and renders as structured JSON, while anything larger is a
/// long-context call whose tree view would be unreadable anyway, and the raw
/// capture still holds its exact bytes. Together the two caps bound one
/// event's bodies to well under a megabyte.
const PARSED_CAP: usize = 64 * 1024;

/// Truncate a raw payload to the cap (on a char boundary), appending a marker.
/// The request half of a monitor event.
#[derive(Default)]
struct RequestParts {
    meta: RequestMeta,
    /// Parsed body, capped for display.
    display: Option<serde_json::Value>,
    /// Raw body text, capped.
    raw: Option<String>,
    tool_count: usize,
    /// The whole parsed body, for correlation.
    full: Option<serde_json::Value>,
}

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

/// The request bytes the monitor may parse: the buffered body, or the capture
/// of a pass-through body when it holds all of it undecoded.
fn request_bytes(ex: &ObservedExchange) -> Option<&[u8]> {
    ex.request_body.as_deref().or_else(|| {
        ex.request_capture
            .as_ref()
            .and_then(BoundedCapture::inspectable)
    })
}

/// Raw text shown for a request body that was forwarded without buffering.
fn raw_capture(capture: &BoundedCapture) -> Option<String> {
    if capture.total_len == 0 {
        return None;
    }
    if capture.encoded {
        return Some(format!(
            "[{} bytes forwarded with their Content-Encoding; not decoded]",
            capture.total_len
        ));
    }
    if capture.head.is_empty() {
        return None;
    }
    // The capture may end inside a multi-byte character; drop the fragment.
    let text = match std::str::from_utf8(&capture.head) {
        Ok(text) => text.to_string(),
        Err(e) if e.error_len().is_none() => {
            String::from_utf8_lossy(&capture.head[..e.valid_up_to()]).into_owned()
        }
        Err(_) => String::from_utf8_lossy(&capture.head).into_owned(),
    };
    let mut text = cap_raw(&text);
    if capture.truncated {
        text.push_str(&format!(
            "\n… [truncated {} bytes]",
            capture.total_len - capture.head.len() as u64
        ));
    }
    if !capture.complete {
        text.push_str("\n… [request body incomplete]");
    }
    Some(text)
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

    fn body_policy(&self, _head: &RequestHead<'_>) -> BodyPolicy {
        // Observe-only: the body is never needed to decide anything.
        BodyPolicy::PassThrough
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

    fn begin_passthrough(&self, exchange: &PassthroughExchange) -> Option<String> {
        Some(self.open_passthrough(exchange))
    }

    fn end_passthrough(&self, event_id: Option<String>, exchange: &PassthroughExchange) {
        self.close_passthrough(event_id, exchange);
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

    /// A pass-through exchange as the transport hands it over: no buffered
    /// body, a capture of what was forwarded.
    fn pass_through(body: &[u8], cap: usize) -> ObservedExchange {
        let mut ex = exchange();
        ex.request_body = None;
        ex.request_capture = Some(BoundedCapture {
            head: body[..cap.min(body.len())].to_vec(),
            total_len: body.len() as u64,
            truncated: body.len() > cap,
            complete: true,
            encoded: false,
        });
        ex
    }

    fn llm_call(store: &MonitorEventStore, id: &str) -> serde_json::Value {
        serde_json::to_value(store.get(id).expect("event")).unwrap()["data"].clone()
    }

    #[test]
    fn pass_through_requests_get_their_request_half_on_completion() {
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone());
        let body = exchange().request_body.unwrap();

        // Opened before a single body byte was read.
        let mut pending = pass_through(&body, RAW_CAP);
        pending.request_capture = Some(BoundedCapture::default());
        pending.status = None;
        pending.response_body = None;
        let id = it.emit_pending(&pending).expect("pending event");
        let data = llm_call(&store, &id);
        assert_eq!(data["model"], "");
        assert!(data["request_body"].is_null());
        assert!(data["raw_request"].is_null());

        let mut done = pass_through(&body, RAW_CAP);
        done.event_id = Some(id.clone());
        it.complete(&id, &done);
        let event = store.get(&id).unwrap();
        assert_eq!(event.status, EventStatus::Complete);
        // Correlated like a buffered request, from the same bytes.
        assert!(event.conversation.is_some());
        let data = llm_call(&store, &id);
        assert_eq!(data["model"], "claude-sonnet-4-20250514");
        assert_eq!(data["requested_model"], "claude-sonnet-4-20250514");
        assert_eq!(data["message_count"], 1);
        assert_eq!(data["has_tools"], true);
        assert_eq!(data["tool_count"], 1);
        assert_eq!(data["request_body"]["messages"][0]["content"], "hi");
        assert_eq!(data["raw_request"], String::from_utf8(body).unwrap());
        assert_eq!(data["input_tokens"], 5);
    }

    #[test]
    fn a_buffered_request_is_not_overwritten_on_completion() {
        // Only pass-through requests arrive late; a buffered one keeps the
        // request half its pending event was opened with.
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone());
        let id = it.emit_pending(&exchange()).expect("pending event");
        let mut done = exchange();
        done.request_body = Some(br#"{"model":"other","messages":[]}"#.to_vec());
        it.complete(&id, &done);
        let data = llm_call(&store, &id);
        assert_eq!(data["requested_model"], "claude-sonnet-4-20250514");
        assert_eq!(data["message_count"], 1);
    }

    #[test]
    fn truncated_captures_are_shown_raw_and_never_parsed() {
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone());
        let body = exchange().request_body.unwrap();
        it.record(&pass_through(&body, 20));
        let id = store.list(0, 10, None).events[0].id.clone();
        let data = llm_call(&store, &id);
        assert_eq!(
            data["raw_request"],
            format!(
                "{}\n… [truncated {} bytes]",
                String::from_utf8_lossy(&body[..20]),
                body.len() - 20
            )
        );
        assert!(data["request_body"].is_null());
        assert_eq!(data["message_count"], 0);
        // The model still comes from wherever it is known; here, nowhere.
        assert_eq!(data["model"], "");
    }

    #[test]
    fn capture_text_handles_every_shape() {
        let capture = |head: &[u8], total: u64, complete: bool, encoded: bool| BoundedCapture {
            head: head.to_vec(),
            total_len: total,
            truncated: total > head.len() as u64,
            complete,
            encoded,
        };
        // Nothing sent, and paths whose content is never captured.
        assert_eq!(raw_capture(&capture(b"", 0, true, false)), None);
        assert_eq!(raw_capture(&capture(b"", 4096, true, false)), None);
        // Whole.
        assert_eq!(
            raw_capture(&capture(b"{}", 2, true, false)).as_deref(),
            Some("{}")
        );
        // Cut inside a multi-byte character: the fragment is dropped, and the
        // marker counts bytes of the body, not of the text.
        let text = "naïve".as_bytes();
        assert_eq!(
            raw_capture(&capture(&text[..3], text.len() as u64, true, false)).as_deref(),
            Some("na\n… [truncated 3 bytes]")
        );
        // Bytes that are not text at all are shown lossily, not dropped.
        assert_eq!(
            raw_capture(&capture(b"a\xffb", 3, true, false)).as_deref(),
            Some("a\u{fffd}b")
        );
        // An upload that never finished.
        assert_eq!(
            raw_capture(&capture(b"{\"a\"", 4, false, false)).as_deref(),
            Some("{\"a\"\n… [request body incomplete]")
        );
        // Compressed bytes are described, never rendered.
        assert_eq!(
            raw_capture(&capture(b"\x28\xb5\x2f\xfd", 9000, true, true)).as_deref(),
            Some("[9000 bytes forwarded with their Content-Encoding; not decoded]")
        );
        // Only a whole identity-encoded body is offered for parsing.
        assert!(capture(b"{}", 2, true, false).inspectable().is_some());
        assert!(capture(b"{}", 2, false, false).inspectable().is_none());
        assert!(capture(b"{}", 3, true, false).inspectable().is_none());
        assert!(capture(b"{}", 2, true, true).inspectable().is_none());
    }

    #[test]
    fn inspected_passthroughs_report_the_forwarded_length_only() {
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone());
        let mut ex = pass_through(b"binary upload bytes", 0);
        ex.path = "/v1/files?purpose=secret-query".into();
        ex.status = None;
        ex.response_body = None;
        let id = it.emit_pending(&ex).expect("passthrough event");
        ex.status = Some(200);
        it.complete(&id, &ex);
        let event = store.get(&id).unwrap();
        assert_eq!(event.status, EventStatus::Complete);
        let MonitorEventData::ProxyPassthrough {
            bytes_sent, path, ..
        } = &event.data
        else {
            panic!("expected a passthrough event");
        };
        assert_eq!(*bytes_sent, Some(19));
        assert_eq!(path.as_deref(), Some("/v1/files"));
        let text = serde_json::to_string(&event).unwrap();
        assert!(!text.contains("binary upload") && !text.contains("secret-query"));
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
    fn oversized_bodies_are_capped_but_metadata_survives() {
        // A long-context call: the parsed bodies are what used to make one
        // event cost megabytes.
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone());

        let req = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{"role": "user", "content": "x".repeat(PARSED_CAP * 2)}],
            "tools": [{"name": "t"}, {"name": "u"}],
        });
        let resp = json!({
            "content": [{"type": "text", "text": "y".repeat(PARSED_CAP * 2)}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 5, "output_tokens": 3},
        });
        let mut ex = exchange();
        ex.request_body = Some(serde_json::to_vec(&req).unwrap());
        ex.response_body = Some(serde_json::to_vec(&resp).unwrap());

        it.record(&ex);

        let id = store.list(0, 10, None).events[0].id.clone();
        let event = store.get(&id).expect("event");
        let MonitorEventData::LlmCall {
            model,
            tool_count,
            message_count,
            request_body,
            response_body,
            input_tokens,
            output_tokens,
            ..
        } = &event.data
        else {
            panic!("expected an LLM call");
        };

        // Metadata is read off the full body before the cap applies.
        assert_eq!(model, "claude-sonnet-4-20250514");
        assert_eq!(*tool_count, 2);
        assert_eq!(*message_count, 1);
        assert_eq!(*input_tokens, Some(5));
        assert_eq!(*output_tokens, Some(3));

        // The bodies themselves are replaced by the truncation marker.
        assert_eq!(request_body["_truncated"], json!(true));
        assert_eq!(response_body.as_ref().unwrap()["_truncated"], json!(true));
        // Two raw captures at RAW_CAP plus two truncation markers at
        // PARSED_CAP — a fraction of the multi-megabyte event this used to be.
        assert!(lr_monitor::event_size(&event) < 2 * RAW_CAP + 2 * PARSED_CAP + 8192);
    }

    #[test]
    fn codex_lite_pending_and_completed_summaries_survive_truncation() {
        let store = Arc::new(MonitorEventStore::new(16));
        let interceptor = PassiveInterceptor::new(store.clone());
        let request = json!({"type":"response.create", "model":"codex-auto-review", "stream":true,
            "input":[{"type":"message", "role":"user", "content":[{"type":"input_text", "text":"Synthetic question"}]}],
            "tools":[{"description":"x".repeat(PARSED_CAP * 2)}]});
        let events = [
            json!({"type":"response.output_item.done", "output_index":0, "item":{"type":"message", "role":"assistant",
                "content":[{"type":"output_text", "text":"Synthetic answer"}]}}),
            json!({"type":"response.completed", "response":{"id":"resp_test", "model":"codex-auto-review", "status":"completed",
                "error":null, "output":[], "tools":[{"description":"x".repeat(PARSED_CAP * 2)}],
                "usage":{"input_tokens":100, "output_tokens":20}}}),
        ];
        let mut ex = exchange();
        ex.host = "chatgpt.com".into();
        ex.path = "/backend-api/codex/responses".into();
        ex.request_body = Some(serde_json::to_vec(&request).unwrap());
        ex.response_is_sse = true;
        ex.response_body = Some(
            events
                .iter()
                .map(|v| format!("data: {v}\n\n"))
                .collect::<String>()
                .into_bytes(),
        );
        let id = interceptor.emit_pending(&ex).unwrap();
        let pending = &store.list(0, 10, None).events[0];
        assert_eq!(pending.question, "Synthetic question");
        assert_eq!(pending.answer, "");
        interceptor.complete(&id, &ex);
        let summary = &store.list(0, 10, None).events[0];
        assert_eq!(summary.question, "Synthetic question");
        assert_eq!(summary.answer, "Synthetic answer");
        assert_eq!(summary.status, EventStatus::Complete);
        let event = store.get(&id).unwrap();
        let MonitorEventData::LlmCall {
            request_body,
            response_body,
            input_tokens,
            output_tokens,
            ..
        } = &event.data
        else {
            panic!("LLM call");
        };
        assert_eq!(request_body["_truncated"], true);
        assert_eq!(response_body.as_ref().unwrap()["_truncated"], true);
        assert_eq!(*input_tokens, Some(100));
        assert_eq!(*output_tokens, Some(20));
        assert!(lr_monitor::event_size(&event) < 2 * RAW_CAP + 2 * PARSED_CAP + 8192);
    }

    #[test]
    fn bodies_under_the_cap_are_stored_whole() {
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone());

        it.record(&exchange());

        let id = store.list(0, 10, None).events[0].id.clone();
        let event = store.get(&id).expect("event");
        let MonitorEventData::LlmCall { request_body, .. } = &event.data else {
            panic!("expected an LLM call");
        };
        assert_eq!(request_body["model"], json!("claude-sonnet-4-20250514"));
        assert!(request_body.get("_truncated").is_none());
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
    async fn non_llm_paths_on_inspected_hosts_record_a_passthrough() {
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone());

        let mut ex = exchange();
        ex.path = "/v1/organizations/me?token=secret".to_string();
        ex.port = 443;
        let id = it.begin(&ex).expect("passthrough event opens");
        assert_eq!(store.get(&id).unwrap().status, EventStatus::Pending);

        ex.event_id = Some(id.clone());
        it.on_response(&ex).await;

        let events = store.list(0, 100, None);
        assert_eq!(events.events.len(), 1, "completed in place, not duplicated");
        let full = store.get(&id).unwrap();
        assert_eq!(full.event_type, MonitorEventType::ProxyPassthrough);
        assert_eq!(full.status, EventStatus::Complete);
        match &full.data {
            MonitorEventData::ProxyPassthrough {
                mode,
                host,
                port,
                method,
                path,
                status_code,
                note,
                ..
            } => {
                assert_eq!(*mode, PassthroughMode::Inspected);
                assert_eq!(host, "api.anthropic.com");
                assert_eq!(*port, 443);
                assert_eq!(method.as_deref(), Some("POST"));
                // The query string is dropped — passthrough events never carry
                // request content of any kind.
                assert_eq!(path.as_deref(), Some("/v1/organizations/me"));
                assert_eq!(*status_code, Some(200));
                assert!(note.contains("Not an LLM call"));
                assert!(note.contains("HTTPS_PROXY"));
            }
            other => panic!("unexpected data: {other:?}"),
        }
    }

    #[tokio::test]
    async fn passthrough_events_never_carry_bodies() {
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone());

        let mut ex = exchange();
        ex.path = "/v1/complete".to_string();
        it.on_response(&ex).await;

        let events = store.list(0, 100, None);
        assert_eq!(events.events.len(), 1);
        let full = store.get(&events.events[0].id).unwrap();
        let json = serde_json::to_string(&full.data).unwrap();
        assert!(!json.contains("claude-sonnet"), "no request body: {json}");
        assert!(!json.contains("hello"), "no response body: {json}");
    }

    #[tokio::test]
    async fn reverse_proxy_non_llm_paths_are_not_recorded() {
        // The reverse proxy deliberately wraps a provider's own port, so its
        // non-LLM paths are expected traffic — not accidental egress.
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone());

        let mut ex = exchange();
        ex.path = "/api/tags".to_string();
        ex.source = ExchangeSource::ReverseProxy;
        assert!(it.begin(&ex).is_none());
        it.on_response(&ex).await;
        assert!(store.list(0, 100, None).events.is_empty());
    }

    #[test]
    fn tunnels_record_destination_and_volume_only() {
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone()).with_client_names(Arc::new(StaticNames));

        let mut ex = PassthroughExchange::tunnel("client-1", "github.com", 443);
        let id = it.begin_passthrough(&ex).expect("tunnel opens an event");
        assert_eq!(store.get(&id).unwrap().status, EventStatus::Pending);

        ex.bytes_sent = 4096;
        ex.bytes_received = 65536;
        ex.latency_ms = Some(1200);
        it.end_passthrough(Some(id.clone()), &ex);

        let full = store.get(&id).unwrap();
        assert_eq!(full.status, EventStatus::Complete);
        assert_eq!(full.duration_ms, Some(1200));
        assert_eq!(full.client_name.as_deref(), Some("Claude Code"));
        match &full.data {
            MonitorEventData::ProxyPassthrough {
                mode,
                host,
                port,
                method,
                path,
                bytes_sent,
                bytes_received,
                ..
            } => {
                assert_eq!(*mode, PassthroughMode::Tunnel);
                assert_eq!((host.as_str(), *port), ("github.com", 443));
                // TLS was never terminated, so nothing inside is knowable.
                assert_eq!(*method, None);
                assert_eq!(*path, None);
                assert_eq!((*bytes_sent, *bytes_received), (Some(4096), Some(65536)));
            }
            other => panic!("unexpected data: {other:?}"),
        }
        assert!(lr_monitor::generate_summary(&full).contains("github.com"));
    }

    #[test]
    fn a_failed_passthrough_is_an_error_event() {
        let store = Arc::new(MonitorEventStore::new(16));
        let it = PassiveInterceptor::new(store.clone());

        let mut ex = PassthroughExchange::tunnel("c", "unreachable.example", 443);
        let id = it.begin_passthrough(&ex).unwrap();
        ex.error = Some("upstream connect: refused".to_string());
        it.end_passthrough(Some(id.clone()), &ex);

        assert_eq!(store.get(&id).unwrap().status, EventStatus::Error);
    }

    #[tokio::test]
    async fn usage_ledger_records_classified_calls_once() {
        let store = Arc::new(MonitorEventStore::new(16));
        let usage = Arc::new(lr_usage::UsageTracker::new(Default::default()));
        let it = PassiveInterceptor::new(store).with_usage(usage.clone());
        let requests = |t: &lr_usage::UsageTracker| -> u64 {
            t.snapshot()
                .accounts
                .iter()
                .map(|a| a.spend.requests_30d)
                .sum()
        };

        // Unclassified (no auth seen): nothing to attribute.
        it.on_response(&exchange()).await;
        assert_eq!(requests(&usage), 0);

        let mut ex = exchange();
        ex.usage_account = Some(lr_usage::AccountRef::subscription("anthropic"));
        it.on_response(&ex).await;
        assert_eq!(requests(&usage), 1);

        // A duplicate hop and a failed call are not counted.
        let mut dup = ex.clone();
        dup.trace = Some(lr_types::RequestTrace::parse("abc;hop=2").unwrap());
        it.on_response(&dup).await;
        let mut failed = ex.clone();
        failed.status = Some(500);
        it.on_response(&failed).await;
        assert_eq!(requests(&usage), 1);
        let snap = usage.snapshot();
        assert_eq!(snap.accounts[0].id, "anthropic:subscription");
        assert_eq!(snap.accounts[0].spend.tokens_30d, 8);
    }

    #[tokio::test]
    async fn duplicate_hop_is_flagged_and_not_counted() {
        let store = Arc::new(MonitorEventStore::new(16));
        #[allow(deprecated)]
        let metrics = Arc::new(MetricsCollector::with_default_retention());
        let it = PassiveInterceptor::new(store.clone()).with_metrics(metrics.clone());
        let total_requests = |m: &MetricsCollector| -> u64 {
            let now = chrono::Utc::now();
            m.get_global_range(
                now - chrono::Duration::hours(1),
                now + chrono::Duration::hours(1),
            )
            .iter()
            .map(|p| p.requests)
            .sum()
        };

        let mut dup = exchange();
        dup.trace = Some(lr_types::RequestTrace::parse("abc;hop=2").unwrap());
        it.on_response(&dup).await;
        assert_eq!(total_requests(&metrics), 0, "duplicate hop not counted");

        let listed = store.list(0, 10, None);
        assert_eq!(listed.events.len(), 1);
        assert_eq!(listed.events[0].duplicate_hop, Some(2));
        assert_eq!(listed.events[0].trace_id.as_deref(), Some("abc"));
        let full = store.get(&listed.events[0].id).unwrap();
        match &full.data {
            MonitorEventData::LlmCall {
                duplicate_hop,
                trace_id,
                transformations_applied,
                ..
            } => {
                assert_eq!(*duplicate_hop, Some(2));
                assert_eq!(trace_id.as_deref(), Some("abc"));
                assert_eq!(
                    transformations_applied.as_deref(),
                    Some(&[DUPLICATE_HOP_LABEL.to_string()][..])
                );
            }
            other => panic!("unexpected data: {other:?}"),
        }

        // First hop: counted, carries the trace id, not flagged.
        let mut first = exchange();
        first.trace = Some(lr_types::RequestTrace::parse("abc;hop=1").unwrap());
        it.on_response(&first).await;
        assert_eq!(total_requests(&metrics), 1);
        let listed = store.list(0, 10, None);
        let newest = listed
            .events
            .iter()
            .find(|e| e.duplicate_hop.is_none())
            .unwrap();
        assert_eq!(newest.trace_id.as_deref(), Some("abc"));

        // The pending → complete path keeps the flag too.
        let id = it.begin(&dup).expect("pending event");
        let mut done = dup.clone();
        done.event_id = Some(id.clone());
        it.on_response(&done).await;
        assert_eq!(total_requests(&metrics), 1, "still not counted");
        let ev = store.get(&id).unwrap();
        assert!(matches!(
            ev.data,
            MonitorEventData::LlmCall {
                duplicate_hop: Some(2),
                ..
            }
        ));

        // A caller-provided wire header must still count after ingress stamps it.
        let mut headers = hyper::HeaderMap::new();
        headers.insert(lr_types::TRACE_HEADER, "external;hop=99".parse().unwrap());
        let mut forged = exchange();
        forged.trace = crate::stamp_trace(&mut headers, true);
        it.on_response(&forged).await;
        assert_eq!(
            total_requests(&metrics),
            2,
            "forged wire hop must be counted"
        );
    }
}
