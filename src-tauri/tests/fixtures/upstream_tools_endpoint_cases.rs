use super::*;

async fn gateway() -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"test"}]})))
        .mount(&mock)
        .await;
    mock
}
fn upstream(mock: &MockServer) -> Upstream {
    (
        "gw",
        "llmgateway",
        vec![("base_url", mock.uri()), ("api_key", "fixture".into())],
    )
}
fn allow(config: &mut AppConfig) {
    allow_replay_fixture(config);
    config.clients[0].security.upstream_tools.allowed_tools =
        vec![lr_config::UpstreamToolKind::WebSearch];
    config.clients[0].security.upstream_tools.max_tool_calls = None;
}
fn chat() -> Value {
    json!({"model":"gw/test","messages":[{"role":"user","content":"Find a source"}],"tools":[{"type":"web_search"}]})
}
#[tokio::test]
async fn server_tools_require_explicit_current_permission_and_verified_limits_before_io() {
    let mock = gateway().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock)
        .await;
    let (base, secret, state) =
        start_replay_test_server(vec![upstream(&mock)], allow_replay_fixture, true).await;
    let response = post(&base, "/v1/chat/completions", &secret, &chat()).await;
    assert_eq!(response.status(), 400);
    assert!(response.text().await.unwrap().contains("not enabled"));
    state
        .config_manager
        .update(|config| {
            allow(config);
            config.clients[0].security.upstream_tools.max_tool_calls = Some(8);
        })
        .unwrap();
    let response = post(&base, "/v1/chat/completions", &secret, &chat()).await;
    assert_eq!(response.status(), 400);
    assert!(response.text().await.unwrap().contains("step-cap"));
    state.config_manager.update(allow).unwrap();
    let mut injected = chat();
    injected["extensions"] = json!({"_localrouter_server_tools":{"tools":[]}});
    assert_eq!(
        post(&base, "/v1/chat/completions", &secret, &injected)
            .await
            .status(),
        400
    );
    let mut filtered = chat();
    filtered["tools"][0]["filters"] = json!({"allowed_domains":["example.com"]});
    assert_eq!(
        post(&base, "/v1/chat/completions", &secret, &filtered)
            .await
            .status(),
        400
    );
    state
        .config_manager
        .update(|config| {
            allow(config);
            config.strategies[0].free_tier_only = true;
        })
        .unwrap();
    assert_eq!(
        post(&base, "/v1/chat/completions", &secret, &chat())
            .await
            .status(),
        400
    );
    state
        .config_manager
        .update(|config| {
            config.strategies[0].free_tier_only = false;
            config.clients[0]
                .security
                .upstream_tools
                .allowed_tools
                .clear();
        })
        .unwrap();
    assert_eq!(
        post(&base, "/v1/chat/completions", &secret, &chat())
            .await
            .status(),
        400
    );
    mock.verify().await;
}
#[tokio::test]
async fn server_search_preserves_normal_and_streaming_citations_across_chat_and_responses() {
    let mock = gateway().await;
    let citation = json!({"type":"url_citation","url":"https://example.com/source","title":"Source","start_index":0,"end_index":2});
    let value = json!({"id":"search","object":"chat.completion","created":1,"model":"test","choices":[{"index":0,"message":{"role":"assistant","content":"ok","annotations":[citation.clone()]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2,"server_tool_use":{"web_search_requests":1}}});
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("x-no-fallback", "true"))
        .and(body_partial_json(
            json!({"tools":[{"type":"web_search"}],"stream":false}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .expect(2)
        .mount(&mock)
        .await;
    let chunk = json!({"id":"stream","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{"role":"assistant","content":"ok","annotations":[citation.clone()]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2,"server_tool_use":{"web_search_requests":1}}});
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("x-no-fallback", "true"))
        .and(body_partial_json(
            json!({"tools":[{"type":"web_search"}],"stream":true}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(format!("data: {chunk}\n\ndata: [DONE]\n\n"))
                .insert_header("content-type", "text/event-stream"),
        )
        .expect(2)
        .mount(&mock)
        .await;
    let (base, secret, _) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    for endpoint in ["/v1/chat/completions", "/v1/responses"] {
        for stream in [false, true] {
            let mut body = if endpoint.ends_with("responses") {
                json!({"model":"gw/test","input":"Find a source","store":false,"tools":[{"type":"web_search"}]})
            } else {
                chat()
            };
            body["stream"] = json!(stream);
            let response = post(&base, endpoint, &secret, &body).await;
            let status = response.status();
            let text = response.text().await.unwrap();
            assert_eq!(status, 200, "{endpoint} stream={stream}: {text}");
            assert!(text.contains("https://example.com/source"), "{text}");
            assert!(text.contains("url_citation"), "{text}");
        }
    }
    mock.verify().await;
}
