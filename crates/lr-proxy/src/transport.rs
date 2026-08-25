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
    ClientCtx, ConnectDecision, ObservedExchange, PassthroughExchange, ProxyInterceptor,
    RequestAction,
};
use crate::resolver::ClientResolver;
use crate::tls::TlsFactory;
use crate::{websocket, wire};

/// Cap on the bytes captured per request/response for monitoring (1 MiB).
const MAX_CAPTURE: usize = 1024 * 1024;
/// Cap on the CONNECT header block, to bound a misbehaving client.
const MAX_CONNECT_HEADER: usize = 16 * 1024;

/// Shared collaborators for handling proxied connections.
pub struct ProxyContext {
    pub interceptor: Arc<dyn ProxyInterceptor>,
    pub resolver: Arc<dyn ClientResolver>,
    pub tls: Arc<TlsFactory>,
    /// Whether to stamp `X-LocalRouter-Trace` on forwarded requests and pass
    /// already-traced (duplicate-hop) requests through uncounted. Shared with
    /// the app so the setting can change without restarting the listener.
    pub dedupe_enabled: Arc<std::sync::atomic::AtomicBool>,
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

    // Buffer the (small) request body so we can both forward and inspect it.
    let (mut parts, body) = req.into_parts();
    let req_bytes = match body.collect().await {
        Ok(c) => c.to_bytes(),
        Err(_) => return bad_gateway("failed reading request body"),
    };

    // Ask the upstream for an uncompressed response so we can read it. Without
    // this, providers honor the client's `Accept-Encoding: gzip, br` and we'd
    // capture compressed bytes we can't parse. The client still receives a
    // valid (now uncompressed) response — semantics are unchanged.
    parts.headers.remove(hyper::header::ACCEPT_ENCODING);
    // hyper sets Content-Length from the forwarded body; drop the client's so a
    // rewritten (or re-framed) request never carries a stale length.
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

    // Base exchange (request half); response fields filled at stream end.
    let base = ObservedExchange {
        client_id: (*client_id).clone(),
        strategy_id: (*strategy_id).clone(),
        host: (*host).clone(),
        port,
        method,
        path,
        request_body: (!req_bytes.is_empty()).then(|| req_bytes.to_vec()),
        trace,
        ..Default::default()
    };

    // Firewall: forward, rewrite, or reject. (Passive returns Forward.)
    // A websocket upgrade carries no LLM request — the firewall evaluates each
    // websocket message inside the connection instead (see websocket::WsSession),
    // so don't ask it to judge the empty upgrade body (Ask-mode would pop a
    // model-less approval dialog).
    let action = if is_ws_upgrade {
        RequestAction::Forward
    } else {
        ctx.interceptor.on_request(&base).await
    };
    let forward_bytes: Bytes = match action {
        RequestAction::Forward => req_bytes,
        RequestAction::Replace(new_body) => Bytes::from(new_body),
        RequestAction::Reject {
            status,
            content_type,
            body,
        } => {
            // Record the blocked call so it shows in the monitor, then answer
            // the client directly without ever contacting the upstream.
            let interceptor = ctx.interceptor.clone();
            let mut blocked = base;
            blocked.status = Some(status);
            blocked.response_body = Some(body.clone());
            blocked.latency_ms = Some(started.elapsed().as_millis() as u64);
            tokio::spawn(async move {
                interceptor.on_response(&blocked).await;
            });
            return synthesized_response(status, &content_type, body);
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

    // Establish a fresh upstream TLS connection for this request.
    let upstream = match connect_upstream(&ctx, &host, port).await {
        Ok(u) => u,
        Err(e) => {
            fail_pending(&ctx, &base, event_id, started);
            return bad_gateway(&format!("upstream connect: {e}"));
        }
    };
    let (mut sender, conn) =
        match hyper::client::conn::http1::handshake(TokioIo::new(upstream)).await {
            Ok(pair) => pair,
            Err(e) => {
                fail_pending(&ctx, &base, event_id, started);
                return bad_gateway(&format!("upstream handshake: {e}"));
            }
        };
    tokio::spawn(async move {
        // `with_upgrades` lets the driver hand the connection over after a 101.
        let _ = conn.with_upgrades().await;
    });

    let up_req = Request::from_parts(parts, Full::new(forward_bytes));
    let mut resp = match sender.send_request(up_req).await {
        Ok(r) => r,
        Err(e) => {
            fail_pending(&ctx, &base, event_id, started);
            return bad_gateway(&format!("upstream request: {e}"));
        }
    };

    // Completed websocket handshake: relay the upgraded byte streams (with
    // per-message firewall + capture on recognized LLM paths) and answer the
    // client with the upstream's 101 verbatim. A refused upgrade (non-101)
    // falls through to the normal response path below.
    if is_ws_upgrade && resp.status() == StatusCode::SWITCHING_PROTOCOLS {
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
            Arc::new(websocket::WsSession::new(
                ctx.interceptor.clone(),
                format,
                base,
            ))
        });
        let passthrough = passthrough.map(|pt| (ctx.interceptor.begin_passthrough(&pt), pt));
        let ws_ctx = ctx.clone();
        tokio::spawn(async move {
            let (client_io, upstream_io) = match tokio::try_join!(client_upgrade, upstream_upgrade)
            {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::warn!("websocket upgrade failed: {e}");
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
        return Response::from_parts(rparts, empty);
    }

    let (rparts, rbody) = resp.into_parts();
    let status = rparts.status.as_u16();
    let is_sse = rparts
        .headers
        .get(hyper::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));

    // On stream end, record the full exchange to the interceptor.
    let interceptor = ctx.interceptor.clone();
    let mut recorded = base;
    recorded.event_id = event_id;
    recorded.status = Some(status);
    recorded.response_is_sse = is_sse;
    let on_end: Box<dyn FnOnce(Vec<u8>) + Send> = Box::new(move |bytes| {
        let mut ex = recorded;
        ex.response_body = (!bytes.is_empty()).then_some(bytes);
        ex.latency_ms = Some(started.elapsed().as_millis() as u64);
        tokio::spawn(async move {
            interceptor.on_response(&ex).await;
        });
    });

    let tapped = crate::tap::TappedBody::new(rbody, MAX_CAPTURE, on_end);
    Response::from_parts(rparts, BodyExt::boxed_unsync(tapped))
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

/// Close out a pending monitor event as an upstream failure, so a `begin()`ed
/// event never stays stuck in Pending when the upstream call errors before any
/// response arrives. No-op when there was no pending event.
fn fail_pending(
    ctx: &Arc<ProxyContext>,
    base: &ObservedExchange,
    event_id: Option<String>,
    started: std::time::Instant,
) {
    let Some(id) = event_id else { return };
    let interceptor = ctx.interceptor.clone();
    let mut ex = base.clone();
    ex.event_id = Some(id);
    ex.status = Some(StatusCode::BAD_GATEWAY.as_u16());
    ex.latency_ms = Some(started.elapsed().as_millis() as u64);
    tokio::spawn(async move {
        interceptor.on_response(&ex).await;
    });
}

fn bad_gateway(msg: &str) -> Response<BoxedBody> {
    tracing::warn!("proxy 502: {msg}");
    let body = Full::new(Bytes::from("upstream error"))
        .map_err(|e: Infallible| match e {})
        .boxed_unsync();
    Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .body(body)
        .expect("static 502 response")
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
