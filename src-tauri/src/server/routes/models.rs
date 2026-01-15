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

    // Filter models based on API key's model selection
    let filtered_models = match &auth_context.model_selection {
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
