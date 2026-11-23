//! Websocket messages through the relay with the real firewall: the Secret
//! Scan session (scanner + vault) streams them as it streams HTTP bodies, so
//! the upstream gets the same masked request as the buffered path, never a
//! detected original, and Ask questions are answered while nothing leaves.
//! Mirrors the HTTP parity tests in the parent module.

use super::*;
use lr_proxy::active::{ActiveInterceptor, FirewallRequest};
use lr_proxy::interceptor::{BodyPolicy, ObservedExchange, RequestHead};
use lr_proxy::websocket::{encode_frame, relay, FrameDecoder, WsMessage, WsSession};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// The app firewall with a smaller stream lag, so test messages stream.
struct Lagged {
    inner: Arc<AppFirewall>,
    lag: usize,
}

#[async_trait::async_trait]
impl Firewall for Lagged {
    fn body_policy(&self, head: &RequestHead<'_>) -> BodyPolicy {
        match self.inner.body_policy(head) {
            BodyPolicy::Stream(mut policy) => {
                policy.lag = self.lag;
                BodyPolicy::Stream(policy)
            }
            other => other,
        }
    }
    async fn evaluate(&self, req: &FirewallRequest) -> RequestAction {
        self.inner.evaluate(req).await
    }
    async fn open_stream(&self, req: &StreamRequest, policy: &StreamPolicy) -> StreamOpen {
        self.inner.open_stream(req, policy).await
    }
    fn protects_secrets(&self, client_id: &str) -> bool {
        self.inner.protects_secrets(client_id)
    }
}

/// `payload` as one text message in masked frames of `size` bytes.
fn frames(payload: &[u8], size: usize) -> Vec<u8> {
    let pieces: Vec<&[u8]> = payload.chunks(size.max(1)).collect();
    let mut wire = Vec::new();
    for (i, piece) in pieces.iter().enumerate() {
        let fin = if i + 1 == pieces.len() { 0x80 } else { 0 };
        let opcode = if i == 0 { 0x1 } else { 0x0 };
        wire.push(fin | opcode);
        match piece.len() {
            n if n < 126 => wire.push(0x80 | n as u8),
            n if n <= 0xFFFF => {
                wire.push(0x80 | 126);
                wire.extend_from_slice(&(n as u16).to_be_bytes());
            }
            n => {
                wire.push(0x80 | 127);
                wire.extend_from_slice(&(n as u64).to_be_bytes());
            }
        }
        // A zero masking key leaves the payload as it is.
        wire.extend_from_slice(&[0, 0, 0, 0]);
        wire.extend_from_slice(piece);
    }
    wire
}

/// A relay for one websocket connection of the fixture's client.
struct Connection {
    client: tokio::io::DuplexStream,
    upstream: tokio::io::DuplexStream,
    relay: tokio::task::JoinHandle<()>,
    from_upstream: FrameDecoder,
    from_client: FrameDecoder,
    /// Client messages decoded but not yet looked at.
    inbox: std::collections::VecDeque<WsMessage>,
}

fn connect(firewall: Arc<AppFirewall>, base: &FirewallRequest, lag: usize) -> Connection {
    let store = Arc::new(lr_monitor::MonitorEventStore::new(32));
    let interceptor = Arc::new(ActiveInterceptor::new(
        PassiveInterceptor::new(store),
        Arc::new(Lagged {
            inner: firewall,
            lag,
        }),
    ));
    let exchange = ObservedExchange {
        client_id: base.client_id.clone(),
        secret_session: base.secret_session.clone(),
        host: base.host.clone(),
        method: "GET".into(),
        path: base.path.clone(),
        ..Default::default()
    };
    let session = WsSession::new(
        interceptor,
        lr_proxy::wire::WireFormat::OpenAiResponses,
        exchange,
    );
    let (client, proxy_client) = tokio::io::duplex(1024 * 1024);
    let (upstream, proxy_upstream) = tokio::io::duplex(1024 * 1024);
    let relay = tokio::spawn(relay(proxy_client, proxy_upstream, Some(Arc::new(session))));
    Connection {
        client,
        upstream,
        relay,
        from_upstream: FrameDecoder::with_limit(256 * 1024 * 1024),
        from_client: FrameDecoder::new(),
        inbox: Default::default(),
    }
}

impl Connection {
    /// The next message (data or close) the upstream receives.
    async fn upstream_message(&mut self) -> WsMessage {
        let mut buf = vec![0u8; 64 * 1024];
        tokio::time::timeout(WAIT, async {
            loop {
                let n = self.upstream.read(&mut buf).await.unwrap();
                assert!(n > 0, "the upstream connection ended");
                if let Some(message) = self
                    .from_upstream
                    .push(&buf[..n])
                    .into_iter()
                    .find(|m| !matches!(m, WsMessage::Ping(_) | WsMessage::Pong(_)))
                {
                    return message;
                }
            }
        })
        .await
        .expect("an upstream message")
    }

    /// The next message the client receives.
    async fn client_message(&mut self) -> WsMessage {
        let mut buf = vec![0u8; 64 * 1024];
        tokio::time::timeout(WAIT, async {
            loop {
                if let Some(message) = self.inbox.pop_front() {
                    return message;
                }
                let n = self.client.read(&mut buf).await.unwrap();
                assert!(n > 0, "the client connection ended");
                self.inbox.extend(self.from_client.push(&buf[..n]));
            }
        })
        .await
        .expect("a client message")
    }

    async fn finish(self) {
        drop(self.client);
        drop(self.upstream);
        tokio::time::timeout(WAIT, self.relay)
            .await
            .expect("the relay ends")
            .unwrap();
    }
}

fn text(message: WsMessage) -> String {
    match message {
        WsMessage::Text(text) => text,
        other => panic!("expected a text message, got {other:?}"),
    }
}

/// A Redact message streamed through the relay is forwarded exactly as the
/// buffered path masks it, at every lag and frame size; the detected secret
/// never reaches the upstream outside the opaque state both paths keep; the
/// findings are recorded once per message; a pretty-printed server event
/// carrying a reference is restored for the client.
#[tokio::test]
async fn streamed_websocket_redaction_matches_the_buffered_path() {
    let token = github_token();
    let body = conversation(&token, &token);
    let bytes = encode(&body);
    for (lag, frame) in [
        (bytes.len() + 1, 997),
        (4096, 1),
        (4096, 333),
        (16 * 1024, 4096),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (firewall, mut base) = fixture(
            root.path(),
            lr_config::LlmMode::Proxy,
            Arc::new(lr_api_keys::MockKeychain::new()),
        );
        let firewall = Arc::new(firewall);
        base.body = body.clone();
        let mut connection = connect(firewall.clone(), &base, lag);
        connection
            .client
            .write_all(&frames(&bytes, frame))
            .await
            .unwrap();
        let forwarded = text(connection.upstream_message().await);
        let RequestAction::Protect {
            body: buffered,
            vault: _,
        } = firewall.evaluate(&base).await
        else {
            panic!("Redact protects")
        };
        let forwarded_value: Value = serde_json::from_str(&forwarded).unwrap();
        let buffered_value: Value = serde_json::from_slice(&buffered).unwrap();
        assert_eq!(forwarded_value, buffered_value, "lag {lag} frame {frame}");
        assert_eq!(forwarded.matches(token.as_str()).count(), 1, "lag {lag}");
        assert!(forwarded.contains(&format!("opaque-{token}")));
        let findings = secret_scan_findings(&firewall);
        assert_eq!(findings.len(), 2, "lag {lag}");
        assert_eq!(findings[0], findings[1], "lag {lag}");
        assert!(!findings[0].is_empty());

        // The server echoes a reference; the client gets the original.
        let reference = forwarded_value["input"][13]["output"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(!reference.contains(&token));
        let event = serde_json::to_string_pretty(&json!({
            "type": "response.output_text.done", "item_id": "msg_1",
            "output_index": 0, "content_index": 0, "text": reference,
        }))
        .unwrap();
        connection
            .upstream
            .write_all(&encode_frame(&WsMessage::Text(event), false))
            .await
            .unwrap();
        let delivered = text(connection.client_message().await);
        assert!(delivered.contains(&token), "lag {lag}: {delivered}");
        connection.finish().await;
    }
}

/// Notify forwards the client's bytes unchanged and records the findings
/// once.
#[tokio::test]
async fn streamed_websocket_notify_forwards_the_original_bytes() {
    let body = conversation(&aws_key(), &github_token());
    let bytes = encode(&body);
    let root = tempfile::tempdir().unwrap();
    let (firewall, base) = fixture(
        root.path(),
        lr_config::LlmMode::Proxy,
        Arc::new(lr_api_keys::MockKeychain::new()),
    );
    firewall
        .config_manager
        .update(|config| config.secret_scanning.action = lr_config::SecretScanAction::Notify)
        .unwrap();
    let firewall = Arc::new(firewall);
    let mut connection = connect(firewall.clone(), &base, 4096);
    connection
        .client
        .write_all(&frames(&bytes, 512))
        .await
        .unwrap();
    let forwarded = text(connection.upstream_message().await);
    assert_eq!(forwarded.as_bytes(), bytes.as_slice());
    let findings = secret_scan_findings(&firewall);
    assert_eq!(findings.len(), 1);
    assert!(!findings[0].is_empty());
    connection.finish().await;
}

/// Ask: each finding is a popup while nothing is released; RedactOnce masks
/// it in what the upstream receives. Deny before anything was released
/// answers the client with the denial and keeps the socket open; Deny after
/// earlier units were released closes the upstream with 1008 before the
/// finding's content left, and the client gets the denial and a 1011 close.
#[tokio::test]
async fn streamed_websocket_ask_answers_apply() {
    use lr_mcp::gateway::firewall::FirewallApprovalAction as A;
    let token = github_token();
    let body = conversation(&aws_key(), &token);
    let bytes = encode(&body);
    for (answer, released_first) in [(A::RedactOnce, false), (A::Deny, false), (A::Deny, true)] {
        let root = tempfile::tempdir().unwrap();
        let (firewall, base) = fixture(
            root.path(),
            lr_config::LlmMode::Proxy,
            Arc::new(lr_api_keys::MockKeychain::new()),
        );
        firewall
            .config_manager
            .update(|config| config.secret_scanning.action = lr_config::SecretScanAction::Ask)
            .unwrap();
        let firewall = Arc::new(firewall);
        let mut connection = connect(firewall.clone(), &base, 4096);
        let answering = {
            let firewall = firewall.clone();
            let answer = answer.clone();
            let token = token.clone();
            tokio::spawn(async move {
                let mut asked = Vec::new();
                loop {
                    let pending = tokio::time::timeout(
                        std::time::Duration::from_secs(3),
                        next_approval(&firewall),
                    )
                    .await;
                    let Ok(pending) = pending else {
                        return asked;
                    };
                    let details = pending.secret_scan_details.clone().unwrap();
                    assert_eq!(details.findings.len(), 1, "one finding per popup");
                    let raw = details.findings[0].raw_text.clone();
                    let action = if raw.contains(&token) {
                        answer.clone()
                    } else {
                        A::AllowOnce
                    };
                    asked.push(raw);
                    firewall
                        .firewall
                        .submit_response(&pending.request_id, action, None)
                        .unwrap();
                }
            })
        };
        let wire = frames(&bytes, 700);
        if released_first {
            // Earlier units are released (more than the lag behind the
            // head) before the token's unit arrives.
            let before = String::from_utf8_lossy(&bytes)
                .find(token.as_str())
                .unwrap()
                - 200;
            connection.client.write_all(&wire[..before]).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            connection.client.write_all(&wire[before..]).await.unwrap();
        } else {
            connection.client.write_all(&wire).await.unwrap();
        }
        match answer {
            A::Deny if !released_first => {
                // The question stopped the release before the first fragment.
                let reply: Value =
                    serde_json::from_str(&text(connection.client_message().await)).unwrap();
                assert_eq!(reply["type"], "error");
                assert_eq!(reply["status"], 403);
                let mut probe = [0u8; 1];
                let read = tokio::time::timeout(
                    std::time::Duration::from_millis(300),
                    connection.upstream.read(&mut probe),
                )
                .await;
                assert!(read.is_err(), "the upstream received nothing");
            }
            A::Deny => {
                let WsMessage::Close(code) = connection.upstream_message().await else {
                    panic!("expected the upstream to be closed");
                };
                assert_eq!(code[..2], 1008u16.to_be_bytes());
                let reply: Value =
                    serde_json::from_str(&text(connection.client_message().await)).unwrap();
                assert_eq!(reply["type"], "error");
                assert_eq!(reply["status"], 403);
                let WsMessage::Close(code) = connection.client_message().await else {
                    panic!("expected the client to be closed");
                };
                assert_eq!(code[..2], 1011u16.to_be_bytes());
            }
            _ => {
                let forwarded = text(connection.upstream_message().await);
                let value: Value = serde_json::from_str(&forwarded).unwrap();
                assert!(!value["input"][12].to_string().contains(&token));
                assert!(!value["input"][13].to_string().contains(&token));
            }
        }
        let asked = answering.await.unwrap();
        assert_eq!(
            asked.iter().filter(|raw| raw.contains(&token)).count(),
            1,
            "{answer:?}"
        );
        connection.finish().await;
    }
}
