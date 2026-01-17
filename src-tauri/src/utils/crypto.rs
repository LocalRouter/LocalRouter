//! Cryptographic utilities
//!
//! Functions for encryption, hashing, and secure key generation.

use anyhow::Result;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::rand::{SecureRandom, SystemRandom};

/// Generate a secure random API key with format: lr-{base64url(32 bytes)}
pub fn generate_api_key() -> Result<String> {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Failed to generate random bytes"))?;

    let encoded = URL_SAFE_NO_PAD.encode(bytes);
    Ok(format!("lr-{}", encoded))
}

/// Hash an API key for storage
#[allow(dead_code)]
pub fn hash_api_key(key: &str) -> Result<String> {
    bcrypt::hash(key, bcrypt::DEFAULT_COST).map_err(Into::into)
}

/// Verify an API key against a stored hash
#[allow(dead_code)]
pub fn verify_api_key(key: &str, hash: &str) -> Result<bool> {
    bcrypt::verify(key, hash).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_api_key() {
        let key = generate_api_key().unwrap();
        assert!(key.starts_with("lr-"));
        assert_eq!(key.len(), 46); // "lr-" + 43 base64 chars
    }

    #[test]
    fn test_hash_and_verify() {
        let key = "test-key-123";
        let hash = hash_api_key(key).unwrap();
        assert!(verify_api_key(key, &hash).unwrap());
        assert!(!verify_api_key("wrong-key", &hash).unwrap());
    }
}
