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
    async fn evaluate_uninspectable(
        &self,
        req: &lr_proxy::active::FirewallRequest,
        error: lr_proxy::request_body::DecodeError,
    ) -> lr_proxy::interceptor::RequestAction {
        use lr_config::PermissionState;
        use lr_proxy::interceptor::RequestAction;
        let config = self.config_manager.get();
        let Some(client) = config
            .clients
            .iter()
            .find(|client| client.id == req.client_id)
        else {
            return RequestAction::reject_json(403, "Client policy is no longer available");
        };
        if !client.enabled || !(client.llm_proxy_enabled() || client.llm_reverse_proxy_enabled()) {
            return RequestAction::reject_json(403, "Proxy access is disabled for this client");
        }
        let Some(strategy) = config
            .strategies
            .iter()
            .find(|strategy| strategy.id == client.strategy_id)
        else {
            return RequestAction::reject_json(403, "Client strategy is no longer available");
        };
        let restricted_models = strategy.model_permissions.resolve_provider(&req.provider)
            != PermissionState::Allow
            || strategy
                .model_permissions
                .models
                .iter()
                .any(|(name, permission)| {
                    name.starts_with(&format!("{}__", req.provider))
                        && *permission != PermissionState::Allow
                });
        let actions = lr_server::routes::pipeline::merge_guardrail_category_actions(
            client.guardrails.category_actions.as_deref(),
            &config.guardrails.category_actions,
        );
        let restricted_content = config.guardrails.scan_requests
            && !actions.is_empty()
            && !lr_server::routes::pipeline::guardrails_allow_everything(&actions)
            && self
                .state
                .safety_engine
                .read()
                .as_ref()
                .is_some_and(|engine| engine.has_models());
        if restricted_models || restricted_content {
            return RequestAction::reject_json(
                error.status(),
                "Configured model or GuardRails permissions require a readable request body",
            );
        }
        lr_server::routes::pipeline::secret_scan_warning(&self.state,&req.client_id,req.session_id.as_deref(),"Request encoding could not be inspected; original request forwarded without Secret Scan coverage");
        self.evaluate_policy(req).await
    }
    fn secret_inspection_warning(
        &self,
        exchange: &lr_proxy::interceptor::ObservedExchange,
        message: &str,
    ) {
        lr_server::routes::pipeline::secret_scan_warning(
            &self.state,
            &exchange.client_id,
            exchange.session_id.as_deref(),
            message,
        );
    }
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
        let Some(client) = config.clients.iter().find(|c| c.id == req.client_id) else {
            return RequestAction::reject_json(403, "Client policy is no longer available");
        };
        if !client.enabled || !(client.llm_proxy_enabled() || client.llm_reverse_proxy_enabled()) {
            return RequestAction::reject_json(403, "Proxy access is disabled for this client");
        }
        let action = client
            .secret_scanning
            .action
            .as_ref()
            .unwrap_or(&config.secret_scanning.action);
        let masking = matches!(
            action,
            lr_config::SecretScanAction::Redact | lr_config::SecretScanAction::Ask
        );
        let resource = lr_proxy::wire::response_id(&req.path);
        let retrieval = resource.is_some()
            && (matches!(req.method.as_str(), "GET" | "DELETE")
                || (req.method == "POST"
                    && req
                        .path
                        .split('?')
                        .next()
                        .is_some_and(|p| p.ends_with("/cancel"))));
        if retrieval {
            let Some(strategy) = config
                .strategies
                .iter()
                .find(|s| s.id == client.strategy_id)
            else {
                return RequestAction::reject_json(403, "Client strategy is no longer available");
            };
            if req.method == "GET"
                && strategy
                    .auto_config
                    .as_ref()
                    .is_some_and(|a| a.permission == lr_config::PermissionState::Off)
            {
                return RequestAction::reject_json(403, "LLM access is disabled for this client");
            }
        }
        if !req.body.is_object() && !retrieval {
            return self
                .evaluate_uninspectable(req, lr_proxy::request_body::DecodeError::Invalid)
                .await;
        }
        let storage = self.secret_storage.clone();
        let continuation = match Vault::find_client(
            storage.clone(),
            &req.client_id,
            lr_secret_scanner::placeholders::RETENTION_SECS,
        ) {
            Ok(vault) => vault,
            Err(_) if retrieval && req.method != "GET" => return RequestAction::Forward,
            Err(_) if !masking => {
                return if retrieval {
                    RequestAction::Forward
                } else {
                    self.evaluate_policy(req).await
                }
            }
            Err(_) => {
                lr_server::routes::pipeline::secret_scan_warning(&self.state, &req.client_id, req.session_id.as_deref(), "Stored references could not be loaded; request proceeds with a fresh client map");
                None
            }
        };
        if retrieval {
            if !masking && continuation.is_none() {
                return RequestAction::Forward;
            }
            let vault = continuation.unwrap_or_else(|| Vault::for_client(storage, &req.client_id));
            lr_server::attach_secret_monitor(
                &self.state,
                &vault,
                &req.client_id,
                req.session_id.as_deref(),
            );
            if !masking {
                vault.set_restore_only();
            }
            return RequestAction::Protect {
                body: Vec::new(),
                vault: Arc::new(vault),
            };
        }
        if !masking {
            let Some(vault) = continuation else {
                return self.evaluate_policy(req).await;
            };
            // Keep ordinary Off/Notify outbound policy and data unchanged.
            // Only references issued to this authenticated client are restored.
            let action = self.evaluate_policy(req).await;
            let body = match action {
                RequestAction::Forward => req.body.clone(),
                RequestAction::Replace(bytes) => match serde_json::from_slice(&bytes) {
                    Ok(body) => body,
                    Err(_) => {
                        return RequestAction::reject_json(400, "Invalid edited request JSON")
                    }
                },
                other => return other,
            };
            lr_server::attach_secret_monitor(
                &self.state,
                &vault,
                &req.client_id,
                req.session_id.as_deref(),
            );
            vault.set_restore_only();
            if vault.set_request_contract(&body).is_err() {
                vault.protection_diagnostic("Restoration contract unavailable; request forwarded");
            }
            return match if retrieval {
                Ok(Vec::new())
            } else {
                serde_json::to_vec(&body)
            } {
                Ok(body) => RequestAction::Protect {
                    body,
                    vault: Arc::new(vault),
                },
                Err(_) => RequestAction::Forward,
            };
        }
        let vault = continuation.unwrap_or_else(|| Vault::for_client(storage, &req.client_id));
        let vault = Arc::new(vault);
        lr_server::attach_secret_monitor(
            &self.state,
            &vault,
            &req.client_id,
            req.session_id.as_deref(),
        );
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
            let _ = lr_server::documents::protect_text_files(&mut body, context.as_ref());
            let _ = lr_types::secret_context::protect(context.as_ref(), &mut body);
            // Hosted tools receive masked inputs. The contract checks local
            // function schemas and compatibility with declared text grammars.
            if let Err(error) = vault.set_request_contract(&body) {
                let _ = error;
                vault.protection_diagnostic("Restoration contract unavailable; request forwarded");
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
                Err(_) => RequestAction::Forward,
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
                lr_server::routes::pipeline::secret_scan_warning(
                    &self.state,
                    &req.client_id,
                    req.session_id.as_deref(),
                    "Document inspection unavailable; original request inspected where possible",
                );
                (body.clone(), true)
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
            lr_server::routes::pipeline::SecretScanOutcome::Deny(_) => {
                return RequestAction::secret_scan_denied()
            }
            lr_server::routes::pipeline::SecretScanOutcome::Redact(redaction) => {
                if lr_server::documents::redact_text_files(&mut body, &redaction).is_err() {
                    lr_server::routes::pipeline::secret_scan_warning(
                        &self.state,
                        &req.client_id,
                        req.session_id.as_deref(),
                        "Attachment redaction unavailable; original attachment forwarded",
                    );
                }
                redaction.apply(&mut body);
                Some(redaction)
            }
            lr_server::routes::pipeline::SecretScanOutcome::Allow => None,
        };

        if let Some(context) = lr_types::secret_context::current() {
            let _ = lr_server::documents::protect_text_files(&mut body, context.as_ref());
            let _ = lr_types::secret_context::protect(context.as_ref(), &mut body);
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
                                        lr_server::routes::pipeline::secret_scan_warning(&self.state, &req.client_id, req.session_id.as_deref(), "Edited document redaction unavailable; original attachment forwarded");
                                    }
                                    redaction.apply(&mut edited);
                                }
                                let (projection, _) =
                                    match lr_server::documents::scan_projection(&edited) {
                                        Ok(value) => value,
                                        Err(_) => (edited.clone(), true),
                                    };
                                match lr_server::routes::pipeline::scan_request_for_secrets_with_source(&self.state, &req.client_id, req.model.as_deref().unwrap_or_default(), &projection, Some(&req.provider), req.session_id.as_deref()).await {
                                lr_server::routes::pipeline::SecretScanOutcome::Deny(_) => return RequestAction::secret_scan_denied(),
                                lr_server::routes::pipeline::SecretScanOutcome::Redact(redaction) => {
                                    let _ = lr_server::documents::redact_text_files(&mut edited, &redaction);
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
                Err(_) => RequestAction::Forward,
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
            secret_storage: lr_api_keys::keychain_trait::redacted_secret_storage(),
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
    pub(super) const KEY: &str = "6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQr";
    pub(super) fn fixture(
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
    async fn untracked_provider_continuation_does_not_require_a_local_secret_record() {
        let root = tempfile::tempdir().unwrap();
        let (firewall, mut request) = fixture(
            root.path(),
            lr_config::LlmMode::Proxy,
            Arc::new(lr_api_keys::MockKeychain::new()),
        );
        request.secret_session = None;
        request.body["previous_response_id"] = json!("provider-response-before-redaction");
        match firewall.evaluate(&request).await {
            RequestAction::Protect { body, .. } => {
                assert!(!String::from_utf8_lossy(&body).contains(KEY));
                assert_eq!(
                    serde_json::from_slice::<Value>(&body).unwrap()["previous_response_id"],
                    "provider-response-before-redaction"
                );
            }
            RequestAction::Reject { status, body, .. } => panic!(
                "valid continuation rejected {status}: {}",
                String::from_utf8_lossy(&body)
            ),
            _ => panic!("Redact must protect the continuation"),
        }
    }
    #[tokio::test]
    async fn continuation_only_custom_tool_content_is_scanned_and_restorable() {
        use lr_types::secret_context::SecretContext;
        for item in [
            json!({"type":"custom_tool_call","call_id":"call-1","name":"exec","input":format!("aws_secret_access_key={KEY}")}),
            json!({"type":"custom_tool_call_output","call_id":"call-1","output":[{"type":"input_text","text":format!("aws_secret_access_key={KEY}")}]}),
            json!({"type":"additional_tools","role":"developer","tools":[{"type":"function","name":"write","description":format!("aws_secret_access_key={KEY}"),"parameters":{"type":"object"}}]}),
        ] {
            let root = tempfile::tempdir().unwrap();
            let (firewall, mut request) = fixture(
                root.path(),
                lr_config::LlmMode::Proxy,
                Arc::new(lr_api_keys::MockKeychain::new()),
            );
            request.body =
                json!({"model":"local-test","previous_response_id":"untracked","input":[item]});
            let RequestAction::Protect { body, vault } = firewall.evaluate(&request).await else {
                panic!("custom continuation must protect")
            };
            assert!(
                !String::from_utf8_lossy(&body).contains(KEY),
                "first-time custom content must be scanned"
            );
            let mut restored: Value = serde_json::from_slice(&body).unwrap();
            vault.restore(&mut restored, false).unwrap();
            assert!(
                restored.to_string().contains(KEY),
                "newly detected original must be recoverable"
            );
        }
    }
    #[tokio::test]
    async fn escaped_multiline_credentials_are_masked_as_originals_in_json_text() {
        use base64::Engine;
        use lr_types::secret_context::SecretContext;
        let material="whtWShJK3py2Nh1bWey7TsPWCMmmPRsRIs0cg240wR5nPxMgnPFkNj6Dl0IbSfxxVikEf/GcBB/TEZ5PhCH5a2Px9L8ihbSWxS+T2qPjUrE3Xtt5QSqydTGneV72XJxG";
        let secret = format!("-----BEGIN PRIVATE KEY-----\n{material}\n-----END PRIVATE KEY-----");
        let text = json!({"private_key":secret}).to_string();
        for input in [
            json!({"role":"user","content":text}),
            json!({"type":"custom_tool_call_output","call_id":"call-1","output":[{"type":"input_text","text":text}]}),
            json!({"role":"user","content":[{"type":"input_file","filename":"credentials.json","file_data":format!("data:text/plain;base64,{}",base64::engine::general_purpose::STANDARD.encode(&text))}]}),
        ] {
            let root = tempfile::tempdir().unwrap();
            let (firewall, mut request) = fixture(
                root.path(),
                lr_config::LlmMode::Proxy,
                Arc::new(lr_api_keys::MockKeychain::new()),
            );
            request.body = json!({"model":"local-test","input":[input]});
            let RequestAction::Protect { body, vault } = firewall.evaluate(&request).await else {
                panic!("multiline credential must protect")
            };
            let protected: Value = serde_json::from_slice(&body).unwrap();
            let (mut restored, _) = lr_server::documents::scan_projection(&protected).unwrap();
            assert!(
                !restored.to_string().contains(material),
                "JSON escaping must not bypass redaction"
            );
            vault.restore(&mut restored, false).unwrap();
            let (original, _) = lr_server::documents::scan_projection(&request.body).unwrap();
            assert_eq!(restored, original, "original JSON encoding must round-trip");
        }
    }
    #[tokio::test]
    async fn encoded_known_originals_are_absent_from_the_actual_forwarded_code_body() {
        use lr_types::secret_context::SecretContext;
        let root = tempfile::tempdir().unwrap();
        let storage = Arc::new(lr_api_keys::MockKeychain::new());
        let (firewall, mut request) =
            fixture(root.path(), lr_config::LlmMode::Proxy, storage.clone());
        let secret = "wire-proof-\"quoted\"-'apostrophe'-\\path\n<&>-🔐";
        let vault = lr_secret_scanner::placeholders::Vault::for_client(storage, &request.client_id);
        vault.register(secret).unwrap();
        let code = format!(
            "tools.exec_command({});",
            json!({"cmd":format!("expected = {}",serde_json::to_string(secret).unwrap())})
        );
        request.body = json!({"model":"local-test","input":[{"type":"custom_tool_call","name":"exec","call_id":"call-1","input":code}]});
        let RequestAction::Protect { body, vault } = firewall.evaluate(&request).await else {
            panic!("known history must protect");
        };
        // These are the bytes returned to the transport, before Monitor's separate sanitization.
        assert!(!String::from_utf8_lossy(&body).contains("wire-proof"));
        let mut protected: Value = serde_json::from_slice(&body).unwrap();
        vault.restore(&mut protected, true).unwrap();
        assert_eq!(protected, request.body);
    }
    #[tokio::test]
    async fn owned_response_lifecycle_operations_accept_opaque_ids_and_empty_bodies() {
        use lr_types::secret_context::SecretContext;
        let root = tempfile::tempdir().unwrap();
        let (firewall, mut request) = fixture(
            root.path(),
            lr_config::LlmMode::Proxy,
            Arc::new(lr_api_keys::MockKeychain::new()),
        );
        let RequestAction::Protect { body, vault } = firewall.evaluate(&request).await else {
            panic!("initial scope")
        };
        let body: Value = serde_json::from_slice(&body).unwrap();
        let tag = body["input"][0]["content"]
            .as_str()
            .unwrap()
            .split_once('=')
            .unwrap()
            .1
            .to_owned();
        let mut codec = lr_secret_scanner::wire::ResponseRestorer::new(vault);
        codec
            .json(
                json!({"object":"response","id":"opaque/id:ABC","output":[]})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        drop(codec);
        request.secret_session = None;
        request.body = Value::Null;
        for action in [
            lr_config::SecretScanAction::Redact,
            lr_config::SecretScanAction::Notify,
            lr_config::SecretScanAction::Off,
        ] {
            firewall
                .config_manager
                .update(|c| c.secret_scanning.action = action)
                .unwrap();
            for (method, suffix) in [
                ("GET", ""),
                ("GET", "/input_items"),
                ("POST", "/cancel"),
                ("DELETE", ""),
            ] {
                request.method = method.into();
                request.path = format!("/backend-api/codex/responses/opaque%2Fid%3AABC{suffix}");
                let RequestAction::Protect { body, vault } = firewall.evaluate(&request).await
                else {
                    panic!("response lifecycle must keep its owned restoration scope")
                };
                assert!(body.is_empty());
                assert_eq!(vault.restore_text(&tag, true).unwrap(), KEY);
            }
        }
        request.method = "GET".into();
        request.path = "/backend-api/codex/responses/opaque%2Fid%3AABC".into();
        firewall
            .config_manager
            .update(|c| {
                c.strategies[0]
                    .auto_config
                    .get_or_insert_with(Default::default)
                    .permission = lr_config::PermissionState::Off
            })
            .unwrap();
        assert!(
            matches!(
                firewall.evaluate(&request).await,
                RequestAction::Reject { .. }
            ),
            "disabling LLM access must still block owned response reads"
        );
        request.method = "POST".into();
        request.path.push_str("/cancel");
        assert!(
            matches!(
                firewall.evaluate(&request).await,
                RequestAction::Protect { .. }
            ),
            "generation permissions must not prevent cancellation"
        );
        firewall
            .config_manager
            .update(|c| c.clients[0].enabled = false)
            .unwrap();
        assert!(
            matches!(
                firewall.evaluate(&request).await,
                RequestAction::Reject { .. }
            ),
            "disabled clients cannot use lifecycle operations"
        );
    }
    #[tokio::test]
    async fn unreadable_encoding_proceeds_when_only_secret_inspection_is_unavailable() {
        let root = tempfile::tempdir().unwrap();
        let (firewall, mut request) = fixture(
            root.path(),
            lr_config::LlmMode::Proxy,
            Arc::new(lr_api_keys::MockKeychain::new()),
        );
        request.body = Value::Null;
        request.model = None;
        assert!(matches!(
            firewall
                .evaluate_uninspectable(&request, lr_proxy::request_body::DecodeError::Unsupported)
                .await,
            RequestAction::Forward
        ));
        firewall
            .config_manager
            .update(|config| {
                config.strategies[0].model_permissions.models.insert(
                    "openai__forbidden-model".into(),
                    lr_config::PermissionState::Off,
                );
            })
            .unwrap();
        assert!(
            matches!(
                firewall
                    .evaluate_uninspectable(
                        &request,
                        lr_proxy::request_body::DecodeError::Unsupported
                    )
                    .await,
                RequestAction::Reject { .. }
            ),
            "separate model permissions must not be bypassed"
        );
    }

    #[tokio::test]
    async fn off_and_cancellation_do_not_acquire_a_new_native_store_requirement() {
        struct Unavailable;
        impl lr_api_keys::KeychainStorage for Unavailable {
            fn store(&self, _: &str, _: &str, _: &str) -> lr_types::AppResult<()> {
                Err(lr_types::AppError::Internal("synthetic unavailable".into()))
            }
            fn get(&self, _: &str, _: &str) -> lr_types::AppResult<Option<String>> {
                Err(lr_types::AppError::Internal("synthetic unavailable".into()))
            }
            fn delete(&self, _: &str, _: &str) -> lr_types::AppResult<()> {
                Err(lr_types::AppError::Internal("synthetic unavailable".into()))
            }
        }
        let root = tempfile::tempdir().unwrap();
        let (firewall, mut request) = fixture(
            root.path(),
            lr_config::LlmMode::Proxy,
            Arc::new(Unavailable),
        );
        firewall
            .config_manager
            .update(|c| c.secret_scanning.action = lr_config::SecretScanAction::Off)
            .unwrap();
        request.body["previous_response_id"] = json!("ordinary-provider-id");
        assert!(matches!(
            firewall.evaluate(&request).await,
            RequestAction::Forward
        ));
        firewall
            .config_manager
            .update(|c| c.secret_scanning.action = lr_config::SecretScanAction::Redact)
            .unwrap();
        request.body = Value::Null;
        request.path = "/backend-api/codex/responses/ordinary-provider-id/cancel".into();
        assert!(
            matches!(firewall.evaluate(&request).await, RequestAction::Forward),
            "cancellation must reach the provider even if the native store is unavailable"
        );
        request.method = "GET".into();
        request.path = "/backend-api/codex/responses/ordinary-provider-id".into();
        assert!(
            matches!(
                firewall.evaluate(&request).await,
                RequestAction::Protect { .. }
            ),
            "protected retrieval must proceed despite inaccessible secret storage"
        );
    }
    #[tokio::test]
    async fn foreign_response_id_cannot_restore_another_clients_secret() {
        use lr_types::secret_context::SecretContext;
        let root = tempfile::tempdir().unwrap();
        let other_root = tempfile::tempdir().unwrap();
        let storage = Arc::new(lr_api_keys::MockKeychain::new());
        let (owner, request) = fixture(root.path(), lr_config::LlmMode::Proxy, storage.clone());
        let RequestAction::Protect { body, vault } = owner.evaluate(&request).await else {
            panic!("owner request must protect")
        };
        let protected: Value = serde_json::from_slice(&body).unwrap();
        let tag = protected["input"][0]["content"]
            .as_str()
            .unwrap()
            .split_once('=')
            .unwrap()
            .1;
        let mut restorer = lr_secret_scanner::wire::ResponseRestorer::new(vault);
        restorer
            .message(
                &json!({"type":"response.created","response":{"id":"opaque-owner-response"}})
                    .to_string(),
            )
            .unwrap();
        drop(restorer);
        let (other, mut request) = fixture(other_root.path(), lr_config::LlmMode::Proxy, storage);
        request.secret_session = None;
        request.body = json!({"model":"local-test","previous_response_id":"opaque-owner-response","input":"ordinary continuation"});
        let RequestAction::Protect { vault, .. } = other.evaluate(&request).await else {
            panic!("untracked provider ID must not block request")
        };
        assert_eq!(
            vault.restore_text(tag, true).unwrap(),
            tag,
            "foreign provider ID cannot grant a secret capability"
        );
        let mut text = Value::String(tag.into());
        vault.restore(&mut text, false).unwrap();
        assert_eq!(text, tag);
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
    async fn redaction_retains_masked_scan_and_correlates_the_parent_request() {
        use lr_proxy::interceptor::{ObservedExchange, ProxyInterceptor};
        let root = tempfile::tempdir().unwrap();
        let (firewall, request) = fixture(
            root.path(),
            lr_config::LlmMode::Proxy,
            Arc::new(lr_api_keys::MockKeychain::new()),
        );
        let store = firewall.state.monitor_store.clone();
        let recorder = lr_proxy::passive::PassiveInterceptor::new(store.clone());
        let RequestAction::Protect { body, vault } = firewall.evaluate(&request).await else {
            panic!("request must be protected")
        };
        let exchange = ObservedExchange {
            client_id: request.client_id.clone(),
            session_id: request.session_id.clone(),
            secret_session: Some("caller-conversation".into()),
            path: request.path.clone(),
            request_body: Some(body),
            ..Default::default()
        };
        recorder.begin(&exchange);
        let events: Vec<_> = store
            .list(0, 100, None)
            .events
            .iter()
            .map(|e| store.get(&e.id).unwrap())
            .collect();
        let scan = events
            .iter()
            .find(|e| e.event_type == lr_monitor::MonitorEventType::SecretScan)
            .unwrap();
        let llm = events
            .iter()
            .find(|e| e.event_type == lr_monitor::MonitorEventType::LlmCall)
            .unwrap();
        for event in [&scan, &llm] {
            assert!(!event.payload_redacted);
            assert_eq!(event.session_id, request.session_id);
            let serialized = serde_json::to_string(event).unwrap();
            assert!(!serialized.contains(KEY));
            assert!(serialized.contains("localrouter-redacted"));
        }
        assert_eq!(scan.client_name.as_deref(), Some("Synthetic"));
        assert_eq!(
            scan.conversation.as_ref().unwrap().id,
            llm.conversation.as_ref().unwrap().id
        );
        let lr_monitor::MonitorEventData::SecretScan {
            findings,
            action_taken,
            ..
        } = &scan.data
        else {
            unreachable!()
        };
        assert_eq!(action_taken.as_deref(), Some("redact"));
        assert!(findings
            .as_ref()
            .unwrap()
            .as_array()
            .is_some_and(|v| !v.is_empty()));
        use lr_types::secret_context::SecretContext;
        let tag = findings.as_ref().unwrap()[0]["replacement"]
            .as_str()
            .unwrap();
        assert_eq!(vault.restore_text(tag, true).unwrap(), KEY);
        assert_eq!(
            vault.restore_text_at(tag, true, "Tool arguments").unwrap(),
            KEY
        );
        let phases: Vec<_> = store
            .list(0, 100, None)
            .events
            .into_iter()
            .filter(|event| event.event_type == lr_monitor::MonitorEventType::SecretScan)
            .collect();
        assert_eq!(phases.len(), 2);
        let restored = phases
            .iter()
            .find(|event| event.secret_action.as_deref() == Some("restore"))
            .unwrap();
        assert_eq!(restored.session_id, request.session_id);
        assert_eq!(
            restored.conversation.as_ref().unwrap().id,
            scan.conversation.as_ref().unwrap().id
        );
        let restored = store.get(&restored.id).unwrap();
        assert!(!serde_json::to_string(&restored).unwrap().contains(KEY));
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
    async fn native_policy_continues_when_storage_or_scanner_is_unavailable() {
        let root =
            std::env::temp_dir().join(format!("lr-native-policy-failure-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let (firewall, request) = fixture(&root, lr_config::LlmMode::Proxy, Arc::new(Unavailable));
        assert!(matches!(
            firewall.evaluate(&request).await,
            RequestAction::Protect { .. }
        ));
        *firewall.state.secret_scanner.write() = None;
        assert!(matches!(
            firewall.evaluate(&request).await,
            RequestAction::Protect { .. }
        ));
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
#[path = "proxy_lifecycle_tests.rs"]
mod lifecycle_tests;
