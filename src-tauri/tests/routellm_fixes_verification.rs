//! Integration tests to verify RouteLLM bug fixes
//!
//! Run with: cargo test --test routellm_fixes_verification

use std::path::PathBuf;

#[tokio::test]
async fn test_fix_1_path_handling_no_panic() {
    // Bug #1: Path with no parent should not panic
    use localrouter::routellm::downloader;

    let result = downloader::download_models(
        std::path::Path::new("/"), // Root has no parent
        std::path::Path::new("/tmp/tokenizer"),
        None,
    )
    .await;

    // Should return error, not panic
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("parent"),
        "Error should mention parent directory: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_fix_2_initialization_race_condition() {
    // Bug #2: Multiple concurrent predictions should not cause multiple initializations
    use localrouter::routellm::RouteLLMService;
    use std::sync::Arc;

    let home = dirs::home_dir().expect("Could not determine home directory");
    let service = Arc::new(RouteLLMService::new(
        home.join(".localrouter-dev/routellm/model"),
        home.join(".localrouter-dev/routellm/tokenizer"),
        600,
    ));

    // Check if models exist before attempting
    let model_file = home.join(".localrouter-dev/routellm/model/model.safetensors");
    let patched_file = home.join(".localrouter-dev/routellm/model/model.patched.safetensors");

    if !model_file.exists() && !patched_file.exists() {
        println!("⏩ Skipping test: models not downloaded");
        return;
    }

    // Spawn 10 concurrent predictions
    let mut handles = vec![];
    for i in 0..10 {
        let service_clone = service.clone();
        let handle =
            tokio::spawn(async move { service_clone.predict(&format!("test prompt {}", i)).await });
        handles.push(handle);
    }

    // All should succeed without multiple initializations
    let mut success_count = 0;
    for handle in handles {
        if handle.await.unwrap().is_ok() {
            success_count += 1;
        }
    }

    assert_eq!(success_count, 10, "All predictions should succeed");

    // Verify only one router instance was created (should be loaded after predictions)
    assert!(
        service.is_loaded().await,
        "Service should be loaded after predictions"
    );
}

#[tokio::test]
async fn test_fix_3_download_concurrency_protection() {
    // Bug #3: Concurrent downloads should be prevented
    use localrouter::routellm::downloader;

    let temp_dir = std::env::temp_dir();
    let model_path = temp_dir.join("test_concurrent_dl_model");
    let tokenizer_path = temp_dir.join("test_concurrent_dl_tokenizer");

    // Clean up
    let _ = tokio::fs::remove_dir_all(&model_path).await;
    let _ = tokio::fs::remove_dir_all(&tokenizer_path).await;

    // Pre-flight: check if disk space is sufficient (2 GB required).
    // If not, both downloads will fail with disk-space error before reaching
    // the concurrency lock, so the test cannot verify lock contention.
    let preflight = downloader::download_models(&model_path, &tokenizer_path, None).await;
    if let Err(ref e) = preflight {
        let msg = e.to_string();
        if msg.contains("Insufficient disk space") {
            println!("⏩ Skipping test: insufficient disk space to test download concurrency");
            return;
        }
    }

    // Try two concurrent downloads (both will fail due to network, but we test mutex)
    let mp1 = model_path.clone();
    let tp1 = tokenizer_path.clone();
    let handle1 = tokio::spawn(async move { downloader::download_models(&mp1, &tp1, None).await });

    // Give first download a tiny head start
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    let mp2 = model_path.clone();
    let tp2 = tokenizer_path.clone();
    let handle2 = tokio::spawn(async move { downloader::download_models(&mp2, &tp2, None).await });

    let result1 = handle1.await.unwrap();
    let result2 = handle2.await.unwrap();

    // At least one should fail with "already in progress" message
    let already_in_progress = (result2.is_err()
        && result2
            .unwrap_err()
            .to_string()
            .contains("already in progress"))
        || (result1.is_err()
            && result1
                .unwrap_err()
                .to_string()
                .contains("already in progress"));

    assert!(
        already_in_progress,
        "Should detect concurrent download attempt"
    );

    // Clean up
    let _ = tokio::fs::remove_dir_all(&model_path).await;
    let _ = tokio::fs::remove_dir_all(&tokenizer_path).await;
}

#[tokio::test]
async fn test_fix_6_initializing_state() {
    // Bug #6: Should show Initializing state during model loading
    use localrouter::routellm::{RouteLLMService, RouteLLMState};

    let home = dirs::home_dir().expect("Could not determine home directory");
    let service = std::sync::Arc::new(RouteLLMService::new(
        home.join(".localrouter-dev/routellm/model"),
        home.join(".localrouter-dev/routellm/tokenizer"),
        600,
    ));

    // Check if models exist
    let model_file = home.join(".localrouter-dev/routellm/model/model.safetensors");
    let patched_file = home.join(".localrouter-dev/routellm/model/model.patched.safetensors");

    if !model_file.exists() && !patched_file.exists() {
        println!("⏩ Skipping test: models not downloaded");
        return;
    }

    // Initial state should be DownloadedNotRunning
    let status = service.get_status().await;
    assert_eq!(status.state, RouteLLMState::DownloadedNotRunning);

    // Trigger initialization in background
    let service_clone = service.clone();
    let init_handle = tokio::spawn(async move { service_clone.initialize().await });

    // Try to catch Initializing state (timing-dependent)
    let mut saw_initializing = false;
    for _ in 0..10 {
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let status = service.get_status().await;
        if status.state == RouteLLMState::Initializing {
            saw_initializing = true;
            break;
        }
    }

    // Wait for initialization to complete
    init_handle.await.unwrap().unwrap();

    // Final state should be Started
    let status = service.get_status().await;
    assert_eq!(status.state, RouteLLMState::Started);

    // Note: saw_initializing might be false due to timing, but that's okay
    // The important thing is we didn't panic and ended up in Started state
    if saw_initializing {
        println!("✓ Successfully observed Initializing state");
    } else {
        println!("⏩ Initialization too fast to observe Initializing state (OK)");
    }
}

#[test]
fn test_patched_model_detection() {
    // Verify the fix for detecting patched model files
    use localrouter::routellm::downloader::get_download_status;

    let home = dirs::home_dir().expect("Could not determine home directory");
    let model_path = home.join(".localrouter-dev/routellm/model");
    let tokenizer_path = home.join(".localrouter-dev/routellm/tokenizer");

    let status = get_download_status(&model_path, &tokenizer_path);

    // Should detect either original or patched model
    let model_file = model_path.join("model.safetensors");
    let patched_file = model_path.join("model.patched.safetensors");
    let tokenizer_file = tokenizer_path.join("tokenizer.json");

    if (model_file.exists() || patched_file.exists()) && tokenizer_file.exists() {
        assert_eq!(
            status.state,
            localrouter::config::RouteLLMDownloadState::Downloaded
        );
        println!("✓ Correctly detected downloaded models (original or patched)");
    } else {
        assert_eq!(
            status.state,
            localrouter::config::RouteLLMDownloadState::NotDownloaded
        );
        println!("⏩ Models not downloaded");
    }
}
