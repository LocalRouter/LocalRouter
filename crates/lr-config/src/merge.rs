//! Field-level three-way merge of configurations.
//!
//! Used when the config file changed outside the app while the app holds
//! changes of its own that are not on disk yet: `base` is the configuration
//! the file held when the app last read or wrote it, `ours` the running one,
//! `theirs` the file's new content. A setting changed on one side only takes
//! that side's value; a setting changed on both sides to different values is
//! a conflict.
//!
//! The merge works on the serialized form. Mappings merge key by key. Lists
//! whose items all carry a unique string `id` (clients, providers, MCP
//! servers, strategies, …) merge item by item, so two people editing
//! different clients do not conflict; other lists are single values.

use super::AppConfig;
use lr_types::{AppError, AppResult};
use serde_yaml::{Mapping, Value};
use std::collections::HashMap;

/// The outcome of a merge.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Merged {
    /// The merged configuration. Where the two sides conflict it holds
    /// `theirs` (the file).
    pub config: AppConfig,
    /// Settings changed differently on both sides, as dotted paths
    /// (`server.port`, `clients[<id>].name`).
    pub conflicts: Vec<String>,
}

fn to_value(config: &AppConfig) -> AppResult<Value> {
    serde_yaml::to_value(config)
        .map_err(|e| AppError::Config(format!("Failed to serialize configuration: {e}")))
}

/// Merge `ours` and `theirs`, both derived from `base`.
pub(crate) fn merge3(base: &AppConfig, ours: &AppConfig, theirs: &AppConfig) -> AppResult<Merged> {
    let (base_v, ours_v, theirs_v) = (to_value(base)?, to_value(ours)?, to_value(theirs)?);
    let mut conflicts = Vec::new();
    let merged = merge_value(
        "",
        Some(&base_v),
        Some(&ours_v),
        Some(&theirs_v),
        &mut conflicts,
    )
    .unwrap_or(Value::Null);
    let config: AppConfig = serde_yaml::from_value(merged).map_err(|e| {
        AppError::Config(format!(
            "The changes made in the app and in the file cannot be combined: {e}"
        ))
    })?;
    super::validation::validate_config(&config).map_err(|e| {
        AppError::Config(format!(
            "The changes made in the app and in the file cannot be combined: {e}"
        ))
    })?;
    Ok(Merged { config, conflicts })
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

fn key_name(key: &Value) -> String {
    match key {
        Value::String(s) => s.clone(),
        other => serde_yaml::to_string(other)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

/// Merge one value. `None` means absent (a deleted key or list item).
fn merge_value(
    path: &str,
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
    conflicts: &mut Vec<String>,
) -> Option<Value> {
    if ours == theirs {
        return ours.cloned();
    }
    if ours == base {
        return theirs.cloned();
    }
    if theirs == base {
        return ours.cloned();
    }
    // Changed on both sides, differently.
    match (base, ours, theirs) {
        (None | Some(Value::Mapping(_)), Some(Value::Mapping(o)), Some(Value::Mapping(t))) => {
            let empty = Mapping::new();
            let b = match base {
                Some(Value::Mapping(b)) => b,
                _ => &empty,
            };
            Some(Value::Mapping(merge_mappings(path, b, o, t, conflicts)))
        }
        (_, Some(Value::Sequence(o)), Some(Value::Sequence(t))) => {
            let b = match base {
                Some(Value::Sequence(b)) => Some(b.as_slice()),
                None => Some(&[][..]),
                _ => None,
            };
            if let (Some(b), Some(o_ids), Some(t_ids)) = (b, ids_of(o), ids_of(t)) {
                if let Some(b_ids) = ids_of(b) {
                    return Some(Value::Sequence(merge_keyed(
                        path, b, &b_ids, o, &o_ids, t, &t_ids, conflicts,
                    )));
                }
            }
            conflicts.push(path.to_string());
            theirs.cloned()
        }
        _ => {
            conflicts.push(path.to_string());
            theirs.cloned()
        }
    }
}

fn merge_mappings(
    path: &str,
    base: &Mapping,
    ours: &Mapping,
    theirs: &Mapping,
    conflicts: &mut Vec<String>,
) -> Mapping {
    let mut keys: Vec<&Value> = theirs.keys().collect();
    for key in ours.keys().chain(base.keys()) {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let mut merged = Mapping::new();
    for key in keys {
        let child = join(path, &key_name(key));
        if let Some(value) = merge_value(
            &child,
            base.get(key),
            ours.get(key),
            theirs.get(key),
            conflicts,
        ) {
            merged.insert(key.clone(), value);
        }
    }
    merged
}

/// The `id` of every item, if every item is a mapping with a unique string
/// `id`.
fn ids_of(items: &[Value]) -> Option<Vec<String>> {
    let mut ids = Vec::with_capacity(items.len());
    for item in items {
        let id = item.as_mapping()?.get("id")?.as_str()?.to_string();
        if ids.contains(&id) {
            return None;
        }
        ids.push(id);
    }
    Some(ids)
}

fn merge_keyed(
    path: &str,
    base: &[Value],
    base_ids: &[String],
    ours: &[Value],
    ours_ids: &[String],
    theirs: &[Value],
    theirs_ids: &[String],
    conflicts: &mut Vec<String>,
) -> Vec<Value> {
    let index = |ids: &[String]| -> HashMap<String, usize> {
        ids.iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect()
    };
    let (b, o, t) = (index(base_ids), index(ours_ids), index(theirs_ids));
    // The file's order, then items only the app added, in its order.
    let mut order: Vec<&String> = theirs_ids.iter().collect();
    order.extend(ours_ids.iter().filter(|id| !t.contains_key(*id)));
    // Items both sides deleted are in neither list and stay deleted.
    let mut merged = Vec::new();
    for id in order {
        let item = merge_value(
            &format!("{path}[{id}]"),
            b.get(id).map(|&i| &base[i]),
            o.get(id).map(|&i| &ours[i]),
            t.get(id).map(|&i| &theirs[i]),
            conflicts,
        );
        merged.extend(item);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Client, LlmMode, Strategy};

    fn client(id: &str, name: &str) -> Client {
        let mut c = Client::new_with_strategy(name.to_string(), format!("s-{id}"));
        c.id = id.to_string();
        c
    }

    fn strategy(id: &str, name: &str) -> Strategy {
        Strategy::new_for_client_with_id(format!("s-{id}"), id.to_string(), name.to_string())
    }

    fn base() -> AppConfig {
        AppConfig {
            clients: vec![client("a", "Alpha"), client("b", "Beta")],
            strategies: vec![strategy("a", "Alpha"), strategy("b", "Beta")],
            ..Default::default()
        }
    }

    fn add(config: &mut AppConfig, id: &str, name: &str) {
        config.clients.push(client(id, name));
        config.strategies.push(strategy(id, name));
    }

    #[test]
    fn changes_to_different_settings_are_combined() {
        let base = base();
        let mut ours = base.clone();
        ours.server.enable_cors = !base.server.enable_cors;
        let mut theirs = base.clone();
        theirs.server.port = 4999;

        let merged = merge3(&base, &ours, &theirs).unwrap();
        assert!(merged.conflicts.is_empty());
        assert_eq!(merged.config.server.port, 4999);
        assert_eq!(merged.config.server.enable_cors, ours.server.enable_cors);
    }

    #[test]
    fn the_same_setting_changed_differently_is_a_conflict_won_by_the_file() {
        let base = base();
        let mut ours = base.clone();
        ours.server.port = 4001;
        let mut theirs = base.clone();
        theirs.server.port = 4002;

        let merged = merge3(&base, &ours, &theirs).unwrap();
        assert_eq!(merged.conflicts, vec!["server.port".to_string()]);
        assert_eq!(merged.config.server.port, 4002);
    }

    #[test]
    fn the_same_change_on_both_sides_is_not_a_conflict() {
        let base = base();
        let mut ours = base.clone();
        ours.server.port = 4001;
        let theirs = ours.clone();
        let merged = merge3(&base, &ours, &theirs).unwrap();
        assert!(merged.conflicts.is_empty());
        assert_eq!(merged.config, ours);
    }

    #[test]
    fn list_items_with_ids_merge_one_by_one() {
        let base = base();
        // The app renames a and adds c; the file changes b and adds d.
        let mut ours = base.clone();
        ours.clients[0].name = "Alpha 2".into();
        add(&mut ours, "c", "Gamma");
        let mut theirs = base.clone();
        theirs.clients[1].llm_mode = LlmMode::Proxy;
        add(&mut theirs, "d", "Delta");

        let merged = merge3(&base, &ours, &theirs).unwrap();
        assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
        let names: Vec<(&str, &str)> = merged
            .config
            .clients
            .iter()
            .map(|c| (c.id.as_str(), c.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("a", "Alpha 2"),
                ("b", "Beta"),
                ("d", "Delta"),
                ("c", "Gamma")
            ]
        );
        assert_eq!(merged.config.clients[1].llm_mode, LlmMode::Proxy);
    }

    #[test]
    fn deleting_an_item_the_other_side_left_alone_deletes_it() {
        let base = base();
        let mut ours = base.clone();
        ours.clients.retain(|c| c.id != "b");
        ours.strategies.retain(|s| s.id != "s-b");
        let mut theirs = base.clone();
        theirs.clients[0].name = "Alpha 2".into();

        let merged = merge3(&base, &ours, &theirs).unwrap();
        assert!(merged.conflicts.is_empty());
        let ids: Vec<&str> = merged
            .config
            .clients
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(ids, ["a"]);
        assert_eq!(merged.config.clients[0].name, "Alpha 2");
    }

    #[test]
    fn deleting_an_item_the_other_side_changed_is_a_conflict() {
        let base = base();
        let mut ours = base.clone();
        ours.clients.retain(|c| c.id != "b");
        let mut theirs = base.clone();
        theirs.clients[1].name = "Beta 2".into();

        let merged = merge3(&base, &ours, &theirs).unwrap();
        assert_eq!(merged.conflicts, vec!["clients[b]".to_string()]);
        // The file wins: b is kept with its new name.
        assert!(merged.config.clients.iter().any(|c| c.name == "Beta 2"));
    }

    #[test]
    fn lists_without_ids_are_single_values() {
        let base = base();
        let mut ours = base.clone();
        ours.skills.paths = vec!["/a".into()];
        let mut theirs = base.clone();
        theirs.skills.paths = vec!["/b".into()];
        let merged = merge3(&base, &ours, &theirs).unwrap();
        assert_eq!(merged.conflicts, vec!["skills.paths".to_string()]);
        assert_eq!(merged.config.skills.paths, vec!["/b".to_string()]);
    }
}
