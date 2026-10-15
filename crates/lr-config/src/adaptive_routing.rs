//! Opt-in ordering and local dispatch budgets for automatic routes.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateOrdering {
    #[default]
    Priority,
    EstimatedCost,
    ObservedTtft,
    ObservedThroughput,
    LeastInFlight,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdaptiveRoutingConfig {
    pub ordering: CandidateOrdering,
    /// Minimum real observations before using measured time or throughput.
    pub min_samples: usize,
    pub observation_window_secs: u64,
    /// Number of actual upstream dispatches; exclusions do not consume attempts.
    pub max_attempts: Option<usize>,
    pub attempt_timeout_ms: Option<u64>,
    /// Includes classifier, preflight and full streaming lifetime.
    pub total_timeout_ms: Option<u64>,
    /// Passive circuit recovery for infrastructure errors, bounded to 60s.
    pub infrastructure_cooldown: bool,
}
impl Default for AdaptiveRoutingConfig {
    fn default() -> Self {
        Self {
            ordering: CandidateOrdering::Priority,
            min_samples: 5,
            observation_window_secs: 300,
            max_attempts: None,
            attempt_timeout_ms: None,
            total_timeout_ms: None,
            infrastructure_cooldown: false,
        }
    }
}
impl AdaptiveRoutingConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=128).contains(&self.min_samples) {
            return Err("Adaptive routing min_samples must be between 1 and 128".into());
        }
        if !(1..=3600).contains(&self.observation_window_secs) {
            return Err(
                "Adaptive routing observation window must be between 1 and 3600 seconds".into(),
            );
        }
        if self
            .max_attempts
            .is_some_and(|count| !(1..=64).contains(&count))
        {
            return Err("Adaptive routing max_attempts must be between 1 and 64".into());
        }
        for timeout in [self.attempt_timeout_ms, self.total_timeout_ms]
            .into_iter()
            .flatten()
        {
            if !(100..=3_600_000).contains(&timeout) {
                return Err("Routing timeout must be between 100ms and one hour".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_config_preserves_priority_and_unbounded_dispatches() {
        let config: AdaptiveRoutingConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config, AdaptiveRoutingConfig::default());
        config.validate().unwrap();
        assert!(config.max_attempts.is_none());
        assert!(config.total_timeout_ms.is_none());
    }
    #[test]
    fn rejects_unbounded_or_zero_limits() {
        for value in [
            serde_json::json!({"max_attempts":0}),
            serde_json::json!({"max_attempts":65}),
            serde_json::json!({"attempt_timeout_ms":99}),
            serde_json::json!({"total_timeout_ms":3600001}),
            serde_json::json!({"min_samples":0}),
            serde_json::json!({"observation_window_secs":3601}),
        ] {
            assert!(serde_json::from_value::<AdaptiveRoutingConfig>(value)
                .unwrap()
                .validate()
                .is_err());
        }
        assert!(serde_json::from_str::<AdaptiveRoutingConfig>("{\"unknown\":true}").is_err());
    }
}
