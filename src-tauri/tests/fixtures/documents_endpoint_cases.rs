use super::*;
fn file() -> Value {
    json!({"filename":"source.pdf","file_data":"data:application/pdf;base64,JVBERi0xLjc="})
}
async fn gateway() -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"google/gemini-test"}]})),
        )
        .mount(&mock)
        .await;
    mock
}
fn input(endpoint: &str, stream: bool) -> Value {
    if endpoint.ends_with("responses") {
        json!({"model":"gw/google/gemini-test","store":false,"stream":stream,"input":[{"role":"user","content":[{"type":"input_file","filename":file()["filename"],"file_data":file()["file_data"]}]}]})
    } else {
        json!({"model":"gw/google/gemini-test","stream":stream,"messages":[{"role":"user","content":[{"type":"file","file":file()}]}]})
    }
}
#[tokio::test]
async fn native_document_bytes_and_provenance_survive_chat_responses_and_streams() {
    let mock = gateway().await;
    for stream in [false, true] {
        let response = if stream {
            let chunk = json!({"id":"doc-stream","object":"chat.completion.chunk","created":1,"model":"google/gemini-test","choices":[{"index":0,"delta":{"content":"document result"},"finish_reason":"stop"}],"usage":{"prompt_tokens":77,"completion_tokens":2,"total_tokens":79}});
            ResponseTemplate::new(200)
                .set_body_string(format!("data: {chunk}\n\ndata: [DONE]\n\n"))
                .insert_header("content-type", "text/event-stream")
        } else {
            ResponseTemplate::new(200).set_body_json(json!({"id":"doc","object":"chat.completion","created":1,"model":"google/gemini-test","choices":[{"index":0,"message":{"role":"assistant","content":"document result"},"finish_reason":"stop"}],"usage":{"prompt_tokens":77,"completion_tokens":2,"total_tokens":79}}))
        };
        Mock::given(method("POST")).and(path("/chat/completions")).and(header("x-no-fallback","true")).and(body_partial_json(json!({"stream":stream,"messages":[{"role":"user","content":[{"type":"file","file":file()}]}]}))).respond_with(response).expect(2).mount(&mock).await;
    }
    let (base, secret, _) = start_replay_test_server(
        vec![(
            "gw",
            "llmgateway",
            vec![("base_url", mock.uri()), ("api_key", "fixture".into())],
        )],
        allow_replay_fixture,
        true,
    )
    .await;
    for endpoint in ["/v1/chat/completions", "/v1/responses"] {
        for stream in [false, true] {
            let response = post(&base, endpoint, &secret, &input(endpoint, stream)).await;
            let status = response.status();
            let text = response.text().await.unwrap();
            assert_eq!(status, 200, "{text}");
            assert!(text.contains("document result"), "{text}");
            if !stream && endpoint.ends_with("completions") {
                let body: Value = serde_json::from_str(&text).unwrap();
                let metadata = &body["extensions"]["localrouter.document_processing"];
                assert_eq!(metadata["files"][0]["filename"], "source.pdf");
                assert_eq!(metadata["input_usage_reported"], true);
            }
        }
    }
    mock.verify().await;
}
#[tokio::test]
async fn unsupported_native_models_and_cost_limits_cannot_dispatch_or_classify_files() {
    let mock = gateway().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock)
        .await;
    let (base, secret, state) = start_replay_test_server(
        vec![(
            "gw",
            "llmgateway",
            vec![("base_url", mock.uri()), ("api_key", "fixture".into())],
        )],
        allow_replay_fixture,
        true,
    )
    .await;
    for model in ["localrouter/auto", "gw/text-only", "gw/anthropic/claude"] {
        let mut body = input("/v1/chat/completions", false);
        body["model"] = json!(model);
        assert_eq!(
            post(&base, "/v1/chat/completions", &secret, &body)
                .await
                .status(),
            400
        );
    }
    state
        .config_manager
        .update(|c| c.strategies[0].free_tier_only = true)
        .unwrap();
    assert_eq!(
        post(
            &base,
            "/v1/chat/completions",
            &secret,
            &input("/v1/chat/completions", false)
        )
        .await
        .status(),
        400
    );
    mock.verify().await;
}
#[tokio::test]
async fn openrouter_selects_native_pdf_engine_and_disables_fallbacks() {
    let mock = gateway().await;
    Mock::given(method("POST")).and(path("/chat/completions")).and(body_partial_json(json!({"plugins":[{"id":"file-parser","pdf":{"engine":"native"}}],"provider":{"allow_fallbacks":false},"messages":[{"role":"user","content":[{"type":"file","file":file()}]}]}))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"doc","created":1,"model":"google/gemini-test","choices":[{"index":0,"message":{"role":"assistant","content":"native result"},"finish_reason":"stop"}],"usage":{"prompt_tokens":77,"completion_tokens":2,"total_tokens":79}}))).expect(1).mount(&mock).await;
    let (base, secret, _) = start_replay_test_server(
        vec![(
            "gw",
            "document_openrouter_fixture",
            vec![("base_url", mock.uri()), ("api_key", "fixture".into())],
        )],
        allow_replay_fixture,
        true,
    )
    .await;
    let response = post(
        &base,
        "/v1/chat/completions",
        &secret,
        &input("/v1/chat/completions", false),
    )
    .await;
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    assert!(text.contains("native result"));
    mock.verify().await;
}

#[tokio::test]
async fn failed_native_document_dispatch_is_not_automatically_retryable() {
    let mock = gateway().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&mock)
        .await;
    let (base, secret, _) = start_replay_test_server(
        vec![(
            "gw",
            "llmgateway",
            vec![("base_url", mock.uri()), ("api_key", "fixture".into())],
        )],
        allow_replay_fixture,
        true,
    )
    .await;
    let response = post(
        &base,
        "/v1/chat/completions",
        &secret,
        &input("/v1/chat/completions", false),
    )
    .await;
    assert_eq!(response.status(), 502);
    assert_eq!(response.headers()["x-should-retry"], "false");
    mock.verify().await;
}
#[tokio::test]
async fn native_document_replay_is_rejected_without_dispatching_the_file_again() {
    let mock = gateway().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock)
        .await;
    let (base, secret, _) = start_replay_test_server(
        vec![(
            "gw",
            "llmgateway",
            vec![("base_url", mock.uri()), ("api_key", "fixture".into())],
        )],
        allow_replay_fixture,
        true,
    )
    .await;
    let mut body = input("/v1/chat/completions", false);
    body["extensions"] = json!({"localrouter.replay":{"tools":"disabled"}});
    let response = post(&base, "/v1/chat/completions", &secret, &body).await;
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 400, "{text}");
    assert!(text.contains("replay"), "{text}");
    mock.verify().await;
}

pub(super) struct TestDocumentGatewayFactory;
impl localrouter::providers::factory::ProviderFactory for TestDocumentGatewayFactory {
    fn provider_type(&self) -> &str {
        "document_openrouter_fixture"
    }
    fn display_name(&self) -> &str {
        "Document gateway fixture"
    }
    fn category(&self) -> localrouter::providers::factory::ProviderCategory {
        localrouter::providers::factory::ProviderCategory::ThirdParty
    }
    fn description(&self) -> &str {
        "Loopback OpenRouter document fixture"
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
            localrouter::providers::openrouter::OpenRouterProvider::with_base_url(
                config["api_key"].clone(),
                config["base_url"].clone(),
            ),
        ))
    }
}

#[tokio::test]
async fn native_document_stream_without_input_usage_fails_instead_of_claiming_completion() {
    let mock = gateway().await;
    let chunk = json!({"id":"doc-stream","object":"chat.completion.chunk","created":1,"model":"google/gemini-test","choices":[{"index":0,"delta":{"content":"partial result"},"finish_reason":"stop"}]});
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(format!("data: {chunk}\n\ndata: [DONE]\n\n"))
                .insert_header("content-type", "text/event-stream"),
        )
        .expect(1)
        .mount(&mock)
        .await;
    let (base, secret, _) = start_replay_test_server(
        vec![(
            "gw",
            "llmgateway",
            vec![("base_url", mock.uri()), ("api_key", "fixture".into())],
        )],
        allow_replay_fixture,
        true,
    )
    .await;
    let response = post(
        &base,
        "/v1/responses",
        &secret,
        &input("/v1/responses", true),
    )
    .await;
    let text = response.text().await.unwrap();
    assert!(text.contains("response.failed"), "{text}");
    assert!(text.contains("reported input usage"), "{text}");
    assert!(!text.contains("response.completed"), "{text}");
    mock.verify().await;
}
