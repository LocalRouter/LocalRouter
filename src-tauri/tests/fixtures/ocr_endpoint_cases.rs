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
    config.clients[0].security.native_operations = vec![lr_config::NativeOperation::Ocr];
}
fn input() -> Value {
    json!({"model":"gw/mistral-ocr-latest","filename":"source.pdf","document":{"type":"document_url","document_url":"data:application/pdf;base64,JVBERi0xLjc="},"pages":[0]})
}
fn result() -> Value {
    json!({"model":"mistral-ocr-latest","pages":[{"index":0,"markdown":"# Extracted source\n![untrusted](https://example.com/image.png)","images":[],"dimensions":{"dpi":200}}],"usage_info":{"pages_processed":1,"doc_size_bytes":8}})
}
#[tokio::test]
async fn ocr_requires_an_explicit_current_grant_and_valid_bounded_page_selection_before_io() {
    let mock = mock().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock)
        .await;
    let (base, secret, state) =
        start_replay_test_server(vec![upstream(&mock)], allow_replay_fixture, true).await;
    assert_eq!(
        post(&base, "/v1/ocr", &secret, &input()).await.status(),
        400
    );
    state.config_manager.update(allow).unwrap();
    for pages in [json!([]), json!([0, 0]), json!("0-20"), json!([10000])] {
        let mut body = input();
        body["pages"] = pages;
        assert_eq!(post(&base, "/ocr", &secret, &body).await.status(), 400);
    }
    let mut body = input();
    body.as_object_mut().unwrap().remove("pages");
    assert_eq!(post(&base, "/ocr", &secret, &body).await.status(), 400);
    let mut body = input();
    body["document"]["document_url"] = json!("https://example.com/remote.pdf");
    assert_eq!(post(&base, "/ocr", &secret, &body).await.status(), 400);
    let mut body = input();
    body["model"] = json!("localrouter/auto");
    assert_eq!(post(&base, "/ocr", &secret, &body).await.status(), 400);
    mock.verify().await;
}
#[tokio::test]
async fn ocr_native_wire_preserves_source_page_provenance_and_unknown_fees_on_both_aliases() {
    let mock = mock().await;
    Mock::given(method("POST")).and(path("/ocr")).and(header("x-no-fallback","true")).and(body_partial_json(json!({"model":"mistral-ocr-latest","pages":[0],"document":input()["document"],"include_image_base64":false,"image_limit":0}))).respond_with(ResponseTemplate::new(200).set_body_json(result())).expect(2).mount(&mock).await;
    let (base, secret, _) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    for endpoint in ["/v1/ocr", "/ocr"] {
        let response = post(&base, endpoint, &secret, &input()).await;
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["pages"][0]["index"], 0);
        assert_eq!(body["pages"][0]["dimensions"]["dpi"], 200);
        assert_eq!(
            body["extensions"]["localrouter.ocr"]["source"]["filename"],
            "source.pdf"
        );
        assert!(body["extensions"]["localrouter.ocr"]["cost_usd"].is_null());
        assert_eq!(body["usage_info"]["pages_processed"], 1);
    }
    for request in mock
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method == "POST")
    {
        let body: Value = request.body_json().unwrap();
        assert!(body.get("filename").is_none());
        assert!(body.get("extensions").is_none());
    }
    mock.verify().await;
}
#[tokio::test]
async fn ocr_privacy_free_only_and_live_revocation_cannot_dispatch() {
    let mock = mock().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock)
        .await;
    let (base, secret, state) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    for privacy in [
        lr_config::ClientPrivacyConfig {
            local_only: true,
            ..Default::default()
        },
        lr_config::ClientPrivacyConfig {
            no_retention: true,
            ..Default::default()
        },
    ] {
        state
            .config_manager
            .update(|c| c.clients[0].security.privacy = privacy.clone())
            .unwrap();
        assert_eq!(post(&base, "/ocr", &secret, &input()).await.status(), 400);
    }
    state
        .config_manager
        .update(|c| {
            c.clients[0].security.privacy = Default::default();
            c.strategies[0].free_tier_only = true;
        })
        .unwrap();
    assert_eq!(post(&base, "/ocr", &secret, &input()).await.status(), 400);
    state
        .config_manager
        .update(|c| {
            c.strategies[0].free_tier_only = false;
            c.clients[0].security.native_operations.clear();
        })
        .unwrap();
    assert_eq!(post(&base, "/ocr", &secret, &input()).await.status(), 400);
    mock.verify().await;
}
#[tokio::test]
async fn ocr_ambiguous_failure_is_not_retryable_and_consumes_one_request_quota() {
    let mock = mock().await;
    Mock::given(method("POST"))
        .and(path("/ocr"))
        .respond_with(ResponseTemplate::new(503))
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
    let response = post(&base, "/ocr", &secret, &input()).await;
    assert_eq!(response.status(), 502);
    assert_eq!(response.headers()["x-should-retry"], "false");
    assert_eq!(post(&base, "/ocr", &secret, &input()).await.status(), 429);
    mock.verify().await;
}
#[tokio::test]
async fn ocr_aliases_are_authenticated_and_live_openapi_registers_the_operation() {
    let mock = mock().await;
    let (base, _, _) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    assert_eq!(
        reqwest::Client::new()
            .post(format!("{base}/ocr"))
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
    assert!(schema["paths"]["/v1/ocr"]["post"].is_object());
    assert!(schema["components"]["schemas"]["OcrRequest"]["required"]
        .as_array()
        .unwrap()
        .contains(&json!("pages")));
}

#[tokio::test]
async fn cancelled_ocr_dispatch_consumes_its_request_quota_without_replaying_the_native_call() {
    let mock = mock().await;
    Mock::given(method("POST"))
        .and(path("/ocr"))
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
    let request: lr_providers::ocr::OcrRequest = serde_json::from_value(input()).unwrap();
    let dispatch_client = client.clone();
    let dispatch_request = request.clone();
    let task = tokio::spawn(async move { router.ocr(&dispatch_client, dispatch_request).await });
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
    assert!(state.router.ocr(&client, request).await.is_err());
    mock.verify().await;
}

#[tokio::test]
async fn ocr_rejects_configured_cost_limits_before_native_dispatch() {
    let mock = mock().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock)
        .await;
    let (base, secret, _) = start_replay_test_server(
        vec![upstream(&mock)],
        |c| {
            allow(c);
            c.strategies[0].rate_limits = vec![lr_config::StrategyRateLimit {
                limit_type: lr_config::RateLimitType::Cost,
                value: 1.0,
                time_window: lr_config::RateLimitTimeWindow::Minute,
                enabled: true,
            }];
        },
        true,
    )
    .await;
    assert_eq!(post(&base, "/ocr", &secret, &input()).await.status(), 400);
    mock.verify().await;
}

#[tokio::test]
async fn opaque_ocr_cannot_bypass_a_configured_blocking_secret_scan() {
    let mock = mock().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock)
        .await;
    let (base, secret, state) = start_replay_test_server(
        vec![upstream(&mock)],
        |c| {
            allow(c);
            c.secret_scanning.action = lr_config::SecretScanAction::Ask;
        },
        true,
    )
    .await;
    let scanner =
        lr_secret_scanner::SecretScanEngine::new(&lr_secret_scanner::SecretScanEngineConfig {
            entropy_threshold: 3.0,
            allowlist: vec![],
            scan_system_messages: false,
        })
        .unwrap();
    assert!(scanner.has_rules());
    *state.secret_scanner.write() = Some(Arc::new(scanner));
    let response = post(&base, "/ocr", &secret, &input()).await;
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 400, "{text}");
    assert!(text.contains("Opaque"), "{text}");
    mock.verify().await;
}
