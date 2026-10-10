//! API key storage in system keyring
//!
//! Stores actual API keys in the system keyring:
//! - macOS: Keychain
//! - Windows: Credential Manager
//! - Linux: Secret Service / keyutils
//!
//! Keys are stored with service="LocalRouter-APIKeys" and username=key_id.

#![allow(dead_code)]

use crate::keychain_trait::{KeychainStorage, SystemKeychain};
use lr_types::AppResult;
use tracing::debug;

const KEYRING_SERVICE: &str = "LocalRouter-APIKeys";

/// Store an API key in the system keyring
///
/// Goes through [`SystemKeychain`], which splits keys longer than the
/// platform's per-entry limit across several entries.
///
/// # Arguments
/// * `key_id` - The unique key identifier
/// * `api_key` - The actual API key string
pub fn store_api_key(key_id: &str, api_key: &str) -> AppResult<()> {
    SystemKeychain.store(KEYRING_SERVICE, key_id, api_key)?;
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
    let key = SystemKeychain.get(KEYRING_SERVICE, key_id)?;
    if key.is_some() {
        debug!("Retrieved API key '{}' from system keyring", key_id);
    } else {
        debug!("No API key found for '{}'", key_id);
    }
    Ok(key)
}

/// Delete an API key from the system keyring
///
/// # Arguments
/// * `key_id` - The unique key identifier
pub fn delete_api_key(key_id: &str) -> AppResult<()> {
    SystemKeychain.delete(KEYRING_SERVICE, key_id)?;
    debug!("Deleted API key '{}' from system keyring", key_id);
    Ok(())
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
    #[ignore = "requires access to the real operating-system credential store"]
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
    #[ignore = "requires access to the real operating-system credential store"]
    #[serial]
    fn test_get_nonexistent_key() {
        cleanup_test_key();

        let retrieved = get_api_key(TEST_KEY_ID).unwrap();
        assert!(retrieved.is_none());
    }

    #[test]
    #[ignore = "requires access to the real operating-system credential store"]
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
