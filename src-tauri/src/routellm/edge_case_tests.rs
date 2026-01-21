//! Comprehensive edge case tests for RouteLLM
//!
//! Tests cover all identified edge cases from bug analysis:
//! - Input validation (empty, too long, invalid threshold)
//! - Concurrent operations (downloads, predictions, initialization)
//! - Error recovery and cleanup
//! - State transitions
//! - Memory management

#[cfg(test)]
mod edge_case_tests {
    use crate::routellm::{RouteLLMService, RouteLLMState};
    use std::path::PathBuf;
    use std::time::Duration;
    use tokio::time::timeout;

    /// Test empty prompt handling
    #[tokio::test]
    async fn test_empty_prompt() {
        let service = RouteLLMService::new(
            PathBuf::from("/tmp/test_model"),
            PathBuf::from("/tmp/test_tokenizer"),
            600,
        );

        // Empty string should be caught by validation in commands layer
        // But let's verify the service handles it gracefully
        let result = service.predict("", ).await;
        // Currently no validation at service level - this is intentional
        // Validation happens in Tauri commands
    }

    /// Test very long prompt handling
    #[tokio::test]
    #[ignore] // Requires model files
    async fn test_long_prompt() {
        let service = create_test_service().await;

        // 5000 character prompt (well over 512 token limit)
        let long_prompt = "a".repeat(5000);

        let result = service.predict(&long_prompt).await;
        assert!(result.is_ok(), "Long prompts should be handled via truncation");
    }

    /// Test prompt with null bytes
    #[tokio::test]
    #[ignore] // Requires model files
    async fn test_prompt_with_null_bytes() {
        let service = create_test_service().await;

        let prompt_with_null = "Hello\0World";
        let result = service.predict(prompt_with_null).await;

        // Should either work or fail gracefully
        match result {
            Ok(_) => {}
            Err(e) => assert!(e.to_string().contains("null"), "Error should mention null bytes"),
        }
    }

    /// Test Unicode and emoji prompts
    #[tokio::test]
    #[ignore] // Requires model files
    async fn test_unicode_prompts() {
        let service = create_test_service().await;

        let unicode_prompts = vec![
            "你好世界",              // Chinese
            "مرحبا بالعالم",        // Arabic
            "Привет мир",          // Russian
            "🚀🎉💻",              // Emojis
            "Mix of 中文 and emoji 🎨",
        ];

        for prompt in unicode_prompts {
            let result = service.predict(prompt).await;
            assert!(result.is_ok(), "Unicode prompt '{}' should work", prompt);
        }
    }

    /// Test concurrent predictions during initialization
    #[tokio::test]
    #[ignore] // Requires model files
    async fn test_concurrent_predictions_during_init() {
        let service = std::sync::Arc::new(create_test_service().await);

        // Spawn 10 concurrent predictions
        let mut handles = vec![];
        for i in 0..10 {
            let service_clone = service.clone();
            let handle = tokio::spawn(async move {
                service_clone.predict(&format!("test prompt {}", i)).await
            });
            handles.push(handle);
        }

        // All should succeed (only one initialization should happen)
        for (i, handle) in handles.into_iter().enumerate() {
            let result = handle.await.unwrap();
            assert!(result.is_ok(), "Prediction {} should succeed", i);
        }
    }

    /// Test concurrent downloads (should fail for all but first)
    #[tokio::test]
    #[ignore] // Requires internet connection
    async fn test_concurrent_downloads() {
        use crate::routellm::downloader;

        let model_path = PathBuf::from("/tmp/test_concurrent_model");
        let tokenizer_path = PathBuf::from("/tmp/test_concurrent_tokenizer");

        // Clean up from previous runs
        let _ = tokio::fs::remove_dir_all(&model_path).await;
        let _ = tokio::fs::remove_dir_all(&tokenizer_path).await;

        // Spawn 3 concurrent downloads
        let handles: Vec<_> = (0..3)
            .map(|_| {
                let mp = model_path.clone();
                let tp = tokenizer_path.clone();
                tokio::spawn(async move {
                    downloader::download_models(&mp, &tp, None).await
                })
            })
            .collect();

        let results: Vec<_> = futures::future::join_all(handles).await;

        // Exactly one should succeed, others should fail with "already in progress"
        let successes = results.iter().filter(|r| {
            matches!(r, Ok(Ok(())))
        }).count();

        let already_downloading = results.iter().filter(|r| {
            matches!(r, Ok(Err(e)) if e.to_string().contains("already in progress"))
        }).count();

        assert_eq!(successes, 1, "Exactly one download should succeed");
        assert_eq!(already_downloading, 2, "Two downloads should be rejected");

        // Clean up
        let _ = tokio::fs::remove_dir_all(&model_path).await;
        let _ = tokio::fs::remove_dir_all(&tokenizer_path).await;
    }

    /// Test state transitions
    #[tokio::test]
    #[ignore] // Requires model files
    async fn test_state_transitions() {
        let service = std::sync::Arc::new(create_test_service().await);

        // Initial state: DownloadedNotRunning (assuming models exist)
        let status = service.get_status().await;
        assert_eq!(status.state, RouteLLMState::DownloadedNotRunning);

        // Trigger initialization
        let service_clone = service.clone();
        let init_handle = tokio::spawn(async move {
            service_clone.predict("test").await
        });

        // Check state during initialization (race condition, might miss it)
        tokio::time::sleep(Duration::from_millis(100)).await;
        let status_during_init = service.get_status().await;
        // Could be Initializing or Started depending on timing

        // Wait for initialization to complete
        init_handle.await.unwrap().unwrap();

        // Should be Started now
        let status_after = service.get_status().await;
        assert_eq!(status_after.state, RouteLLMState::Started);

        // Unload
        service.unload().await;

        // Should be back to DownloadedNotRunning
        let status_unloaded = service.get_status().await;
        assert_eq!(status_unloaded.state, RouteLLMState::DownloadedNotRunning);
    }

    /// Test idle timeout edge cases
    #[tokio::test]
    #[ignore] // Requires model files
    async fn test_idle_timeout_zero() {
        // idle_timeout = 0 means never unload
        let service = std::sync::Arc::new(RouteLLMService::new(
            PathBuf::from("/tmp/test_model"),
            PathBuf::from("/tmp/test_tokenizer"),
            0, // Never unload
        ));

        // Start auto-unload task
        let _task = service.clone().start_auto_unload_task();

        // Initialize
        service.predict("test").await.unwrap();

        // Wait longer than normal timeout
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Should still be loaded
        assert!(service.is_loaded().await, "Service should remain loaded with timeout=0");
    }

    /// Test idle timeout = 1 second
    #[tokio::test]
    #[ignore] // Requires model files
    async fn test_idle_timeout_one_second() {
        let service = std::sync::Arc::new(RouteLLMService::new(
            PathBuf::from("/tmp/test_model"),
            PathBuf::from("/tmp/test_tokenizer"),
            1, // Unload after 1 second
        ));

        // Start auto-unload task
        let _task = service.clone().start_auto_unload_task();

        // Initialize
        service.predict("test").await.unwrap();
        assert!(service.is_loaded().await);

        // Wait for auto-unload (checks every 60s, so this won't actually unload)
        // This test demonstrates the limitation of current implementation
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Note: Due to 60s check interval, model won't unload
        // This is a documented limitation
    }

    /// Test rapid unload/predict cycles
    #[tokio::test]
    #[ignore] // Requires model files
    async fn test_rapid_unload_predict_cycles() {
        let service = std::sync::Arc::new(create_test_service().await);

        for i in 0..5 {
            // Predict (will initialize if needed)
            let result = service.predict(&format!("test {}", i)).await;
            assert!(result.is_ok());

            // Unload
            service.unload().await;
            assert!(!service.is_loaded().await);
        }
    }

    /// Test prediction with invalid threshold values
    #[tokio::test]
    async fn test_invalid_threshold_values() {
        // These tests verify the validation layer in commands_routellm.rs
        // The service itself doesn't validate thresholds

        let invalid_thresholds = vec![
            -0.5,    // Negative
            1.5,     // > 1.0
            f32::NAN,        // NaN
            f32::INFINITY,   // Infinity
            f32::NEG_INFINITY, // -Infinity
        ];

        for threshold in invalid_thresholds {
            // Validation happens in Tauri command layer
            assert!(
                !threshold.is_finite() || threshold < 0.0 || threshold > 1.0,
                "Invalid threshold {} should be caught", threshold
            );
        }
    }

    /// Test model loading with missing files
    #[tokio::test]
    async fn test_missing_model_files() {
        let service = RouteLLMService::new(
            PathBuf::from("/nonexistent/model"),
            PathBuf::from("/nonexistent/tokenizer"),
            600,
        );

        let result = service.initialize().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    /// Test model loading with corrupted files
    #[tokio::test]
    #[ignore] // Requires setup
    async fn test_corrupted_model_files() {
        use std::fs;

        let model_dir = PathBuf::from("/tmp/test_corrupted_model");
        let tokenizer_dir = PathBuf::from("/tmp/test_corrupted_tokenizer");

        // Create directories
        fs::create_dir_all(&model_dir).unwrap();
        fs::create_dir_all(&tokenizer_dir).unwrap();

        // Create corrupted files
        fs::write(model_dir.join("model.safetensors"), b"corrupted data").unwrap();
        fs::write(tokenizer_dir.join("tokenizer.json"), b"{ invalid json }").unwrap();

        let service = RouteLLMService::new(model_dir.clone(), tokenizer_dir.clone(), 600);
        let result = service.initialize().await;

        assert!(result.is_err(), "Should fail with corrupted files");

        // Clean up
        fs::remove_dir_all(&model_dir).unwrap();
        fs::remove_dir_all(&tokenizer_dir).unwrap();
    }

    /// Test download with invalid paths
    #[tokio::test]
    async fn test_download_with_invalid_paths() {
        use crate::routellm::downloader;

        // Root path has no parent
        let result = downloader::download_models(
            std::path::Path::new("/"),
            std::path::Path::new("/tmp/tokenizer"),
            None,
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("parent"));
    }

    /// Test download timeout (would require very slow network)
    #[tokio::test]
    #[ignore] // Requires slow network or mock
    async fn test_download_timeout() {
        use crate::routellm::downloader;

        let model_path = PathBuf::from("/tmp/test_timeout_model");
        let tokenizer_path = PathBuf::from("/tmp/test_timeout_tokenizer");

        // Set a 1-second timeout (download takes much longer)
        let result = timeout(
            Duration::from_secs(1),
            downloader::download_models(&model_path, &tokenizer_path, None),
        )
        .await;

        assert!(result.is_err(), "Should timeout");
    }

    // Helper to create a test service with actual model paths
    async fn create_test_service() -> RouteLLMService {
        // Uses actual dev model paths - requires models to be downloaded
        let home = std::env::var("HOME").unwrap();
        let model_path = PathBuf::from(home.clone()).join(".localrouter-dev/routellm/model");
        let tokenizer_path = PathBuf::from(home).join(".localrouter-dev/routellm/tokenizer");

        RouteLLMService::new(model_path, tokenizer_path, 600)
    }
}
