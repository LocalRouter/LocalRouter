//! Audio endpoints (STT + TTS)
//!
//! POST /v1/audio/transcriptions — Speech-to-text
//! POST /v1/audio/translations  — Speech-to-English translation
//! POST /v1/audio/speech         — Text-to-speech

use axum::{
    body::Body,
    extract::State,
    http::header,
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::Utc;
use std::time::Instant;
use uuid::Uuid;

use super::helpers::{
    check_llm_access_with_state, check_strategy_permission, get_client_with_strategy,
    get_enabled_client, get_enabled_client_from_manager, validate_strategy_model_access,
};
use crate::middleware::client_auth::ClientAuthContext;
use crate::middleware::error::{ApiErrorResponse, ApiResult};
use crate::state::{AppState, AuthContext, GenerationDetails};
use crate::types::{AudioTranscriptionResponse, SpeechRequest, TokenUsage};

/// POST /v1/audio/transcriptions
/// Transcribe audio to text
#[utoipa::path(
    post,
    path = "/v1/audio/transcriptions",
    tag = "audio",
    responses(
        (status = 200, description = "Successful transcription", body = AudioTranscriptionResponse),
        (status = 400, description = "Bad request", body = crate::types::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 502, description = "Provider error", body = crate::types::ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn audio_transcriptions(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    mut multipart: axum::extract::Multipart,
) -> ApiResult<Response> {
    state.emit_event("llm-request", "audio");
    let session_id = uuid::Uuid::new_v4().to_string();

    // Emit monitor event for traffic inspection (model not yet known from multipart)
    // Will be emitted after multipart parsing below

    state.record_client_activity(&auth.api_key_id);

    if auth.api_key_id != "internal-test" {
        let client = get_enabled_client(&state, &auth.api_key_id)?;
        check_llm_access_with_state(&state, &client)?;
    }

    // Parse multipart form fields
    let mut file_data: Option<Vec<u8>> = None;
    let mut file_name: Option<String> = None;
    let mut model: Option<String> = None;
    let mut language: Option<String> = None;
    let mut prompt: Option<String> = None;
    let mut response_format: Option<String> = None;
    let mut temperature: Option<f32> = None;
    let mut timestamp_granularities: Option<Vec<String>> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        let msg = format!("Invalid multipart data: {}", e);
        super::monitor_helpers::emit_validation_error(
            &state,
            client_auth.as_ref(),
            Some(&session_id),
            "/v1/audio/transcriptions",
            None,
            &msg,
            400,
        );
        ApiErrorResponse::bad_request(msg)
    })? {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                file_name = field.file_name().map(|s| s.to_string());
                file_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| {
                            let msg = format!("Failed to read file: {}", e);
                            super::monitor_helpers::emit_validation_error(
                                &state,
                                client_auth.as_ref(),
                                Some(&session_id),
                                "/v1/audio/transcriptions",
                                Some("file"),
                                &msg,
                                400,
                            );
                            ApiErrorResponse::bad_request(msg)
                        })?
                        .to_vec(),
                );
            }
            "model" => {
                model = Some(field.text().await.map_err(|e| {
                    let msg = format!("Invalid model field: {}", e);
                    super::monitor_helpers::emit_validation_error(
                        &state,
                        client_auth.as_ref(),
                        Some(&session_id),
                        "/v1/audio/transcriptions",
                        Some("model"),
                        &msg,
                        400,
                    );
                    ApiErrorResponse::bad_request(msg)
                })?);
            }
            "language" => {
                language = Some(field.text().await.map_err(|e| {
                    let msg = format!("Invalid language field: {}", e);
                    super::monitor_helpers::emit_validation_error(
                        &state,
                        client_auth.as_ref(),
                        Some(&session_id),
                        "/v1/audio/transcriptions",
                        Some("language"),
                        &msg,
                        400,
                    );
                    ApiErrorResponse::bad_request(msg)
                })?);
            }
            "prompt" => {
                prompt = Some(field.text().await.map_err(|e| {
                    let msg = format!("Invalid prompt field: {}", e);
                    super::monitor_helpers::emit_validation_error(
                        &state,
                        client_auth.as_ref(),
                        Some(&session_id),
                        "/v1/audio/transcriptions",
                        Some("prompt"),
                        &msg,
                        400,
                    );
                    ApiErrorResponse::bad_request(msg)
                })?);
            }
            "response_format" => {
                response_format = Some(field.text().await.map_err(|e| {
                    let msg = format!("Invalid response_format field: {}", e);
                    super::monitor_helpers::emit_validation_error(
                        &state,
                        client_auth.as_ref(),
                        Some(&session_id),
                        "/v1/audio/transcriptions",
                        Some("response_format"),
                        &msg,
                        400,
                    );
                    ApiErrorResponse::bad_request(msg)
                })?);
            }
            "temperature" => {
                let text = field.text().await.map_err(|e| {
                    let msg = format!("Invalid temperature field: {}", e);
                    super::monitor_helpers::emit_validation_error(
                        &state,
                        client_auth.as_ref(),
                        Some(&session_id),
                        "/v1/audio/transcriptions",
                        Some("temperature"),
                        &msg,
                        400,
                    );
                    ApiErrorResponse::bad_request(msg)
                })?;
                temperature = Some(text.parse::<f32>().map_err(|_| {
                    super::monitor_helpers::emit_validation_error(
                        &state,
                        client_auth.as_ref(),
                        Some(&session_id),
                        "/v1/audio/transcriptions",
                        Some("temperature"),
                        "temperature must be a number",
                        400,
                    );
                    ApiErrorResponse::bad_request("temperature must be a number")
                        .with_param("temperature")
                })?);
            }
            "timestamp_granularities[]" => {
                let text = field.text().await.map_err(|e| {
                    let msg = format!("Invalid timestamp_granularities field: {}", e);
                    super::monitor_helpers::emit_validation_error(
                        &state,
                        client_auth.as_ref(),
                        Some(&session_id),
                        "/v1/audio/transcriptions",
                        Some("timestamp_granularities"),
                        &msg,
                        400,
                    );
                    ApiErrorResponse::bad_request(msg)
                })?;
                timestamp_granularities
                    .get_or_insert_with(Vec::new)
                    .push(text);
            }
            _ => {} // Ignore unknown fields
        }
    }

    let file_data = file_data.ok_or_else(|| {
        super::monitor_helpers::emit_validation_error(
            &state,
            client_auth.as_ref(),
            Some(&session_id),
            "/v1/audio/transcriptions",
            Some("file"),
            "file is required",
            400,
        );
        ApiErrorResponse::bad_request("file is required").with_param("file")
    })?;
    let model = model.ok_or_else(|| {
        super::monitor_helpers::emit_validation_error(
            &state,
            client_auth.as_ref(),
            Some(&session_id),
            "/v1/audio/transcriptions",
            Some("model"),
            "model is required",
            400,
        );
        ApiErrorResponse::bad_request("model is required").with_param("model")
    })?;

    if file_data.is_empty() {
        super::monitor_helpers::emit_validation_error(
            &state,
            client_auth.as_ref(),
            Some(&session_id),
            "/v1/audio/transcriptions",
            Some("file"),
            "file cannot be empty",
            400,
        );
        return Err(ApiErrorResponse::bad_request("file cannot be empty").with_param("file"));
    }

    if model.is_empty() {
        super::monitor_helpers::emit_validation_error(
            &state,
            client_auth.as_ref(),
            Some(&session_id),
            "/v1/audio/transcriptions",
            Some("model"),
            "model cannot be empty",
            400,
        );
        return Err(ApiErrorResponse::bad_request("model cannot be empty").with_param("model"));
    }

    // Validate temperature if provided
    if let Some(temp) = temperature {
        if !(0.0..=1.0).contains(&temp) {
            super::monitor_helpers::emit_validation_error(
                &state,
                client_auth.as_ref(),
                Some(&session_id),
                "/v1/audio/transcriptions",
                Some("temperature"),
                "temperature must be between 0 and 1",
                400,
            );
            return Err(
                ApiErrorResponse::bad_request("temperature must be between 0 and 1")
                    .with_param("temperature"),
            );
        }
    }

    // Validate response_format if provided
    if let Some(ref fmt) = response_format {
        let valid_formats = ["json", "text", "srt", "verbose_json", "vtt"];
        if !valid_formats.contains(&fmt.as_str()) {
            let msg = format!(
                "Invalid response_format '{}'. Valid formats: {}",
                fmt,
                valid_formats.join(", ")
            );
            super::monitor_helpers::emit_validation_error(
                &state,
                client_auth.as_ref(),
                Some(&session_id),
                "/v1/audio/transcriptions",
                Some("response_format"),
                &msg,
                400,
            );
            return Err(ApiErrorResponse::bad_request(msg).with_param("response_format"));
        }
    }

    // Strategy-level model access checks
    if let Ok((_, ref strategy)) = get_client_with_strategy(&state, &auth.api_key_id) {
        check_strategy_permission(strategy)?;
        validate_strategy_model_access(&state, strategy, &model)?;
    }

    // Validate client provider access
    validate_client_provider_access(&state, client_auth.as_ref().map(|e| &e.0), &model).await?;

    // Emit monitor event for traffic inspection
    let monitor_body = serde_json::json!({"model": &model, "endpoint": "/v1/audio/transcriptions"});
    let llm_guard = super::monitor_helpers::emit_llm_call(
        &state,
        client_auth.as_ref(),
        Some(&session_id),
        "/v1/audio/transcriptions",
        &model,
        false,
        &monitor_body,
    );

    let request_id = format!("audio-{}", Uuid::new_v4());
    let created_at = Utc::now();
    let started_at = Instant::now();

    let provider_request = lr_providers::AudioTranscriptionRequest {
        file: file_data,
        file_name: file_name.unwrap_or_else(|| "audio.wav".to_string()),
        model,
        language,
        prompt,
        response_format,
        temperature,
        timestamp_granularities,
    };

    let model_for_log = provider_request.model.clone();

    let response = match state
        .router
        .transcribe(&auth.api_key_id, provider_request)
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            let latency = Instant::now().duration_since(started_at).as_millis() as u64;
            let strategy_id = state
                .client_manager
                .get_client(&auth.api_key_id)
                .map(|c| c.strategy_id.clone())
                .unwrap_or_else(|| "default".to_string());
            state.metrics_collector.record_failure(
                &auth.api_key_id,
                "unknown",
                &model_for_log,
                &strategy_id,
                latency,
            );
            if let Err(log_err) = state.access_logger.log_failure(
                &auth.api_key_id,
                "unknown",
                &model_for_log,
                latency,
                &request_id,
                502,
            ) {
                tracing::warn!("Failed to write access log: {}", log_err);
            }

            // Emit monitor error event
            llm_guard.complete_error(&state, "unknown", &model_for_log, 502, &e.to_string());

            tracing::error!("Audio transcription failed: {}", e);
            return Err(ApiErrorResponse::bad_gateway(format!(
                "Provider error: {}",
                e
            )));
        }
    };

    let completed_at = Instant::now();
    let latency_ms = completed_at.duration_since(started_at).as_millis() as u64;

    // Extract provider from model string for metrics
    let provider = model_for_log
        .split('/')
        .next()
        .unwrap_or("unknown")
        .to_string();

    let strategy_id = state
        .client_manager
        .get_client(&auth.api_key_id)
        .map(|c| c.strategy_id.clone())
        .unwrap_or_else(|| "default".to_string());

    // Record success metrics
    state
        .metrics_collector
        .record_success(&lr_monitoring::metrics::RequestMetrics {
            api_key_name: &auth.api_key_id,
            provider: &provider,
            model: &model_for_log,
            strategy_id: &strategy_id,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            latency_ms,
        });

    // Log to access log
    if let Err(e) = state.access_logger.log_success(
        &auth.api_key_id,
        &provider,
        &model_for_log,
        0,
        0,
        0.0,
        latency_ms,
        &request_id,
    ) {
        tracing::warn!("Failed to write access log: {}", e);
    }

    // Emit monitor response event
    let content_preview = if response.text.len() > 200 {
        &response.text[..200]
    } else {
        &response.text
    };
    llm_guard.complete(
        &state,
        &provider,
        &model_for_log,
        200,
        0,
        0,
        None,
        None,
        latency_ms,
        Some("stop"),
        content_preview,
        false,
    );

    // Record generation for tracking
    let generation_details = GenerationDetails {
        id: request_id.clone(),
        model: model_for_log.clone(),
        provider: provider.clone(),
        created_at,
        finish_reason: "stop".to_string(),
        tokens: TokenUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        cost: None,
        started_at,
        completed_at,
        provider_health: None,
        api_key_id: auth.api_key_id,
        user: None,
        stream: false,
    };
    state
        .generation_tracker
        .record(generation_details.id.clone(), generation_details);

    tracing::info!(
        "Audio transcription completed: id={}, model={}, latency={}ms",
        request_id,
        model_for_log,
        latency_ms
    );

    let api_response = AudioTranscriptionResponse {
        text: response.text,
        task: response.task,
        language: response.language,
        duration: response.duration,
        words: response.words,
        segments: response.segments,
    };

    Ok(Json(api_response).into_response())
}

/// POST /v1/audio/translations
/// Translate audio to English text
#[utoipa::path(
    post,
    path = "/v1/audio/translations",
    tag = "audio",
    responses(
        (status = 200, description = "Successful translation", body = AudioTranscriptionResponse),
        (status = 400, description = "Bad request", body = crate::types::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 502, description = "Provider error", body = crate::types::ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn audio_translations(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    mut multipart: axum::extract::Multipart,
) -> ApiResult<Response> {
    state.emit_event("llm-request", "audio");
    let session_id = uuid::Uuid::new_v4().to_string();
    state.record_client_activity(&auth.api_key_id);

    if auth.api_key_id != "internal-test" {
        let client = get_enabled_client(&state, &auth.api_key_id)?;
        check_llm_access_with_state(&state, &client)?;
    }

    // Parse multipart form fields
    let mut file_data: Option<Vec<u8>> = None;
    let mut file_name: Option<String> = None;
    let mut model: Option<String> = None;
    let mut prompt: Option<String> = None;
    let mut response_format: Option<String> = None;
    let mut temperature: Option<f32> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        let msg = format!("Invalid multipart data: {}", e);
        super::monitor_helpers::emit_validation_error(
            &state,
            client_auth.as_ref(),
            Some(&session_id),
            "/v1/audio/translations",
            None,
            &msg,
            400,
        );
        ApiErrorResponse::bad_request(msg)
    })? {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                file_name = field.file_name().map(|s| s.to_string());
                file_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| {
                            let msg = format!("Failed to read file: {}", e);
                            super::monitor_helpers::emit_validation_error(
                                &state,
                                client_auth.as_ref(),
                                Some(&session_id),
                                "/v1/audio/translations",
                                Some("file"),
                                &msg,
                                400,
                            );
                            ApiErrorResponse::bad_request(msg)
                        })?
                        .to_vec(),
                );
            }
            "model" => {
                model = Some(field.text().await.map_err(|e| {
                    let msg = format!("Invalid model field: {}", e);
                    super::monitor_helpers::emit_validation_error(
                        &state,
                        client_auth.as_ref(),
                        Some(&session_id),
                        "/v1/audio/translations",
                        Some("model"),
                        &msg,
                        400,
                    );
                    ApiErrorResponse::bad_request(msg)
                })?);
            }
            "prompt" => {
                prompt = Some(field.text().await.map_err(|e| {
                    let msg = format!("Invalid prompt field: {}", e);
                    super::monitor_helpers::emit_validation_error(
                        &state,
                        client_auth.as_ref(),
                        Some(&session_id),
                        "/v1/audio/translations",
                        Some("prompt"),
                        &msg,
                        400,
                    );
                    ApiErrorResponse::bad_request(msg)
                })?);
            }
            "response_format" => {
                response_format = Some(field.text().await.map_err(|e| {
                    let msg = format!("Invalid response_format field: {}", e);
                    super::monitor_helpers::emit_validation_error(
                        &state,
                        client_auth.as_ref(),
                        Some(&session_id),
                        "/v1/audio/translations",
                        Some("response_format"),
                        &msg,
                        400,
                    );
                    ApiErrorResponse::bad_request(msg)
                })?);
            }
            "temperature" => {
                let text = field.text().await.map_err(|e| {
                    let msg = format!("Invalid temperature field: {}", e);
                    super::monitor_helpers::emit_validation_error(
                        &state,
                        client_auth.as_ref(),
                        Some(&session_id),
                        "/v1/audio/translations",
                        Some("temperature"),
                        &msg,
                        400,
                    );
                    ApiErrorResponse::bad_request(msg)
                })?;
                temperature = Some(text.parse::<f32>().map_err(|_| {
                    super::monitor_helpers::emit_validation_error(
                        &state,
                        client_auth.as_ref(),
                        Some(&session_id),
                        "/v1/audio/translations",
                        Some("temperature"),
                        "temperature must be a number",
                        400,
                    );
                    ApiErrorResponse::bad_request("temperature must be a number")
                        .with_param("temperature")
                })?);
            }
            _ => {}
        }
    }

    let file_data = file_data.ok_or_else(|| {
        super::monitor_helpers::emit_validation_error(
            &state,
            client_auth.as_ref(),
            Some(&session_id),
            "/v1/audio/translations",
            Some("file"),
            "file is required",
            400,
        );
        ApiErrorResponse::bad_request("file is required").with_param("file")
    })?;
    let model = model.ok_or_else(|| {
        super::monitor_helpers::emit_validation_error(
            &state,
            client_auth.as_ref(),
            Some(&session_id),
            "/v1/audio/translations",
            Some("model"),
            "model is required",
            400,
        );
        ApiErrorResponse::bad_request("model is required").with_param("model")
    })?;

    if file_data.is_empty() {
        super::monitor_helpers::emit_validation_error(
            &state,
            client_auth.as_ref(),
            Some(&session_id),
            "/v1/audio/translations",
            Some("file"),
            "file cannot be empty",
            400,
        );
        return Err(ApiErrorResponse::bad_request("file cannot be empty").with_param("file"));
    }

    if model.is_empty() {
        super::monitor_helpers::emit_validation_error(
            &state,
            client_auth.as_ref(),
            Some(&session_id),
            "/v1/audio/translations",
            Some("model"),
            "model cannot be empty",
            400,
        );
        return Err(ApiErrorResponse::bad_request("model cannot be empty").with_param("model"));
    }

    if let Some(temp) = temperature {
        if !(0.0..=1.0).contains(&temp) {
            super::monitor_helpers::emit_validation_error(
                &state,
                client_auth.as_ref(),
                Some(&session_id),
                "/v1/audio/translations",
                Some("temperature"),
                "temperature must be between 0 and 1",
                400,
            );
            return Err(
                ApiErrorResponse::bad_request("temperature must be between 0 and 1")
                    .with_param("temperature"),
            );
        }
    }

    if let Some(ref fmt) = response_format {
        let valid_formats = ["json", "text", "srt", "verbose_json", "vtt"];
        if !valid_formats.contains(&fmt.as_str()) {
            let msg = format!(
                "Invalid response_format '{}'. Valid formats: {}",
                fmt,
                valid_formats.join(", ")
            );
            super::monitor_helpers::emit_validation_error(
                &state,
                client_auth.as_ref(),
                Some(&session_id),
                "/v1/audio/translations",
                Some("response_format"),
                &msg,
                400,
            );
            return Err(ApiErrorResponse::bad_request(msg).with_param("response_format"));
        }
    }

    // Strategy-level model access checks
    if let Ok((_, ref strategy)) = get_client_with_strategy(&state, &auth.api_key_id) {
        check_strategy_permission(strategy)?;
        validate_strategy_model_access(&state, strategy, &model)?;
    }

    // Validate client provider access
    validate_client_provider_access(&state, client_auth.as_ref().map(|e| &e.0), &model).await?;

    // Emit monitor event for traffic inspection
    let monitor_body = serde_json::json!({"model": &model, "endpoint": "/v1/audio/translations"});
    let llm_guard = super::monitor_helpers::emit_llm_call(
        &state,
        client_auth.as_ref(),
        Some(&session_id),
        "/v1/audio/translations",
        &model,
        false,
        &monitor_body,
    );

    let request_id = format!("audio-{}", Uuid::new_v4());
    let created_at = Utc::now();
    let started_at = Instant::now();

    let provider_request = lr_providers::AudioTranslationRequest {
        file: file_data,
        file_name: file_name.unwrap_or_else(|| "audio.wav".to_string()),
        model,
        prompt,
        response_format,
        temperature,
    };

    let model_for_log = provider_request.model.clone();

    let response = match state
        .router
        .translate_audio(&auth.api_key_id, provider_request)
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            let latency = Instant::now().duration_since(started_at).as_millis() as u64;
            let strategy_id = state
                .client_manager
                .get_client(&auth.api_key_id)
                .map(|c| c.strategy_id.clone())
                .unwrap_or_else(|| "default".to_string());
            state.metrics_collector.record_failure(
                &auth.api_key_id,
                "unknown",
                &model_for_log,
                &strategy_id,
                latency,
            );
            if let Err(log_err) = state.access_logger.log_failure(
                &auth.api_key_id,
                "unknown",
                &model_for_log,
                latency,
                &request_id,
                502,
            ) {
                tracing::warn!("Failed to write access log: {}", log_err);
            }

            // Emit monitor error event
            llm_guard.complete_error(&state, "unknown", &model_for_log, 502, &e.to_string());

            tracing::error!("Audio translation failed: {}", e);
            return Err(ApiErrorResponse::bad_gateway(format!(
                "Provider error: {}",
                e
            )));
        }
    };

    let completed_at = Instant::now();
    let latency_ms = completed_at.duration_since(started_at).as_millis() as u64;

    let provider = model_for_log
        .split('/')
        .next()
        .unwrap_or("unknown")
        .to_string();

    let strategy_id = state
        .client_manager
        .get_client(&auth.api_key_id)
        .map(|c| c.strategy_id.clone())
        .unwrap_or_else(|| "default".to_string());

    state
        .metrics_collector
        .record_success(&lr_monitoring::metrics::RequestMetrics {
            api_key_name: &auth.api_key_id,
            provider: &provider,
            model: &model_for_log,
            strategy_id: &strategy_id,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            latency_ms,
        });

    if let Err(e) = state.access_logger.log_success(
        &auth.api_key_id,
        &provider,
        &model_for_log,
        0,
        0,
        0.0,
        latency_ms,
        &request_id,
    ) {
        tracing::warn!("Failed to write access log: {}", e);
    }

    // Emit monitor response event
    let content_preview = if response.text.len() > 200 {
        &response.text[..200]
    } else {
        &response.text
    };
    llm_guard.complete(
        &state,
        &provider,
        &model_for_log,
        200,
        0,
        0,
        None,
        None,
        latency_ms,
        Some("stop"),
        content_preview,
        false,
    );

    let generation_details = GenerationDetails {
        id: request_id.clone(),
        model: model_for_log.clone(),
        provider: provider.clone(),
        created_at,
        finish_reason: "stop".to_string(),
        tokens: TokenUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        cost: None,
        started_at,
        completed_at,
        provider_health: None,
        api_key_id: auth.api_key_id,
        user: None,
        stream: false,
    };
    state
        .generation_tracker
        .record(generation_details.id.clone(), generation_details);

    tracing::info!(
        "Audio translation completed: id={}, model={}, latency={}ms",
        request_id,
        model_for_log,
        latency_ms
    );

    let api_response = AudioTranscriptionResponse {
        text: response.text,
        task: response.task,
        language: response.language,
        duration: response.duration,
        words: response.words,
        segments: response.segments,
    };

    Ok(Json(api_response).into_response())
}

/// POST /v1/audio/speech
/// Generate speech from text
#[utoipa::path(
    post,
    path = "/v1/audio/speech",
    tag = "audio",
    request_body = SpeechRequest,
    responses(
        (status = 200, description = "Audio data", content_type = "audio/mpeg"),
        (status = 400, description = "Bad request", body = crate::types::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 502, description = "Provider error", body = crate::types::ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn audio_speech(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    Json(request): Json<SpeechRequest>,
) -> ApiResult<Response> {
    state.emit_event("llm-request", "audio");
    let session_id = uuid::Uuid::new_v4().to_string();

    // Emit monitor event for traffic inspection
    let monitor_body = serde_json::json!({"model": &request.model, "endpoint": "/v1/audio/speech", "voice": &request.voice});
    let mut llm_guard = super::monitor_helpers::emit_llm_call(
        &state,
        client_auth.as_ref(),
        Some(&session_id),
        "/v1/audio/speech",
        &request.model,
        false,
        &monitor_body,
    );

    state.record_client_activity(&auth.api_key_id);

    if auth.api_key_id != "internal-test" {
        let client =
            get_enabled_client(&state, &auth.api_key_id).map_err(|e| llm_guard.capture_err(e))?;
        check_llm_access_with_state(&state, &client).map_err(|e| llm_guard.capture_err(e))?;
    }

    // Validate request
    if let Err(e) = validate_speech_request(&request) {
        super::monitor_helpers::emit_validation_error(
            &state,
            client_auth.as_ref(),
            Some(&session_id),
            "/v1/audio/speech",
            e.error.error.param.as_deref(),
            &e.error.error.message,
            400,
        );
        return Err(llm_guard.capture_err(e));
    }

    // Strategy-level model access checks
    if let Ok((_, ref strategy)) = get_client_with_strategy(&state, &auth.api_key_id) {
        check_strategy_permission(strategy).map_err(|e| llm_guard.capture_err(e))?;
        validate_strategy_model_access(&state, strategy, &request.model)
            .map_err(|e| llm_guard.capture_err(e))?;
    }

    // Validate client provider access
    validate_client_provider_access(&state, client_auth.as_ref().map(|e| &e.0), &request.model)
        .await
        .map_err(|e| llm_guard.capture_err(e))?;

    let request_id = format!("tts-{}", Uuid::new_v4());
    let created_at = Utc::now();
    let started_at = Instant::now();

    let provider_request = lr_providers::SpeechRequest {
        model: request.model.clone(),
        input: request.input.clone(),
        voice: request.voice.clone(),
        response_format: request.response_format.clone(),
        speed: request.speed,
    };

    let response = match state
        .router
        .speech(&auth.api_key_id, provider_request)
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            let latency = Instant::now().duration_since(started_at).as_millis() as u64;
            let strategy_id = state
                .client_manager
                .get_client(&auth.api_key_id)
                .map(|c| c.strategy_id.clone())
                .unwrap_or_else(|| "default".to_string());
            state.metrics_collector.record_failure(
                &auth.api_key_id,
                "unknown",
                &request.model,
                &strategy_id,
                latency,
            );
            if let Err(log_err) = state.access_logger.log_failure(
                &auth.api_key_id,
                "unknown",
                &request.model,
                latency,
                &request_id,
                502,
            ) {
                tracing::warn!("Failed to write access log: {}", log_err);
            }

            // Emit monitor error event
            llm_guard.complete_error(&state, "unknown", &request.model, 502, &e.to_string());

            tracing::error!("Speech generation failed: {}", e);
            return Err(ApiErrorResponse::bad_gateway(format!(
                "Provider error: {}",
                e
            )));
        }
    };

    let completed_at = Instant::now();
    let latency_ms = completed_at.duration_since(started_at).as_millis() as u64;

    let provider = request
        .model
        .split('/')
        .next()
        .unwrap_or("unknown")
        .to_string();

    let strategy_id = state
        .client_manager
        .get_client(&auth.api_key_id)
        .map(|c| c.strategy_id.clone())
        .unwrap_or_else(|| "default".to_string());

    // Estimate tokens from input text length (TTS: ~4 chars per token)
    let estimated_tokens = (request.input.len() as u64 / 4).max(1);

    state
        .metrics_collector
        .record_success(&lr_monitoring::metrics::RequestMetrics {
            api_key_name: &auth.api_key_id,
            provider: &provider,
            model: &request.model,
            strategy_id: &strategy_id,
            input_tokens: estimated_tokens,
            output_tokens: 0,
            cost_usd: 0.0,
            latency_ms,
        });

    if let Err(e) = state.access_logger.log_success(
        &auth.api_key_id,
        &provider,
        &request.model,
        estimated_tokens,
        0,
        0.0,
        latency_ms,
        &request_id,
    ) {
        tracing::warn!("Failed to write access log: {}", e);
    }

    // Emit monitor response event
    llm_guard.complete(
        &state,
        &provider,
        &request.model,
        200,
        estimated_tokens,
        0,
        None,
        None,
        latency_ms,
        Some("stop"),
        &format!("[audio: {}B]", response.audio_data.len()),
        false,
    );

    let generation_details = GenerationDetails {
        id: request_id.clone(),
        model: request.model.clone(),
        provider: provider.clone(),
        created_at,
        finish_reason: "stop".to_string(),
        tokens: TokenUsage {
            prompt_tokens: estimated_tokens as u32,
            completion_tokens: 0,
            total_tokens: estimated_tokens as u32,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        cost: None,
        started_at,
        completed_at,
        provider_health: None,
        api_key_id: auth.api_key_id,
        user: None,
        stream: false,
    };
    state
        .generation_tracker
        .record(generation_details.id.clone(), generation_details);

    tracing::info!(
        "Speech generation completed: id={}, size={}B, latency={}ms",
        request_id,
        response.audio_data.len(),
        latency_ms
    );

    // Return binary audio response with Content-Length
    let audio_len = response.audio_data.len();
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, response.content_type)
        .header(header::CONTENT_LENGTH, audio_len)
        .body(Body::from(response.audio_data))
        .unwrap())
}

/// Validate that the client has access to the requested audio model's provider
async fn validate_client_provider_access(
    state: &AppState,
    client_context: Option<&ClientAuthContext>,
    model: &str,
) -> ApiResult<()> {
    let Some(client_ctx) = client_context else {
        return Ok(());
    };

    let client = get_enabled_client_from_manager(state, &client_ctx.client_id)?;

    // Extract provider and model_id from model string
    let (provider, model_id) = if let Some((prov, m)) = model.split_once('/') {
        (prov.to_string(), m.to_string())
    } else {
        // No provider specified — find which provider has this model
        let all_models = state.provider_registry.list_all_models_instant();

        let matching_models: Vec<_> = all_models
            .iter()
            .filter(|m| m.id.eq_ignore_ascii_case(model))
            .collect();

        let matching_model = matching_models
            .iter()
            .find(|m| {
                client
                    .model_permissions
                    .resolve_model(&m.provider, &m.id)
                    .is_enabled()
            })
            .or(matching_models.first())
            .ok_or_else(|| {
                ApiErrorResponse::not_found(format!("Model not found: {}", model))
                    .with_param("model")
            })?;

        (matching_model.provider.clone(), matching_model.id.clone())
    };

    let permission_state = client.model_permissions.resolve_model(&provider, &model_id);

    if !permission_state.is_enabled() {
        tracing::warn!(
            "Client {} attempted to access unauthorized audio model: {}/{}",
            client.id,
            provider,
            model_id
        );

        super::monitor_helpers::emit_access_denied_for_client(
            state,
            &client.id,
            None,
            "model_not_allowed",
            "/v1/audio",
            &format!(
                "Access denied: Client is not authorized to use model '{}/{}'",
                provider, model_id
            ),
            403,
        );

        return Err(ApiErrorResponse::forbidden(format!(
            "Access denied: Client is not authorized to use model '{}/{}'. Contact administrator to grant access.",
            provider, model_id
        ))
        .with_param("model"));
    }

    Ok(())
}

/// Validate speech request
fn validate_speech_request(request: &SpeechRequest) -> ApiResult<()> {
    if request.model.is_empty() {
        return Err(ApiErrorResponse::bad_request("model is required").with_param("model"));
    }

    if request.input.is_empty() {
        return Err(ApiErrorResponse::bad_request("input is required").with_param("input"));
    }

    if request.input.chars().count() > 4096 {
        return Err(
            ApiErrorResponse::bad_request("input must be 4096 characters or less")
                .with_param("input"),
        );
    }

    if request.voice.is_empty() {
        return Err(ApiErrorResponse::bad_request("voice is required").with_param("voice"));
    }

    if let Some(format) = &request.response_format {
        let valid_formats = ["mp3", "opus", "aac", "flac", "wav", "pcm"];
        if !valid_formats.contains(&format.as_str()) {
            return Err(ApiErrorResponse::bad_request(format!(
                "Invalid response_format '{}'. Valid formats: {}",
                format,
                valid_formats.join(", ")
            ))
            .with_param("response_format"));
        }
    }

    if let Some(speed) = request.speed {
        if !(0.25..=4.0).contains(&speed) {
            return Err(
                ApiErrorResponse::bad_request("speed must be between 0.25 and 4.0")
                    .with_param("speed"),
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== Speech Validation Tests ====================

    fn make_speech_request(model: &str, input: &str, voice: &str) -> SpeechRequest {
        SpeechRequest {
            model: model.to_string(),
            input: input.to_string(),
            voice: voice.to_string(),
            response_format: None,
            speed: None,
        }
    }

    #[test]
    fn test_validate_speech_request_valid() {
        assert!(
            validate_speech_request(&make_speech_request("tts-1", "Hello, world!", "alloy"))
                .is_ok()
        );
    }

    #[test]
    fn test_validate_speech_request_all_fields() {
        let request = SpeechRequest {
            model: "tts-1-hd".to_string(),
            input: "Test input".to_string(),
            voice: "nova".to_string(),
            response_format: Some("opus".to_string()),
            speed: Some(1.5),
        };
        assert!(validate_speech_request(&request).is_ok());
    }

    #[test]
    fn test_validate_speech_request_empty_model() {
        assert!(validate_speech_request(&make_speech_request("", "Hello", "alloy")).is_err());
    }

    #[test]
    fn test_validate_speech_request_empty_input() {
        assert!(validate_speech_request(&make_speech_request("tts-1", "", "alloy")).is_err());
    }

    #[test]
    fn test_validate_speech_request_input_too_long_ascii() {
        let request = make_speech_request("tts-1", &"a".repeat(4097), "alloy");
        assert!(validate_speech_request(&request).is_err());
    }

    #[test]
    fn test_validate_speech_request_input_exactly_4096_chars() {
        let request = make_speech_request("tts-1", &"a".repeat(4096), "alloy");
        assert!(validate_speech_request(&request).is_ok());
    }

    #[test]
    fn test_validate_speech_request_input_multibyte_under_limit() {
        // 2000 multi-byte chars (each char is 3 bytes = 6000 bytes, but only 2000 chars)
        let input: String = std::iter::repeat_n('\u{1F600}', 2000).collect(); // emoji chars
        let request = make_speech_request("tts-1", &input, "alloy");
        assert!(validate_speech_request(&request).is_ok());
    }

    #[test]
    fn test_validate_speech_request_input_multibyte_over_limit() {
        // 4097 multi-byte chars — over limit by character count
        let input: String = std::iter::repeat_n('\u{00E9}', 4097).collect(); // é chars
        let request = make_speech_request("tts-1", &input, "alloy");
        assert!(validate_speech_request(&request).is_err());
    }

    #[test]
    fn test_validate_speech_request_empty_voice() {
        assert!(validate_speech_request(&make_speech_request("tts-1", "Hello", "")).is_err());
    }

    #[test]
    fn test_validate_speech_request_invalid_format() {
        let mut request = make_speech_request("tts-1", "Hello", "alloy");
        request.response_format = Some("invalid".to_string());
        assert!(validate_speech_request(&request).is_err());
    }

    #[test]
    fn test_validate_speech_request_valid_formats() {
        for format in ["mp3", "opus", "aac", "flac", "wav", "pcm"] {
            let mut request = make_speech_request("tts-1", "Hello", "alloy");
            request.response_format = Some(format.to_string());
            assert!(
                validate_speech_request(&request).is_ok(),
                "format '{}' should be valid",
                format
            );
        }
    }

    #[test]
    fn test_validate_speech_request_speed_too_low() {
        let mut request = make_speech_request("tts-1", "Hello", "alloy");
        request.speed = Some(0.1);
        assert!(validate_speech_request(&request).is_err());

        request.speed = Some(0.24);
        assert!(validate_speech_request(&request).is_err());
    }

    #[test]
    fn test_validate_speech_request_speed_too_high() {
        let mut request = make_speech_request("tts-1", "Hello", "alloy");
        request.speed = Some(4.01);
        assert!(validate_speech_request(&request).is_err());

        request.speed = Some(5.0);
        assert!(validate_speech_request(&request).is_err());
    }

    #[test]
    fn test_validate_speech_request_speed_boundaries() {
        let mut request = make_speech_request("tts-1", "Hello", "alloy");

        request.speed = Some(0.25);
        assert!(validate_speech_request(&request).is_ok());

        request.speed = Some(4.0);
        assert!(validate_speech_request(&request).is_ok());

        request.speed = Some(1.0);
        assert!(validate_speech_request(&request).is_ok());
    }

    #[test]
    fn test_validate_speech_request_speed_none_is_valid() {
        let request = make_speech_request("tts-1", "Hello", "alloy");
        assert!(request.speed.is_none());
        assert!(validate_speech_request(&request).is_ok());
    }

    // ==================== Audio Type Serialization Tests ====================

    #[test]
    fn test_speech_request_json_serialization() {
        let request = SpeechRequest {
            model: "tts-1".to_string(),
            input: "Hello".to_string(),
            voice: "alloy".to_string(),
            response_format: Some("mp3".to_string()),
            speed: Some(1.0),
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["model"], "tts-1");
        assert_eq!(json["input"], "Hello");
        assert_eq!(json["voice"], "alloy");
        assert_eq!(json["response_format"], "mp3");
        assert_eq!(json["speed"], 1.0);
    }

    #[test]
    fn test_speech_request_json_optional_fields_omitted() {
        let request = SpeechRequest {
            model: "tts-1".to_string(),
            input: "Hello".to_string(),
            voice: "alloy".to_string(),
            response_format: None,
            speed: None,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert!(json.get("response_format").is_none());
        assert!(json.get("speed").is_none());
    }

    #[test]
    fn test_speech_request_json_deserialization() {
        let json = r#"{"model": "tts-1", "input": "Hi", "voice": "echo"}"#;
        let request: SpeechRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.model, "tts-1");
        assert_eq!(request.input, "Hi");
        assert_eq!(request.voice, "echo");
        assert!(request.response_format.is_none());
        assert!(request.speed.is_none());
    }

    #[test]
    fn test_speech_request_json_deserialization_with_all_fields() {
        let json = r#"{"model": "tts-1-hd", "input": "Hi", "voice": "nova", "response_format": "opus", "speed": 2.0}"#;
        let request: SpeechRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.model, "tts-1-hd");
        assert_eq!(request.voice, "nova");
        assert_eq!(request.response_format.as_deref(), Some("opus"));
        assert_eq!(request.speed, Some(2.0));
    }

    #[test]
    fn test_transcription_response_json_serialization() {
        let response = AudioTranscriptionResponse {
            text: "Hello world".to_string(),
            task: Some("transcribe".to_string()),
            language: Some("en".to_string()),
            duration: Some(1.5),
            words: None,
            segments: None,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["text"], "Hello world");
        assert_eq!(json["task"], "transcribe");
        assert_eq!(json["language"], "en");
        assert_eq!(json["duration"], 1.5);
        assert!(json.get("words").is_none()); // skip_serializing_if
        assert!(json.get("segments").is_none());
    }

    #[test]
    fn test_transcription_response_minimal_json() {
        let response = AudioTranscriptionResponse {
            text: "Hello".to_string(),
            task: None,
            language: None,
            duration: None,
            words: None,
            segments: None,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["text"], "Hello");
        // All optional fields should be omitted
        let obj = json.as_object().unwrap();
        assert_eq!(obj.len(), 1, "Only 'text' field should be present");
    }

    #[test]
    fn test_transcription_response_with_words() {
        let response = AudioTranscriptionResponse {
            text: "Hello world".to_string(),
            task: None,
            language: None,
            duration: None,
            words: Some(vec![
                lr_providers::TranscriptionWord {
                    word: "Hello".to_string(),
                    start: 0.0,
                    end: 0.5,
                },
                lr_providers::TranscriptionWord {
                    word: "world".to_string(),
                    start: 0.5,
                    end: 1.0,
                },
            ]),
            segments: None,
        };
        let json = serde_json::to_value(&response).unwrap();
        let words = json["words"].as_array().unwrap();
        assert_eq!(words.len(), 2);
        assert_eq!(words[0]["word"], "Hello");
        assert_eq!(words[0]["start"], 0.0);
        assert_eq!(words[1]["word"], "world");
        assert_eq!(words[1]["end"], 1.0);
    }

    #[test]
    fn test_transcription_response_deserialization_from_provider() {
        // Simulate what a provider API would return
        let json = r#"{
            "text": "Bonjour le monde",
            "task": "transcribe",
            "language": "fr",
            "duration": 2.35,
            "words": [
                {"word": "Bonjour", "start": 0.0, "end": 0.8},
                {"word": "le", "start": 0.8, "end": 1.0},
                {"word": "monde", "start": 1.0, "end": 2.35}
            ]
        }"#;
        let response: lr_providers::AudioTranscriptionResponse =
            serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "Bonjour le monde");
        assert_eq!(response.language.as_deref(), Some("fr"));
        assert_eq!(response.duration, Some(2.35));
        assert_eq!(response.words.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn test_transcription_response_deserialization_minimal() {
        // Some providers return minimal JSON
        let json = r#"{"text": "Hello"}"#;
        let response: lr_providers::AudioTranscriptionResponse =
            serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "Hello");
        assert!(response.task.is_none());
        assert!(response.language.is_none());
        assert!(response.duration.is_none());
    }

    // ==================== Provider Audio Type Tests ====================

    #[test]
    fn test_audio_transcription_request_fields() {
        let req = lr_providers::AudioTranscriptionRequest {
            file: vec![0u8; 100],
            file_name: "test.mp3".to_string(),
            model: "whisper-1".to_string(),
            language: Some("en".to_string()),
            prompt: Some("context".to_string()),
            response_format: Some("verbose_json".to_string()),
            temperature: Some(0.5),
            timestamp_granularities: Some(vec!["word".to_string(), "segment".to_string()]),
        };
        assert_eq!(req.file.len(), 100);
        assert_eq!(req.file_name, "test.mp3");
        assert_eq!(req.model, "whisper-1");
        assert_eq!(req.language.as_deref(), Some("en"));
        assert_eq!(req.timestamp_granularities.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_audio_translation_request_no_language_field() {
        let req = lr_providers::AudioTranslationRequest {
            file: vec![0u8; 50],
            file_name: "audio.wav".to_string(),
            model: "whisper-1".to_string(),
            prompt: None,
            response_format: None,
            temperature: None,
        };
        // AudioTranslationRequest has no language field — translation always outputs English
        assert_eq!(req.model, "whisper-1");
    }

    #[test]
    fn test_speech_request_provider_type() {
        let req = lr_providers::SpeechRequest {
            model: "tts-1".to_string(),
            input: "Hello".to_string(),
            voice: "alloy".to_string(),
            response_format: None,
            speed: None,
        };
        // Provider SpeechRequest should be serializable (used as JSON body)
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "tts-1");
        assert_eq!(json["voice"], "alloy");
    }

    // ==================== Provider Feature Support Tests ====================

    #[test]
    fn test_transcription_word_serialization() {
        let word = lr_providers::TranscriptionWord {
            word: "hello".to_string(),
            start: 0.0,
            end: 0.5,
        };
        let json = serde_json::to_value(&word).unwrap();
        assert_eq!(json["word"], "hello");
        assert_eq!(json["start"], 0.0);
        assert_eq!(json["end"], 0.5);
    }

    #[test]
    fn test_transcription_segment_serialization() {
        let segment = lr_providers::TranscriptionSegment {
            id: 0,
            seek: 0,
            start: 0.0,
            end: 5.0,
            text: "Hello world".to_string(),
            tokens: vec![1, 2, 3],
            temperature: 0.0,
            avg_logprob: -0.5,
            compression_ratio: 1.2,
            no_speech_prob: 0.01,
        };
        let json = serde_json::to_value(&segment).unwrap();
        assert_eq!(json["id"], 0);
        assert_eq!(json["text"], "Hello world");
        assert_eq!(json["tokens"].as_array().unwrap().len(), 3);
        assert_eq!(json["no_speech_prob"], 0.01);
    }
}
