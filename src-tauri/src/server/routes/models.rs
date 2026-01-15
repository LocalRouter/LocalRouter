//! GET /v1/models endpoint
//!
//! Lists available models filtered by API key's model selection.

use axum::{extract::State, http::Request, Json};

use crate::server::middleware::error::{ApiErrorResponse, ApiResult};
use crate::server::state::{AppState, AuthContext};
use crate::server::types::{ModelData, ModelPricing, ModelsResponse};

/// GET /v1/models
/// List available models filtered by the authenticated API key's model selection
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
