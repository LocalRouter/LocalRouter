//! Integration tests for configuration and API key management
//!
//! These tests verify:
//! - Configuration file loading, saving, and reloading
//! - API key creation, storage (mocked keychain + metadata), and retrieval
//! - API key deletion (from both keychain and config)
//! - Configuration updates and persistence
//!
//! Tests use temporary directories and mock keychain for complete isolation.
//! No system keychain or cleanup required!

use localrouter_ai::api_keys::{ApiKeyManager, KeychainStorage, MockKeychain, SystemKeychain};
use localrouter_ai::config::{AppConfig, ConfigManager, LogLevel, ModelSelection};
use serial_test::serial;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

// ============================================================================
// Test Utilities
// ============================================================================

fn create_temp_config_dir() -> TempDir {
    TempDir::new().expect("Failed to create temporary directory")
}

fn test_config_path(temp_dir: &TempDir) -> PathBuf {
    temp_dir.path().join("config.yaml")
}

// ============================================================================
// Configuration Tests
// ============================================================================

#[tokio::test]
async fn test_config_creation_and_defaults() {
    let config = AppConfig::default();

    assert_eq!(config.version, 1);
    assert_eq!(config.server.host, "127.0.0.1");
    assert_eq!(config.server.port, 3625);
    assert!(config.server.enable_cors);
    assert_eq!(config.api_keys.len(), 0);
    assert_eq!(config.routers.len(), 2);
    assert_eq!(config.providers.len(), 1);
    assert_eq!(config.logging.level, LogLevel::Info);
}

#[tokio::test]
async fn test_config_save_and_load() {
    let temp_dir = create_temp_config_dir();
    let config_path = test_config_path(&temp_dir);

    let mut config = AppConfig::default();
    config.server.port = 9999;
    config.logging.level = LogLevel::Debug;

    localrouter_ai::config::save_config(&config, &config_path)
        .await
        .expect("Failed to save config");

    assert!(config_path.exists());

    let loaded_config = localrouter_ai::config::load_config(&config_path)
        .await
        .expect("Failed to load config");

    assert_eq!(loaded_config.server.port, 9999);
    assert_eq!(loaded_config.logging.level, LogLevel::Debug);
}

#[tokio::test]
async fn test_config_manager_update_and_save() {
    let temp_dir = create_temp_config_dir();
    let config_path = test_config_path(&temp_dir);

    let config = AppConfig::default();
    localrouter_ai::config::save_config(&config, &config_path)
        .await
        .expect("Failed to save initial config");

    let manager = ConfigManager::load_from_path(config_path.clone())
        .await
        .expect("Failed to load config manager");

    manager
        .update(|cfg| {
            cfg.server.port = 8888;
            cfg.logging.level = LogLevel::Warn;
        })
        .expect("Failed to update config");

    manager.save().await.expect("Failed to save config");

    let reloaded = localrouter_ai::config::load_config(&config_path)
        .await
        .expect("Failed to reload config");

    assert_eq!(reloaded.server.port, 8888);
    assert_eq!(reloaded.logging.level, LogLevel::Warn);
}

#[tokio::test]
async fn test_config_reload() {
    let temp_dir = create_temp_config_dir();
    let config_path = test_config_path(&temp_dir);

    let config = AppConfig::default();
    localrouter_ai::config::save_config(&config, &config_path)
        .await
        .expect("Failed to save initial config");

    let manager = ConfigManager::load_from_path(config_path.clone())
        .await
        .expect("Failed to load config manager");

    let mut modified_config = config;
    modified_config.server.port = 5555;
    localrouter_ai::config::save_config(&modified_config, &config_path)
        .await
        .expect("Failed to save modified config");

    manager.reload().await.expect("Failed to reload config");

    let reloaded = manager.get();
    assert_eq!(reloaded.server.port, 5555);
}

// ============================================================================
// API Key Tests (with Mock Keychain)
// ============================================================================

#[tokio::test]
async fn test_api_key_creation_with_mock() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = ApiKeyManager::with_keychain(vec![], mock_keychain.clone());

    let result = manager
        .create_key(Some("test-app".to_string()))
        .await;

    assert!(result.is_ok());
    let (key, config) = result.unwrap();

    // Verify key format
    assert!(key.starts_with("lr-"));
    assert!(key.len() > 10);

    // Verify config
    assert_eq!(config.name, "test-app");
    assert!(config.enabled);

    // Verify key is in mock keychain
    let stored_key = mock_keychain
        .get("LocalRouter-APIKeys", &config.id)
        .expect("Failed to get from keychain")
        .expect("Key not found in mock keychain");
    assert_eq!(stored_key, key);
}

#[tokio::test]
async fn test_api_key_auto_naming() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = ApiKeyManager::with_keychain(vec![], mock_keychain);

    let (_, config1) = manager
        .create_key(None)
        .await
        .expect("Failed to create first key");

    assert_eq!(config1.name, "my-app-1");

    let (_, config2) = manager
        .create_key(None)
        .await
        .expect("Failed to create second key");

    assert_eq!(config2.name, "my-app-2");
}

#[tokio::test]
async fn test_api_key_list() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = ApiKeyManager::with_keychain(vec![], mock_keychain);

    for i in 1..=3 {
        manager
            .create_key(Some(format!("app-{}", i)))
            .await
            .expect("Failed to create key");
    }

    let keys = manager.list_keys();
    assert_eq!(keys.len(), 3);
    assert_eq!(keys[0].name, "app-1");
    assert_eq!(keys[1].name, "app-2");
    assert_eq!(keys[2].name, "app-3");
}

#[tokio::test]
async fn test_api_key_get_value() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = ApiKeyManager::with_keychain(vec![], mock_keychain);

    let (original_key, config) = manager
        .create_key(Some("test-key".to_string()))
        .await
        .expect("Failed to create key");

    let retrieved_key = manager
        .get_key_value(&config.id)
        .expect("Failed to get key value")
        .expect("Key not found");

    assert_eq!(retrieved_key, original_key);

    let nonexistent = manager
        .get_key_value("nonexistent-id")
        .expect("Failed to query");
    assert!(nonexistent.is_none());
}

#[tokio::test]
async fn test_api_key_verify() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = ApiKeyManager::with_keychain(vec![], mock_keychain);

    let (key, config) = manager
        .create_key(Some("verify-test".to_string()))
        .await
        .expect("Failed to create key");

    let verified = manager.verify_key(&key);
    assert!(verified.is_some());
    assert_eq!(verified.unwrap().id, config.id);

    let wrong_verified = manager.verify_key("lr-wrongkey123");
    assert!(wrong_verified.is_none());
}

#[tokio::test]
async fn test_api_key_delete() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = ApiKeyManager::with_keychain(vec![], mock_keychain.clone());

    let (_, config) = manager
        .create_key(Some("delete-test".to_string()))
        .await
        .expect("Failed to create key");

    let key_id = config.id.clone();

    // Verify key exists in mock keychain
    assert!(mock_keychain
        .get("LocalRouter-APIKeys", &key_id)
        .unwrap()
        .is_some());

    manager.delete_key(&key_id).expect("Failed to delete key");

    assert!(manager.get_key(&key_id).is_none());

    // Verify key is gone from mock keychain
    assert!(mock_keychain
        .get("LocalRouter-APIKeys", &key_id)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn test_api_key_disabled() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = ApiKeyManager::with_keychain(vec![], mock_keychain);

    let (key, config) = manager
        .create_key(Some("disabled-test".to_string()))
        .await
        .expect("Failed to create key");

    let updated = manager
        .update_key(&config.id, |cfg| {
            cfg.enabled = false;
        })
        .expect("Failed to update key");

    assert!(!updated.enabled);

    // Verify disabled key can't be verified
    let verified = manager.verify_key(&key);
    assert!(verified.is_none());
}

#[tokio::test]
async fn test_api_key_update_model_selection() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = ApiKeyManager::with_keychain(vec![], mock_keychain);

    // Create a key without model selection
    let (_, config) = manager
        .create_key(Some("model-update-test".to_string()))
        .await
        .expect("Failed to create key");

    // Verify no model selection initially
    assert!(config.model_selection.is_none());

    // Update to add DirectModel selection
    let updated = manager
        .update_key(&config.id, |cfg| {
            cfg.model_selection = Some(ModelSelection::DirectModel {
                provider: "Ollama".to_string(),
                model: "llama2".to_string(),
            });
        })
        .expect("Failed to update model selection");

    assert!(updated.model_selection.is_some());
    match updated.model_selection.as_ref().unwrap() {
        ModelSelection::DirectModel { provider, model } => {
            assert_eq!(provider, "Ollama");
            assert_eq!(model, "llama2");
        }
        _ => panic!("Expected DirectModel selection"),
    }

    // Update to Router selection
    let updated2 = manager
        .update_key(&config.id, |cfg| {
            cfg.model_selection = Some(ModelSelection::Router {
                router_name: "Minimum Cost".to_string(),
            });
        })
        .expect("Failed to update to router selection");

    assert!(updated2.model_selection.is_some());
    match updated2.model_selection.as_ref().unwrap() {
        ModelSelection::Router { router_name } => {
            assert_eq!(router_name, "Minimum Cost");
        }
        _ => panic!("Expected Router selection"),
    }

    // Clear model selection
    let updated3 = manager
        .update_key(&config.id, |cfg| {
            cfg.model_selection = None;
        })
        .expect("Failed to clear model selection");

    assert!(updated3.model_selection.is_none());
}

// ============================================================================
// Configuration + API Key Integration Tests
// ============================================================================

#[tokio::test]
async fn test_config_with_api_keys_persistence() {
    let temp_dir = create_temp_config_dir();
    let config_path = test_config_path(&temp_dir);
    let mock_keychain = Arc::new(MockKeychain::new());

    let config = AppConfig::default();
    localrouter_ai::config::save_config(&config, &config_path)
        .await
        .expect("Failed to save initial config");

    let config_manager = ConfigManager::load_from_path(config_path.clone())
        .await
        .expect("Failed to load config manager");

    let key_manager = ApiKeyManager::with_keychain(vec![], mock_keychain.clone());

    let (key, key_config) = key_manager
        .create_key(Some("integration-test".to_string()))
        .await
        .expect("Failed to create key");

    config_manager
        .update(|cfg| {
            cfg.api_keys.push(key_config.clone());
        })
        .expect("Failed to update config");

    config_manager
        .save()
        .await
        .expect("Failed to save config");

    let reloaded = localrouter_ai::config::load_config(&config_path)
        .await
        .expect("Failed to reload config");

    assert_eq!(reloaded.api_keys.len(), 1);
    assert_eq!(reloaded.api_keys[0].name, "integration-test");
    assert_eq!(reloaded.api_keys[0].id, key_config.id);

    // Verify actual key is still in mock keychain
    let key_manager2 = ApiKeyManager::with_keychain(reloaded.api_keys, mock_keychain);
    let retrieved_key = key_manager2
        .get_key_value(&key_config.id)
        .expect("Failed to get key")
        .expect("Key not found");
    assert_eq!(retrieved_key, key);
}

#[tokio::test]
async fn test_multiple_api_keys_with_config() {
    let temp_dir = create_temp_config_dir();
    let config_path = test_config_path(&temp_dir);
    let mock_keychain = Arc::new(MockKeychain::new());

    let config = AppConfig::default();
    localrouter_ai::config::save_config(&config, &config_path)
        .await
        .expect("Failed to save config");

    let config_manager = ConfigManager::load_from_path(config_path.clone())
        .await
        .expect("Failed to load config manager");

    let key_manager = ApiKeyManager::with_keychain(vec![], mock_keychain.clone());

    for i in 1..=3 {
        let (_, key_config) = key_manager
            .create_key(Some(format!("multi-test-{}", i)))
            .await
            .expect("Failed to create key");

        config_manager
            .update(|cfg| {
                cfg.api_keys.push(key_config.clone());
            })
            .expect("Failed to update config");
    }

    config_manager
        .save()
        .await
        .expect("Failed to save config");

    let reloaded = localrouter_ai::config::load_config(&config_path)
        .await
        .expect("Failed to reload config");

    assert_eq!(reloaded.api_keys.len(), 3);
    assert_eq!(reloaded.api_keys[0].name, "multi-test-1");
    assert_eq!(reloaded.api_keys[1].name, "multi-test-2");
    assert_eq!(reloaded.api_keys[2].name, "multi-test-3");

    // Verify all keys are in mock keychain
    let key_manager2 = ApiKeyManager::with_keychain(vec![], mock_keychain);
    for key_config in &reloaded.api_keys {
        let key_value = key_manager2
            .get_key_value(&key_config.id)
            .expect("Failed to get key")
            .expect("Key not found");
        assert!(key_value.starts_with("lr-"));
    }
}

#[tokio::test]
async fn test_api_key_deletion_updates_config() {
    let temp_dir = create_temp_config_dir();
    let config_path = test_config_path(&temp_dir);
    let mock_keychain = Arc::new(MockKeychain::new());

    let config = AppConfig::default();
    localrouter_ai::config::save_config(&config, &config_path)
        .await
        .expect("Failed to save config");

    let config_manager = ConfigManager::load_from_path(config_path.clone())
        .await
        .expect("Failed to load config manager");

    let key_manager = ApiKeyManager::with_keychain(vec![], mock_keychain);

    let (_, key1) = key_manager
        .create_key(Some("key1".to_string()))
        .await
        .expect("Failed to create key1");

    let (_, key2) = key_manager
        .create_key(Some("key2".to_string()))
        .await
        .expect("Failed to create key2");

    config_manager
        .update(|cfg| {
            cfg.api_keys.push(key1.clone());
            cfg.api_keys.push(key2.clone());
        })
        .expect("Failed to update config");

    config_manager
        .save()
        .await
        .expect("Failed to save config");

    key_manager
        .delete_key(&key1.id)
        .expect("Failed to delete key");

    config_manager
        .update(|cfg| {
            cfg.api_keys.retain(|k| k.id != key1.id);
        })
        .expect("Failed to update config");

    config_manager
        .save()
        .await
        .expect("Failed to save config");

    let reloaded = localrouter_ai::config::load_config(&config_path)
        .await
        .expect("Failed to reload config");

    assert_eq!(reloaded.api_keys.len(), 1);
    assert_eq!(reloaded.api_keys[0].id, key2.id);
}

#[tokio::test]
async fn test_full_workflow_integration() {
    let temp_dir = create_temp_config_dir();
    let config_path = test_config_path(&temp_dir);
    let mock_keychain = Arc::new(MockKeychain::new());

    // 1. Create initial config
    let config = AppConfig::default();
    localrouter_ai::config::save_config(&config, &config_path)
        .await
        .expect("Failed to save config");

    // 2. Load config manager
    let config_manager = ConfigManager::load_from_path(config_path.clone())
        .await
        .expect("Failed to load config manager");

    // 3. Create API keys
    let key_manager = ApiKeyManager::with_keychain(vec![], mock_keychain.clone());

    let (key1, config1) = key_manager
        .create_key(Some("app1".to_string()))
        .await
        .expect("Failed to create key1");

    let (key2, config2) = key_manager
        .create_key(Some("app2".to_string()))
        .await
        .expect("Failed to create key2");

    // 4. Add API keys to config
    config_manager
        .update(|cfg| {
            cfg.api_keys.push(config1.clone());
            cfg.api_keys.push(config2.clone());
        })
        .expect("Failed to update config");

    // 5. Update server settings
    config_manager
        .update(|cfg| {
            cfg.server.port = 4444;
            cfg.logging.level = LogLevel::Debug;
        })
        .expect("Failed to update server config");

    // 6. Save everything
    config_manager
        .save()
        .await
        .expect("Failed to save config");

    // 7. Reload and verify everything
    let reloaded_config = localrouter_ai::config::load_config(&config_path)
        .await
        .expect("Failed to reload config");

    assert_eq!(reloaded_config.server.port, 4444);
    assert_eq!(reloaded_config.logging.level, LogLevel::Debug);
    assert_eq!(reloaded_config.api_keys.len(), 2);
    assert_eq!(reloaded_config.api_keys[0].name, "app1");
    assert_eq!(reloaded_config.api_keys[1].name, "app2");

    // Verify API keys in mock keychain
    let reloaded_key_manager =
        ApiKeyManager::with_keychain(reloaded_config.api_keys.clone(), mock_keychain);

    let retrieved_key1 = reloaded_key_manager
        .get_key_value(&config1.id)
        .expect("Failed to get key1")
        .expect("Key1 not found");
    assert_eq!(retrieved_key1, key1);

    let retrieved_key2 = reloaded_key_manager
        .get_key_value(&config2.id)
        .expect("Failed to get key2")
        .expect("Key2 not found");
    assert_eq!(retrieved_key2, key2);

    // 8. Test deletion
    reloaded_key_manager
        .delete_key(&config1.id)
        .expect("Failed to delete key1");

    config_manager
        .update(|cfg| {
            cfg.api_keys.retain(|k| k.id != config1.id);
        })
        .expect("Failed to update after delete");

    config_manager
        .save()
        .await
        .expect("Failed to save after delete");

    let final_config = localrouter_ai::config::load_config(&config_path)
        .await
        .expect("Failed to load final config");
    assert_eq!(final_config.api_keys.len(), 1);
    assert_eq!(final_config.api_keys[0].id, config2.id);
}

#[tokio::test]
async fn test_config_isolation() {
    let temp_dir1 = create_temp_config_dir();
    let temp_dir2 = create_temp_config_dir();

    let config_path1 = test_config_path(&temp_dir1);
    let config_path2 = test_config_path(&temp_dir2);

    let mut config1 = AppConfig::default();
    config1.server.port = 1111;

    let mut config2 = AppConfig::default();
    config2.server.port = 2222;

    localrouter_ai::config::save_config(&config1, &config_path1)
        .await
        .expect("Failed to save config1");

    localrouter_ai::config::save_config(&config2, &config_path2)
        .await
        .expect("Failed to save config2");

    let loaded1 = localrouter_ai::config::load_config(&config_path1)
        .await
        .expect("Failed to load config1");

    let loaded2 = localrouter_ai::config::load_config(&config_path2)
        .await
        .expect("Failed to load config2");

    assert_eq!(loaded1.server.port, 1111);
    assert_eq!(loaded2.server.port, 2222);
}

// ============================================================================
// Real Keychain Tests
// ============================================================================
//
// NOTE: These tests use the actual system keychain and may require user interaction:
// - macOS: Touch ID or password prompt
// - Windows: Credential Manager access
// - Linux: Secret Service D-Bus
//
// If these tests fail in CI/automated environments, that's expected.
// They're designed for local manual verification of keychain integration.

#[tokio::test]
#[serial]
#[ignore] // Ignored by default - run with: cargo test -- --ignored
async fn test_real_keychain_integration() {
    // This test uses the actual system keychain
    // It's marked with #[serial] to avoid conflicts with other tests
    // and includes proper cleanup
    //
    // Run with: cargo test test_real_keychain_integration -- --ignored --nocapture

    println!("🔐 Testing real system keychain integration...");
    println!("⚠️  You may be prompted for Touch ID/password to access the keychain");

    let system_keychain = Arc::new(SystemKeychain);
    let manager = ApiKeyManager::with_keychain(vec![], system_keychain.clone());

    // Create an API key with real keychain
    println!("Creating API key in system keychain...");
    let result = manager
        .create_key(Some("test-real-keychain".to_string()))
        .await;

    if result.is_err() {
        eprintln!("❌ Failed to create key: {:?}", result.err());
        eprintln!("This is likely due to keychain access being denied.");
        eprintln!("On macOS, you need to approve the keychain access prompt.");
        panic!("Keychain access required - approve the system prompt and try again");
    }

    let (key, config) = result.unwrap();
    println!("✅ Created key: {} (ID: {})", config.name, config.id);

    let key_id = config.id.clone();

    // Verify key format
    assert!(key.starts_with("lr-"));
    assert_eq!(config.name, "test-real-keychain");

    // Verify key is actually in the system keychain
    println!("Verifying key in system keychain...");
    let retrieved_key = system_keychain
        .get("LocalRouter-APIKeys", &key_id)
        .expect("Failed to get from system keychain")
        .expect("Key not found in system keychain");
    assert_eq!(retrieved_key, key);
    println!("✅ Key verified in system keychain");

    // Verify through manager
    let manager_retrieved = manager
        .get_key_value(&key_id)
        .expect("Failed to get key through manager")
        .expect("Key not found through manager");
    assert_eq!(manager_retrieved, key);

    // Test key verification
    let verified = manager.verify_key(&key);
    assert!(verified.is_some());
    assert_eq!(verified.unwrap().id, key_id);

    // Clean up: Delete the key
    println!("Cleaning up: deleting key from keychain...");
    let delete_result = manager.delete_key(&key_id);
    assert!(delete_result.is_ok(), "Failed to delete key");
    println!("✅ Key deleted successfully");

    // Verify key is gone from metadata
    assert!(manager.get_key(&key_id).is_none());

    // Verify key is gone from system keychain
    let check_deleted = system_keychain
        .get("LocalRouter-APIKeys", &key_id)
        .expect("Failed to check keychain after delete");
    assert!(
        check_deleted.is_none(),
        "Key still exists in system keychain after deletion"
    );

    // Extra safety: Try to delete again (should succeed even if already gone)
    let _ = system_keychain.delete("LocalRouter-APIKeys", &key_id);

    println!("✅ Real keychain integration test passed!");
}

#[tokio::test]
#[serial]
#[ignore] // Ignored by default - run with: cargo test -- --ignored
async fn test_real_keychain_rotation() {
    // Test key rotation with real system keychain
    println!("🔐 Testing key rotation with real system keychain...");
    println!("⚠️  You may be prompted for Touch ID/password");

    let system_keychain = Arc::new(SystemKeychain);
    let manager = ApiKeyManager::with_keychain(vec![], system_keychain.clone());

    // Create a key
    let (original_key, config) = manager
        .create_key(Some("test-rotation".to_string()))
        .await
        .expect("Failed to create key");

    let key_id = config.id.clone();

    // Verify original key works
    assert!(manager.verify_key(&original_key).is_some());

    // Rotate the key
    let rotated_key = manager
        .rotate_key(&key_id)
        .await
        .expect("Failed to rotate key");

    // Verify rotated key is different
    assert_ne!(original_key, rotated_key);

    // Verify original key no longer works
    assert!(manager.verify_key(&original_key).is_none());

    // Verify rotated key works
    assert!(manager.verify_key(&rotated_key).is_some());

    // Verify rotated key is in keychain
    let stored = system_keychain
        .get("LocalRouter-APIKeys", &key_id)
        .expect("Failed to get from keychain")
        .expect("Key not found");
    assert_eq!(stored, rotated_key);

    // Clean up
    println!("Cleaning up...");
    manager.delete_key(&key_id).expect("Failed to delete key");
    let _ = system_keychain.delete("LocalRouter-APIKeys", &key_id);
    println!("✅ Key rotation test passed!");
}

#[tokio::test]
#[serial]
#[ignore] // Ignored by default - run with: cargo test -- --ignored
async fn test_real_keychain_with_config_persistence() {
    // Test full integration: real keychain + config file persistence
    println!("🔐 Testing real keychain + config persistence...");
    println!("⚠️  You may be prompted for Touch ID/password");

    let temp_dir = create_temp_config_dir();
    let config_path = test_config_path(&temp_dir);
    let system_keychain = Arc::new(SystemKeychain);

    // Create config
    let config = AppConfig::default();
    localrouter_ai::config::save_config(&config, &config_path)
        .await
        .expect("Failed to save config");

    let config_manager = ConfigManager::load_from_path(config_path.clone())
        .await
        .expect("Failed to load config manager");

    // Create key manager with real keychain
    let key_manager = ApiKeyManager::with_keychain(vec![], system_keychain.clone());

    // Create an API key
    let (key, key_config) = key_manager
        .create_key(Some("integration-real-keychain".to_string()))
        .await
        .expect("Failed to create key");

    let key_id = key_config.id.clone();

    // Add to config and save
    config_manager
        .update(|cfg| {
            cfg.api_keys.push(key_config.clone());
        })
        .expect("Failed to update config");

    config_manager
        .save()
        .await
        .expect("Failed to save config");

    // Reload config
    let reloaded = localrouter_ai::config::load_config(&config_path)
        .await
        .expect("Failed to reload config");

    assert_eq!(reloaded.api_keys.len(), 1);
    assert_eq!(reloaded.api_keys[0].name, "integration-real-keychain");

    // Create new manager with reloaded config
    let key_manager2 = ApiKeyManager::with_keychain(reloaded.api_keys, system_keychain.clone());

    // Verify key still works after reload
    let retrieved_key = key_manager2
        .get_key_value(&key_id)
        .expect("Failed to get key")
        .expect("Key not found after reload");
    assert_eq!(retrieved_key, key);

    // Verify key verification still works
    assert!(key_manager2.verify_key(&key).is_some());

    // Clean up keychain
    println!("Cleaning up...");
    key_manager2
        .delete_key(&key_id)
        .expect("Failed to delete key");
    let _ = system_keychain.delete("LocalRouter-APIKeys", &key_id);
    println!("✅ Full integration test passed!");
}

