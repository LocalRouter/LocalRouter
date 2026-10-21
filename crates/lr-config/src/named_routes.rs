//! Bounded route snapshots. Authorization always stays on the live strategy.
use crate::{AdaptiveRoutingConfig, AutoModelConfig, RoutingPolicy, SessionRoutingConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RouteCondition {
    pub client_ids: Vec<String>,
    pub has_tools: Option<bool>,
    pub has_images: Option<bool>,
    pub min_context_chars: Option<u32>,
    pub max_context_chars: Option<u32>,
    pub max_estimated_cost_usd: Option<f64>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteBranch {
    pub route: String,
    pub weight: u16,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteRule {
    pub id: String,
    #[serde(default)]
    pub condition: RouteCondition,
    pub branches: Vec<RouteBranch>,
}
impl RouteRule {
    pub fn validate(&self, routes: &HashSet<&str>) -> Result<(), String> {
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err("Rule IDs need 1–64 letters, numbers, underscores or hyphens".into());
        }
        let c = &self.condition;
        if c.client_ids.len() > 64
            || c.client_ids
                .iter()
                .any(|id| id.is_empty() || id.len() > 128)
            || c.min_context_chars
                .zip(c.max_context_chars)
                .is_some_and(|(min, max)| min > max)
            || c.max_estimated_cost_usd
                .is_some_and(|cost| !cost.is_finite() || cost < 0.0)
        {
            return Err("Invalid bounded route condition".into());
        }
        if self.branches.is_empty()
            || self.branches.len() > 16
            || self
                .branches
                .iter()
                .any(|branch| branch.weight == 0 || !routes.contains(branch.route.as_str()))
            || self
                .branches
                .iter()
                .map(|branch| u32::from(branch.weight))
                .sum::<u32>()
                != 10_000
        {
            return Err(
                "Rule branches must name existing routes and have positive weights totaling 10000"
                    .into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteSnapshot {
    pub model_name: String,
    pub prioritized_models: Vec<(String, String)>,
    pub available_models: Vec<(String, String)>,
    pub routing_policy: Option<RoutingPolicy>,
    pub adaptive_routing: AdaptiveRoutingConfig,
    pub session_routing: SessionRoutingConfig,
}
impl From<&AutoModelConfig> for RouteSnapshot {
    fn from(auto: &AutoModelConfig) -> Self {
        Self {
            model_name: auto.model_name.clone(),
            prioritized_models: auto.prioritized_models.clone(),
            available_models: auto.available_models.clone(),
            routing_policy: auto.routing_policy.clone(),
            adaptive_routing: auto.adaptive_routing.clone(),
            session_routing: auto.session_routing.clone(),
        }
    }
}
impl RouteSnapshot {
    pub fn apply(&self, auto: &mut AutoModelConfig) {
        // Deliberately preserve live auto permission and migration fields.
        auto.model_name = self.model_name.clone();
        auto.prioritized_models = self.prioritized_models.clone();
        auto.available_models = self.available_models.clone();
        auto.routing_policy = self.routing_policy.clone();
        auto.adaptive_routing = self.adaptive_routing.clone();
        auto.session_routing = self.session_routing.clone();
    }
    /// Shared content never replaces a binding's client-facing model alias.
    pub fn apply_content(&self, auto: &mut AutoModelConfig) {
        let alias = auto.model_name.clone();
        self.apply(auto);
        auto.model_name = alias;
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.model_name.trim().is_empty()
            || self.model_name.len() > 128
            || self.model_name.chars().any(char::is_control)
            || self.prioritized_models.len() > 256
            || self.available_models.len() > 256
            || self
                .prioritized_models
                .iter()
                .chain(&self.available_models)
                .any(|(p, m)| {
                    p.is_empty()
                        || p.len() > 128
                        || m.is_empty()
                        || m.len() > 512
                        || p == "localrouter"
                })
        {
            return Err("Invalid route alias or bounded model pool".into());
        }
        self.adaptive_routing.validate()?;
        self.session_routing.validate()?;
        if let Some(policy) = &self.routing_policy {
            policy.validate()?;
        }
        if serde_json::to_vec(self)
            .map_err(|_| "Invalid snapshot")?
            .len()
            > 128 * 1024
        {
            return Err("Route snapshot exceeds 128 KiB".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteRevision {
    pub revision: u64,
    pub label: String,
    pub published_at: i64,
    pub snapshot: RouteSnapshot,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NamedRouteVersions {
    pub draft: Option<RouteSnapshot>,
    pub active_revision: Option<u64>,
    pub revisions: Vec<RouteRevision>,
    pub next_revision: u64,
}
impl NamedRouteVersions {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.next_revision > u32::MAX as u64 {
            return Err("Route revision counter exceeds the supported range".into());
        }
        if self.revisions.len() > 32
            || serde_json::to_vec(self)
                .map_err(|_| "Invalid route history")?
                .len()
                > 1024 * 1024
        {
            return Err("Route history is limited to 32 revisions and 1 MiB; explicitly remove an inactive revision first".into());
        }
        if let Some(draft) = &self.draft {
            draft.validate()?;
        }
        let mut ids = HashSet::new();
        for revision in &self.revisions {
            if revision.revision == 0
                || revision.revision >= self.next_revision
                || !ids.insert(revision.revision)
                || revision.label.len() > 128
                || revision.label.chars().any(char::is_control)
                || !(0..=253402300799).contains(&revision.published_at)
            {
                return Err("Invalid route revision metadata".into());
            }
            revision.snapshot.validate()?;
        }
        if self
            .active_revision
            .is_some_and(|revision| !ids.contains(&revision))
        {
            return Err("Active route revision does not exist".into());
        }
        Ok(())
    }
    pub fn active_matches(&self, auto: &AutoModelConfig) -> bool {
        self.active_revision
            .and_then(|id| self.revisions.iter().find(|r| r.revision == id))
            .is_some_and(|revision| revision.snapshot == RouteSnapshot::from(auto))
    }
    pub fn publish(
        &mut self,
        auto: &mut AutoModelConfig,
        label: String,
        now: i64,
    ) -> Result<u64, String> {
        let snapshot = self.draft.clone().ok_or("Save a draft before publishing")?;
        snapshot.validate()?;
        if snapshot.model_name != auto.model_name {
            return Err("Published revisions must retain the current named-route alias".into());
        }
        let revision = self.next_revision.max(1);
        let next = revision
            .checked_add(1)
            .ok_or("Route revision counter exhausted")?;
        let mut updated = self.clone();
        updated.draft = None;
        updated.next_revision = next;
        updated.active_revision = Some(revision);
        updated.revisions.push(RouteRevision {
            revision,
            label,
            published_at: now,
            snapshot: snapshot.clone(),
        });
        updated.validate()?;
        snapshot.apply(auto);
        *self = updated;
        Ok(revision)
    }
    pub fn rollback(&mut self, auto: &mut AutoModelConfig, revision: u64) -> Result<(), String> {
        let snapshot = &self
            .revisions
            .iter()
            .find(|r| r.revision == revision)
            .ok_or("Unknown route revision")?
            .snapshot;
        snapshot.validate()?;
        if snapshot.model_name != auto.model_name {
            return Err("Rollback cannot change the current route alias".into());
        }
        snapshot.apply(auto);
        self.active_revision = Some(revision);
        Ok(())
    }
}

impl crate::Strategy {
    pub fn active_named_revision(&self) -> Option<u64> {
        if self.routing_profile.is_some() {
            return None;
        }
        self.auto_config
            .as_ref()
            .filter(|auto| self.route_versions.active_matches(auto))
            .and(self.route_versions.active_revision)
    }
    /// Drafts and inactive history are editorial state, not request policy.
    pub fn routing_identity(&self) -> serde_json::Value {
        let mut value = serde_json::json!(self);
        let object = value.as_object_mut().expect("strategy object");
        object.remove("route_versions");
        if let Some(revision) = self.active_named_revision() {
            object.insert("_named_route_revision".into(), serde_json::json!(revision));
        }
        value
    }
    pub fn routing_scope(&self) -> serde_json::Value {
        let mut scope =
            serde_json::json!({"auto":self.auto_config,"revision":self.active_named_revision()});
        if let Some(binding) = &self.routing_profile {
            scope["profile"] = serde_json::json!(binding);
        }
        scope
    }
}
impl crate::AppConfig {
    pub fn routing_identity(&self) -> serde_json::Value {
        let mut value = serde_json::json!(self);
        value
            .as_object_mut()
            .expect("config object")
            .remove("routing_profiles");
        value["strategies"] = serde_json::Value::Array(
            self.strategies
                .iter()
                .map(crate::Strategy::routing_identity)
                .collect(),
        );
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn auto() -> AutoModelConfig {
        AutoModelConfig {
            permission: crate::PermissionState::Off,
            prioritized_models: vec![("provider".into(), "old".into())],
            ..Default::default()
        }
    }
    #[test]
    fn drafts_and_inactive_history_do_not_invalidate_live_routing_identity() {
        let mut strategy = crate::Strategy::new("identity".into());
        let original = strategy.routing_identity();
        strategy.route_versions.draft =
            Some(RouteSnapshot::from(strategy.auto_config.as_ref().unwrap()));
        assert_eq!(strategy.routing_identity(), original);
        strategy
            .route_versions
            .publish(strategy.auto_config.as_mut().unwrap(), "first".into(), 100)
            .unwrap();
        let published = strategy.routing_identity();
        assert_ne!(published, original);
        strategy.route_versions.draft =
            Some(RouteSnapshot::from(strategy.auto_config.as_ref().unwrap()));
        assert_eq!(strategy.routing_identity(), published);
        strategy.route_versions.revisions[0].label = "Editorial label".into();
        assert_eq!(strategy.routing_identity(), published);
    }
    #[test]
    fn publication_and_rollback_preserve_live_permissions_and_draft() {
        let mut auto = auto();
        let original = auto.clone();
        let mut versions = NamedRouteVersions::default();
        let mut snapshot = RouteSnapshot::from(&auto);
        snapshot.prioritized_models = vec![("provider".into(), "new".into())];
        versions.draft = Some(snapshot);
        assert_eq!(versions.publish(&mut auto, "first".into(), 100).unwrap(), 1);
        assert_eq!(auto.permission, crate::PermissionState::Off);
        assert!(versions.active_matches(&auto));
        assert!(versions.draft.is_none());
        versions.draft = Some(RouteSnapshot::from(&original));
        assert_eq!(
            versions.publish(&mut auto, "second".into(), 200).unwrap(),
            2
        );
        let mut draft = RouteSnapshot::from(&auto);
        draft.prioritized_models.clear();
        versions.draft = Some(draft.clone());
        versions.rollback(&mut auto, 1).unwrap();
        assert_eq!(versions.draft, Some(draft));
        assert_eq!(auto.prioritized_models[0].1, "new");
        assert_eq!(auto.permission, crate::PermissionState::Off);
        auto.available_models
            .push(("provider".into(), "changed".into()));
        assert!(!versions.active_matches(&auto));
        assert!(versions.rollback(&mut auto, 999).is_err());
        assert_eq!(versions.active_revision, Some(1));
    }
    #[test]
    fn invalid_publication_is_atomic_and_bounded_without_silent_history_pruning() {
        let mut auto = auto();
        let mut versions = NamedRouteVersions::default();
        let mut snapshot = RouteSnapshot::from(&auto);
        snapshot.model_name = "different".into();
        versions.draft = Some(snapshot);
        let before = versions.clone();
        let active = auto.clone();
        assert!(versions.publish(&mut auto, "label".into(), 100).is_err());
        assert_eq!(versions, before);
        assert_eq!(auto, active);
        versions.draft = Some(RouteSnapshot::from(&auto));
        for i in 0..32 {
            versions.draft = Some(RouteSnapshot::from(&auto));
            versions
                .publish(&mut auto, format!("revision {i}"), 100)
                .unwrap();
        }
        versions.draft = Some(RouteSnapshot::from(&auto));
        let before = versions.clone();
        assert!(versions.publish(&mut auto, "overflow".into(), 100).is_err());
        assert_eq!(versions, before);
        assert_eq!(versions.revisions.len(), 32);
        versions.next_revision = u64::MAX;
        assert!(versions.validate().is_err());
    }
    #[test]
    fn rules_require_valid_destinations_unique_ids_and_complete_weights() {
        let mut policy:RoutingPolicy=serde_json::from_value(serde_json::json!({"enabled":true,"options":[{"id":"a","description":"A","models":[]},{"id":"b","description":"B","models":[]}],"default_route":"a","rules":[{"id":"split","branches":[{"route":"a","weight":5000},{"route":"b","weight":5000}]}]})).unwrap();
        policy.validate().unwrap();
        assert_eq!(
            policy.setup_issue(),
            Some(crate::RoutingPolicyIssue::NeedsDestinationModels)
        );
        policy.rules[0].branches[0].weight = 4999;
        assert!(policy.validate().is_err());
        policy.rules[0].branches[0].weight = 5000;
        policy.rules[0].branches[0].route = "missing".into();
        assert!(policy.validate().is_err());
        policy.rules[0].branches[0].route = "a".into();
        policy.rules.push(policy.rules[0].clone());
        assert!(policy.validate().is_err());
        policy.rules.pop();
        policy.rules[0].condition.min_context_chars = Some(100);
        policy.rules[0].condition.max_context_chars = Some(99);
        assert!(policy.validate().is_err());
        assert!(serde_json::from_value::<RouteRule>(
            serde_json::json!({"id":"unknown","branches":[],"untrusted":true})
        )
        .is_err());
        assert!(serde_json::from_value::<RouteSnapshot>(
            serde_json::json!({"model_name":"route","permission":"allow"})
        )
        .is_err());
    }
    #[test]
    fn missing_route_history_defaults_without_changing_old_strategy_behavior() {
        let strategy = crate::Strategy::new("old".into());
        let mut value = serde_json::to_value(&strategy).unwrap();
        value.as_object_mut().unwrap().remove("route_versions");
        let decoded: crate::Strategy = serde_json::from_value(value).unwrap();
        assert!(decoded.route_versions.is_default());
        assert_eq!(decoded.auto_config, strategy.auto_config);
    }
}
