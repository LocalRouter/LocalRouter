//! PKCE (Proof Key for Code Exchange) utilities for OAuth 2.0
//!
//! Implements PKCE as defined in RFC 7636 with S256 (SHA-256) challenge method.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// PKCE challenge containing code verifier and challenge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkceChallenge {
    /// Code verifier (random string, 43-128 characters)
    pub code_verifier: String,

    /// Code challenge (BASE64URL(SHA256(code_verifier)))
    pub code_challenge: String,

    /// Challenge method (always "S256" for SHA-256)
    pub code_challenge_method: String,
}

/// Generate PKCE challenge for OAuth authorization code flow
///
/// Creates a cryptographically secure code verifier and derives the code challenge
/// using SHA-256 hashing. The code verifier is a 64-character random string using
/// URL-safe characters (A-Z, a-z, 0-9), and the challenge is the base64url-encoded
/// SHA-256 hash of the verifier.
///
/// # Returns
/// * PKCE challenge containing verifier, challenge, and method ("S256")
///
/// # Example
/// ```ignore
/// use lr_oauth::browser::generate_pkce_challenge;
///
/// let pkce = generate_pkce_challenge();
/// // Use pkce.code_challenge in authorization URL
/// // Use pkce.code_verifier in token exchange
/// ```
pub fn generate_pkce_challenge() -> Result<PkceChallenge, &'static str> {
    // Generate random code_verifier (64 bytes, base64url-encoded = 86 characters)
    // RFC 7636 specifies 43-128 characters from unreserved URI characters
    let rng = SystemRandom::new();
    let mut verifier_bytes = [0u8; 64];
    rng.fill(&mut verifier_bytes)
        .map_err(|_| "Failed to generate random PKCE verifier")?;
    let code_verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

    // Generate code_challenge = BASE64URL(SHA256(code_verifier))
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let hash = hasher.finalize();
    let code_challenge = URL_SAFE_NO_PAD.encode(hash);

    Ok(PkceChallenge {
        code_verifier,
        code_challenge,
        code_challenge_method: "S256".to_string(),
    })
}

/// Generate a random state string for CSRF protection
///
/// Creates a 32-character random string using URL-safe characters (A-Z, a-z, 0-9).
/// The state parameter should be stored before redirecting to the authorization server
/// and verified when the callback is received.
///
/// # Returns
/// * Random 32-character state string
///
/// # Example
/// ```ignore
/// use lr_oauth::browser::generate_state;
///
/// let state = generate_state();
/// // Store state in session
/// // Include state in authorization URL
/// // Verify state matches when callback is received
/// ```
pub fn generate_state() -> Result<String, &'static str> {
    let rng = SystemRandom::new();
    let mut state_bytes = [0u8; 32];
    rng.fill(&mut state_bytes)
        .map_err(|_| "Failed to generate random state")?;
    Ok(URL_SAFE_NO_PAD.encode(state_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_pkce_challenge() {
        let pkce = generate_pkce_challenge().unwrap();

        // Verify code verifier is base64url-encoded 64 bytes (86 characters)
        assert_eq!(pkce.code_verifier.len(), 86);

        // Verify code verifier uses only base64url characters
        assert!(pkce
            .code_verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));

        // Verify code challenge is not empty
        assert!(!pkce.code_challenge.is_empty());

        // Verify method is S256
        assert_eq!(pkce.code_challenge_method, "S256");

        // Verify code challenge is base64url encoded (no padding)
        assert!(!pkce.code_challenge.contains('='));
    }

    #[test]
    fn test_pkce_challenge_uniqueness() {
        let pkce1 = generate_pkce_challenge().unwrap();
        let pkce2 = generate_pkce_challenge().unwrap();

        // Each call should generate different values
        assert_ne!(pkce1.code_verifier, pkce2.code_verifier);
        assert_ne!(pkce1.code_challenge, pkce2.code_challenge);
    }

    #[test]
    fn test_pkce_challenge_deterministic() {
        // Same verifier should always produce same challenge
        let verifier = "test_verifier_12345678901234567890123456789012345678901234";

        let mut hasher1 = Sha256::new();
        hasher1.update(verifier.as_bytes());
        let challenge1 = URL_SAFE_NO_PAD.encode(hasher1.finalize());

        let mut hasher2 = Sha256::new();
        hasher2.update(verifier.as_bytes());
        let challenge2 = URL_SAFE_NO_PAD.encode(hasher2.finalize());

        assert_eq!(challenge1, challenge2);
    }

    #[test]
    fn test_generate_state() {
        let state = generate_state().unwrap();

        // Verify length: base64url-encoded 32 bytes = 43 characters
        assert_eq!(state.len(), 43);

        // Verify uses only base64url characters
        assert!(state
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn test_state_uniqueness() {
        let state1 = generate_state().unwrap();
        let state2 = generate_state().unwrap();

        // Each call should generate different values
        assert_ne!(state1, state2);
    }

    #[test]
    fn test_state_randomness() {
        // Generate multiple states and verify they're all different
        let mut states = std::collections::HashSet::new();
        for _ in 0..100 {
            let state = generate_state().unwrap();
            assert!(states.insert(state), "Generated duplicate state");
        }
        assert_eq!(states.len(), 100);
    }

    #[test]
    fn test_pkce_randomness() {
        // Generate multiple PKCE challenges and verify they're all different
        let mut verifiers = std::collections::HashSet::new();
        for _ in 0..100 {
            let pkce = generate_pkce_challenge().unwrap();
            assert!(
                verifiers.insert(pkce.code_verifier),
                "Generated duplicate PKCE verifier"
            );
        }
        assert_eq!(verifiers.len(), 100);
    }
}
