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

/// Enforces a proxied client's LLM policy using the *same* configuration a
/// gateway client uses: the strategy's Model Permissions (allowed-models list +
/// the Allow/Ask/Off access control) and its rate limits. There is no separate
/// proxy firewall config — what you set on the LLM tab drives interception.
struct AppFirewall {
    config_manager: lr_config::ConfigManager,
    approval: Arc<ProxyApprovalManager>,
    /// Metrics-based rate-limit backend, shared with the gateway path. Proxied
    /// exchanges are already recorded here (keyed by strategy), so the rolling
    /// windows include proxy traffic.
    metrics: Arc<lr_monitoring::metrics::MetricsCollector>,
}

/// Replicates the gateway's metrics-based strategy rate-limit check
/// ([`lr_router`]'s `check_strategy_rate_limits`): projected usage over the
/// rolling window must not exceed any enabled limit. Returns the exceeded limit
/// type (for the client-facing message) when the request should be blocked.
fn rate_limit_exceeded(
    metrics: &lr_monitoring::metrics::MetricsCollector,
    strategy: &lr_config::Strategy,
) -> Option<&'static str> {
    if strategy.rate_limits.is_empty() {
        return None;
    }
    let (avg_tokens, avg_cost) = metrics.get_pre_estimate_for_strategy(&strategy.id, 10);
    for limit in &strategy.rate_limits {
        let window_secs = limit.time_window.to_seconds();
        let (requests, tokens, cost) =
            metrics.get_recent_usage_for_strategy(&strategy.id, window_secs);
        if let Some(kind) = limit_exceeded(limit, requests, tokens, cost, avg_tokens, avg_cost) {
            return Some(kind);
        }
    }
    None
}

/// The pure projection decision for one limit: would this next request push the
/// rolling window over `limit.value`? Split out from the metrics fetch so it is
/// unit-testable without a live [`MetricsCollector`].
fn limit_exceeded(
    limit: &lr_config::StrategyRateLimit,
    current_requests: u64,
    current_tokens: u64,
    current_cost: f64,
    avg_tokens: f64,
    avg_cost: f64,
) -> Option<&'static str> {
    if !limit.enabled {
        return None;
    }
    let (projected, label) = match limit.limit_type {
        lr_config::RateLimitType::Requests => (current_requests as f64 + 1.0, "requests"),
        lr_config::RateLimitType::TotalTokens => (current_tokens as f64 + avg_tokens, "tokens"),
        lr_config::RateLimitType::Cost => {
            // Free models (avg_cost == 0) don't count against a cost limit.
            if avg_cost == 0.0 {
                return None;
            }
            (current_cost + avg_cost, "cost")
        }
        // Input/Output token limits aren't pre-checkable.
        _ => return None,
    };
    (projected > limit.value).then_some(label)
}

#[async_trait::async_trait]
impl lr_proxy::active::Firewall for AppFirewall {
    async fn evaluate(
        &self,
        req: &lr_proxy::active::FirewallRequest,
    ) -> lr_proxy::interceptor::RequestAction {
        use lr_config::PermissionState;
        use lr_proxy::interceptor::RequestAction;

        let config = self.config_manager.get();
        let Some(client) = config.clients.iter().find(|c| c.id == req.client_id) else {
            return RequestAction::Forward;
        };
        let Some(strategy) = config
            .strategies
            .iter()
            .find(|s| s.id == client.strategy_id)
        else {
            return RequestAction::Forward;
        };

        // Access control (the Model Permissions button): Off blocks everything.
        let permission = strategy
            .auto_config
            .as_ref()
            .map(|a| a.permission.clone())
            .unwrap_or(PermissionState::Allow);
        if permission == PermissionState::Off {
            return RequestAction::reject_json(403, "LLM access is disabled for this client");
        }

        // Allowed-models list: deny a model that isn't permitted. (Allow-all
        // strategies permit every model, so this never fires.)
        if let Some(model) = &req.model {
            let bare = model.rsplit('/').next().unwrap_or(model);
            if !strategy.is_model_allowed("anthropic", bare) {
                return RequestAction::reject_json(
                    403,
                    &format!("Model '{model}' is not permitted for this client"),
                );
            }
        }

        // Rate limits (shared with the gateway).
        if let Some(kind) = rate_limit_exceeded(&self.metrics, strategy) {
            return RequestAction::reject_json(429, &format!("Rate limit exceeded ({kind})"));
        }

        // Ask → interactive approval popup; Allow → forward unchanged.
        if permission == PermissionState::Ask {
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
            if !self.approval.request(payload).await {
                return RequestAction::reject_json(403, "Denied by user");
            }
        }

        RequestAction::Forward
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
            .with_metrics(metrics_collector.clone())
            .with_pricing(Arc::new(CatalogPricing));
        let firewall = Arc::new(AppFirewall {
            config_manager,
            approval,
            metrics: metrics_collector,
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

    fn limit(
        limit_type: lr_config::RateLimitType,
        value: f64,
        enabled: bool,
    ) -> lr_config::StrategyRateLimit {
        lr_config::StrategyRateLimit {
            limit_type,
            value,
            time_window: lr_config::RateLimitTimeWindow::Minute,
            enabled,
        }
    }

    #[test]
    fn request_limit_trips_when_projected_over() {
        use lr_config::RateLimitType::Requests;
        // 5 requests already this window, limit is 5 → the 6th (projected) is over.
        assert_eq!(
            limit_exceeded(&limit(Requests, 5.0, true), 5, 0, 0.0, 0.0, 0.0),
            Some("requests")
        );
        // 4 used, limit 5 → projected 5, not over.
        assert!(limit_exceeded(&limit(Requests, 5.0, true), 4, 0, 0.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn disabled_limit_never_trips() {
        use lr_config::RateLimitType::Requests;
        assert!(limit_exceeded(&limit(Requests, 0.0, false), 999, 0, 0.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn token_and_cost_limits_project_from_averages() {
        use lr_config::RateLimitType::{Cost, TotalTokens};
        // 900 tokens used + 200 avg = 1100 projected > 1000 limit.
        assert_eq!(
            limit_exceeded(&limit(TotalTokens, 1000.0, true), 0, 900, 0.0, 200.0, 0.0),
            Some("tokens")
        );
        // Cost limit with a free model (avg_cost 0) never counts.
        assert!(limit_exceeded(&limit(Cost, 1.0, true), 0, 0, 5.0, 0.0, 0.0).is_none());
        // Cost limit with non-free average that exceeds.
        assert_eq!(
            limit_exceeded(&limit(Cost, 1.0, true), 0, 0, 0.9, 0.0, 0.2),
            Some("cost")
        );
    }
}
