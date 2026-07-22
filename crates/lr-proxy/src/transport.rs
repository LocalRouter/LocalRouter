//! The MITM data-path: parse `CONNECT`, authenticate, then either blind-tunnel
//! or terminate TLS and ferry HTTP/1.1 while teeing traffic to the interceptor.

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
    ClientCtx, ConnectDecision, ObservedExchange, ProxyInterceptor, RequestAction,
};
use crate::resolver::ClientResolver;
use crate::tls::TlsFactory;

/// Cap on the bytes captured per request/response for monitoring (1 MiB).
const MAX_CAPTURE: usize = 1024 * 1024;
/// Cap on the CONNECT header block, to bound a misbehaving client.
const MAX_CONNECT_HEADER: usize = 16 * 1024;

/// Shared collaborators for handling proxied connections.
pub struct ProxyContext {
    pub interceptor: Arc<dyn ProxyInterceptor>,
    pub resolver: Arc<dyn ClientResolver>,
    pub tls: Arc<TlsFactory>,
}

/// A parsed `CONNECT` request line + relevant headers.
struct ConnectReq {
    host: String,
    port: u16,
    proxy_auth: Option<String>,
}

/// Entry point: drive one accepted TCP connection to completion.
pub async fn handle_connection(client: TcpStream, ctx: Arc<ProxyContext>) {
    if let Err(e) = handle_inner(client, ctx).await {
        tracing::debug!("proxy connection ended: {e}");
    }
}

async fn handle_inner(mut client: TcpStream, ctx: Arc<ProxyContext>) -> Result<(), ProxyError> {
    let connect = read_connect(&mut client).await?;

    // Authenticate via Proxy-Authorization (Basic client_id:secret).
    let client_ctx = match connect
        .proxy_auth
        .as_deref()
        .and_then(parse_basic_auth)
        .and_then(|(id, secret)| ctx.resolver.resolve(&id, &secret))
    {
        Some(c) => c,
        None => {
            write_status(
                &mut client,
                "407 Proxy Authentication Required",
                "Proxy-Authenticate: Basic realm=\"LocalRouter\"\r\n",
            )
            .await?;
            return Ok(());
        }
    };

    match ctx.interceptor.on_connect(&connect.host, &client_ctx) {
        ConnectDecision::Reject(reason) => {
            tracing::info!("proxy rejected CONNECT {}: {}", connect.host, reason);
            write_status(&mut client, "403 Forbidden", "").await?;
            Ok(())
        }
        ConnectDecision::Tunnel => tunnel(client, &connect).await,
        ConnectDecision::Mitm => mitm(client, connect, client_ctx, ctx).await,
    }
}

/// Blind byte tunnel — no decryption.
async fn tunnel(mut client: TcpStream, connect: &ConnectReq) -> Result<(), ProxyError> {
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    let mut upstream = TcpStream::connect((connect.host.as_str(), connect.port)).await?;
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
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
        .await
        .map_err(|e| ProxyError::Tls(format!("client HTTP/1.1 serve: {e}")))?;
    Ok(())
}

type BoxedBody = http_body_util::combinators::UnsyncBoxBody<Bytes, std::io::Error>;

/// Handle one decrypted request: forward it upstream verbatim, tee the response.
async fn proxy_request(
    req: Request<Incoming>,
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

    // Base exchange (request half); response fields filled at stream end.
    let base = ObservedExchange {
        client_id: (*client_id).clone(),
        strategy_id: (*strategy_id).clone(),
        host: (*host).clone(),
        method,
        path,
        request_body: (!req_bytes.is_empty()).then(|| req_bytes.to_vec()),
        ..Default::default()
    };

    // Firewall: forward, rewrite, or reject. (Passive returns Forward.)
    let forward_bytes: Bytes = match ctx.interceptor.on_request(&base).await {
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

    // Establish a fresh upstream TLS connection for this request.
    let upstream = match connect_upstream(&ctx, &host, port).await {
        Ok(u) => u,
        Err(e) => return bad_gateway(&format!("upstream connect: {e}")),
    };
    let (mut sender, conn) =
        match hyper::client::conn::http1::handshake(TokioIo::new(upstream)).await {
            Ok(pair) => pair,
            Err(e) => return bad_gateway(&format!("upstream handshake: {e}")),
        };
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let up_req = Request::from_parts(parts, Full::new(forward_bytes));
    let resp = match sender.send_request(up_req).await {
        Ok(r) => r,
        Err(e) => return bad_gateway(&format!("upstream request: {e}")),
    };

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

/// Read the `CONNECT` request byte-by-byte up to the header terminator. The
/// client waits for our `200` before sending TLS, so we never over-read.
async fn read_connect(client: &mut TcpStream) -> Result<ConnectReq, ProxyError> {
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
            return Err(ProxyError::Protocol("CONNECT header too large".into()));
        }
    }
    parse_connect(&buf)
}

fn parse_connect(raw: &[u8]) -> Result<ConnectReq, ProxyError> {
    let text = std::str::from_utf8(raw)
        .map_err(|_| ProxyError::Protocol("non-UTF8 CONNECT header".into()))?;
    let mut lines = text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| ProxyError::Protocol("empty CONNECT request".into()))?;

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    if !method.eq_ignore_ascii_case("CONNECT") {
        return Err(ProxyError::Protocol(format!(
            "expected CONNECT, got {method}"
        )));
    }
    let authority = parts
        .next()
        .ok_or_else(|| ProxyError::Protocol("CONNECT missing authority".into()))?;
    let (host, port) = split_host_port(authority)?;

    let mut proxy_auth = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("proxy-authorization") {
                proxy_auth = Some(value.trim().to_string());
            }
        }
    }

    Ok(ConnectReq {
        host,
        port,
        proxy_auth,
    })
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

    #[test]
    fn parses_connect_with_auth() {
        let raw = b"CONNECT api.anthropic.com:443 HTTP/1.1\r\nHost: api.anthropic.com:443\r\nProxy-Authorization: Basic Y2lkOnNlY3JldA==\r\n\r\n";
        let c = parse_connect(raw).unwrap();
        assert_eq!(c.host, "api.anthropic.com");
        assert_eq!(c.port, 443);
        let (u, p) = parse_basic_auth(c.proxy_auth.as_deref().unwrap()).unwrap();
        assert_eq!(u, "cid");
        assert_eq!(p, "secret");
    }

    #[test]
    fn rejects_non_connect() {
        let raw = b"GET / HTTP/1.1\r\n\r\n";
        assert!(parse_connect(raw).is_err());
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
