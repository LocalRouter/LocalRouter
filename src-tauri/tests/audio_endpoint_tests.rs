//! End-to-end tests for audio endpoints (STT + TTS)
//!
//! Tests the full HTTP request flow:
//! - Multipart upload for transcription/translation
//! - JSON POST for speech
//! - Validation (missing fields, empty file, bad params)
//! - Auth (missing, invalid)
//! - Provider-level mock via wiremock
//! - Response format verification

use localrouter::clients::{ClientManager, TokenStore};
use localrouter::config::{AppConfig, Client, ConfigManager, Strategy};
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::openai_compatible::OpenAICompatibleProvider;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::providers::ModelProvider;
use localrouter::router::{RateLimiterManager, Router};
use localrouter::server;
use serde_json::json;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

// ============================================================================
// Test Infrastructure
// ============================================================================

fn create_test_client(id: &str, strategy_id: &str) -> Client {
    Client {
        id: id.to_string(),
        name: "Test Client".to_string(),
        enabled: true,
        allowed_llm_providers: vec![],
        mcp_server_access: lr_config::McpServerAccess::None,
        context_management_enabled: None,
        skills_access: lr_config::SkillsAccess::default(),
        created_at: chrono::Utc::now(),
        last_used: None,
        strategy_id: strategy_id.to_string(),
        roots: None,
        mcp_sampling_enabled: false,
        mcp_sampling_requires_approval: true,
        mcp_sampling_max_tokens: None,
        mcp_sampling_rate_limit: None,
        firewall: lr_config::FirewallRules::default(),
        marketplace_enabled: false,
        mcp_permissions: lr_config::McpPermissions::default(),
        skills_permissions: lr_config::SkillsPermissions::default(),
        coding_agents_permissions: lr_config::CodingAgentsPermissions::default(),
        coding_agent_permission: lr_config::PermissionState::Off,
        coding_agent_type: None,
        model_permissions: lr_config::ModelPermissions::default(),
        marketplace_permission: lr_config::PermissionState::Off,
        client_mode: lr_config::ClientMode::default(),
        template_id: None,
        sync_config: false,
        guardrails_enabled: None,
        guardrails: lr_config::ClientGuardrailsConfig::default(),
        prompt_compression: lr_config::ClientPromptCompressionConfig::default(),
        json_repair: lr_config::ClientJsonRepairConfig::default(),
        secret_scanning: lr_config::ClientSecretScanningConfig::default(),
        mcp_sampling_permission: lr_config::PermissionState::Ask,
        mcp_elicitation_permission: lr_config::PermissionState::Ask,
        catalog_compression_enabled: None,
        client_tools_indexing: None,
        memory_enabled: None,
        memory_folder: None,
    }
}

/// Start a test server and return the base URL + internal test secret
async fn start_test_server() -> (String, String) {
    let test_client = create_test_client("test-api-key", "default");
    let strategy = Strategy::new("Default".to_string());

    let config = AppConfig {
        clients: vec![test_client.clone()],
        strategies: vec![strategy],
        ..Default::default()
    };

    let config_path =
        std::env::temp_dir().join(format!("test_audio_{}.yaml", uuid::Uuid::new_v4()));
    let config_manager = Arc::new(ConfigManager::new(config, config_path));

    let provider_registry = Arc::new(ProviderRegistry::new());
    let mcp_server_manager = Arc::new(McpServerManager::new());

    let metrics_db_path =
        std::env::temp_dir().join(format!("test_audio_{}.db", uuid::Uuid::new_v4()));
    let metrics_db = Arc::new(MetricsDatabase::new(metrics_db_path).unwrap());
    let metrics_collector = Arc::new(MetricsCollector::new(metrics_db));

    let rate_limiter = Arc::new(RateLimiterManager::new(None));
    let router = Arc::new(Router::new(
        config_manager.clone(),
        provider_registry.clone(),
        rate_limiter.clone(),
        metrics_collector.clone(),
        Arc::new(lr_router::FreeTierManager::new(None)),
    ));
    let client_manager = Arc::new(ClientManager::new(vec![test_client]));
    let token_store = Arc::new(TokenStore::new());

    let test_port = 41000 + (std::process::id() % 10000) as u16;
    let server_config = server::ServerConfig {
        host: "127.0.0.1".to_string(),
        port: test_port,
        enable_cors: true,
    };

    let (state, _handle, actual_port) = server::start_server(
        server_config,
        router,
        mcp_server_manager,
        rate_limiter,
        provider_registry,
        config_manager,
        client_manager,
        token_store,
        metrics_collector,
        None,
    )
    .await
    .expect("Failed to start test server");

    let base_url = format!("http://127.0.0.1:{}", actual_port);
    let test_secret = state.get_internal_test_secret();

    sleep(Duration::from_millis(200)).await;

    (base_url, test_secret)
}

// ============================================================================
// Auth Tests
// ============================================================================

#[tokio::test]
async fn test_audio_transcription_requires_auth() {
    let (base_url, _secret) = start_test_server().await;
    let client = reqwest::Client::new();

    // No auth header → 401
    let form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .part(
            "file",
            reqwest::multipart::Part::bytes(vec![0u8; 100])
                .file_name("test.wav")
                .mime_str("audio/wav")
                .unwrap(),
        );

    let response = client
        .post(format!("{}/v1/audio/transcriptions", base_url))
        .multipart(form)
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_audio_translation_requires_auth() {
    let (base_url, _secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .part(
            "file",
            reqwest::multipart::Part::bytes(vec![0u8; 100])
                .file_name("test.wav")
                .mime_str("audio/wav")
                .unwrap(),
        );

    let response = client
        .post(format!("{}/v1/audio/translations", base_url))
        .multipart(form)
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_audio_speech_requires_auth() {
    let (base_url, _secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/v1/audio/speech", base_url))
        .json(&json!({
            "model": "tts-1",
            "input": "Hello",
            "voice": "alloy"
        }))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_audio_speech_invalid_auth() {
    let (base_url, _secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/v1/audio/speech", base_url))
        .bearer_auth("totally-invalid-key")
        .json(&json!({
            "model": "tts-1",
            "input": "Hello",
            "voice": "alloy"
        }))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 401);
}

// ============================================================================
// Validation Tests (with auth)
// ============================================================================

#[tokio::test]
async fn test_audio_speech_validation_empty_model() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/v1/audio/speech", base_url))
        .bearer_auth(&secret)
        .json(&json!({
            "model": "",
            "input": "Hello",
            "voice": "alloy"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["error"]["message"].as_str().unwrap().contains("model"));
}

#[tokio::test]
async fn test_audio_speech_validation_empty_input() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/v1/audio/speech", base_url))
        .bearer_auth(&secret)
        .json(&json!({
            "model": "tts-1",
            "input": "",
            "voice": "alloy"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["error"]["message"].as_str().unwrap().contains("input"));
}

#[tokio::test]
async fn test_audio_speech_validation_empty_voice() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/v1/audio/speech", base_url))
        .bearer_auth(&secret)
        .json(&json!({
            "model": "tts-1",
            "input": "Hello",
            "voice": ""
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["error"]["message"].as_str().unwrap().contains("voice"));
}

#[tokio::test]
async fn test_audio_speech_validation_invalid_format() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/v1/audio/speech", base_url))
        .bearer_auth(&secret)
        .json(&json!({
            "model": "tts-1",
            "input": "Hello",
            "voice": "alloy",
            "response_format": "mp2"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("response_format"));
}

#[tokio::test]
async fn test_audio_speech_validation_speed_out_of_range() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/v1/audio/speech", base_url))
        .bearer_auth(&secret)
        .json(&json!({
            "model": "tts-1",
            "input": "Hello",
            "voice": "alloy",
            "speed": 10.0
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["error"]["message"].as_str().unwrap().contains("speed"));
}

#[tokio::test]
async fn test_audio_speech_validation_input_too_long() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let long_input = "a".repeat(4097);
    let response = client
        .post(format!("{}/v1/audio/speech", base_url))
        .bearer_auth(&secret)
        .json(&json!({
            "model": "tts-1",
            "input": long_input,
            "voice": "alloy"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["error"]["message"].as_str().unwrap().contains("4096"));
}

#[tokio::test]
async fn test_audio_transcription_validation_missing_file() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    // Only send model, no file
    let form = reqwest::multipart::Form::new().text("model", "whisper-1");

    let response = client
        .post(format!("{}/v1/audio/transcriptions", base_url))
        .bearer_auth(&secret)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["error"]["message"].as_str().unwrap().contains("file"));
}

#[tokio::test]
async fn test_audio_transcription_validation_missing_model() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    // Only send file, no model
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(vec![0u8; 100])
            .file_name("test.wav")
            .mime_str("audio/wav")
            .unwrap(),
    );

    let response = client
        .post(format!("{}/v1/audio/transcriptions", base_url))
        .bearer_auth(&secret)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["error"]["message"].as_str().unwrap().contains("model"));
}

#[tokio::test]
async fn test_audio_transcription_validation_empty_file() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .part(
            "file",
            reqwest::multipart::Part::bytes(vec![]) // empty
                .file_name("test.wav")
                .mime_str("audio/wav")
                .unwrap(),
        );

    let response = client
        .post(format!("{}/v1/audio/transcriptions", base_url))
        .bearer_auth(&secret)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["error"]["message"].as_str().unwrap().contains("empty"));
}

#[tokio::test]
async fn test_audio_transcription_validation_bad_temperature() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .text("temperature", "1.5") // out of 0-1 range
        .part(
            "file",
            reqwest::multipart::Part::bytes(vec![0u8; 100])
                .file_name("test.wav")
                .mime_str("audio/wav")
                .unwrap(),
        );

    let response = client
        .post(format!("{}/v1/audio/transcriptions", base_url))
        .bearer_auth(&secret)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("temperature"));
}

#[tokio::test]
async fn test_audio_transcription_validation_non_numeric_temperature() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .text("temperature", "hot")
        .part(
            "file",
            reqwest::multipart::Part::bytes(vec![0u8; 100])
                .file_name("test.wav")
                .mime_str("audio/wav")
                .unwrap(),
        );

    let response = client
        .post(format!("{}/v1/audio/transcriptions", base_url))
        .bearer_auth(&secret)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("temperature"));
}

#[tokio::test]
async fn test_audio_transcription_validation_bad_response_format() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .text("response_format", "mp3") // invalid for STT
        .part(
            "file",
            reqwest::multipart::Part::bytes(vec![0u8; 100])
                .file_name("test.wav")
                .mime_str("audio/wav")
                .unwrap(),
        );

    let response = client
        .post(format!("{}/v1/audio/transcriptions", base_url))
        .bearer_auth(&secret)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("response_format"));
}

#[tokio::test]
async fn test_audio_transcription_valid_response_formats() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    // All valid STT response formats should not get a 400 for response_format
    for fmt in ["json", "text", "srt", "verbose_json", "vtt"] {
        let form = reqwest::multipart::Form::new()
            .text("model", "whisper-1")
            .text("response_format", fmt)
            .part(
                "file",
                reqwest::multipart::Part::bytes(vec![0u8; 100])
                    .file_name("test.wav")
                    .mime_str("audio/wav")
                    .unwrap(),
            );

        let response = client
            .post(format!("{}/v1/audio/transcriptions", base_url))
            .bearer_auth(&secret)
            .multipart(form)
            .send()
            .await
            .unwrap();

        // Should NOT be 400 for response_format — it should pass validation
        // and fail later at provider routing (502) or model not found (404)
        assert_ne!(
            response.status().as_u16(),
            400,
            "Format '{}' should be valid for STT but got 400",
            fmt
        );
    }
}

// ============================================================================
// Without-prefix route tests
// ============================================================================

#[tokio::test]
async fn test_audio_speech_works_without_v1_prefix() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    // /audio/speech (without /v1) should also work — validation applies
    let response = client
        .post(format!("{}/audio/speech", base_url))
        .bearer_auth(&secret)
        .json(&json!({
            "model": "",
            "input": "Hello",
            "voice": "alloy"
        }))
        .send()
        .await
        .unwrap();

    // Route exists (not 404) — handler reached
    assert_ne!(
        response.status().as_u16(),
        404,
        "Route /audio/speech should exist without /v1 prefix"
    );
}

#[tokio::test]
async fn test_audio_transcription_works_without_v1_prefix() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    // /audio/transcriptions (without /v1) should also work
    let form = reqwest::multipart::Form::new().text("model", "whisper-1");

    let response = client
        .post(format!("{}/audio/transcriptions", base_url))
        .bearer_auth(&secret)
        .multipart(form)
        .send()
        .await
        .unwrap();

    // Route exists (not 404) — handler reached
    assert_ne!(
        response.status().as_u16(),
        404,
        "Route /audio/transcriptions should exist without /v1 prefix"
    );
}

#[tokio::test]
async fn test_audio_translation_works_without_v1_prefix() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let form = reqwest::multipart::Form::new().text("model", "whisper-1");

    let response = client
        .post(format!("{}/audio/translations", base_url))
        .bearer_auth(&secret)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_ne!(
        response.status().as_u16(),
        404,
        "Route /audio/translations should exist without /v1 prefix"
    );
}

// ============================================================================
// Provider-level Mock Tests (wiremock)
// ============================================================================

fn create_openai_compatible_provider(base_url: String) -> OpenAICompatibleProvider {
    OpenAICompatibleProvider::new(
        "test-audio".to_string(),
        base_url,
        Some("test-key".to_string()),
    )
}

#[tokio::test]
async fn test_provider_transcribe_success() {
    let mock_server = MockServer::start().await;

    // Mock whisper transcription response
    Mock::given(method("POST"))
        .and(path("/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "text": "Hello, this is a test transcription.",
            "task": "transcribe",
            "language": "en",
            "duration": 3.5
        })))
        .mount(&mock_server)
        .await;

    let provider = localrouter::providers::openai::OpenAIProvider::with_base_url(
        "test-key".to_string(),
        mock_server.uri(),
    )
    .unwrap();

    let request = localrouter::providers::AudioTranscriptionRequest {
        file: vec![0u8; 1000], // fake audio data
        file_name: "test.wav".to_string(),
        model: "whisper-1".to_string(),
        language: Some("en".to_string()),
        prompt: None,
        response_format: Some("verbose_json".to_string()),
        temperature: Some(0.0),
        timestamp_granularities: None,
    };

    let response = provider.transcribe(request).await.unwrap();

    assert_eq!(response.text, "Hello, this is a test transcription.");
    assert_eq!(response.task.as_deref(), Some("transcribe"));
    assert_eq!(response.language.as_deref(), Some("en"));
    assert_eq!(response.duration, Some(3.5));
}

#[tokio::test]
async fn test_provider_transcribe_with_words() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "text": "Hello world",
            "task": "transcribe",
            "language": "en",
            "duration": 1.0,
            "words": [
                {"word": "Hello", "start": 0.0, "end": 0.5},
                {"word": "world", "start": 0.5, "end": 1.0}
            ]
        })))
        .mount(&mock_server)
        .await;

    let provider = localrouter::providers::openai::OpenAIProvider::with_base_url(
        "test-key".to_string(),
        mock_server.uri(),
    )
    .unwrap();

    let request = localrouter::providers::AudioTranscriptionRequest {
        file: vec![0u8; 500],
        file_name: "test.mp3".to_string(),
        model: "whisper-1".to_string(),
        language: None,
        prompt: None,
        response_format: Some("verbose_json".to_string()),
        temperature: None,
        timestamp_granularities: Some(vec!["word".to_string()]),
    };

    let response = provider.transcribe(request).await.unwrap();

    assert_eq!(response.text, "Hello world");
    let words = response.words.unwrap();
    assert_eq!(words.len(), 2);
    assert_eq!(words[0].word, "Hello");
    assert_eq!(words[1].end, 1.0);
}

#[tokio::test]
async fn test_provider_translate_audio_success() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/audio/translations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "text": "Hello in English",
            "task": "translate",
            "language": "en",
            "duration": 2.0
        })))
        .mount(&mock_server)
        .await;

    let provider = localrouter::providers::openai::OpenAIProvider::with_base_url(
        "test-key".to_string(),
        mock_server.uri(),
    )
    .unwrap();

    let request = localrouter::providers::AudioTranslationRequest {
        file: vec![0u8; 800],
        file_name: "french.mp3".to_string(),
        model: "whisper-1".to_string(),
        prompt: None,
        response_format: None,
        temperature: None,
    };

    let response = provider.translate_audio(request).await.unwrap();

    assert_eq!(response.text, "Hello in English");
    assert_eq!(response.task.as_deref(), Some("translate"));
}

#[tokio::test]
async fn test_provider_speech_success() {
    let mock_server = MockServer::start().await;

    // TTS returns binary audio data
    let fake_audio = vec![0xFF, 0xFB, 0x90, 0x00]; // fake MP3 header bytes
    Mock::given(method("POST"))
        .and(path("/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(fake_audio.clone(), "audio/mpeg"))
        .mount(&mock_server)
        .await;

    let provider = localrouter::providers::openai::OpenAIProvider::with_base_url(
        "test-key".to_string(),
        mock_server.uri(),
    )
    .unwrap();

    let request = localrouter::providers::SpeechRequest {
        model: "tts-1".to_string(),
        input: "Hello world".to_string(),
        voice: "alloy".to_string(),
        response_format: None,
        speed: None,
    };

    let response = provider.speech(request).await.unwrap();

    assert_eq!(response.audio_data, fake_audio);
    assert!(response.content_type.contains("audio/mpeg"));
}

#[tokio::test]
async fn test_provider_speech_with_format() {
    let mock_server = MockServer::start().await;

    let fake_opus = vec![0x4F, 0x67, 0x67, 0x53]; // fake Ogg header
    Mock::given(method("POST"))
        .and(path("/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(fake_opus.clone(), "audio/opus"))
        .mount(&mock_server)
        .await;

    let provider = localrouter::providers::openai::OpenAIProvider::with_base_url(
        "test-key".to_string(),
        mock_server.uri(),
    )
    .unwrap();

    let request = localrouter::providers::SpeechRequest {
        model: "tts-1-hd".to_string(),
        input: "Testing opus format".to_string(),
        voice: "nova".to_string(),
        response_format: Some("opus".to_string()),
        speed: Some(1.5),
    };

    let response = provider.speech(request).await.unwrap();

    assert_eq!(response.audio_data, fake_opus);
    assert!(response.content_type.contains("audio/opus"));
}

#[tokio::test]
async fn test_provider_transcribe_auth_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": {"message": "Invalid API key", "type": "invalid_request_error"}
        })))
        .mount(&mock_server)
        .await;

    let provider = localrouter::providers::openai::OpenAIProvider::with_base_url(
        "bad-key".to_string(),
        mock_server.uri(),
    )
    .unwrap();

    let request = localrouter::providers::AudioTranscriptionRequest {
        file: vec![0u8; 100],
        file_name: "test.wav".to_string(),
        model: "whisper-1".to_string(),
        language: None,
        prompt: None,
        response_format: None,
        temperature: None,
        timestamp_granularities: None,
    };

    let result = provider.transcribe(request).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, lr_types::AppError::Unauthorized));
}

#[tokio::test]
async fn test_provider_speech_rate_limited() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/audio/speech"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "error": {"message": "Rate limit exceeded", "type": "rate_limit_error"}
        })))
        .mount(&mock_server)
        .await;

    let provider = localrouter::providers::openai::OpenAIProvider::with_base_url(
        "test-key".to_string(),
        mock_server.uri(),
    )
    .unwrap();

    let request = localrouter::providers::SpeechRequest {
        model: "tts-1".to_string(),
        input: "Hello".to_string(),
        voice: "alloy".to_string(),
        response_format: None,
        speed: None,
    };

    let result = provider.speech(request).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, lr_types::AppError::RateLimitExceeded));
}

#[tokio::test]
async fn test_provider_transcribe_minimal_response() {
    let mock_server = MockServer::start().await;

    // Some providers return only { "text": "..." }
    Mock::given(method("POST"))
        .and(path("/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "text": "Just text, nothing else"
        })))
        .mount(&mock_server)
        .await;

    let provider = localrouter::providers::openai::OpenAIProvider::with_base_url(
        "test-key".to_string(),
        mock_server.uri(),
    )
    .unwrap();

    let request = localrouter::providers::AudioTranscriptionRequest {
        file: vec![0u8; 200],
        file_name: "audio.wav".to_string(),
        model: "whisper-1".to_string(),
        language: None,
        prompt: None,
        response_format: None,
        temperature: None,
        timestamp_granularities: None,
    };

    let response = provider.transcribe(request).await.unwrap();

    assert_eq!(response.text, "Just text, nothing else");
    assert!(response.task.is_none());
    assert!(response.language.is_none());
    assert!(response.duration.is_none());
    assert!(response.words.is_none());
    assert!(response.segments.is_none());
}

// ============================================================================
// Default trait (unsupported provider) tests
// ============================================================================

#[tokio::test]
async fn test_unsupported_provider_returns_error_for_transcribe() {
    // OpenAI-compatible provider doesn't implement audio by default
    let mock_server = MockServer::start().await;
    let provider = create_openai_compatible_provider(mock_server.uri());

    let request = localrouter::providers::AudioTranscriptionRequest {
        file: vec![0u8; 100],
        file_name: "test.wav".to_string(),
        model: "whisper-1".to_string(),
        language: None,
        prompt: None,
        response_format: None,
        temperature: None,
        timestamp_granularities: None,
    };

    let result = provider.transcribe(request).await;
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("does not support audio transcription"));
}

#[tokio::test]
async fn test_unsupported_provider_returns_error_for_translate() {
    let mock_server = MockServer::start().await;
    let provider = create_openai_compatible_provider(mock_server.uri());

    let request = localrouter::providers::AudioTranslationRequest {
        file: vec![0u8; 100],
        file_name: "test.wav".to_string(),
        model: "whisper-1".to_string(),
        prompt: None,
        response_format: None,
        temperature: None,
    };

    let result = provider.translate_audio(request).await;
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("does not support audio translation"));
}

#[tokio::test]
async fn test_unsupported_provider_returns_error_for_speech() {
    let mock_server = MockServer::start().await;
    let provider = create_openai_compatible_provider(mock_server.uri());

    let request = localrouter::providers::SpeechRequest {
        model: "tts-1".to_string(),
        input: "Hello".to_string(),
        voice: "alloy".to_string(),
        response_format: None,
        speed: None,
    };

    let result = provider.speech(request).await;
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("does not support text-to-speech"));
}

// ============================================================================
// OpenAPI spec tests
// ============================================================================

#[tokio::test]
async fn test_openapi_includes_audio_paths() {
    let (base_url, _secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let response = client
        .get(format!("{}/openapi.json", base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let spec: serde_json::Value = response.json().await.unwrap();

    let paths = spec["paths"].as_object().unwrap();
    assert!(
        paths.contains_key("/v1/audio/transcriptions"),
        "OpenAPI spec missing /v1/audio/transcriptions"
    );
    assert!(
        paths.contains_key("/v1/audio/translations"),
        "OpenAPI spec missing /v1/audio/translations"
    );
    assert!(
        paths.contains_key("/v1/audio/speech"),
        "OpenAPI spec missing /v1/audio/speech"
    );

    // Verify they have POST methods
    assert!(spec["paths"]["/v1/audio/transcriptions"]["post"].is_object());
    assert!(spec["paths"]["/v1/audio/translations"]["post"].is_object());
    assert!(spec["paths"]["/v1/audio/speech"]["post"].is_object());

    // Verify audio tag exists
    let tags: Vec<&str> = spec["tags"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(tags.contains(&"audio"), "OpenAPI spec missing 'audio' tag");
}
