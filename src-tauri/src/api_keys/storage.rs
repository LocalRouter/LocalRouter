//! Encrypted storage for API keys
//!
//! Handles reading and writing API keys to disk with encryption.
//! Uses system keyring for encryption key storage with fallback to file-based encryption.

use crate::config::ApiKeyConfig;
use crate::utils::errors::{AppError, AppResult};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tokio::fs;
use tracing::{debug, warn};

const ENCRYPTION_KEY_NAME: &str = "localrouter-api-keys-encryption";
const NONCE_SIZE: usize = 12;

/// In-memory cache for encryption key (stays consistent within a process)
static ENCRYPTION_KEY_CACHE: OnceLock<[u8; 32]> = OnceLock::new();

/// Encrypted API keys storage format
#[derive(Debug, Serialize, Deserialize)]
struct EncryptedStorage {
    /// Version of the storage format
    version: u32,
    /// Nonce used for encryption
    nonce: Vec<u8>,
    /// Encrypted data
    data: Vec<u8>,
}

/// Get or create encryption key
fn get_encryption_key() -> AppResult<[u8; 32]> {
    // Use get_or_init to ensure only one thread initializes the key
    let key = ENCRYPTION_KEY_CACHE.get_or_init(|| {
        // Try to get key from system keyring
        match keyring::Entry::new("LocalRouter", ENCRYPTION_KEY_NAME) {
            Ok(entry) => {
                match entry.get_password() {
                    Ok(key_str) => {
                        // Parse existing key
                        if let Ok(key_bytes) = hex::decode(&key_str) {
                            if key_bytes.len() == 32 {
                                let mut key = [0u8; 32];
                                key.copy_from_slice(&key_bytes);
                                debug!("Retrieved encryption key from system keyring");
                                return key;
                            }
                        }
                        warn!("Invalid encryption key in keyring, generating new one");
                    }
                    Err(keyring::Error::NoEntry) => {
                        debug!("No encryption key in keyring, generating new one");
                    }
                    Err(e) => {
                        warn!("Failed to retrieve encryption key from keyring: {}", e);
                    }
                }

                // Generate new key
                generate_new_key_internal(&entry)
            }
            Err(e) => {
                warn!("Failed to access system keyring: {}", e);
                warn!("Using fallback encryption key - keys may not persist across app reinstalls");

                // Fallback: use a derived key based on machine ID
                // This is less secure but works without keyring access
                let machine_id = machine_uid::get().unwrap_or_else(|_| "fallback-id".to_string());
                let mut key = [0u8; 32];
                let hash = ring::digest::digest(&ring::digest::SHA256, machine_id.as_bytes());
                key.copy_from_slice(hash.as_ref());
                key
            }
        }
    });

    Ok(*key)
}

/// Generate a new encryption key and store it in the keyring (internal, panics on error)
fn generate_new_key_internal(entry: &keyring::Entry) -> [u8; 32] {
    let rng = SystemRandom::new();
    let mut key = [0u8; 32];
    rng.fill(&mut key)
        .expect("Failed to generate encryption key");

    // Store in keyring
    let key_str = hex::encode(&key);
    if let Err(e) = entry.set_password(&key_str) {
        warn!("Failed to store encryption key in keyring: {}", e);
        warn!("Using in-memory key only - keys will not persist across app reinstalls");
    } else {
        debug!("Stored new encryption key in system keyring");
    }

    key
}

/// Encrypt API keys data
fn encrypt_data(data: &[u8]) -> AppResult<(Vec<u8>, Vec<u8>)> {
    let key_bytes = get_encryption_key()?;
    let rng = SystemRandom::new();

    // Generate random nonce
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    rng.fill(&mut nonce_bytes)
        .map_err(|_| AppError::Internal("Failed to generate nonce".to_string()))?;

    // Create encryption key
    let unbound_key = UnboundKey::new(&AES_256_GCM, &key_bytes)
        .map_err(|_| AppError::Internal("Failed to create encryption key".to_string()))?;
    let sealing_key = LessSafeKey::new(unbound_key);

    // Create nonce
    let nonce = Nonce::try_assume_unique_for_key(&nonce_bytes)
        .map_err(|_| AppError::Internal("Failed to create nonce".to_string()))?;

    // Encrypt data
    let mut encrypted = data.to_vec();
    sealing_key
        .seal_in_place_append_tag(nonce, Aad::empty(), &mut encrypted)
        .map_err(|_| AppError::Internal("Failed to encrypt data".to_string()))?;

    Ok((encrypted, nonce_bytes.to_vec()))
}

/// Decrypt API keys data
fn decrypt_data(encrypted: &[u8], nonce_bytes: &[u8]) -> AppResult<Vec<u8>> {
    let key_bytes = get_encryption_key()?;

    // Create decryption key
    let unbound_key = UnboundKey::new(&AES_256_GCM, &key_bytes)
        .map_err(|_| AppError::Internal("Failed to create decryption key".to_string()))?;
    let opening_key = LessSafeKey::new(unbound_key);

    // Create nonce from bytes
    let mut nonce_array = [0u8; NONCE_SIZE];
    nonce_array.copy_from_slice(nonce_bytes);
    let nonce = Nonce::try_assume_unique_for_key(&nonce_array)
        .map_err(|_| AppError::Internal("Failed to create nonce".to_string()))?;

    // Decrypt data
    let mut decrypted = encrypted.to_vec();
    let decrypted_slice = opening_key
        .open_in_place(nonce, Aad::empty(), &mut decrypted)
        .map_err(|_| AppError::Internal("Failed to decrypt data".to_string()))?;

    Ok(decrypted_slice.to_vec())
}

/// Load API keys from disk
pub async fn load_api_keys() -> AppResult<Vec<ApiKeyConfig>> {
    let file_path = crate::config::paths::api_keys_file()?;

    // If file doesn't exist, return empty list
    if !file_path.exists() {
        debug!("API keys file does not exist, returning empty list");
        return Ok(Vec::new());
    }

    // Read encrypted file
    let encrypted_data = fs::read(&file_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read API keys file: {}", e)))?;

    // Parse encrypted storage
    let encrypted_storage: EncryptedStorage = serde_json::from_slice(&encrypted_data)
        .map_err(|e| AppError::Internal(format!("Failed to parse API keys file: {}", e)))?;

    // Decrypt data
    let decrypted = decrypt_data(&encrypted_storage.data, &encrypted_storage.nonce)?;

    // Parse API keys
    let keys: Vec<ApiKeyConfig> = serde_json::from_slice(&decrypted)
        .map_err(|e| AppError::Internal(format!("Failed to parse API keys data: {}", e)))?;

    debug!("Loaded {} API keys from disk", keys.len());
    Ok(keys)
}

/// Save API keys to disk
pub async fn save_api_keys(keys: &[ApiKeyConfig]) -> AppResult<()> {
    let file_path = crate::config::paths::api_keys_file()?;

    // Ensure config directory exists
    if let Some(parent) = file_path.parent() {
        crate::config::paths::ensure_dir_exists(&parent.to_path_buf())?;
    }

    // Serialize keys
    let json_data = serde_json::to_vec_pretty(keys)
        .map_err(|e| AppError::Internal(format!("Failed to serialize API keys: {}", e)))?;

    // Encrypt data
    let (encrypted, nonce) = encrypt_data(&json_data)?;

    // Create encrypted storage
    let storage = EncryptedStorage {
        version: 1,
        nonce,
        data: encrypted,
    };

    // Serialize encrypted storage
    let encrypted_json = serde_json::to_vec_pretty(&storage)
        .map_err(|e| AppError::Internal(format!("Failed to serialize encrypted storage: {}", e)))?;

    // Write to temporary file first (atomic write)
    let temp_path = file_path.with_extension("json.tmp");
    fs::write(&temp_path, &encrypted_json)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write API keys file: {}", e)))?;

    // Rename to actual file (atomic operation on Unix)
    fs::rename(&temp_path, &file_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to rename API keys file: {}", e)))?;

    // Set restrictive permissions (Unix only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&file_path)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to get file metadata: {}", e)))?
            .permissions();
        perms.set_mode(0o600); // User read/write only
        fs::set_permissions(&file_path, perms)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to set file permissions: {}", e)))?;
    }

    debug!("Saved {} API keys to disk", keys.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ModelSelection;
    use tempfile::tempdir;

    #[test]
    fn test_encrypt_decrypt() {
        let data = b"Hello, World!";
        let (encrypted, nonce) = encrypt_data(data).unwrap();

        assert_ne!(encrypted, data);

        let decrypted = decrypt_data(&encrypted, &nonce).unwrap();
        assert_eq!(decrypted, data);
    }

    #[test]
    fn test_encrypt_decrypt_empty() {
        let data = b"";
        let (encrypted, nonce) = encrypt_data(data).unwrap();
        let decrypted = decrypt_data(&encrypted, &nonce).unwrap();
        assert_eq!(decrypted, data);
    }

    #[test]
    fn test_decrypt_with_wrong_nonce_fails() {
        let data = b"Secret data";
        let (encrypted, _) = encrypt_data(data).unwrap();
        let wrong_nonce = vec![0u8; NONCE_SIZE];

        let result = decrypt_data(&encrypted, &wrong_nonce);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_save_and_load_keys() {
        let _temp_dir = tempdir().unwrap();

        // Create test keys
        let keys = vec![
            ApiKeyConfig::new(
                "test-key-1".to_string(),
                "hash1".to_string(),
                ModelSelection::Router {
                    router_name: "Minimum Cost".to_string(),
                },
            ),
            ApiKeyConfig::new(
                "test-key-2".to_string(),
                "hash2".to_string(),
                ModelSelection::DirectModel {
                    provider: "ollama".to_string(),
                    model: "llama2".to_string(),
                },
            ),
        ];

        // Save keys
        // We can't easily test save_api_keys directly since it uses paths::api_keys_file()
        // but we can test the encrypt/decrypt round trip

        let json = serde_json::to_vec(&keys).unwrap();
        let (encrypted, nonce) = encrypt_data(&json).unwrap();
        let decrypted = decrypt_data(&encrypted, &nonce).unwrap();
        let loaded_keys: Vec<ApiKeyConfig> = serde_json::from_slice(&decrypted).unwrap();

        assert_eq!(loaded_keys.len(), 2);
        assert_eq!(loaded_keys[0].name, "test-key-1");
        assert_eq!(loaded_keys[1].name, "test-key-2");
    }

    #[test]
    fn test_encryption_key_consistency() {
        // Multiple calls should return the same key
        let key1 = get_encryption_key().unwrap();
        let key2 = get_encryption_key().unwrap();
        assert_eq!(key1, key2);
    }
}
