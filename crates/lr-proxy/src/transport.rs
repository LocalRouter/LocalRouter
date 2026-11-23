//! The MITM data-path: parse the proxy request, authenticate, then either
//! blind-tunnel or terminate TLS and ferry HTTP/1.1 while teeing traffic to the
//! interceptor.
//!
//! Two request forms arrive here: `CONNECT host:port` (every HTTPS proxy
//! request) and absolute-form plain HTTP (`GET http://host/path`). Only
//! allow-listed LLM hosts are decrypted; **everything else is forwarded
//! verbatim** and reported to the interceptor as a passthrough, because
//! `HTTPS_PROXY` is process-wide and drags a tool's git/package-manager/
//! telemetry traffic through here too.

use std::convert::Infallible;
use std::sync::Arc;

use base64::Engine;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rustls_pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::error::ProxyError;
use crate::interceptor::{
    BodyPolicy, ClientCtx, ConnectDecision, ObservedExchange, PassthroughExchange,
    ProxyInterceptor, RequestAction, RequestHead,
};
use crate::resolver::ClientResolver;
use crate::tap::{RequestCaptureHandle, RequestTap};
use crate::tls::TlsFactory;
use crate::{websocket, wire};

/// Cap on the bytes captured per request/response for monitoring (1 MiB).
const MAX_CAPTURE: usize = 1024 * 1024;
/// Cap on the CONNECT header block, to bound a misbehaving client.
const MAX_CONNECT_HEADER: usize = 16 * 1024;
/// Cap on a buffered request body. Matches the websocket message cap: when
/// Codex's websocket refuses a request it resends the same body over HTTPS,
/// and long Codex sessions send whole conversations of tens of MiB.
const MAX_REQUEST_BODY: usize = websocket::MAX_MESSAGE;

/// Shared collaborators for handling proxied connections.
#[derive(Clone)]
pub struct ProxyContext {
    pub interceptor: Arc<dyn ProxyInterceptor>,
    pub resolver: Arc<dyn ClientResolver>,
    pub tls: Arc<TlsFactory>,
    /// Whether to stamp `X-LocalRouter-Trace` on forwarded requests and pass
    /// already-traced (duplicate-hop) requests through uncounted. Shared with
    /// the app so the setting can change without restarting the listener.
    pub dedupe_enabled: Arc<std::sync::atomic::AtomicBool>,
    /// Usage-limit tracking: reads rate-limit headers and usage responses.
    pub usage: Option<Arc<lr_usage::UsageTracker>>,
    /// Where held request bytes spill (encrypted) beyond RAM. Without it a
    /// held body is limited to the buffered request limit.
    pub spill: Option<Arc<crate::spill::SpillConfig>>,
    /// Upstreams that get transformed request bodies with a Content-Length.
    pub length_required: Arc<crate::framing::LengthRequiredHosts>,
}

/// A parsed `CONNECT` request line + relevant headers.
struct ConnectReq {
    host: String,
    port: u16,
    proxy_auth: Option<String>,
}

/// A parsed absolute-form plain-HTTP proxy request (`GET http://host/p HTTP/1.1`),
/// already rewritten into the origin-form head we send upstream.
struct PlainReq {
    method: String,
    host: String,
    port: u16,
    /// Origin-form target, query included (as sent upstream).
    target: String,
    proxy_auth: Option<String>,
    /// The full request head to write to the origin, verbatim apart from the
    /// rewritten request line and the removed hop-by-hop proxy headers.
    head: Vec<u8>,
}

/// What a client sent on a fresh proxy connection.
enum ProxyRequest {
    Connect(ConnectReq),
    Plain(PlainReq),
}

impl ProxyRequest {
    fn proxy_auth(&self) -> Option<&str> {
        match self {
            Self::Connect(c) => c.proxy_auth.as_deref(),
            Self::Plain(p) => p.proxy_auth.as_deref(),
        }
    }
}

/// Entry point: drive one accepted TCP connection to completion.
pub async fn handle_connection(client: TcpStream, ctx: Arc<ProxyContext>) {
    if let Err(e) = handle_inner(client, ctx).await {
        tracing::debug!("proxy connection ended: {e}");
    }
}

async fn handle_inner(mut client: TcpStream, ctx: Arc<ProxyContext>) -> Result<(), ProxyError> {
    let request = read_request_head(&mut client).await?;

    // Authenticate via Proxy-Authorization (Basic client_id:secret).
    let client_ctx = match request
        .proxy_auth()
        .and_then(parse_basic_auth)
        .and_then(|(id, secret)| ctx.resolver.resolve(&id, &secret))
    {
        Some(c) => c,
        None => {
            write_status(
                &mut client,
                "407 Proxy Authentication Required",
                "Proxy-Authenticate: Basic realm=\"LocalRouter\"\r\nConnection: close\r\n",
            )
            .await?;
            return Ok(());
        }
    };

    let host = match &request {
        ProxyRequest::Connect(c) => c.host.clone(),
        ProxyRequest::Plain(p) => p.host.clone(),
    };
    // The same policy gate for both forms — a client that isn't in a proxy mode
    // may not use the proxy at all, whatever it asks for.
    let decision = ctx.interceptor.on_connect(&host, &client_ctx);
    if let ConnectDecision::Reject(reason) = decision {
        tracing::info!("proxy rejected {host}: {reason}");
        write_status(&mut client, "403 Forbidden", "").await?;
        return Ok(());
    }

    let connect = match request {
        // Plain HTTP is never an LLM call for us (LLM APIs are HTTPS): forward
        // it untouched so the tool that sent it keeps working.
        ProxyRequest::Plain(plain) => return forward_plain(client, plain, client_ctx, ctx).await,
        ProxyRequest::Connect(c) => c,
    };

    match decision {
        ConnectDecision::Tunnel => tunnel(client, &connect, &client_ctx, &ctx).await,
        ConnectDecision::Mitm => mitm(client, connect, client_ctx, ctx).await,
        // Handled above.
        ConnectDecision::Reject(_) => Ok(()),
    }
}

/// Blind byte tunnel — no decryption. The destination is recorded as a
/// passthrough (host/port and byte volume only) so accidental proxying is
/// visible; nothing inside the tunnel is ever read.
async fn tunnel(
    mut client: TcpStream,
    connect: &ConnectReq,
    client_ctx: &ClientCtx,
    ctx: &Arc<ProxyContext>,
) -> Result<(), ProxyError> {
    let started = std::time::Instant::now();
    let mut ex = PassthroughExchange::tunnel(&client_ctx.client_id, &connect.host, connect.port);
    let event_id = ctx.interceptor.begin_passthrough(&ex);

    // Connect upstream *before* confirming the tunnel, so an unreachable host
    // surfaces as a 502 the client understands rather than a tunnel that dies
    // the moment it is used.
    let mut upstream = match TcpStream::connect((connect.host.as_str(), connect.port)).await {
        Ok(u) => u,
        Err(e) => {
            ex.error = Some(format!("upstream connect: {e}"));
            ex.latency_ms = Some(started.elapsed().as_millis() as u64);
            ctx.interceptor.end_passthrough(event_id, &ex);
            write_status(&mut client, "502 Bad Gateway", "Connection: close\r\n").await?;
            return Err(e.into());
        }
    };
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;

    let copied = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    finish_passthrough(ctx, event_id, ex, copied, started);
    Ok(())
}

/// Forward an absolute-form plain-HTTP request to the origin verbatim.
///
/// Some tools resolve `HTTPS_PROXY`/`ALL_PROXY` for `http://` URLs too; before
/// this path existed those requests were dropped on the floor. The head is
/// rewritten to origin-form with the hop-by-hop proxy headers removed — the
/// minimum a forward proxy must do — and the rest of the connection is copied
/// byte-for-byte in both directions.
async fn forward_plain(
    mut client: TcpStream,
    plain: PlainReq,
    client_ctx: ClientCtx,
    ctx: Arc<ProxyContext>,
) -> Result<(), ProxyError> {
    let started = std::time::Instant::now();
    let mut ex = PassthroughExchange {
        client_id: client_ctx.client_id.clone(),
        mode: lr_monitor::PassthroughMode::Http,
        host: plain.host.clone(),
        port: plain.port,
        method: Some(plain.method.clone()),
        path: Some(PassthroughExchange::strip_query(&plain.target)),
        ..Default::default()
    };
    let event_id = ctx.interceptor.begin_passthrough(&ex);

    let mut upstream = match TcpStream::connect((plain.host.as_str(), plain.port)).await {
        Ok(u) => u,
        Err(e) => {
            ex.error = Some(format!("upstream connect: {e}"));
            ex.latency_ms = Some(started.elapsed().as_millis() as u64);
            ctx.interceptor.end_passthrough(event_id, &ex);
            write_status(&mut client, "502 Bad Gateway", "Connection: close\r\n").await?;
            return Err(e.into());
        }
    };
    if let Err(e) = upstream.write_all(&plain.head).await {
        ex.error = Some(format!("upstream write: {e}"));
        ex.latency_ms = Some(started.elapsed().as_millis() as u64);
        ctx.interceptor.end_passthrough(event_id, &ex);
        return Err(e.into());
    }
    ex.bytes_sent = plain.head.len() as u64;

    let copied = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    finish_passthrough(&ctx, event_id, ex, copied, started);
    Ok(())
}

/// Close out a passthrough event with the copied byte counts and duration.
///
/// A peer hanging up mid-copy is how connections normally end (TLS close
/// without `close_notify`, a client Ctrl-C); those are not reported as errors,
/// so the monitor only flags passthroughs that genuinely failed.
fn finish_passthrough(
    ctx: &Arc<ProxyContext>,
    event_id: Option<String>,
    mut ex: PassthroughExchange,
    copied: std::io::Result<(u64, u64)>,
    started: std::time::Instant,
) {
    match copied {
        Ok((up, down)) => {
            ex.bytes_sent += up;
            ex.bytes_received += down;
        }
        Err(e) if is_normal_disconnect(&e) => {}
        Err(e) => ex.error = Some(e.to_string()),
    }
    ex.latency_ms = Some(started.elapsed().as_millis() as u64);
    ctx.interceptor.end_passthrough(event_id, &ex);
}

/// Whether an I/O error is just the other end going away.
fn is_normal_disconnect(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::*;
    matches!(
        e.kind(),
        ConnectionReset | ConnectionAborted | BrokenPipe | UnexpectedEof | NotConnected
    )
}

/// Terminate the client's TLS with a forged leaf, then serve HTTP/1.1 and ferry
/// each request to a fresh upstream connection (so http/1.1 request framing is
/// never shared across concurrent requests).
async fn mitm(
    mut client: TcpStream,
    connect: ConnectReq,
    client_ctx: ClientCtx,
    ctx: Arc<ProxyContext>,
) -> Result<(), ProxyError> {
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;

    let server_cfg = ctx.tls.server_config_for(&connect.host)?;
    let client_tls = TlsAcceptor::from(server_cfg)
        .accept(client)
        .await
        .map_err(|e| ProxyError::Tls(format!("client TLS accept: {e}")))?;

    let host = Arc::new(connect.host);
    let port = connect.port;
    let client_id = Arc::new(client_ctx.client_id);
    let strategy_id = Arc::new(client_ctx.strategy_id);

    let service = service_fn(move |req: Request<Incoming>| {
        let ctx = ctx.clone();
        let host = host.clone();
        let client_id = client_id.clone();
        let strategy_id = strategy_id.clone();
        async move {
            Ok::<_, Infallible>(proxy_request(req, ctx, host, port, client_id, strategy_id).await)
        }
    });

    hyper::server::conn::http1::Builder::new()
        .serve_connection(TokioIo::new(client_tls), service)
        .with_upgrades()
        .await
        .map_err(|e| ProxyError::Tls(format!("client HTTP/1.1 serve: {e}")))?;
    Ok(())
}

type BoxedBody = http_body_util::combinators::UnsyncBoxBody<Bytes, std::io::Error>;
/// The request body sent upstream: buffered bytes, or the client's body
/// forwarded as it arrives.
type UpstreamBody =
    http_body_util::combinators::UnsyncBoxBody<Bytes, Box<dyn std::error::Error + Send + Sync>>;

/// Handle one decrypted request: forward it upstream verbatim, tee the response.
async fn proxy_request(
    mut req: Request<Incoming>,
    ctx: Arc<ProxyContext>,
    host: Arc<String>,
    port: u16,
    client_id: Arc<String>,
    strategy_id: Arc<String>,
) -> Response<BoxedBody> {
    let started = std::time::Instant::now();
    let method = req.method().to_string();
    let path = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());

    // A websocket upgrade (e.g. Codex's Responses transport). Grab the
    // client-side upgrade handle now — the upgraded byte stream only becomes
    // usable after we've returned the upstream's 101 to the client.
    let is_ws_upgrade = req
        .headers()
        .get(hyper::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .any(|p| p.trim().eq_ignore_ascii_case("websocket"))
        });
    let client_upgrade = is_ws_upgrade.then(|| hyper::upgrade::on(&mut req));

    let (mut parts, body) = req.into_parts();
    // The client's declared length (Content-Length), before any is read.
    let client_length = http_body::Body::size_hint(&body).exact();

    // An absent Accept-Encoding permits any coding. Explicitly request
    // identity so the response can be inspected and restored while streaming.
    parts.headers.insert(
        hyper::header::ACCEPT_ENCODING,
        hyper::header::HeaderValue::from_static("identity"),
    );
    // hyper derives the framing from the forwarded body: a buffered or
    // rewritten body gets a fresh Content-Length, and a pass-through body
    // keeps the client's through its exact size hint.
    parts.headers.remove(hyper::header::CONTENT_LENGTH);
    if is_ws_upgrade {
        // Refuse websocket extension negotiation (permessage-deflate) — the
        // websocket analog of the Accept-Encoding strip above, so every frame
        // payload stays parseable plaintext. Clients fall back to
        // uncompressed frames per RFC 6455.
        parts
            .headers
            .remove(hyper::header::SEC_WEBSOCKET_EXTENSIONS);
    }

    // Cross-hop trace: recognize a request an earlier LocalRouter hop already
    // handled, and mark the forwarded copy for the next hop.
    let trace = crate::stamp_trace(
        &mut parts.headers,
        ctx.dedupe_enabled
            .load(std::sync::atomic::Ordering::Relaxed),
    );

    // Request headers for usage tracking: how the request authenticated says
    // which account (subscription vs API key) its rate-limit headers belong to.
    let usage_request_headers = ctx.usage.as_ref().map(|_| parts.headers.clone());
    let observe_usage = |headers: &hyper::HeaderMap| -> Option<lr_usage::AccountRef> {
        let tracker = ctx.usage.as_ref()?;
        tracker.observe_response(
            &host,
            &path,
            usage_request_headers.as_ref()?,
            headers,
            lr_usage::DataSource::ProxyHeaders,
        )
    };

    // Decide from the head alone whether the body must be buffered. A
    // websocket upgrade keeps the buffered path: its messages are judged
    // inside the connection.
    let policy = if is_ws_upgrade {
        BodyPolicy::Buffered
    } else {
        ctx.interceptor.body_policy(&RequestHead {
            client_id: &client_id,
            strategy_id: &strategy_id,
            host: &host,
            port,
            method: &method,
            path: &path,
            headers: &parts.headers,
            trace: trace.as_ref(),
            provider_override: None,
            source: crate::interceptor::ExchangeSource::Proxy,
        })
    };
    // Content is only ever captured for LLM calls; other paths record a length.
    let capture_cap = if wire::detect(&path).is_some() {
        crate::passive::RAW_CAP
    } else {
        0
    };
    let encoded = crate::tap::content_encoded(&parts.headers);

    // Base exchange (request half); response fields filled at stream end.
    let mut base = ObservedExchange {
        client_id: (*client_id).clone(),
        session_id: Some(uuid::Uuid::new_v4().to_string()),
        secret_session: [
            "x-session-id",
            "x-session-affinity",
            "session_id",
            "session-id",
        ]
        .iter()
        .find_map(|name| parts.headers.get(*name))
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned),
        strategy_id: (*strategy_id).clone(),
        host: (*host).clone(),
        port,
        method,
        path: path.clone(),
        trace,
        ..Default::default()
    };

    let mut req_bytes = Bytes::new();
    let mut request_tap = None;
    let mut capture: Option<RequestCaptureHandle> = None;
    let mut streamed: Option<Box<crate::request_stream::StreamStart>> = None;
    match policy {
        BodyPolicy::Reject(status, message) => {
            // A client that waits for `100 Continue` has sent no body yet and
            // gets the refusal at once. Any other client is mid-upload: its
            // body is read and discarded so it can read the refusal.
            if !expects_continue(&parts.headers) {
                base.request_capture = Some(
                    crate::tap::drain(body, capture_cap, MAX_REQUEST_BODY as u64, encoded).await,
                );
            }
            return reject(
                &ctx,
                base,
                started,
                RequestAction::reject_json(status, &message),
            );
        }
        BodyPolicy::PassThrough => {
            // If the upstream fails or answers before the upload ended, the
            // rest is drained so the client can read the answer.
            let (tap, handle) = RequestTap::new(body, capture_cap, encoded);
            let tap = tap.drain_when_abandoned();
            base.request_capture = Some(handle.snapshot());
            request_tap = Some(tap);
            capture = Some(handle);
        }
        // A body above the stream lag is forwarded while it uploads, if the
        // firewall agrees once its model is known; anything smaller, or
        // anything the pipeline cannot stream, ends up buffered as below.
        BodyPolicy::Stream(stream_policy) => {
            match prepare_stream(
                body,
                &parts.headers,
                stream_policy,
                &ctx,
                &base,
                capture_cap,
                client_length,
            )
            .await
            {
                Prepared::Buffered(bytes) => {
                    req_bytes = bytes;
                    let inspected =
                        crate::request_body::inspect(&path, &parts.headers, req_bytes.clone())
                            .await;
                    base.request_body = inspected
                        .as_ref()
                        .ok()
                        .filter(|b| !b.is_empty())
                        .map(|b| b.to_vec());
                    base.request_body_error = inspected.err();
                }
                Prepared::Failed { too_large, error } => {
                    return body_read_failed(&ctx, base, started, too_large, error)
                }
                Prepared::Rejected { action, capture } => {
                    base.request_capture = Some(capture);
                    return reject(&ctx, base, started, action);
                }
                Prepared::Stream(start) => streamed = Some(start),
            }
        }
        // Buffer the request body so we can both forward and inspect it.
        // Bodies above the inspection limit are still forwarded;
        // `request_body::inspect` and the firewall decide how to treat what
        // they cannot fully inspect.
        BodyPolicy::Buffered => {
            req_bytes = match http_body_util::Limited::new(body, MAX_REQUEST_BODY)
                .collect()
                .await
            {
                Ok(c) => c.to_bytes(),
                Err(e) => {
                    let too_large = e.is::<http_body_util::LengthLimitError>();
                    return body_read_failed(&ctx, base, started, too_large, e.to_string());
                }
            };
            let inspected =
                crate::request_body::inspect(&path, &parts.headers, req_bytes.clone()).await;
            base.request_body = inspected
                .as_ref()
                .ok()
                .filter(|b| !b.is_empty())
                .map(|b| b.to_vec());
            base.request_body_error = inspected.err();
        }
    }

    // Firewall: forward, rewrite, or reject. (Passive returns Forward.)
    // A websocket upgrade carries no LLM request — the firewall evaluates each
    // websocket message inside the connection instead (see websocket::WsSession),
    // so don't ask it to judge the empty upgrade body (Ask-mode would pop a
    // model-less approval dialog).
    // A streamed body was judged by `open_stream`; its session protects it.
    let action = if streamed.is_some() || (is_ws_upgrade && wire::detect(&base.path).is_some()) {
        RequestAction::Forward
    } else {
        ctx.interceptor.on_request(&base).await
    };
    let mut secret_vault = None;
    let mut progress: Option<crate::request_stream::StreamProgress> = None;
    let mut stream_ready = None;
    let forward_body: UpstreamBody = if let Some(start) = streamed.take() {
        // A tee forwards the client's bytes with their encoding and length.
        // Otherwise the forwarded body is decoded and possibly rewritten:
        // hyper frames it as chunked, or with the Content-Length of the
        // held output for a host that requires one.
        if !matches!(start.output, crate::stream_driver::Output::Tee { .. }) {
            crate::request_body::rewritten(&mut parts.headers);
        } else {
            // A held tee is sent with a length the client may not have
            // declared: hyper frames the body from its size hint, and an
            // explicit `Transfer-Encoding: chunked` would override that.
            parts.headers.remove(hyper::header::TRANSFER_ENCODING);
        }
        secret_vault = start.vault.clone();
        base.request_stream = Some(crate::request_stream::streamed_request(
            start.format,
            start.pipeline.metadata(),
            None,
        ));
        base.request_capture = Some(Default::default());
        let handle = (*start).spawn();
        progress = Some(handle.progress);
        stream_ready = Some(handle.ready);
        handle.body.boxed_unsync()
    } else {
        match action {
            RequestAction::Forward => match request_tap.take() {
                Some(tap) => tap.boxed_unsync(),
                None => buffered_body(req_bytes),
            },
            // A pass-through body was never read, so there is nothing a rewrite
            // could have been derived from. Refuse rather than forward bytes the
            // interceptor meant to replace.
            RequestAction::Replace(_) | RequestAction::Protect { .. } if capture.is_some() => {
                tracing::error!("proxy: interceptor rewrote a pass-through request; refusing it");
                return reject(
                    &ctx,
                    base,
                    started,
                    RequestAction::reject_json(
                        StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                        "Request policy changed while the request was being forwarded",
                    ),
                );
            }
            RequestAction::Replace(new_body) => {
                crate::request_body::rewritten(&mut parts.headers);
                base.request_body = Some(new_body.clone());
                buffered_body(Bytes::from(new_body))
            }
            RequestAction::Protect { body, vault } => {
                crate::request_body::rewritten(&mut parts.headers);
                // Monitoring sees only the masked outbound request.
                base.request_body = Some(body.clone());
                secret_vault = Some(vault);
                buffered_body(Bytes::from(body))
            }
            // Record the blocked call so it shows in the monitor, then answer
            // the client directly without ever contacting the upstream.
            rejected @ RequestAction::Reject { .. } => {
                return reject(&ctx, base, started, rejected)
            }
        }
    };

    // The firewall allowed the request through — open a pending monitor event now
    // so the in-flight call is visible while we wait on the upstream. The id is
    // threaded into `recorded` so the response half completes it (not a 2nd event).
    // Websocket upgrades don't get an HTTP-level event: the relay opens one
    // monitor event per request/response cycle inside the connection instead.
    let event_id = if is_ws_upgrade {
        None
    } else {
        ctx.interceptor.begin(&base)
    };

    let pending_ctx = ctx.clone();
    let pending_base = base.clone();
    let pending_id = event_id.clone();
    let request_progress = Progress {
        tap: capture.clone(),
        stream: progress,
    };
    let pending_capture = request_progress.clone();
    let mut pending = crate::tap::CancellationGuard::new(move || {
        // Only a pending event needs closing; a cancelled request without one
        // (a websocket upgrade) is not an upstream failure.
        if pending_id.is_none() {
            return;
        }
        fail_pending(
            &pending_ctx,
            &pending_base,
            &pending_capture,
            pending_id,
            started,
            "Proxy request cancelled before response delivery",
        );
    });

    // A streamed body connects upstream only once it has something to
    // send: a body refused before that never reaches the upstream.
    if let Some(ready) = stream_ready {
        let refused = match ready.await {
            Ok(Ok(())) => None,
            Ok(Err(abort)) => Some(abort),
            Err(_) => Some(crate::request_stream::StreamAbort::new(
                500,
                "Request inspection ended unexpectedly",
            )),
        };
        if let Some(abort) = refused {
            pending.disarm();
            base.refused_locally = true;
            let status = StatusCode::from_u16(abort.status).unwrap_or(StatusCode::BAD_GATEWAY);
            tracing::warn!("proxy {}: {}", status.as_u16(), abort.message);
            // The monitor shows the refusal the client gets, as for a
            // request refused before it was read.
            if let Some(json) = &abort.json {
                base.response_body = Some(json.clone());
            }
            fail_pending_with(
                &ctx,
                &base,
                &request_progress,
                event_id,
                started,
                status.as_u16(),
                &abort.message,
            );
            return match abort.json {
                Some(json) => synthesized_response(status.as_u16(), "application/json", json),
                None => proxy_error(status, &abort.message),
            };
        }
    }

    // Establish a fresh upstream TLS connection for this request.
    let upstream = match connect_upstream(&ctx, &host, port).await {
        Ok(u) => u,
        Err(e) => {
            pending.disarm();
            fail_pending(
                &ctx,
                &base,
                &request_progress,
                event_id,
                started,
                &format!("upstream connect: {e}"),
            );
            return bad_gateway(&format!("upstream connect: {e}"));
        }
    };
    let (mut sender, conn) =
        match hyper::client::conn::http1::handshake(TokioIo::new(upstream)).await {
            Ok(pair) => pair,
            Err(e) => {
                pending.disarm();
                fail_pending(
                    &ctx,
                    &base,
                    &request_progress,
                    event_id,
                    started,
                    &format!("upstream handshake: {e}"),
                );
                return bad_gateway(&format!("upstream handshake: {e}"));
            }
        };
    tokio::spawn(async move {
        // `with_upgrades` lets the driver hand the connection over after a 101.
        let _ = conn.with_upgrades().await;
    });

    // A pass-through body is read from the client only as fast as the
    // upstream accepts it, and the response may arrive before it ends.
    let up_req = Request::from_parts(parts, forward_body);
    let mut resp = match sender.send_request(up_req).await {
        Ok(r) => r,
        Err(e) => {
            pending.disarm();
            // A pass-through upload that the client abandoned surfaces here
            // as a failed upstream request; name the real cause.
            let error = match capture.as_ref().and_then(RequestCaptureHandle::error) {
                Some(client) => format!("client request body: {client}"),
                None => format!("upstream request: {e}"),
            };
            // A streamed upload LocalRouter ended itself names its reason.
            if let Some(abort) = request_progress.stream.as_ref().and_then(|p| p.abort()) {
                let status = StatusCode::from_u16(abort.status).unwrap_or(StatusCode::BAD_GATEWAY);
                tracing::warn!("proxy {}: {}", status.as_u16(), abort.message);
                fail_pending_with(
                    &ctx,
                    &base,
                    &request_progress,
                    event_id,
                    started,
                    status.as_u16(),
                    &abort.message,
                );
                return match abort.json {
                    Some(json) => synthesized_response(status.as_u16(), "application/json", json),
                    None => proxy_error(status, &abort.message),
                };
            }
            fail_pending(&ctx, &base, &request_progress, event_id, started, &error);
            return bad_gateway(&error);
        }
    };

    // Completed websocket handshake: relay the upgraded byte streams (with
    // per-message firewall + capture on recognized LLM paths) and answer the
    // client with the upstream's 101 verbatim. A refused upgrade (non-101)
    // falls through to the normal response path below.
    if is_ws_upgrade && resp.status() == StatusCode::SWITCHING_PROTOCOLS {
        base.usage_account = observe_usage(resp.headers());
        let upstream_upgrade = hyper::upgrade::on(&mut resp);
        let client_upgrade = client_upgrade.expect("present for websocket upgrades");
        // A websocket on a path we don't recognize carries no LLM traffic: it
        // is relayed untouched and only reported as a passthrough.
        let passthrough = match wire::detect(&base.path) {
            Some(_) => None,
            None => Some(PassthroughExchange {
                client_id: base.client_id.clone(),
                mode: lr_monitor::PassthroughMode::Websocket,
                host: base.host.clone(),
                port,
                method: Some(base.method.clone()),
                path: Some(PassthroughExchange::strip_query(&base.path)),
                ..Default::default()
            }),
        };
        let session = wire::detect(&base.path).map(|format| {
            Arc::new(
                websocket::WsSession::new(ctx.interceptor.clone(), format, base)
                    .with_usage(ctx.usage.clone()),
            )
        });
        let passthrough = passthrough.map(|pt| (ctx.interceptor.begin_passthrough(&pt), pt));
        let ws_ctx = ctx.clone();
        tokio::spawn(async move {
            let (client_io, upstream_io) = match tokio::try_join!(client_upgrade, upstream_upgrade)
            {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::warn!("websocket upgrade failed: {e}");
                    if let Some(session) = &session {
                        session
                            .record_refused(
                                StatusCode::BAD_GATEWAY.as_u16(),
                                format!("websocket upgrade failed: {e}"),
                            )
                            .await;
                    }
                    if let Some((event_id, mut pt)) = passthrough {
                        pt.error = Some(format!("websocket upgrade failed: {e}"));
                        pt.latency_ms = Some(started.elapsed().as_millis() as u64);
                        ws_ctx.interceptor.end_passthrough(event_id, &pt);
                    }
                    return;
                }
            };
            websocket::relay(TokioIo::new(client_io), TokioIo::new(upstream_io), session).await;
            if let Some((event_id, mut pt)) = passthrough {
                pt.latency_ms = Some(started.elapsed().as_millis() as u64);
                ws_ctx.interceptor.end_passthrough(event_id, &pt);
            }
        });
        let (rparts, _rbody) = resp.into_parts();
        let empty = Full::new(Bytes::new())
            .map_err(|e: Infallible| match e {})
            .boxed_unsync();
        pending.disarm();
        return Response::from_parts(rparts, empty);
    }

    let (rparts, rbody) = resp.into_parts();
    let status = rparts.status.as_u16();
    base.usage_account = observe_usage(&rparts.headers);
    // A streamed body sent chunked that the upstream refused for lacking a
    // length: later ones to this host are held and sent with one.
    let chunked_stream = request_progress.stream.as_ref().is_some_and(|p| !p.sized());
    if chunked_stream && crate::framing::refuses_chunked(status, &[]) {
        ctx.length_required.learn(&host);
    }
    let learn_length =
        (chunked_stream && status == 400).then(|| (ctx.length_required.clone(), (*host).clone()));
    // Usage responses (Claude Code's `/usage`, Codex's `/status`) passing
    // through are read too, once complete.
    let usage_body = ctx
        .usage
        .clone()
        .filter(|_| (200..300).contains(&status))
        .zip(lr_usage::usage_endpoint(&host, &path));
    let content_type = rparts
        .headers
        .get(hyper::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_sse = content_type.contains("text/event-stream");
    let is_ndjson = content_type.contains("ndjson") || content_type.contains("jsonl");

    // On stream end, record the full exchange to the interceptor.
    let interceptor = ctx.interceptor.clone();
    let mut recorded = base;
    recorded.event_id = event_id;
    recorded.status = Some(status);
    recorded.response_is_sse = is_sse;
    recorded.response_is_ndjson = is_ndjson;
    let request_capture = request_progress;
    let on_end: Box<dyn FnOnce(crate::tap::Capture) + Send> = Box::new(move |mut capture| {
        let response_tail = capture.take_tail();
        let bytes = capture.bytes;
        if let Some((hosts, host)) = learn_length {
            if crate::framing::refuses_chunked(400, &bytes) {
                hosts.learn(&host);
            }
        }
        if let Some((tracker, endpoint)) = usage_body {
            tracker.observe_usage_body(
                endpoint,
                recorded.usage_account.as_ref(),
                &bytes,
                lr_usage::DataSource::ProxyUsageResponse,
            );
        }
        let mut ex = recorded;
        ex.response_tail = response_tail;
        ex.error = capture.error;
        ex.response_body = (!bytes.is_empty()).then_some(bytes);
        // What was forwarded by now; the upload may still be running when
        // the upstream answered early.
        request_capture.apply(&mut ex);
        ex.latency_ms = Some(started.elapsed().as_millis() as u64);
        tokio::spawn(async move {
            interceptor.on_response(&ex).await;
        });
    });

    let response = if let Some(vault) = secret_vault {
        match crate::protected_response::restore_recorded(
            rparts, rbody, vault, is_sse, is_ndjson, on_end,
        )
        .await
        {
            Ok(response) => response,
            Err(_) => bad_gateway("protected response restoration failed"),
        }
    } else {
        // Only an LLM response's end matters (its terminal event carries
        // the usage); other responses keep the head alone.
        let tapped = if wire::detect(&path).is_some() {
            crate::tap::TappedBody::with_tail(rbody, MAX_CAPTURE, on_end)
        } else {
            crate::tap::TappedBody::new(rbody, MAX_CAPTURE, on_end)
        };
        Response::from_parts(rparts, BodyExt::boxed_unsync(tapped))
    };
    pending.disarm();
    response
}

async fn connect_upstream(
    ctx: &ProxyContext,
    host: &str,
    port: u16,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, ProxyError> {
    let tcp = TcpStream::connect((host, port)).await?;
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|e| ProxyError::Tls(format!("invalid upstream host {host}: {e}")))?;
    TlsConnector::from(ctx.tls.upstream())
        .connect(server_name, tcp)
        .await
        .map_err(|e| ProxyError::Tls(format!("upstream TLS: {e}")))
}

/// Record an upstream failure before any response arrived: completes the
/// `begin()`ed pending event, or — for requests without one, such as websocket
/// upgrades — records the failure as its own event, so the cause the client
/// only sees as a 502 is visible in the monitor.
/// What the transport can report about a request body that was not
/// buffered: a pass-through tap, or a streamed upload.
#[derive(Clone, Default)]
struct Progress {
    tap: Option<RequestCaptureHandle>,
    stream: Option<crate::request_stream::StreamProgress>,
}

impl Progress {
    fn apply(&self, ex: &mut ObservedExchange) {
        if let Some(tap) = &self.tap {
            ex.request_capture = Some(tap.snapshot());
        }
        if let Some(stream) = &self.stream {
            ex.request_capture = Some(stream.snapshot());
            if let Some(summary) = stream.summary() {
                ex.request_stream = Some(summary);
            }
        }
    }
}

fn fail_pending(
    ctx: &Arc<ProxyContext>,
    base: &ObservedExchange,
    capture: &Progress,
    event_id: Option<String>,
    started: std::time::Instant,
    error: &str,
) {
    fail_pending_with(
        ctx,
        base,
        capture,
        event_id,
        started,
        StatusCode::BAD_GATEWAY.as_u16(),
        error,
    );
}

/// [`fail_pending`] with the status the client was answered with.
fn fail_pending_with(
    ctx: &Arc<ProxyContext>,
    base: &ObservedExchange,
    capture: &Progress,
    event_id: Option<String>,
    started: std::time::Instant,
    status: u16,
    error: &str,
) {
    let interceptor = ctx.interceptor.clone();
    let mut ex = base.clone();
    capture.apply(&mut ex);
    ex.event_id = event_id;
    ex.status = Some(status);
    ex.error = Some(error.to_owned());
    ex.latency_ms = Some(started.elapsed().as_millis() as u64);
    tokio::spawn(async move {
        interceptor.on_response(&ex).await;
    });
}

/// Answer a request whose body could not be read for buffering, and record
/// the failure.
fn body_read_failed(
    ctx: &Arc<ProxyContext>,
    base: ObservedExchange,
    started: std::time::Instant,
    too_large: bool,
    error: String,
) -> Response<BoxedBody> {
    let (status, error) = if too_large {
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "request body exceeds LocalRouter's {} MiB proxy limit",
                MAX_REQUEST_BODY / (1024 * 1024)
            ),
        )
    } else {
        (
            StatusCode::BAD_REQUEST,
            format!("failed reading request body: {error}"),
        )
    };
    tracing::warn!("proxy {}: {error}", status.as_u16());
    let failed = ObservedExchange {
        client_id: base.client_id,
        session_id: base.session_id,
        strategy_id: base.strategy_id,
        host: base.host,
        port: base.port,
        method: base.method,
        path: base.path,
        status: Some(status.as_u16()),
        error: Some(error.clone()),
        latency_ms: Some(started.elapsed().as_millis() as u64),
        admission: base.admission,
        refused_locally: true,
        ..Default::default()
    };
    let interceptor = ctx.interceptor.clone();
    tokio::spawn(async move {
        interceptor.on_response(&failed).await;
    });
    proxy_error(status, &error)
}

/// The outcome of reading a [`BodyPolicy::Stream`] body up to the point
/// where it either ended or outgrew its lag.
enum Prepared {
    /// The whole body, for the buffered path.
    Buffered(Bytes),
    Failed {
        too_large: bool,
        error: String,
    },
    Rejected {
        action: RequestAction,
        capture: crate::interceptor::BoundedCapture,
    },
    Stream(Box<crate::request_stream::StreamStart>),
}

impl From<crate::stream_driver::IntakeError> for Prepared {
    fn from(error: crate::stream_driver::IntakeError) -> Self {
        use crate::stream_driver::IntakeError;
        match error {
            IntakeError::TooLarge => Prepared::Failed {
                too_large: true,
                error: String::new(),
            },
            IntakeError::Failed(error) => Prepared::Failed {
                too_large: false,
                error,
            },
        }
    }
}

/// Push a batch into the intake off the executor (it may decode, lex and
/// spill).
async fn intake_step(
    intake: &mut Option<crate::stream_driver::Intake>,
    data: Vec<u8>,
) -> Result<(), crate::stream_driver::IntakeError> {
    let mut taken = intake.take().expect("intake between steps");
    let (returned, result) = tokio::task::spawn_blocking(move || {
        let result = taken.push(&data);
        (taken, result)
    })
    .await
    .map_err(|_| crate::stream_driver::IntakeError::Failed("request intake failed".into()))?;
    *intake = Some(returned);
    result
}

/// Read a streamable body. Until it outgrows the lag (or when it cannot be
/// streamed) it is held whole, raw; nothing is forwarded before the
/// interceptor opens the stream. While the interceptor decides (which may
/// wait for an approval popup) the client keeps uploading and its bytes are
/// held, spilling to disk beyond RAM.
async fn prepare_stream(
    body: Incoming,
    headers: &hyper::HeaderMap,
    policy: crate::interceptor::StreamPolicy,
    ctx: &Arc<ProxyContext>,
    base: &ObservedExchange,
    capture_cap: usize,
    client_length: Option<u64>,
) -> Prepared {
    use crate::interceptor::StreamOpen;
    use crate::stream_driver::{ClientReader, Intake, Output, PREVIEW_LIMIT};
    let format = wire::detect(&base.path);
    // Undecodable encodings and unknown formats are judged on the buffered
    // path, exactly as before.
    let streamable = format.is_some() && crate::request_body::StreamDecoder::new(headers).is_ok();
    let mut intake = Some(Intake::new(
        headers,
        policy.lag,
        ctx.spill.clone(),
        MAX_REQUEST_BODY,
        streamable,
    ));
    let mut reader = ClientReader::new(body);
    loop {
        let data = match reader.batch(crate::request_stream::STEP).await {
            Ok(data) => data,
            Err(error) => {
                return Prepared::Failed {
                    too_large: false,
                    error,
                }
            }
        };
        if let Err(error) = intake_step(&mut intake, data).await {
            return error.into();
        }
        let current = intake.as_ref().expect("intake between steps");
        if reader.eof() || current.ready_to_open() {
            break;
        }
    }
    let mut intake = intake.expect("intake after reading");
    if !intake.ready_to_open() {
        return intake
            .into_buffered()
            .map_or_else(Prepared::from, Prepared::Buffered);
    }
    let metadata = intake.metadata().cloned().unwrap_or_default();
    let mut offer = base.clone();
    if let Some((head, received)) = intake.preview(PREVIEW_LIMIT) {
        offer.request_capture = Some(crate::interceptor::BoundedCapture {
            truncated: received > head.len() as u64,
            head,
            total_len: received,
            complete: false,
            encoded: false,
        });
    }
    // Keep reading (and holding) while the interceptor decides.
    let decision = {
        let decide = ctx.interceptor.open_stream(&offer, &metadata, &policy);
        tokio::pin!(decide);
        loop {
            if reader.eof() {
                break (&mut decide).await;
            }
            tokio::select! {
                decision = &mut decide => break decision,
                frame = reader.frame() => match frame {
                    Ok(Some(data)) => {
                        if let Err(error) = intake.push(&data) {
                            return error.into();
                        }
                    }
                    Ok(None) => {}
                    Err(error) => return Prepared::Failed { too_large: false, error },
                },
            }
        }
    };
    drop(offer);
    // Bytes that arrived meanwhile may have shown the body cannot be
    // streamed after all (a repeated key, invalid JSON): nothing was sent,
    // so it takes the buffered path.
    let decision = match decision {
        StreamOpen::Stream { .. } if !intake.streamable() => StreamOpen::Buffer,
        decision => decision,
    };
    match decision {
        StreamOpen::Buffer => {
            if let Err(error) = intake.give_up() {
                return error.into();
            }
            let mut intake = Some(intake);
            while !reader.eof() {
                let data = match reader.batch(crate::request_stream::STEP).await {
                    Ok(data) => data,
                    Err(error) => {
                        return Prepared::Failed {
                            too_large: false,
                            error,
                        }
                    }
                };
                if let Err(error) = intake_step(&mut intake, data).await {
                    return error.into();
                }
            }
            intake
                .expect("intake after reading")
                .into_buffered()
                .map_or_else(Prepared::from, Prepared::Buffered)
        }
        StreamOpen::Reject(action) => {
            // Read and discard the rest, so the client can read the refusal.
            let head = intake.raw_head(capture_cap);
            let received = intake.received();
            drop(intake);
            let (rest, complete) = reader
                .drain(
                    (MAX_REQUEST_BODY as u64).saturating_sub(received),
                    crate::stream_driver::DRAIN_TIMEOUT,
                )
                .await;
            let total_len = received + rest;
            let capture = crate::interceptor::BoundedCapture {
                truncated: total_len > head.len() as u64,
                head,
                total_len,
                complete,
                encoded: crate::tap::content_encoded(headers),
            };
            Prepared::Rejected { action, capture }
        }
        StreamOpen::Stream {
            session,
            vault,
            tee,
        } => {
            let Some((decoder, pipeline, raw)) = intake.into_stream() else {
                return Prepared::Failed {
                    too_large: false,
                    error: "request stream state lost".into(),
                };
            };
            let length_required = ctx.length_required.requires(&base.host);
            let output = if tee {
                Output::Tee {
                    length: client_length,
                    hold: client_length.is_none() && length_required,
                }
            } else if length_required {
                Output::Sized
            } else {
                Output::Chunked
            };
            Prepared::Stream(Box::new(crate::request_stream::StreamStart {
                reader,
                decoder,
                pipeline,
                raw,
                session,
                format: format.expect("a streamable body has a format"),
                capture_cap,
                vault,
                output,
                strict: policy.strict,
                encoded: tee && crate::tap::content_encoded(headers),
                spill: ctx.spill.clone(),
            }))
        }
    }
}

/// Whether the client holds its body back until it is told to continue.
fn expects_continue(headers: &hyper::HeaderMap) -> bool {
    headers
        .get(hyper::header::EXPECT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("100-continue"))
}

/// A fully buffered request body for the upstream.
fn buffered_body(bytes: Bytes) -> UpstreamBody {
    Full::new(bytes)
        .map_err(|e: Infallible| match e {})
        .boxed_unsync()
}

/// Answer the client with a firewall refusal without contacting the
/// upstream, and record the blocked call so it shows in the monitor.
fn reject(
    ctx: &Arc<ProxyContext>,
    mut blocked: ObservedExchange,
    started: std::time::Instant,
    action: RequestAction,
) -> Response<BoxedBody> {
    let RequestAction::Reject {
        status,
        content_type,
        body,
    } = action
    else {
        unreachable!("reject is only called with a Reject action");
    };
    let interceptor = ctx.interceptor.clone();
    blocked.status = Some(status);
    blocked.response_body = Some(body.clone());
    blocked.latency_ms = Some(started.elapsed().as_millis() as u64);
    blocked.refused_locally = true;
    tokio::spawn(async move {
        interceptor.on_response(&blocked).await;
    });
    synthesized_response(status, &content_type, body)
}

fn bad_gateway(msg: &str) -> Response<BoxedBody> {
    tracing::warn!("proxy 502: {msg}");
    proxy_error(StatusCode::BAD_GATEWAY, msg)
}

/// A proxy-generated error response. The body names the cause, because
/// clients such as Codex show it to the user verbatim.
fn proxy_error(status: StatusCode, msg: &str) -> Response<BoxedBody> {
    let body = Full::new(Bytes::from(format!("LocalRouter proxy error: {msg}")))
        .map_err(|e: Infallible| match e {})
        .boxed_unsync();
    Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(body)
        .expect("static proxy error response")
}

/// A locally-synthesized response returned to the client (firewall deny), never
/// contacting the upstream.
fn synthesized_response(status: u16, content_type: &str, body: Vec<u8>) -> Response<BoxedBody> {
    let boxed = Full::new(Bytes::from(body))
        .map_err(|e: Infallible| match e {})
        .boxed_unsync();
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::FORBIDDEN))
        .header(hyper::header::CONTENT_TYPE, content_type)
        .body(boxed)
        .expect("synthesized response")
}

/// Read the proxy request head byte-by-byte up to the header terminator, so a
/// plain-HTTP request body (which follows the head) is never over-read. A
/// `CONNECT` client waits for our `200` before sending TLS.
async fn read_request_head(client: &mut TcpStream) -> Result<ProxyRequest, ProxyError> {
    let mut buf = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    loop {
        let n = client.read(&mut byte).await?;
        if n == 0 {
            return Err(ProxyError::Protocol("client closed before CONNECT".into()));
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if buf.len() > MAX_CONNECT_HEADER {
            return Err(ProxyError::Protocol("request head too large".into()));
        }
    }
    parse_head(&buf)
}

/// Headers that are between the client and *this* proxy, and must not be
/// relayed to the origin server.
const HOP_BY_HOP: &[&str] = &["proxy-authorization", "proxy-connection"];

fn parse_head(raw: &[u8]) -> Result<ProxyRequest, ProxyError> {
    let text = std::str::from_utf8(raw)
        .map_err(|_| ProxyError::Protocol("non-UTF8 request head".into()))?;
    let mut lines = text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| ProxyError::Protocol("empty proxy request".into()))?;

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts
        .next()
        .ok_or_else(|| ProxyError::Protocol("proxy request missing target".into()))?;
    let version = parts.next().unwrap_or("HTTP/1.1");

    // Headers are parsed once, for both forms.
    let mut proxy_auth = None;
    let mut kept_headers = String::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name_trimmed = name.trim();
        if name_trimmed.eq_ignore_ascii_case("proxy-authorization") {
            proxy_auth = Some(value.trim().to_string());
        }
        if !HOP_BY_HOP
            .iter()
            .any(|h| name_trimmed.eq_ignore_ascii_case(h))
        {
            kept_headers.push_str(line);
            kept_headers.push_str("\r\n");
        }
    }

    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = split_host_port(target)?;
        return Ok(ProxyRequest::Connect(ConnectReq {
            host,
            port,
            proxy_auth,
        }));
    }

    // Absolute-form plain HTTP (`GET http://host/path HTTP/1.1`) — the only
    // other thing a client may legitimately send to a forward proxy.
    let (host, port, origin_target) = split_absolute_uri(target)?;
    let head = format!("{method} {origin_target} {version}\r\n{kept_headers}\r\n").into_bytes();
    Ok(ProxyRequest::Plain(PlainReq {
        method,
        host,
        port,
        target: origin_target,
        proxy_auth,
        head,
    }))
}

fn split_host_port(authority: &str) -> Result<(String, u16), ProxyError> {
    let (host, port) = authority.rsplit_once(':').ok_or_else(|| {
        ProxyError::Protocol(format!("CONNECT authority missing port: {authority}"))
    })?;
    let port: u16 = port
        .parse()
        .map_err(|_| ProxyError::Protocol(format!("invalid CONNECT port: {port}")))?;
    Ok((host.to_string(), port))
}

/// Split an absolute-form request target into (host, port, origin-form target).
fn split_absolute_uri(target: &str) -> Result<(String, u16, String), ProxyError> {
    let (scheme, rest) = target
        .split_once("://")
        .ok_or_else(|| ProxyError::Protocol(format!("not an absolute-form target: {target}")))?;
    let default_port = match scheme.to_ascii_lowercase().as_str() {
        "http" => 80,
        "https" => 443,
        other => {
            return Err(ProxyError::Protocol(format!(
                "unsupported proxy scheme: {other}"
            )))
        }
    };
    let (authority, path) = match rest.find(['/', '?', '#']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    // Discard any userinfo — it belongs to the origin request line, not to us.
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    let (host, port) = match authority.rsplit_once(':') {
        // An IPv6 literal's colons live inside brackets; a trailing `:port`
        // never does.
        Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => (
            h,
            p.parse::<u16>()
                .map_err(|_| ProxyError::Protocol(format!("invalid proxy port: {p}")))?,
        ),
        _ => (authority, default_port),
    };
    if host.is_empty() {
        return Err(ProxyError::Protocol(format!(
            "absolute-form target has no host: {target}"
        )));
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    Ok((host.to_string(), port, path.to_string()))
}

/// Parse a `Basic base64(user:pass)` header value into (user, pass).
fn parse_basic_auth(header: &str) -> Option<(String, String)> {
    let b64 = header
        .strip_prefix("Basic ")
        .or_else(|| header.strip_prefix("basic "))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (user, pass) = decoded.split_once(':')?;
    Some((user.to_string(), pass.to_string()))
}

async fn write_status(
    client: &mut TcpStream,
    status: &str,
    extra_headers: &str,
) -> Result<(), ProxyError> {
    let response = format!("HTTP/1.1 {status}\r\n{extra_headers}Content-Length: 0\r\n\r\n");
    client.write_all(response.as_bytes()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connect(raw: &[u8]) -> ConnectReq {
        match parse_head(raw).unwrap() {
            ProxyRequest::Connect(c) => c,
            ProxyRequest::Plain(_) => panic!("expected CONNECT"),
        }
    }

    fn plain(raw: &[u8]) -> PlainReq {
        match parse_head(raw).unwrap() {
            ProxyRequest::Plain(p) => p,
            ProxyRequest::Connect(_) => panic!("expected plain HTTP"),
        }
    }

    #[test]
    fn parses_connect_with_auth() {
        let raw = b"CONNECT api.anthropic.com:443 HTTP/1.1\r\nHost: api.anthropic.com:443\r\nProxy-Authorization: Basic Y2lkOnNlY3JldA==\r\n\r\n";
        let c = connect(raw);
        assert_eq!(c.host, "api.anthropic.com");
        assert_eq!(c.port, 443);
        let (u, p) = parse_basic_auth(c.proxy_auth.as_deref().unwrap()).unwrap();
        assert_eq!(u, "cid");
        assert_eq!(p, "secret");
    }

    #[test]
    fn parses_absolute_form_plain_http() {
        // What a client sends for an `http://` URL when it resolves a proxy —
        // previously a hard error that dropped the connection.
        let raw = b"GET http://github.com/o/r/info/refs?service=git-upload-pack HTTP/1.1\r\n\
                    Host: github.com\r\n\
                    Proxy-Authorization: Basic Y2lkOnNlY3JldA==\r\n\
                    Proxy-Connection: Keep-Alive\r\n\
                    User-Agent: git/2.44\r\n\r\n";
        let p = plain(raw);
        assert_eq!(p.method, "GET");
        assert_eq!(p.host, "github.com");
        assert_eq!(p.port, 80);
        assert_eq!(p.target, "/o/r/info/refs?service=git-upload-pack");
        assert_eq!(
            parse_basic_auth(p.proxy_auth.as_deref().unwrap()).unwrap(),
            ("cid".to_string(), "secret".to_string())
        );

        // The head sent upstream is origin-form, with the proxy-only headers
        // dropped and everything else byte-for-byte intact.
        let head = String::from_utf8(p.head).unwrap();
        assert_eq!(
            head,
            "GET /o/r/info/refs?service=git-upload-pack HTTP/1.1\r\n\
             Host: github.com\r\n\
             User-Agent: git/2.44\r\n\r\n"
        );
    }

    #[test]
    fn absolute_uri_variants() {
        assert_eq!(
            split_absolute_uri("http://example.com").unwrap(),
            ("example.com".to_string(), 80, "/".to_string())
        );
        assert_eq!(
            split_absolute_uri("http://example.com:8080/p").unwrap(),
            ("example.com".to_string(), 8080, "/p".to_string())
        );
        assert_eq!(
            split_absolute_uri("https://example.com/p").unwrap(),
            ("example.com".to_string(), 443, "/p".to_string())
        );
        // Userinfo belongs to the proxy hop, not the origin request line.
        assert_eq!(
            split_absolute_uri("http://user:pw@example.com/p").unwrap(),
            ("example.com".to_string(), 80, "/p".to_string())
        );
        // Query-only and IPv6 targets.
        assert_eq!(
            split_absolute_uri("http://example.com?a=1").unwrap(),
            ("example.com".to_string(), 80, "?a=1".to_string())
        );
        assert_eq!(
            split_absolute_uri("http://[::1]:9000/p").unwrap(),
            ("::1".to_string(), 9000, "/p".to_string())
        );
        // Not absolute-form: someone talking to the proxy port directly.
        assert!(split_absolute_uri("/plain/path").is_err());
        assert!(split_absolute_uri("ftp://example.com/f").is_err());
    }

    #[test]
    fn origin_form_request_is_not_proxyable() {
        // A direct request to the proxy port has no destination to forward to.
        assert!(parse_head(b"GET / HTTP/1.1\r\n\r\n").is_err());
    }

    #[test]
    fn split_host_port_parses() {
        assert_eq!(
            split_host_port("example.com:8443").unwrap(),
            ("example.com".to_string(), 8443)
        );
        assert!(split_host_port("noport").is_err());
    }

    #[test]
    fn basic_auth_roundtrip() {
        let enc = base64::engine::general_purpose::STANDARD.encode("alice:pw:with:colons");
        let (u, p) = parse_basic_auth(&format!("Basic {enc}")).unwrap();
        assert_eq!(u, "alice");
        // Only the first colon splits user/pass; the rest stays in the password.
        assert_eq!(p, "pw:with:colons");
    }
}
