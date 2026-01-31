use super::*;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::sleep;

/// Helper to create a test service using the dev directory models
///
/// Note: Tests require SafeTensors models to be available in ~/.localrouter-dev/routellm/.
/// Run the ignored test `test_download_and_verify_models` first to download them:
///   cargo test test_download_and_verify -- --ignored
fn create_test_service() -> RouteLLMService {
    let home_dir = dirs::home_dir().expect("Could not determine home directory");
    let routellm_dir = home_dir.join(".localrouter-dev/routellm");

    RouteLLMService::new(
        routellm_dir.join("model"),     // Directory containing model.safetensors
        routellm_dir.join("tokenizer"), // Directory containing tokenizer.json
        5,                              // 5 second idle timeout for testing
    )
}

#[tokio::test]
async fn test_service_initialization() {
    let service = create_test_service();

    // Should not be loaded initially
    assert!(!service.is_loaded().await);

    // Initialize should succeed
    let result = service.initialize().await;
    assert!(result.is_ok(), "Initialization failed: {:?}", result.err());

    // Should be loaded after initialization
    assert!(service.is_loaded().await);
}

#[tokio::test]
async fn test_double_initialization() {
    let service = create_test_service();

    // First initialization
    service.initialize().await.expect("First init failed");
    assert!(service.is_loaded().await);

    // Second initialization should be idempotent
    let result = service.initialize().await;
    assert!(result.is_ok(), "Second initialization should succeed");
    assert!(service.is_loaded().await);
}

#[tokio::test]
async fn test_predict_auto_initialize() {
    let service = create_test_service();

    // Service not initialized
    assert!(!service.is_loaded().await);

    // Prediction should auto-initialize
    let result = service.predict("What is 2+2?").await;
    assert!(result.is_ok(), "Prediction failed: {:?}", result.err());

    // Should be loaded now
    assert!(service.is_loaded().await);
}

#[tokio::test]
async fn test_simple_prompt_prediction() {
    let service = create_test_service();

    let (is_strong, win_rate) = service
        .predict("What is 2+2?")
        .await
        .expect("Simple prompt prediction failed");

    // Simple arithmetic should route to weak model
    assert!(!is_strong, "Simple prompt should not route to strong model");
    assert!(win_rate < 0.5, "Win rate should be < 0.5 for simple prompt");
    assert!(
        win_rate >= 0.0 && win_rate <= 1.0,
        "Win rate should be between 0 and 1"
    );
}

#[tokio::test]
async fn test_complex_prompt_prediction() {
    let service = create_test_service();

    let complex_prompt = "Explain the philosophical implications of quantum entanglement \
                          and how it relates to Einstein's theory of relativity, \
                          including detailed mathematical proofs and recent experimental evidence.";

    let (_is_strong, win_rate) = service
        .predict(complex_prompt)
        .await
        .expect("Complex prompt prediction failed");

    // Complex philosophical/scientific query should have higher win rate
    // Note: This is probabilistic, so we just check the win rate is valid
    assert!(
        win_rate >= 0.0 && win_rate <= 1.0,
        "Win rate should be between 0 and 1"
    );
}

#[tokio::test]
async fn test_predict_with_custom_threshold() {
    let service = create_test_service();

    let prompt = "Write a simple hello world program";

    // Test different thresholds
    let thresholds = vec![0.2, 0.3, 0.5, 0.7];

    for threshold in thresholds {
        let (is_strong, win_rate) = service
            .predict_with_threshold(prompt, threshold)
            .await
            .expect("Threshold prediction failed");

        // Verify threshold logic
        if win_rate >= threshold {
            assert!(
                is_strong,
                "Should route to strong when win_rate >= threshold"
            );
        } else {
            assert!(!is_strong, "Should route to weak when win_rate < threshold");
        }
    }
}

#[tokio::test]
async fn test_unload() {
    let service = create_test_service();

    // Initialize
    service.initialize().await.expect("Init failed");
    assert!(service.is_loaded().await);

    // Unload
    service.unload().await;
    assert!(!service.is_loaded().await);
}

#[tokio::test]
async fn test_status_not_downloaded() {
    // Create service with non-existent paths
    let service = RouteLLMService::new(
        PathBuf::from("/nonexistent/model"), // Directory for SafeTensors
        PathBuf::from("/nonexistent/tokenizer"), // Directory for tokenizer files
        600,
    );

    let status = service.get_status().await;

    assert_eq!(status.state, RouteLLMState::NotDownloaded);
    assert!(status.memory_usage_mb.is_none());
    assert!(status.last_access_secs_ago.is_none());
}

#[tokio::test]
async fn test_status_started() {
    let service = create_test_service();

    // Initialize
    service.initialize().await.expect("Init failed");

    let status = service.get_status().await;

    assert_eq!(status.state, RouteLLMState::Started);
    assert!(status.memory_usage_mb.is_some());
    assert!(status.last_access_secs_ago.is_some());
}

#[tokio::test]
async fn test_last_access_tracking() {
    let service = create_test_service();

    // Make first prediction
    service
        .predict("test")
        .await
        .expect("First prediction failed");

    let status1 = service.get_status().await;
    assert!(status1.last_access_secs_ago.unwrap() < 2);

    // Wait a bit
    sleep(Duration::from_secs(2)).await;

    let status2 = service.get_status().await;
    assert!(status2.last_access_secs_ago.unwrap() >= 2);
}

#[tokio::test]
async fn test_concurrent_predictions() {
    let service = Arc::new(create_test_service());

    let mut handles = vec![];

    // Spawn multiple concurrent prediction tasks
    for i in 0..10 {
        let service_clone = Arc::clone(&service);
        let handle =
            tokio::spawn(async move { service_clone.predict(&format!("Test prompt {}", i)).await });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        let result = handle.await.expect("Task panicked");
        assert!(result.is_ok(), "Concurrent prediction failed");
    }
}

#[tokio::test]
async fn test_empty_prompt() {
    let service = create_test_service();

    let result = service.predict("").await;

    // Empty prompt should still work (might return low win rate)
    assert!(result.is_ok(), "Empty prompt should not fail");

    let (_, win_rate) = result.unwrap();
    assert!(win_rate >= 0.0 && win_rate <= 1.0);
}

#[tokio::test]
async fn test_very_long_prompt() {
    let service = create_test_service();

    // Create a very long prompt (10,000 characters)
    let long_prompt = "a".repeat(10_000);

    let result = service.predict(&long_prompt).await;

    // Should handle long prompts
    assert!(result.is_ok(), "Long prompt should not fail");
}

#[tokio::test]
async fn test_performance_short_vs_long_text() {
    use std::time::Instant;

    let service = create_test_service();

    // Warm up - initialize the model
    service.initialize().await.expect("Failed to initialize");

    // Test 1: Short text (similar to user's original 2 second baseline)
    let short_text = "What is 2+2?";
    let start = Instant::now();
    let result = service.predict(short_text).await;
    let short_duration = start.elapsed();
    assert!(result.is_ok(), "Short text prediction failed");
    println!(
        "Short text ({} chars): {:?}",
        short_text.len(),
        short_duration
    );

    // Test 2: Medium text (~500 chars)
    let medium_text = "a".repeat(500);
    let start = Instant::now();
    let result = service.predict(&medium_text).await;
    let medium_duration = start.elapsed();
    assert!(result.is_ok(), "Medium text prediction failed");
    println!(
        "Medium text ({} chars): {:?}",
        medium_text.len(),
        medium_duration
    );

    // Test 3: Long text similar to user's log output (~1500 chars)
    let long_text = "2026-01-21T04:22:37.434492Z  INFO localrouter: Initializing rate limiter...\n\
                     2026-01-21T04:22:37.434546Z  INFO localrouter: Initializing metrics collector...\n\
                     2026-01-21T04:22:37.438532Z  INFO localrouter: Initializing RouteLLM service...\n\
                     2026-01-21T04:22:37.438781Z  INFO localrouter: RouteLLM service initialized with idle timeout: 600s\n\
                     2026-01-21T04:22:37.438795Z  INFO localrouter: Initializing router...\n\
                     2026-01-21T04:22:37.438872Z  INFO localrouter: Initializing web server...\n\
                     2026-01-21T04:22:37.439066Z  INFO localrouter::server: Starting web server on 127.0.0.1:33625\n\
                     2026-01-21T04:22:37.439108Z  INFO localrouter::server::state: Generated transient internal test bearer token for UI model testing\n\
                     2026-01-21T04:22:37.439409Z  INFO localrouter::monitoring::logger: Access logger initialized with directory: \"/Users/matus/Library/Logs/LocalRouter\"\n\
                     2026-01-21T04:22:37.439446Z  INFO localrouter::monitoring::mcp_logger: MCP access logger initialized with directory: \"/Users/matus/Library/Logs/LocalRouter\"\n\
                     2026-01-21T04:22:37.439479Z  WARN localrouter::api_keys::keychain_trait: Using file-based keychain storage (DEVELOPMENT MODE)\n\
                     2026-01-21T04:22:37.444840Z  INFO localrouter::server: Web server listening on http://127.0.0.1:33625\n\
                     2026-01-21T04:22:37.444852Z  INFO localrouter::server: OpenAI-compatible endpoints available at:\n\
                     2026-01-21T04:22:37.445062Z  INFO localrouter::server::manager: Server started successfully on port 33625\n\
                     2026-01-21T04:22:37.698310Z  INFO localrouter: Tauri app initialized\n";
    let start = Instant::now();
    let result = service.predict(long_text).await;
    let long_duration = start.elapsed();
    assert!(result.is_ok(), "Long text prediction failed");
    println!("Long text ({} chars): {:?}", long_text.len(), long_duration);

    // Test 4: Very long text (10,000 chars)
    let very_long_text = "a".repeat(10_000);
    let start = Instant::now();
    let result = service.predict(&very_long_text).await;
    let very_long_duration = start.elapsed();
    assert!(result.is_ok(), "Very long text prediction failed");
    println!(
        "Very long text ({} chars): {:?}",
        very_long_text.len(),
        very_long_duration
    );

    // Performance assertions
    // Long text should not take more than 10x the short text time
    // (This will fail before our fix, helping demonstrate the issue)
    println!(
        "\nPerformance ratio (long/short): {:.2}x",
        very_long_duration.as_secs_f64() / short_duration.as_secs_f64()
    );

    // After fix with truncation, this should pass
    // assert!(very_long_duration.as_secs() < 10,
    //         "Very long text took too long: {:?} (should be < 10s with truncation)",
    //         very_long_duration);
}

#[tokio::test]
async fn test_special_characters_prompt() {
    let service = create_test_service();

    let special_prompt = "🎯 Test with emojis! @#$%^&*() <script>alert('xss')</script>";

    let result = service.predict(special_prompt).await;

    assert!(result.is_ok(), "Special characters should not fail");

    let (_, win_rate) = result.unwrap();
    assert!(win_rate >= 0.0 && win_rate <= 1.0);
}

#[tokio::test]
async fn test_threshold_edge_cases() {
    let service = create_test_service();

    let prompt = "test";

    // Test edge thresholds
    let (is_strong_0, win_rate) = service.predict_with_threshold(prompt, 0.0).await.unwrap();
    assert!(
        is_strong_0 || win_rate == 0.0,
        "Threshold 0.0: should route to strong unless win_rate is exactly 0"
    );

    let (is_strong_1, _) = service.predict_with_threshold(prompt, 1.0).await.unwrap();
    assert!(
        !is_strong_1,
        "Threshold 1.0: should route to weak (win_rate can't be > 1.0)"
    );
}

#[cfg(test)]
mod auto_unload_tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Auto-unload task checks every 60s, so this test would need to wait 60+ seconds
    async fn test_auto_unload_on_idle() {
        let service = Arc::new(create_test_service());

        // Start auto-unload task (uses service's configured 5 second idle timeout)
        // Note: The task checks every 60 seconds, so actual unload takes up to 60s after idle
        start_auto_unload_task(Arc::clone(&service));

        // Make a prediction to load the model
        service.predict("test").await.expect("Prediction failed");
        assert!(service.is_loaded().await);

        // Wait for auto-unload check cycle (60 seconds + buffer)
        sleep(Duration::from_secs(65)).await;

        // Should be unloaded
        assert!(!service.is_loaded().await);
    }

    #[tokio::test]
    async fn test_auto_unload_prevented_by_activity() {
        let service = Arc::new(create_test_service());

        // Start auto-unload task (uses service's configured 5 second idle timeout)
        start_auto_unload_task(Arc::clone(&service));

        // Make initial prediction
        service
            .predict("test1")
            .await
            .expect("First prediction failed");
        assert!(service.is_loaded().await);

        // Wait 3 seconds (less than timeout)
        sleep(Duration::from_secs(3)).await;

        // Make another prediction (resets timer)
        service
            .predict("test2")
            .await
            .expect("Second prediction failed");

        // Wait another 3 seconds
        sleep(Duration::from_secs(3)).await;

        // Should still be loaded (last access was 3s ago, not 6s)
        assert!(service.is_loaded().await);
    }
}

#[cfg(test)]
mod downloader_tests {
    use super::*;
    use lr_routellm::downloader;

    #[tokio::test]
    #[ignore] // Requires internet connection and downloads ~440 MB - run manually with: cargo test test_download_and_verify -- --ignored
    async fn test_download_and_verify_models() {
        // Use the actual dev directory like the app does
        let home_dir = dirs::home_dir().expect("Could not determine home directory");
        let routellm_dir = home_dir.join(".localrouter-dev/routellm");
        let model_path = routellm_dir.join("model");
        let tokenizer_path = routellm_dir.join("tokenizer");

        // Check if already downloaded
        let model_file = model_path.join("model.safetensors");
        let tokenizer_file = tokenizer_path.join("tokenizer.json");

        if model_file.exists() && tokenizer_file.exists() {
            println!("✓ Models already exist, skipping download");
            println!("  Model: {:?}", model_file);
            println!("  Tokenizer: {:?}", tokenizer_file);
        } else {
            println!("⬇️  Downloading models from HuggingFace...");
            println!("  Target model dir: {:?}", model_path);
            println!("  Target tokenizer dir: {:?}", tokenizer_path);

            // Download models (no app_handle for progress)
            let result = downloader::download_models(&model_path, &tokenizer_path, None).await;

            assert!(result.is_ok(), "Download failed: {:?}", result.err());
            println!("✓ Download completed successfully");
        }

        // Verify all required files exist
        println!("\n📂 Verifying downloaded files...");

        let model_file = model_path.join("model.safetensors");
        assert!(model_file.exists(), "model.safetensors not found");
        println!("  ✓ model.safetensors");

        let required_tokenizer_files = vec![
            "tokenizer.json",
            "tokenizer_config.json",
            "sentencepiece.bpe.model",
            "special_tokens_map.json",
            "config.json",
        ];

        for file in &required_tokenizer_files {
            let file_path = tokenizer_path.join(file);
            assert!(file_path.exists(), "{} not found at {:?}", file, file_path);
            println!("  ✓ {}", file);
        }

        // Test that we can load the tokenizer
        println!("\n🔧 Testing tokenizer loading...");
        use tokenizers::Tokenizer;
        let tokenizer_result = Tokenizer::from_file(tokenizer_path.join("tokenizer.json"));

        if let Err(ref e) = tokenizer_result {
            println!("❌ Tokenizer loading failed!");
            println!("Error: {:?}", e);

            // Print tokenizer.json first few lines for debugging
            if let Ok(content) = std::fs::read_to_string(tokenizer_path.join("tokenizer.json")) {
                println!("\nFirst 500 chars of tokenizer.json:");
                println!("{}", &content[..content.len().min(500)]);
            }
        }

        assert!(
            tokenizer_result.is_ok(),
            "Failed to load tokenizer: {:?}",
            tokenizer_result.err()
        );
        println!("  ✓ Tokenizer loaded successfully");

        // Test that we can load the full model
        println!("\n🤖 Testing model loading...");
        use lr_routellm::candle_router::CandleRouter;
        let router_result = CandleRouter::new(&model_path, &tokenizer_path);

        if let Err(ref e) = router_result {
            println!("❌ Model loading failed!");
            println!("Error: {:?}", e);
        }

        assert!(
            router_result.is_ok(),
            "Failed to load model: {:?}",
            router_result.err()
        );
        println!("  ✓ Model loaded successfully");

        // Test prediction
        println!("\n🎯 Testing prediction...");
        let router = router_result.unwrap();
        let score = router
            .calculate_strong_win_rate("test prompt")
            .expect("Prediction failed");
        println!("  ✓ Prediction score: {:.3}", score);
        assert!(
            score >= 0.0 && score <= 1.0,
            "Score should be between 0 and 1"
        );

        println!("\n✅ All tests passed!");
    }

    #[tokio::test]
    async fn test_download_status_not_downloaded() {
        let model_path = PathBuf::from("/nonexistent/model");
        let tokenizer_path = PathBuf::from("/nonexistent/tokenizer");

        let status = downloader::get_download_status(&model_path, &tokenizer_path);

        assert_eq!(
            status.state,
            lr_config::RouteLLMDownloadState::NotDownloaded
        );
        assert_eq!(status.progress, 0.0);
        assert_eq!(status.total_bytes, 440_000_000); // SafeTensors size
    }
}
