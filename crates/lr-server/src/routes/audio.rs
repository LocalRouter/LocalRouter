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
use std::time::Instant;
use uuid::Uuid;

use super::helpers::{check_llm_access, get_enabled_client};
use crate::middleware::error::{ApiErrorResponse, ApiResult};
use crate::state::{AppState, AuthContext};
use crate::types::{AudioTranscriptionResponse, SpeechRequest};

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
    mut multipart: axum::extract::Multipart,
) -> ApiResult<Response> {
    state.emit_event("llm-request", "audio");
    state.record_client_activity(&auth.api_key_id);

    if auth.api_key_id != "internal-test" {
        let client = get_enabled_client(&state, &auth.api_key_id)?;
        check_llm_access(&client)?;
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

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiErrorResponse::bad_request(format!("Invalid multipart data: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                file_name = field.file_name().map(|s| s.to_string());
                file_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| {
                            ApiErrorResponse::bad_request(format!("Failed to read file: {}", e))
                        })?
                        .to_vec(),
                );
            }
            "model" => {
                model = Some(field.text().await.map_err(|e| {
                    ApiErrorResponse::bad_request(format!("Invalid model field: {}", e))
                })?);
            }
            "language" => {
                language = Some(field.text().await.map_err(|e| {
                    ApiErrorResponse::bad_request(format!("Invalid language field: {}", e))
                })?);
            }
            "prompt" => {
                prompt = Some(field.text().await.map_err(|e| {
                    ApiErrorResponse::bad_request(format!("Invalid prompt field: {}", e))
                })?);
            }
            "response_format" => {
                response_format = Some(field.text().await.map_err(|e| {
                    ApiErrorResponse::bad_request(format!("Invalid response_format field: {}", e))
                })?);
            }
            "temperature" => {
                let text = field.text().await.map_err(|e| {
                    ApiErrorResponse::bad_request(format!("Invalid temperature field: {}", e))
                })?;
                temperature = Some(text.parse::<f32>().map_err(|_| {
                    ApiErrorResponse::bad_request("temperature must be a number")
                        .with_param("temperature")
                })?);
            }
            "timestamp_granularities[]" => {
                let text = field.text().await.map_err(|e| {
                    ApiErrorResponse::bad_request(format!(
                        "Invalid timestamp_granularities field: {}",
                        e
                    ))
                })?;
                timestamp_granularities
                    .get_or_insert_with(Vec::new)
                    .push(text);
            }
            _ => {} // Ignore unknown fields
        }
    }

    let file_data = file_data
        .ok_or_else(|| ApiErrorResponse::bad_request("file is required").with_param("file"))?;
    let model = model
        .ok_or_else(|| ApiErrorResponse::bad_request("model is required").with_param("model"))?;

    if model.is_empty() {
        return Err(ApiErrorResponse::bad_request("model cannot be empty").with_param("model"));
    }

    let request_id = format!("audio-{}", Uuid::new_v4());
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

    let response = state
        .router
        .transcribe(&auth.api_key_id, provider_request)
        .await
        .map_err(|e| {
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
            tracing::error!("Audio transcription failed: {}", e);
            ApiErrorResponse::bad_gateway(format!("Provider error: {}", e))
        })?;

    let latency_ms = Instant::now().duration_since(started_at).as_millis() as u64;

    tracing::info!(
        "Audio transcription completed: id={}, latency={}ms",
        request_id,
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
    mut multipart: axum::extract::Multipart,
) -> ApiResult<Response> {
    state.emit_event("llm-request", "audio");
    state.record_client_activity(&auth.api_key_id);

    if auth.api_key_id != "internal-test" {
        let client = get_enabled_client(&state, &auth.api_key_id)?;
        check_llm_access(&client)?;
    }

    // Parse multipart form fields
    let mut file_data: Option<Vec<u8>> = None;
    let mut file_name: Option<String> = None;
    let mut model: Option<String> = None;
    let mut prompt: Option<String> = None;
    let mut response_format: Option<String> = None;
    let mut temperature: Option<f32> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiErrorResponse::bad_request(format!("Invalid multipart data: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                file_name = field.file_name().map(|s| s.to_string());
                file_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| {
                            ApiErrorResponse::bad_request(format!("Failed to read file: {}", e))
                        })?
                        .to_vec(),
                );
            }
            "model" => {
                model = Some(field.text().await.map_err(|e| {
                    ApiErrorResponse::bad_request(format!("Invalid model field: {}", e))
                })?);
            }
            "prompt" => {
                prompt = Some(field.text().await.map_err(|e| {
                    ApiErrorResponse::bad_request(format!("Invalid prompt field: {}", e))
                })?);
            }
            "response_format" => {
                response_format = Some(field.text().await.map_err(|e| {
                    ApiErrorResponse::bad_request(format!("Invalid response_format field: {}", e))
                })?);
            }
            "temperature" => {
                let text = field.text().await.map_err(|e| {
                    ApiErrorResponse::bad_request(format!("Invalid temperature field: {}", e))
                })?;
                temperature = Some(text.parse::<f32>().map_err(|_| {
                    ApiErrorResponse::bad_request("temperature must be a number")
                        .with_param("temperature")
                })?);
            }
            _ => {}
        }
    }

    let file_data = file_data
        .ok_or_else(|| ApiErrorResponse::bad_request("file is required").with_param("file"))?;
    let model = model
        .ok_or_else(|| ApiErrorResponse::bad_request("model is required").with_param("model"))?;

    if model.is_empty() {
        return Err(ApiErrorResponse::bad_request("model cannot be empty").with_param("model"));
    }

    let request_id = format!("audio-{}", Uuid::new_v4());
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

    let response = state
        .router
        .translate_audio(&auth.api_key_id, provider_request)
        .await
        .map_err(|e| {
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
            tracing::error!("Audio translation failed: {}", e);
            ApiErrorResponse::bad_gateway(format!("Provider error: {}", e))
        })?;

    let latency_ms = Instant::now().duration_since(started_at).as_millis() as u64;

    tracing::info!(
        "Audio translation completed: id={}, latency={}ms",
        request_id,
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
    Json(request): Json<SpeechRequest>,
) -> ApiResult<Response> {
    state.emit_event("llm-request", "audio");
    state.record_client_activity(&auth.api_key_id);

    if auth.api_key_id != "internal-test" {
        let client = get_enabled_client(&state, &auth.api_key_id)?;
        check_llm_access(&client)?;
    }

    // Validate request
    validate_speech_request(&request)?;

    let request_id = format!("tts-{}", Uuid::new_v4());
    let started_at = Instant::now();

    let provider_request = lr_providers::SpeechRequest {
        model: request.model.clone(),
        input: request.input.clone(),
        voice: request.voice.clone(),
        response_format: request.response_format.clone(),
        speed: request.speed,
    };

    let response = state
        .router
        .speech(&auth.api_key_id, provider_request)
        .await
        .map_err(|e| {
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
            tracing::error!("Speech generation failed: {}", e);
            ApiErrorResponse::bad_gateway(format!("Provider error: {}", e))
        })?;

    let latency_ms = Instant::now().duration_since(started_at).as_millis() as u64;

    tracing::info!(
        "Speech generation completed: id={}, size={}B, latency={}ms",
        request_id,
        response.audio_data.len(),
        latency_ms
    );

    // Return binary audio response
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, response.content_type)
        .header(header::TRANSFER_ENCODING, "chunked")
        .body(Body::from(response.audio_data))
        .unwrap())
}

/// Validate speech request
fn validate_speech_request(request: &SpeechRequest) -> ApiResult<()> {
    if request.model.is_empty() {
        return Err(ApiErrorResponse::bad_request("model is required").with_param("model"));
    }

    if request.input.is_empty() {
        return Err(ApiErrorResponse::bad_request("input is required").with_param("input"));
    }

    if request.input.len() > 4096 {
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

    #[test]
    fn test_validate_speech_request_valid() {
        let request = SpeechRequest {
            model: "tts-1".to_string(),
            input: "Hello, world!".to_string(),
            voice: "alloy".to_string(),
            response_format: None,
            speed: None,
        };
        assert!(validate_speech_request(&request).is_ok());
    }

    #[test]
    fn test_validate_speech_request_empty_model() {
        let request = SpeechRequest {
            model: "".to_string(),
            input: "Hello".to_string(),
            voice: "alloy".to_string(),
            response_format: None,
            speed: None,
        };
        assert!(validate_speech_request(&request).is_err());
    }

    #[test]
    fn test_validate_speech_request_empty_input() {
        let request = SpeechRequest {
            model: "tts-1".to_string(),
            input: "".to_string(),
            voice: "alloy".to_string(),
            response_format: None,
            speed: None,
        };
        assert!(validate_speech_request(&request).is_err());
    }

    #[test]
    fn test_validate_speech_request_input_too_long() {
        let request = SpeechRequest {
            model: "tts-1".to_string(),
            input: "a".repeat(4097),
            voice: "alloy".to_string(),
            response_format: None,
            speed: None,
        };
        assert!(validate_speech_request(&request).is_err());
    }

    #[test]
    fn test_validate_speech_request_empty_voice() {
        let request = SpeechRequest {
            model: "tts-1".to_string(),
            input: "Hello".to_string(),
            voice: "".to_string(),
            response_format: None,
            speed: None,
        };
        assert!(validate_speech_request(&request).is_err());
    }

    #[test]
    fn test_validate_speech_request_invalid_format() {
        let request = SpeechRequest {
            model: "tts-1".to_string(),
            input: "Hello".to_string(),
            voice: "alloy".to_string(),
            response_format: Some("invalid".to_string()),
            speed: None,
        };
        assert!(validate_speech_request(&request).is_err());
    }

    #[test]
    fn test_validate_speech_request_valid_formats() {
        for format in ["mp3", "opus", "aac", "flac", "wav", "pcm"] {
            let request = SpeechRequest {
                model: "tts-1".to_string(),
                input: "Hello".to_string(),
                voice: "alloy".to_string(),
                response_format: Some(format.to_string()),
                speed: None,
            };
            assert!(validate_speech_request(&request).is_ok());
        }
    }

    #[test]
    fn test_validate_speech_request_speed_bounds() {
        let mut request = SpeechRequest {
            model: "tts-1".to_string(),
            input: "Hello".to_string(),
            voice: "alloy".to_string(),
            response_format: None,
            speed: Some(0.1),
        };
        assert!(validate_speech_request(&request).is_err());

        request.speed = Some(5.0);
        assert!(validate_speech_request(&request).is_err());

        request.speed = Some(1.0);
        assert!(validate_speech_request(&request).is_ok());

        request.speed = Some(0.25);
        assert!(validate_speech_request(&request).is_ok());

        request.speed = Some(4.0);
        assert!(validate_speech_request(&request).is_ok());
    }
}
