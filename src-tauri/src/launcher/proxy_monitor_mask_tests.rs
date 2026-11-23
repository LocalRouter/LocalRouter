//! Requests LocalRouter refuses itself (rate limits, model or GuardRails
//! refusals, Secret Scan denials, refusals at stream open) never leave a
//! detectable secret in the monitor, through the real AppFirewall masking.
use super::{secret_policy_tests::fixture, AppFirewall};
use base64::Engine;
use lr_monitor::MonitorEventStore;
use lr_proxy::{
    active::{ActiveInterceptor, Firewall},
    interceptor::{BoundedCapture, ObservedExchange, ProxyInterceptor},
    passive::PassiveInterceptor,
};
use serde_json::{json, Value};
use std::sync::Arc;

const KEY: &str = super::secret_policy_tests::KEY;

fn firewall(root: &std::path::Path, action: lr_config::SecretScanAction) -> AppFirewall {
    let (firewall, _) = fixture(
        root,
        lr_config::LlmMode::Proxy,
        Arc::new(lr_api_keys::MockKeychain::new()),
    );
    firewall
        .config_manager
        .update(|c| c.secret_scanning.action = action)
        .unwrap();
    firewall
}

fn client_id(firewall: &AppFirewall) -> String {
    firewall.config_manager.get().clients[0].id.clone()
}

fn body() -> Value {
    let attachment = base64::engine::general_purpose::STANDARD
        .encode(format!("[default]\naws_secret_access_key={KEY}\n"));
    json!({
        "model": "local-test",
        "input": [
            {"role": "user", "content": format!("aws_secret_access_key={KEY}")},
            {"role": "user", "content": [{"type": "input_file", "filename": "credentials",
                "file_data": format!("data:text/plain;base64,{attachment}")}]},
        ],
    })
}

/// Everything a monitor event of the store keeps, as text, with every
/// base64 attachment decoded too.
fn stored(store: &MonitorEventStore) -> String {
    let events: Vec<_> = store
        .list(0, 32, None)
        .events
        .iter()
        .map(|summary| store.get(&summary.id).expect("stored event"))
        .collect();
    let mut text = serde_json::to_string(&events).unwrap();
    let decoded: Vec<String> = text
        .split("base64,")
        .skip(1)
        .filter_map(|rest| {
            let data: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
                .collect();
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        })
        .collect();
    for d in decoded {
        text.push_str(&d);
    }
    text
}

fn refused(client_id: &str, body: &Value) -> ObservedExchange {
    ObservedExchange {
        client_id: client_id.to_owned(),
        session_id: Some("refused".into()),
        host: "chatgpt.com".into(),
        method: "POST".into(),
        path: "/backend-api/codex/responses".into(),
        status: Some(429),
        request_body: Some(serde_json::to_vec(body).unwrap()),
        response_body: Some(br#"{"error":"Rate limit exceeded (requests per day)"}"#.to_vec()),
        error: Some(format!("refused: aws_secret_access_key={KEY}")),
        refused_locally: true,
        ..Default::default()
    }
}

fn interceptor(firewall: AppFirewall) -> (ActiveInterceptor, Arc<MonitorEventStore>) {
    let store = Arc::new(MonitorEventStore::new(32));
    (
        ActiveInterceptor::new(PassiveInterceptor::new(store.clone()), Arc::new(firewall)),
        store,
    )
}

/// A refused buffered body: the raw capture, the parsed body, the error,
/// the conversation and the decoded attachment all lose the secret, for
/// every action that scans.
#[tokio::test]
async fn refused_requests_keep_no_secret_for_actions_that_scan() {
    for action in [
        lr_config::SecretScanAction::Redact,
        lr_config::SecretScanAction::Ask,
        lr_config::SecretScanAction::Notify,
    ] {
        let root = tempfile::tempdir().unwrap();
        let firewall = firewall(root.path(), action.clone());
        let client = client_id(&firewall);
        let (it, store) = interceptor(firewall);
        it.on_response(&refused(&client, &body())).await;
        let events = stored(&store);
        assert!(!events.contains(KEY), "{action:?}: {events}");
        assert!(events.contains("REDACTED"), "{action:?}: {events}");
    }
}

/// The head of a refused body (a head-time refusal, a refusal at stream
/// open), cut by the capture cap inside the secret.
#[tokio::test]
async fn a_refused_capture_cut_inside_a_secret_keeps_none_of_it() {
    let root = tempfile::tempdir().unwrap();
    let firewall = firewall(root.path(), lr_config::SecretScanAction::Redact);
    let client = client_id(&firewall);
    let (it, store) = interceptor(firewall);
    let bytes = serde_json::to_vec(&body()).unwrap();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    // Cut 6 characters into the secret: the prefix is not detectable.
    let cut = text.find(KEY).unwrap() + 6;
    let mut ex = refused(&client, &body());
    ex.request_body = None;
    ex.request_capture = Some(BoundedCapture {
        head: bytes[..cut].to_vec(),
        total_len: bytes.len() as u64,
        truncated: true,
        complete: false,
        ..Default::default()
    });
    it.on_response(&ex).await;
    let events = stored(&store);
    assert!(!events.contains(&KEY[..6]), "{events}");
    assert!(events.contains("local-test"), "{events}");
}

/// Secret Scan Off forwards and records requests as they are: a refusal is
/// kept as it is too, as on the success path.
#[tokio::test]
async fn secret_scan_off_keeps_a_refusal_as_it_is() {
    let root = tempfile::tempdir().unwrap();
    let firewall = firewall(root.path(), lr_config::SecretScanAction::Off);
    let client = client_id(&firewall);
    let mut content = serde_json::to_vec(&body()).unwrap();
    let original = content.clone();
    assert!(firewall.mask_for_monitor(&client, &mut content));
    assert_eq!(content, original);
}

/// Without a scanner nothing can be masked: only the length is kept.
#[tokio::test]
async fn without_a_scanner_only_the_length_is_kept() {
    let root = tempfile::tempdir().unwrap();
    let firewall = firewall(root.path(), lr_config::SecretScanAction::Redact);
    *firewall.state.secret_scanner.write() = None;
    let client = client_id(&firewall);
    let (it, store) = interceptor(firewall);
    let ex = refused(&client, &body());
    let len = ex.request_body.as_ref().unwrap().len();
    it.on_response(&ex).await;
    let events = stored(&store);
    assert!(!events.contains(KEY), "{events}");
    assert!(
        events.contains(&format!("[{len} bytes not kept")),
        "{events}"
    );
}

/// The real policy refusing a request (a full strategy limit) and the
/// monitor event of that refusal, through the same interceptor.
#[tokio::test]
async fn a_rate_limited_request_through_the_firewall_keeps_no_secret() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, mut request) = fixture(
        root.path(),
        lr_config::LlmMode::Proxy,
        Arc::new(lr_api_keys::MockKeychain::new()),
    );
    // Built directly: a validated update refuses a limit of zero.
    let mut config = firewall.config_manager.get();
    config.strategies[0].rate_limits = vec![lr_config::StrategyRateLimit {
        limit_type: lr_config::RateLimitType::Requests,
        value: 0.0,
        time_window: lr_config::RateLimitTimeWindow::Day,
        enabled: true,
    }];
    let firewall = AppFirewall {
        config_manager: lr_config::ConfigManager::new(config, root.path().join("limited.yaml")),
        ..firewall
    };
    request.body = body();
    let action = firewall.evaluate(&request).await;
    let lr_proxy::interceptor::RequestAction::Reject {
        status,
        body: reply,
        ..
    } = action
    else {
        panic!("a refusal");
    };
    assert_eq!(status, 429);
    let client = request.client_id.clone();
    let (it, store) = interceptor(firewall);
    let mut ex = refused(&client, &request.body);
    ex.response_body = Some(reply);
    ex.error = None;
    it.on_response(&ex).await;
    let events = stored(&store);
    assert!(!events.contains(KEY), "{events}");
    assert!(events.contains("requests per day"), "{events}");
}
