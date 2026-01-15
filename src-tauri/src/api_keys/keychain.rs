//! API key storage in system keyring
//!
//! Stores actual API keys in the system keyring:
//! - macOS: Keychain
//! - Windows: Credential Manager
//! - Linux: Secret Service / keyutils
//!
//! Keys are stored with service="LocalRouter-APIKeys" and username=key_id.

use crate::utils::errors::{AppError, AppResult};
use tracing::debug;

const KEYRING_SERVICE: &str = "LocalRouter-APIKeys";

/// Store an API key in the system keyring
///
/// # Arguments
/// * `key_id` - The unique key identifier
/// * `api_key` - The actual API key string
pub fn store_api_key(key_id: &str, api_key: &str) -> AppResult<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, key_id)
        .map_err(|e| AppError::Internal(format!("Failed to access keyring: {}", e)))?;

    entry
        .set_password(api_key)
        .map_err(|e| AppError::Internal(format!("Failed to store API key: {}", e)))?;

    debug!("Stored API key '{}' in system keyring", key_id);
    Ok(())
}

/// Retrieve an API key from the system keyring
///
/// # Arguments
/// * `key_id` - The unique key identifier
///
/// # Returns
/// * `Ok(Some(key))` if key exists
/// * `Ok(None)` if key doesn't exist
pub fn get_api_key(key_id: &str) -> AppResult<Option<String>> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, key_id)
        .map_err(|e| AppError::Internal(format!("Failed to access keyring: {}", e)))?;

    match entry.get_password() {
        Ok(key) => {
            debug!("Retrieved API key '{}' from system keyring", key_id);
            Ok(Some(key))
        }
        Err(keyring::Error::NoEntry) => {
            debug!("No API key found for '{}'", key_id);
            Ok(None)
        }
        Err(e) => Err(AppError::Internal(format!(
            "Failed to retrieve API key: {}",
            e
        ))),
    }
}

/// Delete an API key from the system keyring
///
/// # Arguments
/// * `key_id` - The unique key identifier
pub fn delete_api_key(key_id: &str) -> AppResult<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, key_id)
        .map_err(|e| AppError::Internal(format!("Failed to access keyring: {}", e)))?;

    match entry.delete_credential() {
        Ok(()) => {
            debug!("Deleted API key '{}' from system keyring", key_id);
            Ok(())
        }
        Err(keyring::Error::NoEntry) => {
            debug!("No API key to delete for '{}' (already absent)", key_id);
            Ok(())
        }
        Err(e) => Err(AppError::Internal(format!(
            "Failed to delete API key: {}",
            e
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    const TEST_KEY_ID: &str = "test-key-id-12345";
    const TEST_API_KEY: &str = "lr-test1234567890abcdef";

    fn cleanup_test_key() {
        let _ = delete_api_key(TEST_KEY_ID);
    }

    #[test]
    #[serial]
    fn test_store_and_retrieve_key() {
        cleanup_test_key();

        // Store key
        store_api_key(TEST_KEY_ID, TEST_API_KEY).unwrap();

        // Retrieve key
        let retrieved = get_api_key(TEST_KEY_ID).unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap(), TEST_API_KEY);

        cleanup_test_key();
    }

    #[test]
    #[serial]
    fn test_get_nonexistent_key() {
        cleanup_test_key();

        let retrieved = get_api_key(TEST_KEY_ID).unwrap();
        assert!(retrieved.is_none());
    }

    #[test]
    #[serial]
    fn test_delete_key() {
        cleanup_test_key();

        // Store key
        store_api_key(TEST_KEY_ID, TEST_API_KEY).unwrap();

        // Delete key
        delete_api_key(TEST_KEY_ID).unwrap();

        // Verify it's gone
        let retrieved = get_api_key(TEST_KEY_ID).unwrap();
        assert!(retrieved.is_none());
    }
}
