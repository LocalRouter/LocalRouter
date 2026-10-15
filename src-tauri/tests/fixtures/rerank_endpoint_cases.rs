use super::*;
async fn mock() -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[]})))
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
    config.clients[0].security.native_operations = vec![lr_config::NativeOperation::Rerank];
}
fn input() -> Value {
    json!({"model":"gw/qwen3-reranker-8b","query":"source query","documents":["first source","second source"],"top_n":2,"return_documents":true})
}
fn result() -> Value {
    json!({"id":"native-rank","results":[{"index":1,"relevance_score":0.9,"document":{"text":"mismatched upstream text"}},{"index":0,"relevance_score":0.3}],"meta":{"billed_units":{"input_tokens":55,"total_tokens":55}}})
}
#[tokio::test]
async fn rerank_preserves_native_order_original_sources_and_units_on_both_authenticated_aliases() {
    let mock = mock().await;
    Mock::given(method("POST")).and(path("/rerank")).and(header("x-no-fallback","true")).and(body_partial_json(json!({"model":"qwen3-reranker-8b","query":"source query","documents":["first source","second source"],"top_n":2}))).respond_with(ResponseTemplate::new(200).set_body_json(result())).expect(2).mount(&mock).await;
    let (base, secret, state) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    for endpoint in ["/v1/rerank", "/rerank"] {
        let response = post(&base, endpoint, &secret, &input()).await;
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["id"], "native-rank");
        assert_eq!(body["results"][0]["index"], 1);
        assert_eq!(body["results"][0]["document"]["text"], "second source");
        assert_eq!(body["meta"]["billed_units"]["input_tokens"], 55);
        assert!(body["extensions"]["localrouter.rerank"]["cost_usd"].is_null());
    }
    for request in mock
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method == "POST")
    {
        let body: Value = request.body_json().unwrap();
        assert!(body.get("return_documents").is_none());
    }
    let strategy = state.config_manager.get().strategies[0].id.clone();
    let usage = state
        .metrics_collector
        .get_recent_usage_for_strategy(&strategy, 60);
    assert_eq!(usage.0, 2);
    assert_eq!(usage.1, 110);
    mock.verify().await;
}
#[tokio::test]
async fn rerank_requires_a_distinct_live_grant_and_bounds_before_io() {
    let mock = mock().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock)
        .await;
    let (base, secret, state) = start_replay_test_server(
        vec![upstream(&mock)],
        |c| {
            allow_replay_fixture(c);
            c.clients[0].security.native_operations = vec![lr_config::NativeOperation::Ocr];
        },
        true,
    )
    .await;
    assert_eq!(
        post(&base, "/rerank", &secret, &input()).await.status(),
        400
    );
    state.config_manager.update(allow).unwrap();
    for patch in [
        json!({"documents":[]}),
        json!({"query":""}),
        json!({"top_n":3}),
        json!({"model":"localrouter/auto"}),
        json!({"max_tokens_per_doc":4}),
        json!({"max_chunks_per_doc":4}),
    ] {
        let mut body = input();
        body.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert_eq!(post(&base, "/rerank", &secret, &body).await.status(), 400);
    }
    state
        .config_manager
        .update(|c| c.clients[0].security.native_operations.clear())
        .unwrap();
    assert_eq!(
        post(&base, "/rerank", &secret, &input()).await.status(),
        400
    );
    mock.verify().await;
}
#[tokio::test]
async fn rerank_rejects_strict_privacy_and_unverified_cost_or_token_ceilings() {
    let mock = mock().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock)
        .await;
    let (base, secret, state) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    for limit_type in [
        lr_config::RateLimitType::Cost,
        lr_config::RateLimitType::InputTokens,
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
            post(&base, "/rerank", &secret, &input()).await.status(),
            400
        );
    }
    state
        .config_manager
        .update(|c| {
            c.strategies[0].rate_limits.clear();
            c.clients[0].security.privacy.no_retention = true;
        })
        .unwrap();
    assert_eq!(
        post(&base, "/rerank", &secret, &input()).await.status(),
        400
    );
    state
        .config_manager
        .update(|c| {
            c.clients[0].security.privacy.no_retention = false;
            c.strategies[0].free_tier_only = true;
        })
        .unwrap();
    assert_eq!(
        post(&base, "/rerank", &secret, &input()).await.status(),
        400
    );
    mock.verify().await;
}
#[tokio::test]
async fn invalid_native_rankings_do_not_retry_and_consume_one_request_quota() {
    let mock = mock().await;
    Mock::given(method("POST"))
        .and(path("/rerank"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"results":[{"index":999,"relevance_score":0.9}]})),
        )
        .expect(1)
        .mount(&mock)
        .await;
    let (base, secret, _) = start_replay_test_server(
        vec![upstream(&mock)],
        |c| {
            allow(c);
            c.strategies[0].rate_limits = vec![lr_config::StrategyRateLimit {
                limit_type: lr_config::RateLimitType::Requests,
                value: 1.0,
                time_window: lr_config::RateLimitTimeWindow::Minute,
                enabled: true,
            }];
        },
        true,
    )
    .await;
    let response = post(&base, "/rerank", &secret, &input()).await;
    assert_eq!(response.status(), 502);
    assert_eq!(response.headers()["x-should-retry"], "false");
    assert_eq!(
        post(&base, "/rerank", &secret, &input()).await.status(),
        429
    );
    mock.verify().await;
}
#[tokio::test]
async fn rerank_alias_requires_auth_and_openapi_registers_the_native_contract() {
    let mock = mock().await;
    let (base, _, _) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    assert_eq!(
        reqwest::Client::new()
            .post(format!("{base}/rerank"))
            .json(&input())
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let schema: Value = reqwest::get(format!("{base}/openapi.json"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(schema["paths"]["/v1/rerank"]["post"].is_object());
    assert!(schema["components"]["schemas"]["RerankRequest"]["required"]
        .as_array()
        .unwrap()
        .contains(&json!("documents")));
}

#[tokio::test]
async fn cancelled_rerank_dispatch_keeps_request_quota_and_does_not_replay() {
    let mock = mock().await;
    Mock::given(method("POST"))
        .and(path("/rerank"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(result())
                .set_delay(Duration::from_secs(5)),
        )
        .expect(1)
        .mount(&mock)
        .await;
    let (_, _, state) = start_replay_test_server(
        vec![upstream(&mock)],
        |c| {
            allow(c);
            c.strategies[0].rate_limits = vec![lr_config::StrategyRateLimit {
                limit_type: lr_config::RateLimitType::Requests,
                value: 1.0,
                time_window: lr_config::RateLimitTimeWindow::Minute,
                enabled: true,
            }];
        },
        true,
    )
    .await;
    let router = state.router.clone();
    let client = state.config_manager.get().clients[0].id.clone();
    let request: lr_providers::rerank::RerankRequest = serde_json::from_value(input()).unwrap();
    let dispatch_client = client.clone();
    let dispatch_request = request.clone();
    let task = tokio::spawn(async move { router.rerank(&dispatch_client, dispatch_request).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if mock
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|r| r.method == "POST")
            {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    assert!(state.router.rerank(&client, request).await.is_err());
    mock.verify().await;
}
