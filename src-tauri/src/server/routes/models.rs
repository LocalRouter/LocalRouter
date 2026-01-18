//! GET /v1/models endpoints
//!
//! Lists available models filtered by API key's model selection.

use axum::{extract::{Path, State}, http::Request, Json};

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

    // Get all models from provider registry
    let all_models = state
        .provider_registry
        .list_all_models()
        .await
        .map_err(|e| ApiErrorResponse::internal_error(format!("Failed to list models: {}", e)))?;

    // Filter models based on API key's routing configuration
    let filtered_models = if let Some(routing_config) = &auth_context.routing_config {
        // Use new routing config system
        use crate::config::ActiveRoutingStrategy;

        match routing_config.active_strategy {
            ActiveRoutingStrategy::AvailableModels => {
                // Filter to only available models
                all_models
                    .into_iter()
                    .filter(|m| routing_config.is_model_allowed(&m.provider, &m.id))
                    .collect()
            }
            ActiveRoutingStrategy::ForceModel => {
                // Return only the forced model
                if let Some((forced_provider, forced_model)) = &routing_config.forced_model {
                    all_models
                        .into_iter()
                        .filter(|m| {
                            m.provider.eq_ignore_ascii_case(forced_provider)
                                && m.id.eq_ignore_ascii_case(forced_model)
                        })
                        .collect()
                } else {
                    // No forced model configured - return empty
                    vec![]
                }
            }
            ActiveRoutingStrategy::PrioritizedList => {
                // Return models in the prioritized list order
                let mut prioritized = Vec::new();
                for (provider, model) in &routing_config.prioritized_models {
                    if let Some(model_info) = all_models.iter().find(|m| {
                        m.provider.eq_ignore_ascii_case(provider)
                            && m.id.eq_ignore_ascii_case(model)
                    }) {
                        prioritized.push(model_info.clone());
                    }
                }
                prioritized
            }
        }
    } else {
        // Fallback to old model_selection for backward compatibility
        match &auth_context.model_selection {
            Some(selection) => {
                // Use the is_model_allowed method to filter models
                all_models
                    .into_iter()
                    .filter(|m| selection.is_model_allowed(&m.provider, &m.id))
                    .collect()
            }
            None => {
                // No model selection configured - allow all models
                all_models
            }
        }
    };

    // Convert to API response format
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

    // Get all models from provider registry
    let all_models = state
        .provider_registry
        .list_all_models()
        .await
        .map_err(|e| ApiErrorResponse::internal_error(format!("Failed to list models: {}", e)))?;

    // Find the requested model
    let model_info = all_models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| ApiErrorResponse::not_found(format!("Model '{}' not found", model_id)))?;

    // Check if API key has access to this model
    if let Some(routing_config) = &auth_context.routing_config {
        if !routing_config.is_model_allowed(&model_info.provider, &model_info.id) {
            return Err(ApiErrorResponse::forbidden(format!(
                "API key does not have access to model '{}'",
                model_id
            )));
        }
    } else if let Some(selection) = &auth_context.model_selection {
        if !selection.is_model_allowed(&model_info.provider, &model_info.id) {
            return Err(ApiErrorResponse::forbidden(format!(
                "API key does not have access to model '{}'",
                model_id
            )));
        }
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

    // Check if API key has access to this model
    if let Some(routing_config) = &auth_context.routing_config {
        if !routing_config.is_model_allowed(&provider, &model) {
            return Err(ApiErrorResponse::forbidden(format!(
                "API key does not have access to model '{}/{}'",
                provider, model
            )));
        }
    } else if let Some(selection) = &auth_context.model_selection {
        if !selection.is_model_allowed(&provider, &model) {
            return Err(ApiErrorResponse::forbidden(format!(
                "API key does not have access to model '{}/{}'",
                provider, model
            )));
        }
    }

    // Get provider instance
    let provider_instance = state
        .provider_registry
        .get_provider(&provider)
        .ok_or_else(|| {
            ApiErrorResponse::not_found(format!("Provider '{}' not found", provider))
        })?;

    // Get pricing information
    let pricing_info = provider_instance
        .get_pricing(&model)
        .await
        .map_err(|e| {
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
