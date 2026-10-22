//! Shared routing content is projected atomically into existing strategies.
//! Permissions, aliases and spending controls always remain on the strategy.
use crate::{AppConfig, AutoModelConfig, NamedRouteVersions, RouteSnapshot, Strategy};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingProfileBinding {
    pub profile_id: String,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingProfile {
    pub id: String,
    pub name: String,
    pub active: RouteSnapshot,
    pub versions: NamedRouteVersions,
}

impl RoutingProfile {
    pub fn new(
        id: String,
        name: String,
        mut snapshot: RouteSnapshot,
        now: i64,
    ) -> Result<Self, String> {
        // Aliases are local to each binding, never part of the shared namespace.
        snapshot.model_name = "auto".into();
        let mut auto = AutoModelConfig::default();
        let mut versions = NamedRouteVersions {
            draft: Some(snapshot),
            ..Default::default()
        };
        versions.publish(&mut auto, "Initial version".into(), now)?;
        let profile = Self {
            id,
            name,
            active: RouteSnapshot::from(&auto),
            versions,
        };
        profile.validate()?;
        Ok(profile)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || self.id.len() > 128
            || self.name.trim().is_empty()
            || self.name.len() > 128
            || self.name.chars().any(char::is_control)
        {
            return Err("Profile needs a valid ID and a name of 1–128 characters".into());
        }
        self.active.validate()?;
        self.versions.validate()?;
        if self.active.model_name != "auto"
            || self
                .versions
                .draft
                .as_ref()
                .is_some_and(|s| s.model_name != "auto")
            || self
                .versions
                .revisions
                .iter()
                .any(|r| r.snapshot.model_name != "auto")
        {
            return Err("Shared profiles cannot change client model aliases".into());
        }
        let mut auto = AutoModelConfig::default();
        self.active.apply(&mut auto);
        if !self.versions.active_matches(&auto) {
            return Err("Profile active routing must match its published revision".into());
        }
        Ok(())
    }
    pub fn revision(&self) -> u64 {
        self.versions.active_revision.unwrap_or(0)
    }
    pub fn publish(&mut self, label: String, now: i64) -> Result<(), String> {
        let mut auto = AutoModelConfig::default();
        self.active.apply(&mut auto);
        self.versions.publish(&mut auto, label, now)?;
        self.active = RouteSnapshot::from(&auto);
        Ok(())
    }
    pub fn rollback(&mut self, revision: u64) -> Result<(), String> {
        let mut auto = AutoModelConfig::default();
        self.active.apply(&mut auto);
        self.versions.rollback(&mut auto, revision)?;
        self.active = RouteSnapshot::from(&auto);
        Ok(())
    }
    pub fn apply(&self, strategy: &mut Strategy) -> Result<(), String> {
        let auto = strategy
            .auto_config
            .as_mut()
            .ok_or("Enable auto routing before attaching a profile")?;
        self.active.apply_content(auto);
        strategy.routing_profile = Some(RoutingProfileBinding {
            profile_id: self.id.clone(),
            revision: self.revision(),
        });
        Ok(())
    }
}

impl AppConfig {
    pub fn validate_routing_profiles(&self) -> Result<(), String> {
        if self.routing_profiles.len() > 128 {
            return Err("At most 128 routing profiles are supported".into());
        }
        let mut ids = HashSet::new();
        for profile in &self.routing_profiles {
            profile.validate()?;
            if !ids.insert(&profile.id) {
                return Err("Duplicate routing profile ID".into());
            }
        }
        for strategy in &self.strategies {
            if let Some(binding) = &strategy.routing_profile {
                let profile = self
                    .routing_profiles
                    .iter()
                    .find(|p| p.id == binding.profile_id)
                    .ok_or(
                        "Attached routing profile does not exist; detach it before removing it",
                    )?;
                let mut projected = strategy.clone();
                profile.apply(&mut projected)?;
                if projected.routing_profile != strategy.routing_profile
                    || projected.auto_config != strategy.auto_config
                {
                    return Err("Shared routing changed; edit the profile or make an independent copy before changing routing models".into());
                }
            }
        }
        Ok(())
    }
    /// Used inside the configuration write transaction. All live readers keep
    /// consuming the same existing strategy shape, including older app versions.
    pub fn project_routing_profile(&mut self, id: &str) -> Result<(), String> {
        let profile = self
            .routing_profiles
            .iter()
            .find(|p| p.id == id)
            .ok_or("Profile not found")?
            .clone();
        profile.validate()?;
        for strategy in &mut self.strategies {
            if strategy
                .routing_profile
                .as_ref()
                .is_some_and(|b| b.profile_id == id)
            {
                profile.apply(strategy)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> RoutingProfile {
        let auto = AutoModelConfig {
            prioritized_models: vec![("provider".into(), "free".into())],
            ..Default::default()
        };
        RoutingProfile::new(
            "coding".into(),
            "Coding".into(),
            RouteSnapshot::from(&auto),
            1,
        )
        .unwrap()
    }
    #[test]
    fn legacy_configs_keep_identical_routing_and_omit_additive_fields() {
        let strategy = Strategy::new("legacy".into());
        let value = serde_json::to_value(&strategy).unwrap();
        assert!(value.get("routing_profile").is_none());
        let decoded: Strategy = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, strategy);
        let config = AppConfig {
            strategies: vec![strategy],
            ..Default::default()
        };
        let json = serde_json::to_value(&config).unwrap();
        assert!(json.get("routing_profiles").is_none());
        let roundtrip: AppConfig = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, config);
        roundtrip.validate_routing_profiles().unwrap();
    }
    #[test]
    fn publish_rollback_detach_preserve_alias_permissions_limits_and_consent() {
        let profile = profile();
        let mut a = Strategy::new("a".into());
        let mut b = Strategy::new("b".into());
        a.auto_config.as_mut().unwrap().model_name = "chatbot".into();
        a.auto_config.as_mut().unwrap().permission = crate::PermissionState::Ask;
        a.model_permissions.global = crate::PermissionState::Off;
        a.rate_limits.push(crate::StrategyRateLimit {
            limit_type: crate::RateLimitType::Requests,
            value: 10.0,
            time_window: crate::RateLimitTimeWindow::Minute,
            enabled: true,
        });
        a.free_tier_only = true;
        a.free_tier_fallback = crate::FreeTierFallback::Ask;
        b.free_tier_fallback = crate::FreeTierFallback::Allow;
        let before_a = a.clone();
        let before_b = b.clone();
        profile.apply(&mut a).unwrap();
        profile.apply(&mut b).unwrap();
        let mut config = AppConfig {
            strategies: vec![a, b, Strategy::new("unrelated".into())],
            routing_profiles: vec![profile],
            ..Default::default()
        };
        config.validate_routing_profiles().unwrap();
        let unrelated = config.strategies[2].clone();
        let original_identity = config.routing_identity();
        let mut draft = config.routing_profiles[0].active.clone();
        draft.prioritized_models[0].1 = "paid".into();
        config.routing_profiles[0].versions.draft = Some(draft);
        assert_eq!(config.routing_identity(), original_identity);
        config.routing_profiles[0].publish("new".into(), 2).unwrap();
        config.project_routing_profile("coding").unwrap();
        config.validate_routing_profiles().unwrap();
        assert_ne!(config.routing_identity(), original_identity);
        assert_eq!(
            config.strategies[0]
                .auto_config
                .as_ref()
                .unwrap()
                .prioritized_models[0]
                .1,
            "paid"
        );
        assert_eq!(
            config.strategies[0].model_permissions,
            before_a.model_permissions
        );
        assert_eq!(config.strategies[0].rate_limits, before_a.rate_limits);
        assert_eq!(
            config.strategies[0].free_tier_fallback,
            before_a.free_tier_fallback
        );
        assert!(config.strategies[0].free_tier_only);
        assert_eq!(
            config.strategies[0]
                .auto_config
                .as_ref()
                .unwrap()
                .model_name,
            "chatbot"
        );
        assert_eq!(
            config.strategies[0]
                .auto_config
                .as_ref()
                .unwrap()
                .permission,
            crate::PermissionState::Ask
        );
        assert_eq!(
            config.strategies[1].model_permissions,
            before_b.model_permissions
        );
        assert_eq!(
            config.strategies[1].free_tier_fallback,
            before_b.free_tier_fallback
        );
        assert_eq!(config.strategies[2], unrelated);
        config.strategies[1].routing_profile = None;
        let independent = config.strategies[1].clone();
        config.routing_profiles[0].rollback(1).unwrap();
        config.project_routing_profile("coding").unwrap();
        config.validate_routing_profiles().unwrap();
        assert_eq!(config.strategies[1], independent);
        assert_eq!(
            config.strategies[0]
                .auto_config
                .as_ref()
                .unwrap()
                .prioritized_models[0]
                .1,
            "free"
        );
        config.routing_profiles.clear();
        assert!(config.validate_routing_profiles().is_err());
    }
    #[test]
    fn attached_inline_edits_missing_bindings_and_alias_changes_are_rejected() {
        let mut s = Strategy::new("s".into());
        let p = profile();
        p.apply(&mut s).unwrap();
        let mut config = AppConfig {
            strategies: vec![s],
            routing_profiles: vec![p],
            ..Default::default()
        };
        config.strategies[0]
            .auto_config
            .as_mut()
            .unwrap()
            .prioritized_models
            .clear();
        assert!(config.validate_routing_profiles().is_err());
        config.project_routing_profile("coding").unwrap();
        config.strategies[0]
            .auto_config
            .as_mut()
            .unwrap()
            .model_name = "local-alias".into();
        config.validate_routing_profiles().unwrap();
        config.routing_profiles[0].versions.draft = Some(config.routing_profiles[0].active.clone());
        config.routing_profiles[0]
            .versions
            .draft
            .as_mut()
            .unwrap()
            .model_name = "other".into();
        assert!(config.validate_routing_profiles().is_err());
    }
}
