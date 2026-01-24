//! GET /v1/models endpoints
//!
//! Lists available models filtered by API key's model selection.

use axum::{
    extract::{Path, State},
    http::Request,
    Json,
};

use super::helpers::get_client_with_strategy;
use crate::server::middleware::error::{ApiErrorResponse, ApiResult};
use crate::server::state::{AppState, AuthContext};
use crate::server::types::{ModelData, ModelPricing, ModelsResponse};

/// GET /v1/models
/// List available models filtered by the authenticated API key's model selection
#[utoipa::path(
    get,
    path = "/v1/models",
    tag = "models",
    responses(
        (status = 200, description = "List of available models", body = ModelsResponse),
        (status = 401, description = "Unauthorized", body = crate::server::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::server::types::ErrorResponse)
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
    let (_client, strategy) = get_client_with_strategy(&state, &auth_context.api_key_id)?;

    // If auto-routing is enabled, return ONLY the auto router model
    // This simplifies the client experience - they see one model to use
    if let Some(auto_config) = &strategy.auto_config {
        if auto_config.enabled {
            return Ok(Json(ModelsResponse {
                object: "list".to_string(),
                data: vec![ModelData {
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
                }],
            }));
        }
    }

    // Auto-routing disabled: return allowed models filtered by strategy
    let all_models = state
        .provider_registry
        .list_all_models()
        .await
        .map_err(|e| ApiErrorResponse::internal_error(format!("Failed to list models: {}", e)))?;

    // Filter models by strategy's allowed models
    let filtered_models: Vec<_> = all_models
        .into_iter()
        .filter(|model| {
            // Check if model is allowed by strategy
            strategy.is_model_allowed(&model.provider, &model.id)
        })
        .collect();

    // Convert to API response format with pricing information
    let mut model_data_vec = Vec::new();

    for model_info in filtered_models {
        let mut model_data: ModelData = (&model_info).into();

        // Fetch pricing information
        if let Some(provider) = state.provider_registry.get_provider(&model_info.provider) {
            if let Ok(pricing_info) = provider.get_pricing(&model_info.id).await {
                model_data.pricing = Some(ModelPricing {
                    input_cost_per_1k: pricing_info.input_cost_per_1k,
                    output_cost_per_1k: pricing_info.output_cost_per_1k,
                    currency: pricing_info.currency,
                });
            }
        }

        model_data_vec.push(model_data);
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
        (status = 401, description = "Unauthorized", body = crate::server::types::ErrorResponse),
        (status = 403, description = "Forbidden - no access to this model", body = crate::server::types::ErrorResponse),
        (status = 404, description = "Model not found", body = crate::server::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::server::types::ErrorResponse)
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
    let (_client, strategy) = get_client_with_strategy(&state, &auth_context.api_key_id)?;

    // Special handling for auto router virtual model
    if let Some(auto_config) = &strategy.auto_config {
        if auto_config.enabled && model_id == auto_config.model_name {
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

    // Check if requesting auto router model but it's not enabled
    if model_id == "localrouter/auto"
        || model_id.starts_with("localrouter/")
        || (strategy
            .auto_config
            .as_ref()
            .map(|c| model_id == c.model_name)
            .unwrap_or(false))
    {
        return Err(ApiErrorResponse::not_found(
            "Auto router model is not enabled for this client".to_string(),
        ));
    }

    // Get all models from provider registry
    let all_models = state
        .provider_registry
        .list_all_models()
        .await
        .map_err(|e| ApiErrorResponse::internal_error(format!("Failed to list models: {}", e)))?;

    // Find the requested model (case-insensitive comparison for consistency with chat endpoint)
    let model_info = all_models
        .iter()
        .find(|m| m.id.eq_ignore_ascii_case(&model_id))
        .ok_or_else(|| ApiErrorResponse::not_found(format!("Model '{}' not found", model_id)))?;

    // Check if strategy allows access to this model
    if !strategy.is_model_allowed(&model_info.provider, &model_info.id) {
        return Err(ApiErrorResponse::forbidden(format!(
            "API key does not have access to model '{}'",
            model_id
        )));
    }

    // Convert to API response format with enhanced details
    let mut model_data: ModelData = model_info.into();

    // Fetch pricing information
    if let Some(provider) = state.provider_registry.get_provider(&model_info.provider) {
        if let Ok(pricing_info) = provider.get_pricing(&model_info.id).await {
            model_data.pricing = Some(ModelPricing {
                input_cost_per_1k: pricing_info.input_cost_per_1k,
                output_cost_per_1k: pricing_info.output_cost_per_1k,
                currency: pricing_info.currency,
            });
        }
    }

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
        (status = 401, description = "Unauthorized", body = crate::server::types::ErrorResponse),
        (status = 403, description = "Forbidden - no access to this model", body = crate::server::types::ErrorResponse),
        (status = 404, description = "Model or pricing not found", body = crate::server::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::server::types::ErrorResponse)
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
    let (_client, strategy) = get_client_with_strategy(&state, &auth_context.api_key_id)?;

    // Check if strategy allows access to this model
    if !strategy.is_model_allowed(&provider, &model) {
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
