//! Engines LocalRouter downloaded itself (only stable-diffusion.cpp, on the
//! user's click).
//!
//! Layout, per recipe:
//!
//! ```text
//! {config_dir}/engines/managed/<recipe>/
//!   current.json            {tag, build, asset, binary, installed_at}
//!   <tag>-<build>/          the extracted release
//! ```
//!
//! `current.json` is written atomically after the release folder is in
//! place, so a failed or cancelled install never leaves a pointer to a
//! half-extracted folder. `binary` is relative to the recipe folder.

use std::io::Write;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::recipes::RecipeId;

/// Name of the pointer file inside a recipe's managed folder.
pub const CURRENT_FILE: &str = "current.json";

/// Contents of `current.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentPointer {
    /// Release tag, e.g. `master-920-2f88688`.
    pub tag: String,
    /// Build name, e.g. `vulkan`.
    pub build: String,
    /// Release asset the install came from.
    pub asset: String,
    /// Executable path relative to the recipe folder, `/`-separated,
    /// e.g. `master-920-2f88688-vulkan/sd-server`.
    pub binary: String,
    pub installed_at: DateTime<Utc>,
}

/// An installed managed engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedInstall {
    pub tag: String,
    pub build: String,
    pub asset: String,
    /// The release folder (`<tag>-<build>`).
    pub dir: PathBuf,
    /// Absolute path of the executable.
    pub binary: PathBuf,
    pub installed_at: DateTime<Utc>,
}

/// `{config_dir}/engines/managed`.
pub fn managed_root() -> Option<PathBuf> {
    lr_utils::paths::config_dir()
        .ok()
        .map(|d| d.join("engines").join("managed"))
}

/// The managed folder for one recipe.
pub fn recipe_dir(recipe: RecipeId) -> Option<PathBuf> {
    managed_root().map(|root| root.join(recipe.as_str()))
}

/// The managed install of `recipe`, if one is installed and its executable
/// exists.
pub fn installed(recipe: RecipeId) -> Option<ManagedInstall> {
    installed_in(&recipe_dir(recipe)?)
}

/// The managed install's executable.
pub fn binary(recipe: RecipeId) -> Option<PathBuf> {
    installed(recipe).map(|i| i.binary)
}

/// Read the install `current.json` in `dir` points at.
pub fn installed_in(dir: &Path) -> Option<ManagedInstall> {
    let text = std::fs::read_to_string(dir.join(CURRENT_FILE)).ok()?;
    let pointer: CurrentPointer = serde_json::from_str(&text).ok()?;
    let rel = safe_relative(&pointer.binary)?;
    let binary = dir.join(&rel);
    if !binary.is_file() {
        return None;
    }
    let top = rel.components().next()?;
    Some(ManagedInstall {
        dir: dir.join(top.as_os_str()),
        binary,
        tag: pointer.tag,
        build: pointer.build,
        asset: pointer.asset,
        installed_at: pointer.installed_at,
    })
}

/// A relative path made only of normal components (no `..`, no root, no
/// drive prefix), or `None`. Accepts `/` and `\` separators.
pub(crate) fn safe_relative(path: &str) -> Option<PathBuf> {
    if path.starts_with(['/', '\\']) {
        return None;
    }
    let mut out = PathBuf::new();
    for part in path.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        // `C:` style prefixes, and anything that is not a plain name.
        if part.contains(':') || part.contains('\0') {
            return None;
        }
        let mut comps = Path::new(part).components();
        match (comps.next(), comps.next()) {
            (Some(Component::Normal(_)), None) => out.push(part),
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// Write `current.json` atomically (temp file + rename).
pub(crate) fn write_current(dir: &Path, pointer: &CurrentPointer) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(pointer).map_err(std::io::Error::other)?;
    let tmp = dir.join(format!(".{CURRENT_FILE}.{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&json)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, dir.join(CURRENT_FILE))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Remove everything in `dir` except `keep` (a release folder name) and
/// `current.json`: older releases and leftovers of interrupted installs.
/// Returns what could not be removed (e.g. a running engine's folder on
/// Windows).
pub(crate) fn remove_other_installs(dir: &Path, keep: &str) -> Vec<(PathBuf, std::io::Error)> {
    let mut failures = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return failures;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == keep || name == CURRENT_FILE {
            continue;
        }
        let path = entry.path();
        let result = match entry.file_type() {
            Ok(t) if t.is_dir() => std::fs::remove_dir_all(&path),
            _ => std::fs::remove_file(&path),
        };
        if let Err(e) = result {
            failures.push((path, e));
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pointer(binary: &str) -> CurrentPointer {
        CurrentPointer {
            tag: "master-920-2f88688".into(),
            build: "vulkan".into(),
            asset: "sd-master-2f88688-bin-win-vulkan-x64.zip".into(),
            binary: binary.into(),
            installed_at: Utc::now(),
        }
    }

    #[test]
    fn safe_relative_rejects_escapes() {
        assert_eq!(
            safe_relative("a/b/sd-server"),
            Some(PathBuf::from("a").join("b").join("sd-server"))
        );
        assert_eq!(
            safe_relative("./a\\b.exe"),
            Some(PathBuf::from("a").join("b.exe"))
        );
        for bad in [
            "",
            ".",
            "../x",
            "a/../../x",
            "a/..",
            "/etc/passwd",
            "\\\\server\\share",
            "C:\\x",
            "C:x",
        ] {
            assert!(safe_relative(bad).is_none(), "{bad} accepted");
        }
    }

    #[test]
    fn current_pointer_round_trips_and_needs_the_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        assert!(installed_in(dir).is_none(), "nothing installed");

        let p = pointer("master-920-2f88688-vulkan/bin/sd-server");
        write_current(dir, &p).unwrap();
        assert!(installed_in(dir).is_none(), "binary missing");

        let bin_dir = dir.join("master-920-2f88688-vulkan").join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("sd-server"), b"x").unwrap();
        let installed = installed_in(dir).unwrap();
        assert_eq!(installed.tag, "master-920-2f88688");
        assert_eq!(installed.build, "vulkan");
        assert_eq!(installed.binary, bin_dir.join("sd-server"));
        assert_eq!(installed.dir, dir.join("master-920-2f88688-vulkan"));

        // No temp files are left behind.
        let names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
    }

    #[test]
    fn pointer_outside_the_folder_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let inner = tmp.path().join("sdcpp");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(tmp.path().join("evil"), b"x").unwrap();
        write_current(&inner, &pointer("../evil")).unwrap();
        assert!(installed_in(&inner).is_none());
        std::fs::write(inner.join(CURRENT_FILE), b"not json").unwrap();
        assert!(installed_in(&inner).is_none());
    }

    #[test]
    fn removes_everything_but_the_current_release() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        for d in ["old-cpu", "new-vulkan", ".tmp-123"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
            std::fs::write(dir.join(d).join("f"), b"x").unwrap();
        }
        write_current(dir, &pointer("new-vulkan/f")).unwrap();
        std::fs::write(dir.join("stray"), b"x").unwrap();
        assert!(remove_other_installs(dir, "new-vulkan").is_empty());
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![CURRENT_FILE.to_string(), "new-vulkan".to_string()]
        );
    }
}
