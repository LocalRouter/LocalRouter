//! Permanently ignoring individual secrets, without ever storing them.
//!
//! When a user says "this particular value is fine, stop flagging it", we have
//! to recognize that same value on every later request while keeping it out of
//! the config file, the monitor store, and the logs. So we store a salted
//! PBKDF2-HMAC-SHA256 digest instead of the secret.
//!
//! Two properties matter here:
//!
//! - **Salted.** A random per-client salt makes precomputed/rainbow tables
//!   useless and stops the same secret from being correlated across clients.
//! - **Slow.** Detected secrets are not always high-entropy — plenty are
//!   human-chosen passwords — so a plain digest would fall to a dictionary
//!   attack if the config file ever leaked. The KDF cost makes that expensive.
//!
//! The iteration count is recorded alongside each digest so it can be raised
//! later without invalidating entries written by older versions.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ring::pbkdf2;
use ring::rand::{SecureRandom, SystemRandom};
use std::num::NonZeroU32;

/// PBKDF2 iterations for new entries (OWASP's PBKDF2-HMAC-SHA256 guidance).
///
/// This is deliberately expensive. Callers must memoize verification per
/// (client, secret) rather than paying it on every request — see
/// `DismissalCache`.
pub const DEFAULT_ITERATIONS: u32 = 600_000;

const ALGORITHM: pbkdf2::Algorithm = pbkdf2::PBKDF2_HMAC_SHA256;
const DIGEST_LEN: usize = 32;
const SALT_LEN: usize = 16;

/// Generate a fresh random salt, base64-encoded. One per client is enough:
/// the salt defeats rainbow tables and cross-client correlation, and sharing it
/// within a client only reveals that two entries hold the same value — which
/// we deduplicate anyway — while keeping lookup to a single KDF run.
pub fn new_salt() -> Result<String, String> {
    let mut salt = [0u8; SALT_LEN];
    SystemRandom::new()
        .fill(&mut salt)
        .map_err(|_| "Failed to generate salt".to_string())?;
    Ok(BASE64.encode(salt))
}

/// Derive the stored digest for `secret`, base64-encoded.
pub fn hash_secret(secret: &str, salt_b64: &str, iterations: u32) -> Result<String, String> {
    let salt = BASE64
        .decode(salt_b64)
        .map_err(|e| format!("Invalid salt: {e}"))?;
    let iterations =
        NonZeroU32::new(iterations).ok_or_else(|| "Iterations must be non-zero".to_string())?;

    let mut digest = [0u8; DIGEST_LEN];
    pbkdf2::derive(ALGORITHM, iterations, &salt, secret.as_bytes(), &mut digest);
    Ok(BASE64.encode(digest))
}

/// Check `secret` against a stored digest in constant time.
///
/// Returns false rather than an error on malformed input: a corrupt entry must
/// never be read as "this secret was dismissed".
pub fn verify_secret(secret: &str, salt_b64: &str, iterations: u32, expected_b64: &str) -> bool {
    let (Ok(salt), Ok(expected)) = (BASE64.decode(salt_b64), BASE64.decode(expected_b64)) else {
        return false;
    };
    let Some(iterations) = NonZeroU32::new(iterations) else {
        return false;
    };
    pbkdf2::verify(ALGORITHM, iterations, &salt, secret.as_bytes(), &expected).is_ok()
}

/// One stored digest to check a candidate secret against.
#[derive(Debug, Clone, Copy)]
pub struct DismissedDigest<'a> {
    pub hash: &'a str,
    pub iterations: u32,
}

/// Memoizes dismissal verdicts.
///
/// The KDF is deliberately expensive (~100ms in release), and a dismissed
/// secret by definition recurs on every later request that carries it — so
/// paying it per request would be a visible stall. Verdicts are cached per
/// client and dropped whenever that client's salt or entry set changes, so
/// removing an exception in settings takes effect on the next request.
///
/// The cache is keyed by a SHA-256 of the secret, never the secret itself.
#[derive(Default)]
pub struct DismissalCache {
    clients: std::sync::Mutex<std::collections::HashMap<String, ClientVerdicts>>,
}

#[derive(Default)]
struct ClientVerdicts {
    /// Identity of the entry set these verdicts were computed against.
    fingerprint: [u8; DIGEST_LEN],
    verdicts: std::collections::HashMap<[u8; DIGEST_LEN], bool>,
}

fn sha256(bytes: &[u8]) -> [u8; DIGEST_LEN] {
    let d = ring::digest::digest(&ring::digest::SHA256, bytes);
    let mut out = [0u8; DIGEST_LEN];
    out.copy_from_slice(d.as_ref());
    out
}

impl DismissalCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `secret` is one this client has permanently ignored.
    pub fn is_dismissed(
        &self,
        client_id: &str,
        salt_b64: Option<&str>,
        entries: &[DismissedDigest<'_>],
        secret: &str,
    ) -> bool {
        let (Some(salt), false) = (salt_b64, entries.is_empty()) else {
            return false;
        };

        let fingerprint = {
            let mut material = salt.as_bytes().to_vec();
            for e in entries {
                material.extend_from_slice(e.hash.as_bytes());
                material.extend_from_slice(&e.iterations.to_le_bytes());
            }
            sha256(&material)
        };
        let key = sha256(secret.as_bytes());

        if let Ok(mut clients) = self.clients.lock() {
            let cached = clients.entry(client_id.to_string()).or_default();
            if cached.fingerprint != fingerprint {
                // Entry set changed — previous verdicts no longer apply.
                cached.fingerprint = fingerprint;
                cached.verdicts.clear();
            } else if let Some(&verdict) = cached.verdicts.get(&key) {
                return verdict;
            }
        }

        let verdict = entries
            .iter()
            .any(|e| verify_secret(secret, salt, e.iterations, e.hash));

        if let Ok(mut clients) = self.clients.lock() {
            let cached = clients.entry(client_id.to_string()).or_default();
            // Only record against the entry set we actually verified against;
            // a concurrent config change may have swapped it underneath us.
            if cached.fingerprint == fingerprint {
                cached.verdicts.insert(key, verdict);
            }
        }
        verdict
    }

    /// Drop every cached verdict for a client (config edits, client removal).
    pub fn invalidate(&self, client_id: &str) {
        if let Ok(mut clients) = self.clients.lock() {
            clients.remove(client_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_verifies() {
        let salt = new_salt().unwrap();
        // Keep tests fast: the cost parameter is what's expensive, not the math.
        let hash = hash_secret("hunter2", &salt, 10).unwrap();

        assert_eq!(hash_secret("hunter2", &salt, 10).unwrap(), hash);
        assert!(verify_secret("hunter2", &salt, 10, &hash));
        assert!(!verify_secret("hunter3", &salt, 10, &hash));
    }

    #[test]
    fn salts_are_unique_and_change_the_digest() {
        let a = new_salt().unwrap();
        let b = new_salt().unwrap();
        assert_ne!(a, b);
        assert_ne!(
            hash_secret("hunter2", &a, 10).unwrap(),
            hash_secret("hunter2", &b, 10).unwrap(),
            "the salt must actually feed the digest"
        );
    }

    #[test]
    fn digest_does_not_contain_the_secret() {
        let salt = new_salt().unwrap();
        let hash = hash_secret("AKIAIOSFODNN7EXAMPLE", &salt, 10).unwrap();
        assert!(!hash.contains("AKIA"));
    }

    #[test]
    fn iteration_count_is_part_of_the_digest() {
        let salt = new_salt().unwrap();
        let hash = hash_secret("hunter2", &salt, 10).unwrap();
        assert!(
            !verify_secret("hunter2", &salt, 20, &hash),
            "a different cost must not verify"
        );
    }

    fn digests<'a>(hashes: &'a [String]) -> Vec<DismissedDigest<'a>> {
        hashes
            .iter()
            .map(|h| DismissedDigest {
                hash: h,
                iterations: 10,
            })
            .collect()
    }

    #[test]
    fn cache_matches_only_dismissed_secrets() {
        let salt = new_salt().unwrap();
        let hashes = vec![hash_secret("hunter2", &salt, 10).unwrap()];
        let entries = digests(&hashes);
        let cache = DismissalCache::new();

        assert!(cache.is_dismissed("c1", Some(&salt), &entries, "hunter2"));
        // Cached path returns the same verdict
        assert!(cache.is_dismissed("c1", Some(&salt), &entries, "hunter2"));
        assert!(!cache.is_dismissed("c1", Some(&salt), &entries, "other-secret"));
        // Exceptions do not leak across clients
        assert!(cache.is_dismissed("c2", Some(&salt), &entries, "hunter2"));
    }

    #[test]
    fn no_entries_or_no_salt_means_not_dismissed() {
        let salt = new_salt().unwrap();
        let hashes = vec![hash_secret("hunter2", &salt, 10).unwrap()];
        let cache = DismissalCache::new();

        assert!(!cache.is_dismissed("c1", Some(&salt), &[], "hunter2"));
        assert!(!cache.is_dismissed("c1", None, &digests(&hashes), "hunter2"));
    }

    /// Removing an exception in settings must take effect immediately, even
    /// though the previous verdict is cached.
    #[test]
    fn changing_entries_invalidates_cached_verdicts() {
        let salt = new_salt().unwrap();
        let hashes = vec![hash_secret("hunter2", &salt, 10).unwrap()];
        let cache = DismissalCache::new();
        assert!(cache.is_dismissed("c1", Some(&salt), &digests(&hashes), "hunter2"));

        // Entry removed
        assert!(!cache.is_dismissed("c1", Some(&salt), &[], "hunter2"));
        // ...and re-added
        assert!(cache.is_dismissed("c1", Some(&salt), &digests(&hashes), "hunter2"));

        // Explicit invalidation also forces a re-check
        cache.invalidate("c1");
        assert!(cache.is_dismissed("c1", Some(&salt), &digests(&hashes), "hunter2"));
    }

    /// A rotated salt must not let stale digests keep matching.
    #[test]
    fn rotating_the_salt_invalidates_cached_verdicts() {
        let salt = new_salt().unwrap();
        let hashes = vec![hash_secret("hunter2", &salt, 10).unwrap()];
        let entries = digests(&hashes);
        let cache = DismissalCache::new();
        assert!(cache.is_dismissed("c1", Some(&salt), &entries, "hunter2"));

        let new_salt_value = new_salt().unwrap();
        assert!(
            !cache.is_dismissed("c1", Some(&new_salt_value), &entries, "hunter2"),
            "digests from the old salt must not verify under a new one"
        );
    }

    /// Corrupt or truncated config entries must read as "not dismissed".
    #[test]
    fn malformed_entries_never_verify() {
        let salt = new_salt().unwrap();
        let hash = hash_secret("hunter2", &salt, 10).unwrap();

        assert!(!verify_secret("hunter2", "not base64!!", 10, &hash));
        assert!(!verify_secret("hunter2", &salt, 10, "not base64!!"));
        assert!(!verify_secret("hunter2", &salt, 0, &hash));
        assert!(!verify_secret("hunter2", &salt, 10, ""));
    }
}
