//! Profiles reuse the authenticated gateway pipeline, including legacy aliases.
use super::*;

#[tokio::test]
async fn shared_profiles_preserve_alias_discovery_explicit_requests_permissions_streaming_and_fallback(
) {
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":[{"id":"fast"},{"id":"general"}]})),
        )
        .mount(&upstream)
        .await;
    for model in ["fast", "general"] {
        Mock::given(method("POST")).and(path("/chat/completions")).and(body_partial_json(json!({"model":model,"stream":false})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"profile-result","object":"chat.completion","created":1,"model":model,"choices":[{"index":0,"message":{"role":"assistant","content":"answer"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}))).mount(&upstream).await;
    }
    let chunk = json!({"id":"profile-stream","object":"chat.completion.chunk","created":1,"model":"fast","choices":[{"index":0,"delta":{"content":"answer"},"finish_reason":null}]});
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(json!({"model":"fast","stream":true})))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(format!("data: {chunk}\n\ndata: [DONE]\n\n")),
        )
        .mount(&upstream)
        .await;
    let (base, secret, state) = start_replay_test_server(vec![("gw", "llmgateway", vec![("base_url", upstream.uri()), ("api_key", "fixture".into())])], |config| {
        allow_replay_fixture(config);
        let auto = config.strategies[0].auto_config.as_mut().unwrap();
        auto.model_name = "my-router".into();
        auto.prioritized_models = vec![("gw".into(), "fast".into())];
        auto.routing_policy = Some(serde_json::from_value(json!({"enabled":true,"mode":"client_mode","mode_rules":[{"mode":"plan","route":"planning"}],"default_route":"general","options":[{"id":"planning","description":"Plan","models":[["gw","fast"]]},{"id":"general","description":"General","models":[["gw","general"]]}]})).unwrap());
    }, true).await;
    state
        .provider_registry
        .list_provider_models_cached("gw")
        .await
        .unwrap();
    let request = json!({"model":"my-router","metadata":{"localrouter.mode":"plan"},"messages":[{"role":"user","content":"Plan"}]});
    let original = state.config_manager.get().strategies[0].clone();
    let response = post(&base, "/v1/chat/completions", &secret, &request).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>().await.unwrap()["model"], "fast");
    state
        .config_manager
        .update(|config| {
            let profile = localrouter::config::RoutingProfile::new(
                "coding".into(),
                "Coding".into(),
                localrouter::config::RouteSnapshot::from(
                    config.strategies[0].auto_config.as_ref().unwrap(),
                ),
                1,
            )
            .unwrap();
            profile.apply(&mut config.strategies[0]).unwrap();
            config.routing_profiles.push(profile);
        })
        .unwrap();
    let listed: Value = reqwest::Client::new()
        .get(format!("{base}/v1/models"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["id"] == "my-router"));
    let mut explicit = request.clone();
    explicit["model"] = json!("gw/general");
    let response = post(&base, "/v1/chat/completions", &secret, &explicit).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>().await.unwrap()["model"], "general");
    state
        .config_manager
        .update(|config| {
            let p = &mut config.routing_profiles[0];
            let mut snapshot = p.active.clone();
            snapshot.routing_policy.as_mut().unwrap().options[0].models =
                vec![("gw".into(), "general".into())];
            p.versions.draft = Some(snapshot);
        })
        .unwrap();
    assert_eq!(
        post(&base, "/v1/chat/completions", &secret, &request)
            .await
            .json::<Value>()
            .await
            .unwrap()["model"],
        "fast"
    );
    state
        .config_manager
        .update(|config| {
            config.routing_profiles[0]
                .publish("General planner".into(), 2)
                .unwrap();
            config.project_routing_profile("coding").unwrap();
        })
        .unwrap();
    let body = post(&base, "/v1/chat/completions", &secret, &request)
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(body["model"], "general");
    assert_eq!(
        body["extensions"]["localrouter.routing"]["explanation"]["named_route"]["profile"]
            ["profile_id"],
        "coding"
    );
    assert_eq!(
        body["extensions"]["localrouter.routing"]["explanation"]["named_route"]["profile"]
            ["revision"],
        2
    );
    let current = state.config_manager.get().strategies[0].clone();
    assert_eq!(current.model_permissions, original.model_permissions);
    assert_eq!(current.rate_limits, original.rate_limits);
    assert_eq!(current.free_tier_only, original.free_tier_only);
    assert_eq!(current.free_tier_fallback, original.free_tier_fallback);
    assert_eq!(
        current.auto_config.as_ref().unwrap().model_name,
        "my-router"
    );
    let calls_before = upstream.received_requests().await.unwrap().len();
    state
        .config_manager
        .update(|config| {
            config.strategies[0].model_permissions.models.insert(
                "gw__general".into(),
                localrouter::config::PermissionState::Off,
            );
        })
        .unwrap();
    assert_eq!(
        post(&base, "/v1/chat/completions", &secret, &request)
            .await
            .json::<Value>()
            .await
            .unwrap()["model"],
        "fast"
    );
    let calls = upstream.received_requests().await.unwrap();
    assert!(calls[calls_before..]
        .iter()
        .filter(|r| r.method == "POST")
        .all(|r| serde_json::from_slice::<Value>(&r.body).unwrap()["model"] != "general"));
    state
        .config_manager
        .update(|config| {
            config.routing_profiles[0].rollback(1).unwrap();
            config.project_routing_profile("coding").unwrap();
        })
        .unwrap();
    let mut stream = request.clone();
    stream["stream"] = json!(true);
    let response = post(&base, "/v1/chat/completions", &secret, &stream).await;
    assert_eq!(response.status(), 200);
    let raw = response.text().await.unwrap();
    assert!(raw.contains("[DONE]"));
    assert!(raw.contains("coding"));
    state
        .config_manager
        .update(|config| config.strategies[0].routing_profile = None)
        .unwrap();
    assert_eq!(
        post(&base, "/v1/chat/completions", &secret, &request)
            .await
            .json::<Value>()
            .await
            .unwrap()["model"],
        "fast"
    );
}

#[tokio::test]
async fn profile_drafts_preserve_response_cache_and_published_revisions_invalidate_it() {
    let (_upstream, base, secret, state) = exact_cache_fixture(2, true).await;
    state
        .config_manager
        .update(|config| {
            let p = localrouter::config::RoutingProfile::new(
                "cache-profile".into(),
                "Cache".into(),
                localrouter::config::RouteSnapshot::from(
                    config.strategies[0].auto_config.as_ref().unwrap(),
                ),
                1,
            )
            .unwrap();
            p.apply(&mut config.strategies[0]).unwrap();
            config.routing_profiles.push(p);
        })
        .unwrap();
    async fn call(base: &str, secret: &str) {
        assert_eq!(
            post(base, "/v1/chat/completions", secret, &exact_cache_chat())
                .await
                .status(),
            200
        );
    }
    call(&base, &secret).await;
    state
        .config_manager
        .update(|config| {
            config.routing_profiles[0].versions.draft =
                Some(config.routing_profiles[0].active.clone())
        })
        .unwrap();
    call(&base, &secret).await;
    assert_eq!(state.router.response_cache_stats(None).hits, 1);
    state
        .config_manager
        .update(|config| {
            config.routing_profiles[0]
                .publish("same routing, new revision".into(), 2)
                .unwrap();
            config.project_routing_profile("cache-profile").unwrap();
        })
        .unwrap();
    call(&base, &secret).await;
    assert_eq!(state.router.response_cache_stats(None).misses, 2);
}
