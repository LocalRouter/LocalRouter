//! Server state across restarts, and per-request monitor event noise.

use std::sync::Arc;

use localrouter::clients::{ClientManager, TokenStore};
use localrouter::config::{AppConfig, ConfigManager};
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use localrouter::server::manager::ServerDependencies;
use localrouter::server::routes::pipeline::{scan_request_for_secrets, SecretScanOutcome};
use localrouter::server::state::AppState;
use localrouter::server::{ServerConfig, ServerManager};
use lr_mcp::McpServerManager;
use lr_monitor::{EventStatus, MonitorEventData, MonitorEventType};

fn dependencies() -> ServerDependencies {
    let config_path =
        std::env::temp_dir().join(format!("test_monitor_{}.yaml", uuid::Uuid::new_v4()));
    let config_manager = Arc::new(ConfigManager::new(AppConfig::default(), config_path));
    let provider_registry = Arc::new(ProviderRegistry::new());
    let rate_limiter = Arc::new(RateLimiterManager::new(None));
    let metrics_db_path =
        std::env::temp_dir().join(format!("test_monitor_{}.db", uuid::Uuid::new_v4()));
    let metrics_collector = Arc::new(MetricsCollector::new(Arc::new(
        MetricsDatabase::new(metrics_db_path).unwrap(),
    )));
    let router = Arc::new(Router::new(
        config_manager.clone(),
        provider_registry.clone(),
        rate_limiter.clone(),
        metrics_collector.clone(),
        Arc::new(lr_router::FreeTierManager::new(None)),
    ));
    ServerDependencies {
        router,
        mcp_server_manager: Arc::new(McpServerManager::new()),
        rate_limiter,
        provider_registry,
        config_manager,
        client_manager: Arc::new(ClientManager::new(vec![])),
        token_store: Arc::new(TokenStore::new()),
        metrics_collector,
        health_cache: None,
    }
}

fn server_config() -> ServerConfig {
    ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        enable_cors: false,
    }
}

fn push_marker(state: &AppState) -> String {
    state.monitor_store.push(
        MonitorEventType::PromptCompression,
        None,
        None,
        None,
        MonitorEventData::PromptCompression {
            original_tokens: 10,
            compressed_tokens: 5,
            reduction_percent: 50.0,
            duration_ms: 1,
            method: "test".to_string(),
        },
        EventStatus::Complete,
        Some(1),
    )
}

async fn health(manager: &ServerManager) -> reqwest::StatusCode {
    let port = manager.get_actual_port().expect("listening");
    reqwest::get(format!("http://127.0.0.1:{port}/health"))
        .await
        .expect("server answers")
        .status()
}

/// A restart serves the state built by the first start, so everything wired
/// into it at launch (and held by the proxies and UI commands) stays live, and
/// monitor events stay readable while the server is stopped.
#[tokio::test]
async fn restart_serves_the_same_state() {
    let manager = ServerManager::new();
    assert!(manager.monitor_store().is_none());
    manager
        .start(server_config(), dependencies())
        .await
        .expect("server starts");
    assert!(health(&manager).await.is_success());
    let first = manager.get_state().expect("running");
    let id = push_marker(&first);
    let engine =
        lr_secret_scanner::SecretScanEngine::new(&lr_secret_scanner::SecretScanEngineConfig {
            entropy_threshold: 3.0,
            allowlist: vec![],
            scan_system_messages: false,
        })
        .expect("engine builds");
    *first.secret_scanner.write() = Some(Arc::new(engine));

    manager.stop().await;
    assert!(manager.get_state().is_none());
    let store = manager.monitor_store().expect("built");
    assert!(Arc::ptr_eq(&store, &first.monitor_store));
    assert_eq!(store.list(0, 10, None).events[0].id, id);

    manager
        .start(server_config(), dependencies())
        .await
        .expect("server restarts");
    assert!(health(&manager).await.is_success());
    let second = manager.get_state().expect("running");
    assert!(Arc::ptr_eq(&second.monitor_store, &first.monitor_store));
    assert!(Arc::ptr_eq(&second.mcp_gateway, &first.mcp_gateway));
    assert!(second.secret_scanner.read().is_some());
    assert!(second.monitor_store.get(&id).is_some());
    manager.stop().await;
}

async fn secret_scan_state() -> AppState {
    let deps = dependencies();
    let manager = ServerManager::new();
    manager
        .start(server_config(), deps)
        .await
        .expect("server starts");
    let state = manager.get_state().expect("running");
    let engine =
        lr_secret_scanner::SecretScanEngine::new(&lr_secret_scanner::SecretScanEngineConfig {
            entropy_threshold: 3.0,
            allowlist: vec![],
            scan_system_messages: false,
        })
        .expect("engine builds");
    *state.secret_scanner.write() = Some(Arc::new(engine));
    state
}

fn chat_body(content: &str) -> serde_json::Value {
    serde_json::json!({
        "model": "gpt-5.5",
        "messages": [{"role": "user", "content": content}]
    })
}

/// A clean secret scan runs on every request, so it records nothing.
#[tokio::test]
async fn clean_secret_scan_records_no_event() {
    let state = secret_scan_state().await;
    let outcome =
        scan_request_for_secrets(&state, "client", "gpt-5.5", &chat_body("hello there")).await;
    assert!(matches!(outcome, SecretScanOutcome::Allow));
    assert_eq!(state.monitor_store.list(0, 10, None).total, 0);
}

/// A scan with findings records one finished event carrying them.
#[tokio::test]
async fn secret_scan_with_findings_records_one_event() {
    let state = secret_scan_state().await;
    scan_request_for_secrets(
        &state,
        "client",
        "gpt-5.5",
        &chat_body("aws key AKIAZ7Q3VJ2M4XK9PLR6"),
    )
    .await;
    let listed = state.monitor_store.list(0, 10, None);
    assert_eq!(listed.total, 1);
    let event = state
        .monitor_store
        .get(&listed.events[0].id)
        .expect("event stored");
    assert_eq!(event.event_type, MonitorEventType::SecretScan);
    assert_eq!(event.status, EventStatus::Complete);
    match event.data {
        MonitorEventData::SecretScan {
            findings_count,
            action_taken,
            ..
        } => {
            assert!(findings_count.unwrap_or(0) >= 1);
            assert_eq!(action_taken.as_deref(), Some("notify"));
        }
        other => panic!("unexpected data {other:?}"),
    }
}

const REDACTION_SECRET: &str = "AKIAZ7Q3VJ2M4XK9PLR6";

fn remember_redaction(state: &AppState) {
    let salt = lr_secret_scanner::new_salt().unwrap();
    let mut client =
        lr_config::Client::new_with_strategy("Redaction test".into(), "strategy".into());
    client.id = "client".into();
    client.secret_scanning.action = Some(lr_config::SecretScanAction::Ask);
    client.secret_scanning.dismiss_salt = Some(salt.clone());
    client
        .secret_scanning
        .dismissed_secrets
        .push(lr_config::DismissedSecret {
            redact: true,
            id: "redact-entry".into(),
            hash: lr_secret_scanner::hash_secret(REDACTION_SECRET, &salt, 10).unwrap(),
            iterations: 10,
            rule_id: "aws-access-token".into(),
            rule_description: "AWS Access Key".into(),
            hint: "AKIA************PLR6".into(),
            dismissed_at: "2026-10-21T00:00:00Z".into(),
        });
    // Simulate a settings save/load, including compatibility of stored hashes.
    let serialized = serde_json::to_string(&client).unwrap();
    assert!(!serialized.contains(REDACTION_SECRET));
    let client = serde_json::from_str(&serialized).unwrap();
    state.client_manager.sync_clients(vec![client]);
}

#[tokio::test]
async fn remembered_secret_redacts_after_reload_and_during_bypass() {
    let state = secret_scan_state().await;
    remember_redaction(&state);
    for bypass in [false, true] {
        if bypass {
            state
                .secret_scan_approval_tracker
                .add_bypass("client", std::time::Duration::from_secs(60));
        }
        let mut body = chat_body(&format!("aws key {0} and again {0}", REDACTION_SECRET));
        let outcome = scan_request_for_secrets(&state, "client", "gpt-5.5", &body).await;
        let SecretScanOutcome::Redact(redaction) = outcome else {
            panic!("expected redaction")
        };
        redaction.apply(&mut body);
        assert!(!body.to_string().contains(REDACTION_SECRET));
        assert!(body
            .to_string()
            .contains(lr_secret_scanner::redaction::REDACTION_MESSAGE));
        assert!(state.mcp_gateway.firewall_manager.list_pending().is_empty());
    }
    // Removing the saved entry invalidates the cached verdict immediately.
    let mut client = state.client_manager.get_client("client").unwrap();
    client.secret_scanning.dismissed_secrets.clear();
    client.secret_scanning.action = Some(lr_config::SecretScanAction::Notify);
    state.client_manager.sync_clients(vec![client]);
    let body = chat_body(&format!("aws key {REDACTION_SECRET}"));
    assert!(matches!(
        scan_request_for_secrets(&state, "client", "gpt-5.5", &body).await,
        SecretScanOutcome::Allow
    ));

    // Another client must still see its own policy; no shared redaction.
    let body = chat_body(&format!("aws key {REDACTION_SECRET}"));
    assert!(matches!(
        scan_request_for_secrets(&state, "another-client", "gpt-5.5", &body).await,
        SecretScanOutcome::Allow
    ));
    assert!(state
        .mcp_gateway
        .firewall_manager
        .list_pending()
        .iter()
        .any(|p| p.client_id == "another-client"));
}

#[tokio::test]
async fn redact_once_unblocks_with_plan_and_does_not_create_a_policy() {
    let state = secret_scan_state().await;
    state
        .config_manager
        .update(|cfg| cfg.secret_scanning.action = lr_config::SecretScanAction::Ask)
        .unwrap();
    for _ in 0..2 {
        let scan_state = state.clone();
        let scan = tokio::spawn(async move {
            scan_request_for_secrets(
                &scan_state,
                "client",
                "gpt-5.5",
                &chat_body(&format!("aws key {REDACTION_SECRET}")),
            )
            .await
        });
        let approval = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(pending) = state
                    .mcp_gateway
                    .firewall_manager
                    .list_pending()
                    .into_iter()
                    .find(|p| !p.is_notify_only)
                {
                    break pending;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("approval should appear on each request");
        state
            .mcp_gateway
            .firewall_manager
            .submit_response(
                &approval.request_id,
                lr_mcp::gateway::firewall::FirewallApprovalAction::RedactOnce,
                None,
            )
            .unwrap();
        let SecretScanOutcome::Redact(redaction) = scan.await.unwrap() else {
            panic!("must redact instead of allowing original")
        };
        assert_eq!(
            redaction.apply_text(REDACTION_SECRET),
            lr_secret_scanner::redaction::REDACTION_MESSAGE
        );
        assert!(state
            .config_manager
            .get()
            .clients
            .iter()
            .all(|c| c.secret_scanning.dismissed_secrets.is_empty()));
    }
}

#[tokio::test]
async fn remembered_redaction_does_not_allow_other_secrets() {
    let state = secret_scan_state().await;
    remember_redaction(&state);
    let scan_state = state.clone();
    let scan = tokio::spawn(async move {
        scan_request_for_secrets(
            &scan_state,
            "client",
            "gpt-5.5",
            &chat_body(&format!(
                "aws key {REDACTION_SECRET} other aws key AKIAX9Q7W5E3R1T8Y6U4"
            )),
        )
        .await
    });
    let approval = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(pending) = state
                .mcp_gateway
                .firewall_manager
                .list_pending()
                .into_iter()
                .find(|p| !p.is_notify_only)
            {
                break pending;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(approval
        .secret_scan_details
        .unwrap()
        .findings
        .iter()
        .all(|f| !f.matched_text.contains("PLR6")));
    state
        .mcp_gateway
        .firewall_manager
        .submit_response(
            &approval.request_id,
            lr_mcp::gateway::firewall::FirewallApprovalAction::Deny,
            None,
        )
        .unwrap();
    assert!(matches!(scan.await.unwrap(), SecretScanOutcome::Deny(_)));
}

async fn next_secret_approval(state: &AppState) -> lr_mcp::gateway::firewall::PendingApprovalInfo {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(pending) = state
                .mcp_gateway
                .firewall_manager
                .list_pending()
                .into_iter()
                .find(|p| !p.is_notify_only)
            {
                return pending;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("next secret approval should appear")
}

#[tokio::test]
async fn secrets_are_approved_one_at_a_time_and_denial_stops_the_sequence() {
    use lr_mcp::gateway::firewall::FirewallApprovalAction as Action;
    const SECOND: &str = "AKIAX9Q7W5E3R1T8Y6U4";
    for actions in [
        vec![Action::Deny],
        vec![Action::AllowOnce, Action::Deny],
        vec![Action::RedactOnce, Action::AllowOnce],
        vec![Action::AllowOnce, Action::RedactOnce],
    ] {
        let state = secret_scan_state().await;
        state
            .config_manager
            .update(|cfg| cfg.secret_scanning.action = lr_config::SecretScanAction::Ask)
            .unwrap();
        let body = chat_body(&format!("aws key {REDACTION_SECRET}, second key {SECOND}"));
        let scan_state = state.clone();
        let scan_body = body.clone();
        let scan = tokio::spawn(async move {
            scan_request_for_secrets(&scan_state, "client", "gpt-5.5", &scan_body).await
        });
        let mut redacted_values = Vec::new();
        for action in &actions {
            let approval = next_secret_approval(&state).await;
            assert_eq!(state.mcp_gateway.firewall_manager.list_pending().len(), 1);
            let findings = &approval.secret_scan_details.unwrap().findings;
            assert_eq!(findings.len(), 1, "each popup concerns exactly one secret");
            assert!(
                !scan.is_finished(),
                "the request must remain paused until all secrets have a decision"
            );
            if *action == Action::RedactOnce {
                redacted_values.push(findings[0].raw_text.clone());
            }
            state
                .mcp_gateway
                .firewall_manager
                .submit_response(&approval.request_id, action.clone(), None)
                .unwrap();
        }
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), scan)
            .await
            .unwrap()
            .unwrap();
        if actions.contains(&Action::Deny) {
            assert!(matches!(outcome, SecretScanOutcome::Deny(_)));
        } else {
            let SecretScanOutcome::Redact(plan) = outcome else {
                panic!("expected selective redaction")
            };
            let mut redacted = body.clone();
            plan.apply(&mut redacted);
            let result = redacted.to_string();
            for secret in [REDACTION_SECRET, SECOND] {
                assert_eq!(
                    !result.contains(secret),
                    redacted_values.iter().any(|s| s == secret),
                    "only the selected secret should be redacted"
                );
            }
        }
        assert!(
            state.mcp_gateway.firewall_manager.list_pending().is_empty(),
            "a blocked request must not leave later popups behind"
        );
    }
}

#[tokio::test]
async fn remembered_allow_only_allows_this_specific_secret() {
    let state = secret_scan_state().await;
    remember_redaction(&state);
    let mut client = state.client_manager.get_client("client").unwrap();
    client.secret_scanning.dismissed_secrets[0].redact = false;
    state.client_manager.sync_clients(vec![client]);
    let body = chat_body(&format!("aws key {REDACTION_SECRET}"));
    assert!(matches!(
        scan_request_for_secrets(&state, "client", "gpt-5.5", &body).await,
        SecretScanOutcome::Allow
    ));
    assert!(state.mcp_gateway.firewall_manager.list_pending().is_empty());
    let scan_state = state.clone();
    let scan = tokio::spawn(async move {
        scan_request_for_secrets(
            &scan_state,
            "client",
            "gpt-5.5",
            &chat_body(&format!(
                "aws key {REDACTION_SECRET}, second key AKIAX9Q7W5E3R1T8Y6U4"
            )),
        )
        .await
    });
    let approval = next_secret_approval(&state).await;
    let findings = &approval.secret_scan_details.unwrap().findings;
    assert_eq!(findings.len(), 1);
    assert_ne!(findings[0].raw_text, REDACTION_SECRET);
    state
        .mcp_gateway
        .firewall_manager
        .submit_response(
            &approval.request_id,
            lr_mcp::gateway::firewall::FirewallApprovalAction::Deny,
            None,
        )
        .unwrap();
    assert!(matches!(scan.await.unwrap(), SecretScanOutcome::Deny(_)));
}
