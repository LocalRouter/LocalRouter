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
    config.clients[0].security.upstream_tools.allowed_tools =
        vec![lr_config::UpstreamToolKind::WebSearch];
    config.clients[0]
        .security
        .upstream_tools
        .default_search_model = Some("gw/perplexity/perplexity-search".into());
}
fn result() -> Value {
    json!({"id":"search-upstream","model":"perplexity/perplexity-search","results":[{"title":"Primary source","url":"https://example.com/source","snippet":"Untrusted retrieved text","date":"2026-10-15"}],"server_time":"0.02"})
}
#[tokio::test]
async fn ranked_search_uses_explicit_client_default_and_keeps_unknown_fees_and_request_quotas() {
    let mock = mock().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .and(header("x-no-fallback", "true"))
        .and(body_partial_json(
            json!({"model":"perplexity/perplexity-search","query":"source","max_results":2}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(result()))
        .expect(2)
        .mount(&mock)
        .await;
    let (base, secret, state) = start_replay_test_server(
        vec![upstream(&mock)],
        |config| {
            allow(config);
            config.strategies[0].rate_limits = vec![localrouter::config::StrategyRateLimit {
                limit_type: localrouter::config::RateLimitType::Requests,
                value: 2.0,
                time_window: localrouter::config::RateLimitTimeWindow::Minute,
                enabled: true,
            }];
        },
        true,
    )
    .await;
    for endpoint in ["/v1/search", "/search"] {
        let response = post(
            &base,
            endpoint,
            &secret,
            &json!({"query":"source","max_results":2}),
        )
        .await;
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["id"], "search-upstream");
        assert_eq!(body["results"][0]["url"], "https://example.com/source");
        assert!(body.get("choices").is_none());
        assert!(body["extensions"]["localrouter.search"]["cost_usd"].is_null());
        assert_eq!(
            body["extensions"]["localrouter.search"]["cost_source"],
            "unknown"
        );
    }
    let response = post(
        &base,
        "/v1/search",
        &secret,
        &json!({"query":"source","max_results":2}),
    )
    .await;
    assert_eq!(response.status(), 429, "{}", response.text().await.unwrap());
    let strategy = &state.config_manager.get().strategies[0].id;
    assert_eq!(
        state
            .metrics_collector
            .get_recent_usage_for_strategy(strategy, 60)
            .0,
        2
    );
    let schema: Value = reqwest::get(format!("{base}/openapi.json"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(schema["paths"]["/v1/search"]["post"].is_object());
    assert!(schema["components"]["schemas"]["SearchRequest"]["properties"]["query"].is_object());
    mock.verify().await;
}
#[tokio::test]
async fn ranked_search_rejects_permissions_privacy_missing_defaults_and_unsupported_modes_before_post(
) {
    let mock = mock().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock)
        .await;
    let (base, secret, state) =
        start_replay_test_server(vec![upstream(&mock)], allow_replay_fixture, true).await;
    let body = json!({"model":"gw/perplexity/perplexity-search","query":"source"});
    assert_eq!(post(&base, "/search", &secret, &body).await.status(), 400);
    state.config_manager.update(allow).unwrap();
    for extra in [
        json!({"search_type":"people"}),
        json!({"model":"gw/sonar"}),
        json!({"search_domain_filter":["https://example.com"]}),
        json!({"query":vec!["q";6]}),
    ] {
        let mut request = body.clone();
        request
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert_eq!(
            post(&base, "/v1/search", &secret, &request).await.status(),
            400,
            "{extra}"
        );
    }
    state
        .config_manager
        .update(|config| {
            config.clients[0]
                .security
                .upstream_tools
                .default_search_model = None;
        })
        .unwrap();
    assert_eq!(
        post(&base, "/v1/search", &secret, &json!({"query":"source"}))
            .await
            .status(),
        400
    );
    state
        .config_manager
        .update(|config| {
            config.strategies[0].free_tier_only = true;
        })
        .unwrap();
    assert_eq!(
        post(&base, "/v1/search", &secret, &body).await.status(),
        400
    );
    state
        .config_manager
        .update(|config| {
            config.strategies[0].free_tier_only = false;
            config.clients[0].security.privacy.local_only = true;
        })
        .unwrap();
    assert_eq!(
        post(&base, "/v1/search", &secret, &body).await.status(),
        400
    );
    state
        .config_manager
        .update(|config| {
            config.clients[0].security.privacy.local_only = false;
            config.strategies[0].rate_limits = vec![localrouter::config::StrategyRateLimit {
                limit_type: localrouter::config::RateLimitType::Cost,
                value: 1.0,
                time_window: localrouter::config::RateLimitTimeWindow::Minute,
                enabled: true,
            }];
        })
        .unwrap();
    assert_eq!(
        post(&base, "/v1/search", &secret, &body).await.status(),
        400
    );
    mock.verify().await;
}
#[tokio::test]
async fn cancelled_ranked_search_keeps_its_dispatch_count_and_never_retries() {
    let mock = mock().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_secs(2))
                .set_body_json(result()),
        )
        .expect(1)
        .mount(&mock)
        .await;
    let (_, _, state) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    let router = state.router.clone();
    let request: lr_providers::search::SearchRequest =
        serde_json::from_value(json!({"query":"source"})).unwrap();
    let task = tokio::spawn(async move { router.ranked_search("test-api-key", request).await });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while mock
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|request| request.method.as_str() != "POST")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "Search was never dispatched"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let strategy = &state.config_manager.get().strategies[0].id;
    assert_eq!(
        state
            .metrics_collector
            .get_recent_usage_for_strategy(strategy, 60)
            .0,
        1
    );
    let requests = mock.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method.as_str() == "POST")
            .count(),
        1
    );
}

#[tokio::test]
async fn ranked_search_errors_tell_sdks_not_to_repeat_an_uncertain_dispatch() {
    let mock = mock().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(
            ResponseTemplate::new(503)
                .set_body_json(json!({"error":{"message":"fixture failure"}})),
        )
        .expect(1)
        .mount(&mock)
        .await;
    let (base, secret, _) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    let response = post(&base, "/v1/search", &secret, &json!({"query":"source"})).await;
    assert!(response.status().is_server_error());
    assert_eq!(response.headers()["x-should-retry"], "false");
    mock.verify().await;
}
