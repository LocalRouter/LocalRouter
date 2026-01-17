//! Provider API key storage in system keyring
//!
//! Stores provider API keys (OpenAI, Anthropic, etc.) directly in the system keyring:
//! - macOS: Keychain
//! - Windows: Credential Manager
//! - Linux: Secret Service / keyutils
//!
//! Keys are stored with service="LocalRouter-Providers" and username=provider_name.
//!
//! Uses CachedKeychain to avoid repeated password prompts.

use crate::api_keys::keychain_trait::{CachedKeychain, KeychainStorage};
use crate::utils::errors::AppResult;
use std::sync::OnceLock;
use tracing::{debug, warn};

const KEYRING_SERVICE: &str = "LocalRouter-Providers";

/// Global cached keychain instance for provider keys
/// This ensures all provider key access goes through a single cached instance
static PROVIDER_KEYCHAIN: OnceLock<CachedKeychain> = OnceLock::new();

/// Get the global provider keychain instance (with caching)
fn get_keychain() -> &'static CachedKeychain {
    PROVIDER_KEYCHAIN.get_or_init(|| {
        CachedKeychain::auto().unwrap_or_else(|e| {
            warn!("Failed to create auto keychain: {}, falling back to system", e);
            CachedKeychain::system()
        })
    })
}

/// Store a provider API key in the system keyring
///
/// # Arguments
/// * `provider_name` - The provider identifier (e.g., "openai", "anthropic", "custom:my-provider")
/// * `api_key` - The API key to store
///
/// # Returns
/// * `Ok(())` if successful
/// * `Err(AppError)` if keyring access fails
///
/// Note: Uses CachedKeychain which automatically caches the key in memory
pub fn store_provider_key(provider_name: &str, api_key: &str) -> AppResult<()> {
    let keychain = get_keychain();
    keychain.store(KEYRING_SERVICE, provider_name, api_key)?;

    debug!(
        "Stored API key for provider '{}' in system keyring (cached)",
        provider_name
    );
    Ok(())
}

/// Retrieve a provider API key from the system keyring
///
/// # Arguments
/// * `provider_name` - The provider identifier
///
/// # Returns
/// * `Ok(Some(key))` if key exists
/// * `Ok(None)` if key doesn't exist
/// * `Err(AppError)` if keyring access fails
///
/// Note: First access prompts for keychain password, then cached in memory
/// for the lifetime of the application process.
pub fn get_provider_key(provider_name: &str) -> AppResult<Option<String>> {
    let keychain = get_keychain();
    let result = keychain.get(KEYRING_SERVICE, provider_name)?;

    if result.is_some() {
        debug!(
            "Retrieved API key for provider '{}' (from cache or keyring)",
            provider_name
        );
    } else {
        debug!("No API key found for provider '{}'", provider_name);
    }

    Ok(result)
}

/// Delete a provider API key from the system keyring
///
/// # Arguments
/// * `provider_name` - The provider identifier
///
/// # Returns
/// * `Ok(())` if successful (even if key didn't exist)
/// * `Err(AppError)` if keyring access fails
///
/// Note: Also removes the key from the in-memory cache
pub fn delete_provider_key(provider_name: &str) -> AppResult<()> {
    let keychain = get_keychain();
    keychain.delete(KEYRING_SERVICE, provider_name)?;

    debug!(
        "Deleted API key for provider '{}' from system keyring and cache",
        provider_name
    );
    Ok(())
}

/// Check if a provider has an API key stored
///
/// # Arguments
/// * `provider_name` - The provider identifier
///
/// # Returns
/// * `Ok(true)` if key exists
/// * `Ok(false)` if key doesn't exist
/// * `Err(AppError)` if keyring access fails
pub fn has_provider_key(provider_name: &str) -> AppResult<bool> {
    get_provider_key(provider_name).map(|key| key.is_some())
}

/// List all provider names that have API keys stored
///
/// Note: This function is limited by the keyring API - some platforms
/// may not support enumeration. In such cases, it returns an empty list
/// with a warning.
///
/// # Returns
/// * `Ok(Vec<String>)` with provider names
/// * `Err(AppError)` if keyring access fails
#[allow(dead_code)]
pub fn list_provider_keys() -> AppResult<Vec<String>> {
    // Unfortunately, the keyring crate doesn't provide a way to enumerate entries
    // for a given service across all platforms. This is a limitation of the underlying
    // platform APIs (especially on Windows and Linux).
    //
    // For now, we return an empty list and log a warning. In a production app,
    // you would maintain a separate list of provider names in the config.
    warn!("list_provider_keys() is not supported by the keyring API - returning empty list");
    warn!("To track which providers have keys, maintain a list in your application config");
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    const TEST_PROVIDER: &str = "test-provider-for-unit-tests";
    const TEST_API_KEY: &str = "test-api-key-12345";

    // Clean up test key before and after each test
    fn cleanup_test_key() {
        let _ = delete_provider_key(TEST_PROVIDER);
    }

    // Note: Keyring tests may fail in automated/CI environments due to:
    // - macOS: Keychain access requires user authorization (Touch ID/password)
    // - Linux: Secret Service D-Bus may not be available in headless environments
    // - Windows: Credential Manager may require interactive session
    //
    // These tests are designed for local development and manual verification.
    // In production, the keyring will work correctly with user interaction.

    #[test]
    #[serial]
    fn test_store_and_retrieve_key() {
        cleanup_test_key();

        // Store key
        let result = store_provider_key(TEST_PROVIDER, TEST_API_KEY);
        match &result {
            Ok(()) => println!("✓ Store succeeded"),
            Err(e) => println!("✗ Store failed: {}", e),
        }
        assert!(result.is_ok(), "Failed to store key: {:?}", result);

        // Retrieve key
        let retrieved_result = get_provider_key(TEST_PROVIDER);
        match &retrieved_result {
            Ok(Some(key)) => println!("✓ Retrieved key: {}", key),
            Ok(None) => println!("✗ Key not found"),
            Err(e) => println!("✗ Retrieve failed: {}", e),
        }
        let retrieved = retrieved_result.unwrap();
        assert!(retrieved.is_some(), "Key was stored but could not be retrieved - this may indicate keyring access issues on your system");
        assert_eq!(retrieved.unwrap(), TEST_API_KEY);

        cleanup_test_key();
    }

    #[test]
    #[serial]
    fn test_get_nonexistent_key() {
        cleanup_test_key();

        let retrieved = get_provider_key(TEST_PROVIDER).unwrap();
        assert!(retrieved.is_none());
    }

    #[test]
    #[serial]
    fn test_delete_key() {
        cleanup_test_key();

        // Store key
        store_provider_key(TEST_PROVIDER, TEST_API_KEY).unwrap();

        // Verify it exists
        let retrieved = get_provider_key(TEST_PROVIDER).unwrap();
        assert!(retrieved.is_some());

        // Delete key
        let result = delete_provider_key(TEST_PROVIDER);
        assert!(result.is_ok());

        // Verify it's gone
        let retrieved = get_provider_key(TEST_PROVIDER).unwrap();
        assert!(retrieved.is_none());
    }

    #[test]
    #[serial]
    fn test_delete_nonexistent_key() {
        cleanup_test_key();

        // Delete should succeed even if key doesn't exist
        let result = delete_provider_key(TEST_PROVIDER);
        assert!(result.is_ok());
    }

    #[test]
    #[serial]
    fn test_has_provider_key() {
        cleanup_test_key();

        // Should return false for nonexistent key
        assert!(!has_provider_key(TEST_PROVIDER).unwrap());

        // Store key
        store_provider_key(TEST_PROVIDER, TEST_API_KEY).unwrap();

        // Should return true now
        assert!(has_provider_key(TEST_PROVIDER).unwrap());

        cleanup_test_key();
    }

    #[test]
    #[serial]
    fn test_overwrite_existing_key() {
        cleanup_test_key();

        // Store initial key
        store_provider_key(TEST_PROVIDER, "old-key").unwrap();

        // Overwrite with new key
        store_provider_key(TEST_PROVIDER, "new-key").unwrap();

        // Retrieve and verify it's the new key
        let retrieved = get_provider_key(TEST_PROVIDER).unwrap();
        assert_eq!(retrieved.unwrap(), "new-key");

        cleanup_test_key();
    }

    #[test]
    #[serial]
    fn test_multiple_providers() {
        let provider1 = "test-provider-1";
        let provider2 = "test-provider-2";

        // Clean up
        let _ = delete_provider_key(provider1);
        let _ = delete_provider_key(provider2);

        // Store keys for different providers
        store_provider_key(provider1, "key-1").unwrap();
        store_provider_key(provider2, "key-2").unwrap();

        // Retrieve and verify they're independent
        assert_eq!(get_provider_key(provider1).unwrap().unwrap(), "key-1");
        assert_eq!(get_provider_key(provider2).unwrap().unwrap(), "key-2");

        // Clean up
        let _ = delete_provider_key(provider1);
        let _ = delete_provider_key(provider2);
    }

    #[test]
    #[serial]
    fn test_custom_provider_naming() {
        let custom_provider = "custom:my-special-provider";

        // Clean up
        let _ = delete_provider_key(custom_provider);

        // Store and retrieve with custom naming
        store_provider_key(custom_provider, "custom-key").unwrap();
        assert_eq!(
            get_provider_key(custom_provider).unwrap().unwrap(),
            "custom-key"
        );

        // Clean up
        let _ = delete_provider_key(custom_provider);
    }
}
