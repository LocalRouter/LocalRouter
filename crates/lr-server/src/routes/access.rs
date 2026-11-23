//! Who may use a native LLM route, decided before its content is read.
//!
//! Every refusal here depends on the caller and the requested model only:
//! the client is found and enabled, its LLM mode is the native gateway, its
//! strategy exists, allows model access and the model, the route's native
//! operation or ranked search is authorized, and the API key is within its
//! rate limits. The route handlers make these checks through these
//! functions, and the Secret Scan boundary makes the same checks
//! ([`precheck`]) before it scans a request: a request that will be refused
//! opens no Secret Scan popup and records no unscanned content.
//!
//! Interactive approvals (the auto-router popup, a model's Ask permission)
//! are not here: they stay in the handlers, after the Secret Scan.
use axum::Extension;
use serde_json::Value;

use super::helpers::{
    check_llm_access_with_state, check_strategy_permission, get_client_with_strategy,
    get_enabled_client, validate_strategy_model_access,
};
use crate::middleware::client_auth::ClientAuthContext;
use crate::middleware::error::{ApiErrorResponse, ApiResult};
use crate::state::{AppState, AuthContext};
use lr_config::Strategy;

const AUTO: &str = "localrouter/auto";

/// The client exists, is enabled, and may call the native LLM endpoints.
pub(crate) fn client_llm_access(state: &AppState, client_id: &str) -> ApiResult<()> {
    let client = get_enabled_client(state, client_id)?;
    check_llm_access_with_state(state, &client)
}

/// `model`, with the strategy's custom auto-model name replaced by
/// `localrouter/auto` (unchanged when the client or strategy is missing).
pub(crate) fn normalized_model(state: &AppState, client_id: &str, model: &str) -> String {
    match get_client_with_strategy(state, client_id) {
        Ok((_, strategy)) => alias(&strategy, model),
        Err(_) => model.to_string(),
    }
}

fn alias(strategy: &Strategy, model: &str) -> String {
    if strategy
        .auto_config
        .as_ref()
        .is_some_and(|auto| auto.model_name == model)
    {
        AUTO.to_string()
    } else {
        model.to_string()
    }
}

/// The client's strategy exists and allows model access and, unless it is
/// auto-routed, `model`.
pub(crate) fn strategy_model_access(
    state: &AppState,
    client_id: &str,
    model: &str,
) -> ApiResult<()> {
    let (_, strategy) = get_client_with_strategy(state, client_id)?;
    check_strategy_permission(&strategy)?;
    if alias(&strategy, model) != AUTO {
        validate_strategy_model_access(state, &strategy, model)?;
    }
    Ok(())
}

/// The refusals of the chat-family model access checks
/// (`pipeline::apply_model_access_checks`): the client and its strategy,
/// auto routing configured with models for chat, the strategy permission,
/// the model allow-list (unless auto-routed) and the client's LLM mode.
pub(crate) fn chat_model_access(state: &AppState, client_id: &str, model: &str) -> ApiResult<()> {
    let (client, strategy) = get_client_with_strategy(state, client_id)?;
    let model = alias(&strategy, model);
    if model == AUTO {
        match &strategy.auto_config {
            Some(auto) if !auto.has_chat_candidates() => {
                return Err(ApiErrorResponse::bad_request(
                    "Auto routing has no models for requests that use the default route: \
                     add models to the priority list or to the routing policy's default route",
                ));
            }
            Some(_) => {}
            None => {
                return Err(ApiErrorResponse::not_found(
                    "Auto routing is not configured for this client",
                ));
            }
        }
    }
    check_strategy_permission(&strategy)?;
    if model != AUTO {
        validate_strategy_model_access(state, &strategy, &model)?;
    }
    check_llm_access_with_state(state, &client)
}

/// The refusals of the System One model access checks: the client's
/// strategy exists, auto routing is configured with prioritized models (not
/// checked again on a duplicate hop), the strategy permission and the model
/// allow-list.
pub(crate) fn systemone_model_access(
    state: &AppState,
    client_id: &str,
    model: Option<&str>,
    is_duplicate: bool,
) -> ApiResult<()> {
    let (_, strategy) = get_client_with_strategy(state, client_id)?;
    let model = model.map(|model| alias(&strategy, model));
    let is_auto = model.as_deref() == Some(AUTO);
    if is_auto && !is_duplicate {
        let Some(auto) = strategy.auto_config.as_ref() else {
            return Err(ApiErrorResponse::not_found(
                "Auto routing is not configured for this client",
            ));
        };
        if auto.prioritized_models.is_empty() {
            return Err(ApiErrorResponse::bad_request(
                "Auto routing has no prioritized models configured",
            ));
        }
    }
    check_strategy_permission(&strategy)?;
    if let Some(model) = model.as_deref().filter(|model| *model != AUTO) {
        validate_strategy_model_access(state, &strategy, model)?;
    }
    Ok(())
}

/// Image generation and edits: the client must be enabled and allowed LLM
/// access (the internal Try It Out token skips this), and its strategy must
/// exist and allow `model`.
pub(crate) fn image_access(state: &AppState, auth: &AuthContext, model: &str) -> ApiResult<()> {
    image_client_access(state, auth)?;
    let (_, strategy) = get_client_with_strategy(state, &auth.api_key_id)?;
    check_strategy_permission(&strategy)?;
    validate_strategy_model_access(state, &strategy, model)
}

fn image_client_access(state: &AppState, auth: &AuthContext) -> ApiResult<()> {
    if auth.api_key_id == "internal-test" {
        return Ok(());
    }
    client_llm_access(state, &auth.api_key_id)
}

/// The pre-flight rate-limit check of the API key: refused while any of its
/// limits is reached (the limiter does not use an estimate here). Nothing is
/// reserved; the router reserves at dispatch.
pub(crate) async fn rate_limit(state: &AppState, api_key_id: &str) -> ApiResult<()> {
    let result = state
        .rate_limiter
        .check_api_key(
            api_key_id,
            &lr_router::UsageInfo {
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: 0.0,
            },
        )
        .await
        .map_err(|e| ApiErrorResponse::internal_error(format!("Rate limit check failed: {}", e)))?;
    if result.allowed {
        return Ok(());
    }
    let mut error = ApiErrorResponse::rate_limited(format!(
        "Rate limit exceeded: {}/{} used",
        result.current_usage, result.limit
    ));
    if let Some(retry_after) = result.retry_after_secs {
        error.error.error.code = Some(format!("retry_after_{}", retry_after));
    }
    Err(error)
}

/// How a refusal is recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefusalKind {
    /// Answered before the handler records the call (route authorization,
    /// model resolution): nothing beyond what the check itself records.
    Route,
    /// Access or model permission: an LLM call event with the error.
    Access,
    /// Rate limit: a rate-limit event and an LLM call event with the error.
    RateLimit,
}

/// A request the route refuses before reading its content.
#[derive(Debug)]
pub(crate) struct Refusal {
    pub error: ApiErrorResponse,
    pub kind: RefusalKind,
}

impl Refusal {
    fn route(error: impl Into<ApiErrorResponse>) -> Self {
        Self {
            error: error.into(),
            kind: RefusalKind::Route,
        }
    }

    fn access(error: ApiErrorResponse) -> Self {
        Self {
            error,
            kind: RefusalKind::Access,
        }
    }

    /// Record the refusal as the route's handler records its own, with the
    /// request content left out: it was not scanned. Returns the error.
    pub(crate) fn record(
        self,
        state: &AppState,
        client_auth: Option<&Extension<ClientAuthContext>>,
        endpoint: &str,
        model: &str,
        stream: bool,
    ) -> ApiErrorResponse {
        if self.kind == RefusalKind::Route {
            return self.error;
        }
        let session = super::monitor_helpers::new_request_session();
        if self.kind == RefusalKind::RateLimit {
            super::monitor_helpers::emit_rate_limit_event(
                state,
                client_auth,
                Some(&session),
                "rate_limit_exceeded",
                endpoint,
                &self.error.error.error.message,
                429,
                None,
            );
        }
        super::monitor_helpers::emit_llm_call(
            state,
            client_auth,
            Some(&session),
            endpoint,
            model,
            stream,
            &serde_json::json!({ "model": model }),
        )
        .capture_err(self.error)
    }
}

async fn rate_limited(state: &AppState, api_key_id: &str) -> Result<(), Refusal> {
    rate_limit(state, api_key_id)
        .await
        .map_err(|error| Refusal {
            error,
            kind: RefusalKind::RateLimit,
        })
}

/// The checks of `pipeline::run_turn_pipeline` before its Secret Scan:
/// privacy auxiliaries, the chat model access refusals (when the request
/// names a model), and the rate limit (not on a duplicate hop).
async fn turn_admission(
    state: &AppState,
    client_id: &str,
    model: Option<&str>,
) -> Result<(), Refusal> {
    crate::privacy::check_auxiliaries(state, client_id).map_err(Refusal::access)?;
    if let Some(model) = model {
        chat_model_access(state, client_id, model).map_err(Refusal::access)?;
    }
    if !lr_types::is_duplicate_hop() {
        rate_limited(state, client_id).await?;
    }
    Ok(())
}

/// `POST /v1/videos` after its native operation was authorized: the turn
/// pipeline's refusals for `model`. Multipart video runs this before it
/// scans its form.
pub(crate) async fn video_admission(
    state: &AppState,
    client_id: &str,
    model: &str,
) -> Result<(), Refusal> {
    turn_admission(state, client_id, Some(model)).await
}

/// A model named without its provider, resolved as the Messages handlers
/// resolve it.
async fn messages_model(state: &AppState, client_id: &str, model: &str) -> Result<String, Refusal> {
    if model.contains('/') {
        return Ok(model.to_string());
    }
    state
        .router
        .resolve_messages_model(client_id, model)
        .await
        .map_err(Refusal::route)
}

/// The refusals the handler of `path` (without its `/v1` prefix) makes
/// before it scans `body`, in the handler's order, through the functions
/// the handler calls. Checks that need a field the request lacks are left
/// to the handler, which rejects the request as invalid; other paths pass.
pub(crate) async fn precheck(
    state: &AppState,
    auth: &AuthContext,
    path: &str,
    body: &Value,
) -> Result<(), Refusal> {
    let client = auth.api_key_id.as_str();
    let model = body.get("model").and_then(Value::as_str);
    match path {
        "/chat/completions" | "/responses" => turn_admission(state, client, model).await?,
        "/completions" => {
            client_llm_access(state, client).map_err(Refusal::access)?;
            turn_admission(state, client, model).await?;
        }
        "/messages" | "/messages/count_tokens" => {
            let model = match model {
                Some(model) => Some(messages_model(state, client, model).await?),
                None => None,
            };
            turn_admission(state, client, model.as_deref()).await?;
        }
        "/systemone" | "/decisions" => {
            client_llm_access(state, client).map_err(Refusal::access)?;
            systemone_model_access(state, client, model, lr_types::is_duplicate_hop())
                .map_err(Refusal::access)?;
            rate_limited(state, client).await?;
        }
        "/embeddings" => {
            client_llm_access(state, client).map_err(Refusal::access)?;
            if let Some(model) = model {
                strategy_model_access(state, client, model).map_err(Refusal::access)?;
            }
            rate_limited(state, client).await?;
        }
        "/audio/speech" => {
            client_llm_access(state, client).map_err(Refusal::access)?;
            if let Some(model) = model {
                strategy_model_access(state, client, model).map_err(Refusal::access)?;
            }
        }
        "/images/generations" => match model {
            Some(model) => image_access(state, auth, model).map_err(Refusal::access)?,
            None => image_client_access(state, auth).map_err(Refusal::access)?,
        },
        "/moderations" => {
            client_llm_access(state, client).map_err(Refusal::access)?;
        }
        "/rerank" | "/ocr" | "/videos" => {
            let operation = match path {
                "/rerank" => lr_config::NativeOperation::Rerank,
                "/ocr" => lr_config::NativeOperation::Ocr,
                _ => lr_config::NativeOperation::Video,
            };
            state
                .router
                .authorize_native_operation(client, operation)
                .map_err(Refusal::route)?;
            turn_admission(state, client, model).await?;
        }
        "/search" => {
            let policy = state
                .router
                .authorize_ranked_search(client)
                .map_err(Refusal::route)?;
            let model = model.map(str::to_owned).or(policy.default_search_model);
            turn_admission(state, client, model.as_deref()).await?;
        }
        _ => {}
    }
    Ok(())
}
