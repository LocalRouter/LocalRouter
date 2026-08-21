//! Reverse proxy: impersonate a local LLM provider's original port.
//!
//! Where the MITM proxy in [`crate::transport`] intercepts *outbound* traffic a
//! client was already sending elsewhere, the reverse proxy takes over the
//! *inbound* address a local provider used to own. The provider is relocated to
//! a different port; LocalRouter binds the original one and forwards every
//! request through verbatim, teeing a bounded copy for the monitor.
//!
//! Consequences of that shape, and why it looks different from `transport.rs`:
//! - **No authentication.** The listener *is* the client's identity — apps
//!   pointed at `localhost:11434` send no credentials and must keep working
//!   untouched. One listener therefore belongs to exactly one client.
//! - **Plain HTTP, no TLS.** Local providers speak `http://` on loopback.
//! - **Every path is forwarded**, not just LLM endpoints: native `/api/*`,
//!   OpenAI-shaped `/v1/*`, health probes, model pulls. Anything the wrapped
//!   provider serves keeps working, monitored or not.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{HeaderName, HeaderValue};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, TcpStream};

use crate::error::ProxyError;

/// Cap on the bytes captured per request/response for monitoring (1 MiB).
const MAX_CAPTURE: usize = 1024 * 1024;
/// Cap on a buffered request body (16 MiB) — matches the gateway's body limit.
const MAX_REQUEST_BODY: usize = 16 * 1024 * 1024;

/// Headers that describe a single hop and must never be forwarded.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// One completed request/response through a reverse-proxy listener.
#[derive(Debug, Clone, Default)]
pub struct ReverseExchange {
    /// Client that owns this listener.
    pub client_id: String,
    /// Strategy attached to that client (for metrics attribution).
    pub strategy_id: String,
    /// Provider instance name being wrapped, when known.
    pub provider_instance: Option<String>,
    /// Upstream base the request was forwarded to (`http://127.0.0.1:11435`).
    pub upstream: String,
    pub method: String,
    /// Path with query, e.g. `/api/chat`.
    pub path: String,
    pub status: Option<u16>,
    pub request_body: Option<Vec<u8>>,
    pub response_body: Option<Vec<u8>>,
    /// Response was `text/event-stream` (OpenAI-style streaming).
    pub response_is_sse: bool,
    /// Response was newline-delimited JSON (Ollama-style streaming).
    pub response_is_ndjson: bool,
    pub started_at: Option<DateTime<Utc>>,
    pub latency_ms: Option<u64>,
    /// Set when the upstream could not be reached at all.
    pub error: Option<String>,
    /// Monitor event opened by [`ReverseRecorder::begin`], completed on end.
    pub event_id: Option<String>,
    /// Cross-hop trace stamped on the forwarded request; `hop > 1` marks a
    /// request an earlier LocalRouter hop already handled (not counted).
    pub trace: Option<lr_types::RequestTrace>,
}

/// Sink for observed exchanges. Implemented by the app layer (monitor +
/// metrics); kept as a trait so this crate stays free of app dependencies.
#[async_trait::async_trait]
pub trait ReverseRecorder: Send + Sync {
    /// Open a pending monitor event for an in-flight request. Returning `None`
    /// (the default) simply means no pending event is shown.
    fn begin(&self, _exchange: &ReverseExchange) -> Option<String> {
        None
    }

    /// Record a finished (or failed) exchange.
    async fn record(&self, exchange: ReverseExchange);
}

/// A no-op recorder, useful for tests and for running without monitoring.
pub struct NoopRecorder;

#[async_trait::async_trait]
impl ReverseRecorder for NoopRecorder {
    async fn record(&self, _exchange: ReverseExchange) {}
}

/// Identity carried by a listener and stamped onto every exchange.
#[derive(Debug, Clone, Default)]
pub struct ReverseClient {
    pub client_id: String,
    pub strategy_id: String,
    pub provider_instance: Option<String>,
}

/// A reverse-proxy listener's configuration and collaborators.
pub struct ReverseProxy {
    /// `scheme://host:port` of the relocated provider, no trailing slash.
    upstream: String,
    upstream_host: String,
    upstream_port: u16,
    client: ReverseClient,
    recorder: Arc<dyn ReverseRecorder>,
    /// Live "duplicate request detection" flag shared with the app.
    dedupe_enabled: Arc<std::sync::atomic::AtomicBool>,
}

impl ReverseProxy {
    /// Build a reverse proxy forwarding to `upstream_url` (plain `http://`).
    pub fn new(
        upstream_url: &str,
        client: ReverseClient,
        recorder: Arc<dyn ReverseRecorder>,
    ) -> Result<Self, ProxyError> {
        let (host, port) = parse_http_upstream(upstream_url)?;
        Ok(Self {
            upstream: format!("http://{host}:{port}"),
            upstream_host: host,
            upstream_port: port,
            client,
            recorder,
            dedupe_enabled: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        })
    }

    /// Share the app's live "duplicate request detection" flag (defaults to
    /// enabled).
    pub fn with_dedupe_flag(mut self, flag: Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.dedupe_enabled = flag;
        self
    }

    /// The normalized upstream base URL.
    pub fn upstream(&self) -> &str {
        &self.upstream
    }

    /// Bind a listener on `host:port` (port 0 = OS-assigned).
    pub async fn bind(host: &str, port: u16) -> Result<TcpListener, ProxyError> {
        Ok(TcpListener::bind((host, port)).await?)
    }

    /// Accept until `shutdown` resolves; one task per connection.
    pub async fn serve(
        self: Arc<Self>,
        listener: TcpListener,
        shutdown: impl std::future::Future<Output = ()> + Send,
    ) {
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = &mut shutdown => {
                    tracing::info!(
                        client_id = %self.client.client_id,
                        "reverse proxy accept loop shutting down"
                    );
                    break;
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _peer)) => {
                            let this = self.clone();
                            tokio::spawn(async move { this.serve_conn(stream).await });
                        }
                        Err(e) => tracing::warn!("reverse proxy accept error: {e}"),
                    }
                }
            }
        }
    }

    async fn serve_conn(self: Arc<Self>, stream: TcpStream) {
        // Nagle off: streamed tokens should reach the app as they arrive.
        let _ = stream.set_nodelay(true);
        let service = service_fn(move |req: Request<Incoming>| {
            let this = self.clone();
            async move { Ok::<_, Infallible>(this.forward(req).await) }
        });
        if let Err(e) = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await
        {
            // Clients disconnecting mid-stream is normal; log at debug.
            tracing::debug!("reverse proxy connection ended: {e}");
        }
    }

    /// Forward one request to the relocated provider, teeing the response.
    async fn forward(&self, req: Request<Incoming>) -> Response<BoxedBody> {
        let started = Instant::now();
        let started_at = Utc::now();
        let method = req.method().to_string();
        let path = req
            .uri()
            .path_and_query()
            .map(|p| p.as_str().to_string())
            .unwrap_or_else(|| "/".to_string());

        let (mut parts, body) = req.into_parts();

        let req_bytes = match http_body_util::Limited::new(body, MAX_REQUEST_BODY)
            .collect()
            .await
        {
            Ok(c) => c.to_bytes(),
            Err(_) => {
                return error_response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "request body too large or unreadable",
                )
            }
        };

        // Rewrite for the new hop: strip per-hop headers, point Host at the
        // relocated provider, and ask for an uncompressed response so the
        // captured copy is parseable. `content-length` is re-derived by hyper.
        for name in HOP_BY_HOP {
            parts.headers.remove(*name);
        }
        parts.headers.remove(hyper::header::ACCEPT_ENCODING);
        parts.headers.remove(hyper::header::CONTENT_LENGTH);
        if let Ok(host) = HeaderValue::from_str(&self.authority()) {
            parts.headers.insert(hyper::header::HOST, host);
        }
        // The upstream connection is per-request, so send an origin-form URI.
        parts.uri = match path.parse() {
            Ok(uri) => uri,
            Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid request path"),
        };
        // Cross-hop trace: recognize a request an earlier LocalRouter hop
        // already handled, and mark the forwarded copy for the next hop.
        let trace = crate::stamp_trace(
            &mut parts.headers,
            self.dedupe_enabled
                .load(std::sync::atomic::Ordering::Relaxed),
        );

        let mut base = ReverseExchange {
            client_id: self.client.client_id.clone(),
            strategy_id: self.client.strategy_id.clone(),
            provider_instance: self.client.provider_instance.clone(),
            upstream: self.upstream.clone(),
            method,
            path,
            request_body: (!req_bytes.is_empty()).then(|| req_bytes.to_vec()),
            started_at: Some(started_at),
            trace,
            ..Default::default()
        };
        base.event_id = self.recorder.begin(&base);

        let stream =
            match TcpStream::connect((self.upstream_host.as_str(), self.upstream_port)).await {
                Ok(s) => s,
                Err(e) => {
                    return self.fail(
                        base,
                        started,
                        format!("cannot reach {}: {e}", self.upstream),
                    )
                }
            };
        let _ = stream.set_nodelay(true);

        let (mut sender, conn) =
            match hyper::client::conn::http1::handshake(TokioIo::new(stream)).await {
                Ok(pair) => pair,
                Err(e) => return self.fail(base, started, format!("upstream handshake: {e}")),
            };
        tokio::spawn(async move {
            let _ = conn.await;
        });

        let up_req = Request::from_parts(parts, Full::new(req_bytes));
        let resp = match sender.send_request(up_req).await {
            Ok(r) => r,
            Err(e) => return self.fail(base, started, format!("upstream request: {e}")),
        };

        let (mut rparts, rbody) = resp.into_parts();
        let content_type = rparts
            .headers
            .get(hyper::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        base.status = Some(rparts.status.as_u16());
        base.response_is_sse = content_type.contains("text/event-stream");
        base.response_is_ndjson = content_type.contains("application/x-ndjson")
            || content_type.contains("application/jsonlines");
        for name in HOP_BY_HOP {
            rparts.headers.remove(*name);
        }
        // Mark the hop so a curious user can tell the traffic went through
        // LocalRouter — the only header we add.
        rparts.headers.insert(
            HeaderName::from_static("x-localrouter-reverse-proxy"),
            HeaderValue::from_static("1"),
        );

        let recorder = self.recorder.clone();
        let mut recorded = base;
        let on_end: Box<dyn FnOnce(Vec<u8>) + Send> = Box::new(move |bytes| {
            recorded.response_body = (!bytes.is_empty()).then_some(bytes);
            recorded.latency_ms = Some(started.elapsed().as_millis() as u64);
            tokio::spawn(async move {
                recorder.record(recorded).await;
            });
        });

        let tapped = crate::tap::TappedBody::new(rbody, MAX_CAPTURE, on_end);
        Response::from_parts(rparts, BodyExt::boxed_unsync(tapped))
    }

    fn authority(&self) -> String {
        format!("{}:{}", self.upstream_host, self.upstream_port)
    }

    /// Record a failed attempt and answer the caller with a diagnosable 502.
    /// The message names the upstream, because the overwhelmingly likely cause
    /// is that the wrapped provider was never relocated (or is not running).
    fn fail(&self, mut ex: ReverseExchange, started: Instant, msg: String) -> Response<BoxedBody> {
        tracing::warn!(client_id = %ex.client_id, "reverse proxy: {msg}");
        ex.status = Some(StatusCode::BAD_GATEWAY.as_u16());
        ex.latency_ms = Some(started.elapsed().as_millis() as u64);
        ex.error = Some(msg.clone());
        let recorder = self.recorder.clone();
        tokio::spawn(async move {
            recorder.record(ex).await;
        });
        error_response(StatusCode::BAD_GATEWAY, &msg)
    }
}

type BoxedBody = http_body_util::combinators::UnsyncBoxBody<Bytes, std::io::Error>;

/// Split `http://host:port` into its parts, rejecting anything else.
///
/// Public so callers that need the same host/port view of a configured upstream
/// (status probes, setup UI) parse it exactly the way the data path does,
/// rather than re-deriving it and disagreeing on edge cases like a trailing
/// `/v1` path.
pub fn parse_http_upstream(url: &str) -> Result<(String, u16), ProxyError> {
    let url = url.trim().trim_end_matches('/');
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        ProxyError::Config(format!(
            "reverse-proxy upstream must be a plain http:// URL (got '{url}')"
        ))
    })?;
    // Ignore any path component — forwarding always uses the request's own path.
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty() {
        return Err(ProxyError::Config(
            "reverse-proxy upstream has no host".into(),
        ));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => {
            let port: u16 = p.parse().map_err(|_| {
                ProxyError::Config(format!("invalid reverse-proxy upstream port '{p}'"))
            })?;
            (h.to_string(), port)
        }
        None => (authority.to_string(), 80),
    };
    if host.is_empty() || port == 0 {
        return Err(ProxyError::Config(format!(
            "invalid reverse-proxy upstream '{url}'"
        )));
    }
    Ok((host, port))
}

fn error_response(status: StatusCode, msg: &str) -> Response<BoxedBody> {
    let body = serde_json::json!({
        "error": {
            "message": format!("LocalRouter reverse proxy: {msg}"),
            "type": "reverse_proxy_error",
        }
    })
    .to_string();
    Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .body(
            Full::new(Bytes::from(body))
                .map_err(|e: Infallible| match e {})
                .boxed_unsync(),
        )
        .expect("static response builds")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_http_upstreams() {
        assert_eq!(
            parse_http_upstream("http://127.0.0.1:11435").unwrap(),
            ("127.0.0.1".to_string(), 11435)
        );
        assert_eq!(
            parse_http_upstream("http://localhost:1235/v1/").unwrap(),
            ("localhost".to_string(), 1235)
        );
        assert_eq!(
            parse_http_upstream("http://example.test").unwrap(),
            ("example.test".to_string(), 80)
        );
    }

    #[test]
    fn rejects_non_http_upstreams() {
        assert!(parse_http_upstream("https://127.0.0.1:11435").is_err());
        assert!(parse_http_upstream("127.0.0.1:11435").is_err());
        assert!(parse_http_upstream("http://").is_err());
        assert!(parse_http_upstream("http://host:abc").is_err());
        assert!(parse_http_upstream("http://host:0").is_err());
    }
}
