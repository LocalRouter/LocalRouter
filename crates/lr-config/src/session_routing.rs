//! Optional process-local session routing; no conversation payloads are retained.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionRoutingConfig {
    pub enabled: bool,
    pub classifier_reuse: bool,
    pub ttl_secs: u64,
    pub forward_cache_key: bool,
}
impl Default for SessionRoutingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            classifier_reuse: false,
            ttl_secs: 900,
            forward_cache_key: false,
        }
    }
}
impl SessionRoutingConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=3600).contains(&self.ttl_secs) {
            return Err("Session routing TTL must be between 1 and 3600 seconds".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opt_in_defaults_and_bounded_expiry() {
        let config: SessionRoutingConfig = serde_json::from_str("{}").unwrap();
        assert!(config.is_default());
        assert!(!config.enabled && !config.classifier_reuse && !config.forward_cache_key);
        for ttl in [0, 3601, u64::MAX] {
            assert!(SessionRoutingConfig {
                ttl_secs: ttl,
                ..Default::default()
            }
            .validate()
            .is_err());
        }
        assert!(serde_json::from_str::<SessionRoutingConfig>(r#"{"typo":true}"#).is_err());
    }
}
