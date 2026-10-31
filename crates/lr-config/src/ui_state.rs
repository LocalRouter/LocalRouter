//! Durable UI state: small JSON values the desktop UI remembers between runs
//! (which dashboard sections are folded, where a panel is docked, a filter).
//!
//! It lives in its own file, `ui_state.json` beside `settings.yaml`, rather than
//! in the main config, so a UI toggle never rewrites the config and never emits
//! `config-changed`. Every change is written at once (temp file, fsync, rename),
//! because the WebView's own `localStorage` buffers writes and loses the recent
//! ones when the app process ends without a graceful WebKit shutdown.

use parking_lot::Mutex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// File name of the store inside the config directory.
pub const UI_STATE_FILE: &str = "ui_state.json";
/// Longest accepted key, in characters.
pub const UI_STATE_MAX_KEY_CHARS: usize = 200;
/// Largest accepted value, in bytes of compact JSON.
pub const UI_STATE_MAX_VALUE_BYTES: usize = 16 * 1024;
/// Most keys the store holds.
pub const UI_STATE_MAX_KEYS: usize = 500;

/// A JSON map of UI state persisted to one file.
pub struct UiStateStore {
    path: PathBuf,
    entries: Mutex<BTreeMap<String, Value>>,
}

impl UiStateStore {
    /// Opens the store at `path`, loading what is there.
    ///
    /// A missing file is an empty store. An unreadable or malformed file is set
    /// aside as `<file>.corrupt` (so nothing is silently overwritten) and the
    /// store starts empty. Entries that break the bounds are dropped on load.
    pub fn open(path: PathBuf) -> Self {
        let entries = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<BTreeMap<String, Value>>(&bytes) {
                Ok(map) => {
                    let mut kept = BTreeMap::new();
                    for (key, value) in map {
                        if kept.len() >= UI_STATE_MAX_KEYS {
                            break;
                        }
                        if validate(&key, &value).is_ok() && !value.is_null() {
                            kept.insert(key, value);
                        }
                    }
                    kept
                }
                Err(e) => {
                    tracing::warn!(
                        "UI state file {} is malformed ({e}); starting empty",
                        path.display()
                    );
                    let aside = path.with_extension("json.corrupt");
                    if let Err(e) = fs::rename(&path, &aside) {
                        tracing::warn!("Could not set aside malformed UI state file: {e}");
                    }
                    BTreeMap::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => {
                tracing::warn!(
                    "Could not read UI state file {}: {e}; starting empty",
                    path.display()
                );
                BTreeMap::new()
            }
        };
        Self {
            path,
            entries: Mutex::new(entries),
        }
    }

    /// Opens `ui_state.json` in `config_dir`.
    pub fn open_in(config_dir: &Path) -> Self {
        Self::open(config_dir.join(UI_STATE_FILE))
    }

    /// Path of the backing file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every stored entry.
    pub fn get_all(&self) -> BTreeMap<String, Value> {
        self.entries.lock().clone()
    }

    /// Stores `value` under `key` and writes the file before returning.
    /// A JSON `null` removes the key. Setting a key to the value it already
    /// holds writes nothing.
    pub fn set(&self, key: &str, value: Value) -> Result<(), String> {
        validate(key, &value)?;
        // The lock is held across the write so concurrent sets reach the file
        // in the order they changed the map.
        let mut entries = self.entries.lock();
        if value.is_null() {
            if entries.remove(key).is_none() {
                return Ok(());
            }
        } else {
            match entries.get(key) {
                Some(existing) if *existing == value => return Ok(()),
                Some(_) => {}
                None if entries.len() >= UI_STATE_MAX_KEYS => {
                    return Err(format!(
                        "UI state is full ({UI_STATE_MAX_KEYS} keys); refusing new key"
                    ));
                }
                None => {}
            }
            entries.insert(key.to_string(), value);
        }
        let written = write_atomically(&self.path, &entries);
        if let Err(e) = written {
            // Keep memory and disk in agreement: reload what the file holds.
            *entries = Self::open(self.path.clone()).entries.into_inner();
            return Err(e);
        }
        Ok(())
    }
}

fn validate(key: &str, value: &Value) -> Result<(), String> {
    if key.is_empty() {
        return Err("UI state key must not be empty".into());
    }
    if key.chars().count() > UI_STATE_MAX_KEY_CHARS {
        return Err(format!(
            "UI state key is longer than {UI_STATE_MAX_KEY_CHARS} characters"
        ));
    }
    let size = serde_json::to_vec(value).map_err(|e| e.to_string())?.len();
    if size > UI_STATE_MAX_VALUE_BYTES {
        return Err(format!(
            "UI state value for {key:?} is {size} bytes; the limit is {UI_STATE_MAX_VALUE_BYTES}"
        ));
    }
    Ok(())
}

/// Writes `entries` to a temp file in the same directory, syncs it and
/// renames it over `path`, so a reader sees either the old file or the new
/// one, never a partial write.
fn write_atomically(path: &Path, entries: &BTreeMap<String, Value>) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| format!("UI state path {} has no parent", path.display()))?;
    fs::create_dir_all(dir).map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
    let bytes = serde_json::to_vec_pretty(entries).map_err(|e| e.to_string())?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(UI_STATE_FILE);
    let temp = dir.join(format!(".{file_name}.tmp"));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::File::create(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&temp);
        return Err(format!("Failed to write {}: {e}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn store(dir: &TempDir) -> UiStateStore {
        UiStateStore::open_in(dir.path())
    }

    #[test]
    fn missing_file_is_an_empty_store() {
        let dir = TempDir::new().unwrap();
        assert!(store(&dir).get_all().is_empty());
        assert!(!dir.path().join(UI_STATE_FILE).exists());
    }

    #[test]
    fn values_persist_and_reload() {
        let dir = TempDir::new().unwrap();
        let s = store(&dir);
        s.set("dashboard.traffic.collapsed", json!(true)).unwrap();
        s.set("monitor.filter", json!({"status": "error", "search": null}))
            .unwrap();
        s.set("monitor.detailSize.bottom", json!(42)).unwrap();

        // Written before `set` returned: a fresh store sees every value.
        let reloaded = store(&dir).get_all();
        assert_eq!(reloaded.len(), 3);
        assert_eq!(reloaded["dashboard.traffic.collapsed"], json!(true));
        assert_eq!(reloaded["monitor.filter"]["status"], json!("error"));
        assert_eq!(reloaded["monitor.detailSize.bottom"], json!(42));

        s.set("dashboard.traffic.collapsed", json!(false)).unwrap();
        assert_eq!(
            store(&dir).get_all()["dashboard.traffic.collapsed"],
            json!(false)
        );
    }

    #[test]
    fn null_removes_a_key() {
        let dir = TempDir::new().unwrap();
        let s = store(&dir);
        s.set("a", json!(1)).unwrap();
        s.set("a", Value::Null).unwrap();
        assert!(store(&dir).get_all().is_empty());
        // Removing an absent key is a no-op, not an error.
        s.set("absent", Value::Null).unwrap();
    }

    #[test]
    fn key_bounds_are_enforced() {
        let dir = TempDir::new().unwrap();
        let s = store(&dir);
        assert!(s.set("", json!(1)).is_err());
        let longest = "k".repeat(UI_STATE_MAX_KEY_CHARS);
        s.set(&longest, json!(1)).unwrap();
        assert!(s.set(&format!("{longest}k"), json!(1)).is_err());
        // Characters, not bytes: 200 multi-byte characters are accepted.
        s.set(&"é".repeat(UI_STATE_MAX_KEY_CHARS), json!(1))
            .unwrap();
    }

    #[test]
    fn value_size_is_bounded() {
        let dir = TempDir::new().unwrap();
        let s = store(&dir);
        // A JSON string costs its content plus two quotes.
        let fits = "x".repeat(UI_STATE_MAX_VALUE_BYTES - 2);
        s.set("fits", json!(fits)).unwrap();
        let err = s.set("too_big", json!(format!("{fits}x"))).unwrap_err();
        assert!(err.contains("limit"), "{err}");
        assert!(!store(&dir).get_all().contains_key("too_big"));
    }

    #[test]
    fn key_count_is_bounded_but_existing_keys_stay_writable() {
        let dir = TempDir::new().unwrap();
        let s = store(&dir);
        for i in 0..UI_STATE_MAX_KEYS {
            s.set(&format!("k{i}"), json!(i)).unwrap();
        }
        assert!(s.set("one_more", json!(true)).is_err());
        s.set("k0", json!("updated")).unwrap();
        s.set("k1", Value::Null).unwrap();
        s.set("one_more", json!(true)).unwrap();
        let reloaded = store(&dir).get_all();
        assert_eq!(reloaded.len(), UI_STATE_MAX_KEYS);
        assert_eq!(reloaded["k0"], json!("updated"));
    }

    #[test]
    fn write_is_atomic_and_leaves_no_temp_file() {
        let dir = TempDir::new().unwrap();
        let s = store(&dir);
        s.set("a", json!(1)).unwrap();
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec![UI_STATE_FILE.to_string()]);

        // A stale temp file from an interrupted write never shadows the real one.
        fs::write(
            dir.path().join(format!(".{UI_STATE_FILE}.tmp")),
            b"{\"a\": 2",
        )
        .unwrap();
        assert_eq!(store(&dir).get_all()["a"], json!(1));
        s.set("b", json!(2)).unwrap();
        let reloaded = store(&dir).get_all();
        assert_eq!(reloaded["a"], json!(1));
        assert_eq!(reloaded["b"], json!(2));
    }

    #[test]
    fn failed_write_reports_an_error_and_keeps_memory_consistent() {
        let dir = TempDir::new().unwrap();
        let s = store(&dir);
        s.set("a", json!(1)).unwrap();
        // Make the destination a directory so the rename fails.
        fs::remove_file(s.path()).unwrap();
        fs::create_dir(s.path()).unwrap();
        assert!(s.set("b", json!(2)).is_err());
        assert!(!s.get_all().contains_key("b"));
    }

    #[test]
    fn malformed_file_is_set_aside_and_store_starts_empty() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(UI_STATE_FILE), b"not json").unwrap();
        let s = store(&dir);
        assert!(s.get_all().is_empty());
        assert!(dir.path().join("ui_state.json.corrupt").exists());
        s.set("a", json!(1)).unwrap();
        assert_eq!(store(&dir).get_all()["a"], json!(1));
    }

    #[test]
    fn out_of_bounds_entries_are_dropped_on_load() {
        let dir = TempDir::new().unwrap();
        let long_key = "k".repeat(UI_STATE_MAX_KEY_CHARS + 1);
        let file = json!({ "ok": 1, long_key: 2, "big": "x".repeat(UI_STATE_MAX_VALUE_BYTES) });
        fs::write(dir.path().join(UI_STATE_FILE), file.to_string()).unwrap();
        let loaded = store(&dir).get_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["ok"], json!(1));
    }

    #[test]
    fn unchanged_value_does_not_rewrite_the_file() {
        let dir = TempDir::new().unwrap();
        let s = store(&dir);
        s.set("a", json!(1)).unwrap();
        // Replace the file behind the store's back; an unchanged set must not touch it.
        fs::write(s.path(), b"{\"a\": 1, \"marker\": true}").unwrap();
        s.set("a", json!(1)).unwrap();
        assert_eq!(store(&dir).get_all()["marker"], json!(true));
    }
}
