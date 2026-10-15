use crate::{
    middleware::error::{ApiErrorResponse, ApiResult},
    routes::helpers,
    state::{AppState, AuthContext},
};
use lr_providers::{
    batches::{NativeBatchKind, NativeBatchTransport},
    Capability, ModelProvider,
};
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) struct NativeContext {
    pub provider_instance: String,
    pub model: String,
    pub permission_model: String,
    pub provider: Arc<dyn ModelProvider>,
    pub transport: NativeBatchTransport,
}
#[derive(Clone)]
pub(crate) struct PolicySeal {
    pub fingerprint: String,
    pub value: Value,
}

pub(crate) fn authorize(state: &AppState, auth: &AuthContext, qualified: &str) -> ApiResult<()> {
    super::super::routes::files::owner(state, auth)?;
    if lr_types::is_duplicate_hop() {
        return Err(ApiErrorResponse::forbidden(
            "Native batches do not allow duplicate-hop policy bypass",
        ));
    }
    let (client, strategy) = helpers::get_client_with_strategy(state, &auth.api_key_id)?;
    helpers::check_llm_access_with_state(state, &client)?;
    helpers::check_strategy_permission(&strategy)?;
    helpers::validate_strategy_model_access(state, &strategy, qualified)?;
    if client.is_mcp_via_llm() {
        return Err(ApiErrorResponse::bad_request(
            "Native batches do not execute automatic MCP/tool loops",
        ));
    }
    if strategy.free_tier_only {
        return Err(ApiErrorResponse::bad_request(
            "Native batches do not yet support this strategy's free-tier-only routing policy",
        ));
    }
    Ok(())
}
pub(crate) fn context(
    state: &AppState,
    auth: &AuthContext,
    qualified: &str,
) -> ApiResult<NativeContext> {
    let names = state.provider_registry.get_provider_names();
    let mut matching = names
        .iter()
        .filter_map(|name| {
            qualified
                .strip_prefix(&format!("{name}/"))
                .map(|model| (name, model))
        })
        .collect::<Vec<_>>();
    matching.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    let (instance, model) = matching.first().copied().ok_or_else(|| {
        ApiErrorResponse::bad_request(
            "Every native batch row requires an explicit enabled provider-instance/model",
        )
    })?;
    if model.is_empty()
        || model.eq_ignore_ascii_case("auto")
        || model.starts_with("openrouter/auto")
        || model == "openrouter/free"
        || instance.contains('/')
    {
        return Err(ApiErrorResponse::bad_request(
            "Native batches require one unambiguous fixed model, not a router alias",
        ));
    }
    crate::routes::files::owner(state, auth)?;
    let provider = state
        .provider_registry
        .get_provider(instance)
        .ok_or_else(|| {
            ApiErrorResponse::bad_request("Native batch provider is unavailable or disabled")
        })?;
    let transport = provider
        .native_batch_transport()
        .map_err(ApiErrorResponse::from)?
        .ok_or_else(|| {
            ApiErrorResponse::bad_request(
                "This provider/account has no verified native batch transport",
            )
        })?;
    let permission_model = if transport.kind() == NativeBatchKind::Openrouter {
        if model.contains(':') {
            return Err(ApiErrorResponse::bad_request("OpenRouter batch requests use a base model slug; :batch is the catalog/permission entry"));
        }
        format!("{model}:batch")
    } else {
        model.into()
    };
    authorize(state, auth, &format!("{instance}/{permission_model}"))?;
    Ok(NativeContext {
        provider_instance: instance.clone(),
        model: model.into(),
        permission_model,
        provider,
        transport,
    })
}
pub(crate) async fn verify_model(context: &NativeContext, endpoint: &str) -> ApiResult<()> {
    let models = context.provider.list_models().await.map_err(|_| {
        ApiErrorResponse::bad_gateway(
            "Native provider model discovery failed; check the configured account",
        )
    })?;
    let matching = models
        .iter()
        .filter(|m| m.id == context.permission_model)
        .collect::<Vec<_>>();
    let model = matching
        .first()
        .filter(|_| matching.len() == 1)
        .ok_or_else(|| {
            ApiErrorResponse::bad_request(
                "Native batch model is not uniquely advertised by the configured provider",
            )
        })?;
    let capability = if endpoint == "/v1/embeddings" {
        Capability::Embedding
    } else {
        Capability::Chat
    };
    if !model.capabilities.contains(&capability) {
        return Err(ApiErrorResponse::bad_request(
            "The selected model does not advertise this endpoint capability",
        ));
    }
    Ok(())
}
pub(crate) fn validate_kind(kind: NativeBatchKind, endpoint: &str, body: &Value) -> ApiResult<()> {
    if kind == NativeBatchKind::Openrouter
        && endpoint == "/v1/chat/completions"
        && (body.get("max_completion_tokens").is_some() || body.get("reasoning_effort").is_some())
    {
        return Err(ApiErrorResponse::bad_request("OpenRouter native chat batches currently require max_tokens and do not expose unverified reasoning-effort aliases"));
    }
    Ok(())
}
pub(crate) fn seal(
    state: &AppState,
    auth: &AuthContext,
    context: &NativeContext,
) -> ApiResult<PolicySeal> {
    use lr_mcp::gateway::firewall::InterceptCategory;
    let cfg = state.config_manager.get();
    let (mut client, strategy) = helpers::get_client_with_strategy(state, &auth.api_key_id)?;
    client.last_used = None;
    let intercept = |category| {
        state
            .mcp_gateway
            .firewall_manager
            .should_intercept(&client.id, category)
    };
    let value = json!({"client":client,"strategy":strategy,"guardrails":cfg.guardrails,"secrets":cfg.secret_scanning,
        "provider_config":state.provider_registry.get_provider_config(&context.provider_instance),
        "resolved_permission":strategy.model_permissions.resolve_model(&context.provider_instance,&context.permission_model),
        "model_approval":state.model_approval_tracker.has_valid_approval(&client.id,&context.provider_instance,&context.permission_model),
        "guardrail_bypass":state.guardrail_approval_tracker.has_valid_bypass(&client.id),
        "secret_bypass":state.secret_scan_approval_tracker.has_valid_bypass(&client.id),
        "intercepts":{"llm":intercept(InterceptCategory::Llm),"guardrails":intercept(InterceptCategory::Guardrails),"secrets":intercept(InterceptCategory::SecretScan)}});
    Ok(PolicySeal {
        fingerprint: lr_router::explanation::fingerprint(value.clone()),
        value,
    })
}
/// A user's permanent Allow or new time grant can relax policy mid-validation.
/// Only the pinned model and explicit grant booleans can change here; rules,
/// modes, credentials, limits and every other setting must remain identical.
pub(crate) fn approval_relaxation(before: &PolicySeal, after: &PolicySeal) -> bool {
    let old = &before.value;
    let new = &after.value;
    let permission = |a: &Value, b: &Value| a == b || (a == "ask" && b == "allow");
    if !permission(&old["resolved_permission"], &new["resolved_permission"])
        || !permission(
            &old["strategy"]["auto_config"]["permission"],
            &new["strategy"]["auto_config"]["permission"],
        )
    {
        return false;
    }
    for key in ["model_approval", "guardrail_bypass", "secret_bypass"] {
        if old[key] == true && new[key] != true {
            return false;
        }
    }
    let normalize = |mut value: Value| {
        for key in [
            "model_approval",
            "guardrail_bypass",
            "secret_bypass",
            "resolved_permission",
        ] {
            value.as_object_mut().unwrap().remove(key);
        }
        for key in ["client", "strategy"] {
            if let Some(object) = value[key].as_object_mut() {
                object.remove("model_permissions");
            }
        }
        if let Some(auto) = value["strategy"]["auto_config"].as_object_mut() {
            auto.remove("permission");
        }
        value
    };
    normalize(old.clone()) == normalize(new.clone())
}
