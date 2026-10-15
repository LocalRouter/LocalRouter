use super::*;
use futures::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as ClientMessage};
struct NativeRealtime {
    base: String,
    connections: Arc<AtomicUsize>,
    events: Arc<parking_lot::Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for NativeRealtime {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn native() -> NativeRealtime {
    let connections = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let c = connections.clone();
    let e = events.clone();
    let app=axum::Router::new().route("/models",axum::routing::get(||async{axum::Json(json!({"data":[]}))})).route("/realtime",axum::routing::get(move|headers:axum::http::HeaderMap,ws:axum::extract::ws::WebSocketUpgrade|{let c=c.clone();let e=e.clone();async move {
        assert_eq!(headers["authorization"],"Bearer fixture");assert_eq!(headers["x-no-fallback"],"true");assert!(headers.contains_key(lr_types::TRACE_HEADER));c.fetch_add(1,Ordering::SeqCst);
        ws.on_upgrade(move|mut socket|async move {
            use axum::extract::ws::Message;
            let mut session=json!({"type":"realtime","model":"native-realtime","max_output_tokens":1024,"audio":{"input":{"turn_detection":null}},"tools":[]});
            let _=socket.send(Message::Text(json!({"type":"session.created","session":session}).to_string().into())).await;
            let mut response=0;
            while let Some(Ok(message))=socket.next().await {
                let Message::Text(text)=message else {if matches!(message,Message::Close(_)){break;}continue;};
                let value:Value=serde_json::from_str(&text).unwrap();e.lock().push(value.clone());
                let outputs=match value["type"].as_str() {
                    Some("session.update")=>{session.as_object_mut().unwrap().extend(value["session"].as_object().unwrap().clone());vec![json!({"type":"session.updated","session":session})]},
                    Some("conversation.item.create")=>vec![json!({"type":"conversation.item.created","item":{"id":"item_native"}})],
                    Some("response.create")=>{response+=1;let id=format!("resp_{response}");let mut frames=vec![json!({"type":"response.created","response":{"id":id}}),json!({"type":"response.output_text.delta","delta":"Native fixture answer"})];if value["response"]["metadata"]["fixture_hold"]!="true" {let done=json!({"type":"response.done","response":{"id":id,"status":"completed","usage":{"input_tokens":10,"output_tokens":2}}});frames.extend([done.clone(),done]);}frames},
                    Some("response.cancel")=>{let done=json!({"type":"response.done","response":{"id":format!("resp_{response}"),"status":"cancelled","usage":{"input_tokens":10,"output_tokens":2}}});vec![done.clone(),done]},
                    _=>vec![],
                };
                for output in outputs {if socket.send(Message::Text(output.to_string().into())).await.is_err(){return;}}
            }
        })
    }}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    NativeRealtime {
        base,
        connections,
        events,
        task,
    }
}
fn upstream(mock: &NativeRealtime) -> Upstream {
    (
        "gw",
        "llmgateway",
        vec![
            ("base_url", mock.base.clone()),
            ("api_key", "fixture".into()),
        ],
    )
}
fn allow(c: &mut AppConfig) {
    allow_replay_fixture(c);
    c.clients[0].security.native_operations = vec![lr_config::NativeOperation::Realtime];
}
type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn connect(base: &str, secret: &str) -> Ws {
    let mut request = format!(
        "{}/v1/realtime?model=gw/native-realtime",
        base.replace("http://", "ws://")
    )
    .into_client_request()
    .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {secret}").parse().unwrap());
    tokio_tungstenite::connect_async(request).await.unwrap().0
}
async fn event(socket: &mut Ws, kind: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let frame = socket.next().await.expect("socket closed").unwrap();
            if let ClientMessage::Text(text) = frame {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["type"] == kind {
                    return value;
                }
                assert_ne!(value["type"], "error", "{value}");
            }
        }
    })
    .await
    .unwrap()
}
async fn mint_ticket(base: &str, secret: &str) -> Value {
    let response = post(
        base,
        "/realtime/client_secrets",
        secret,
        &json!({"session":{"type":"realtime","model":"gw/native-realtime"}}),
    )
    .await;
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    assert_eq!(status, 200, "{body}");
    body
}
#[tokio::test]
async fn realtime_relays_real_native_events_cancellation_and_deduplicated_usage_without_exposing_keys(
) {
    let mock = native().await;
    let (base, secret, state) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    let mut socket = connect(&base, &secret).await;
    // SDKs may send events immediately after opening. They must wait in the
    // socket until native setup is acknowledged rather than bypassing caps.
    socket.send(ClientMessage::Text(json!({"type":"session.update","session":{"instructions":"Local fixture instructions"}}).to_string())).await.unwrap();
    socket
        .send(ClientMessage::Text(
            json!({"type":"input_audio_buffer.append","audio":"AQID"}).to_string(),
        ))
        .await
        .unwrap();
    socket.send(ClientMessage::Text(json!({"type":"conversation.item.create","item":{"type":"message","role":"user","content":[{"type":"input_text","text":"Actual native input"}]}}).to_string())).await.unwrap();
    socket
        .send(ClientMessage::Text(
            json!({"type":"response.create","response":{"output_modalities":["text"],"metadata":{"fixture_hold":"true"}}}).to_string(),
        ))
        .await
        .unwrap();
    let answer = event(&mut socket, "response.output_text.delta").await;
    assert_eq!(answer["delta"], "Native fixture answer");
    socket
        .send(ClientMessage::Text(
            json!({"type":"response.cancel"}).to_string(),
        ))
        .await
        .unwrap();
    for _ in 0..2 {
        assert_eq!(
            event(&mut socket, "response.done").await["response"]["status"],
            "cancelled"
        );
    }
    socket.close(None).await.unwrap();
    let strategy = state.config_manager.get().strategies[0].id.clone();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let usage = state
                .metrics_collector
                .get_recent_usage_for_strategy(&strategy, 60);
            if usage.0 == 2 {
                assert_eq!(usage.1, 0);
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if mock
                .events
                .lock()
                .iter()
                .any(|e| e["type"] == "response.cancel")
            {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let found = state
                .monitor_store
                .list(0, 100, None)
                .events
                .iter()
                .filter_map(|e| state.monitor_store.get(&e.id))
                .any(|e| {
                    let value = serde_json::to_value(&e).unwrap();
                    value["data"]["endpoint"] == "/v1/realtime"
                        && value["data"]["input_tokens"] == 10
                        && value["data"]["output_tokens"] == 2
                });
            if found {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(mock.connections.load(Ordering::SeqCst), 1);
    assert!(mock
        .events
        .lock()
        .iter()
        .any(|e| e["type"] == "input_audio_buffer.append" && e["audio"] == "AQID"));
    let events = mock.events.lock();
    assert_eq!(events[0]["session"]["max_output_tokens"], 1024);
    assert!(events[0]["session"]["audio"]["input"]["turn_detection"].is_null());
    assert!(events
        .iter()
        .any(|e| e["item"]["content"][0]["text"] == "Actual native input"));
    assert!(!serde_json::to_string(&*events).unwrap().contains(&secret));
}

#[tokio::test]
async fn realtime_browser_ticket_is_one_use_model_pinned_and_cannot_authenticate_other_routes() {
    let mock = native().await;
    let (base, secret, _) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    let ticket = mint_ticket(&base, &secret).await;
    let spec: Value = reqwest::get(format!("{base}/openapi.json"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(spec["paths"]["/v1/realtime"]["get"]["responses"]["101"].is_object());
    assert!(spec["paths"]["/v1/realtime/client_secrets"]["post"]["requestBody"].is_object());
    assert!(spec["components"]["schemas"]["TicketResponse"].is_object());
    let value = ticket["value"].as_str().unwrap();
    assert!(value.starts_with("ek_lr_"));
    assert_eq!(
        reqwest::Client::new()
            .get(format!("{base}/v1/models"))
            .bearer_auth(value)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let mut request = format!("{}/realtime", base.replace("http://", "ws://"))
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        format!("realtime, openai-insecure-api-key.{value}")
            .parse()
            .unwrap(),
    );
    let (mut socket, response) = tokio_tungstenite::connect_async(request.clone())
        .await
        .unwrap();
    assert_eq!(response.headers()["sec-websocket-protocol"], "realtime");
    event(&mut socket, "session.updated").await;
    assert!(tokio_tungstenite::connect_async(request).await.is_err());
    socket.close(None).await.unwrap();
    assert_eq!(mock.connections.load(Ordering::SeqCst), 1);
    let ticket = mint_ticket(&base, &secret).await;
    let mut request = format!(
        "{}/v1/realtime?model=gw/different",
        base.replace("http://", "ws://")
    )
    .into_client_request()
    .unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", ticket["value"].as_str().unwrap())
            .parse()
            .unwrap(),
    );
    assert!(tokio_tungstenite::connect_async(request).await.is_err());
    assert_eq!(mock.connections.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn realtime_current_grant_and_secret_rotation_close_established_sockets_without_reconnecting()
{
    let mock = native().await;
    let (base, secret, state) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    let mut socket = connect(&base, &secret).await;
    event(&mut socket, "session.updated").await;
    state
        .config_manager
        .update(|c| c.clients[0].security.native_operations.clear())
        .unwrap();
    let closed = event_allow_error(&mut socket).await;
    assert_eq!(closed["error"]["code"], "session_closed");
    state.config_manager.update(allow).unwrap();
    let mut socket = connect(&base, &secret).await;
    event(&mut socket, "session.updated").await;
    let id = state.config_manager.get().clients[0].id.clone();
    state.client_manager.rotate_secret(&id).unwrap();
    assert_eq!(
        event_allow_error(&mut socket).await["error"]["code"],
        "session_closed"
    );
    assert_eq!(mock.connections.load(Ordering::SeqCst), 2);
}
async fn event_allow_error(socket: &mut Ws) -> Value {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if let Some(Ok(ClientMessage::Text(text))) = socket.next().await {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "error" {
                    return value;
                }
            } else {
                panic!("missing explicit error event");
            }
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn realtime_rejects_model_escape_binary_frames_missing_grants_and_unverified_budgets() {
    let mock = native().await;
    let (base, secret, state) =
        start_replay_test_server(vec![upstream(&mock)], allow_replay_fixture, true).await;
    assert_eq!(
        post(
            &base,
            "/v1/realtime/client_secrets",
            &secret,
            &json!({"session":{"type":"realtime","model":"gw/native-realtime"}})
        )
        .await
        .status(),
        400
    );
    assert_eq!(mock.connections.load(Ordering::SeqCst), 0);
    state.config_manager.update(allow).unwrap();
    let mut socket = connect(&base, &secret).await;
    event(&mut socket, "session.updated").await;
    socket
        .send(ClientMessage::Text(
            json!({"type":"session.update","session":{"model":"other"}}).to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(event_allow_error(&mut socket).await["type"], "error");
    let mut socket = connect(&base, &secret).await;
    event(&mut socket, "session.updated").await;
    socket
        .send(ClientMessage::Binary(vec![1, 2, 3]))
        .await
        .unwrap();
    assert_eq!(event_allow_error(&mut socket).await["type"], "error");
    state
        .config_manager
        .update(|c| c.clients[0].security.privacy.no_retention = true)
        .unwrap();
    assert_eq!(
        post(
            &base,
            "/realtime/client_secrets",
            &secret,
            &json!({"session":{"type":"realtime","model":"gw/native-realtime"}})
        )
        .await
        .status(),
        400
    );
    assert_eq!(mock.connections.load(Ordering::SeqCst), 2);
    state
        .config_manager
        .update(|c| c.clients[0].security.privacy.no_retention = false)
        .unwrap();
    for limit_type in [
        lr_config::RateLimitType::Cost,
        lr_config::RateLimitType::TotalTokens,
    ] {
        state
            .config_manager
            .update(|c| {
                c.strategies[0].rate_limits = vec![lr_config::StrategyRateLimit {
                    limit_type,
                    value: 1.0,
                    time_window: lr_config::RateLimitTimeWindow::Minute,
                    enabled: true,
                }]
            })
            .unwrap();
        assert_eq!(
            post(
                &base,
                "/realtime/client_secrets",
                &secret,
                &json!({"session":{"type":"realtime","model":"gw/native-realtime"}})
            )
            .await
            .status(),
            400
        );
    }
    assert_eq!(mock.connections.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn realtime_concurrency_and_provider_binding_are_enforced_without_reconnects() {
    let mock = native().await;
    let (base, secret, state) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    let mut one = connect(&base, &secret).await;
    event(&mut one, "session.updated").await;
    let mut two = connect(&base, &secret).await;
    event(&mut two, "session.updated").await;
    let mut request = format!(
        "{}/v1/realtime?model=gw/native-realtime",
        base.replace("http://", "ws://")
    )
    .into_client_request()
    .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {secret}").parse().unwrap());
    assert!(tokio_tungstenite::connect_async(request).await.is_err());
    assert_eq!(mock.connections.load(Ordering::SeqCst), 2);
    state.provider_registry.remove_provider("gw").unwrap();
    state
        .provider_registry
        .create_provider(
            "gw".into(),
            "llmgateway".into(),
            HashMap::from([
                ("base_url".into(), mock.base.clone()),
                ("api_key".into(), "changed".into()),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(event_allow_error(&mut one).await["type"], "error");
    assert_eq!(event_allow_error(&mut two).await["type"], "error");
    assert_eq!(mock.connections.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn realtime_handshake_failure_never_retries_or_follows_credential_redirects() {
    let target = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&target)
        .await;
    for status in [307, 503] {
        let source = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[]})))
            .mount(&source)
            .await;
        Mock::given(method("GET"))
            .and(path("/realtime"))
            .and(header("authorization", "Bearer fixture"))
            .respond_with(ResponseTemplate::new(status).insert_header("location", target.uri()))
            .expect(1)
            .mount(&source)
            .await;
        let (base, secret, _) = start_replay_test_server(
            vec![(
                "gw",
                "llmgateway",
                vec![("base_url", source.uri()), ("api_key", "fixture".into())],
            )],
            allow,
            true,
        )
        .await;
        let mut socket = connect(&base, &secret).await;
        assert_eq!(
            event_allow_error(&mut socket).await["error"]["code"],
            "session_closed"
        );
        source.verify().await;
    }
    target.verify().await;
}
#[tokio::test]
async fn realtime_request_quota_counts_active_connections_before_allowing_another_response() {
    let mock = native().await;
    let (base, secret, _) = start_replay_test_server(
        vec![upstream(&mock)],
        |c| {
            allow(c);
            c.strategies[0].rate_limits = vec![lr_config::StrategyRateLimit {
                limit_type: lr_config::RateLimitType::Requests,
                value: 2.0,
                time_window: lr_config::RateLimitTimeWindow::Minute,
                enabled: true,
            }];
        },
        true,
    )
    .await;
    let mut socket = connect(&base, &secret).await;
    event(&mut socket, "session.updated").await;
    socket
        .send(ClientMessage::Text(
            json!({"type":"response.create"}).to_string(),
        ))
        .await
        .unwrap();
    event(&mut socket, "response.done").await;
    socket
        .send(ClientMessage::Text(
            json!({"type":"response.create"}).to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(
        event_allow_error(&mut socket).await["error"]["code"],
        "session_closed"
    );
    assert_eq!(
        mock.events
            .lock()
            .iter()
            .filter(|e| e["type"] == "response.create")
            .count(),
        1
    );
}
