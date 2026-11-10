//! Three-way reconciliation for credential snapshots. Missing baselines never
//! authorize replacing an unread durable value with a partial memory snapshot.
use lr_types::{AppError, AppResult};
use serde_json::Value;
use std::collections::BTreeSet;

fn conflict() -> AppError {
    AppError::Storage("Credential recovery conflict: unread durable data was preserved".into())
}
pub(crate) fn reconcile(
    base: Option<Option<&str>>,
    remote: Option<&str>,
    local: Option<&str>,
) -> AppResult<Option<String>> {
    if local.is_none() {
        return Ok(None);
    } // Explicit deletion/tombstone.
    if remote == local || base == Some(remote) {
        return Ok(local.map(str::to_owned));
    }
    if base == Some(local) {
        return Ok(remote.map(str::to_owned));
    }
    let Some(remote) = remote else {
        return Ok(local.map(str::to_owned));
    };
    let remote: Value = serde_json::from_str(remote).map_err(|_| conflict())?;
    let local: Value =
        serde_json::from_str(local.expect("local exists")).map_err(|_| conflict())?;
    let base = base
        .flatten()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|_| conflict())?;
    merge(base.as_ref(), Some(&remote), Some(&local)).map(|v| v.map(|v| v.to_string()))
}
fn merge(
    base: Option<&Value>,
    remote: Option<&Value>,
    local: Option<&Value>,
) -> AppResult<Option<Value>> {
    if local == remote || remote == base {
        return Ok(local.cloned());
    }
    if local == base {
        return Ok(remote.cloned());
    }
    if local.is_none() && base.is_some() {
        return Ok(None);
    }
    let (Some(remote), Some(local)) = (remote, local) else {
        return Ok(local.or(remote).cloned());
    };
    if let (Some(remote), Some(local)) = (remote.as_object(), local.as_object()) {
        let base = base.and_then(Value::as_object);
        let keys: BTreeSet<_> = remote
            .keys()
            .chain(local.keys())
            .chain(base.into_iter().flat_map(|b| b.keys()))
            .collect();
        let mut result = serde_json::Map::new();
        for key in keys {
            if let Some(value) = merge(
                base.and_then(|b| b.get(key)),
                remote.get(key),
                local.get(key),
            )? {
                result.insert(key.clone(), value);
            }
        }
        return Ok(Some(Value::Object(result)));
    }
    if let (Some(remote), Some(local)) = (remote.as_array(), local.as_array()) {
        // Registries are string sets. Do not guess how to merge arbitrary lists.
        let set = |v: &[Value]| -> AppResult<BTreeSet<String>> {
            v.iter()
                .map(|v| v.as_str().map(str::to_owned).ok_or_else(conflict))
                .collect()
        };
        let mut result = set(remote)?;
        let local = set(local)?;
        if let Some(base) = base.and_then(Value::as_array) {
            let base = set(base)?;
            for removed in base.difference(&local) {
                result.remove(removed);
            }
        }
        result.extend(local);
        return Ok(Some(Value::Array(
            result.into_iter().map(Value::String).collect(),
        )));
    }
    if base.is_none() {
        if let (Some(a), Some(b)) = (remote.as_u64(), local.as_u64()) {
            return Ok(Some(a.max(b).into()));
        }
        if let (Some(a), Some(b)) = (remote.as_bool(), local.as_bool()) {
            return Ok(Some((a || b).into()));
        }
        return Err(conflict());
    }
    // A local change based on a successfully read baseline is authoritative.
    Ok(Some(local.clone()))
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn unread_owned_records_merge_without_overwriting_originals() {
        let old = json!({"owner":"client-a","entries":{"old":"old secret"}}).to_string();
        let new = json!({"owner":"client-a","entries":{"new":"new secret"}}).to_string();
        let merged = reconcile(None, Some(&old), Some(&new)).unwrap().unwrap();
        let value: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(
            value["entries"],
            json!({"old":"old secret","new":"new secret"})
        );
        assert!(reconcile(
            None,
            Some(&old),
            Some(&json!({"owner":"client-b","entries":{}}).to_string())
        )
        .is_err());
    }
    #[test]
    fn known_deletions_and_unread_registries_are_preserved() {
        let base = json!({"entries":{"purged":"secret","kept":"other"}}).to_string();
        let local = json!({"entries":{"kept":"other"}}).to_string();
        let remote =
            json!({"entries":{"purged":"secret","kept":"other","late":"late"}}).to_string();
        let merged: Value = serde_json::from_str(
            &reconcile(Some(Some(&base)), Some(&remote), Some(&local))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(merged, json!({"entries":{"kept":"other","late":"late"}}));
        assert_eq!(
            reconcile(None, Some("[\"old\"]"), Some("[\"new\"]")).unwrap(),
            Some("[\"new\",\"old\"]".into())
        );
        assert_eq!(reconcile(None, Some(&remote), None).unwrap(), None);
        assert!(reconcile(None, Some("unread-original"), Some("new-original")).is_err());
    }
}
