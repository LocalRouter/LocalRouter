//! Standalone search through real TCP, active dispatch, the shared detector,
//! reversible masking, rolling response restoration and Monitor recording.
use std::{
    convert::Infallible,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::{
    body::{Frame, Incoming},
    service::service_fn,
    Request, Response,
};
use hyper_util::rt::TokioIo;
use lr_api_keys::{keychain_trait::ResilientSecretStorage, KeychainStorage, MockKeychain};
use lr_monitor::{EventStatus, MonitorEventData, MonitorEventStore};
use lr_proxy::{
    active::{ActiveInterceptor, Firewall, FirewallRequest},
    interceptor::{ObservedExchange, RequestAction},
    passive::PassiveInterceptor,
    reverse::{ReverseClient, ReverseProxy},
};
use lr_secret_scanner::{placeholders::Vault, SecretScanEngine, SecretScanEngineConfig};
use lr_types::{secret_context::SecretContext, AppError, AppResult, LlmApi};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};

const PATH: &str = "/backend-api/codex/alpha/search";
const SECRET: &str = "Z9v4qW8rT2yU6iO0pA5sD1fG3hJ7kL9zX2cV4bNQ";

struct BrokenStorage;
impl KeychainStorage for BrokenStorage {
    fn get(&self, _: &str, _: &str) -> AppResult<Option<String>> {
        Err(AppError::Storage("fixture disk full".into()))
    }
    fn store(&self, _: &str, _: &str, _: &str) -> AppResult<()> {
        Err(AppError::Storage("fixture disk full".into()))
    }
    fn delete(&self, _: &str, _: &str) -> AppResult<()> {
        Err(AppError::Storage("fixture disk full".into()))
    }
}

struct SearchFirewall {
    storage: Arc<dyn KeychainStorage>,
    warnings: AtomicUsize,
}
#[async_trait]
impl Firewall for SearchFirewall {
    fn protects_secrets(&self, _: &str) -> bool {
        true
    }
    fn secret_inspection_warning(&self, _: &ObservedExchange, _: &str) {
        self.warnings.fetch_add(1, Ordering::Relaxed);
    }
    async fn evaluate_uninspectable(
        &self,
        _: &FirewallRequest,
        _: lr_proxy::request_body::DecodeError,
    ) -> RequestAction {
        RequestAction::Forward
    }
    async fn evaluate(&self, request: &FirewallRequest) -> RequestAction {
        let vault = Arc::new(Vault::for_client(self.storage.clone(), &request.client_id));
        let scanner = SecretScanEngine::new(&SecretScanEngineConfig {
            entropy_threshold: 3.0,
            allowlist: vec![],
            scan_system_messages: true,
        })
        .unwrap();
        let texts = lr_guardrails::text_extractor::extract_request_text(&request.body)
            .into_iter()
            .enumerate()
            .map(|(i, t)| lr_secret_scanner::ExtractedText {
                text: t.text,
                label: t.label,
                message_index: i,
            })
            .collect::<Vec<_>>();
        let findings = scanner.scan(&texts).findings;
        assert!(
            !findings.is_empty(),
            "standalone commands must reach the production text extractor"
        );
        for finding in findings {
            vault.register(&finding.raw_text).unwrap();
        }
        let mut body = request.body.clone();
        vault.protect(&mut body).unwrap();
        vault.set_request_contract(&body).unwrap();
        RequestAction::Protect {
            body: serde_json::to_vec(&body).unwrap(),
            vault,
        }
    }
}

async fn roundtrip(storage_offline: bool, malformed_response: bool) {
    assert_eq!(SECRET.len(), 40);
    let upstream = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = upstream.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        loop {
            let (tcp, _) = upstream.accept().await.unwrap();
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<Incoming>| async move {
                    assert_eq!(request.uri().path(), PATH);
                    let bytes = request.into_body().collect().await.unwrap().to_bytes();
                    let body: Value = serde_json::from_slice(&bytes).unwrap();
                    assert!(!body.to_string().contains(SECRET));
                    assert_eq!(body["id"], "search-session");
                    assert_eq!(body["settings"]["external_web_access"], true);
                    assert_eq!(body["future_field"], json!({"retained":true}));
                    assert_eq!(body["commands"]["open"][0]["ref_id"], "turn0search0");
                    let text = body["commands"]["search_query"][0]["q"].as_str().unwrap();
                    let tag = text.strip_prefix("aws_secret_access_key=").unwrap();
                    assert!(tag.starts_with("<localrouter-redacted"));
                    if let Some(input) = body.get("input") {
                        assert_eq!(input[0]["content"][0]["text"], text);
                        assert_eq!(input[1]["encrypted_content"], "opaque-state");
                    }
                    let payload = if malformed_response {
                        b"{not-json-provider-response".to_vec()
                    } else {
                        serde_json::to_vec(&json!({"encrypted_output":format!("cipher:{tag}"),"output":format!("🔎 {tag}"),"results":[{"type":"text_result","ref_id":"turn0search0","url":"https://example.com/","snippet":tag}],"future_field":{"retained":true}})).unwrap()
                    };
                    let chunks = payload
                        .chunks(3)
                        .map(|chunk| {
                            Ok::<_, Infallible>(Frame::data(Bytes::copy_from_slice(chunk)))
                        })
                        .collect::<Vec<_>>();
                    Ok::<_, Infallible>(
                        Response::builder()
                            .header("content-type", "application/json")
                            .body(StreamBody::new(futures::stream::iter(chunks)))
                            .unwrap(),
                    )
                });
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(tcp), service)
                    .await
                    .unwrap();
            });
        }
    });
    let storage: Arc<dyn KeychainStorage> = if storage_offline {
        Arc::new(ResilientSecretStorage::new(Arc::new(BrokenStorage)))
    } else {
        Arc::new(MockKeychain::new())
    };
    let firewall = Arc::new(SearchFirewall {
        storage,
        warnings: AtomicUsize::new(0),
    });
    let monitor = Arc::new(MonitorEventStore::new(32));
    let recorder = Arc::new(ActiveInterceptor::new(
        PassiveInterceptor::new(monitor.clone()),
        firewall.clone(),
    ));
    let proxy = ReverseProxy::new(
        &format!("http://127.0.0.1:{port}"),
        ReverseClient {
            client_id: "search-client".into(),
            ..Default::default()
        },
        recorder,
    )
    .unwrap();
    let listener = ReverseProxy::bind("127.0.0.1", 0).await.unwrap();
    let proxy_port = listener.local_addr().unwrap().port();
    let proxy_task = tokio::spawn(async move {
        Arc::new(proxy)
            .serve(listener, std::future::pending::<()>())
            .await
    });
    let tcp = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tcp))
        .await
        .unwrap();
    let client_task = tokio::spawn(async move { connection.await.unwrap() });
    for context in [false, true] {
        let text = format!("aws_secret_access_key={SECRET}");
        let mut body = json!({"id":"search-session","model":"local-test","commands":{"search_query":[{"q":text}],"open":[{"ref_id":"turn0search0"}]},"settings":{"external_web_access":true},"future_field":{"retained":true}});
        if context {
            body["input"] = json!([{"type":"message","role":"user","content":[{"type":"input_text","text":text}]},{"type":"web_search_call","encrypted_content":"opaque-state"}]);
        }
        let request = Request::builder()
            .method("POST")
            .uri(PATH)
            .header("host", "localhost")
            .body(Full::new(Bytes::from(body.to_string())))
            .unwrap();
        let response = sender.send_request(request).await.unwrap();
        assert_eq!(
            response.status(),
            200,
            "Secret Scan must not reject a standalone search"
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        if malformed_response {
            assert_eq!(bytes.as_ref(), b"{not-json-provider-response");
        } else {
            let response: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(response["output"], format!("🔎 {SECRET}"));
            assert_eq!(response["results"][0]["snippet"], SECRET);
            assert!(response["encrypted_output"]
                .as_str()
                .unwrap()
                .contains("<localrouter-redacted"));
            assert_eq!(response["results"][0]["ref_id"], "turn0search0");
        }
    }
    for _ in 0..100 {
        let events = monitor.list(0, 10, None).events;
        if events.len() == 2 && events.iter().all(|e| e.status != EventStatus::Pending) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let events = monitor.list(0, 10, None).events;
    assert_eq!(events.len(), 2);
    for event in events {
        assert_eq!(event.status, EventStatus::Complete);
        let full = monitor.get(&event.id).unwrap();
        assert!(!serde_json::to_string(&full).unwrap().contains(SECRET));
        assert!(
            event.question.contains("localrouter-redacted"),
            "search commands must appear in the preview"
        );
        let MonitorEventData::LlmCall {
            client_api,
            upstream_api,
            endpoint,
            ..
        } = full.data
        else {
            panic!("search must be recorded as an inspected API call")
        };
        assert_eq!(client_api, Some(LlmApi::Search));
        assert_eq!(upstream_api, Some(LlmApi::Search));
        assert_eq!(endpoint, PATH);
    }
    assert_eq!(
        firewall.warnings.load(Ordering::Relaxed),
        0,
        "recognized searches must not emit unsupported-endpoint warnings"
    );
    proxy_task.abort();
    client_task.abort();
    server.abort();
}

#[tokio::test]
async fn standalone_search_masks_queries_and_context_and_restores_fragmented_results() {
    roundtrip(false, false).await;
}
#[tokio::test]
async fn standalone_search_keeps_working_during_credential_storage_outage() {
    roundtrip(true, false).await;
}
#[tokio::test]
async fn standalone_search_delivers_malformed_provider_response_unchanged() {
    roundtrip(false, true).await;
}
