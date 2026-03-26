//! Negative capability cache for endpoint-model compatibility
//!
//! Caches (provider, model, endpoint_type) → unsupported with TTL,
//! so the router can skip known-incompatible models without retrying them.

use std::time::{Duration, Instant};

use dashmap::DashMap;
use lr_providers::EndpointType;

/// Key for the negative cache
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    provider: String,
    model: String,
    endpoint: EndpointType,
}

/// Cache that remembers which (provider, model) pairs don't support
/// specific endpoint types. Entries expire after a configurable TTL.
pub struct EndpointCapabilityCache {
    entries: DashMap<CacheKey, Instant>,
    ttl: Duration,
}

impl EndpointCapabilityCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            entries: DashMap::new(),
            ttl,
        }
    }

    /// Record that a (provider, model) does not support the given endpoint type
    pub fn record_unsupported(&self, provider: &str, model: &str, endpoint: EndpointType) {
        let key = CacheKey {
            provider: provider.to_string(),
            model: model.to_string(),
            endpoint,
        };
        self.entries.insert(key, Instant::now());
    }

    /// Check if a (provider, model) is known to not support the given endpoint.
    /// Returns true if the model is known-unsupported (should skip).
    pub fn is_known_unsupported(
        &self,
        provider: &str,
        model: &str,
        endpoint: EndpointType,
    ) -> bool {
        let key = CacheKey {
            provider: provider.to_string(),
            model: model.to_string(),
            endpoint,
        };
        if let Some(entry) = self.entries.get(&key) {
            if entry.elapsed() < self.ttl {
                return true;
            }
            // Expired — remove it
            drop(entry);
            self.entries.remove(&key);
        }
        false
    }

    /// Remove expired entries (call periodically if desired)
    pub fn cleanup_expired(&self) {
        self.entries
            .retain(|_, inserted_at| inserted_at.elapsed() < self.ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_check() {
        let cache = EndpointCapabilityCache::new(Duration::from_secs(3600));
        assert!(!cache.is_known_unsupported("openai", "gpt-4o", EndpointType::Embedding));

        cache.record_unsupported("openai", "gpt-4o", EndpointType::Embedding);
        assert!(cache.is_known_unsupported("openai", "gpt-4o", EndpointType::Embedding));

        // Different endpoint is not affected
        assert!(!cache.is_known_unsupported("openai", "gpt-4o", EndpointType::Chat));

        // Different model is not affected
        assert!(!cache.is_known_unsupported(
            "openai",
            "text-embedding-3-small",
            EndpointType::Embedding
        ));
    }

    #[test]
    fn test_ttl_expiry() {
        let cache = EndpointCapabilityCache::new(Duration::from_millis(1));
        cache.record_unsupported("openai", "gpt-4o", EndpointType::Embedding);

        // Wait for expiry
        std::thread::sleep(Duration::from_millis(5));
        assert!(!cache.is_known_unsupported("openai", "gpt-4o", EndpointType::Embedding));
    }

    #[test]
    fn test_cleanup_expired() {
        let cache = EndpointCapabilityCache::new(Duration::from_millis(1));
        cache.record_unsupported("openai", "gpt-4o", EndpointType::Embedding);
        cache.record_unsupported("openai", "gpt-4o", EndpointType::Chat);

        std::thread::sleep(Duration::from_millis(5));
        cache.cleanup_expired();
        assert!(cache.entries.is_empty());
    }
}
