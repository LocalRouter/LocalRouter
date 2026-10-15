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
fn allow(c: &mut AppConfig) {
    allow_replay_fixture(c);
    c.clients[0].security.native_operations = vec![lr_config::NativeOperation::Video];
}
fn input() -> Value {
    json!({"model":"gw/native-video","prompt":"A quiet river","seconds":"4"})
}
async fn create(base: &str, token: &str, key: &str, body: &Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{base}/v1/videos"))
        .bearer_auth(token)
        .header("Idempotency-Key", key)
        .json(body)
        .send()
        .await
        .unwrap()
}
async fn get(base: &str, token: &str, route: &str) -> reqwest::Response {
    reqwest::Client::new()
        .get(format!("{base}{route}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}
#[tokio::test]
async fn videos_concurrent_retries_dispatch_once_and_job_fees_are_not_poll_charges() {
    let mock = mock().await;
    Mock::given(method("POST")).and(path("/videos")).and(header("authorization","Bearer fixture")).and(header("x-no-fallback","true")).and(body_partial_json(json!({"model":"native-video","seconds":4,"size":"1280x720"}))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"native_1","status":"queued","progress":0,"url":"https://untrusted.invalid/private"})).set_delay(Duration::from_millis(150))).expect(1).mount(&mock).await;
    Mock::given(method("GET"))
        .and(path("/videos/native_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"id":"native_1","status":"completed","progress":100,"usage":{"cost":0.4}}),
        ))
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/videos/native_1/content"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "video/mp4")
                .set_body_bytes(b"fixture-mp4".to_vec()),
        )
        .expect(1)
        .mount(&mock)
        .await;
    let (base, secret, state) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    let body = input();
    let (a, b) = tokio::join!(
        create(&base, &secret, "intent-one", &body),
        create(&base, &secret, "intent-one", &body)
    );
    let sa = a.status();
    let sb = b.status();
    let a: Value = a.json().await.unwrap();
    let b: Value = b.json().await.unwrap();
    assert_eq!(sa, 200, "{a}");
    assert_eq!(sb, 200, "{b}");
    assert_eq!(a["id"], b["id"]);
    let id = a["id"].as_str().unwrap();
    assert!(id.starts_with("video_lr_"));
    assert!(!a.to_string().contains("native_1"));
    assert!(!a.to_string().contains("untrusted.invalid"));
    let mut changed = input();
    changed["prompt"] = json!("Different request");
    assert_eq!(
        create(&base, &secret, "intent-one", &changed)
            .await
            .status(),
        409
    );
    for _ in 0..2 {
        let r = get(&base, &secret, &format!("/videos/{id}")).await;
        let status = r.status();
        let value: Value = r.json().await.unwrap();
        assert_eq!(status, 200, "{value}");
        assert_eq!(value["status"], "completed");
        assert_eq!(value["extensions"]["localrouter.video"]["cost_usd"], 0.4);
    }
    let list: Value = get(&base, &secret, "/videos").await.json().await.unwrap();
    assert_eq!(list["data"].as_array().unwrap().len(), 1);
    assert_eq!(
        list["extensions"]["localrouter.video"]["known_subtotal_usd"],
        0.4
    );
    let media = get(&base, &secret, &format!("/v1/videos/{id}/content")).await;
    assert_eq!(media.status(), 200);
    assert_eq!(media.headers()["content-type"], "video/mp4");
    assert_eq!(media.bytes().await.unwrap().as_ref(), b"fixture-mp4");
    let mut other = state.config_manager.get().clients[0].clone();
    other.id = "another-video-client".into();
    let other_secret = state
        .client_manager
        .add_client_with_secret(other.clone())
        .unwrap();
    state
        .config_manager
        .update(|c| c.clients.push(other))
        .unwrap();
    for suffix in ["", "/content"] {
        assert_eq!(
            get(&base, &other_secret, &format!("/videos/{id}{suffix}"))
                .await
                .status(),
            404
        );
    }
    assert_eq!(get(&base, &secret, "/videos/native_1").await.status(), 404);
    let strategy = state.config_manager.get().strategies[0].id.clone();
    assert_eq!(
        state
            .metrics_collector
            .get_recent_usage_for_strategy(&strategy, 60)
            .0,
        1
    );
    mock.verify().await;
}
#[tokio::test]
async fn videos_ambiguous_create_retains_unknown_intent_and_never_retries_or_cancels_upstream() {
    let mock = mock().await;
    Mock::given(method("POST"))
        .and(path("/videos"))
        .respond_with(ResponseTemplate::new(503).set_body_string("private provider diagnostic"))
        .expect(1)
        .mount(&mock)
        .await;
    let (base, secret, _) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    let response = create(&base, &secret, "ambiguous", &input()).await;
    assert!(response.status().is_server_error());
    assert_eq!(response.headers()["x-should-retry"], "false");
    let body: Value = response.json().await.unwrap();
    assert!(!body.to_string().contains("private provider diagnostic"));
    let id = body["extensions"]["localrouter.video"]["id"]
        .as_str()
        .unwrap();
    let retry: Value = create(&base, &secret, "ambiguous", &input())
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(retry["id"], id);
    assert_eq!(retry["status"], "unknown");
    assert!(retry["extensions"]["localrouter.video"]["cost_usd"].is_null());
    let cached: Value = get(&base, &secret, &format!("/videos/{id}"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(cached["status"], "unknown");
    assert_eq!(
        post(&base, &format!("/videos/{id}/cancel"), &secret, &json!({}))
            .await
            .status(),
        501
    );
    mock.verify().await;
}
#[tokio::test]
async fn videos_validate_permissions_inputs_and_unsupported_budgets_before_native_io() {
    let mock = mock().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock)
        .await;
    let (base, secret, state) =
        start_replay_test_server(vec![upstream(&mock)], allow_replay_fixture, true).await;
    assert_eq!(
        create(&base, &secret, "disabled", &input()).await.status(),
        400
    );
    state.config_manager.update(allow).unwrap();
    assert_eq!(
        post(&base, "/videos", &secret, &input()).await.status(),
        400
    );
    for patch in [
        json!({"seconds":0}),
        json!({"size":"1920x1080"}),
        json!({"input_reference":"https://invalid"}),
        json!({"callback_url":"https://invalid"}),
        json!({"model":"localrouter/auto"}),
        json!({"prompt":""}),
    ] {
        let mut body = input();
        body.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert_eq!(create(&base, &secret, "invalid", &body).await.status(), 400);
    }
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
            create(&base, &secret, "budget", &input()).await.status(),
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
        create(&base, &secret, "privacy", &input()).await.status(),
        400
    );
    mock.verify().await;
}
#[tokio::test]
async fn videos_sdk_multipart_and_live_schema_preserve_text_fields_and_explicit_duration() {
    let mock = mock().await;
    let prompt = "p".repeat(2048);
    Mock::given(method("POST"))
        .and(path("/videos"))
        .and(body_partial_json(json!({"prompt":prompt,"seconds":4})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id":"native_form","status":"queued"})),
        )
        .expect(1)
        .mount(&mock)
        .await;
    let (base, secret, _) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    let form = reqwest::multipart::Form::new()
        .text("model", "gw/native-video")
        .text("prompt", prompt)
        .text("seconds", "4");
    let response = reqwest::Client::new()
        .post(format!("{base}/videos"))
        .bearer_auth(&secret)
        .header("Idempotency-Key", "sdk-form")
        .multipart(form)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    assert_eq!(status, 200, "{body}");
    let spec: Value = get(&base, &secret, "/openapi.json")
        .await
        .json()
        .await
        .unwrap();
    let content = &spec["paths"]["/v1/videos"]["post"]["requestBody"]["content"];
    assert!(content.get("application/json").is_some());
    assert!(content.get("multipart/form-data").is_some());
    assert!(spec["paths"]["/v1/videos/{video_id}/content"].is_object());
    assert_eq!(
        reqwest::get(format!("{base}/videos"))
            .await
            .unwrap()
            .status(),
        401
    );
    mock.verify().await;
}
#[tokio::test]
async fn videos_job_binding_and_current_grant_are_rechecked_before_status_io() {
    let mock = mock().await;
    Mock::given(method("POST"))
        .and(path("/videos"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"native_bound","status":"queued"})),
        )
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/videos/native_bound"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock)
        .await;
    let (base, secret, state) = start_replay_test_server(vec![upstream(&mock)], allow, true).await;
    let job: Value = create(&base, &secret, "binding", &input())
        .await
        .json()
        .await
        .unwrap();
    let route = format!("/videos/{}", job["id"].as_str().unwrap());
    state
        .config_manager
        .update(|c| c.clients[0].security.native_operations.clear())
        .unwrap();
    assert_eq!(get(&base, &secret, &route).await.status(), 400);
    state.config_manager.update(allow).unwrap();
    state.provider_registry.remove_provider("gw").unwrap();
    state
        .provider_registry
        .create_provider(
            "gw".into(),
            "llmgateway".into(),
            HashMap::from([
                ("base_url".into(), mock.uri()),
                ("api_key".into(), "changed-credential".into()),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(get(&base, &secret, &route).await.status(), 409);
    mock.verify().await;
}

#[tokio::test]
async fn videos_request_quota_counts_unknown_dispatch_but_not_idempotent_replay_or_rejected_generation(
) {
    let mock = mock().await;
    Mock::given(method("POST"))
        .and(path("/videos"))
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
    assert_eq!(
        create(&base, &secret, "first-intent", &input())
            .await
            .status(),
        502
    );
    let replay = create(&base, &secret, "first-intent", &input()).await;
    assert_eq!(replay.status(), 200);
    assert_eq!(replay.json::<Value>().await.unwrap()["status"], "unknown");
    let rejected = create(&base, &secret, "next-intent", &input()).await;
    assert_eq!(rejected.status(), 429);
    let error: Value = rejected.json().await.unwrap();
    assert_eq!(
        error["extensions"]["localrouter.video"]["dispatch_outcome"],
        "not_dispatched"
    );
    let retry: Value = create(&base, &secret, "next-intent", &input())
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        retry["extensions"]["localrouter.video"]["cost_usd"].as_f64(),
        Some(0.0)
    );
    assert_eq!(
        retry["extensions"]["localrouter.video"]["cost_source"],
        "not_dispatched"
    );
    mock.verify().await;
}
