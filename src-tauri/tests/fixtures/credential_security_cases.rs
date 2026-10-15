//! Expiry applies to both long-lived secrets and previously issued OAuth tokens.
use super::*;
#[tokio::test]
async fn expired_client_blocks_secret_files_and_existing_oauth_tokens() {
    let (base, secret, state) = start_replay_test_server(
        vec![],
        |config| {
            allow_replay_fixture(config);
            config.clients[0].mcp_mode = lr_config::McpMode::Gateway;
        },
        true,
    )
    .await;
    let http = reqwest::Client::new();
    assert_eq!(
        http.get(format!("{base}/v1/files"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let (token, _) = state
        .token_store
        .generate_token("test-api-key".into())
        .unwrap();
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"credential-fixture","version":"1"}}});
    let valid = http
        .post(format!("{base}/mcp"))
        .bearer_auth(&token)
        .header("accept", "application/json, text/event-stream")
        .json(&initialize)
        .send()
        .await
        .unwrap();
    assert_eq!(valid.status(), 200, "{}", valid.text().await.unwrap());
    state
        .client_manager
        .set_security(
            "test-api-key",
            lr_config::ClientSecurityConfig {
                credential_expires_at: Some(chrono::Utc::now() - chrono::Duration::seconds(1)),
                review_idle_after_days: None,
                ..Default::default()
            },
        )
        .unwrap();
    for path in ["/v1/files", "/v1/models", "/v1/batches"] {
        assert_eq!(
            http.get(format!("{base}{path}"))
                .bearer_auth(&secret)
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
    }
    assert!(state.token_store.verify_token(&token).is_some());
    let expired = http
        .post(format!("{base}/mcp"))
        .bearer_auth(&token)
        .header("accept", "application/json, text/event-stream")
        .json(&initialize)
        .send()
        .await
        .unwrap();
    assert_eq!(expired.status(), 401);
    assert_eq!(
        http.post(format!("{base}/oauth/token"))
            .json(&json!({"grant_type":"client_credentials","client_id":"test-api-key","client_secret":secret}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
}

#[tokio::test]
async fn privacy_unknown_provider_is_blocked_before_inference_and_request_cannot_relax_it() {
    let (upstream, base, secret, state) = exact_cache_fixture(0, true).await;
    state
        .config_manager
        .update(|config| {
            config.clients[0].security.privacy.local_only = true;
        })
        .unwrap();
    let mut body = exact_cache_chat();
    body["extensions"] = json!({"_localrouter_privacy":{"local_only":false}});
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/chat/completions"))
        .bearer_auth(&secret)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert!(
        !response.status().is_success(),
        "{}",
        response.text().await.unwrap()
    );
    for path in [
        "/v1/images/generations",
        "/v1/audio/speech",
        "/v1/moderations",
    ] {
        let response = reqwest::Client::new()
            .post(format!("{base}{path}"))
            .bearer_auth(&secret)
            .json(&json!({"model":"cached/cache-fixture","input":"sensitive"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 403, "{}", response.text().await.unwrap());
    }
    let engine = lr_guardrails::SafetyEngine::from_config(
        &[lr_guardrails::SafetyModelConfigInput {
            id: "safety".into(),
            model_type: "llama_guard_4".into(),
            provider_id: Some("stale".into()),
            model_name: Some("guard".into()),
            enabled_categories: None,
        }],
        0.5,
        &HashMap::from([(
            "stale".into(),
            lr_guardrails::ProviderInfo {
                name: "stale".into(),
                base_url: upstream.uri(),
                api_key: None,
                provider_type: "openai".into(),
            },
        )]),
    );
    assert!(engine.has_models());
    state.replace_safety_engine(Arc::new(engine));
    state
        .config_manager
        .update(|config| {
            config.guardrails.category_actions = vec![lr_config::CategoryActionEntry {
                category: "__global".into(),
                action: "block".into(),
            }];
        })
        .unwrap();
    for path in ["/v1/chat/completions", "/v1/systemone"] {
        let response = reqwest::Client::new()
            .post(format!("{base}{path}"))
            .bearer_auth(&secret)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 403, "{}", response.text().await.unwrap());
    }
    assert!(upstream
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method != "POST"));
}

#[tokio::test]
async fn no_retention_disables_exact_cache_capture_and_new_persistent_artifacts() {
    let (_upstream, base, secret, state) = exact_cache_fixture(2, true).await;
    let mut provider_config = state
        .provider_registry
        .get_provider_config("cached")
        .unwrap();
    let sealed = lr_providers::privacy::seal_attestation(
        "llmgateway",
        &provider_config,
        lr_providers::privacy::ProviderPrivacyAttestation {
            local_execution: false,
            no_retention: true,
            source: "Loopback fixture reviewed chain settings".into(),
            reviewed_at: chrono::Utc::now().timestamp(),
            expires_at: None,
        },
    )
    .unwrap();
    provider_config.insert(lr_providers::privacy::ATTESTATION_CONFIG.into(), sealed);
    state
        .provider_registry
        .update_provider("cached".into(), "llmgateway".into(), provider_config)
        .await
        .unwrap();
    state
        .config_manager
        .update(|config| {
            config.clients[0].security.privacy.no_retention = true;
        })
        .unwrap();
    let http = reqwest::Client::new();
    for _ in 0..2 {
        let response = http
            .post(format!("{base}/v1/chat/completions"))
            .bearer_auth(&secret)
            .json(&exact_cache_chat())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    }
    let events = state.monitor_store.list(0, 100, None);
    assert!(!events.events.is_empty());
    for event in events
        .events
        .iter()
        .filter(|event| event.client_id.as_deref() == Some("test-api-key"))
    {
        assert!(event.payload_redacted);
        let detail = state.monitor_store.get(&event.id).unwrap();
        let serialized = serde_json::to_string(&detail).unwrap();
        assert!(!serialized.contains("Cached answer"));
        assert!(!serialized.contains("\"hello\""));
    }
    let response = http
        .post(format!("{base}/v1/responses"))
        .bearer_auth(&secret)
        .json(&json!({"model":"cached/cache-fixture","input":"hello","store":true}))
        .send()
        .await
        .unwrap();
    assert!(!response.status().is_success());
    let response = http
        .post(format!("{base}/v1/files"))
        .bearer_auth(&secret)
        .multipart(
            reqwest::multipart::Form::new()
                .text("purpose", "batch")
                .part(
                    "file",
                    reqwest::multipart::Part::bytes(b"{}\n".to_vec()).file_name("input.jsonl"),
                ),
        )
        .send()
        .await
        .unwrap();
    assert!(!response.status().is_success());
    let response=http.post(format!("{base}/v1/batches")).bearer_auth(&secret).json(&json!({"input_file_id":"unowned","endpoint":"/v1/chat/completions","completion_window":"24h"})).send().await.unwrap();
    assert!(!response.status().is_success());
}
