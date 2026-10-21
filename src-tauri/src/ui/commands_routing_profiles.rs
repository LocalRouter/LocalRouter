//! Shared routing publication is atomic and never grants client permissions.
use lr_config::{
    AppConfig, AutoModelConfig, ConfigManager, RouteSnapshot, RoutingProfile, Strategy,
};
use lr_server::state::AppState;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{Emitter, State};

#[derive(Clone, Serialize)]
pub struct RoutingProfileClient {
    pub client_id: String,
    pub client_name: String,
    pub strategy_id: String,
    pub issues: Vec<String>,
}
#[derive(Clone, Serialize)]
pub struct RoutingProfileStatus {
    pub profile: RoutingProfile,
    pub fingerprint: String,
    pub clients: Vec<RoutingProfileClient>,
    pub revision_clients: std::collections::BTreeMap<u64, Vec<RoutingProfileClient>>,
}
#[derive(Clone, Serialize)]
pub struct RoutingProfilesState {
    pub profiles: Vec<RoutingProfileStatus>,
    pub strategy: Option<Strategy>,
    pub strategy_fingerprint: Option<String>,
    pub strategy_clients: Vec<RoutingProfileClient>,
}
#[derive(Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum RoutingProfileAction {
    SaveDraft { snapshot: Box<RouteSnapshot> },
    DiscardDraft,
    Publish { label: String },
    Rollback { revision: u64 },
    RemoveRevision { revision: u64 },
    Rename { name: String },
    Delete,
}
fn fingerprint(value: impl Serialize) -> String {
    lr_router::explanation::fingerprint(
        serde_json::to_value(value).expect("serializable routing state"),
    )
}
fn client_status(
    config: &AppConfig,
    strategy: &Strategy,
    snapshot: Option<&RouteSnapshot>,
) -> Vec<RoutingProfileClient> {
    let mut issues = Vec::new();
    let auto = strategy.auto_config.as_ref();
    if auto.is_none_or(|a| !a.permission.is_enabled()) {
        issues.push("Auto routing is off".into());
    }
    let owned = auto.map(RouteSnapshot::from);
    if let Some(snapshot) = snapshot.or(owned.as_ref()) {
        let mut projected = AutoModelConfig::default();
        snapshot.apply(&mut projected);
        if !projected.has_chat_candidates() {
            issues.push("No default-route or ordinary fallback chat models configured".into());
        }
        let mut candidates = snapshot.prioritized_models.clone();
        if let Some(policy) = snapshot.routing_policy.as_ref().filter(|p| p.enabled) {
            candidates.extend(policy.options.iter().flat_map(|o| o.models.clone()));
            if policy.mode == lr_config::RoutingPolicyMode::Semantic {
                candidates.extend(policy.decision_model.clone());
            }
        }
        candidates.sort();
        candidates.dedup();
        for (provider, model) in candidates {
            if !config
                .providers
                .iter()
                .any(|p| p.name == provider && p.enabled)
            {
                issues.push(format!("Provider unavailable: {provider}"));
            }
            match strategy.model_permissions.resolve_model(&provider, &model) {
                lr_config::PermissionState::Off => {
                    issues.push(format!("Model not permitted: {provider}/{model}"))
                }
                lr_config::PermissionState::Ask => {
                    issues.push(format!("Model requires approval: {provider}/{model}"))
                }
                lr_config::PermissionState::Allow => {}
            }
        }
    }
    issues.sort();
    issues.dedup();
    config
        .clients
        .iter()
        .filter(|c| c.strategy_id == strategy.id)
        .map(|c| RoutingProfileClient {
            client_id: c.id.clone(),
            client_name: c.name.clone(),
            strategy_id: strategy.id.clone(),
            issues: issues.clone(),
        })
        .collect()
}
fn profile_status(config: &AppConfig, profile: &RoutingProfile) -> RoutingProfileStatus {
    let strategies: Vec<_> = config
        .strategies
        .iter()
        .filter(|s| {
            s.routing_profile
                .as_ref()
                .is_some_and(|b| b.profile_id == profile.id)
        })
        .collect();
    // Include proposed draft permissions in the reviewed impact, and bind the
    // token to all affected clients so attachment/publication races are rejected.
    let clients = strategies
        .iter()
        .flat_map(|s| client_status(config, s, profile.versions.draft.as_ref()))
        .collect::<Vec<_>>();
    let providers: Vec<_> = config
        .providers
        .iter()
        .map(|p| (&p.name, p.enabled))
        .collect();
    let revision_clients = profile
        .versions
        .revisions
        .iter()
        .map(|revision| {
            (
                revision.revision,
                strategies
                    .iter()
                    .flat_map(|s| client_status(config, s, Some(&revision.snapshot)))
                    .collect(),
            )
        })
        .collect();
    let affected_clients: Vec<_> = config
        .clients
        .iter()
        .filter(|c| strategies.iter().any(|s| s.id == c.strategy_id))
        .collect();
    RoutingProfileStatus {
        revision_clients,
        fingerprint: fingerprint((&profile, &strategies, &clients, providers, affected_clients)),
        profile: profile.clone(),
        clients,
    }
}
fn state(config: &AppConfig, strategy_id: Option<&str>) -> Result<RoutingProfilesState, String> {
    let strategy = strategy_id
        .map(|id| {
            config
                .strategies
                .iter()
                .find(|s| s.id == id)
                .cloned()
                .ok_or_else(|| "Strategy not found".to_string())
        })
        .transpose()?;
    Ok(RoutingProfilesState {
        profiles: config
            .routing_profiles
            .iter()
            .map(|p| profile_status(config, p))
            .collect(),
        strategy_fingerprint: strategy.as_ref().map(fingerprint),
        strategy_clients: strategy
            .as_ref()
            .map(|s| client_status(config, s, None))
            .unwrap_or_default(),
        strategy,
    })
}
/// A failed operation never publishes a partially changed config. The closure
/// runs under ConfigManager's write lock, including all expected-token checks.
fn transaction<T>(
    manager: &ConfigManager,
    edit: impl FnOnce(&mut AppConfig) -> Result<T, String>,
) -> Result<T, String> {
    let mut result = Err("Routing update did not run".into());
    manager
        .update(|config| {
            let mut candidate = config.clone();
            result = edit(&mut candidate).and_then(|value| {
                candidate.validate_routing_profiles()?;
                *config = candidate;
                Ok(value)
            });
        })
        .map_err(|e| e.to_string())?;
    result
}
fn require_profile(config: &AppConfig, id: &str, expected: &str) -> Result<usize, String> {
    let index = config
        .routing_profiles
        .iter()
        .position(|p| p.id == id)
        .ok_or("Profile not found")?;
    if profile_status(config, &config.routing_profiles[index]).fingerprint != expected {
        return Err(
            "Profile or affected clients changed; reload and review before applying".into(),
        );
    }
    Ok(index)
}
fn mutate(
    manager: &ConfigManager,
    id: &str,
    expected: &str,
    operation: RoutingProfileAction,
) -> Result<RoutingProfilesState, String> {
    transaction(manager, |config| {
        let index = require_profile(config, id, expected)?;
        if matches!(operation, RoutingProfileAction::Delete) {
            if config.strategies.iter().any(|s| {
                s.routing_profile
                    .as_ref()
                    .is_some_and(|b| b.profile_id == id)
                    && config.clients.iter().any(|c| c.strategy_id == s.id)
            }) {
                return Err("Reassign or make independent copies of all attached clients before deleting this profile".into());
            }
            // Retired clients can leave reusable strategies behind. Preserve
            // their effective routing as independent copies instead of leaving
            // an undeletable profile or dangling references.
            for strategy in &mut config.strategies {
                if strategy
                    .routing_profile
                    .as_ref()
                    .is_some_and(|b| b.profile_id == id)
                {
                    strategy.routing_profile = None;
                }
            }
            config.routing_profiles.remove(index);
        } else {
            let profile = &mut config.routing_profiles[index];
            match operation {
                RoutingProfileAction::SaveDraft { snapshot } => {
                    snapshot.validate()?;
                    profile.versions.draft = Some(*snapshot);
                }
                RoutingProfileAction::DiscardDraft => profile.versions.draft = None,
                RoutingProfileAction::Publish { label } => {
                    profile.publish(label, chrono::Utc::now().timestamp())?
                }
                RoutingProfileAction::Rollback { revision } => profile.rollback(revision)?,
                RoutingProfileAction::RemoveRevision { revision } => {
                    if profile.versions.active_revision == Some(revision) {
                        return Err("Cannot remove the active revision".into());
                    }
                    if !profile
                        .versions
                        .revisions
                        .iter()
                        .any(|r| r.revision == revision)
                    {
                        return Err("Unknown profile revision".into());
                    }
                    profile
                        .versions
                        .revisions
                        .retain(|r| r.revision != revision);
                }
                RoutingProfileAction::Rename { name } => profile.name = name,
                RoutingProfileAction::Delete => unreachable!(),
            }
            config.project_routing_profile(id)?;
        }
        state(config, None)
    })
}
fn bind(
    manager: &ConfigManager,
    strategy_id: &str,
    profile_id: Option<&str>,
    expected_strategy: &str,
    expected_profile: Option<&str>,
) -> Result<RoutingProfilesState, String> {
    transaction(manager, |config| {
        let index = config
            .strategies
            .iter()
            .position(|s| s.id == strategy_id)
            .ok_or("Strategy not found")?;
        if fingerprint(&config.strategies[index]) != expected_strategy {
            return Err("Client routing changed; reload before applying".into());
        }
        if let Some(id) = profile_id {
            let p = require_profile(
                config,
                id,
                expected_profile.ok_or("Reload the profile before attaching")?,
            )?;
            config.routing_profiles[p].apply(&mut config.strategies[index])?;
        } else {
            // The already-projected effective routing is the independent copy.
            config.strategies[index].routing_profile = None;
        }
        state(config, Some(strategy_id))
    })
}
async fn persist(
    state: &AppState,
    app: &tauri::AppHandle,
    previous_identity: serde_json::Value,
) -> Result<(), String> {
    // Policy identity is already fingerprinted for sessions and preflight.
    // Clear retained response payloads too, including in-flight insertions.
    if state.config_manager.get().routing_identity() != previous_identity {
        state.router.clear_response_cache(None);
    }
    state
        .config_manager
        .save()
        .await
        .map_err(|e| format!("Routing changed in this session but could not be saved: {e}"))?;
    let _ = app.emit("routing-profiles-changed", ());
    let _ = app.emit("strategies-changed", ());
    let _ = app.emit("clients-changed", ());
    let _ = crate::ui::tray::rebuild_tray_menu(app);
    Ok(())
}
#[tauri::command]
pub async fn get_routing_profiles(
    strategy_id: Option<String>,
    state: State<'_, Arc<AppState>>,
) -> Result<RoutingProfilesState, String> {
    self::state(&state.config_manager.get(), strategy_id.as_deref())
}
#[tauri::command]
pub async fn create_routing_profile(
    name: String,
    source_strategy_id: Option<String>,
    source_profile_id: Option<String>,
    expected_fingerprint: Option<String>,
    state: State<'_, Arc<AppState>>,
    app: tauri::AppHandle,
) -> Result<RoutingProfilesState, String> {
    let previous_identity = state.config_manager.get().routing_identity();
    let result = transaction(&state.config_manager, |config| {
        if source_strategy_id.is_some() && source_profile_id.is_some() {
            return Err("Choose one routing source".into());
        }
        let snapshot = if let Some(id) = &source_strategy_id {
            let strategy = config
                .strategies
                .iter()
                .find(|s| &s.id == id)
                .ok_or("Strategy not found")?;
            if expected_fingerprint.as_deref() != Some(fingerprint(strategy).as_str()) {
                return Err("Client routing changed; reload before copying".into());
            }
            RouteSnapshot::from(
                strategy
                    .auto_config
                    .as_ref()
                    .ok_or("This strategy has no auto router")?,
            )
        } else if let Some(id) = &source_profile_id {
            let i = require_profile(
                config,
                id,
                expected_fingerprint
                    .as_deref()
                    .ok_or("Reload before copying")?,
            )?;
            config.routing_profiles[i].active.clone()
        } else {
            RouteSnapshot::from(&AutoModelConfig::default())
        };
        config.routing_profiles.push(RoutingProfile::new(
            uuid::Uuid::new_v4().to_string(),
            name,
            snapshot,
            chrono::Utc::now().timestamp(),
        )?);
        self::state(config, source_strategy_id.as_deref())
    })?;
    persist(&state, &app, previous_identity).await?;
    Ok(result)
}
#[tauri::command]
pub async fn mutate_routing_profile(
    profile_id: String,
    expected_fingerprint: String,
    operation: RoutingProfileAction,
    state: State<'_, Arc<AppState>>,
    app: tauri::AppHandle,
) -> Result<RoutingProfilesState, String> {
    let previous_identity = state.config_manager.get().routing_identity();
    let result = mutate(
        &state.config_manager,
        &profile_id,
        &expected_fingerprint,
        operation,
    )?;
    persist(&state, &app, previous_identity).await?;
    Ok(result)
}
#[tauri::command]
pub async fn bind_routing_profile(
    strategy_id: String,
    profile_id: Option<String>,
    expected_strategy_fingerprint: String,
    expected_profile_fingerprint: Option<String>,
    state: State<'_, Arc<AppState>>,
    app: tauri::AppHandle,
) -> Result<RoutingProfilesState, String> {
    let previous_identity = state.config_manager.get().routing_identity();
    let result = bind(
        &state.config_manager,
        &strategy_id,
        profile_id.as_deref(),
        &expected_strategy_fingerprint,
        expected_profile_fingerprint.as_deref(),
    )?;
    persist(&state, &app, previous_identity).await?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manager() -> ConfigManager {
        let profile = RoutingProfile::new(
            "coding".into(),
            "Coding".into(),
            RouteSnapshot::from(&AutoModelConfig::default()),
            1,
        )
        .unwrap();
        let mut strategy = Strategy::new("client".into());
        strategy.id = "strategy".into();
        ConfigManager::new(
            AppConfig {
                clients: vec![lr_config::Client::new_with_strategy(
                    "Client".into(),
                    "strategy".into(),
                )],
                strategies: vec![strategy],
                routing_profiles: vec![profile],
                ..Default::default()
            },
            std::env::temp_dir().join(format!("profiles-{}.yaml", uuid::Uuid::new_v4())),
        )
    }
    #[test]
    fn stale_attach_publication_delete_and_inline_edits_are_atomic() {
        let m = manager();
        let before = state(&m.get(), Some("strategy")).unwrap();
        let p = &before.profiles[0];
        bind(
            &m,
            "strategy",
            Some("coding"),
            before.strategy_fingerprint.as_ref().unwrap(),
            Some(&p.fingerprint),
        )
        .unwrap();
        assert!(mutate(&m, "coding", &p.fingerprint, RoutingProfileAction::Delete).is_err());
        assert!(bind(
            &m,
            "strategy",
            None,
            before.strategy_fingerprint.as_ref().unwrap(),
            None
        )
        .is_err());
        let current = state(&m.get(), Some("strategy")).unwrap();
        let p = &current.profiles[0];
        assert!(mutate(&m, "coding", &p.fingerprint, RoutingProfileAction::Delete).is_err());
        let saved = m.get();
        assert!(m
            .update(|c| c.strategies[0]
                .auto_config
                .as_mut()
                .unwrap()
                .prioritized_models
                .push(("provider".into(), "wrong".into())))
            .is_err());
        assert_eq!(m.get(), saved);
        bind(
            &m,
            "strategy",
            None,
            current.strategy_fingerprint.as_ref().unwrap(),
            None,
        )
        .unwrap();
        let p = state(&m.get(), None).unwrap().profiles.remove(0);
        mutate(&m, "coding", &p.fingerprint, RoutingProfileAction::Delete).unwrap();
        assert!(m.get().routing_profiles.is_empty());
    }
    #[test]
    fn deleting_profile_after_last_client_retires_keeps_an_independent_strategy() {
        let m = manager();
        let original = state(&m.get(), Some("strategy")).unwrap();
        bind(
            &m,
            "strategy",
            Some("coding"),
            original.strategy_fingerprint.as_ref().unwrap(),
            Some(&original.profiles[0].fingerprint),
        )
        .unwrap();
        m.update(|c| c.clients.clear()).unwrap();
        let before = m.get().strategies[0].auto_config.clone();
        let current = state(&m.get(), None).unwrap();
        mutate(
            &m,
            "coding",
            &current.profiles[0].fingerprint,
            RoutingProfileAction::Delete,
        )
        .unwrap();
        let config = m.get();
        assert!(config.routing_profiles.is_empty());
        assert!(config.strategies[0].routing_profile.is_none());
        assert_eq!(config.strategies[0].auto_config, before);
    }
    #[test]
    fn concurrent_profile_edits_accept_one_token() {
        let manager = Arc::new(manager());
        let token = state(&manager.get(), None).unwrap().profiles[0]
            .fingerprint
            .clone();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|i| {
                let m = manager.clone();
                let token = token.clone();
                let b = barrier.clone();
                std::thread::spawn(move || {
                    b.wait();
                    mutate(
                        &m,
                        "coding",
                        &token,
                        RoutingProfileAction::Rename {
                            name: format!("Coding {i}"),
                        },
                    )
                    .is_ok()
                })
            })
            .collect();
        assert_eq!(
            handles
                .into_iter()
                .map(|h| usize::from(h.join().unwrap()))
                .sum::<usize>(),
            1
        );
    }
}
