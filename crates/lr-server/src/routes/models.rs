//! GET /v1/models endpoints
//!
//! Lists available models filtered by API key's model selection.

use axum::{
    extract::{Path, State},
    http::Request,
    Json,
};

use super::helpers::{check_llm_access_with_state, get_client_with_strategy};
use crate::middleware::error::{ApiErrorResponse, ApiResult};
use crate::state::{AppState, AuthContext};
use crate::types::{CatalogInfo, ModelData, ModelPricing, ModelsResponse, PricingSource};

/// Apply embedded-catalog pricing and provenance (`catalog_info`) to a model entry.
/// Prefers an exact provider+id match, falling back to a name-only match.
fn apply_catalog_pricing(model_data: &mut ModelData, provider_type: &str, model_id: &str) {
    let (catalog_model, matched_via) = match lr_catalog::find_model(provider_type, model_id) {
        Some(cm) => (Some(cm), "provider_and_id"),
        None => (lr_catalog::find_model_by_name(model_id), "name"),
    };

    if let Some(cm) = catalog_model {
        model_data.pricing = Some(ModelPricing {
            input_cost_per_1k: cm.pricing.prompt_cost_per_1k(),
            output_cost_per_1k: cm.pricing.completion_cost_per_1k(),
            currency: cm.pricing.currency.to_string(),
        });
        model_data.catalog_info = Some(CatalogInfo {
            pricing_source: PricingSource::Catalog,
            catalog_date: Some(lr_catalog::metadata().fetch_date().to_rfc3339()),
            matched_via: Some(matched_via.to_string()),
        });
    }
}

/// GET /v1/models
/// List available models filtered by the authenticated API key's model selection
#[utoipa::path(
    get,
    path = "/v1/models",
    tag = "models",
    responses(
        (status = 200, description = "List of available models", body = ModelsResponse),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::types::ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn list_models<B>(
    State(state): State<AppState>,
    req: Request<B>,
) -> ApiResult<Json<ModelsResponse>> {
    // Get auth context from request extensions (set by auth middleware)
    let auth_context = req
        .extensions()
        .get::<AuthContext>()
        .ok_or_else(|| ApiErrorResponse::unauthorized("Authentication required"))?;

    // Get enabled client and strategy
    let (client, strategy) = get_client_with_strategy(&state, &auth_context.api_key_id)?;
    check_llm_access_with_state(&state, &client)?;

    // If auto_config.permission is Off, return empty model list (all access disabled)
    if let Some(auto_config) = &strategy.auto_config {
        if !auto_config.permission.is_enabled() {
            return Ok(Json(ModelsResponse {
                object: "list".to_string(),
                data: vec![],
            }));
        }
    }

    // If auto-routing is configured with prioritized models and permission is enabled,
    // prepend the virtual localrouter/auto model to the list
    let mut auto_model: Option<ModelData> = None;
    if let Some(auto_config) = &strategy.auto_config {
        if auto_config.permission.is_enabled() && !auto_config.prioritized_models.is_empty() {
            auto_model = Some(ModelData {
                id: auto_config.model_name.clone(),
                object: "model".to_string(),
                owned_by: "localrouter".to_string(),
                created: Some(0),
                provider: "localrouter".to_string(),
                parameter_count: None,
                context_window: 0, // Virtual model, delegates to actual models
                supports_streaming: true,
                capabilities: vec!["chat".to_string(), "completion".to_string()],
                pricing: None,
                detailed_capabilities: None,
                features: None,
                supported_parameters: None,
                performance: None,
                catalog_info: None,
            });
        }
    }

    // Always return allowed models filtered by strategy (instant, no network I/O)
    let all_models = state.provider_registry.list_all_models_instant();

    // Filter models by strategy's allowed models
    let filtered_models: Vec<_> = all_models
        .into_iter()
        .filter(|model| {
            // Check if model is allowed by strategy
            strategy.is_model_allowed(&model.provider, &model.id)
        })
        .collect();

    // Convert to API response format with catalog pricing (no network calls)
    let mut model_data_vec = Vec::new();

    for model_info in filtered_models {
        let mut model_data: ModelData = (&model_info).into();

        // Use embedded catalog for instant pricing lookup
        let provider_type = state
            .provider_registry
            .get_provider_type_for_instance(&model_info.provider)
            .unwrap_or_else(|| model_info.provider.clone());

        apply_catalog_pricing(&mut model_data, &provider_type, &model_info.id);

        model_data_vec.push(model_data);
    }

    // Prepend the virtual auto model if applicable
    if let Some(auto_model_data) = auto_model {
        model_data_vec.insert(0, auto_model_data);
    }

    Ok(Json(ModelsResponse {
        object: "list".to_string(),
        data: model_data_vec,
    }))
}

/// GET /v1/models/{id}
/// Get detailed information about a specific model
#[utoipa::path(
    get,
    path = "/v1/models/{id}",
    tag = "models",
    params(
        ("id" = String, Path, description = "Model identifier")
    ),
    responses(
        (status = 200, description = "Model details", body = ModelData),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 403, description = "Forbidden - no access to this model", body = crate::types::ErrorResponse),
        (status = 404, description = "Model not found", body = crate::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::types::ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_model<B>(
    State(state): State<AppState>,
    Path(model_id): Path<String>,
    req: Request<B>,
) -> ApiResult<Json<ModelData>> {
    // Get auth context from request extensions
    let auth_context = req
        .extensions()
        .get::<AuthContext>()
        .ok_or_else(|| ApiErrorResponse::unauthorized("Authentication required"))?;

    // Get enabled client and strategy
    let (client, strategy) = get_client_with_strategy(&state, &auth_context.api_key_id)?;
    check_llm_access_with_state(&state, &client)?;

    // Special handling for auto router virtual model
    if let Some(auto_config) = &strategy.auto_config {
        if !auto_config.prioritized_models.is_empty() && model_id == auto_config.model_name {
            return Ok(Json(ModelData {
                id: auto_config.model_name.clone(),
                object: "model".to_string(),
                owned_by: "localrouter".to_string(),
                created: Some(0),
                provider: "localrouter".to_string(),
                parameter_count: None,
                context_window: 0, // Virtual model, delegates to actual models
                supports_streaming: true,
                capabilities: vec!["chat".to_string(), "completion".to_string()],
                pricing: None,
                detailed_capabilities: None,
                features: None,
                supported_parameters: None,
                performance: None,
                catalog_info: None,
            }));
        }
    }

    // Check if requesting auto router model but it's not configured
    if model_id == "localrouter/auto"
        || model_id.starts_with("localrouter/")
        || (strategy
            .auto_config
            .as_ref()
            .map(|c| model_id == c.model_name)
            .unwrap_or(false))
    {
        return Err(ApiErrorResponse::not_found(
            "Auto router model is not configured for this client".to_string(),
        ));
    }

    // Get all models from provider registry (instant, no network I/O)
    let all_models = state.provider_registry.list_all_models_instant();

    // Find the requested model (case-insensitive comparison for consistency with chat endpoint)
    let model_info = all_models
        .iter()
        .find(|m| m.id.eq_ignore_ascii_case(&model_id))
        .ok_or_else(|| {
            super::monitor_helpers::emit_access_denied_for_client(
                &state,
                &auth_context.api_key_id,
                None,
                "model_not_found",
                "/v1/models/{id}",
                &format!("Model '{}' not found", model_id),
                404,
            );
            ApiErrorResponse::not_found(format!("Model '{}' not found", model_id))
        })?;

    // Check if strategy allows access to this model
    if !strategy.is_model_allowed(&model_info.provider, &model_info.id) {
        super::monitor_helpers::emit_access_denied_for_client(
            &state,
            &auth_context.api_key_id,
            None,
            "model_not_allowed",
            "/v1/models/{id}",
            &format!("API key does not have access to model '{}'", model_id),
            403,
        );
        return Err(ApiErrorResponse::forbidden(format!(
            "API key does not have access to model '{}'",
            model_id
        )));
    }

    // Convert to API response format with catalog pricing (no network calls)
    let mut model_data: ModelData = model_info.into();

    // Use embedded catalog for instant pricing lookup
    let provider_type = state
        .provider_registry
        .get_provider_type_for_instance(&model_info.provider)
        .unwrap_or_else(|| model_info.provider.clone());

    apply_catalog_pricing(&mut model_data, &provider_type, &model_info.id);

    Ok(Json(model_data))
}

/// GET /v1/models/{provider}/{model}/pricing
/// Get pricing information for a specific model from a provider
#[utoipa::path(
    get,
    path = "/v1/models/{provider}/{model}/pricing",
    tag = "models",
    params(
        ("provider" = String, Path, description = "Provider name"),
        ("model" = String, Path, description = "Model name")
    ),
    responses(
        (status = 200, description = "Model pricing information", body = ModelPricing),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 403, description = "Forbidden - no access to this model", body = crate::types::ErrorResponse),
        (status = 404, description = "Model or pricing not found", body = crate::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::types::ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_model_pricing<B>(
    State(state): State<AppState>,
    Path((provider, model)): Path<(String, String)>,
    req: Request<B>,
) -> ApiResult<Json<ModelPricing>> {
    // Get auth context from request extensions
    let auth_context = req
        .extensions()
        .get::<AuthContext>()
        .ok_or_else(|| ApiErrorResponse::unauthorized("Authentication required"))?;

    // Get enabled client and strategy
    let (client, strategy) = get_client_with_strategy(&state, &auth_context.api_key_id)?;
    check_llm_access_with_state(&state, &client)?;

    // Check if strategy allows access to this model
    if !strategy.is_model_allowed(&provider, &model) {
        super::monitor_helpers::emit_access_denied_for_client(
            &state,
            &auth_context.api_key_id,
            None,
            "model_not_allowed",
            "/v1/models/{provider}/{model}/pricing",
            &format!(
                "API key does not have access to model '{}/{}'",
                provider, model
            ),
            403,
        );
        return Err(ApiErrorResponse::forbidden(format!(
            "API key does not have access to model '{}/{}'",
            provider, model
        )));
    }

    // Get provider instance
    let provider_instance = state
        .provider_registry
        .get_provider(&provider)
        .ok_or_else(|| ApiErrorResponse::not_found(format!("Provider '{}' not found", provider)))?;

    // Get pricing information
    let pricing_info = provider_instance.get_pricing(&model).await.map_err(|e| {
        ApiErrorResponse::internal_error(format!(
            "Failed to get pricing for model '{}': {}",
            model, e
        ))
    })?;

    Ok(Json(ModelPricing {
        input_cost_per_1k: pricing_info.input_cost_per_1k,
        output_cost_per_1k: pricing_info.output_cost_per_1k,
        currency: pricing_info.currency,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank_model(id: &str) -> ModelData {
        ModelData {
            id: id.to_string(),
            object: "model".to_string(),
            owned_by: "test".to_string(),
            created: None,
            provider: "test".to_string(),
            parameter_count: None,
            context_window: 0,
            supports_streaming: true,
            capabilities: vec![],
            pricing: None,
            detailed_capabilities: None,
            features: None,
            supported_parameters: None,
            performance: None,
            catalog_info: None,
        }
    }

    #[test]
    fn apply_catalog_pricing_populates_pricing_and_provenance() {
        // A model that exists in the embedded models.dev catalog.
        let model_id = "gpt-4o";
        assert!(
            lr_catalog::find_model_by_name(model_id).is_some(),
            "expected {model_id} in embedded catalog"
        );
        let mut data = blank_model(model_id);

        apply_catalog_pricing(&mut data, "no-such-provider", model_id);

        assert!(data.pricing.is_some());
        let info = data.catalog_info.expect("catalog_info populated");
        assert!(matches!(info.pricing_source, PricingSource::Catalog));
        assert!(info.matched_via.is_some());
        assert!(info.catalog_date.is_some());
    }

    #[test]
    fn apply_catalog_pricing_leaves_unknown_model_untouched() {
        let mut data = blank_model("definitely-not-a-real-model-xyz");

        apply_catalog_pricing(
            &mut data,
            "no-such-provider",
            "definitely-not-a-real-model-xyz",
        );

        assert!(data.pricing.is_none());
        assert!(data.catalog_info.is_none());
    }
}
