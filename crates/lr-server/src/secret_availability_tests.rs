use super::*;
use axum::{
    body::{to_bytes, Bytes},
    routing::post,
    Extension,
};
use lr_api_keys::KeychainStorage;
use lr_secret_scanner::placeholders::Vault;
use lr_types::{AppError, AppResult};
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tower::ServiceExt;
struct Broken;
impl KeychainStorage for Broken {
    fn get(&self, _: &str, _: &str) -> AppResult<Option<String>> {
        Err(AppError::Storage("injected outage".into()))
    }
    fn store(&self, _: &str, _: &str, _: &str) -> AppResult<()> {
        Err(AppError::Storage("injected outage".into()))
    }
    fn delete(&self, _: &str, _: &str) -> AppResult<()> {
        Err(AppError::Storage("injected outage".into()))
    }
}

fn app(state: AppState, storage: Arc<dyn KeychainStorage>, seen: Arc<AtomicUsize>) -> axum::Router {
    let id = state.config_manager.get().clients[0].id.clone();
    let echo = move |body: Bytes| {
        let seen = seen.clone();
        async move {
            seen.fetch_add(1, Ordering::Relaxed);
            Response::builder()
                .status(201)
                .body(Body::from(body))
                .unwrap()
        }
    };
    axum::Router::new()
        .route("/v1/chat/completions", post(echo.clone()))
        .route("/v1/embeddings", post(echo))
        .layer(axum::middleware::from_fn_with_state(
            state,
            move |State(state): State<AppState>, request: Request, next: Next| {
                let storage = storage.clone();
                async move {
                    match run(state, request, next, storage).await {
                        Ok(response) => response,
                        Err(error) => error.into_response(),
                    }
                }
            },
        ))
        .layer(Extension(AuthContext {
            api_key_id: id,
            credential_scope: None,
            model_selection: None,
        }))
}
fn request(path: &str, body: String) -> Request {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap()
}
#[tokio::test]
async fn gateway_forwards_failures_and_unsupported_content_to_the_endpoint() {
    for case in [
        "storage",
        "scanner",
        "mixed-code",
        "session",
        "malformed-json",
        "endpoint",
        "attachment",
    ] {
        let root = tempfile::tempdir().unwrap();
        let state = super::boundary_tests::state(root.path());
        let owner = state.config_manager.get().clients[0].id.clone();
        let memory = Arc::new(lr_api_keys::MockKeychain::new());
        let storage: Arc<dyn KeychainStorage> = if case == "storage" {
            Arc::new(Broken)
        } else {
            memory.clone()
        };
        if case == "scanner" {
            *state.secret_scanner.write() = None;
        }
        if case == "mixed-code" {
            Vault::for_client(memory, &owner)
                .register("unrelated-saved-original")
                .unwrap();
        }
        let mut body =
            json!({"model":"fake/model","messages":[{"role":"user","content":"ordinary input"}]});
        if case == "storage" || case == "scanner" {
            body["messages"][0]["content"] =
                json!("aws_secret_access_key=6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQr");
        }
        if case == "mixed-code" {
            body["messages"][0]["content"] = json!(
                r#"Example: {"code":"print("hello\nworld")", "html":"&amp;", "percent":"%41"}"#
            );
        }
        if case == "session" {
            body["extensions"] = json!({"localrouter.session":{"id":""}});
        }
        if case == "attachment" {
            body["messages"][0]["content"] =
                json!([{"type":"input_file","file_id":"provider-owned-file"}]);
        }
        let body = if case == "malformed-json" {
            "not valid JSON".into()
        } else {
            body.to_string()
        };
        let path = if case == "endpoint" {
            "/v1/embeddings"
        } else {
            "/v1/chat/completions"
        };
        let seen = Arc::new(AtomicUsize::new(0));
        let router = app(state, storage, seen.clone());
        let response = router.oneshot(request(path, body.clone())).await.unwrap();
        assert_eq!(response.status(), 201, "{case}");
        let bytes = to_bytes(response.into_body(), LIMIT).await.unwrap();
        if case == "malformed-json" {
            assert_eq!(bytes.as_ref(), body.as_bytes());
        } else {
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap(),
                serde_json::from_str::<Value>(&body).unwrap(),
                "{case}"
            );
        }
        assert_eq!(seen.load(Ordering::Relaxed), 1, "{case}");
    }
}
#[tokio::test]
async fn only_an_explicit_user_denial_returns_the_specific_403() {
    for dismiss in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = super::boundary_tests::state(root.path());
        state
            .config_manager
            .update(|config| config.secret_scanning.action = lr_config::SecretScanAction::Ask)
            .unwrap();
        let seen = Arc::new(AtomicUsize::new(0));
        let router = app(
            state.clone(),
            Arc::new(lr_api_keys::MockKeychain::new()),
            seen.clone(),
        );
        let body=json!({"model":"fake/model","messages":[{"role":"user","content":"aws_secret_access_key=6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQr"}]}).to_string();
        let task = tokio::spawn(router.oneshot(request("/v1/chat/completions", body)));
        let pending = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(p) = state
                    .mcp_gateway
                    .firewall_manager
                    .list_pending()
                    .into_iter()
                    .next()
                {
                    break p;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        if dismiss {
            state
                .mcp_gateway
                .firewall_manager
                .cancel_request(&pending.request_id)
                .unwrap();
        } else {
            state
                .mcp_gateway
                .firewall_manager
                .submit_response(
                    &pending.request_id,
                    lr_mcp::gateway::firewall::FirewallApprovalAction::Deny,
                    None,
                )
                .unwrap();
        }
        let response = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if dismiss {
            assert_eq!(response.status(), 201);
            assert_eq!(seen.load(Ordering::Relaxed), 1);
        } else {
            assert_eq!(response.status(), 403);
            assert_eq!(seen.load(Ordering::Relaxed), 0);
            let bytes = to_bytes(response.into_body(), LIMIT).await.unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap()["error"]["code"],
                "secret_scan_denied_by_user"
            );
        }
    }
}
