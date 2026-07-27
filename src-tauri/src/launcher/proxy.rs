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
    fn cost_usd(
        &self,
        provider: &str,
        model: &str,
        usage: lr_proxy::interceptor::TokenUsage,
    ) -> Option<f64> {
        let m = lr_catalog::find_model(provider, model)?;
        Some(m.pricing.calculate_cost_with_cache(
            usage.input as u32,
            usage.output as u32,
            usage.cache_read as u32,
            usage.cache_write as u32,
        ))
    }
}

/// Enforces a proxied client's LLM policy using the *same* configuration a
/// gateway client uses: the strategy's Model Permissions (allowed-models list +
/// the Allow/Ask/Off access control) and its rate limits. There is no separate
/// proxy firewall config — what you set on the LLM tab drives interception.
struct AppFirewall {
    config_manager: lr_config::ConfigManager,
    /// The app's shared firewall — the SAME approval system the MCP gateway/LLM
    /// path uses. An "ask" opens the real popup *window* (works from the tray)
    /// and supports editing/rewriting the request, rather than a bespoke dialog.
    firewall: Arc<lr_mcp::gateway::firewall::FirewallManager>,
    /// Time-based model approvals ("Allow 1 Minute"/"Allow 1 Hour" from the
    /// popup), shared with the gateway so a bypass granted there also
    /// suppresses proxy asks — and vice versa.
    model_approvals: Arc<lr_server::state::ModelApprovalTracker>,
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
            if !strategy.is_model_allowed(&req.provider, bare) {
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

        // Ask → the shared firewall approval popup (a real window that works from
        // the tray and lets the user edit the request). Allow → forward unchanged.
        // Grants made from the popup suppress subsequent asks: a still-valid
        // time-based approval ("Allow 1 Minute"/"Allow 1 Hour") or an explicit
        // per-model Allow ("Allow Permanent" writes one into the strategy's
        // model_permissions), same as the gateway path.
        let has_time_bypass = req.model.as_deref().is_some_and(|m| {
            self.model_approvals
                .has_valid_approval(&client.id, &req.provider, m)
        });
        let has_explicit_allow = req.model.as_deref().is_some_and(|m| {
            matches!(
                strategy
                    .model_permissions
                    .models
                    .get(&format!("{}__{m}", req.provider)),
                Some(PermissionState::Allow)
            )
        });
        if permission == PermissionState::Ask && !has_time_bypass && !has_explicit_allow {
            use lr_mcp::gateway::firewall::FirewallApprovalAction as A;
            let resp = self
                .firewall
                .request_model_approval(
                    client.id.clone(),
                    client.name.clone(),
                    req.model.clone().unwrap_or_default(),
                    req.provider.clone(),
                    Some(120),
                    Some(req.body.clone()), // full request → editable in the popup
                    false,                  // not mcp-via-llm
                )
                .await;

            match resp {
                Ok(r) => match r.action {
                    A::AllowOnce
                    | A::AllowSession
                    | A::Allow1Minute
                    | A::Allow1Hour
                    | A::AllowPermanent
                    | A::AllowCategories => {
                        // If the user edited the request in the popup, send the
                        // rewritten body upstream instead of the original.
                        if let Some(edited) = r.edited_arguments {
                            if let Ok(bytes) = serde_json::to_vec(&edited) {
                                return RequestAction::Replace(bytes);
                            }
                        }
                    }
                    // Any deny/disable action blocks the request.
                    _ => return RequestAction::reject_json(403, "Denied by user"),
                },
                // Timeout / channel error → fail closed.
                Err(_) => return RequestAction::reject_json(403, "Approval timed out"),
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
        firewall_manager: Arc<lr_mcp::gateway::firewall::FirewallManager>,
        model_approvals: Arc<lr_server::state::ModelApprovalTracker>,
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
            firewall: firewall_manager,
            model_approvals,
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
