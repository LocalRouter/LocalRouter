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

/// The gateway's strategy rate-limit check ([`lr_router::strategy_limits`]):
/// projected usage over the rolling window must not exceed any enabled
/// limit. Returns the refusal, with when the request would fit, when the
/// request should be blocked.
///
/// `in_flight` is what the strategy's admitted requests (gateway and proxied)
/// hold while they are still running (metrics only show completed ones).
fn rate_limit_exceeded(
    metrics: &lr_monitoring::metrics::MetricsCollector,
    strategy: &lr_config::Strategy,
    in_flight: &lr_router::rate_limit::ReservedUsage,
) -> Option<lr_router::strategy_limits::StrategyLimitDenial> {
    if strategy.rate_limits.is_empty() {
        return None;
    }
    let (avg_tokens, avg_cost) = metrics.get_pre_estimate_for_strategy(&strategy.id, 10);
    lr_router::strategy_limits::check(metrics, strategy, avg_tokens, avg_cost, in_flight).err()
}

/// The client-facing message of a strategy-limit refusal.
fn strategy_limit_message(denial: &lr_router::strategy_limits::StrategyLimitDenial) -> String {
    format!(
        "Rate limit exceeded ({} per {})",
        denial.limit,
        denial.window_name()
    )
}

/// Error code of a strategy-limit refusal, as the native server reports it.
const STRATEGY_LIMIT_CODE: &str = "strategy_rate_limit_exceeded";

/// A strategy-limit refusal: 429 with `Retry-After`, as the native server
/// answers it.
fn strategy_limited(
    denial: &lr_router::strategy_limits::StrategyLimitDenial,
) -> lr_proxy::interceptor::RequestAction {
    lr_proxy::interceptor::RequestAction::rate_limited_with_code(
        &strategy_limit_message(denial),
        Some(STRATEGY_LIMIT_CODE),
        Some(denial.retry_after_secs),
    )
}

/// Refusal for a request whose body cannot be decoded while a configured
/// model or GuardRails restriction needs to read it.
const READABLE_BODY_REQUIRED: &str =
    "Configured model or GuardRails permissions require a readable request body";

/// Whether a request for a stored response only retrieves, deletes or cancels
/// it, and so carries no conversation of its own.
fn is_retrieval(method: &str, path: &str) -> bool {
    lr_proxy::wire::response_id(path).is_some()
        && (matches!(method, "GET" | "DELETE")
            || (method == "POST"
                && path
                    .split('?')
                    .next()
                    .is_some_and(|p| p.ends_with("/cancel"))))
}

#[async_trait::async_trait]
impl lr_proxy::active::Firewall for AppFirewall {
    /// The refusals [`evaluate`](Self::evaluate) reaches whatever the body
    /// holds, answered before the body is read. Everything else is buffered:
    /// the secret scan, GuardRails, the allowed-models list and the model
    /// approval all read the body.
    fn body_policy(
        &self,
        head: &lr_proxy::interceptor::RequestHead<'_>,
    ) -> lr_proxy::interceptor::BodyPolicy {
        use lr_config::PermissionState;
        use lr_proxy::interceptor::BodyPolicy;
        let config = self.config_manager.get();
        let Some(client) = config.clients.iter().find(|c| c.id == head.client_id) else {
            return BodyPolicy::Reject(403, "Client policy is no longer available".into());
        };
        if !client.enabled || !(client.llm_proxy_enabled() || client.llm_reverse_proxy_enabled()) {
            return BodyPolicy::Reject(403, "Proxy access is disabled for this client".into());
        }
        let Some(strategy) = config
            .strategies
            .iter()
            .find(|s| s.id == client.strategy_id)
        else {
            return BodyPolicy::Reject(403, "Client strategy is no longer available".into());
        };
        // An undecodable body is refused first, with the decoder's status.
        if let Some(error) = head.content_encoding_error() {
            if self.requires_readable_body(&config, client, strategy, &head.provider()) {
                return BodyPolicy::Reject(error.status(), READABLE_BODY_REQUIRED.into());
            }
        }
        let off = strategy
            .auto_config
            .as_ref()
            .is_some_and(|a| a.permission == PermissionState::Off);
        if is_retrieval(head.method, head.path) {
            // Retrievals skip the model policy and the rate limits; only
            // reading a stored response is refused while access is off.
            if off && head.method == "GET" {
                return BodyPolicy::Reject(403, "LLM access is disabled for this client".into());
            }
            return BodyPolicy::Buffered;
        }
        if off {
            return BodyPolicy::Reject(403, "LLM access is disabled for this client".into());
        }
        let in_flight = self.strategy_in_flight(&strategy.id, head.trace);
        if let Some(denial) = rate_limit_exceeded(&self.metrics, strategy, &in_flight) {
            return BodyPolicy::RateLimited {
                message: strategy_limit_message(&denial),
                code: Some(STRATEGY_LIMIT_CODE.into()),
                retry_after: denial.retry_after_secs,
            };
        }
        // A body above the stream lag is forwarded as it uploads when nothing
        // needs it whole; smaller bodies are still read whole and evaluated.
        self.stream_policy(&config, client, strategy, head)
            .map(BodyPolicy::Stream)
            .unwrap_or(BodyPolicy::Buffered)
    }
    async fn evaluate_uninspectable(
        &self,
        req: &lr_proxy::active::FirewallRequest,
        error: lr_proxy::request_body::DecodeError,
    ) -> lr_proxy::interceptor::RequestAction {
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
        if self.requires_readable_body(&config, client, strategy, &req.provider) {
            return RequestAction::reject_json(error.status(), READABLE_BODY_REQUIRED);
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
    /// The decisions [`evaluate`](Self::evaluate) makes without reading the
    /// content, for a body that outgrew its stream lag: refusals that do not
    /// depend on the content, the model approval popup (with a read-only
    /// preview: the body is too large to edit there), then a streaming
    /// session. Secret Scan Ask streams too: each new finding holds the
    /// upload at its unit until the user answered. Active GuardRails hold
    /// the whole (masked) body until their verdict on its text projection;
    /// the model approval is then asked after that verdict, in the buffered
    /// path's order.
    async fn open_stream(
        &self,
        req: &lr_proxy::active::StreamRequest,
        policy: &lr_proxy::interceptor::StreamPolicy,
    ) -> lr_proxy::interceptor::StreamOpen {
        use lr_config::PermissionState;
        use lr_proxy::interceptor::{RequestAction, StreamMasking, StreamOpen};
        use lr_secret_scanner::placeholders::Vault;
        let config = self.config_manager.get();
        let Some(client) = config.clients.iter().find(|c| c.id == req.client_id) else {
            return StreamOpen::Reject(RequestAction::reject_json(
                403,
                "Client policy is no longer available",
            ));
        };
        if !client.enabled || !(client.llm_proxy_enabled() || client.llm_reverse_proxy_enabled()) {
            return StreamOpen::Reject(RequestAction::reject_json(
                403,
                "Proxy access is disabled for this client",
            ));
        }
        let Some(strategy) = config
            .strategies
            .iter()
            .find(|s| s.id == client.strategy_id)
        else {
            return StreamOpen::Reject(RequestAction::reject_json(
                403,
                "Client strategy is no longer available",
            ));
        };
        let action = client
            .secret_scanning
            .action
            .as_ref()
            .unwrap_or(&config.secret_scanning.action);
        // The policy may have changed since the head was read.
        let expected = masking_for(action);
        if expected != policy.masking || config.secret_scanning.strict_streaming != policy.strict {
            return StreamOpen::Buffer;
        }
        // GuardRails judge the whole body: it is held (spilled, encrypted)
        // until their verdict, and only its text projection is kept.
        let (_, guardrails) = self.restrictions(&config, client, strategy, &req.provider);
        // A duplicate hop was judged and asked by its first hop (see
        // `judge_policy`): it streams only to protect secrets.
        let duplicate_hop = req.trace.as_ref().is_some_and(|t| t.is_duplicate());
        let guardrails = guardrails && !duplicate_hop;
        let permission = strategy
            .auto_config
            .as_ref()
            .map(|a| a.permission.clone())
            .unwrap_or(PermissionState::Allow);
        if permission == PermissionState::Off {
            return StreamOpen::Reject(RequestAction::reject_json(
                403,
                "LLM access is disabled for this client",
            ));
        }
        let Some(model) = req.model.as_deref() else {
            return StreamOpen::Buffer;
        };
        let bare = model.rsplit('/').next().unwrap_or(model);
        if !strategy.is_model_allowed(&req.provider, bare) {
            return StreamOpen::Reject(RequestAction::reject_json(
                403,
                &format!("Model '{model}' is not permitted for this client"),
            ));
        }
        let in_flight = self.strategy_in_flight(&strategy.id, req.trace.as_ref());
        if let Some(denial) = rate_limit_exceeded(&self.metrics, strategy, &in_flight) {
            return StreamOpen::Reject(strategy_limited(&denial));
        }
        let mut deferred_model_ask: Option<super::proxy_stream::Deferred> = None;
        if permission == PermissionState::Ask && !duplicate_hop {
            let has_time_bypass =
                self.model_approvals
                    .has_valid_approval(&client.id, &req.provider, model);
            let has_explicit_allow = matches!(
                strategy
                    .model_permissions
                    .models
                    .get(&format!("{}__{model}", req.provider)),
                Some(PermissionState::Allow)
            );
            // The body is too large to edit in the popup: it shows the
            // beginning read-only. The client's upload is held meanwhile.
            if !has_time_bypass && !has_explicit_allow {
                let ask = StreamModelAsk {
                    firewall: self.firewall.clone(),
                    state: self.state.clone(),
                    client_id: client.id.clone(),
                    client_name: client.name.clone(),
                    model: model.to_owned(),
                    provider: req.provider.clone(),
                    session_id: req.session_id.clone(),
                    preview: self.masked_preview(&req.client_id, req.preview.as_ref()),
                };
                if guardrails {
                    // Asked after the GuardRails verdict, as on the
                    // buffered path; nothing is sent before either.
                    deferred_model_ask = Some(Box::new(move || {
                        Box::pin(async move {
                            ask.run()
                                .await
                                .map_err(|message| super::proxy_stream::refusal(403, message))
                        })
                    }));
                } else if let Err(message) = ask.run().await {
                    return StreamOpen::Reject(RequestAction::reject_json(403, message));
                }
            }
        }
        // A caller that releases as it goes (a websocket message) cannot
        // hold the body until the GuardRails verdict: it is judged whole by
        // `evaluate` instead. Decided here, before a reservation, a vault
        // and a scan session are built only to be thrown away.
        if guardrails && policy.incremental {
            return StreamOpen::Buffer;
        }
        // Admitted: reserve its rate-limit share, held while it streams.
        if let Err(refusal) = self
            .admit(&req.client_id, strategy, req.trace.as_ref(), &req.admission)
            .await
        {
            return StreamOpen::Reject(refusal);
        }
        // Ask masks what the user chose to replace, as Redact masks every
        // finding.
        let masking = matches!(expected, StreamMasking::Redact | StreamMasking::Ask);
        let storage = self.secret_storage.clone();
        let continuation = match Vault::find_client(
            storage.clone(),
            &req.client_id,
            lr_secret_scanner::placeholders::RETENTION_SECS,
        ) {
            Ok(vault) => vault,
            Err(_) if !masking => None,
            Err(_) => {
                lr_server::routes::pipeline::secret_scan_warning(
                    &self.state,
                    &req.client_id,
                    req.session_id.as_deref(),
                    "Stored references could not be loaded; request proceeds with a fresh client map",
                );
                None
            }
        };
        let vault = if masking {
            Some(continuation.unwrap_or_else(|| Vault::for_client(storage, &req.client_id)))
        } else {
            continuation
        }
        .map(Arc::new);
        if let Some(vault) = &vault {
            lr_server::attach_secret_monitor(
                &self.state,
                vault,
                &req.client_id,
                req.session_id.as_deref(),
            );
            if !masking {
                vault.set_restore_only();
            }
        }
        let context = vault
            .clone()
            .filter(|_| masking)
            .map(|vault| vault as Arc<dyn lr_types::secret_context::SecretContext>);
        let session_id = req
            .session_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let scan = lr_server::routes::pipeline::stream_scan::StreamSecretScan::new(
            &self.state,
            &req.client_id,
            &session_id,
            Some(&req.provider),
            context,
        )
        .map(|scan| scan.with_model(model));
        // Nothing changes the bytes of an Off/Notify request unless a
        // remembered "always redact" decision may apply: forward the
        // original bytes as they arrive and inspect a copy.
        let tee = !masking
            && scan
                .as_ref()
                .is_none_or(|scan| !scan.remembers_redactions());
        let mut session = super::proxy_stream::AppStreamSession::new(
            self.state.clone(),
            req.client_id.clone(),
            req.session_id.clone(),
            scan,
            vault.clone(),
            masking,
        );
        if guardrails {
            session = session
                .with_guardrails(super::proxy_stream::GuardrailsHold::new(deferred_model_ask));
        }
        StreamOpen::Stream {
            session: Box::new(session),
            vault,
            tee,
        }
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
    /// Secret Scan Off forwards (and records) a request as it is, so its
    /// refusal is kept as it is too. Any other action masks every finding of
    /// the scanner and every original the client's vault protects: Notify
    /// and findings an Ask/Redact decision would have let through are
    /// masked as well, because the request was never sent and no decision
    /// was taken on it. Without a scanner or a readable vault nothing can be
    /// masked, and only the length is kept.
    fn mask_for_monitor(&self, client_id: &str, content: &mut Vec<u8>) -> bool {
        let config = self.config_manager.get();
        let action = config
            .clients
            .iter()
            .find(|c| c.id == client_id)
            .and_then(|c| c.secret_scanning.action.as_ref())
            .unwrap_or(&config.secret_scanning.action);
        if matches!(action, lr_config::SecretScanAction::Off) {
            return true;
        }
        let Some(scanner) = self.state.secret_scanner.read().as_ref().cloned() else {
            return false;
        };
        let Ok(vault) = lr_secret_scanner::placeholders::Vault::find_client(
            self.secret_storage.clone(),
            client_id,
            lr_secret_scanner::placeholders::RETENTION_SECS,
        ) else {
            return false;
        };
        mask_monitor_content(&scanner, vault.as_ref(), content)
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
        let retrieval = is_retrieval(&req.method, &req.path);
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

/// The model approval popup for a streamed body (read-only preview).
struct StreamModelAsk {
    firewall: Arc<lr_mcp::gateway::firewall::FirewallManager>,
    state: lr_server::state::AppState,
    client_id: String,
    client_name: String,
    model: String,
    provider: String,
    session_id: Option<String>,
    preview: lr_mcp::gateway::firewall::RequestPreview,
}

impl StreamModelAsk {
    /// Ask; `Err` carries the refusal message the buffered path answers.
    async fn run(self) -> Result<(), &'static str> {
        use lr_mcp::gateway::firewall::FirewallApprovalAction as A;
        let response = self
            .firewall
            .request_model_approval_with_preview(
                self.client_id.clone(),
                self.client_name,
                self.model.clone(),
                self.provider,
                Some(120),
                self.preview,
            )
            .await;
        if let Ok(response) = &response {
            lr_server::routes::monitor_helpers::emit_firewall_decision(
                &self.state,
                Some(&lr_server::middleware::client_auth::ClientAuthContext {
                    client_id: self.client_id.clone(),
                }),
                self.session_id.as_deref(),
                "model",
                &self.model,
                &format!("{:?}", response.action),
                None,
            );
        }
        match response {
            Ok(response) => match response.action {
                A::AllowOnce
                | A::AllowSession
                | A::Allow1Minute
                | A::Allow1Hour
                | A::AllowPermanent
                | A::AllowCategories => Ok(()),
                _ => Err("Denied by user"),
            },
            Err(_) => Err("Approval timed out"),
        }
    }
}

/// A proxied exchange's rate-limit shares, kept in its admission slot until
/// the recorder has written its metrics row (which then counts for the
/// strategy), and charged to the client's limiters with the recorded usage.
struct ProxyAdmission {
    limiter: Arc<lr_router::RateLimiterManager>,
    client_id: String,
    /// The client's limiters, when it has any.
    client: lr_router::AdmissionReservation,
    strategy: lr_router::AdmissionReservation,
}

impl lr_proxy::interceptor::AdmissionGuard for ProxyAdmission {
    fn settle(self: Box<Self>, usage: &lr_proxy::interceptor::RecordedUsage) {
        // Client limiters count what they are charged, so the request's
        // actual usage replaces its estimate. A duplicate hop holds no
        // client share and is never charged (its first hop was).
        if usage.billed && self.client.is_reserved() {
            self.limiter.charge_api_key(
                &self.client_id,
                &lr_router::UsageInfo {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cost_usd: usage.cost_usd,
                },
            );
        }
        // Dropping `self` releases both shares.
    }
}

/// Mask `content` for the monitor: every secret `scanner` finds and every
/// original `vault` protects. A JSON body is scanned string by string
/// (decoded text attachments included) and keeps its structure; anything
/// else (a capture's head, an error) is scanned as one text. False when it
/// cannot be done.
fn mask_monitor_content(
    scanner: &lr_secret_scanner::SecretScanEngine,
    vault: Option<&lr_secret_scanner::placeholders::Vault>,
    content: &mut Vec<u8>,
) -> bool {
    use lr_secret_scanner::{redaction::Redaction, ExtractedText};
    use lr_types::secret_context::SecretContext;
    fn strings(value: &serde_json::Value, out: &mut Vec<ExtractedText>) {
        match value {
            serde_json::Value::String(text) => out.push(ExtractedText {
                label: "monitor".into(),
                text: text.clone(),
                message_index: out.len(),
            }),
            serde_json::Value::Array(items) => items.iter().for_each(|v| strings(v, out)),
            serde_json::Value::Object(map) => map.values().for_each(|v| strings(v, out)),
            _ => {}
        }
    }
    let findings = |texts: &[ExtractedText]| {
        let mut redaction = Redaction::default();
        for finding in scanner.scan(texts).findings {
            redaction.add(&finding.raw_text);
        }
        redaction
    };
    match serde_json::from_slice::<serde_json::Value>(content) {
        Ok(mut body) => {
            let Ok((projection, _)) = lr_server::documents::scan_projection(&body) else {
                return false;
            };
            let mut texts = Vec::new();
            strings(&projection, &mut texts);
            let redaction = findings(&texts);
            let mut changed = false;
            if !redaction.is_empty() {
                if lr_server::documents::redact_text_files(&mut body, &redaction).is_err() {
                    return false;
                }
                redaction.apply(&mut body);
                changed = true;
            }
            if let Some(vault) = vault {
                if vault.redact_monitor(&mut body).is_err() {
                    return false;
                }
                changed = true;
            }
            if changed {
                match serde_json::to_vec(&body) {
                    Ok(bytes) => *content = bytes,
                    Err(_) => return false,
                }
            }
            true
        }
        Err(_) => {
            let text = String::from_utf8_lossy(content).into_owned();
            let redaction = findings(&[ExtractedText {
                label: "monitor".into(),
                text: text.clone(),
                message_index: 0,
            }]);
            let mut value = serde_json::Value::String(redaction.apply_text(&text));
            if let Some(vault) = vault {
                if vault.redact_monitor(&mut value).is_err() {
                    return false;
                }
            }
            *content = value.as_str().unwrap_or_default().as_bytes().to_vec();
            true
        }
    }
}

/// How a streamed body is handled for a Secret Scan action.
fn masking_for(action: &lr_config::SecretScanAction) -> lr_proxy::interceptor::StreamMasking {
    use lr_proxy::interceptor::StreamMasking;
    match action {
        lr_config::SecretScanAction::Redact => StreamMasking::Redact,
        lr_config::SecretScanAction::Notify => StreamMasking::Notify,
        lr_config::SecretScanAction::Off => StreamMasking::Off,
        lr_config::SecretScanAction::Ask => StreamMasking::Ask,
    }
}

impl AppFirewall {
    /// The beginning of a streamed request for the model approval popup,
    /// with every secret the scanner finds in it, and every original the
    /// client's vault already protects, masked.
    fn masked_preview(
        &self,
        client_id: &str,
        preview: Option<&lr_proxy::active::StreamPreview>,
    ) -> lr_mcp::gateway::firewall::RequestPreview {
        use lr_types::secret_context::SecretContext;
        let Some(preview) = preview else {
            return lr_mcp::gateway::firewall::RequestPreview {
                text: String::new(),
                total_bytes: 0,
                truncated: true,
            };
        };
        let mut text = preview.text.clone();
        let scanner = self.state.secret_scanner.read().as_ref().cloned();
        if let Some(scanner) = scanner {
            let found = scanner.scan(&[lr_secret_scanner::ExtractedText {
                label: "preview".into(),
                text: text.clone(),
                message_index: 0,
            }]);
            let mut mask = lr_secret_scanner::redaction::Redaction::default();
            for finding in &found.findings {
                mask.add(&finding.raw_text);
            }
            text = mask.apply_text(&text);
        }
        if let Ok(Some(vault)) = lr_secret_scanner::placeholders::Vault::find_client(
            self.secret_storage.clone(),
            client_id,
            lr_secret_scanner::placeholders::RETENTION_SECS,
        ) {
            let mut value = serde_json::Value::String(text);
            if vault.redact_monitor(&mut value).is_err() {
                value = serde_json::Value::String(String::new());
            }
            text = value.as_str().unwrap_or_default().to_owned();
        }
        lr_mcp::gateway::firewall::RequestPreview {
            text,
            total_bytes: preview.received,
            truncated: preview.truncated,
        }
    }

    /// Whether a model restriction or an active GuardRails policy has to read
    /// the request body, so a body that cannot be decoded must be refused.
    fn requires_readable_body(
        &self,
        config: &lr_config::AppConfig,
        client: &lr_config::Client,
        strategy: &lr_config::Strategy,
        provider: &str,
    ) -> bool {
        let (models, content) = self.restrictions(config, client, strategy, provider);
        models || content
    }

    /// Whether a model restriction applies to `provider`, and whether an
    /// active GuardRails policy has to judge the request content.
    fn restrictions(
        &self,
        config: &lr_config::AppConfig,
        client: &lr_config::Client,
        strategy: &lr_config::Strategy,
        provider: &str,
    ) -> (bool, bool) {
        use lr_config::PermissionState;
        let restricted_models = strategy.model_permissions.resolve_provider(provider)
            != PermissionState::Allow
            || strategy
                .model_permissions
                .models
                .iter()
                .any(|(name, permission)| {
                    name.starts_with(&format!("{provider}__"))
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
        (restricted_models, restricted_content)
    }

    /// The stream policy for a recognized LLM request that passed the
    /// header-time checks, or `None` when its body must be read whole:
    /// requests other than a POST of a chat/messages/responses call.
    /// Whether the model permission needs the whole body (Ask without a
    /// standing approval) is decided once the model is known
    /// ([`open_stream`](lr_proxy::active::Firewall::open_stream)).
    fn stream_policy(
        &self,
        config: &lr_config::AppConfig,
        client: &lr_config::Client,
        strategy: &lr_config::Strategy,
        head: &lr_proxy::interceptor::RequestHead<'_>,
    ) -> Option<lr_proxy::interceptor::StreamPolicy> {
        use lr_proxy::interceptor::{StreamPolicy, DEFAULT_STREAM_LAG};
        use lr_proxy::wire::WireFormat;
        // An encoding the proxy cannot decode is judged on the buffered path.
        if head.method != "POST"
            || head.content_encoding_error().is_some()
            || !matches!(
                lr_proxy::wire::detect(head.path),
                Some(
                    WireFormat::AnthropicMessages
                        | WireFormat::OpenAiChat
                        | WireFormat::OpenAiResponses
                        | WireFormat::Ollama(_)
                )
            )
        {
            return None;
        }
        let masking = masking_for(
            client
                .secret_scanning
                .action
                .as_ref()
                .unwrap_or(&config.secret_scanning.action),
        );
        // Active GuardRails stream too: `open_stream` holds the whole body
        // until their verdict (bodies within the lag stay buffered).
        let (restricted_models, _) = self.restrictions(config, client, strategy, &head.provider());
        let ask = strategy
            .auto_config
            .as_ref()
            .is_some_and(|a| a.permission == lr_config::PermissionState::Ask);
        Some(StreamPolicy {
            masking,
            needs_model: restricted_models || ask,
            whole_document: false,
            lag: DEFAULT_STREAM_LAG,
            restore_only: false,
            strict: config.secret_scanning.strict_streaming,
            incremental: false,
        })
    }

    /// The policy decision for a buffered request; an admitted one reserves
    /// its rate-limit share (kept with the exchange until its usage is
    /// recorded).
    async fn evaluate_policy(
        &self,
        req: &lr_proxy::active::FirewallRequest,
    ) -> lr_proxy::interceptor::RequestAction {
        use lr_proxy::interceptor::RequestAction;
        let action = self.judge_policy(req).await;
        if matches!(action, RequestAction::Reject { .. }) {
            return action;
        }
        let config = self.config_manager.get();
        let Some(strategy) = config
            .clients
            .iter()
            .find(|c| c.id == req.client_id)
            .and_then(|client| {
                config
                    .strategies
                    .iter()
                    .find(|s| s.id == client.strategy_id)
            })
        else {
            return RequestAction::reject_json(403, "Client policy is no longer available");
        };
        match self
            .admit(&req.client_id, strategy, req.trace.as_ref(), &req.admission)
            .await
        {
            Ok(()) => action,
            Err(refusal) => refusal,
        }
    }

    /// What a strategy's admitted requests hold, as admission counts it for a
    /// request with this trace: nothing for a duplicate hop, whose first hop
    /// holds the request's share.
    fn strategy_in_flight(
        &self,
        strategy_id: &str,
        trace: Option<&lr_types::RequestTrace>,
    ) -> lr_router::rate_limit::ReservedUsage {
        if trace.is_some_and(|t| t.is_duplicate()) {
            return Default::default();
        }
        self.state.rate_limiter.strategy_in_flight(strategy_id)
    }

    /// Reserve an admitted request's share of its client's and strategy's
    /// limits, checked and held atomically, and keep it in `slot`. The share
    /// is one request plus the strategy's average tokens and cost, as the
    /// check projects. A duplicate hop is checked against recorded usage only
    /// and holds nothing, as on the native path.
    async fn admit(
        &self,
        client_id: &str,
        strategy: &lr_config::Strategy,
        trace: Option<&lr_types::RequestTrace>,
        slot: &lr_proxy::interceptor::ExchangeAdmission,
    ) -> Result<(), lr_proxy::interceptor::RequestAction> {
        // The exchange's metrics row goes to the strategy decided under
        // now, the one the reservation comes from (a long-lived tunnel may
        // have been opened under another).
        slot.decided_under(&strategy.id);
        let admission =
            lr_types::with_outbound_trace(trace.cloned(), self.reserve(client_id, strategy))
                .await?;
        if admission.client.is_reserved() || admission.strategy.is_reserved() {
            slot.hold(Box::new(admission));
        }
        Ok(())
    }

    async fn reserve(
        &self,
        client_id: &str,
        strategy: &lr_config::Strategy,
    ) -> Result<ProxyAdmission, lr_proxy::interceptor::RequestAction> {
        use lr_proxy::interceptor::RequestAction;
        let limiter = &self.state.rate_limiter;
        let (avg_tokens, avg_cost) = self.metrics.get_pre_estimate_for_strategy(&strategy.id, 10);
        let estimate = lr_router::rate_limit::ReservedUsage {
            requests: 1,
            input_tokens: if avg_tokens.is_finite() && avg_tokens > 0.0 {
                avg_tokens.ceil() as u64
            } else {
                0
            },
            output_tokens: 0,
            cost_usd: Some(if avg_cost.is_finite() {
                avg_cost.max(0.0)
            } else {
                0.0
            }),
        };
        let client = match limiter
            .reserve_api_key(
                client_id,
                &lr_router::UsageInfo {
                    input_tokens: estimate.input_tokens,
                    output_tokens: 0,
                    cost_usd: estimate.cost_usd.unwrap_or(0.0),
                },
            )
            .await
        {
            Ok(Ok(reservation)) => reservation,
            // When room comes back, as the native router answers it.
            Ok(Err(check)) => {
                return Err(RequestAction::rate_limited(
                    "Rate limit exceeded (client)",
                    check
                        .retry_after_secs
                        .and_then(|secs| u64::try_from(secs).ok()),
                ))
            }
            Err(_) => {
                return Err(RequestAction::reject_json(
                    503,
                    "Rate limit state is unavailable",
                ))
            }
        };
        let strategy_share = if strategy.rate_limits.is_empty() {
            lr_router::AdmissionReservation::unreserved()
        } else {
            limiter
                .reserve_strategy(
                    &strategy.id,
                    estimate,
                    |in_flight| match rate_limit_exceeded(&self.metrics, strategy, in_flight) {
                        Some(denial) => Err(denial),
                        None => Ok(()),
                    },
                )
                .map_err(|denial| strategy_limited(&denial))?
        };
        Ok(ProxyAdmission {
            limiter: limiter.clone(),
            client_id: client_id.to_owned(),
            client,
            strategy: strategy_share,
        })
    }

    async fn judge_policy(
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

        // Rate limits (shared with the gateway), before any popup; the share
        // is reserved once the request is admitted (`evaluate_policy`).
        let in_flight = self.strategy_in_flight(&strategy.id, req.trace.as_ref());
        if let Some(denial) = rate_limit_exceeded(&self.metrics, strategy, &in_flight) {
            return strategy_limited(&denial);
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
        // Skips itself when the resolved policy allows every category. Text
        // attachments are judged decoded, as the gateway judges them.
        // A duplicate hop (a header this install signed: an earlier hop of
        // this request already ran GuardRails and asked for approval) is not
        // asked again; it reaches this policy only to protect secrets.
        let duplicate_hop = req.trace.as_ref().is_some_and(|t| t.is_duplicate());
        if duplicate_hop {
            // Nothing to ask: fall through to the protected body.
        } else if let lr_server::routes::pipeline::GuardrailScanOutcome::Deny(message) =
            lr_server::routes::pipeline::scan_request_for_guardrails(
                &self.state,
                &req.client_id,
                req.model.as_deref().unwrap_or_default(),
                &super::proxy_stream::guardrail_projection(&body),
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
        if permission == PermissionState::Ask
            && !has_time_bypass
            && !has_explicit_allow
            && !duplicate_hop
        {
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
    /// Listen host for the next bind (replaced when the configured host changes).
    host: Mutex<String>,
    interceptor: Arc<dyn lr_proxy::interceptor::ProxyInterceptor>,
    resolver: Arc<AppClientResolver>,
    dedupe: RequestDedupeFlag,
    usage: Arc<lr_usage::UsageTracker>,
    /// Where held request bytes spill, encrypted, beyond RAM.
    spill: Option<Arc<lr_proxy::spill::SpillConfig>>,
    /// Hosts that get transformed bodies with a Content-Length; learned
    /// hosts survive listener restarts.
    length_required: Arc<lr_proxy::framing::LengthRequiredHosts>,
    config: Option<lr_config::ConfigManager>,
    running: Mutex<Option<RunningProxy>>,
    /// Serializes start/stop/restart so two lifecycle requests cannot
    /// interleave (e.g. a server restart racing a config-driven rebind).
    lifecycle: tokio::sync::Mutex<()>,
}

struct RunningProxy {
    /// Host the listener is bound on.
    host: String,
    /// Port that was asked for (0 = OS-assigned), to tell whether the
    /// configured endpoint still matches what is bound.
    requested_port: u16,
    /// Port actually bound.
    port: u16,
    shutdown: tokio::sync::oneshot::Sender<()>,
    /// The accept loop; finished once the listening socket is released.
    task: tokio::task::JoinHandle<()>,
}

/// How long [`ProxyService::shutdown`] waits for the accept loop to release
/// the listening socket before giving up on it.
const SHUTDOWN_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// The disk budget for held (spilled, encrypted) request bodies, shared by
/// the HTTPS proxy and every reverse-proxy listener: each has its own spill
/// directory, but together they never hold more than one budget on disk.
pub(crate) fn spill_budget() -> Arc<lr_proxy::spill::SpillBudget> {
    static BUDGET: std::sync::OnceLock<Arc<lr_proxy::spill::SpillBudget>> =
        std::sync::OnceLock::new();
    BUDGET
        .get_or_init(|| {
            Arc::new(lr_proxy::spill::SpillBudget::new(
                lr_proxy::spill::DEFAULT_GLOBAL_BUDGET,
            ))
        })
        .clone()
}

/// A spill configuration in `dir` spending from the shared
/// [`spill_budget`], with what a crash left in `dir` removed (spill files
/// are unlinked when created; a platform without that can leave some).
pub(crate) fn shared_spill(dir: PathBuf, owner: &str) -> Arc<lr_proxy::spill::SpillConfig> {
    let spill = Arc::new(lr_proxy::spill::SpillConfig::shared(dir, spill_budget()));
    let stale = spill.cleanup_stale();
    if stale > 0 {
        tracing::info!("{owner}: removed {stale} stale spill file(s)");
    }
    spill
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
        let spill = shared_spill(dir.join("spill"), "proxy");
        // Learned hosts persist across restarts (host names only, bounded,
        // each expiring); configured ones follow the config (`apply_config`).
        let length_required = Arc::new(lr_proxy::framing::LengthRequiredHosts::persistent(
            config_manager.get().proxy.content_length_hosts.clone(),
            dir.join("length_required_hosts.json"),
        ));
        let config = Some(config_manager.clone());
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
            host: Mutex::new(host),
            interceptor: Arc::new(interceptor),
            resolver: Arc::new(AppClientResolver { client_manager }),
            dedupe,
            usage,
            spill: Some(spill),
            length_required,
            config,
            running: Mutex::new(None),
            lifecycle: tokio::sync::Mutex::new(()),
        })
    }

    /// A service with a throwaway CA under `ca_dir` and a recorder-only
    /// interceptor, for exercising the listener lifecycle without the app.
    #[cfg(test)]
    fn for_lifecycle_tests(ca_dir: &std::path::Path) -> Self {
        Self {
            ca: Arc::new(CertAuthority::load_or_create(ca_dir).unwrap()),
            host: Mutex::new("127.0.0.1".to_string()),
            interceptor: Arc::new(PassiveInterceptor::new(Arc::new(
                lr_monitor::MonitorEventStore::new(8),
            ))),
            resolver: Arc::new(AppClientResolver {
                client_manager: Arc::new(lr_clients::ClientManager::with_keychain(
                    vec![],
                    Arc::new(lr_api_keys::keychain_trait::MockKeychain::new()),
                )),
            }),
            dedupe: RequestDedupeFlag::new(false),
            usage: Arc::new(lr_usage::UsageTracker::new(Default::default())),
            spill: None,
            length_required: Default::default(),
            config: None,
            running: Mutex::new(None),
            lifecycle: tokio::sync::Mutex::new(()),
        }
    }

    /// Path to the root CA clients must trust (`NODE_EXTRA_CA_CERTS`).
    pub fn ca_cert_path(&self) -> PathBuf {
        self.ca.ca_cert_path().to_path_buf()
    }

    /// The bound port, if the proxy is currently running.
    #[cfg(test)]
    pub fn port(&self) -> Option<u16> {
        self.running.lock().as_ref().map(|r| r.port)
    }

    pub fn is_running(&self) -> bool {
        self.running.lock().is_some()
    }

    /// Start the listener on `port` (0 = OS-assigned). Idempotent: a no-op if
    /// already running. Returns the bound port.
    pub async fn start(&self, port: u16) -> AppResult<u16> {
        let _lifecycle = self.lifecycle.lock().await;
        self.start_locked(port).await
    }

    async fn start_locked(&self, port: u16) -> AppResult<u16> {
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
        .with_usage_tracker(self.usage.clone())
        .with_spill(self.spill.clone())
        .with_length_required(self.length_required.clone());
        if let Some(config) = &self.config {
            self.length_required
                .set_configured(config.get().proxy.content_length_hosts.clone());
        }

        let host = self.host.lock().clone();
        let listener = ProxyManager::bind(&host, port)
            .await
            .map_err(|e| AppError::Internal(format!("proxy bind {host}:{port}: {e}")))?;
        let bound = listener
            .local_addr()
            .map_err(|e| AppError::Internal(format!("proxy local_addr: {e}")))?
            .port();

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            manager
                .serve(listener, async {
                    let _ = rx.await;
                })
                .await;
        });

        tracing::info!("HTTPS inspection proxy listening on {host}:{bound}");
        *self.running.lock() = Some(RunningProxy {
            host,
            requested_port: port,
            port: bound,
            shutdown: tx,
            task,
        });
        Ok(bound)
    }

    /// Signal the listener to stop, without waiting for the socket to be
    /// released. For app exit, where nothing rebinds afterwards and the
    /// caller may not be able to await.
    pub fn stop(&self) {
        if let Some(r) = self.running.lock().take() {
            let _ = r.shutdown.send(());
            tracing::info!("HTTPS inspection proxy stopped");
        }
    }

    /// Stop the listener and wait until the accept loop has released the
    /// socket, so the same port can be bound again straight away. Connections
    /// already being served run to completion on their own tasks. The app
    /// itself only needs this as part of [`Self::restart`] / [`Self::ensure`];
    /// at exit it uses [`Self::stop`].
    #[cfg(test)]
    pub async fn shutdown(&self) {
        let _lifecycle = self.lifecycle.lock().await;
        self.shutdown_locked().await;
    }

    async fn shutdown_locked(&self) {
        let Some(running) = self.running.lock().take() else {
            return;
        };
        let _ = running.shutdown.send(());
        if tokio::time::timeout(SHUTDOWN_WAIT, running.task)
            .await
            .is_err()
        {
            tracing::warn!(
                "HTTPS inspection proxy accept loop did not stop within {SHUTDOWN_WAIT:?}"
            );
        }
        tracing::info!("HTTPS inspection proxy stopped");
    }

    /// Stop the listener (if any) and bind it again on `host:port`. Returns
    /// the bound port. On a bind failure the proxy is left stopped.
    pub async fn restart(&self, host: &str, port: u16) -> AppResult<u16> {
        let _lifecycle = self.lifecycle.lock().await;
        self.shutdown_locked().await;
        *self.host.lock() = host.to_string();
        self.start_locked(port).await
    }

    /// Make the listener match the configured endpoint: rebinds when the host
    /// or requested port differs from what is running, or when nothing is
    /// running (an earlier bind may have failed). Returns `true` when the
    /// listener was (re)bound, i.e. client proxy URLs may have changed.
    pub async fn ensure(&self, host: &str, port: u16) -> AppResult<bool> {
        let _lifecycle = self.lifecycle.lock().await;
        let matches = self
            .running
            .lock()
            .as_ref()
            .is_some_and(|r| r.host == host && r.requested_port == port);
        if matches {
            return Ok(false);
        }
        self.shutdown_locked().await;
        *self.host.lock() = host.to_string();
        self.start_locked(port).await.map(|_| true)
    }

    /// Apply the parts of the app config the running listener reads per
    /// request (the hosts that get transformed bodies with a length), so a
    /// change applies without restarting the listener.
    pub fn apply_config(&self, config: &lr_config::AppConfig) {
        self.length_required
            .set_configured(config.proxy.content_length_hosts.clone());
    }

    /// The `HTTPS_PROXY` URL for a client (embeds Basic auth).
    pub fn client_proxy_url(&self, client_id: &str, secret: &str) -> Option<String> {
        let running = self.running.lock();
        running
            .as_ref()
            .map(|r| proxy_url(&r.host, r.port, client_id, secret))
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

    async fn port_accepts(port: u16) -> bool {
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
    }

    #[tokio::test]
    async fn content_length_hosts_apply_to_a_running_listener() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ProxyService::for_lifecycle_tests(dir.path());
        svc.start(0).await.unwrap();
        assert!(!svc.length_required.requires("api.example.com"));
        let mut config = lr_config::AppConfig::default();
        config.proxy.content_length_hosts = vec!["API.example.com".into()];
        svc.apply_config(&config);
        // The listener reads the same shared set per request: no restart.
        assert!(svc.is_running());
        assert!(svc.length_required.requires("api.example.com"));
        config.proxy.content_length_hosts.clear();
        svc.apply_config(&config);
        assert!(!svc.length_required.requires("api.example.com"));
        svc.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_releases_the_port_and_clears_the_client_url() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ProxyService::for_lifecycle_tests(dir.path());
        assert!(svc.client_proxy_url("cid", "sec").is_none());

        let port = svc.start(0).await.unwrap();
        assert!(svc.is_running());
        assert!(port_accepts(port).await);
        assert_eq!(
            svc.client_proxy_url("cid", "sec").unwrap(),
            format!("http://cid:sec@127.0.0.1:{port}")
        );

        svc.shutdown().await;
        assert!(!svc.is_running());
        assert!(svc.client_proxy_url("cid", "sec").is_none());
        // The socket is released by the time shutdown returns, so the same
        // port can be bound again immediately.
        assert!(!port_accepts(port).await);
        let rebound = tokio::net::TcpListener::bind(("127.0.0.1", port)).await;
        assert!(rebound.is_ok(), "port {port} was still held after shutdown");
    }

    #[tokio::test]
    async fn restart_rebinds_the_same_port_without_a_stale_listener() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ProxyService::for_lifecycle_tests(dir.path());
        let port = svc.start(0).await.unwrap();

        // Restarting onto the very port that is in use only works if the old
        // listener is really gone first.
        let again = svc.restart("127.0.0.1", port).await.unwrap();
        assert_eq!(again, port);
        assert!(svc.is_running());
        assert!(port_accepts(port).await);
        svc.shutdown().await;
    }

    #[tokio::test]
    async fn ensure_rebinds_only_when_the_configured_endpoint_changes() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ProxyService::for_lifecycle_tests(dir.path());

        // Nothing running yet: ensure starts it.
        assert!(svc.ensure("127.0.0.1", 0).await.unwrap());
        let first = svc.port().unwrap();

        // Same configured endpoint: left alone, same socket.
        assert!(!svc.ensure("127.0.0.1", 0).await.unwrap());
        assert_eq!(svc.port(), Some(first));

        // A different configured port moves the listener; the old one is gone.
        let free = {
            let probe = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            probe.local_addr().unwrap().port()
        };
        assert!(svc.ensure("127.0.0.1", free).await.unwrap());
        assert_eq!(svc.port(), Some(free));
        assert!(!port_accepts(first).await);
        assert!(port_accepts(free).await);
        svc.shutdown().await;
    }

    #[tokio::test]
    async fn a_failed_rebind_leaves_the_proxy_stopped_and_retryable() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ProxyService::for_lifecycle_tests(dir.path());
        svc.start(0).await.unwrap();

        let blocker = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let taken = blocker.local_addr().unwrap().port();
        assert!(svc.restart("127.0.0.1", taken).await.is_err());
        assert!(!svc.is_running());
        assert!(svc.client_proxy_url("cid", "sec").is_none());

        // Once the port frees up, the next ensure brings the proxy back.
        drop(blocker);
        assert!(svc.ensure("127.0.0.1", taken).await.unwrap());
        assert_eq!(svc.port(), Some(taken));
        svc.shutdown().await;
    }
}

/// Header-time policy: every refusal `body_policy` gives must be one
/// `evaluate` reaches too, and anything that reads the body stays buffered.
#[cfg(test)]
mod body_policy_tests {
    use super::secret_policy_tests::fixture;
    use super::*;
    use lr_proxy::{
        active::{Firewall, FirewallRequest},
        interceptor::{BodyPolicy, ExchangeSource, RequestAction, RequestHead},
    };

    fn policy(
        firewall: &AppFirewall,
        request: &FirewallRequest,
        headers: &hyper::HeaderMap,
    ) -> BodyPolicy {
        firewall.body_policy(&RequestHead {
            client_id: &request.client_id,
            strategy_id: "",
            host: &request.host,
            port: 443,
            method: &request.method,
            path: &request.path,
            headers,
            trace: None,
            provider_override: None,
            source: ExchangeSource::Proxy,
        })
    }

    fn setup(root: &std::path::Path) -> (AppFirewall, FirewallRequest) {
        fixture(
            root,
            lr_config::LlmMode::Proxy,
            Arc::new(lr_api_keys::MockKeychain::new()),
        )
    }

    fn rejects(policy: &BodyPolicy, status: u16, message: &str) -> bool {
        match policy {
            BodyPolicy::Reject(s, m) => *s == status && m.contains(message),
            BodyPolicy::RateLimited { message: m, .. } => status == 429 && m.contains(message),
            _ => false,
        }
    }

    /// The buffered path refuses the same request, with the same status.
    async fn evaluate_rejects(firewall: &AppFirewall, request: &FirewallRequest, status: u16) {
        match firewall.evaluate(request).await {
            RequestAction::Reject { status: s, .. } => assert_eq!(s, status),
            _ => panic!("evaluate must refuse what body_policy refuses"),
        }
    }

    fn streams(masking: lr_proxy::interceptor::StreamMasking, needs_model: bool) -> BodyPolicy {
        BodyPolicy::Stream(lr_proxy::interceptor::StreamPolicy {
            masking,
            needs_model,
            whole_document: false,
            lag: lr_proxy::interceptor::DEFAULT_STREAM_LAG,
            restore_only: false,
            strict: false,
            incremental: false,
        })
    }

    #[tokio::test]
    async fn inspected_requests_stream_unless_a_consumer_needs_the_whole_body() {
        use lr_proxy::interceptor::StreamMasking;
        let root = tempfile::tempdir().unwrap();
        let (firewall, request) = setup(root.path());
        let headers = hyper::HeaderMap::new();
        // Redact (the fixture default), Off and Notify may stream; Ask's
        // popups need the whole request.
        assert_eq!(
            policy(&firewall, &request, &headers),
            streams(StreamMasking::Redact, false)
        );
        for (action, expected) in [
            (
                lr_config::SecretScanAction::Off,
                streams(StreamMasking::Off, false),
            ),
            (
                lr_config::SecretScanAction::Notify,
                streams(StreamMasking::Notify, false),
            ),
            // Ask streams too: each finding holds the upload until answered.
            (
                lr_config::SecretScanAction::Ask,
                streams(StreamMasking::Ask, false),
            ),
        ] {
            firewall
                .config_manager
                .update(|c| c.secret_scanning.action = action)
                .unwrap();
            assert_eq!(policy(&firewall, &request, &headers), expected);
        }
        firewall
            .config_manager
            .update(|c| c.secret_scanning.action = lr_config::SecretScanAction::Redact)
            .unwrap();
        // Model Ask: whether its popup needs the body is decided once the
        // model is known.
        firewall
            .config_manager
            .update(|c| {
                c.strategies[0]
                    .auto_config
                    .get_or_insert_with(Default::default)
                    .permission = lr_config::PermissionState::Ask;
            })
            .unwrap();
        assert_eq!(
            policy(&firewall, &request, &headers),
            streams(StreamMasking::Redact, true)
        );
        // A supported encoding is decoded while streaming, not refused.
        let mut encoded = hyper::HeaderMap::new();
        encoded.insert(hyper::header::CONTENT_ENCODING, "zstd".parse().unwrap());
        assert_eq!(
            policy(&firewall, &request, &encoded),
            streams(StreamMasking::Redact, true)
        );
        // Ollama's native chat and generate stream (the reverse proxy
        // wrapping a local Ollama); its other endpoints are not inspected.
        let mut request = request;
        for path in ["/api/chat", "/api/generate"] {
            request.path = path.into();
            assert_eq!(
                policy(&firewall, &request, &headers),
                streams(StreamMasking::Redact, true),
                "{path}"
            );
        }
        request.path = "/api/embed".into();
        assert_eq!(policy(&firewall, &request, &headers), BodyPolicy::Buffered);
        // Only POSTs of conversation calls stream.
        request.path = "/v1/messages".into();
        request.method = "GET".into();
        assert_eq!(policy(&firewall, &request, &headers), BodyPolicy::Buffered);
    }

    #[tokio::test]
    async fn unknown_disabled_and_strategyless_clients_are_refused_from_the_head() {
        let root = tempfile::tempdir().unwrap();
        let (firewall, request) = setup(root.path());
        let headers = hyper::HeaderMap::new();

        let unknown = FirewallRequest {
            client_id: "no-such-client".into(),
            session_id: None,
            secret_session: None,
            host: request.host.clone(),
            method: request.method.clone(),
            path: request.path.clone(),
            provider: request.provider.clone(),
            model: request.model.clone(),
            has_tools: false,
            message_count: 1,
            body: request.body.clone(),
            trace: None,
            admission: Default::default(),
        };
        assert!(rejects(
            &policy(&firewall, &unknown, &headers),
            403,
            "Client policy is no longer available"
        ));
        evaluate_rejects(&firewall, &unknown, 403).await;

        firewall
            .config_manager
            .update(|c| c.clients[0].enabled = false)
            .unwrap();
        assert!(rejects(
            &policy(&firewall, &request, &headers),
            403,
            "Proxy access is disabled"
        ));
        evaluate_rejects(&firewall, &request, 403).await;
        firewall
            .config_manager
            .update(|c| c.clients[0].enabled = true)
            .unwrap();

        // A client whose strategy no longer exists. Built directly, because a
        // validated update refuses to create that state.
        let mut firewall = firewall;
        let mut config = firewall.config_manager.get();
        config.strategies.clear();
        firewall.config_manager =
            lr_config::ConfigManager::new(config, root.path().join("strategyless.yaml"));
        assert!(rejects(
            &policy(&firewall, &request, &headers),
            403,
            "Client strategy is no longer available"
        ));
        evaluate_rejects(&firewall, &request, 403).await;
    }

    #[tokio::test]
    async fn clients_outside_a_proxy_mode_are_refused_from_the_head() {
        let root = tempfile::tempdir().unwrap();
        let (firewall, request) = fixture(
            root.path(),
            lr_config::LlmMode::Gateway,
            Arc::new(lr_api_keys::MockKeychain::new()),
        );
        assert!(rejects(
            &policy(&firewall, &request, &hyper::HeaderMap::new()),
            403,
            "Proxy access is disabled"
        ));
        evaluate_rejects(&firewall, &request, 403).await;
    }

    #[tokio::test]
    async fn disabled_llm_access_is_refused_from_the_head_except_for_cleanup_calls() {
        let root = tempfile::tempdir().unwrap();
        let (firewall, mut request) = setup(root.path());
        let headers = hyper::HeaderMap::new();
        firewall
            .config_manager
            .update(|c| {
                c.strategies[0]
                    .auto_config
                    .get_or_insert_with(Default::default)
                    .permission = lr_config::PermissionState::Off;
            })
            .unwrap();
        assert!(rejects(
            &policy(&firewall, &request, &headers),
            403,
            "LLM access is disabled"
        ));
        evaluate_rejects(&firewall, &request, 403).await;

        // Reading a stored response is refused as well.
        request.body = serde_json::Value::Null;
        request.method = "GET".into();
        request.path = "/backend-api/codex/responses/resp_stored".into();
        assert!(rejects(
            &policy(&firewall, &request, &headers),
            403,
            "LLM access is disabled"
        ));
        evaluate_rejects(&firewall, &request, 403).await;

        // Deleting or cancelling one is not refused by `evaluate`, so the
        // head alone must not refuse it either.
        request.method = "DELETE".into();
        assert_eq!(policy(&firewall, &request, &headers), BodyPolicy::Buffered);
        assert!(!matches!(
            firewall.evaluate(&request).await,
            RequestAction::Reject { .. }
        ));
        request.method = "POST".into();
        request.path = "/backend-api/codex/responses/resp_stored/cancel?reason=user".into();
        assert_eq!(policy(&firewall, &request, &headers), BodyPolicy::Buffered);
        assert!(!matches!(
            firewall.evaluate(&request).await,
            RequestAction::Reject { .. }
        ));
    }

    #[tokio::test]
    async fn an_exceeded_rate_limit_is_refused_from_the_head_but_not_for_retrievals() {
        let root = tempfile::tempdir().unwrap();
        let (mut firewall, mut request) = setup(root.path());
        let headers = hyper::HeaderMap::new();
        // A limit of zero is exceeded by the next request without recording
        // any traffic. Built directly, because a validated update refuses it.
        let mut config = firewall.config_manager.get();
        config.strategies[0].rate_limits = vec![lr_config::StrategyRateLimit {
            limit_type: lr_config::RateLimitType::Requests,
            value: 0.0,
            time_window: lr_config::RateLimitTimeWindow::Minute,
            enabled: true,
        }];
        firewall.config_manager =
            lr_config::ConfigManager::new(config, root.path().join("rate-limited.yaml"));
        assert!(rejects(
            &policy(&firewall, &request, &headers),
            429,
            "Rate limit exceeded (requests per minute)"
        ));
        // When room returns: a limit of zero never has any, so a whole
        // window from the current minute.
        let BodyPolicy::RateLimited {
            retry_after, code, ..
        } = policy(&firewall, &request, &headers)
        else {
            panic!("a rate-limit refusal");
        };
        assert!((60..=120).contains(&retry_after), "{retry_after}");
        assert_eq!(code.as_deref(), Some("strategy_rate_limit_exceeded"));
        evaluate_rejects(&firewall, &request, 429).await;
        let RequestAction::Reject {
            retry_after, body, ..
        } = firewall.evaluate(&request).await
        else {
            panic!("a rate-limit refusal");
        };
        assert!(retry_after.is_some_and(|secs| (60..=120).contains(&secs)));
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"]["code"], "strategy_rate_limit_exceeded");
        assert_eq!(
            body["error"]["retry_after_secs"].as_u64(),
            retry_after,
            "{body}"
        );

        request.body = serde_json::Value::Null;
        request.method = "GET".into();
        request.path = "/backend-api/codex/responses/resp_stored".into();
        assert_eq!(policy(&firewall, &request, &headers), BodyPolicy::Buffered);
        assert!(!matches!(
            firewall.evaluate(&request).await,
            RequestAction::Reject { .. }
        ));
    }

    #[tokio::test]
    async fn an_undecodable_encoding_is_refused_from_the_head_only_while_restricted() {
        let root = tempfile::tempdir().unwrap();
        let (firewall, mut request) = setup(root.path());
        let mut headers = hyper::HeaderMap::new();
        headers.insert(hyper::header::CONTENT_ENCODING, "snappy".parse().unwrap());
        // Unrestricted: the buffered path forwards it with a warning.
        assert_eq!(policy(&firewall, &request, &headers), BodyPolicy::Buffered);

        firewall
            .config_manager
            .update(|c| {
                c.strategies[0].model_permissions.models.insert(
                    "openai__forbidden-model".into(),
                    lr_config::PermissionState::Off,
                );
            })
            .unwrap();
        assert!(rejects(
            &policy(&firewall, &request, &headers),
            415,
            "require a readable request body"
        ));
        request.body = serde_json::Value::Null;
        request.model = None;
        match firewall
            .evaluate_uninspectable(&request, lr_proxy::request_body::DecodeError::Unsupported)
            .await
        {
            RequestAction::Reject { status, .. } => assert_eq!(status, 415),
            _ => panic!("the buffered path must refuse it too"),
        }
        // The restriction is per provider: another provider's is irrelevant.
        request.host = "api.anthropic.com".into();
        assert_eq!(policy(&firewall, &request, &headers), BodyPolicy::Buffered);
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
            trace: None,
            admission: Default::default(),
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
    async fn mcp_result_private_keys_are_protected_before_any_llm_transport() {
        use lr_types::secret_context::SecretContext;
        let material = "whtWShJK3py2Nh1bWey7TsPWCMmmPRsRIs0cg240wR5nPxMgnPFkNj6Dl0IbSfxxVikEf/GcBB/TEZ5PhCH5a2Px9L8ihbSWxS+T2qPjUrE3Xtt5QSqydTGneV72XJxG";
        let key = format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{material}\n-----END OPENSSH PRIVATE KEY-----"
        );
        let mcp = json!({"content":[{"type":"text","text":json!({"private_key":key}).to_string()}],"isError":false}).to_string();
        for (path, provider, body) in [
            (
                "/backend-api/codex/responses",
                "openai",
                json!({"model":"local-test","input":[{"type":"function_call_output","call_id":"mcp-1","output":mcp}]}),
            ),
            (
                "/v1/chat/completions",
                "openai",
                json!({"model":"local-test","messages":[{"role":"tool","tool_call_id":"mcp-1","content":mcp}]}),
            ),
            (
                "/v1/messages",
                "anthropic",
                json!({"model":"local-test","messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"mcp-1","content":[{"type":"text","text":mcp}]}]}]}),
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            let (firewall, mut request) = fixture(
                root.path(),
                lr_config::LlmMode::Proxy,
                Arc::new(lr_api_keys::MockKeychain::new()),
            );
            request.path = path.into();
            request.provider = provider.into();
            request.body = body.clone();
            let RequestAction::Protect {
                body: protected,
                vault,
            } = firewall.evaluate(&request).await
            else {
                panic!("MCP result must proceed with protection on {path}")
            };
            assert!(
                !String::from_utf8_lossy(&protected).contains(material),
                "no private material may remain in forwarded MCP results on {path}"
            );
            assert!(String::from_utf8_lossy(&protected).contains("localrouter-redacted"));
            let mut restored: Value = serde_json::from_slice(&protected).unwrap();
            vault.restore(&mut restored, false).unwrap();
            assert_eq!(
                restored, body,
                "MCP result structure and escaping must round-trip"
            );
        }
    }

    #[tokio::test]
    async fn descriptive_names_do_not_create_secret_mappings() {
        let root = tempfile::tempdir().unwrap();
        let storage = Arc::new(lr_api_keys::MockKeychain::new());
        let (firewall, mut request) =
            fixture(root.path(), lr_config::LlmMode::Proxy, storage.clone());
        let name = "campaignSetupAndManagementContacts";
        request.body = json!({"model":"local-test","input":[{"role":"user","content":format!("token='{name}'")} ]});
        let RequestAction::Protect { body, .. } = firewall.evaluate(&request).await else {
            panic!("descriptive name must proceed")
        };
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            request.body
        );
        assert!(
            lr_secret_scanner::placeholders::Vault::list_client(storage, &request.client_id)
                .unwrap()
                .is_empty()
        );
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
    async fn standalone_search_detects_new_query_secrets_with_optional_conversation_input() {
        use lr_types::secret_context::SecretContext;
        let secret = "Z9v4qW8rT2yU6iO0pA5sD1fG3hJ7kL9zX2cV4bNQ";
        for context in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (firewall, mut request) = fixture(
                root.path(),
                lr_config::LlmMode::Proxy,
                Arc::new(lr_api_keys::MockKeychain::new()),
            );
            request.path = "/backend-api/codex/alpha/search".into();
            request.body = json!({"id":"search-session","model":"local-test","commands":{"search_query":[{"q":format!("aws_secret_access_key={secret}")}],"open":[{"ref_id":"turn0search0"}]},"settings":{"external_web_access":true}});
            if context {
                request.body["input"] = json!([{"type":"message","role":"user","content":[{"type":"input_text","text":format!("aws_secret_access_key={secret}")}]}]);
            }
            let RequestAction::Protect { body, vault } = firewall.evaluate(&request).await else {
                panic!("search must be protected without rejection")
            };
            let masked: Value = serde_json::from_slice(&body).unwrap();
            assert!(!String::from_utf8_lossy(&body).contains(secret));
            assert_eq!(masked["id"], "search-session");
            assert_eq!(masked["commands"]["open"][0]["ref_id"], "turn0search0");
            let tag = masked["commands"]["search_query"][0]["q"]
                .as_str()
                .unwrap()
                .strip_prefix("aws_secret_access_key=")
                .unwrap();
            let mut response = json!({"encrypted_output":tag,"output":tag,"results":[]});
            vault.restore(&mut response, false).unwrap();
            assert_eq!(response["output"], secret);
            assert_eq!(response["encrypted_output"], tag);
        }
    }
    #[tokio::test]
    async fn codex_image_prompts_are_scanned_and_redacted() {
        for path in [
            "/backend-api/codex/images/edits",
            "/backend-api/codex/images/generations",
        ] {
            let root = tempfile::tempdir().unwrap();
            let (firewall, mut request) = fixture(
                root.path(),
                lr_config::LlmMode::Proxy,
                Arc::new(lr_api_keys::MockKeychain::new()),
            );
            request.path = path.into();
            request.body = json!({"images":[{"image_url":"data:image/png;base64,Zm9v"},{"file_id":"file-image"}],"prompt":format!("draw a sign reading aws_secret_access_key={KEY}"),"model":"gpt-image-1.5"});
            let RequestAction::Protect { body, .. } = firewall.evaluate(&request).await else {
                panic!("{path}: image prompt with a secret must be protected")
            };
            let masked: Value = serde_json::from_slice(&body).unwrap();
            assert!(!String::from_utf8_lossy(&body).contains(KEY), "{path}");
            assert_eq!(masked["images"][1]["file_id"], "file-image");
            assert_eq!(masked["model"], "gpt-image-1.5");
        }
    }
    #[tokio::test]
    async fn standalone_search_scanner_unavailable_and_invalid_bodies_do_not_block() {
        let root = tempfile::tempdir().unwrap();
        let (firewall, mut request) = fixture(
            root.path(),
            lr_config::LlmMode::Proxy,
            Arc::new(lr_api_keys::MockKeychain::new()),
        );
        request.path = "/backend-api/codex/alpha/search".into();
        *firewall.state.secret_scanner.write() = None;
        for body in [
            Value::Null,
            json!({"model":"local-test","commands":[]}),
            json!({"model":"local-test","commands":{"search_query":[{"q":"password=generated-unavailable-test-value"}]}}),
        ] {
            request.body = body.clone();
            match firewall.evaluate(&request).await {
                RequestAction::Forward => {}
                RequestAction::Protect { body: bytes, .. } => {
                    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), body)
                }
                _ => panic!("an internal Secret Scan failure must not reject a search"),
            }
        }
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

#[cfg(test)]
#[path = "proxy_stream_tests.rs"]
mod stream_tests;

#[cfg(test)]
#[path = "proxy_guardrails_tests.rs"]
mod guardrails_tests;

#[cfg(test)]
#[path = "proxy_admission_tests.rs"]
mod admission_tests;

#[cfg(test)]
#[path = "proxy_monitor_mask_tests.rs"]
mod monitor_mask_tests;
