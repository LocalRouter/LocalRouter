use super::*;
use crate::ModelProvider;
use serde_json::json;
use wiremock::{
    matchers::{body_json, header, method, path},
    Mock, MockServer, ResponseTemplate,
};

fn object(id: &str, status: &str) -> Value {
    json!({"id":id,"object":"batch","status":status,"endpoint":"/v1/chat/completions","completion_window":"24h","created_at":100,"request_counts":{"total":2,"completed":0,"failed":0}})
}
fn inline_rows() -> Vec<BatchInlineRow> {
    ["one", "two"]
        .into_iter()
        .map(|id| BatchInlineRow {
            custom_id: id.into(),
            body: json!({"messages":[{"role":"user","content":"hello"}]}),
        })
        .collect()
}
fn router(mock: &MockServer, preferences: GatewayPreferences) -> NativeBatchTransport {
    NativeBatchTransport::openrouter(
        &format!("{}/api/v1", mock.uri()),
        "configured-gateway-key",
        preferences,
        Some("LocalRouter"),
        Some("https://localrouter.ai"),
    )
    .unwrap()
}
#[tokio::test]
async fn openai_upload_create_poll_download_cancel_and_owned_file_cleanup() {
    let upstream = MockServer::start().await;
    let transport =
        NativeBatchTransport::openai(&format!("{}/v1", upstream.uri()), "configured-provider-key")
            .unwrap();
    let content = b"{\"custom_id\":\"one\"}\n".to_vec();
    Mock::given(method("POST")).and(path("/v1/files")).and(header("authorization","Bearer configured-provider-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"file_private","object":"file","bytes":content.len(),"created_at":100,"filename":"batch-private.jsonl","purpose":"batch","expires_at":2592100}))).expect(1).mount(&upstream).await;
    let file = transport
        .upload_input(content, "batch-private.jsonl")
        .await
        .unwrap();
    assert_eq!(file.id, "file_private");
    let requests = upstream.received_requests().await.unwrap();
    let multipart = String::from_utf8_lossy(&requests[0].body);
    assert!(multipart.contains("name=\"purpose\"\r\n\r\nbatch"));
    assert!(multipart.contains("filename=\"batch-private.jsonl\""));
    let metadata = HashMap::from([("localrouter_job_id".into(), "owned-nonce".into())]);
    let mut accepted = object("batch_private", "validating");
    accepted["input_file_id"] = json!("file_private");
    accepted["metadata"] = json!(metadata);
    Mock::given(method("POST")).and(path("/v1/batches")).and(body_json(json!({"input_file_id":"file_private","endpoint":"/v1/chat/completions","completion_window":"24h","metadata":metadata,"output_expires_after":{"anchor":"created_at","seconds":3600}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(accepted)).expect(1).mount(&upstream).await;
    let batch = transport
        .submit_file(
            &file.id,
            "/v1/chat/completions",
            &metadata,
            Some(BatchOutputExpiry {
                anchor: "created_at".into(),
                seconds: 3600,
            }),
        )
        .await
        .unwrap();
    assert_eq!(batch.id, "batch_private");
    let mut completed = object("batch_private", "completed");
    completed["input_file_id"] = json!("file_private");
    completed["output_file_id"] = json!("file_output");
    completed["error_file_id"] = json!("file_error");
    completed["request_counts"] = json!({"total":2,"completed":1,"failed":1});
    Mock::given(method("GET"))
        .and(path("/v1/batches/batch_private"))
        .respond_with(ResponseTemplate::new(200).set_body_json(completed))
        .expect(1)
        .mount(&upstream)
        .await;
    assert_eq!(
        transport
            .retrieve("batch_private")
            .await
            .unwrap()
            .error_file_id
            .as_deref(),
        Some("file_error")
    );
    let output = "{\"custom_id\":\"two\",\"error\":{\"code\":\"fixture_error\"}}\n";
    Mock::given(method("GET"))
        .and(path("/v1/files/file_error/content"))
        .respond_with(ResponseTemplate::new(200).set_body_string(output))
        .expect(1)
        .mount(&upstream)
        .await;
    assert_eq!(
        transport.download_file("file_error").await.unwrap(),
        output.as_bytes()
    );
    Mock::given(method("POST"))
        .and(path("/v1/batches/batch_private/cancel"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(object("batch_private", "cancelling")),
        )
        .expect(1)
        .mount(&upstream)
        .await;
    assert_eq!(
        transport.cancel("batch_private").await.unwrap().status,
        NativeBatchStatus::Cancelling
    );
    Mock::given(method("DELETE"))
        .and(path("/v1/files/file_private"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"file_private","object":"file","deleted":true})),
        )
        .expect(1)
        .mount(&upstream)
        .await;
    transport.delete_file("file_private").await.unwrap();
}

#[tokio::test]
async fn openrouter_inline_order_pin_attribution_and_aggregate_usage_are_preserved() {
    let upstream = MockServer::start().await;
    let saved = GatewayPreferences {
        only: Some(vec!["openai".into(), "anthropic".into()]),
        allow_fallbacks: Some(false),
        ..Default::default()
    };
    let transport = router(&upstream, saved);
    let requested = GatewayPreferences {
        only: Some(vec!["openai".into()]),
        ..Default::default()
    };
    let mut accepted = object("batch_inline", "validating");
    accepted["model"] = json!("openai/model");
    Mock::given(method("POST")).and(path("/api/v1/batches")).and(header("authorization","Bearer configured-gateway-key")).and(header("x-title","LocalRouter")).and(header("http-referer","https://localrouter.ai"))
        .and(body_json(json!({"endpoint":"/v1/chat/completions","model":"openai/model","completion_window":"24h","provider":{"only":["openai"]},"requests":[{"custom_id":"one","body":{"messages":[{"role":"user","content":"hello"}]}},{"custom_id":"two","body":{"messages":[{"role":"user","content":"hello"}]}}]})))
        .respond_with(ResponseTemplate::new(202).set_body_json(accepted)).expect(1).mount(&upstream).await;
    let batch = transport
        .submit_inline(
            "/v1/chat/completions",
            "openai/model",
            &inline_rows(),
            &requested,
        )
        .await
        .unwrap();
    assert_eq!(batch.id, "batch_inline");
    let requests = upstream.received_requests().await.unwrap();
    let body = std::str::from_utf8(&requests[0].body).unwrap();
    let rows = body.find("\"requests\"").unwrap();
    for field in ["endpoint", "model", "completion_window", "provider"] {
        assert!(body.find(&format!("\"{field}\"")).unwrap() < rows);
    }
    let mut completed = object("batch_inline", "completed");
    completed["model"] = json!("openai/model");
    completed["request_counts"] = json!({"total":2,"completed":2,"failed":0});
    completed["usage"] = json!({"prompt_tokens":10,"completion_tokens":4,"cost":0.001,"is_byok":true,"cost_details":{"upstream_inference_cost":0.02}});
    completed["results"] = json!([{"custom_id":"two","response":{"status_code":200,"body":{"id":"gen-two"}},"error":null},{"custom_id":"one","response":{"status_code":200,"body":{"id":"gen-one"}},"error":null}]);
    Mock::given(method("GET"))
        .and(path("/api/v1/batches/batch_inline"))
        .respond_with(ResponseTemplate::new(200).set_body_json(completed.clone()))
        .expect(1)
        .mount(&upstream)
        .await;
    let result = transport.retrieve("batch_inline").await.unwrap();
    assert_eq!(result.usage, Some(completed["usage"].clone()));
    assert_eq!(result.results.as_ref().unwrap()[0]["custom_id"], "two");
}

#[tokio::test]
async fn unsupported_gateway_privacy_and_operation_controls_fail_before_io() {
    let upstream = MockServer::start().await;
    let strict = router(
        &upstream,
        GatewayPreferences {
            zdr: Some(true),
            ..Default::default()
        },
    );
    assert!(strict
        .submit_inline(
            "/v1/chat/completions",
            "model",
            &inline_rows(),
            &GatewayPreferences {
                zdr: Some(false),
                ..Default::default()
            }
        )
        .await
        .is_err());
    let transport = router(&upstream, GatewayPreferences::default());
    assert!(transport.cancel("batch_owned").await.is_err());
    assert!(transport.download_file("file_owned").await.is_err());
    assert!(transport.reconciliation_page(None).await.is_err());
    assert!(transport.upload_reconciliation_page(None).await.is_err());
    assert!(transport
        .delete_terminal_batch("batch_owned", NativeBatchStatus::InProgress)
        .await
        .is_err());
    let mut rows = inline_rows();
    rows[0].body["provider"] = json!({"only":["unapproved"]});
    assert!(transport
        .submit_inline(
            "/v1/chat/completions",
            "model",
            &rows,
            &GatewayPreferences::default()
        )
        .await
        .is_err());
    assert!(upstream.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn terminal_cleanup_retains_provider_outcome_and_retries_only_explicitly() {
    let upstream = MockServer::start().await;
    let transport = router(&upstream, GatewayPreferences::default());
    Mock::given(method("DELETE"))
        .and(path("/api/v1/batches/batch_owned"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("retry-after", "7")
                .set_body_json(json!({"error":"private-provider-credential must not escape"})),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&upstream)
        .await;
    let error = transport
        .delete_terminal_batch("batch_owned", NativeBatchStatus::Completed)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AppError::ProviderUnavailable {
            status: 503,
            retry_after_secs: Some(7),
            ..
        }
    ));
    assert!(!error.to_string().contains("private-provider-credential"));
    assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
    Mock::given(method("DELETE")).and(path("/api/v1/batches/batch_owned"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"batch_owned","object":"batch","deletion":{"openrouter":"deleted","upstream":{"provider":"OpenAI","status":"unsupported"}}}))).with_priority(1).expect(1).mount(&upstream).await;
    let result = transport
        .delete_terminal_batch("batch_owned", NativeBatchStatus::Completed)
        .await
        .unwrap();
    assert_eq!(result.deletion.openrouter, "deleted");
    assert_eq!(result.deletion.upstream.unwrap().status, "unsupported");
    assert_eq!(upstream.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn redirects_do_not_receive_credentials_or_replay_mutations() {
    let upstream = MockServer::start().await;
    let redirect = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/batches"))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("location", format!("{}/credential-leak", redirect.uri())),
        )
        .expect(1)
        .mount(&upstream)
        .await;
    let error = router(&upstream, GatewayPreferences::default())
        .submit_inline(
            "/v1/chat/completions",
            "model",
            &inline_rows(),
            &GatewayPreferences::default(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("redirect refused"));
    assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
    assert!(redirect.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn invalid_paths_privacy_and_schema_fail_without_credential_disclosure() {
    let upstream = MockServer::start().await;
    let transport = router(&upstream, GatewayPreferences::default());
    for id in [
        "../another",
        "id?token=private",
        "id/other",
        "https://example.com",
    ] {
        assert!(transport.retrieve(id).await.is_err());
    }
    assert!(upstream.received_requests().await.unwrap().is_empty());
    let mut wrong = object("batch_other", "completed");
    wrong["model"] = json!("model");
    wrong["results"] = json!([]);
    Mock::given(method("GET"))
        .and(path("/api/v1/batches/batch_owned"))
        .respond_with(ResponseTemplate::new(200).set_body_json(wrong))
        .expect(1)
        .mount(&upstream)
        .await;
    assert!(matches!(
        transport.retrieve("batch_owned").await,
        Err(AppError::Provider(_))
    ));
    Mock::given(method("GET"))
        .and(path("/api/v1/batches/batch_no_results"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(object("batch_no_results", "completed")),
        )
        .expect(1)
        .mount(&upstream)
        .await;
    assert!(matches!(
        transport.retrieve("batch_no_results").await,
        Err(AppError::Provider(_))
    ));
    Mock::given(method("GET"))
        .and(path("/api/v1/batches/batch_denied"))
        .respond_with(ResponseTemplate::new(403).set_body_json(
            json!({"error":{"message":"configured-gateway-key echoed private prompt"}}),
        ))
        .expect(1)
        .mount(&upstream)
        .await;
    let error = transport.retrieve("batch_denied").await.unwrap_err();
    assert!(matches!(
        error,
        AppError::ProviderStatus { status: 403, .. }
    ));
    assert!(!error.to_string().contains("configured-gateway-key"));
    assert!(!error.to_string().contains("private prompt"));
}

#[tokio::test]
async fn reconciliation_is_read_only_paged_and_does_not_assume_foreign_endpoint_support() {
    let upstream = MockServer::start().await;
    let transport =
        NativeBatchTransport::openai(&format!("{}/v1", upstream.uri()), "provider-key").unwrap();
    Mock::given(method("GET")).and(path("/v1/batches")).and(wiremock::matchers::query_param("limit","100")).and(wiremock::matchers::query_param("after","batch_cursor"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"object":"list","data":[{"id":"batch_foreign","endpoint":"/v1/videos","metadata":{"different_owner":"do-not-expose"}}],"has_more":false,"first_id":"batch_foreign","last_id":"batch_foreign"}))).expect(1).mount(&upstream).await;
    let page = transport
        .reconciliation_page(Some("batch_cursor"))
        .await
        .unwrap();
    assert_eq!(page.data[0]["endpoint"], "/v1/videos");
    Mock::given(method("GET"))
        .and(path("/v1/files"))
        .and(wiremock::matchers::query_param("purpose", "batch"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":[],"has_more":false,"last_id":null,"first_id":null})),
        )
        .expect(1)
        .mount(&upstream)
        .await;
    assert!(transport
        .upload_reconciliation_page(None)
        .await
        .unwrap()
        .data
        .is_empty());
    assert!(upstream
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method.as_str() == "GET"));
}

#[tokio::test]
async fn bounded_body_helpers_and_malformed_counts_are_rejected() {
    assert!(bounded_json(&json!({"large":"a".repeat(32)}), 16).is_err());
    let upstream = MockServer::start().await;
    let transport =
        NativeBatchTransport::openai(&format!("{}/v1", upstream.uri()), "provider-key").unwrap();
    Mock::given(method("GET"))
        .and(path("/v1/small"))
        .respond_with(ResponseTemplate::new(200).set_body_string("123456789"))
        .expect(1)
        .mount(&upstream)
        .await;
    assert!(
        read_response(transport.request(Method::GET, &["small"]).unwrap(), 8)
            .await
            .is_err()
    );
    let mut wrong = object("batch_owned", "in_progress");
    wrong["request_counts"] = json!({"total":2,"completed":3,"failed":0});
    Mock::given(method("GET"))
        .and(path("/v1/batches/batch_owned"))
        .respond_with(ResponseTemplate::new(200).set_body_json(wrong))
        .expect(1)
        .mount(&upstream)
        .await;
    assert!(transport.retrieve("batch_owned").await.is_err());
}

#[test]
fn binding_identity_is_durable_normalized_and_sensitive_to_origin_key_and_salt() {
    let first = NativeBatchTransport::openai("https://api.example/v1", "private-test-key").unwrap();
    let same = NativeBatchTransport::openai("https://api.example/v1/", "private-test-key").unwrap();
    let salt = "a".repeat(64);
    let binding = first.binding_fingerprint(&salt).unwrap();
    assert_eq!(binding.len(), 64);
    assert_eq!(same.binding_fingerprint(&salt).unwrap(), binding);
    for other in [
        NativeBatchTransport::openai("https://other.example/v1", "private-test-key").unwrap(),
        NativeBatchTransport::openai("https://api.example/v1", "rotated-key").unwrap(),
    ] {
        assert_ne!(other.binding_fingerprint(&salt).unwrap(), binding);
    }
    assert_ne!(first.binding_fingerprint(&"b".repeat(64)).unwrap(), binding);
    assert!(!format!("{first:?}").contains("private-test-key"));
    assert!(!format!("{first:?}").contains("api.example"));
    for root in [
        "file:///tmp/provider",
        "https://user:private@example.com/v1",
        "https://example.com/v1?key=private",
        "https://example.com/v1#fragment",
    ] {
        assert!(NativeBatchTransport::openai(root, "key").is_err());
    }
}

#[tokio::test]
async fn direct_and_generic_provider_capabilities_do_not_imply_protocol_parity() {
    let public = crate::openai::OpenAIProvider::new("key".into());
    assert_eq!(public.native_batch_kind(), Some(NativeBatchKind::Openai));
    assert!(public
        .native_batch_transport()
        .unwrap()
        .unwrap()
        .supports_cancel());
    let subscription = crate::openai::OpenAIProvider::with_base_url(
        "subscription-token".into(),
        "https://chatgpt.com/backend-api/codex".into(),
    )
    .unwrap();
    assert_eq!(subscription.native_batch_kind(), None);
    assert!(subscription.native_batch_transport().unwrap().is_none());
    let generic = crate::openai_compatible::OpenAICompatibleProvider::new(
        "generic".into(),
        "http://127.0.0.1:1/v1".into(),
        Some("key".into()),
    );
    assert_eq!(generic.native_batch_kind(), None);
    assert!(generic.native_batch_transport().unwrap().is_none());
}

#[tokio::test]
async fn openrouter_unavailable_partial_results_and_unknown_spend_stay_unavailable() {
    let upstream = MockServer::start().await;
    let transport = router(&upstream, GatewayPreferences::default());
    for status in ["failed", "expired", "cancelled"] {
        let id = format!("batch_{status}");
        let mut result = object(&id, status);
        result["model"] = json!("model");
        result["results"] = Value::Null;
        result["usage"] =
            json!({"prompt_tokens":12,"completion_tokens":3,"cost":null,"is_byok":true});
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/batches/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(result.clone()))
            .expect(1)
            .mount(&upstream)
            .await;
        let batch = transport.retrieve(&id).await.unwrap();
        assert!(batch.status.is_terminal());
        assert!(batch.results.is_none());
        assert_eq!(batch.usage, Some(result["usage"].clone()));
    }
}
#[tokio::test]
async fn metadata_expiry_and_pin_widening_are_rejected_before_submission() {
    let upstream = MockServer::start().await;
    let direct = NativeBatchTransport::openai(&format!("{}/v1", upstream.uri()), "key").unwrap();
    let mut metadata = HashMap::new();
    for index in 0..17 {
        metadata.insert(format!("key{index}"), "value".into());
    }
    assert!(direct
        .submit_file("file_owned", "/v1/chat/completions", &metadata, None)
        .await
        .is_err());
    assert!(direct
        .submit_file(
            "file_owned",
            "/v1/chat/completions",
            &HashMap::new(),
            Some(BatchOutputExpiry {
                anchor: "created_at".into(),
                seconds: 3599
            })
        )
        .await
        .is_err());
    let gateway = router(
        &upstream,
        GatewayPreferences {
            only: Some(vec!["openai".into()]),
            ..Default::default()
        },
    );
    assert!(gateway
        .submit_inline(
            "/v1/chat/completions",
            "model",
            &inline_rows(),
            &GatewayPreferences {
                only: Some(vec!["anthropic".into()]),
                ..Default::default()
            }
        )
        .await
        .is_err());
    assert!(upstream.received_requests().await.unwrap().is_empty());
}
