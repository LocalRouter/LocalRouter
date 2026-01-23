//! POST /v1/images/generations endpoint
//!
//! Generate images using DALL-E or other image generation models.

use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Extension, Json,
};
use std::time::Instant;

use crate::server::middleware::error::{ApiErrorResponse, ApiResult};
use crate::server::state::{AppState, AuthContext};
use crate::server::types::{ImageData, ImageGenerationRequest, ImageGenerationResponse};

/// POST /v1/images/generations
/// Generate images from a text prompt
#[utoipa::path(
    post,
    path = "/v1/images/generations",
    tag = "images",
    request_body = ImageGenerationRequest,
    responses(
        (status = 200, description = "Successful response", body = ImageGenerationResponse),
        (status = 400, description = "Bad request", body = crate::server::types::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::server::types::ErrorResponse),
        (status = 502, description = "Provider error", body = crate::server::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::server::types::ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn image_generations(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(request): Json<ImageGenerationRequest>,
) -> ApiResult<Response> {
    // Emit LLM request event to trigger tray icon indicator
    state.emit_event("llm-request", "image");

    // Validate request
    validate_request(&request)?;

    let started_at = Instant::now();

    // Parse model to get provider (format: provider/model or just model)
    let (provider_name, model_name) = if let Some((prov, model)) = request.model.split_once('/') {
        (prov.to_string(), model.to_string())
    } else {
        // Default to openai for DALL-E models
        if request.model.starts_with("dall-e") {
            ("openai".to_string(), request.model.clone())
        } else {
            return Err(ApiErrorResponse::bad_request(
                "Model must be in provider/model format or a recognized model name (dall-e-2, dall-e-3)",
            )
            .with_param("model"));
        }
    };

    // Get the provider
    let provider = state
        .provider_registry
        .get_provider(&provider_name)
        .ok_or_else(|| {
            ApiErrorResponse::bad_request(format!("Provider '{}' not found", provider_name))
                .with_param("model")
        })?;

    // Convert server request to provider request
    let provider_request = crate::providers::ImageGenerationRequest {
        model: model_name,
        prompt: request.prompt.clone(),
        n: request.n,
        size: request.size.clone(),
        quality: request.quality.clone(),
        style: request.style.clone(),
        response_format: request.response_format.clone(),
        user: request.user.clone(),
    };

    // Call the provider's generate_image method
    let provider_response = provider.generate_image(provider_request).await.map_err(|e| {
        tracing::error!("Image generation failed: {}", e);
        ApiErrorResponse::bad_gateway(format!("Provider error: {}", e))
    })?;

    let latency_ms = Instant::now().duration_since(started_at).as_millis() as u64;

    // Convert provider response to API response
    let api_response = ImageGenerationResponse {
        created: provider_response.created,
        data: provider_response
            .data
            .into_iter()
            .map(|img| ImageData {
                url: img.url,
                b64_json: img.b64_json,
                revised_prompt: img.revised_prompt,
            })
            .collect(),
    };

    // Log success
    tracing::info!(
        "Image generation completed: client={}, model={}, latency={}ms",
        auth.api_key_id,
        request.model,
        latency_ms
    );

    Ok(Json(api_response).into_response())
}

/// Validate image generation request
fn validate_request(request: &ImageGenerationRequest) -> ApiResult<()> {
    if request.model.is_empty() {
        return Err(ApiErrorResponse::bad_request("model is required").with_param("model"));
    }

    if request.prompt.is_empty() {
        return Err(ApiErrorResponse::bad_request("prompt is required").with_param("prompt"));
    }

    if request.prompt.len() > 4000 {
        return Err(
            ApiErrorResponse::bad_request("prompt must be 4000 characters or less")
                .with_param("prompt"),
        );
    }

    // Validate n (number of images)
    if let Some(n) = request.n {
        if n == 0 || n > 10 {
            return Err(
                ApiErrorResponse::bad_request("n must be between 1 and 10").with_param("n")
            );
        }
    }

    // Validate size if provided
    if let Some(size) = &request.size {
        let valid_sizes = [
            "256x256",
            "512x512",
            "1024x1024",
            "1024x1792",
            "1792x1024",
        ];
        if !valid_sizes.contains(&size.as_str()) {
            return Err(ApiErrorResponse::bad_request(format!(
                "Invalid size '{}'. Valid sizes are: {}",
                size,
                valid_sizes.join(", ")
            ))
            .with_param("size"));
        }
    }

    // Validate quality if provided
    if let Some(quality) = &request.quality {
        if quality != "standard" && quality != "hd" {
            return Err(
                ApiErrorResponse::bad_request("quality must be 'standard' or 'hd'")
                    .with_param("quality"),
            );
        }
    }

    // Validate style if provided
    if let Some(style) = &request.style {
        if style != "vivid" && style != "natural" {
            return Err(
                ApiErrorResponse::bad_request("style must be 'vivid' or 'natural'")
                    .with_param("style"),
            );
        }
    }

    // Validate response_format if provided
    if let Some(format) = &request.response_format {
        if format != "url" && format != "b64_json" {
            return Err(
                ApiErrorResponse::bad_request("response_format must be 'url' or 'b64_json'")
                    .with_param("response_format"),
            );
        }
    }

    Ok(())
}
