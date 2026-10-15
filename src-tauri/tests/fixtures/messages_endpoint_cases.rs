//! Native Messages cases share the authenticated HTTP server fixture.
use super::*;

pub(super) struct TestNativeMessagesFactory;
impl localrouter::providers::factory::ProviderFactory for TestNativeMessagesFactory {
    fn provider_type(&self) -> &str {
        "native_messages_fixture"
    }
    fn display_name(&self) -> &str {
        "Native Messages fixture"
    }
    fn category(&self) -> localrouter::providers::factory::ProviderCategory {
        localrouter::providers::factory::ProviderCategory::FirstParty
    }
    fn description(&self) -> &str {
        "Loopback native Messages"
    }
    fn setup_parameters(&self) -> Vec<localrouter::providers::factory::SetupParameter> {
        vec![]
    }
    fn validate_config(&self, _: &HashMap<String, String>) -> lr_types::AppResult<()> {
        Ok(())
    }
    fn create(
        &self,
        _: String,
        config: HashMap<String, String>,
    ) -> lr_types::AppResult<Arc<dyn ModelProvider>> {
        Ok(Arc::new(
            localrouter::providers::anthropic::AnthropicProvider::with_base_url(
                config["api_key"].clone(),
                config["base_url"].clone(),
            )?,
        ))
    }
}

async fn native_mock() -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/models")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"claude-native-fixture","display_name":"Native fixture","type":"model"}],"has_more":false}))).mount(&mock).await;
    mock
}
fn native_provider(mock: &MockServer) -> Upstream {
    (
        "native",
        "native_messages_fixture",
        vec![
            ("base_url", format!("{}/v1", mock.uri())),
            ("api_key", "configured-provider-key".into()),
        ],
    )
}
fn message_body() -> Value {
    json!({"model":"native/claude-native-fixture","max_tokens":20,"messages":[{"role":"user","content":"hello"}]})
}
fn message_answer() -> Value {
    json!({"id":"msg_native_fixture","type":"message","role":"assistant","model":"claude-native-fixture","content":[{"type":"text","text":"hello back"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":3,"output_tokens":2}})
}

#[tokio::test]
async fn native_messages_preserves_signed_client_tool_loop_and_bypasses_exact_cache() {
    let upstream = native_mock().await;
    let signed =
        json!({"type":"thinking","thinking":"original\nreasoning","signature":"signature+/="});
    let tool =
        json!({"type":"tool_use","id":"toolu_fixture","name":"lookup","input":{"query":"Toronto"}});
    let mut first = message_answer();
    first["content"] = json!([signed, tool]);
    first["stop_reason"] = json!("tool_use");
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(wiremock::matchers::body_json(
            json!({"model":"claude-native-fixture","max_tokens":20,"stream":false,"messages":[{"role":"user","content":"hello"}]}),
        ))
        .and(header("x-api-key", "configured-provider-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(first.clone()))
        .expect(1)
        .mount(&upstream)
        .await;
    let mut followup = message_body();
    followup["max_tokens"] = json!(0);
    followup["system"] =
        json!([{"type":"text","text":"rules","cache_control":{"type":"ephemeral","ttl":"1h"}}]);
    followup["messages"] = json!([{"role":"user","content":"hello"},{"role":"assistant","content":[signed,tool]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_fixture","content":"Toronto is sunny"}]}]);
    let mut expected = followup.clone();
    expected["model"] = json!("claude-native-fixture");
    expected["stream"] = json!(false);
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(wiremock::matchers::body_json(expected))
        .and(header("x-api-key", "configured-provider-key"))
        .and(header("anthropic-beta", "fixture-native-beta"))
        .respond_with(ResponseTemplate::new(200).set_body_json(message_answer()))
        .expect(2)
        .mount(&upstream)
        .await;
    let (base, secret, state) = start_replay_test_server(
        vec![native_provider(&upstream)],
        |config| {
            allow_replay_fixture(config);
            config.strategies[0].response_cache.enabled = true;
        },
        true,
    )
    .await;
    let client = reqwest::Client::new();
    let initial = client
        .post(format!("{base}/v1/messages"))
        .header("x-api-key", &secret)
        .json(&message_body())
        .send()
        .await
        .unwrap();
    assert_eq!(initial.status(), 200);
    assert_eq!(initial.json::<Value>().await.unwrap(), first);
    // The response uses the native tool/thinking envelope, not Chat choices.
    let first_result = upstream.received_requests().await.unwrap();
    assert!(!first_result.is_empty());
    for _ in 0..2 {
        let response = client
            .post(format!("{base}/v1/messages"))
            .header("x-api-key", &secret)
            .header("anthropic-beta", "fixture-native-beta")
            .header("x-stainless-os", "do-not-forward")
            .json(&followup)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.json::<Value>().await.unwrap(), message_answer());
    }
    assert_eq!(
        state.router.response_cache_stats(Some("test-api-key")).hits,
        0
    );
    for request in upstream
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST")
    {
        assert!(!request.headers.contains_key("authorization"));
        assert!(!request.headers.contains_key("x-stainless-os"));
    }
}

#[tokio::test]
async fn native_messages_count_tokens_is_authenticated_and_not_billed_as_inference() {
    let upstream = native_mock().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages/count_tokens"))
        .and(wiremock::matchers::body_json(
            json!({"model":"claude-native-fixture","messages":[{"role":"user","content":"hello"}]}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"input_tokens":17})))
        .expect(2)
        .mount(&upstream)
        .await;
    let (base, secret, state) =
        start_replay_test_server(vec![native_provider(&upstream)], allow_replay_fixture, true)
            .await;
    state.rate_limiter.add_api_key_limiters(
        "test-api-key".into(),
        vec![
            localrouter::router::rate_limit::RateLimiter::new(
                localrouter::router::rate_limit::RateLimitType::Cost,
                100.0,
                60,
            ),
            localrouter::router::rate_limit::RateLimiter::new(
                localrouter::router::rate_limit::RateLimitType::TotalTokens,
                10000.0,
                60,
            ),
        ],
    );
    let body = json!({"model":"native/claude-native-fixture","messages":[{"role":"user","content":"hello"}]});
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .post(format!("{base}/v1/messages/count_tokens"))
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let response = client
        .post(format!("{base}/v1/messages/count_tokens"))
        .header("x-api-key", &secret)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>().await.unwrap()["input_tokens"], 17);
    let direct = client
        .post(format!("{base}/v1/messages/count_tokens"))
        .bearer_auth(state.internal_test_secret.as_str())
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = direct.status();
    let result: Value = direct.json().await.unwrap();
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["input_tokens"], 17);
    for kind in [
        localrouter::router::rate_limit::RateLimitType::Cost,
        localrouter::router::rate_limit::RateLimitType::TotalTokens,
    ] {
        let usage = state
            .rate_limiter
            .get_api_key_usage("test-api-key", kind)
            .await
            .unwrap();
        assert_eq!(usage.0, 0.0);
    }
}

#[tokio::test]
async fn native_messages_rejects_unowned_actions_and_bound_auto_history_before_io() {
    let upstream = native_mock().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&upstream)
        .await;
    let (base, secret, _) =
        start_replay_test_server(vec![native_provider(&upstream)], allow_replay_fixture, true)
            .await;
    let mut bodies = Vec::new();
    let mut body = message_body();
    body["tools"] = json!([{"type":"web_search_20250305","name":"web_search"}]);
    bodies.push(body);
    let mut body = message_body();
    body["container"] = json!("other-client-container");
    bodies.push(body);
    let mut body = message_body();
    body["messages"] = json!([{"role":"user","content":[{"type":"document","source":{"type":"file","file_id":"foreign-file"}}]}]);
    bodies.push(body);
    let mut body = message_body();
    body["model"] = json!("localrouter/auto");
    body["messages"] = json!([{"role":"assistant","content":[{"type":"thinking","thinking":"protected","signature":"signed"}]}]);
    bodies.push(body);
    for body in bodies {
        let response = post(&base, "/v1/messages", &secret, &body).await;
        assert_eq!(response.status(), 400, "{}", response.text().await.unwrap());
    }
}

#[tokio::test]
async fn native_messages_stream_preserves_signature_and_terminal_error_without_done() {
    let upstream = native_mock().await;
    let events = [
        json!({"type":"message_start","message":{"id":"msg_stream","model":"claude-native-fixture","usage":{"input_tokens":3,"output_tokens":0}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"日本語"}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig+/="}}),
        json!({"type":"error","error":{"type":"overloaded_error","message":"fixture busy"}}),
    ];
    let wire = events
        .iter()
        .map(|value| {
            format!(
                "event: {}\ndata: {value}\n\n",
                value["type"].as_str().unwrap()
            )
        })
        .collect::<String>();
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(wire, "text/event-stream"))
        .expect(1)
        .mount(&upstream)
        .await;
    let (base, secret, _) =
        start_replay_test_server(vec![native_provider(&upstream)], allow_replay_fixture, true)
            .await;
    let mut body = message_body();
    body["stream"] = json!(true);
    let response = post(&base, "/v1/messages", &secret, &body).await;
    assert_eq!(response.status(), 200);
    let wire = response.text().await.unwrap();
    assert!(wire.contains("event: content_block_delta"), "{wire}");
    assert!(wire.contains("sig+/="), "{wire}");
    assert!(wire.contains("日本語"), "{wire}");
    assert_eq!(wire.matches("event: error").count(), 1, "{wire}");
    assert!(!wire.contains("[DONE]"), "{wire}");
    assert!(!wire.contains("message_stop"), "{wire}");
}

#[tokio::test]
async fn native_messages_translation_preserves_plain_client_tool_history_and_rejects_native_only_options(
) {
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":[{"id":"messages-translation-fixture"}]})),
        )
        .mount(&upstream)
        .await;
    Mock::given(method("POST")).and(path("/chat/completions"))
        .and(body_partial_json(json!({"model":"messages-translation-fixture","messages":[{"role":"user","content":"hello"},{"role":"assistant","tool_calls":[{"id":"toolu_local","type":"function","function":{"name":"lookup","arguments":"{\"query\":\"Toronto\"}"}}]},{"role":"tool","tool_call_id":"toolu_local","content":"sunny"}]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"chat_native_translation","object":"chat.completion","created":1,"model":"messages-translation-fixture","choices":[{"index":0,"message":{"role":"assistant","content":"Toronto is sunny"},"finish_reason":"stop"}],"usage":{"prompt_tokens":9,"completion_tokens":3,"total_tokens":12}}))).expect(1).mount(&upstream).await;
    let (base, secret, _) = start_replay_test_server(
        vec![(
            "compat",
            "llmgateway",
            vec![
                ("base_url", upstream.uri()),
                ("api_key", "configured-key".into()),
            ],
        )],
        allow_replay_fixture,
        true,
    )
    .await;
    let mut body = json!({"model":"compat/messages-translation-fixture","max_tokens":32,"tools":[{"name":"lookup","input_schema":{"type":"object"}}],"messages":[{"role":"user","content":"hello"},{"role":"assistant","content":[{"type":"tool_use","id":"toolu_local","name":"lookup","input":{"query":"Toronto"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_local","content":"sunny"}]}]});
    let response = post(&base, "/v1/messages", &secret, &body).await;
    assert_eq!(response.status(), 200);
    let answer: Value = response.json().await.unwrap();
    assert_eq!(answer["content"][0]["text"], "Toronto is sunny");
    assert_eq!(answer["stop_reason"], "end_turn");
    body["thinking"] = json!({"type":"enabled","budget_tokens":1024});
    let denied = post(&base, "/v1/messages", &secret, &body).await;
    assert_eq!(denied.status(), 400);
    let error: Value = denied.json().await.unwrap();
    assert_eq!(error["type"], "error");
    assert_eq!(error["error"]["type"], "invalid_request_error");
    let denied=reqwest::Client::new().post(format!("{base}/v1/chat/completions")).header("x-api-key",&secret).json(&json!({"model":"compat/messages-translation-fixture","messages":[{"role":"user","content":"hello"}]})).send().await.unwrap();
    assert_eq!(denied.status(), 401);
}

#[tokio::test]
async fn native_messages_errors_use_sdk_envelope_and_preserve_retry_after() {
    let upstream = native_mock().await;
    Mock::given(method("POST")).and(path("/v1/messages")).respond_with(ResponseTemplate::new(429).insert_header("retry-after","12").set_body_json(json!({"type":"error","error":{"type":"rate_limit_error","message":"fixture limited"}}))).expect(1).mount(&upstream).await;
    let (base, secret, _) =
        start_replay_test_server(vec![native_provider(&upstream)], allow_replay_fixture, true)
            .await;
    let denied = reqwest::Client::new()
        .post(format!("{base}/v1/messages"))
        .json(&message_body())
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);
    assert_eq!(
        denied.json::<Value>().await.unwrap()["error"]["type"],
        "authentication_error"
    );
    let response = post(&base, "/v1/messages", &secret, &message_body()).await;
    assert_eq!(response.status(), 429);
    assert_eq!(response.headers()["retry-after"], "12");
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "rate_limit_error");
}

#[tokio::test]
async fn native_messages_bound_auto_history_requires_a_current_native_session_pin() {
    let upstream = native_mock().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(message_answer()))
        .expect(2)
        .mount(&upstream)
        .await;
    let (base, secret, state) = start_replay_test_server(
        vec![native_provider(&upstream)],
        |config| {
            allow_replay_fixture(config);
            let auto = config.strategies[0].auto_config.as_mut().unwrap();
            auto.prioritized_models = vec![("native".into(), "claude-native-fixture".into())];
            auto.session_routing.enabled = true;
        },
        true,
    )
    .await;
    state
        .provider_registry
        .list_provider_models_cached("native")
        .await
        .unwrap();
    let mut body = message_body();
    body["model"] = json!("localrouter/auto");
    body["extensions"] = json!({"localrouter.session":{"id":"native-session"}});
    let first = post(&base, "/v1/messages", &secret, &body).await;
    assert_eq!(first.status(), 200, "{}", first.text().await.unwrap());
    body["messages"] = json!([{"role":"user","content":"hello"},{"role":"assistant","content":[{"type":"thinking","thinking":"preserved","signature":"signed+/="},{"type":"text","text":"hello back"}]},{"role":"user","content":"continue"}]);
    body["extensions"] =
        json!({"localrouter.session":{"id":"native-session","require_continuity":true}});
    let second = post(&base, "/v1/messages", &secret, &body).await;
    assert_eq!(second.status(), 200, "{}", second.text().await.unwrap());
    body["extensions"] =
        json!({"localrouter.session":{"id":"no-such-pin","require_continuity":true}});
    assert_eq!(
        post(&base, "/v1/messages", &secret, &body).await.status(),
        400
    );
    for request in upstream
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST")
    {
        assert!(!String::from_utf8_lossy(&request.body).contains("native-session"));
    }
}

#[tokio::test]
async fn native_messages_mcp_plain_subset_uses_the_existing_orchestrator_and_protected_history_fails(
) {
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":[{"id":"messages-mcp-fixture"}]})),
        )
        .mount(&upstream)
        .await;
    Mock::given(method("POST")).and(path("/chat/completions")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"mcp_reply","object":"chat.completion","created":1,"model":"messages-mcp-fixture","choices":[{"index":0,"message":{"role":"assistant","content":"authorized MCP path"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}))).expect(1).mount(&upstream).await;
    let (base, secret, _) = start_replay_test_server(
        vec![(
            "compat",
            "llmgateway",
            vec![("base_url", upstream.uri()), ("api_key", "key".into())],
        )],
        |config| {
            allow_replay_fixture(config);
            config.clients[0].mcp_mode = localrouter::config::McpMode::ViaLlm;
        },
        true,
    )
    .await;
    let mut body = json!({"model":"compat/messages-mcp-fixture","max_tokens":20,"messages":[{"role":"user","content":"hello"}]});
    let response = post(&base, "/v1/messages", &secret, &body).await;
    let status = response.status();
    let answer: Value = response.json().await.unwrap();
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["content"][0]["text"], "authorized MCP path");
    body["messages"] = json!([{"role":"assistant","content":[{"type":"thinking","thinking":"protected","signature":"signed"}]}]);
    assert_eq!(
        post(&base, "/v1/messages", &secret, &body).await.status(),
        400
    );
}

#[tokio::test]
async fn native_messages_openapi_describes_local_api_key_auth_and_native_contracts() {
    let upstream = native_mock().await;
    let (base, _, _) =
        start_replay_test_server(vec![native_provider(&upstream)], allow_replay_fixture, true)
            .await;
    let spec: Value = reqwest::get(format!("{base}/openapi.json"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(spec["paths"]["/v1/messages"]["post"].is_object());
    assert!(spec["paths"]["/v1/messages/count_tokens"]["post"].is_object());
    assert!(spec["components"]["schemas"]["MessagesRequest"].is_object());
    assert!(spec["components"]["schemas"]["MessagesErrorResponse"].is_object());
    assert_eq!(
        spec["components"]["securitySchemes"]["anthropic_api_key"]["name"],
        "x-api-key"
    );
}

#[tokio::test]
async fn native_messages_bare_sdk_models_resolve_only_when_unambiguous() {
    let upstream = native_mock().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(message_answer()))
        .expect(1)
        .mount(&upstream)
        .await;
    let (base, secret, _) =
        start_replay_test_server(vec![native_provider(&upstream)], allow_replay_fixture, true)
            .await;
    let body = json!({"model":"claude-native-fixture","max_tokens":4,"messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"protected","signature":"original"}]},{"role":"user","content":"continue"}]});
    let response = post(&base, "/v1/messages", &secret, &body).await;
    let status = response.status();
    let answer: Value = response.json().await.unwrap();
    assert_eq!(status, 200, "{answer}");
    let second = native_mock().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&second)
        .await;
    let (ambiguous, token, _) = start_replay_test_server(
        vec![
            native_provider(&second),
            (
                "other",
                "native_messages_fixture",
                vec![
                    ("base_url", format!("{}/v1", second.uri())),
                    ("api_key", "other-key".into()),
                ],
            ),
        ],
        allow_replay_fixture,
        true,
    )
    .await;
    let denied = post(&ambiguous, "/v1/messages", &token, &body).await;
    assert_eq!(denied.status(), 400);
    assert!(denied.text().await.unwrap().contains("unambiguous"));
}
