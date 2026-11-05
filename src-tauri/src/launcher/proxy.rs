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

/// Resolves client display names for proxy-recorded monitor events, so the
/// Monitor shows "Claude Code" rather than the client UUID.
pub(crate) struct AppClientNames {
    pub(crate) client_manager: Arc<lr_clients::ClientManager>,
}

impl lr_proxy::interceptor::ClientNameResolver for AppClientNames {
    fn name_for(&self, client_id: &str) -> Option<String> {
        self.client_manager.get_client(client_id).map(|c| c.name)
    }
}

/// Prices proxied Anthropic calls from the model catalog (sync, static lookup).
pub(crate) struct CatalogPricing;

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
pub(crate) struct AppFirewall {
    pub(crate) config_manager: lr_config::ConfigManager,
    /// The app's shared firewall — the SAME approval system the MCP gateway/LLM
    /// path uses. An "ask" opens the real popup *window* (works from the tray)
    /// and supports editing/rewriting the request, rather than a bespoke dialog.
    pub(crate) firewall: Arc<lr_mcp::gateway::firewall::FirewallManager>,
    /// Time-based model approvals ("Allow 1 Minute"/"Allow 1 Hour" from the
    /// popup), shared with the gateway so a bypass granted there also
    /// suppresses proxy asks — and vice versa.
    pub(crate) model_approvals: Arc<lr_server::state::ModelApprovalTracker>,
    /// Metrics-based rate-limit backend, shared with the gateway path. Proxied
    /// exchanges are already recorded here (keyed by strategy), so the rolling
    /// windows include proxy traffic.
    pub(crate) metrics: Arc<lr_monitoring::metrics::MetricsCollector>,
    /// Server state, used to run the *same* secret scan the gateway runs
    /// (scanner, per-client action, monitor events, approval popup).
    pub(crate) state: lr_server::state::AppState,
    pub(crate) secret_storage: Arc<dyn lr_api_keys::KeychainStorage>,
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
    fn protects_secrets(&self, client_id: &str) -> bool {
        let config = self.config_manager.get();
        let action = config
            .clients
            .iter()
            .find(|c| c.id == client_id)
            .and_then(|c| c.secret_scanning.action.as_ref())
            .unwrap_or(&config.secret_scanning.action);
        matches!(
            action,
            lr_config::SecretScanAction::Redact | lr_config::SecretScanAction::Ask
        )
    }
    async fn evaluate(
        &self,
        req: &lr_proxy::active::FirewallRequest,
    ) -> lr_proxy::interceptor::RequestAction {
        use lr_proxy::interceptor::RequestAction;
        use lr_secret_scanner::placeholders::Vault;
        use lr_types::secret_context::{self, SecretContext};
        let config = self.config_manager.get();
        let client = config.clients.iter().find(|c| c.id == req.client_id);
        let action = client
            .and_then(|c| c.secret_scanning.action.as_ref())
            .unwrap_or(&config.secret_scanning.action);
        if !matches!(
            action,
            lr_config::SecretScanAction::Redact | lr_config::SecretScanAction::Ask
        ) {
            return self.evaluate_policy(req).await;
        }
        let retrieval = req.method == "GET" && lr_proxy::wire::response_id(&req.path).is_some();
        if !req.body.is_object() && !retrieval {
            return RequestAction::reject_json(400, "Invalid protected request JSON");
        }
        let storage = self.secret_storage.clone();
        let mut session_options = req.body.pointer("/extensions/localrouter.session").cloned();
        if let Some(header) = &req.secret_session {
            let options = session_options.get_or_insert_with(|| serde_json::json!({}));
            let Some(options) = options.as_object_mut() else {
                return RequestAction::reject_json(400, "Invalid secret session options");
            };
            options.insert("id".into(), serde_json::Value::String(header.clone()));
        }
        let session = match lr_providers::session::SessionOptions::parse(session_options.as_ref()) {
            Ok(options) => options.map(|options| options.id),
            Err(_) => return RequestAction::reject_json(400, "Invalid secret session identity"),
        };
        let fallback = uuid::Uuid::new_v4().to_string();
        let previous = req
            .body
            .get("previous_response_id")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                if retrieval {
                    lr_proxy::wire::response_id(&req.path)
                } else {
                    None
                }
            });
        let vault = if let Some(previous) = previous {
            match Vault::continue_response(storage, &req.client_id, previous) {
                Ok(vault) => vault,
                Err(_) => {
                    return RequestAction::reject_json(
                        400,
                        "Secret scope for previous_response_id is missing or expired",
                    )
                }
            }
        } else {
            Vault::new(
                storage,
                &req.client_id,
                session.as_deref().unwrap_or(&fallback),
                session.is_none() && !req.path.contains("responses"),
            )
        };
        let vault = Arc::new(vault);
        if req
            .body
            .pointer("/extensions/localrouter.session/reset")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
            && vault.forget().is_err()
        {
            return RequestAction::reject_json(400, "Could not reset secret scope");
        }
        let context: Arc<dyn SecretContext> = vault.clone();
        secret_context::scope(Some(context.clone()), async {
            let action = self.evaluate_policy(req).await;
            let mut body = match action {
                RequestAction::Forward => req.body.clone(),
                RequestAction::Replace(bytes) => match serde_json::from_slice(&bytes) {
                    Ok(body) => body,
                    Err(_) => {
                        return RequestAction::reject_json(400, "Invalid edited request JSON")
                    }
                },
                other => return other,
            };
            if lr_server::documents::protect_text_files(&mut body, context.as_ref()).is_err()
                || context.protect(&mut body).is_err()
            {
                return RequestAction::reject_json(
                    403,
                    "Could not protect request secrets in OS credential storage",
                );
            }
            // Hosted tools receive masked inputs. The contract checks local
            // function schemas and compatibility with declared text grammars.
            if let Err(error) = vault.set_request_contract(&body) {
                return RequestAction::reject_json(400, &error.to_string());
            }
            if let Some(extensions) = body
                .get_mut("extensions")
                .and_then(serde_json::Value::as_object_mut)
            {
                extensions.remove("localrouter.session");
                if extensions.is_empty() {
                    body.as_object_mut()
                        .expect("request object")
                        .remove("extensions");
                }
            }
            let encoded = if retrieval {
                Ok(Vec::new())
            } else {
                serde_json::to_vec(&body)
            };
            match encoded {
                Ok(body) => RequestAction::Protect { body, vault },
                Err(_) => RequestAction::reject_json(400, "Could not encode protected request"),
            }
        })
        .await
    }
}

impl AppFirewall {
    async fn evaluate_policy(
        &self,
        req: &lr_proxy::active::FirewallRequest,
    ) -> lr_proxy::interceptor::RequestAction {
        use lr_config::PermissionState;
        use lr_proxy::interceptor::RequestAction;

        let config = self.config_manager.get();
        let Some(client) = config.clients.iter().find(|c| c.id == req.client_id) else {
            return RequestAction::reject_json(403, "Client policy is no longer available");
        };
        let Some(strategy) = config
            .strategies
            .iter()
            .find(|s| s.id == client.strategy_id)
        else {
            return RequestAction::reject_json(403, "Client strategy is no longer available");
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

        // Outbound secret scan — the same check, config, monitor events, and
        // approval popup as the gateway path. Runs before the model-approval
        // ask below so a leaked secret is still caught when the model itself is
        // already approved. `req.body` is the provider-native request (Anthropic
        // Messages or OpenAI Responses, including a websocket `response.create`
        // message); the shared text extractor understands all of them.
        let mut body = req.body.clone();
        let (projection, _) = match lr_server::documents::scan_projection(&body) {
            Ok(projection) => projection,
            Err(_) => {
                return RequestAction::reject_json(
                    400,
                    "Could not inspect protected request documents",
                )
            }
        };
        let redacted = match lr_server::routes::pipeline::scan_request_for_secrets_with_source(
            &self.state,
            &req.client_id,
            req.model.as_deref().unwrap_or_default(),
            &projection,
            Some(&req.provider),
            req.session_id.as_deref(),
        )
        .await
        {
            lr_server::routes::pipeline::SecretScanOutcome::Deny(message) => {
                return RequestAction::reject_json(403, &message)
            }
            lr_server::routes::pipeline::SecretScanOutcome::Redact(redaction) => {
                if lr_server::documents::redact_text_files(&mut body, &redaction).is_err() {
                    return RequestAction::reject_json(403, "Could not redact document secrets");
                }
                redaction.apply(&mut body);
                Some(redaction)
            }
            lr_server::routes::pipeline::SecretScanOutcome::Allow => None,
        };

        if let Some(context) = lr_types::secret_context::current() {
            if lr_server::documents::protect_text_files(&mut body, context.as_ref()).is_err()
                || context.protect(&mut body).is_err()
            {
                return RequestAction::reject_json(403, "Could not protect outbound secrets");
            }
        }

        // GuardRails — same safety models, category actions, monitor events and
        // approval popup as the gateway path, so a client's configured
        // guardrails apply whether it connects via the gateway or the proxy.
        // Skips itself when the resolved policy allows every category.
        if let lr_server::routes::pipeline::GuardrailScanOutcome::Deny(message) =
            lr_server::routes::pipeline::scan_request_for_guardrails(
                &self.state,
                &req.client_id,
                req.model.as_deref().unwrap_or_default(),
                &body,
                req.session_id.as_deref(),
            )
            .await
        {
            return RequestAction::reject_json(403, &message);
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
                    Some(body.clone()), // full request → editable in the popup
                    false,              // not mcp-via-llm
                )
                .await;

            if let Ok(response) = &resp {
                lr_server::routes::monitor_helpers::emit_firewall_decision(
                    &self.state,
                    Some(&lr_server::middleware::client_auth::ClientAuthContext {
                        client_id: req.client_id.clone(),
                    }),
                    req.session_id.as_deref(),
                    "model",
                    req.model.as_deref().unwrap_or_default(),
                    &format!("{:?}", response.action),
                    None,
                );
            }
            match resp {
                Ok(r) => {
                    match r.action {
                        A::AllowOnce
                        | A::AllowSession
                        | A::Allow1Minute
                        | A::Allow1Hour
                        | A::AllowPermanent
                        | A::AllowCategories => {
                            // If the user edited the request in the popup, send the
                            // rewritten body upstream instead of the original.
                            if let Some(mut edited) = r.edited_arguments {
                                if let Some(redaction) = &redacted {
                                    if lr_server::documents::redact_text_files(
                                        &mut edited,
                                        redaction,
                                    )
                                    .is_err()
                                    {
                                        return RequestAction::reject_json(
                                            403,
                                            "Could not redact edited document secrets",
                                        );
                                    }
                                    redaction.apply(&mut edited);
                                }
                                let (projection, _) =
                                    match lr_server::documents::scan_projection(&edited) {
                                        Ok(value) => value,
                                        Err(_) => {
                                            return RequestAction::reject_json(
                                                400,
                                                "Could not inspect edited request",
                                            )
                                        }
                                    };
                                match lr_server::routes::pipeline::scan_request_for_secrets_with_source(&self.state, &req.client_id, req.model.as_deref().unwrap_or_default(), &projection, Some(&req.provider), req.session_id.as_deref()).await {
                                lr_server::routes::pipeline::SecretScanOutcome::Deny(message) => return RequestAction::reject_json(403, &message),
                                lr_server::routes::pipeline::SecretScanOutcome::Redact(redaction) => {
                                    if lr_server::documents::redact_text_files(&mut edited, &redaction).is_err() { return RequestAction::reject_json(403, "Could not redact edited document secrets"); }
                                    redaction.apply(&mut edited);
                                },
                                _ => {},
                            }
                                if let Ok(bytes) = serde_json::to_vec(&edited) {
                                    return RequestAction::Replace(bytes);
                                }
                            }
                        }
                        // Any deny/disable action blocks the request.
                        _ => return RequestAction::reject_json(403, "Denied by user"),
                    }
                }
                // Timeout / channel error → fail closed.
                Err(_) => return RequestAction::reject_json(403, "Approval timed out"),
            }
        }

        if redacted.is_some() || body != req.body {
            match serde_json::to_vec(&body) {
                Ok(bytes) => RequestAction::Replace(bytes),
                Err(_) => RequestAction::reject_json(403, "Could not serialize redacted request"),
            }
        } else {
            RequestAction::Forward
        }
    }
}

/// Live "duplicate request detection" switch (`request_dedupe.enabled`),
/// shared by the HTTPS proxy and reverse-proxy listeners so the setting takes
/// effect immediately without rebinding them. The gateway reads the config
/// directly.
#[derive(Clone)]
pub struct RequestDedupeFlag(pub Arc<std::sync::atomic::AtomicBool>);

impl RequestDedupeFlag {
    pub fn new(enabled: bool) -> Self {
        Self(Arc::new(std::sync::atomic::AtomicBool::new(enabled)))
    }

    pub fn set(&self, enabled: bool) {
        self.0.store(enabled, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The running-or-idle proxy service.
pub struct ProxyService {
    ca: Arc<CertAuthority>,
    host: String,
    interceptor: Arc<dyn lr_proxy::interceptor::ProxyInterceptor>,
    resolver: Arc<AppClientResolver>,
    dedupe: RequestDedupeFlag,
    usage: Arc<lr_usage::UsageTracker>,
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
        state: lr_server::state::AppState,
        host: String,
        dedupe: RequestDedupeFlag,
        usage: Arc<lr_usage::UsageTracker>,
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
            .with_pricing(Arc::new(CatalogPricing))
            .with_usage(usage.clone())
            .with_client_names(Arc::new(AppClientNames {
                client_manager: client_manager.clone(),
            }));
        let firewall = Arc::new(AppFirewall {
            secret_storage: Arc::new(lr_api_keys::keychain_trait::SystemKeychain),
            config_manager,
            firewall: firewall_manager,
            model_approvals,
            metrics: metrics_collector,
            state,
        });
        let interceptor = lr_proxy::active::ActiveInterceptor::new(recorder, firewall);
        Ok(Self {
            ca,
            host,
            interceptor: Arc::new(interceptor),
            resolver: Arc::new(AppClientResolver { client_manager }),
            dedupe,
            usage,
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
        .map_err(|e| AppError::Internal(format!("proxy manager: {e}")))?
        .with_dedupe_flag(self.dedupe.0.clone())
        .with_usage_tracker(self.usage.clone());

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

#[cfg(test)]
mod secret_policy_tests {
    use super::*;
    use lr_proxy::{
        active::{Firewall, FirewallRequest},
        interceptor::RequestAction,
    };
    use serde_json::{json, Value};
    const KEY: &str = "6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQr";
    fn fixture(
        root: &std::path::Path,
        mode: lr_config::LlmMode,
        storage: Arc<dyn lr_api_keys::KeychainStorage>,
    ) -> (AppFirewall, FirewallRequest) {
        let mut config = lr_config::AppConfig::default();
        config.secret_scanning.action = lr_config::SecretScanAction::Redact;
        let strategy = lr_config::Strategy::new("Synthetic".into());
        let mut client =
            lr_config::Client::new_with_strategy("Synthetic".into(), strategy.id.clone());
        client.llm_mode = mode;
        config.strategies = vec![strategy];
        config.clients = vec![client.clone()];
        let manager = Arc::new(lr_config::ConfigManager::new(
            config,
            root.join("settings.yaml"),
        ));
        let providers = Arc::new(lr_providers::registry::ProviderRegistry::new());
        let limits = Arc::new(lr_router::RateLimiterManager::new(None));
        let metrics = Arc::new(lr_monitoring::metrics::MetricsCollector::new(Arc::new(
            lr_monitoring::storage::MetricsDatabase::new(root.join("metrics.db")).unwrap(),
        )));
        let router = Arc::new(lr_router::Router::new(
            manager.clone(),
            providers.clone(),
            limits.clone(),
            metrics.clone(),
            Arc::new(lr_router::FreeTierManager::new(None)),
        ));
        let clients = Arc::new(lr_clients::ClientManager::with_keychain(
            vec![client.clone()],
            Arc::new(lr_api_keys::MockKeychain::new()),
        ));
        let state = lr_server::state::AppState::new(
            router,
            limits,
            providers,
            manager.clone(),
            clients,
            Arc::new(lr_clients::TokenStore::new()),
            metrics.clone(),
            None,
        );
        *state.secret_scanner.write() = Some(Arc::new(
            lr_secret_scanner::SecretScanEngine::new(&lr_secret_scanner::SecretScanEngineConfig {
                entropy_threshold: 3.0,
                allowlist: vec![],
                scan_system_messages: true,
            })
            .unwrap(),
        ));
        let firewall = AppFirewall {
            config_manager: (*manager).clone(),
            firewall: state.mcp_gateway.firewall_manager.clone(),
            model_approvals: state.model_approval_tracker.clone(),
            metrics,
            state,
            secret_storage: storage,
        };
        let request = FirewallRequest {
            client_id: client.id,
            session_id: Some("monitor-only".into()),
            secret_session: Some("conversation".into()),
            host: "chatgpt.com".into(),
            method: "POST".into(),
            path: "/backend-api/codex/responses".into(),
            provider: "openai".into(),
            model: Some("local-test".into()),
            has_tools: false,
            message_count: 1,
            body: json!({"model":"local-test","input":[{"role":"user","content":format!("aws_secret_access_key={KEY}")} ]}),
        };
        (firewall, request)
    }
    #[tokio::test]
    async fn codex_default_web_search_can_coexist_with_local_restored_tools() {
        for kind in ["web_search", "web_search_preview"] {
            let root =
                std::env::temp_dir().join(format!("lr-codex-search-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let (firewall, mut request) = fixture(
                &root,
                lr_config::LlmMode::Proxy,
                Arc::new(lr_api_keys::MockKeychain::new()),
            );
            request.has_tools = true;
            request.body["input"][0]["content"] =
                json!([{"type":"input_text","text":format!("aws_secret_access_key={KEY}")}]);
            request.body["tools"] = json!([
                {"type":kind},
                {"type":"function","name":"write_file","parameters":{"type":"object","properties":{"content":{"type":"string"}},"required":["content"]}},
                {"type":"custom","name":"apply_patch","format":{"type":"grammar","syntax":"lark","definition":include_str!("../../../crates/lr-secret-scanner/src/codex-grammars/apply_patch.lark")}},
                {"type":"namespace","name":"mcp_files","tools":[{"type":"function","name":"read","parameters":{"type":"object"}}]}
            ]);
            let RequestAction::Protect { body, vault } = firewall.evaluate(&request).await else {
                panic!("Codex's built-in search must not block local secret restoration");
            };
            let masked: Value = serde_json::from_slice(&body).unwrap();
            assert!(!String::from_utf8_lossy(&body).contains(KEY));
            assert_eq!(masked["tools"][0]["type"], kind);
            let tag = masked["input"][0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .strip_prefix("aws_secret_access_key=")
                .unwrap();
            let response = json!({"id":"resp_search","output":[
                {"type":"web_search_call","action":{"type":"search","query":tag}},
                {"type":"function_call","name":"write_file","arguments":json!({"content":tag}).to_string()},
                {"type":"custom_tool_call","name":"apply_patch","input":format!("*** Begin Patch\n*** Add File: .env\n+{tag}\n*** End Patch\n")}
            ]});
            let restored = lr_secret_scanner::wire::ResponseRestorer::new(vault)
                .json(response.to_string().as_bytes())
                .unwrap();
            let restored: Value = serde_json::from_slice(&restored).unwrap();
            assert_eq!(restored["output"][0]["action"]["query"], KEY);
            assert_eq!(
                restored["output"][2]["input"],
                format!("*** Begin Patch\n*** Add File: .env\n+{KEY}\n*** End Patch\n")
            );
            assert_eq!(
                serde_json::from_str::<Value>(restored["output"][1]["arguments"].as_str().unwrap())
                    .unwrap()["content"],
                KEY
            );
            // Tool declarations do not send originals to hosted execution:
            // the entire outbound JSON is still masked, regardless of type.
            request.body["tools"] =
                json!([{"type":"image_generation"},{"type":"tool_search","execution":"client"}]);
            let RequestAction::Protect { body, .. } = firewall.evaluate(&request).await else {
                panic!("Hosted tool metadata must not block a masked Responses request");
            };
            assert!(!String::from_utf8_lossy(&body).contains(KEY));
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[tokio::test]
    async fn configured_redact_supports_actual_proxy_policy_and_owned_retrieval() {
        for mode in [lr_config::LlmMode::Proxy, lr_config::LlmMode::ReverseProxy] {
            let root =
                std::env::temp_dir().join(format!("lr-native-policy-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let (firewall, mut request) =
                fixture(&root, mode, Arc::new(lr_api_keys::MockKeychain::new()));
            request.body["extensions"] = json!({"localrouter.session":{"id":"body-affinity"}});
            let RequestAction::Protect { body, vault } = firewall.evaluate(&request).await else {
                panic!("Redact must protect this Codex request");
            };
            assert!(!String::from_utf8_lossy(&body).contains(KEY));
            let body: Value = serde_json::from_slice(&body).unwrap();
            assert!(body.get("extensions").is_none());
            let tag = body["input"][0]["content"]
                .as_str()
                .unwrap()
                .strip_prefix("aws_secret_access_key=")
                .unwrap();
            let mut codec = lr_secret_scanner::wire::ResponseRestorer::new(vault);
            let response = json!({"id":"resp_owned","object":"response","output":[{"type":"output_text","text":tag}]});
            assert!(
                String::from_utf8(codec.json(response.to_string().as_bytes()).unwrap())
                    .unwrap()
                    .contains(KEY)
            );
            request.method = "GET".into();
            request.path = "/backend-api/codex/responses/resp_owned".into();
            request.body = Value::Null;
            let RequestAction::Protect { body, vault } = firewall.evaluate(&request).await else {
                panic!("Owned response must be retrievable");
            };
            assert!(body.is_empty());
            assert!(String::from_utf8(
                lr_secret_scanner::wire::ResponseRestorer::new(vault)
                    .json(response.to_string().as_bytes())
                    .unwrap()
            )
            .unwrap()
            .contains(KEY));
            request.client_id = "different-client".into();
            assert!(matches!(
                firewall.evaluate(&request).await,
                RequestAction::Reject { .. }
            ));
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    struct Unavailable;
    impl lr_api_keys::KeychainStorage for Unavailable {
        fn store(&self, _: &str, _: &str, _: &str) -> AppResult<()> {
            Err(AppError::Internal("synthetic storage failure".into()))
        }
        fn get(&self, _: &str, _: &str) -> AppResult<Option<String>> {
            Ok(None)
        }
        fn delete(&self, _: &str, _: &str) -> AppResult<()> {
            Ok(())
        }
    }
    #[tokio::test]
    async fn native_policy_blocks_unavailable_os_storage_or_scanner() {
        let root =
            std::env::temp_dir().join(format!("lr-native-policy-failure-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let (firewall, request) = fixture(&root, lr_config::LlmMode::Proxy, Arc::new(Unavailable));
        assert!(matches!(
            firewall.evaluate(&request).await,
            RequestAction::Reject { .. }
        ));
        *firewall.state.secret_scanner.write() = None;
        assert!(matches!(
            firewall.evaluate(&request).await,
            RequestAction::Reject { .. }
        ));
        std::fs::remove_dir_all(root).unwrap();
    }
}
