//! App-side wiring for the HTTPS inspection proxy.
//!
//! Owns the [`lr_proxy::ProxyManager`] lifecycle, resolves proxied connections
//! against the real client manager, and exposes the connection details
//! (proxy URL + root CA path) that clients need to configure their tools.

use std::path::PathBuf;
use std::sync::Arc;

use lr_proxy::cert::CertAuthority;
use lr_proxy::interceptor::ClientCtx;
use lr_proxy::passive::PassiveInterceptor;
use lr_proxy::resolver::ClientResolver;
use lr_proxy::ProxyManager;
use lr_types::{AppError, AppResult};
use parking_lot::Mutex;

/// Resolves proxy Basic-auth credentials against the client manager, and marks
/// whether the client is actually in a proxy `llm_mode`.
struct AppClientResolver {
    client_manager: Arc<lr_clients::ClientManager>,
}

impl ClientResolver for AppClientResolver {
    fn resolve(&self, client_id: &str, secret: &str) -> Option<ClientCtx> {
        let client = self.client_manager.verify_secret(secret).ok().flatten()?;
        // The username must match the verified client (defense in depth).
        if client.id != client_id {
            return None;
        }
        Some(ClientCtx {
            client_id: client.id.clone(),
            strategy_id: client.strategy_id.clone(),
            proxy_enabled: client.llm_proxy_enabled(),
        })
    }
}

/// Prices proxied Anthropic calls from the model catalog (sync, static lookup).
struct CatalogPricing;

impl lr_proxy::interceptor::PricingResolver for CatalogPricing {
    fn cost_usd(&self, model: &str, usage: lr_proxy::interceptor::TokenUsage) -> Option<f64> {
        let m = lr_catalog::find_model("anthropic", model)?;
        Some(m.pricing.calculate_cost_with_cache(
            usage.input as u32,
            usage.output as u32,
            usage.cache_read as u32,
            usage.cache_write as u32,
        ))
    }
}

/// How long we wait for a user's firewall decision before defaulting to deny.
const APPROVAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Payload sent to the UI when a request needs interactive approval ("ask").
#[derive(serde::Serialize, Clone)]
pub struct FirewallApprovalRequest {
    pub request_id: String,
    pub client_id: String,
    pub client_name: String,
    pub model: Option<String>,
    pub has_tools: bool,
    pub message_count: usize,
    /// Short preview of the request for the popup.
    pub preview: String,
}

/// Manages interactive firewall approvals: emits an event to the UI and awaits
/// the user's decision (with a timeout that defaults to deny).
pub struct ProxyApprovalManager {
    pending:
        parking_lot::Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
    app: parking_lot::RwLock<Option<tauri::AppHandle>>,
}

impl Default for ProxyApprovalManager {
    fn default() -> Self {
        Self {
            pending: parking_lot::Mutex::new(std::collections::HashMap::new()),
            app: parking_lot::RwLock::new(None),
        }
    }
}

impl ProxyApprovalManager {
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        *self.app.write() = Some(handle);
    }

    /// The user (or the UI) answers a pending approval.
    pub fn respond(&self, request_id: &str, allow: bool) {
        if let Some(tx) = self.pending.lock().remove(request_id) {
            let _ = tx.send(allow);
        }
    }

    /// Ask the UI to approve a request; returns true to allow, false to deny.
    async fn request(&self, mut payload: FirewallApprovalRequest) -> bool {
        use tauri::Emitter;
        let Some(app) = self.app.read().clone() else {
            // No UI wired — fail closed.
            return false;
        };
        let request_id = uuid::Uuid::new_v4().to_string();
        payload.request_id = request_id.clone();

        let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
        self.pending.lock().insert(request_id.clone(), tx);

        if app.emit("proxy-firewall-ask", &payload).is_err() {
            self.pending.lock().remove(&request_id);
            return false;
        }

        match tokio::time::timeout(APPROVAL_TIMEOUT, rx).await {
            Ok(Ok(allow)) => allow,
            _ => {
                self.pending.lock().remove(&request_id);
                false
            }
        }
    }
}

/// The app firewall: config-driven rules, model enforcement, model rewrites, and
/// interactive approval. Implements the proxy crate's `Firewall` trait.
struct AppFirewall {
    config_manager: lr_config::ConfigManager,
    approval: Arc<ProxyApprovalManager>,
}

fn rule_matches(m: &lr_config::FirewallRuleMatch, req: &lr_proxy::active::FirewallRequest) -> bool {
    if let Some(sub) = &m.model_contains {
        let model = req.model.as_deref().unwrap_or("");
        if !model
            .to_ascii_lowercase()
            .contains(&sub.to_ascii_lowercase())
        {
            return false;
        }
    }
    if let Some(want) = m.has_tools {
        if req.has_tools != want {
            return false;
        }
    }
    if let Some(sub) = &m.content_contains {
        let text = req.body.to_string().to_ascii_lowercase();
        if !text.contains(&sub.to_ascii_lowercase()) {
            return false;
        }
    }
    true
}

/// Apply forced model rewrites; returns a `Replace` with the mutated body, or
/// `Forward` if nothing changed.
fn apply_rewrites(
    policy: &lr_config::LlmProxyPolicy,
    req: &lr_proxy::active::FirewallRequest,
) -> lr_proxy::interceptor::RequestAction {
    use lr_proxy::interceptor::RequestAction;
    let Some(model) = &req.model else {
        return RequestAction::Forward;
    };
    let Some(rw) = policy.model_rewrites.iter().find(|r| &r.from == model) else {
        return RequestAction::Forward;
    };
    let mut body = req.body.clone();
    if let Some(obj) = body.as_object_mut() {
        obj.insert("model".to_string(), rw.to.clone().into());
    }
    match serde_json::to_vec(&body) {
        Ok(bytes) => RequestAction::Replace(bytes),
        Err(_) => RequestAction::Forward,
    }
}

#[async_trait::async_trait]
impl lr_proxy::active::Firewall for AppFirewall {
    async fn evaluate(
        &self,
        req: &lr_proxy::active::FirewallRequest,
    ) -> lr_proxy::interceptor::RequestAction {
        use lr_config::FirewallAction;
        use lr_proxy::interceptor::RequestAction;

        let config = self.config_manager.get();
        let Some(client) = config.clients.iter().find(|c| c.id == req.client_id) else {
            return RequestAction::Forward;
        };
        let policy = &client.llm_proxy;

        // Model allow-list enforcement (deny disallowed models).
        if policy.enforce_model_permissions {
            if let Some(model) = &req.model {
                let bare = model.rsplit('/').next().unwrap_or(model);
                let allowed = config
                    .strategies
                    .iter()
                    .find(|s| s.id == client.strategy_id)
                    .map(|s| s.is_model_allowed("anthropic", bare))
                    .unwrap_or(true);
                if !allowed {
                    return RequestAction::reject_json(
                        403,
                        &format!("Model '{model}' is not permitted for this client"),
                    );
                }
            }
        }

        // First enabled matching rule wins; else the default action.
        let action = policy
            .rules
            .iter()
            .find(|r| r.enabled && rule_matches(&r.matcher, req))
            .map(|r| r.action)
            .unwrap_or(policy.default_action);

        match action {
            FirewallAction::Deny => {
                RequestAction::reject_json(403, "Blocked by the LocalRouter firewall")
            }
            FirewallAction::Ask => {
                let preview = req
                    .body
                    .get("messages")
                    .and_then(|m| m.as_array())
                    .and_then(|a| a.last())
                    .map(|m| m.to_string())
                    .unwrap_or_default();
                let payload = FirewallApprovalRequest {
                    request_id: String::new(),
                    client_id: req.client_id.clone(),
                    client_name: client.name.clone(),
                    model: req.model.clone(),
                    has_tools: req.has_tools,
                    message_count: req.message_count,
                    preview: preview.chars().take(500).collect(),
                };
                if self.approval.request(payload).await {
                    apply_rewrites(policy, req)
                } else {
                    RequestAction::reject_json(403, "Denied by user")
                }
            }
            FirewallAction::Allow => apply_rewrites(policy, req),
        }
    }
}

/// The running-or-idle proxy service.
pub struct ProxyService {
    ca: Arc<CertAuthority>,
    host: String,
    interceptor: Arc<dyn lr_proxy::interceptor::ProxyInterceptor>,
    resolver: Arc<AppClientResolver>,
    running: Mutex<Option<RunningProxy>>,
}

struct RunningProxy {
    port: u16,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

impl ProxyService {
    /// Build the service (generates/loads the root CA), without starting it.
    pub fn new(
        monitor_store: Arc<lr_monitor::MonitorEventStore>,
        metrics_collector: Arc<lr_monitoring::metrics::MetricsCollector>,
        client_manager: Arc<lr_clients::ClientManager>,
        config_manager: lr_config::ConfigManager,
        approval: Arc<ProxyApprovalManager>,
        host: String,
    ) -> AppResult<Self> {
        let dir = lr_utils::paths::config_dir()?.join("proxy");
        let ca = Arc::new(
            CertAuthority::load_or_create(&dir)
                .map_err(|e| AppError::Internal(format!("proxy CA: {e}")))?,
        );
        // The recorder half (monitor + metrics + cost), reused by the active
        // interceptor which adds the firewall on top.
        let recorder = PassiveInterceptor::new(monitor_store)
            .with_metrics(metrics_collector)
            .with_pricing(Arc::new(CatalogPricing));
        let firewall = Arc::new(AppFirewall {
            config_manager,
            approval,
        });
        let interceptor = lr_proxy::active::ActiveInterceptor::new(recorder, firewall);
        Ok(Self {
            ca,
            host,
            interceptor: Arc::new(interceptor),
            resolver: Arc::new(AppClientResolver { client_manager }),
            running: Mutex::new(None),
        })
    }

    /// Path to the root CA clients must trust (`NODE_EXTRA_CA_CERTS`).
    pub fn ca_cert_path(&self) -> PathBuf {
        self.ca.ca_cert_path().to_path_buf()
    }

    /// The bound port, if the proxy is currently running.
    pub fn port(&self) -> Option<u16> {
        self.running.lock().as_ref().map(|r| r.port)
    }

    pub fn is_running(&self) -> bool {
        self.running.lock().is_some()
    }

    /// Start the listener on `port` (0 = OS-assigned). Idempotent: a no-op if
    /// already running. Returns the bound port.
    pub async fn start(&self, port: u16) -> AppResult<u16> {
        if let Some(r) = self.running.lock().as_ref() {
            return Ok(r.port);
        }

        let manager = ProxyManager::new(
            self.ca.clone(),
            self.interceptor.clone(),
            self.resolver.clone(),
        )
        .map_err(|e| AppError::Internal(format!("proxy manager: {e}")))?;

        let listener = ProxyManager::bind(&self.host, port)
            .await
            .map_err(|e| AppError::Internal(format!("proxy bind {}:{port}: {e}", self.host)))?;
        let bound = listener
            .local_addr()
            .map_err(|e| AppError::Internal(format!("proxy local_addr: {e}")))?
            .port();

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            manager
                .serve(listener, async {
                    let _ = rx.await;
                })
                .await;
        });

        *self.running.lock() = Some(RunningProxy {
            port: bound,
            shutdown: tx,
        });
        tracing::info!("HTTPS inspection proxy listening on {}:{bound}", self.host);
        Ok(bound)
    }

    /// Stop the listener if running.
    // TODO(https-proxy): called on config-driven restart / app teardown (follow-up).
    #[allow(dead_code)]
    pub fn stop(&self) {
        if let Some(r) = self.running.lock().take() {
            let _ = r.shutdown.send(());
            tracing::info!("HTTPS inspection proxy stopped");
        }
    }

    /// The `HTTPS_PROXY` URL for a client (embeds Basic auth).
    pub fn client_proxy_url(&self, client_id: &str, secret: &str) -> Option<String> {
        self.port()
            .map(|port| proxy_url(&self.host, port, client_id, secret))
    }
}

/// Build the `http://<client_id>:<secret>@host:port` proxy URL.
pub fn proxy_url(host: &str, port: u16, client_id: &str, secret: &str) -> String {
    format!("http://{client_id}:{secret}@{host}:{port}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_proxy_url_with_basic_auth() {
        assert_eq!(
            proxy_url("127.0.0.1", 3626, "cid", "lr-secret"),
            "http://cid:lr-secret@127.0.0.1:3626"
        );
    }

    fn fw_req(
        model: &str,
        has_tools: bool,
        body: serde_json::Value,
    ) -> lr_proxy::active::FirewallRequest {
        lr_proxy::active::FirewallRequest {
            client_id: "c".into(),
            host: "api.anthropic.com".into(),
            path: "/v1/messages".into(),
            model: Some(model.into()),
            has_tools,
            message_count: 1,
            body,
        }
    }

    #[test]
    fn rule_matches_model_tools_and_content() {
        use lr_config::FirewallRuleMatch;
        let body =
            serde_json::json!({"model": "claude-opus", "messages": [{"content": "delete prod"}]});
        let req = fw_req("claude-opus", true, body);

        // Model substring (case-insensitive).
        assert!(rule_matches(
            &FirewallRuleMatch {
                model_contains: Some("OPUS".into()),
                ..Default::default()
            },
            &req
        ));
        assert!(!rule_matches(
            &FirewallRuleMatch {
                model_contains: Some("sonnet".into()),
                ..Default::default()
            },
            &req
        ));
        // has_tools.
        assert!(rule_matches(
            &FirewallRuleMatch {
                has_tools: Some(true),
                ..Default::default()
            },
            &req
        ));
        assert!(!rule_matches(
            &FirewallRuleMatch {
                has_tools: Some(false),
                ..Default::default()
            },
            &req
        ));
        // Content substring.
        assert!(rule_matches(
            &FirewallRuleMatch {
                content_contains: Some("delete prod".into()),
                ..Default::default()
            },
            &req
        ));
    }

    #[test]
    fn apply_rewrites_maps_model() {
        use lr_config::{LlmProxyPolicy, ModelRewrite};
        use lr_proxy::interceptor::RequestAction;
        let policy = LlmProxyPolicy {
            model_rewrites: vec![ModelRewrite {
                from: "claude-opus".into(),
                to: "claude-sonnet".into(),
            }],
            ..Default::default()
        };
        let req = fw_req(
            "claude-opus",
            false,
            serde_json::json!({"model": "claude-opus"}),
        );
        match apply_rewrites(&policy, &req) {
            RequestAction::Replace(bytes) => {
                let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(v["model"], "claude-sonnet");
            }
            _ => panic!("expected Replace"),
        }

        // No matching rewrite → Forward.
        let req2 = fw_req(
            "claude-haiku",
            false,
            serde_json::json!({"model": "claude-haiku"}),
        );
        assert!(matches!(
            apply_rewrites(&policy, &req2),
            RequestAction::Forward
        ));
    }
}
