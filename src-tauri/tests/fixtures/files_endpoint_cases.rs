//! Authenticated local Files lifecycle, independent of any upstream model.
use super::*;
fn form(bytes: &'static [u8], purpose: &str) -> reqwest::multipart::Form {
    reqwest::multipart::Form::new()
        .text("purpose", purpose.to_owned())
        .part(
            "file",
            reqwest::multipart::Part::bytes(bytes).file_name("../input.jsonl"),
        )
}

#[tokio::test]
async fn files_owned_lifecycle_survives_key_rotation_and_excludes_other_clients() {
    let (base, secret, state) = start_replay_test_server(vec![], |_| {}, true).await;
    let http = reqwest::Client::new();
    let body=b"{\"custom_id\":\"one\",\"method\":\"POST\",\"url\":\"/v1/chat/completions\",\"body\":{\"model\":\"explicit/model\",\"messages\":[]}}\n";
    let response = http
        .post(format!("{base}/v1/files"))
        .bearer_auth(&secret)
        .multipart(form(body, "batch"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    // Obtain the ID from the owned list to exercise the SDK list envelope.
    let list: Value = http
        .get(format!("{base}/v1/files?purpose=batch&limit=1"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = list["data"][0]["id"].as_str().unwrap();
    assert!(id.starts_with("file-lr-"));
    assert_eq!(list["data"][0]["filename"], "../input.jsonl");
    assert_eq!(list["data"][0]["bytes"], body.len());
    let other = create_test_client("files-other-client", "default");
    let other_secret = state.client_manager.add_client_with_secret(other).unwrap();
    for suffix in ["", "/content"] {
        assert_eq!(
            http.get(format!("{base}/v1/files/{id}{suffix}"))
                .bearer_auth(&other_secret)
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    assert_eq!(
        http.delete(format!("{base}/v1/files/{id}"))
            .bearer_auth(&other_secret)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        http.get(format!("{base}/v1/files?after={id}"))
            .bearer_auth(&other_secret)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let empty: Value = http
        .get(format!("{base}/v1/files"))
        .bearer_auth(&other_secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(empty["data"], json!([]));
    let rotated = state.client_manager.rotate_secret("test-api-key").unwrap();
    assert_eq!(
        http.get(format!("{base}/v1/files/{id}"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let response = http
        .get(format!("{base}/v1/files/{id}/content"))
        .bearer_auth(&rotated)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.bytes().await.unwrap().as_ref(), body);
    let result: Value = http
        .delete(format!("{base}/v1/files/{id}"))
        .bearer_auth(&rotated)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(result, json!({"id":id,"object":"file","deleted":true}));
    assert_eq!(
        http.get(format!("{base}/v1/files/{id}"))
            .bearer_auth(&rotated)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    // Cleanup only this fixture's explicitly owned directory.
    std::fs::remove_dir_all(
        state
            .config_manager
            .config_path()
            .with_extension("batch-store"),
    )
    .unwrap();
}

#[tokio::test]
async fn files_reject_internal_identity_malformed_and_unsupported_uploads_and_publish_openapi() {
    let (base, secret, state) = start_replay_test_server(vec![], |_| {}, true).await;
    let http = reqwest::Client::new();
    let root = state
        .config_manager
        .config_path()
        .with_extension("batch-store");
    for (token, body, purpose, status) in [
        (
            state.get_internal_test_secret(),
            b"{}".as_slice(),
            "batch",
            403,
        ),
        (secret.clone(), b"{}".as_slice(), "assistants", 400),
        (secret.clone(), b"[]\n".as_slice(), "batch", 400),
        (secret.clone(), b"{broken}\n".as_slice(), "batch", 400),
    ] {
        let response = http
            .post(format!("{base}/v1/files"))
            .bearer_auth(token)
            .multipart(form(body, purpose))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            status,
            "{}",
            response.text().await.unwrap()
        );
    }
    assert!(!root.exists());
    assert_eq!(
        http.get(format!("{base}/v1/files"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    for query in [
        "order=random",
        "limit=0",
        "limit=101",
        "purpose=assistants",
        "unknown=x",
    ] {
        let response = http
            .get(format!("{base}/v1/files?{query}"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_client_error(), "{query}");
    }
    let response = http
        .post(format!("{base}/v1/files"))
        .bearer_auth(&secret)
        .multipart(form(b"{}", "batch").text("purpose", "batch"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert!(!root.exists());
    let spec: Value = http
        .get(format!("{base}/openapi.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    for (path, method) in [
        ("/v1/files", "get"),
        ("/v1/files", "post"),
        ("/v1/files/{file_id}", "get"),
        ("/v1/files/{file_id}", "delete"),
        ("/v1/files/{file_id}/content", "get"),
    ] {
        assert!(spec["paths"][path][method].is_object(), "{path} {method}");
        assert_eq!(
            spec["paths"][path][method]["security"][0],
            json!({"bearer_auth":[]})
        );
    }
    assert!(spec["components"]["schemas"]["FileObject"].is_object());
}
