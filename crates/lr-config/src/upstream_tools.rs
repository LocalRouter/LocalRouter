//! Explicit authorization for provider-executed retrieval tools.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamToolKind {
    WebSearch,
    WebFetch,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UpstreamToolsPolicy {
    /// Explicit qualified provider/model for ranked searches that omit model.
    #[serde(default)]
    pub default_search_model: Option<String>,
    pub allowed_tools: Vec<UpstreamToolKind>,
    /// A verified per-upstream-request step cap. None deliberately delegates
    /// limits to the provider; it is never represented as a local guarantee.
    pub max_tool_calls: Option<u32>,
}
impl Default for UpstreamToolsPolicy {
    fn default() -> Self {
        Self {
            default_search_model: None,
            allowed_tools: vec![],
            max_tool_calls: Some(8),
        }
    }
}
impl UpstreamToolsPolicy {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.default_search_model.as_ref().is_some_and(|model| {
            model.len() > 512
                || !model.contains('/')
                || model.starts_with('/')
                || model.ends_with('/')
                || model.chars().any(char::is_whitespace)
        }) {
            return Err(
                "Default search model must be a qualified provider/model up to 512 bytes".into(),
            );
        }
        let mut unique = std::collections::HashSet::new();
        if self.allowed_tools.iter().any(|kind| !unique.insert(*kind))
            || self
                .max_tool_calls
                .is_some_and(|count| !(1..=30).contains(&count))
        {
            return Err(
                "Upstream tool kinds must be unique and a step cap must be between 1 and 30".into(),
            );
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_grant_no_remote_tool_permission_and_bounds_are_explicit() {
        let policy: UpstreamToolsPolicy = serde_json::from_str("{}").unwrap();
        assert!(policy.is_default());
        assert!(policy.allowed_tools.is_empty());
        for count in [0, 31, u32::MAX] {
            assert!(UpstreamToolsPolicy {
                max_tool_calls: Some(count),
                ..Default::default()
            }
            .validate()
            .is_err());
        }
        assert!(UpstreamToolsPolicy {
            max_tool_calls: None,
            ..Default::default()
        }
        .validate()
        .is_ok());
        assert!(UpstreamToolsPolicy {
            allowed_tools: vec![UpstreamToolKind::WebSearch; 2],
            ..Default::default()
        }
        .validate()
        .is_err());
    }
}
