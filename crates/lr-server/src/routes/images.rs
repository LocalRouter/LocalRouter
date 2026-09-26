//! Image endpoints
//!
//! POST /v1/images/generations — generate images from a prompt
//! POST /v1/images/edits       — edit images (multipart: images, mask, prompt)

use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Extension, Json,
};
use std::time::Instant;

use super::helpers::{
    check_llm_access_with_state, check_strategy_permission, get_client_with_strategy,
    get_enabled_client, validate_strategy_model_access,
};
use crate::middleware::error::{ApiErrorResponse, ApiResult};
use crate::state::{AppState, AuthContext};
use crate::types::{ImageData, ImageGenerationRequest, ImageGenerationResponse};

/// POST /v1/images/generations
/// Generate images from a text prompt
#[utoipa::path(
    post,
    path = "/v1/images/generations",
    tag = "images",
    request_body = ImageGenerationRequest,
    responses(
        (status = 200, description = "Successful response", body = ImageGenerationResponse),
        (status = 400, description = "Bad request", body = crate::types::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 502, description = "Provider error", body = crate::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::types::ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn image_generations(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(mut request): Json<ImageGenerationRequest>,
) -> ApiResult<Response> {
    // Emit LLM request event to trigger tray icon indicator
    state.emit_event("llm-request", "image");

    // Generate session ID for correlated monitor events
    let session_id = uuid::Uuid::new_v4().to_string();

    // Emit monitor event for traffic inspection
    let request_json = serde_json::to_value(&request).unwrap_or_default();
    let mut llm_guard = super::monitor_helpers::emit_llm_call(
        &state,
        None,
        Some(&session_id),
        "/v1/images/generations",
        &request.model,
        false,
        &request_json,
    );

    // Record client activity for connection graph
    state.record_client_activity(&auth.api_key_id);

    // Client, access mode and strategy model permissions
    check_image_access(&state, &auth, &request.model).map_err(|e| llm_guard.capture_err(e))?;

    // Validate request
    if let Err(e) = validate_request(&request) {
        super::monitor_helpers::emit_validation_error(
            &state,
            None,
            Some(&session_id),
            "/v1/images/generations",
            e.error.error.param.as_deref(),
            &e.error.error.message,
            400,
        );
        return Err(llm_guard.capture_err(e));
    }

    let started_at = Instant::now();

    // Normalize auto model name for rejection
    if request.model != "localrouter/auto" {
        if let Ok((_, ref strategy)) = get_client_with_strategy(&state, &auth.api_key_id) {
            if let Some(ref ac) = strategy.auto_config {
                if request.model == ac.model_name {
                    request.model = "localrouter/auto".to_string();
                }
            }
        }
    }

    // Parse model to get provider (format: provider/model or just model)
    // Auto-routing is not supported for image generation
    if request.model == "localrouter/auto" {
        super::monitor_helpers::emit_validation_error(
            &state,
            None,
            Some(&session_id),
            "/v1/images/generations",
            Some("model"),
            "Auto-routing is not supported for image generation. Use provider/model format (e.g. openai/dall-e-3)",
            400,
        );
        return Err(llm_guard.capture_err(ApiErrorResponse::bad_request(
            "Auto-routing is not supported for image generation. Use provider/model format (e.g. openai/dall-e-3)",
        )
        .with_param("model")));
    }

    let (provider_name, model_name) = if let Some((prov, model)) = request.model.split_once('/') {
        (prov.to_string(), model.to_string())
    } else {
        // Default to openai for DALL-E models
        if request.model.starts_with("dall-e") {
            ("openai".to_string(), request.model.clone())
        } else {
            super::monitor_helpers::emit_validation_error(
                &state,
                None,
                Some(&session_id),
                "/v1/images/generations",
                Some("model"),
                "Model must be in provider/model format or a recognized model name",
                400,
            );
            return Err(llm_guard.capture_err(ApiErrorResponse::bad_request(
                "Model must be in provider/model format or a recognized model name (dall-e-2, dall-e-3)",
            )
            .with_param("model")));
        }
    };

    // Get the provider
    let provider = match state.provider_registry.get_provider(&provider_name) {
        Some(p) => p,
        None => {
            super::monitor_helpers::emit_validation_error(
                &state,
                None,
                Some(&session_id),
                "/v1/images/generations",
                Some("model"),
                &format!("Provider '{}' not found", provider_name),
                400,
            );
            return Err(llm_guard.capture_err(
                ApiErrorResponse::bad_request(format!("Provider '{}' not found", provider_name))
                    .with_param("model"),
            ));
        }
    };

    // Convert server request to provider request
    let provider_request = lr_providers::ImageGenerationRequest {
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
    let provider_response = match provider.generate_image(provider_request).await {
        Ok(resp) => resp,
        Err(e) => {
            let err_text = e.to_string();
            let api_err: ApiErrorResponse = e.into();
            let status_code = api_err.status.as_u16();
            let latency = Instant::now().duration_since(started_at).as_millis() as u64;

            // Emit monitor error event
            llm_guard.complete_error(
                &state,
                &provider_name,
                &request.model,
                status_code,
                &err_text,
            );

            tracing::error!(
                "Image generation failed: latency={}ms, error={}",
                latency,
                err_text
            );
            return Err(api_err);
        }
    };

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

    // Emit monitor response event
    let image_count = api_response.data.len();
    llm_guard.complete(
        &state,
        &provider_name,
        &request.model,
        200,
        0,
        0,
        None,
        None,
        latency_ms,
        Some("stop"),
        &format!("[{} image(s) generated]", image_count),
        false,
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
            return Err(ApiErrorResponse::bad_request("n must be between 1 and 10").with_param("n"));
        }
    }

    // Validate size if provided
    if let Some(size) = &request.size {
        validate_size(size)?;
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
            return Err(ApiErrorResponse::bad_request(
                "response_format must be 'url' or 'b64_json'",
            )
            .with_param("response_format"));
        }
    }

    Ok(())
}

/// `auto`, or `WIDTHxHEIGHT` with each side 64..=4096 pixels (providers
/// reject sizes their models do not support).
fn validate_size(size: &str) -> ApiResult<()> {
    if size == "auto" {
        return Ok(());
    }
    let ok = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .is_some_and(|(w, h)| (64..=4096).contains(&w) && (64..=4096).contains(&h));
    if ok {
        Ok(())
    } else {
        Err(ApiErrorResponse::bad_request(format!(
            "Invalid size '{size}'. Use WIDTHxHEIGHT (64 to 4096 pixels each), e.g. 1024x1024, or auto"
        ))
        .with_param("size"))
    }
}

/// The client must be enabled and allowed LLM access, and its strategy must
/// allow `model` (the internal Try It Out token skips the client checks).
fn check_image_access(state: &AppState, auth: &AuthContext, model: &str) -> ApiResult<()> {
    if auth.api_key_id != "internal-test" {
        let client = get_enabled_client(state, &auth.api_key_id)?;
        check_llm_access_with_state(state, &client)?;
    }
    if let Ok((_, ref strategy)) = get_client_with_strategy(state, &auth.api_key_id) {
        check_strategy_permission(strategy)?;
        validate_strategy_model_access(state, strategy, model)?;
    }
    Ok(())
}

/// Largest single uploaded image or mask.
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
/// Most reference images per edit.
const MAX_EDIT_IMAGES: usize = 16;

/// Multipart form of `POST /v1/images/edits` (documentation only).
#[derive(utoipa::ToSchema)]
#[allow(dead_code)]
pub struct ImageEditForm {
    /// `provider/model`, e.g. `stable-diffusion.cpp/qwen-image-2.1`
    model: String,
    /// What to change
    prompt: String,
    /// Reference image(s); repeat `image[]` for several (PNG, JPEG or WebP)
    #[schema(value_type = Vec<String>, format = Binary)]
    #[allow(non_snake_case)]
    image: Vec<Vec<u8>>,
    /// Optional mask; transparent areas are edited
    #[schema(value_type = Option<String>, format = Binary)]
    mask: Option<Vec<u8>>,
    /// Number of images (1-10)
    n: Option<u32>,
    /// `WIDTHxHEIGHT` or `auto` (defaults to the first image's size)
    size: Option<String>,
    /// `b64_json` (default) or `url`
    response_format: Option<String>,
    user: Option<String>,
}

/// POST /v1/images/edits
/// Edit images with a prompt
#[utoipa::path(
    post,
    path = "/v1/images/edits",
    tag = "images",
    request_body(content = ImageEditForm, content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "Successful response", body = ImageGenerationResponse),
        (status = 400, description = "Bad request", body = crate::types::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 403, description = "Forbidden", body = crate::types::ErrorResponse),
        (status = 502, description = "Provider error", body = crate::types::ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn image_edits(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    mut multipart: axum::extract::Multipart,
) -> ApiResult<Response> {
    const ENDPOINT: &str = "/v1/images/edits";
    state.emit_event("llm-request", "image");
    let session_id = uuid::Uuid::new_v4().to_string();
    state.record_client_activity(&auth.api_key_id);

    let bad = |state: &AppState, param: Option<&str>, msg: String| {
        super::monitor_helpers::emit_validation_error(
            state,
            None,
            Some(&session_id),
            ENDPOINT,
            param,
            &msg,
            400,
        );
        let err = ApiErrorResponse::bad_request(msg);
        match param {
            Some(p) => err.with_param(p),
            None => err,
        }
    };

    let mut model: Option<String> = None;
    let mut prompt: Option<String> = None;
    let mut images: Vec<lr_providers::ImageInput> = Vec::new();
    let mut mask: Option<lr_providers::ImageInput> = None;
    let mut n: Option<u32> = None;
    let mut size: Option<String> = None;
    let mut response_format: Option<String> = None;
    let mut user: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| bad(&state, None, format!("Invalid multipart data: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "image" | "image[]" | "mask" => {
                let file_name = field.file_name().unwrap_or("image.png").to_string();
                let content_type = field
                    .content_type()
                    .filter(|t| t.starts_with("image/"))
                    .unwrap_or("image/png")
                    .to_string();
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| bad(&state, Some(&name), format!("Failed to read {name}: {e}")))?
                    .to_vec();
                if data.is_empty() {
                    return Err(bad(&state, Some(&name), format!("{name} is empty")));
                }
                if data.len() > MAX_IMAGE_BYTES {
                    return Err(bad(
                        &state,
                        Some(&name),
                        format!("{name} is larger than {} MB", MAX_IMAGE_BYTES / 1024 / 1024),
                    ));
                }
                let input = lr_providers::ImageInput {
                    data,
                    file_name,
                    content_type,
                };
                if name == "mask" {
                    mask = Some(input);
                } else {
                    images.push(input);
                }
            }
            "model" | "prompt" | "n" | "size" | "response_format" | "user" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| bad(&state, Some(&name), format!("Invalid {name} field: {e}")))?;
                match name.as_str() {
                    "model" => model = Some(text),
                    "prompt" => prompt = Some(text),
                    "n" => {
                        n = Some(text.trim().parse().map_err(|_| {
                            bad(&state, Some("n"), "n must be a number".to_string())
                        })?)
                    }
                    "size" => size = Some(text),
                    "response_format" => response_format = Some(text),
                    _ => user = Some(text),
                }
            }
            // OpenAI fields providers here do not use (quality, background,
            // output_format, ...).
            _ => {}
        }
    }

    let model = model
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| bad(&state, Some("model"), "model is required".to_string()))?;
    let prompt = prompt
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| bad(&state, Some("prompt"), "prompt is required".to_string()))?;
    if prompt.len() > 32_000 {
        return Err(bad(
            &state,
            Some("prompt"),
            "prompt must be 32000 characters or less".to_string(),
        ));
    }
    if images.is_empty() {
        return Err(bad(
            &state,
            Some("image"),
            "at least one image is required".to_string(),
        ));
    }
    if images.len() > MAX_EDIT_IMAGES {
        return Err(bad(
            &state,
            Some("image"),
            format!("at most {MAX_EDIT_IMAGES} images are allowed"),
        ));
    }
    if n.is_some_and(|n| n == 0 || n > 10) {
        return Err(bad(
            &state,
            Some("n"),
            "n must be between 1 and 10".to_string(),
        ));
    }
    if let Some(s) = &size {
        validate_size(s).map_err(|e| bad(&state, Some("size"), e.error.error.message.clone()))?;
    }
    if let Some(f) = &response_format {
        if f != "url" && f != "b64_json" {
            return Err(bad(
                &state,
                Some("response_format"),
                "response_format must be 'url' or 'b64_json'".to_string(),
            ));
        }
    }

    // Monitor: the request without image bytes.
    let request_json = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "images": images.iter().map(|i| serde_json::json!({
            "file_name": i.file_name, "content_type": i.content_type, "bytes": i.data.len()
        })).collect::<Vec<_>>(),
        "mask": mask.as_ref().map(|m| serde_json::json!({
            "file_name": m.file_name, "content_type": m.content_type, "bytes": m.data.len()
        })),
        "n": n,
        "size": size,
        "response_format": response_format,
    });
    let mut llm_guard = super::monitor_helpers::emit_llm_call(
        &state,
        None,
        Some(&session_id),
        ENDPOINT,
        &model,
        false,
        &request_json,
    );

    check_image_access(&state, &auth, &model).map_err(|e| llm_guard.capture_err(e))?;

    let Some((provider_name, model_name)) = model
        .split_once('/')
        .map(|(p, m)| (p.to_string(), m.to_string()))
    else {
        return Err(llm_guard.capture_err(
            ApiErrorResponse::bad_request(
                "Model must be in provider/model format (e.g. stable-diffusion.cpp/qwen-image-2.1)",
            )
            .with_param("model"),
        ));
    };
    let provider = state
        .provider_registry
        .get_provider(&provider_name)
        .ok_or_else(|| {
            llm_guard.capture_err(
                ApiErrorResponse::bad_request(format!("Provider '{provider_name}' not found"))
                    .with_param("model"),
            )
        })?;
    if !provider.supports_image_edits() {
        return Err(llm_guard.capture_err(
            ApiErrorResponse::bad_request(format!(
                "Provider '{provider_name}' does not support image edits"
            ))
            .with_param("model"),
        ));
    }

    let started_at = Instant::now();
    let result = provider
        .edit_image(lr_providers::ImageEditRequest {
            model: model_name,
            prompt,
            images,
            mask,
            n,
            size,
            response_format,
            user,
        })
        .await;
    let latency_ms = started_at.elapsed().as_millis() as u64;
    let provider_response = match result {
        Ok(r) => r,
        Err(e) => {
            let err_text = e.to_string();
            let api_err: ApiErrorResponse = e.into();
            llm_guard.complete_error(
                &state,
                &provider_name,
                &model,
                api_err.status.as_u16(),
                &err_text,
            );
            tracing::error!("Image edit failed: latency={latency_ms}ms, error={err_text}");
            return Err(api_err);
        }
    };

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
    tracing::info!(
        "Image edit completed: client={}, model={}, latency={}ms",
        auth.api_key_id,
        model,
        latency_ms
    );
    llm_guard.complete(
        &state,
        &provider_name,
        &model,
        200,
        0,
        0,
        None,
        None,
        latency_ms,
        Some("stop"),
        &format!("[{} image(s) edited]", api_response.data.len()),
        false,
    );
    Ok(Json(api_response).into_response())
}

#[cfg(test)]
mod tests {
    use super::validate_size;

    #[test]
    fn sizes() {
        for ok in ["auto", "1024x1024", "1024x768", "64x4096"] {
            assert!(validate_size(ok).is_ok(), "{ok}");
        }
        for bad in ["", "1024", "0x1024", "5000x100", "axb", "1024x1024x3"] {
            assert!(validate_size(bad).is_err(), "{bad}");
        }
    }
}
