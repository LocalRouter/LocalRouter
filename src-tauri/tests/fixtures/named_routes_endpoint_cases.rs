//! Named aliases use the existing authenticated pipeline and fallback machinery.
use super::*;
#[tokio::test]
async fn named_rules_preserve_alias_fallback_streaming_permissions_and_private_context() {
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":[{"id":"fast"},{"id":"general"}]})),
        )
        .mount(&upstream)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(json!({"model":"fast","stream":false})))
        .respond_with(
            ResponseTemplate::new(503).set_body_json(
                json!({"error":{"message":"fixture failure","type":"server_error"}}),
            ),
        )
        .expect(1)
        .mount(&upstream)
        .await;
    Mock::given(method("POST")).and(path("/chat/completions")).and(body_partial_json(json!({"model":"general","stream":false}))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"named-result","object":"chat.completion","created":1,"model":"general","choices":[{"index":0,"message":{"role":"assistant","content":"answer"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}))).expect(3).mount(&upstream).await;
    let stream="data: {\"id\":\"named-stream\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"fast\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"stream answer\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"named-stream\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"fast\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(json!({"model":"fast","stream":true})))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(stream),
        )
        .expect(1)
        .mount(&upstream)
        .await;
    let (base,secret,state)=start_replay_test_server(vec![("gw","llmgateway",vec![("base_url",upstream.uri()),("api_key","fixture".into())])],|config|{
        allow_replay_fixture(config);
        let strategy=&mut config.strategies[0];let auto=strategy.auto_config.as_mut().unwrap();auto.model_name="localrouter/named".into();auto.prioritized_models=vec![("gw".into(),"general".into())];
        auto.routing_policy=Some(serde_json::from_value(json!({"enabled":true,"default_route":"general","options":[{"id":"fast","description":"Short work","models":[["gw","fast"]]},{"id":"general","description":"Default","models":[["gw","general"]]}],"rules":[{"id":"short","condition":{"client_ids":["test-api-key"],"max_context_chars":5},"branches":[{"route":"fast","weight":10000}]}]})).unwrap());
        strategy.route_versions.draft=Some(localrouter::config::RouteSnapshot::from(&*auto));strategy.route_versions.publish(auto,"initial".into(),chrono::Utc::now().timestamp()).unwrap();
    },true).await;
    state
        .provider_registry
        .list_provider_models_cached("gw")
        .await
        .unwrap();
    let http = reqwest::Client::new();
    let models: Value = http
        .get(format!("{base}/v1/models"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(models["data"]
        .as_array()
        .unwrap()
        .iter()
        .any(|model| model["id"] == "localrouter/named"));
    for prompt in ["hi", "a longer request"] {
        let response=http.post(format!("{base}/v1/chat/completions")).bearer_auth(&secret).json(&json!({"model":"localrouter/named","messages":[{"role":"user","content":prompt}],"extensions":{"_localrouter_route_client":"spoofed"}})).send().await.unwrap();
        assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["model"], "general");
        let explanation = &body["extensions"]["localrouter.routing"]["explanation"];
        assert_eq!(explanation["named_route"]["alias"], "localrouter/named");
        assert_eq!(explanation["named_route"]["revision"], 1);
        assert_eq!(
            explanation["active_route"],
            if prompt == "hi" { "fast" } else { "general" }
        );
    }
    let response=http.post(format!("{base}/v1/chat/completions")).bearer_auth(&secret).json(&json!({"model":"localrouter/named","stream":true,"messages":[{"role":"user","content":"hi"}]})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let raw = response.text().await.unwrap();
    assert!(raw.contains("stream answer"), "{raw}");
    assert!(raw.contains("named_route"), "{raw}");
    state
        .config_manager
        .update(|config| {
            config.strategies[0]
                .model_permissions
                .models
                .insert("gw__fast".into(), localrouter::config::PermissionState::Off);
        })
        .unwrap();
    let response = http
        .post(format!("{base}/v1/chat/completions"))
        .bearer_auth(&secret)
        .json(&json!({"model":"localrouter/named","messages":[{"role":"user","content":"hi"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    let calls = upstream.received_requests().await.unwrap();
    assert!(calls
        .iter()
        .filter(|r| r.method == "POST")
        .all(|r| !String::from_utf8_lossy(&r.body).contains("_localrouter_route_client")));
}

#[tokio::test]
async fn draft_edits_preserve_response_cache_and_publication_changes_its_identity() {
    let (_upstream, base, secret, state) = exact_cache_fixture(2, true).await;
    let http = reqwest::Client::new();
    async fn call(http: &reqwest::Client, base: &str, secret: &str) {
        let response = http
            .post(format!("{base}/v1/chat/completions"))
            .bearer_auth(secret)
            .json(&exact_cache_chat())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    }
    call(&http, &base, &secret).await;
    state
        .config_manager
        .update(|config| {
            let strategy = &mut config.strategies[0];
            strategy.route_versions.draft = Some(localrouter::config::RouteSnapshot::from(
                strategy.auto_config.as_ref().unwrap(),
            ));
        })
        .unwrap();
    call(&http, &base, &secret).await;
    state
        .config_manager
        .update(|config| {
            let strategy = &mut config.strategies[0];
            strategy
                .route_versions
                .publish(
                    strategy.auto_config.as_mut().unwrap(),
                    "cache revision".into(),
                    chrono::Utc::now().timestamp(),
                )
                .unwrap();
        })
        .unwrap();
    call(&http, &base, &secret).await;
}

#[tokio::test]
async fn replay_evaluates_authenticated_rules_locally_and_records_route_metadata() {
    let (base, _, state) = start_replay_test_server(vec![], allow_replay_fixture, true).await;
    let directory = tempfile::tempdir().unwrap();
    let manager = Arc::new(lr_replay::ReplayManager::new(
        directory.path().join("replay"),
    ));
    let mut request = replay_request();
    request.targets=vec![lr_replay::ReplayTarget::Policy {name:"Authenticated rule".into(),policy:Box::new(serde_json::from_value(json!({"enabled":true,"default_route":"other","options":[{"id":"mine","description":"This client","models":[]},{"id":"other","description":"Other clients","models":[]}],"rules":[{"id":"client","condition":{"client_ids":["test-api-key"]},"branches":[{"route":"mine","weight":10000}]}]})).unwrap())}];
    request.cases[0].checks = vec![lr_replay::TaskCheck::ExpectedRoute {
        route: "mine".into(),
    }];
    let context = lr_replay::RunContext::new(state, format!("{base}/")).unwrap();
    let estimates = manager.preview(context.clone(), request.clone()).unwrap();
    assert_eq!(estimates[0].estimated_usd, Some(0.0));
    let id = manager.start(context, request).unwrap();
    let run = await_replay(&manager, &id).await;
    let result = &run.results[0];
    assert_eq!(
        result.status,
        lr_replay::ResultStatus::Success,
        "{result:?}"
    );
    assert!(result.checks[0].passed);
    assert!(!result.http_request_sent);
    assert_eq!(
        result.response.as_ref().unwrap()["decision"]["source"],
        "deterministic_rule"
    );
    assert!(result.policy_snapshot.as_ref().unwrap()["named_route"].is_object());
}
