//! Local drafts/publication with compare-and-swap; no model permissions are changed.
use lr_config::{NamedRouteVersions, RouteSnapshot, Strategy};
use lr_server::state::AppState;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{Emitter, State};

#[derive(Clone, Serialize)]
pub struct NamedRouteStatus {
    pub strategy_id: String,
    pub fingerprint: String,
    pub active: RouteSnapshot,
    pub published_revision: Option<u64>,
    pub modified_since_publication: bool,
    pub versions: NamedRouteVersions,
}
#[derive(Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum NamedRouteAction {
    SaveDraft { snapshot: Box<RouteSnapshot> },
    DiscardDraft,
    Publish { label: String },
    Rollback { revision: u64 },
    RemoveRevision { revision: u64 },
}
fn status(strategy: &Strategy) -> Result<NamedRouteStatus, String> {
    let auto = strategy
        .auto_config
        .as_ref()
        .ok_or("This strategy has no auto router")?;
    let matches = strategy.route_versions.active_matches(auto);
    Ok(NamedRouteStatus {
        strategy_id: strategy.id.clone(),
        fingerprint: lr_router::explanation::fingerprint(serde_json::json!(strategy)),
        active: RouteSnapshot::from(auto),
        published_revision: if matches {
            strategy.route_versions.active_revision
        } else {
            None
        },
        modified_since_publication: strategy.route_versions.active_revision.is_some() && !matches,
        versions: strategy.route_versions.clone(),
    })
}
#[tauri::command]
pub async fn get_named_route(
    strategy_id: String,
    state: State<'_, Arc<AppState>>,
) -> Result<NamedRouteStatus, String> {
    let config = state.config_manager.get();
    status(
        config
            .strategies
            .iter()
            .find(|s| s.id == strategy_id)
            .ok_or("Strategy not found")?,
    )
}
#[tauri::command]
pub async fn mutate_named_route(
    strategy_id: String,
    expected_fingerprint: String,
    operation: NamedRouteAction,
    state: State<'_, Arc<AppState>>,
    app: tauri::AppHandle,
) -> Result<NamedRouteStatus, String> {
    let result = apply_mutation(
        &state.config_manager,
        &strategy_id,
        &expected_fingerprint,
        operation,
    )?;
    state.config_manager.save().await.map_err(|error| {
        format!("Route changed in this session but could not be saved: {error}")
    })?;
    let _ = app.emit("strategies-changed", ());
    let _ = app.emit("clients-changed", ());
    Ok(result)
}

fn apply_mutation(
    config_manager: &lr_config::ConfigManager,
    strategy_id: &str,
    expected_fingerprint: &str,
    operation: NamedRouteAction,
) -> Result<NamedRouteStatus, String> {
    let config = config_manager.get();
    let before = config
        .strategies
        .iter()
        .find(|s| s.id == strategy_id)
        .ok_or("Strategy not found")?;
    if status(before)?.fingerprint != expected_fingerprint {
        return Err("Route changed; reload before editing or publishing".into());
    }
    if before.routing_profile.is_some() {
        return Err("Edit the shared routing profile or make an independent copy first".into());
    }
    let mut updated = before.clone();
    let auto = updated
        .auto_config
        .as_mut()
        .ok_or("This strategy has no auto router")?;
    match operation {
        NamedRouteAction::SaveDraft { snapshot } => {
            snapshot.validate()?;
            updated.route_versions.draft = Some(*snapshot);
        }
        NamedRouteAction::DiscardDraft => updated.route_versions.draft = None,
        NamedRouteAction::Publish { label } => {
            updated
                .route_versions
                .publish(auto, label, chrono::Utc::now().timestamp())?;
        }
        NamedRouteAction::Rollback { revision } => {
            updated.route_versions.rollback(auto, revision)?
        }
        NamedRouteAction::RemoveRevision { revision } => {
            if updated.route_versions.active_revision == Some(revision) {
                return Err("Cannot remove the active revision".into());
            }
            if !updated
                .route_versions
                .revisions
                .iter()
                .any(|r| r.revision == revision)
            {
                return Err("Unknown route revision".into());
            }
            updated
                .route_versions
                .revisions
                .retain(|r| r.revision != revision);
        }
    }
    updated.route_versions.validate()?;
    let mut replaced = false;
    config_manager
        .update(|config| {
            if let Some(current) = config.strategies.iter_mut().find(|s| s.id == strategy_id) {
                if current == before {
                    *current = updated.clone();
                    replaced = true;
                }
            }
        })
        .map_err(|error| error.to_string())?;
    if !replaced {
        return Err("Route changed; reload before editing or publishing".into());
    }
    status(&updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manager() -> (lr_config::ConfigManager, String) {
        let strategy = Strategy::new("named".into());
        let id = strategy.id.clone();
        let config = lr_config::AppConfig {
            strategies: vec![strategy],
            ..Default::default()
        };
        (
            lr_config::ConfigManager::new(
                config,
                std::env::temp_dir()
                    .join(format!("named-route-test-{}.yaml", uuid::Uuid::new_v4())),
            ),
            id,
        )
    }
    #[test]
    fn stale_publications_never_overwrite_live_permissions_or_active_routing() {
        let (config, id) = manager();
        let original = status(&config.get().strategies[0]).unwrap();
        let mut draft = original.active.clone();
        draft.prioritized_models = vec![("provider".into(), "new".into())];
        let saved = apply_mutation(
            &config,
            &id,
            &original.fingerprint,
            NamedRouteAction::SaveDraft {
                snapshot: Box::new(draft),
            },
        )
        .unwrap();
        assert_eq!(saved.active, original.active);
        assert!(apply_mutation(
            &config,
            &id,
            &original.fingerprint,
            NamedRouteAction::Publish {
                label: "stale".into()
            }
        )
        .is_err());
        config
            .update(|cfg| {
                cfg.strategies[0].model_permissions.global = lr_config::PermissionState::Off
            })
            .unwrap();
        assert!(apply_mutation(
            &config,
            &id,
            &saved.fingerprint,
            NamedRouteAction::Publish {
                label: "stale permissions".into()
            }
        )
        .is_err());
        let current = status(&config.get().strategies[0]).unwrap();
        let published = apply_mutation(
            &config,
            &id,
            &current.fingerprint,
            NamedRouteAction::Publish {
                label: "reviewed".into(),
            },
        )
        .unwrap();
        assert_eq!(published.published_revision, Some(1));
        assert_eq!(
            config.get().strategies[0].model_permissions.global,
            lr_config::PermissionState::Off
        );
        assert!(apply_mutation(
            &config,
            &id,
            &published.fingerprint,
            NamedRouteAction::RemoveRevision { revision: 1 }
        )
        .is_err());
    }
    #[test]
    fn rollback_and_explicit_history_cleanup_preserve_other_draft_and_live_state() {
        let (config, id) = manager();
        let mut current = status(&config.get().strategies[0]).unwrap();
        for index in 1..=2 {
            let mut snapshot = current.active.clone();
            snapshot.prioritized_models = vec![("provider".into(), format!("model-{index}"))];
            current = apply_mutation(
                &config,
                &id,
                &current.fingerprint,
                NamedRouteAction::SaveDraft {
                    snapshot: Box::new(snapshot),
                },
            )
            .unwrap();
            current = apply_mutation(
                &config,
                &id,
                &current.fingerprint,
                NamedRouteAction::Publish {
                    label: format!("version {index}"),
                },
            )
            .unwrap();
        }
        let saved_draft = current.active.clone();
        current = apply_mutation(
            &config,
            &id,
            &current.fingerprint,
            NamedRouteAction::SaveDraft {
                snapshot: Box::new(saved_draft.clone()),
            },
        )
        .unwrap();
        current = apply_mutation(
            &config,
            &id,
            &current.fingerprint,
            NamedRouteAction::Rollback { revision: 1 },
        )
        .unwrap();
        assert_eq!(current.versions.draft, Some(saved_draft));
        assert_eq!(current.active.prioritized_models[0].1, "model-1");
        current = apply_mutation(
            &config,
            &id,
            &current.fingerprint,
            NamedRouteAction::RemoveRevision { revision: 2 },
        )
        .unwrap();
        assert_eq!(current.versions.revisions.len(), 1);
        assert_eq!(current.versions.next_revision, 3);
        let active = current.active.clone();
        current = apply_mutation(
            &config,
            &id,
            &current.fingerprint,
            NamedRouteAction::DiscardDraft,
        )
        .unwrap();
        assert!(current.versions.draft.is_none());
        assert_eq!(current.active, active);
        assert!(apply_mutation(
            &config,
            &id,
            &current.fingerprint,
            NamedRouteAction::RemoveRevision { revision: 999 }
        )
        .is_err());
        let roundtrip: lr_config::AppConfig =
            serde_json::from_value(serde_json::to_value(config.get()).unwrap()).unwrap();
        assert_eq!(
            roundtrip.strategies[0].route_versions.active_revision,
            Some(1)
        );
    }
    #[test]
    fn concurrent_draft_edits_accept_exactly_one_expected_fingerprint() {
        let (config, id) = manager();
        let config = Arc::new(config);
        let original = status(&config.get().strategies[0]).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let handles = (0..2)
            .map(|index| {
                let config = config.clone();
                let id = id.clone();
                let original = original.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut snapshot = original.active;
                    snapshot.prioritized_models =
                        vec![("provider".into(), format!("model-{index}"))];
                    barrier.wait();
                    apply_mutation(
                        &config,
                        &id,
                        &original.fingerprint,
                        NamedRouteAction::SaveDraft {
                            snapshot: Box::new(snapshot),
                        },
                    )
                    .is_ok()
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            handles
                .into_iter()
                .map(|handle| usize::from(handle.join().unwrap()))
                .sum::<usize>(),
            1
        );
        assert_eq!(
            config.get().strategies[0]
                .auto_config
                .as_ref()
                .unwrap()
                .prioritized_models
                .len(),
            0
        );
    }
}
