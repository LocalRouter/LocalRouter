//! User-owned routing policies. Route labels are independent of model names.
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingPolicyMode {
    Semantic,
    ClientMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutingOption {
    pub id: String,
    pub description: String,
    /// Empty means the auto router's ordinary prioritized list.
    #[serde(default)]
    pub models: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModeRouteRule {
    pub mode: String,
    pub route: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RoutingPolicy {
    pub version: u32,
    pub enabled: bool,
    pub mode: RoutingPolicyMode,
    pub decision_model: Option<(String, String)>,
    pub question: String,
    pub options: Vec<RoutingOption>,
    pub default_route: String,
    pub mode_rules: Vec<ModeRouteRule>,
    pub min_probability: f64,
    pub timeout_ms: u64,
    pub max_context_chars: usize,
    pub history_messages: usize,
}

impl Default for RoutingPolicy {
    fn default() -> Self {
        Self {
            version: 1,
            enabled: false,
            mode: RoutingPolicyMode::Semantic,
            decision_model: None,
            question: "What kind of work does the latest user request ask for?".into(),
            options: vec![],
            default_route: "general".into(),
            mode_rules: vec![],
            min_probability: 0.0,
            timeout_ms: 3000,
            max_context_chars: 2000,
            history_messages: 2,
        }
    }
}

// Decode the previous strong/weak object once; never carry its calibrated
// threshold onto a different classifier. No decision model is auto-selected.
impl<'de> Deserialize<'de> for RoutingPolicy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(default)]
        struct Wire {
            version: u32,
            enabled: bool,
            mode: RoutingPolicyMode,
            decision_model: Option<(String, String)>,
            question: String,
            options: Vec<RoutingOption>,
            default_route: String,
            mode_rules: Vec<ModeRouteRule>,
            min_probability: f64,
            timeout_ms: u64,
            max_context_chars: usize,
            history_messages: usize,
            weak_models: Option<Vec<(String, String)>>,
        }
        impl Default for Wire {
            fn default() -> Self {
                let p = RoutingPolicy::default();
                Self {
                    version: p.version,
                    enabled: p.enabled,
                    mode: p.mode,
                    decision_model: p.decision_model,
                    question: p.question,
                    options: p.options,
                    default_route: p.default_route,
                    mode_rules: p.mode_rules,
                    min_probability: p.min_probability,
                    timeout_ms: p.timeout_ms,
                    max_context_chars: p.max_context_chars,
                    history_messages: p.history_messages,
                    weak_models: None,
                }
            }
        }
        let w = Wire::deserialize(deserializer)?;
        if let Some(weak) = w.weak_models {
            return Ok(Self {
                enabled: w.enabled,
                question: "Which configured route fits this request? Choose thorough for difficult reasoning or complex work; choose routine for straightforward tasks.".into(),
                options: vec![
                    RoutingOption { id: "thorough".into(), description: "Thorough reasoning and complex tasks (previous strong pool)".into(), models: vec![] },
                    RoutingOption { id: "routine".into(), description: "Straightforward routine tasks (previous weak pool)".into(), models: weak },
                ],
                default_route: "thorough".into(),
                ..Self::default()
            });
        }
        Ok(Self {
            version: w.version,
            enabled: w.enabled,
            mode: w.mode,
            decision_model: w.decision_model,
            question: w.question,
            options: w.options,
            default_route: w.default_route,
            mode_rules: w.mode_rules,
            min_probability: w.min_probability,
            timeout_ms: w.timeout_ms,
            max_context_chars: w.max_context_chars,
            history_messages: w.history_messages,
        })
    }
}

impl RoutingPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("Unsupported routing policy version".into());
        }
        if !(2..=32).contains(&self.options.len()) {
            return Err("A routing policy needs 2 to 32 options".into());
        }
        if self.question.trim().is_empty() || self.question.len() > 8000 {
            return Err("Enter a routing question (up to 8000 bytes)".into());
        }
        let mut ids = HashSet::new();
        for o in &self.options {
            if o.id.is_empty()
                || o.id.len() > 64
                || !o
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                || !ids.insert(o.id.as_str())
            {
                return Err(
                    "Route IDs must be unique, 1–64 letters, numbers, underscores or hyphens"
                        .into(),
                );
            }
            if o.description.trim().is_empty() || o.description.len() > 4000 {
                return Err("Each route needs a description (up to 4000 bytes)".into());
            }
            if o.models.len() > 64 || o.models.iter().any(|(p, m)| p.is_empty() || m.is_empty()) {
                return Err("Invalid route model list".into());
            }
        }
        if !ids.contains(self.default_route.as_str()) {
            return Err("Default route must name an option".into());
        }
        let mut modes = HashSet::new();
        for r in &self.mode_rules {
            if r.mode.trim().is_empty()
                || r.mode.len() > 64
                || !modes.insert(r.mode.as_str())
                || !ids.contains(r.route.as_str())
            {
                return Err("Mode rules need unique modes and valid route IDs".into());
            }
        }
        if !self.min_probability.is_finite() || !(0.0..=1.0).contains(&self.min_probability) {
            return Err("Minimum probability must be between 0 and 1".into());
        }
        if !(100..=30000).contains(&self.timeout_ms)
            || !(128..=64000).contains(&self.max_context_chars)
            || self.history_messages > 20
        {
            return Err("Invalid routing timeout or context limits".into());
        }
        if self
            .decision_model
            .as_ref()
            .is_some_and(|(p, m)| p.is_empty() || m.is_empty() || p == "localrouter")
        {
            return Err("Choose a specific provider decision model".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auto_model_is_available_with_policy_destinations_and_no_priority_list() {
        let mut auto: crate::AutoModelConfig = serde_json::from_value(serde_json::json!({
            "prioritized_models": [],
            "routing_policy": {"enabled": true, "weak_models": [["provider", "model"]]}
        }))
        .unwrap();
        assert!(auto.prioritized_models.is_empty());
        assert!(auto.has_chat_candidates());
        auto.routing_policy.as_mut().unwrap().enabled = false;
        assert!(!auto.has_chat_candidates());
        auto.prioritized_models
            .push(("provider".into(), "default".into()));
        assert!(auto.has_chat_candidates());
    }

    #[test]
    fn legacy_policy_preserves_pools_without_reusing_threshold() {
        let auto: crate::AutoModelConfig = serde_json::from_value(serde_json::json!({
            "prioritized_models": [["cloud", "large"]], "routellm_config": {
                "enabled": true, "threshold": 0.3, "weak_models": [["local", "small"]]
            }
        }))
        .unwrap();
        let p = auto.routing_policy.as_ref().unwrap();
        p.validate().unwrap();
        assert!(p.enabled);
        assert_eq!(p.default_route, "thorough");
        assert!(p.decision_model.is_none());
        assert_eq!(p.min_probability, 0.0);
        assert_eq!(p.options[1].models, vec![("local".into(), "small".into())]);
        assert_eq!(
            auto.prioritized_models,
            vec![("cloud".into(), "large".into())]
        );
        let saved = serde_json::to_value(&auto).unwrap();
        assert!(saved.get("routellm_config").is_none());
        assert_eq!(
            serde_json::from_value::<crate::AutoModelConfig>(saved).unwrap(),
            auto
        );
    }
    #[test]
    fn rejects_duplicate_options_and_dangling_mode_rules() {
        let mut p: RoutingPolicy =
            serde_json::from_value(serde_json::json!({"weak_models": [], "enabled": false}))
                .unwrap();
        p.options[1].id = p.options[0].id.clone();
        assert!(p.validate().is_err());
        p.options[1].id = "routine".into();
        p.mode_rules.push(ModeRouteRule {
            mode: "plan".into(),
            route: "missing".into(),
        });
        assert!(p.validate().is_err());
    }
}
