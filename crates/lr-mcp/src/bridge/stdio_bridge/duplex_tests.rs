//! End-to-end tests of the bridge against a fake Streamable HTTP gateway.

use super::*;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncWriteExt, DuplexStream, Lines};
use tokio::sync::broadcast;

const SECRET: &str = "lr-test-secret";
const SESSION: &str = "sess-1";
const RECV_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
struct Recorded {
    method: Method,
    headers: HeaderMap,
    body: Option<Value>,
}

#[derive(Clone)]
struct Gateway {
    log: Arc<parking_lot::Mutex<Vec<Recorded>>>,
    /// Data lines pushed to every open GET stream; `"__close__"` ends them.
    stream: broadcast::Sender<String>,
    gets: Arc<AtomicUsize>,
}

impl Gateway {
    fn posts(&self) -> Vec<Recorded> {
        self.log
            .lock()
            .iter()
            .filter(|r| r.method == Method::POST)
            .cloned()
            .collect()
    }

    fn post_for(&self, rpc_method: &str) -> Option<Recorded> {
        self.posts()
            .into_iter()
            .find(|r| r.body.as_ref().and_then(|b| b["method"].as_str()) == Some(rpc_method))
    }

    fn of(&self, method: Method) -> Vec<Recorded> {
        self.log
            .lock()
            .iter()
            .filter(|r| r.method == method)
            .cloned()
            .collect()
    }

    /// Wait until `n` GET streams have been opened and one is listening.
    async fn wait_for_stream(&self, n: usize) {
        let deadline = tokio::time::Instant::now() + RECV_TIMEOUT;
        while self.gets.load(Ordering::SeqCst) < n || self.stream.receiver_count() == 0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "bridge never opened GET stream #{n}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn push(&self, message: Value) {
        self.stream.send(message.to_string()).unwrap();
    }
}

fn sse_body(messages: Vec<Value>) -> Response {
    let stream = futures_util::stream::iter(
        messages
            .into_iter()
            .map(|m| Ok::<_, Infallible>(Event::default().event("message").data(m.to_string()))),
    );
    Sse::new(stream).into_response()
}

async fn gateway_handler(
    State(gw): State<Gateway>,
    method: Method,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let body: Option<Value> = serde_json::from_slice(&body).ok();
    gw.log.lock().push(Recorded {
        method: method.clone(),
        headers: headers.clone(),
        body: body.clone(),
    });
    if headers.get("authorization").and_then(|v| v.to_str().ok())
        != Some(&format!("Bearer {SECRET}"))
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    match method {
        Method::GET => {
            gw.gets.fetch_add(1, Ordering::SeqCst);
            let mut rx = gw.stream.subscribe();
            let stream = async_stream::stream! {
                yield Ok::<_, Infallible>(Event::default().event("endpoint").data("/?sessionId=legacy"));
                while let Ok(data) = rx.recv().await {
                    if data == "__close__" {
                        break;
                    }
                    yield Ok(Event::default().event("message").data(data));
                }
            };
            Sse::new(stream)
                .keep_alive(KeepAlive::default())
                .into_response()
        }
        Method::DELETE => StatusCode::NO_CONTENT.into_response(),
        Method::POST => {
            let body = body.unwrap_or(Value::Null);
            let id = body.get("id").cloned().unwrap_or(Value::Null);
            let Some(rpc) = body.get("method").and_then(Value::as_str) else {
                // Client response to a server request.
                return StatusCode::ACCEPTED.into_response();
            };
            if id.is_null() {
                return StatusCode::ACCEPTED.into_response();
            }
            match rpc {
                "initialize" => (
                    [(MCP_SESSION_ID_HEADER, SESSION)],
                    Json(json!({"jsonrpc":"2.0","id":id,"result":{
                        "protocolVersion":"2026-07-28",
                        "capabilities":{"tools":{},"experimental":{"claude/channel":{}}},
                        "serverInfo":{"name":"LocalRouter","version":"test"}
                    }})),
                )
                    .into_response(),
                "events/stream" => sse_body(vec![
                    json!({"jsonrpc":"2.0","method":"notifications/events/active","params":{"n":1}}),
                    json!({"jsonrpc":"2.0","method":"notifications/events/event","params":{"n":2}}),
                    json!({"jsonrpc":"2.0","method":"notifications/events/heartbeat","params":{"n":3}}),
                    json!({"jsonrpc":"2.0","id":id,"result":{"done":true}}),
                ]),
                "slow" => {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    Json(json!({"jsonrpc":"2.0","id":id,"result":{"pad":"x".repeat(32 * 1024)}}))
                        .into_response()
                }
                "async" => StatusCode::ACCEPTED.into_response(),
                "unsupported" => (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32022,"message":"nope"}})),
                )
                    .into_response(),
                _ => Json(json!({"jsonrpc":"2.0","id":id,"result":{"method":rpc}})).into_response(),
            }
        }
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

async fn start_gateway() -> (Gateway, String) {
    let (stream, _) = broadcast::channel(4096);
    let gw = Gateway {
        log: Default::default(),
        stream,
        gets: Default::default(),
    };
    let app = Router::new()
        .route("/mcp", any(gateway_handler))
        .with_state(gw.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (gw, url)
}

struct Harness {
    stdin: Option<DuplexStream>,
    stdout: Lines<BufReader<DuplexStream>>,
    bridge: tokio::task::JoinHandle<AppResult<()>>,
}

impl Harness {
    fn start(url: &str) -> Self {
        let bridge = StdioBridge::with_endpoint("test".into(), SECRET.into(), url.into())
            .unwrap()
            .with_stream_backoff(Duration::from_millis(20));
        let (stdin, bridge_in) = tokio::io::duplex(1 << 20);
        let (bridge_out, stdout) = tokio::io::duplex(1 << 20);
        let bridge = tokio::spawn(bridge.run_with_io(bridge_in, bridge_out));
        Self {
            stdin: Some(stdin),
            stdout: BufReader::new(stdout).lines(),
            bridge,
        }
    }

    async fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        stdin
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn recv(&mut self) -> Value {
        let line = tokio::time::timeout(RECV_TIMEOUT, self.stdout.next_line())
            .await
            .expect("timed out waiting for stdout")
            .unwrap()
            .expect("stdout closed");
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("bad stdout line {line:?}: {e}"))
    }

    async fn initialize(&mut self) -> Value {
        self.send(
            json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
            "protocolVersion":"2026-07-28","capabilities":{},
            "clientInfo":{"name":"claude-code","version":"test"}}}),
        )
        .await;
        let result = self.recv().await;
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await;
        result
    }
}

#[tokio::test]
async fn initialize_captures_session_and_reuses_it() {
    let (gw, url) = start_gateway().await;
    let mut h = Harness::start(&url);

    let init = h.initialize().await;
    assert_eq!(init["id"], 0);
    // Gateway said 2026-07-28; the client only ever sees what was negotiated.
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        init["result"]["capabilities"]["experimental"]["claude/channel"],
        json!({})
    );
    let sent = gw.post_for("initialize").unwrap();
    assert_eq!(
        sent.body.unwrap()["params"]["protocolVersion"],
        "2025-11-25"
    );
    assert!(sent.headers.get(MCP_SESSION_ID_HEADER).is_none());

    h.send(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
        .await;
    // The notification wrote nothing: the next line is the tools/list answer.
    let listed = h.recv().await;
    assert_eq!(listed["id"], 1);
    assert_eq!(listed["result"]["method"], "tools/list");

    for rpc in ["notifications/initialized", "tools/list"] {
        let r = gw.post_for(rpc).unwrap();
        assert_eq!(r.headers[MCP_SESSION_ID_HEADER], SESSION, "{rpc}");
        assert_eq!(r.headers[BRIDGE_HEADER], BRIDGE_HEADER_VALUE, "{rpc}");
        assert_eq!(r.headers["mcp-protocol-version"], "2025-11-25", "{rpc}");
    }
    gw.wait_for_stream(1).await;
    let get = gw.of(Method::GET).remove(0);
    assert_eq!(get.headers[MCP_SESSION_ID_HEADER], SESSION);
    assert_eq!(get.headers[BRIDGE_HEADER], BRIDGE_HEADER_VALUE);
    assert!(get.headers["accept"]
        .to_str()
        .unwrap()
        .contains("text/event-stream"));
}

#[tokio::test]
async fn server_stream_messages_reach_stdout_verbatim_and_responses_go_back() {
    let (gw, url) = start_gateway().await;
    let mut h = Harness::start(&url);
    h.initialize().await;
    gw.wait_for_stream(1).await;

    let channel = json!({"jsonrpc":"2.0","method":"notifications/claude/channel",
        "params":{"content":"new message","meta":{"source":"vocalcord","event_id":"e1"}}});
    let listed = json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"});
    let elicit = json!({"jsonrpc":"2.0","id":"srv-1","method":"elicitation/create",
        "params":{"message":"Continue?","requestedSchema":{"type":"object"}}});
    gw.push(channel.clone());
    gw.push(listed.clone());
    gw.push(elicit.clone());
    assert_eq!(h.recv().await, channel);
    assert_eq!(h.recv().await, listed);
    assert_eq!(h.recv().await, elicit);

    let answer = json!({"jsonrpc":"2.0","id":"srv-1","result":{"action":"accept","content":{}}});
    h.send(answer.clone()).await;
    h.send(json!({"jsonrpc":"2.0","id":2,"method":"ping"}))
        .await;
    // Nothing was written for the client response.
    assert_eq!(h.recv().await["id"], 2);
    let posted = gw
        .posts()
        .into_iter()
        .find(|r| r.body.as_ref() == Some(&answer))
        .expect("client response was not POSTed");
    assert_eq!(posted.headers[MCP_SESSION_ID_HEADER], SESSION);
}

#[tokio::test]
async fn sse_post_response_forwards_every_message() {
    let (_gw, url) = start_gateway().await;
    let mut h = Harness::start(&url);
    h.initialize().await;
    h.send(json!({"jsonrpc":"2.0","id":"s1","method":"events/stream","params":{}}))
        .await;
    let mut got = Vec::new();
    for _ in 0..4 {
        got.push(h.recv().await);
    }
    let methods: Vec<_> = got.iter().map(|m| m["method"].clone()).collect();
    assert_eq!(
        methods[..3],
        [
            json!("notifications/events/active"),
            json!("notifications/events/event"),
            json!("notifications/events/heartbeat")
        ]
    );
    assert_eq!(
        got[3],
        json!({"jsonrpc":"2.0","id":"s1","result":{"done":true}})
    );
}

#[tokio::test]
async fn discover_probe_is_declined_and_initialize_negotiates_down() {
    let (gw, url) = start_gateway().await;
    let mut h = Harness::start(&url);

    h.send(
        json!({"jsonrpc":"2.0","id":"discover-1","method":"server/discover","params":{
        "_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28",
            "io.modelcontextprotocol/clientInfo":{"name":"claude-code","version":"2"},
            "io.modelcontextprotocol/clientCapabilities":{}}}}),
    )
    .await;
    let declined = h.recv().await;
    assert_eq!(declined["id"], "discover-1");
    assert_eq!(declined["error"]["code"], -32601);
    assert!(declined.get("result").is_none());

    h.send(
        json!({"jsonrpc":"2.0","id":3,"method":"tools/list","params":{
        "_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}),
    )
    .await;
    let early = h.recv().await;
    assert_eq!(early["id"], 3);
    assert_eq!(early["error"]["code"], -32600);
    assert!(
        gw.posts().is_empty(),
        "declined probes must not reach the gateway"
    );

    // The same process then accepts the legacy handshake.
    h.send(
        json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
        "protocolVersion":"2026-07-28","capabilities":{},
        "_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"},
        "clientInfo":{"name":"claude-code","version":"2"}}}),
    )
    .await;
    assert_eq!(h.recv().await["result"]["protocolVersion"], "2025-11-25");
    let forwarded = gw.post_for("initialize").unwrap().body.unwrap();
    assert_eq!(forwarded["params"]["protocolVersion"], "2025-11-25");
    assert!(forwarded["params"]["_meta"]
        .get("io.modelcontextprotocol/protocolVersion")
        .is_none());
}

#[tokio::test]
async fn stdout_lines_never_interleave_under_concurrency() {
    let (gw, url) = start_gateway().await;
    let mut h = Harness::start(&url);
    h.initialize().await;
    gw.wait_for_stream(1).await;

    const REQUESTS: usize = 40;
    const PUSHES: usize = 200;
    let pusher = {
        let gw = gw.clone();
        tokio::spawn(async move {
            for i in 0..PUSHES {
                gw.push(json!({"jsonrpc":"2.0","method":"notifications/message",
                    "params":{"i":i,"pad":"y".repeat(16 * 1024)}}));
                tokio::task::yield_now().await;
            }
        })
    };
    for i in 0..REQUESTS {
        h.send(json!({"jsonrpc":"2.0","id":100 + i,"method":"slow"}))
            .await;
    }
    let (mut responses, mut notes) = (0, 0);
    while responses + notes < REQUESTS + PUSHES {
        let m = h.recv().await; // panics on a corrupted line
        if m.get("method").is_some() {
            notes += 1;
        } else {
            assert_eq!(m["result"]["pad"].as_str().unwrap().len(), 32 * 1024);
            responses += 1;
        }
    }
    pusher.await.unwrap();
    assert_eq!((responses, notes), (REQUESTS, PUSHES));
}

#[tokio::test]
async fn server_stream_reconnects_after_drop() {
    let (gw, url) = start_gateway().await;
    let mut h = Harness::start(&url);
    h.initialize().await;
    gw.wait_for_stream(1).await;
    gw.stream.send("__close__".into()).unwrap();
    gw.wait_for_stream(2).await;
    let note = json!({"jsonrpc":"2.0","method":"notifications/resources/list_changed"});
    gw.push(note.clone());
    assert_eq!(h.recv().await, note);
    let gets = gw.of(Method::GET);
    assert!(gets
        .iter()
        .all(|g| g.headers[MCP_SESSION_ID_HEADER] == SESSION));
}

#[tokio::test]
async fn accepted_requests_are_answered_from_the_stream() {
    let (gw, url) = start_gateway().await;
    let mut h = Harness::start(&url);
    h.initialize().await;
    gw.wait_for_stream(1).await;
    h.send(json!({"jsonrpc":"2.0","id":9,"method":"async"}))
        .await;
    // 202 writes nothing; the answer arrives on the GET stream.
    let answer = json!({"jsonrpc":"2.0","id":9,"result":{"late":true}});
    tokio::time::sleep(Duration::from_millis(50)).await;
    gw.push(answer.clone());
    assert_eq!(h.recv().await, answer);
}

#[tokio::test]
async fn error_bodies_keep_the_request_id() {
    let (_gw, url) = start_gateway().await;
    let mut h = Harness::start(&url);
    h.initialize().await;
    h.send(json!({"jsonrpc":"2.0","id":4,"method":"unsupported"}))
        .await;
    let forwarded = h.recv().await;
    assert_eq!(forwarded["id"], 4);
    assert_eq!(forwarded["error"]["code"], -32022);

    h.send(json!({"jsonrpc":"2.0","id":5,"method":"ping"}))
        .await;
    assert_eq!(h.recv().await["id"], 5);
    h.send(json!({"not":"jsonrpc"})).await;
    assert_eq!(h.recv().await["error"]["code"], -32600);
    h.send(json!(5)).await;
    assert_eq!(h.recv().await["error"]["code"], -32600);
}

#[tokio::test]
async fn unreachable_gateway_errors_carry_the_request_id() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    drop(listener);
    let mut h = Harness::start(&url);
    h.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{}}))
        .await;
    h.send(json!({"jsonrpc":"2.0","id":5,"method":"tools/list"}))
        .await;
    // The notification wrote nothing; the request's error names its id.
    let err = h.recv().await;
    assert_eq!(err["id"], 5);
    assert_eq!(err["error"]["code"], -32603);
    assert!(err["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Could not connect"));
}

#[tokio::test]
async fn eof_shuts_down_and_deletes_the_session() {
    let (gw, url) = start_gateway().await;
    let mut h = Harness::start(&url);
    h.initialize().await;
    gw.wait_for_stream(1).await;
    h.stdin.take(); // EOF
    let result = tokio::time::timeout(RECV_TIMEOUT, &mut h.bridge)
        .await
        .expect("bridge did not stop on EOF")
        .unwrap();
    assert!(result.is_ok());
    let deletes = gw.of(Method::DELETE);
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0].headers[MCP_SESSION_ID_HEADER], SESSION);
    assert!(h.stdout.next_line().await.unwrap().is_none());
}

#[test]
fn server_url_resolution() {
    let server = ServerConfig {
        host: "0.0.0.0".into(),
        port: 4000,
        enable_cors: true,
    };
    assert_eq!(
        resolve_server_url(None, None, &server),
        "http://127.0.0.1:4000/mcp"
    );
    assert_eq!(
        resolve_server_url(None, Some("http://h:1/mcp".into()), &server),
        "http://h:1/mcp"
    );
    assert_eq!(
        resolve_server_url(
            Some("http://c:2/mcp".into()),
            Some("http://h:1/mcp".into()),
            &server
        ),
        "http://c:2/mcp"
    );
    let v6 = ServerConfig {
        host: "::1".into(),
        ..server.clone()
    };
    assert_eq!(resolve_server_url(None, None, &v6), "http://[::1]:4000/mcp");
    let default = ServerConfig::default();
    assert_eq!(
        resolve_server_url(None, None, &default),
        format!("http://127.0.0.1:{}/mcp", default.port)
    );
    assert!(is_loopback_url("http://localhost:3625/mcp"));
    assert!(is_loopback_url("http://[::1]:3625/mcp"));
    assert!(!is_loopback_url("http://example.com/mcp"));
}
