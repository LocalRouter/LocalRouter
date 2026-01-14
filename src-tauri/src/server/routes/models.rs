//! GET /v1/models endpoint
//!
//! Lists all available models from all configured providers.

use axum::{extract::State, Json};

use crate::server::middleware::error::{ApiErrorResponse, ApiResult};
use crate::server::state::AppState;
use crate::server::types::{ModelData, ModelPricing, ModelsResponse};

/// GET /v1/models
/// List all available models from all enabled providers
pub async fn list_models(
    State(state): State<AppState>,
) -> ApiResult<Json<ModelsResponse>> {
    // Get all models from provider registry
    let models = state
        .provider_registry
        .list_all_models()
        .await
        .map_err(|e| ApiErrorResponse::internal_error(format!("Failed to list models: {}", e)))?;

    // Convert to API response format
    let mut model_data_vec = Vec::new();

    for model_info in models {
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
