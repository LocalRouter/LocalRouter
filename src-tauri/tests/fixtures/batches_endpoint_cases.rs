//! Native batch SDK integration uses real policy and loopback native transport.
use super::*;
pub(super) struct TestNativeBatchFactory(pub bool);
impl localrouter::providers::factory::ProviderFactory for TestNativeBatchFactory {
    fn provider_type(&self) -> &str {
        if self.0 {
            "native_router_batch_fixture"
        } else {
            "native_batch_fixture"
        }
    }
    fn display_name(&self) -> &str {
        "Native batch fixture"
    }
    fn category(&self) -> localrouter::providers::factory::ProviderCategory {
        localrouter::providers::factory::ProviderCategory::FirstParty
    }
    fn description(&self) -> &str {
        "Loopback native batch"
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
        if self.0 {
            return Ok(Arc::new(
                localrouter::providers::openrouter::OpenRouterProvider::with_base_url(
                    config["api_key"].clone(),
                    config["base_url"].clone(),
                ),
            ));
        }
        Ok(Arc::new(
            localrouter::providers::openai::OpenAIProvider::with_base_url(
                config["api_key"].clone(),
                config["base_url"].clone(),
            )?,
        ))
    }
}
async fn native_mock() -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/models")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"object":"list","data":[{"id":"gpt-sdk-native-batch-fixture-unique","object":"model","created":1,"owned_by":"test"}]}))).mount(&mock).await;
    mock
}
fn provider(mock: &MockServer) -> Upstream {
    (
        "nativebatch",
        "native_batch_fixture",
        vec![
            ("api_key", "native-batch-configured-key".into()),
            ("base_url", format!("{}/v1", mock.uri())),
        ],
    )
}
async fn upload(base: &str, secret: &str) -> String {
    let row=json!({"custom_id":"row-one","method":"POST","url":"/v1/chat/completions","body":{"model":"nativebatch/gpt-sdk-native-batch-fixture-unique","max_tokens":16,"messages":[{"role":"user","content":"hello"}]}}).to_string()+"\n";
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/files"))
        .bearer_auth(secret)
        .multipart(
            reqwest::multipart::Form::new()
                .text("purpose", "batch")
                .part(
                    "file",
                    reqwest::multipart::Part::bytes(row.into_bytes()).file_name("input.jsonl"),
                ),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}
async fn get(base: &str, secret: &str, id: &str) -> Value {
    let response = reqwest::Client::new()
        .get(format!("{base}/v1/batches/{id}"))
        .bearer_auth(secret)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.json().await.unwrap()
}
async fn idle(base: &str, secret: &str, id: &str) -> Value {
    for _ in 0..200 {
        let job = get(base, secret, id).await;
        if job["localrouter"]["worker_running"] == false {
            return job;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("native batch worker did not stop")
}
fn create_body(file: &str, prepare: bool) -> Value {
    json!({"input_file_id":file,"endpoint":"/v1/chat/completions","completion_window":"24h","extensions":{"localrouter.batch":{"prepare_only":prepare}}})
}
fn native(status: &str) -> Value {
    json!({"id":"batch_native_owned","object":"batch","status":status,"endpoint":"/v1/chat/completions","completion_window":"24h","created_at":1,"input_file_id":"file_native_owned","request_counts":{"total":1,"completed":if status=="completed"{1}else{0},"failed":0},"output_file_id":if status=="completed"{json!("file_native_output")}else{Value::Null}})
}
async fn mount_upload(mock: &MockServer) {
    Mock::given(method("POST")).and(path("/v1/files")).and(header("authorization","Bearer native-batch-configured-key")).respond_with(|request:&wiremock::Request|{
        let text=String::from_utf8(request.body.clone()).unwrap();
        let filename=text.split("filename=\"").nth(1).unwrap().split('"').next().unwrap();
        let file_part=text.split("filename=\"").nth(1).unwrap().split("\r\n\r\n").nth(1).unwrap();
        let size=file_part.split("\r\n--").next().unwrap().len();
        ResponseTemplate::new(200).set_body_json(json!({"id":"file_native_owned","object":"file","purpose":"batch","filename":filename,"bytes":size,"created_at":1}))
    }).expect(1).mount(mock).await;
}
#[tokio::test]
async fn native_batch_prepare_submit_results_are_owned_idempotent_and_get_is_local() {
    let mock = native_mock().await;
    mount_upload(&mock).await;
    Mock::given(method("POST"))
        .and(path("/v1/batches"))
        .respond_with(ResponseTemplate::new(200).set_body_json(native("in_progress")))
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/batches/batch_native_owned"))
        .respond_with(ResponseTemplate::new(200).set_body_json(native("completed")))
        .expect(2)
        .mount(&mock)
        .await;
    let output=json!({"id":"row_result","custom_id":"row-one","response":{"status_code":200,"request_id":"request-one","body":{"choices":[{"message":{"content":"hello back"}}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}},"error":null}).to_string()+"\n";
    Mock::given(method("GET"))
        .and(path("/v1/files/file_native_output/content"))
        .respond_with(ResponseTemplate::new(200).set_body_string(output.clone()))
        .expect(2)
        .mount(&mock)
        .await;
    let (base, secret, state) =
        start_replay_test_server(vec![provider(&mock)], allow_replay_fixture, true).await;
    let file = upload(&base, &secret).await;
    let request = create_body(&file, true);
    let http = reqwest::Client::new();
    let response = http
        .post(format!("{base}/v1/batches"))
        .bearer_auth(&secret)
        .header("Idempotency-Key", "sdk-one")
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    let list: Value = http
        .get(format!("{base}/v1/batches"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = list["data"][0]["id"].as_str().unwrap();
    let prepared = idle(&base, &secret, id).await;
    assert_eq!(prepared["localrouter"]["phase"], "prepared", "{prepared}");
    assert_eq!(prepared["localrouter"]["validated_rows"], 1);
    assert_eq!(
        prepared["localrouter"]["reserved_usage"]["cost_usd"],
        Value::Null
    );
    assert!(mock
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method.as_str() == "GET"));
    let snapshot = prepared["localrouter"]["prepared_snapshot_file_id"]
        .as_str()
        .unwrap();
    let snapshot: Value =
        serde_json::from_slice(&state.batch_store.content("test-api-key", snapshot).unwrap())
            .unwrap();
    assert_eq!(
        snapshot["body"]["model"],
        "gpt-sdk-native-batch-fixture-unique"
    );
    assert_eq!(snapshot["body"]["store"], false);
    let response = http
        .post(format!("{base}/v1/batches/{id}/submit"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let submitted = idle(&base, &secret, id).await;
    assert_eq!(
        submitted["localrouter"]["phase"], "submitted",
        "{submitted}"
    );
    for _ in 0..2 {
        let response = http
            .post(format!("{base}/v1/batches/{id}/refresh"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    }
    let complete = get(&base, &secret, id).await;
    assert_eq!(complete["status"], "completed");
    assert_eq!(complete["localrouter"]["accounting_settled"], true);
    assert_eq!(
        complete["localrouter"]["accounting"]["billing"]["actual_input_tokens"],
        3
    );
    assert_eq!(
        complete["localrouter"]["accounting"]["billing"]["quota_usage"]["cost_usd"],
        Value::Null
    );
    let output_id = complete["output_file_id"].as_str().unwrap();
    assert!(output_id.starts_with("file-lr-"));
    assert_eq!(
        state
            .batch_store
            .content("test-api-key", output_id)
            .unwrap(),
        output.as_bytes()
    );
    let other = state
        .client_manager
        .add_client_with_secret(create_test_client("batches-other", "default"))
        .unwrap();
    for url in [
        format!("{base}/v1/batches/{id}"),
        format!("{base}/v1/files/{output_id}/content"),
    ] {
        assert_eq!(
            http.get(url)
                .bearer_auth(&other)
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    state.batch_store.delete("test-api-key", &file).unwrap();
    let rotated = state.client_manager.rotate_secret("test-api-key").unwrap();
    let retry = http
        .post(format!("{base}/v1/batches"))
        .bearer_auth(&rotated)
        .header("Idempotency-Key", "sdk-one")
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(retry.status(), 200);
    assert_eq!(retry.json::<Value>().await.unwrap()["id"], id);
    let mut changed = request;
    changed["metadata"] = json!({"different":"request"});
    assert_eq!(
        http.post(format!("{base}/v1/batches"))
            .bearer_auth(&rotated)
            .header("Idempotency-Key", "sdk-one")
            .json(&changed)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    let spec: Value = http
        .get(format!("{base}/openapi.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(spec["paths"]["/v1/batches/{batch_id}/refresh"]["post"].is_object());
}
#[tokio::test]
async fn native_batch_uncertain_submission_is_never_reissued_and_rejects_internal_credentials() {
    let mock = native_mock().await;
    mount_upload(&mock).await;
    Mock::given(method("POST"))
        .and(path("/v1/batches"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&mock)
        .await;
    let (base, secret, state) =
        start_replay_test_server(vec![provider(&mock)], allow_replay_fixture, true).await;
    let file = upload(&base, &secret).await;
    let request = create_body(&file, false);
    let http = reqwest::Client::new();
    let response = http
        .post(format!("{base}/v1/batches"))
        .bearer_auth(&secret)
        .header("idempotency-key", "uncertain")
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let id = response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let job = idle(&base, &secret, &id).await;
    assert_eq!(
        job["localrouter"]["phase"], "submit_pending_outcome",
        "{job}"
    );
    for action in ["resume", "submit", "refresh", "cancel"] {
        assert_eq!(
            http.post(format!("{base}/v1/batches/{id}/{action}"))
                .bearer_auth(&secret)
                .send()
                .await
                .unwrap()
                .status(),
            409
        );
    }
    assert!(state
        .batch_store
        .job_reservation("test-api-key", &id)
        .unwrap()
        .is_some());
    let retry = http
        .post(format!("{base}/v1/batches"))
        .bearer_auth(&secret)
        .header("idempotency-key", "uncertain")
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(retry.status(), 200);
    assert_eq!(retry.json::<Value>().await.unwrap()["id"], id);
    let (internal, token, _) =
        start_replay_test_server(vec![provider(&mock)], allow_replay_fixture, false).await;
    assert_eq!(
        http.post(format!("{internal}/v1/batches"))
            .bearer_auth(&token)
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
}

#[tokio::test]
async fn openrouter_native_inline_results_keep_byok_billing_scope_and_private_remote_identity() {
    let mock = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api/v1/models")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"vendor/fixture-batch-model:batch","name":"Fixture Batch","context_length":4096}]}))).mount(&mock).await;
    let native = json!({"id":"batch_router_owned","object":"batch","model":"vendor/fixture-batch-model","status":"completed","endpoint":"/v1/chat/completions","completion_window":"24h","created_at":1,"request_counts":{"total":1,"completed":1,"failed":0},"usage":{"input_tokens":3,"output_tokens":2,"cost":0.001,"is_byok":true,"cost_details":{"upstream_inference_cost":0.02}},"results":[{"custom_id":"row-one","response":{"status_code":200,"body":{"usage":{"prompt_tokens":3,"completion_tokens":2}}},"error":null}]});
    Mock::given(method("POST"))
        .and(path("/api/v1/batches"))
        .respond_with(ResponseTemplate::new(202).set_body_json(native))
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("DELETE")).and(path("/api/v1/batches/batch_router_owned")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"batch_router_owned","object":"batch","deletion":{"openrouter":"deleted","upstream":{"provider":"Fixture Provider","status":"unsupported"}}}))).expect(1).mount(&mock).await;
    let (base, secret, state) = start_replay_test_server(
        vec![(
            "nativebatch",
            "native_router_batch_fixture",
            vec![
                ("api_key", "configured-router-key".into()),
                ("base_url", format!("{}/api/v1", mock.uri())),
            ],
        )],
        allow_replay_fixture,
        true,
    )
    .await;
    let content=json!({"custom_id":"row-one","method":"POST","url":"/v1/chat/completions","body":{"model":"nativebatch/vendor/fixture-batch-model","max_tokens":16,"messages":[{"role":"user","content":"hello"}]}}).to_string()+"\n";
    let file = state
        .batch_store
        .upload(
            "test-api-key",
            "input.jsonl",
            "batch",
            content.as_bytes(),
            3600,
        )
        .unwrap();
    let http = reqwest::Client::new();
    let request = create_body(&file.id, false);
    let response = http
        .post(format!("{base}/v1/batches"))
        .bearer_auth(&secret)
        .json(&request)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let value: Value = response.json().await.unwrap();
    assert_eq!(status, 200, "{value}");
    let id = value["id"].as_str().unwrap();
    let completed = idle(&base, &secret, id).await;
    assert_eq!(completed["status"], "completed", "{completed}");
    assert!(completed["localrouter"]["accounting_settled"]
        .as_bool()
        .unwrap());
    let billing = &completed["localrouter"]["accounting"]["billing"];
    assert_eq!(billing["reported_charge_usd"], 0.001);
    assert_eq!(billing["upstream_inference_estimate_usd"], 0.02);
    assert_eq!(billing["quota_usage"]["cost_usd"], 0.021);
    let request = mock
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.method.as_str() == "POST")
        .unwrap();
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["model"], "vendor/fixture-batch-model");
    assert_eq!(body["requests"][0]["body"]["store"], false);
    let response = http
        .post(format!("{base}/v1/batches/{id}/cleanup"))
        .bearer_auth(&secret)
        .json(&json!({"local_payloads":false,"remote":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let cleaned: Value = response.json().await.unwrap();
    assert_eq!(cleaned["localrouter"]["cleanup"]["upstream"], "unsupported");
    let response = http
        .post(format!("{base}/v1/batches/{id}/cleanup"))
        .bearer_auth(&secret)
        .json(&json!({"local_payloads":true,"remote":false}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let cleaned: Value = response.json().await.unwrap();
    assert_eq!(cleaned["output_file_id"], Value::Null);
    assert_eq!(
        cleaned["localrouter"]["accounting"]["billing"]["reported_charge_usd"],
        0.001
    );
}

#[tokio::test]
async fn native_batch_reconciliation_proves_nonce_input_and_snapshot_without_reissuing_create() {
    let mock = native_mock().await;
    mount_upload(&mock).await;
    Mock::given(method("POST"))
        .and(path("/v1/batches"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&mock)
        .await;
    let (base, secret, _) =
        start_replay_test_server(vec![provider(&mock)], allow_replay_fixture, true).await;
    let file = upload(&base, &secret).await;
    let http = reqwest::Client::new();
    let response = http
        .post(format!("{base}/v1/batches"))
        .bearer_auth(&secret)
        .json(&create_body(&file, false))
        .send()
        .await
        .unwrap();
    let id = response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let pending = idle(&base, &secret, &id).await;
    assert_eq!(pending["localrouter"]["phase"], "submit_pending_outcome");
    let sent = mock
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.method.as_str() == "POST" && r.url.path() == "/v1/batches")
        .unwrap();
    let sent: Value = serde_json::from_slice(&sent.body).unwrap();
    let mut proven = native("in_progress");
    proven["metadata"] = sent["metadata"].clone();
    Mock::given(method("GET")).and(path("/v1/batches")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"object":"list","data":[{"id":"foreign-video","endpoint":"/v1/videos","metadata":{"localrouter_nonce":"foreign"}},proven],"has_more":false,"first_id":"foreign-video","last_id":"batch_native_owned"}))).expect(1).mount(&mock).await;
    let response = http
        .post(format!("{base}/v1/batches/{id}/reconcile"))
        .bearer_auth(&secret)
        .json(&json!({"after":null}))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let answer: Value = response.json().await.unwrap();
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["localrouter"]["phase"], "submitted");
    assert_eq!(
        http.post(format!("{base}/v1/batches/{id}/submit"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        http.post(format!("{base}/v1/batches/{id}/resume"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
}

#[tokio::test]
async fn native_batch_changed_credentials_and_current_deny_never_upload_prepared_inference() {
    let mock = native_mock().await;
    let (base, secret, state) =
        start_replay_test_server(vec![provider(&mock)], allow_replay_fixture, true).await;
    let file = upload(&base, &secret).await;
    let http = reqwest::Client::new();
    let response = http
        .post(format!("{base}/v1/batches"))
        .bearer_auth(&secret)
        .json(&create_body(&file, true))
        .send()
        .await
        .unwrap();
    let id = response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        idle(&base, &secret, &id).await["localrouter"]["phase"],
        "prepared"
    );
    state
        .provider_registry
        .update_provider(
            "nativebatch".into(),
            "native_batch_fixture".into(),
            HashMap::from([
                ("base_url".into(), format!("{}/v1", mock.uri())),
                ("api_key".into(), "rotated-fixture-native-key".into()),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(
        http.post(format!("{base}/v1/batches/{id}/submit"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let paused = idle(&base, &secret, &id).await;
    assert_eq!(paused["localrouter"]["phase"], "prepared");
    assert!(!paused["localrouter"]["last_error"].is_null());
    assert!(mock
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method.as_str() == "GET"));
    assert_eq!(
        http.post(format!("{base}/v1/batches/{id}/cancel"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(get(&base, &secret, &id).await["status"], "cancelled");
    let mock2 = native_mock().await;
    let (base, secret, _) = start_replay_test_server(
        vec![provider(&mock2)],
        |config| {
            allow_replay_fixture(config);
            config.strategies[0].model_permissions.global =
                localrouter::config::PermissionState::Off;
        },
        true,
    )
    .await;
    let file = upload(&base, &secret).await;
    let response = http
        .post(format!("{base}/v1/batches"))
        .bearer_auth(&secret)
        .json(&create_body(&file, false))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    assert!(mock2.received_requests().await.unwrap().is_empty());
}

async fn pending_model(
    state: &server::state::AppState,
) -> localrouter::mcp::gateway::firewall::PendingApprovalInfo {
    for _ in 0..200 {
        if let Some(pending) = state
            .mcp_gateway
            .firewall_manager
            .list_pending()
            .into_iter()
            .find(|p| p.is_model_request)
        {
            return pending;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("No batch model approval appeared")
}
#[tokio::test]
async fn native_batch_every_row_uses_real_approval_and_saved_edits_survive_a_later_denial() {
    let mock = native_mock().await;
    let (base, secret, state) = start_replay_test_server(
        vec![provider(&mock)],
        |config| {
            allow_replay_fixture(config);
            config.strategies[0].model_permissions.global =
                localrouter::config::PermissionState::Ask;
        },
        true,
    )
    .await;
    let mut lines = String::new();
    for id in ["one", "two"] {
        lines.push_str(&json!({"custom_id":id,"method":"POST","url":"/v1/chat/completions","body":{"model":"nativebatch/gpt-sdk-native-batch-fixture-unique","max_tokens":16,"messages":[{"role":"user","content":"original"}]}}).to_string());
        lines.push('\n');
    }
    let file = state
        .batch_store
        .upload(
            "test-api-key",
            "two-rows.jsonl",
            "batch",
            lines.as_bytes(),
            3600,
        )
        .unwrap();
    let http = reqwest::Client::new();
    let response = http
        .post(format!("{base}/v1/batches"))
        .bearer_auth(&secret)
        .json(&create_body(&file.id, false))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let value: Value = response.json().await.unwrap();
    assert_eq!(status, 200, "{value}");
    let id = value["id"].as_str().unwrap();
    let first = pending_model(&state).await;
    let mut edited: Value = serde_json::from_str(&first.full_arguments.unwrap()).unwrap();
    edited["messages"][0]["content"] = json!("user-approved edit");
    state
        .mcp_gateway
        .firewall_manager
        .submit_response(
            &first.request_id,
            localrouter::mcp::gateway::firewall::FirewallApprovalAction::AllowOnce,
            Some(edited),
        )
        .unwrap();
    let second = pending_model(&state).await;
    state
        .mcp_gateway
        .firewall_manager
        .submit_response(
            &second.request_id,
            localrouter::mcp::gateway::firewall::FirewallApprovalAction::Deny,
            None,
        )
        .unwrap();
    let paused = idle(&base, &secret, id).await;
    assert_eq!(paused["localrouter"]["phase"], "draft", "{paused}");
    assert_eq!(paused["localrouter"]["validated_rows"], 1);
    let edits = state.batch_store.row_edits("test-api-key", id).unwrap();
    assert_eq!(edits["one"]["messages"][0]["content"], "user-approved edit");
    assert!(mock
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method.as_str() == "GET"));
    assert_eq!(
        http.post(format!("{base}/v1/batches/{id}/resume"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let resumed = pending_model(&state).await;
    assert_eq!(
        serde_json::from_str::<Value>(resumed.full_arguments.as_ref().unwrap()).unwrap()
            ["messages"][0]["content"],
        "user-approved edit"
    );
    state
        .mcp_gateway
        .firewall_manager
        .submit_response(
            &resumed.request_id,
            localrouter::mcp::gateway::firewall::FirewallApprovalAction::Deny,
            None,
        )
        .unwrap();
    idle(&base, &secret, id).await;
    assert!(mock
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method.as_str() == "GET"));
    assert!(state
        .monitor_store
        .list(0, 100, None)
        .events
        .iter()
        .all(|event| event.event_type != lr_monitor::MonitorEventType::LlmCall));
}
#[tokio::test]
async fn native_batch_cost_limits_fail_closed_without_verified_native_rates() {
    let mock = native_mock().await;
    let (base, secret, state) =
        start_replay_test_server(vec![provider(&mock)], allow_replay_fixture, true).await;
    state.rate_limiter.add_api_key_limiters(
        "test-api-key".into(),
        vec![lr_router::rate_limit::RateLimiter::new(
            lr_router::rate_limit::RateLimitType::Cost,
            10.0,
            3600,
        )],
    );
    let file = upload(&base, &secret).await;
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/batches"))
        .bearer_auth(&secret)
        .json(&create_body(&file, false))
        .send()
        .await
        .unwrap();
    let value: Value = response.json().await.unwrap();
    let id = value["id"].as_str().unwrap();
    let paused = idle(&base, &secret, id).await;
    assert_eq!(paused["localrouter"]["phase"], "prepared");
    assert_eq!(
        paused["localrouter"]["last_error"]["code"],
        "native_price_unavailable"
    );
    assert!(mock
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method.as_str() == "GET"));
    assert!(state
        .batch_store
        .job_reservation("test-api-key", id)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn native_batch_shutdown_stops_pending_approval_without_upload_or_auto_resume() {
    let mock = native_mock().await;
    let (base, secret, state) = start_replay_test_server(
        vec![provider(&mock)],
        |config| {
            allow_replay_fixture(config);
            config.strategies[0].model_permissions.global =
                localrouter::config::PermissionState::Ask;
        },
        true,
    )
    .await;
    let file = upload(&base, &secret).await;
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/batches"))
        .bearer_auth(&secret)
        .json(&create_body(&file, false))
        .send()
        .await
        .unwrap();
    let value: Value = response.json().await.unwrap();
    let id = value["id"].as_str().unwrap();
    pending_model(&state).await;
    state.batch_runtime.cancel_all_workers();
    let paused = idle(&base, &secret, id).await;
    assert_eq!(paused["localrouter"]["phase"], "draft");
    assert_eq!(state.mcp_gateway.firewall_manager.pending_count(), 0);
    assert!(mock
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method.as_str() == "GET"));
}
