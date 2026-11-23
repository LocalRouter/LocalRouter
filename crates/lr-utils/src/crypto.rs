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

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_the_known_digest() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn test_generate_api_key() {
        let key = generate_api_key().unwrap();
        assert!(key.starts_with("lr-"));
        assert_eq!(key.len(), 46); // "lr-" + 43 base64 chars
    }
}
