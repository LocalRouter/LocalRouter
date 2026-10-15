//! Explicit process-local payload retention limits. Disabled unless selected.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResponseCacheConfig {
    pub enabled: bool,
    pub ttl_secs: u64,
    pub max_entries: usize,
    pub max_bytes: usize,
}
impl Default for ResponseCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ttl_secs: 300,
            max_entries: 128,
            max_bytes: 8 * 1024 * 1024,
        }
    }
}
impl ResponseCacheConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=3600).contains(&self.ttl_secs)
            || !(1..=1024).contains(&self.max_entries)
            || !(1024..=32 * 1024 * 1024).contains(&self.max_bytes)
        {
            return Err("Response cache requires TTL 1–3600 seconds, 1–1024 entries and 1 KiB–32 MiB per client".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_legacy_defaults_and_bounded_retention() {
        let config: ResponseCacheConfig = serde_json::from_str("{}").unwrap();
        assert!(!config.enabled);
        assert!(config.validate().is_ok());
        for (ttl, entries, bytes) in [
            (0, 1, 1024),
            (3601, 1, 1024),
            (1, 0, 1024),
            (1, 1025, 1024),
            (1, 1, 1023),
            (1, 1, 33 * 1024 * 1024),
        ] {
            assert!(ResponseCacheConfig {
                enabled: true,
                ttl_secs: ttl,
                max_entries: entries,
                max_bytes: bytes
            }
            .validate()
            .is_err());
        }
    }
}
