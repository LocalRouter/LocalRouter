//! Atomic file write with backup utility
//!
//! Port of Ollama's `writeWithBackup` pattern for safe config file modification.

use std::fs;
use std::path::{Path, PathBuf};

/// Write data to path via temp file + rename, backing up any existing file first.
/// Returns the backup path if one was created.
pub fn write_with_backup(path: &Path, data: &[u8]) -> Result<Option<PathBuf>, String> {
    write_with_backup_in(path, data, &default_backup_dir())
}

/// Shared directory for config backups. Only the newest backups in it are kept.
pub fn default_backup_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("localrouter-backups")
}

/// [`write_with_backup`] with an explicit backup directory, so tests can keep
/// their backups (and the pruning of old ones) out of the user's real directory.
pub fn write_with_backup_in(
    path: &Path,
    data: &[u8],
    backup_dir: &Path,
) -> Result<Option<PathBuf>, String> {
    use std::io::Write;

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create directory {:?}: {}", parent, e))?;

    // Do not overwrite a file that could not be read: the backup is required
    // for a recoverable edit, not an optional best effort on permission errors.
    let existing = match fs::read(path) {
        Ok(existing) => Some(existing),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "Failed to read existing file {:?}: {}",
                path, error
            ))
        }
    };
    if existing.as_deref() == Some(data) {
        return Ok(None);
    }

    let backup_path = if let Some(existing) = existing {
        fs::create_dir_all(backup_dir)
            .map_err(|e| format!("Failed to create backup dir: {}", e))?;
        let filename = path.file_name().unwrap_or_default().to_string_lossy();
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        // Random suffixes prevent same-second writes (including settings files
        // with the same basename) from overwriting an earlier backup.
        let mut backup = tempfile::Builder::new()
            .prefix(&format!("{filename}.{timestamp}."))
            .suffix(".bak")
            .tempfile_in(backup_dir)
            .map_err(|e| format!("Failed to create private backup: {}", e))?;
        backup
            .write_all(&existing)
            .and_then(|()| backup.as_file().sync_all())
            .map_err(|e| format!("Failed to write backup: {}", e))?;
        let (_, backup_path) = backup
            .keep()
            .map_err(|e| format!("Failed to preserve backup: {}", e))?;
        tracing::info!("Backed up {:?} to {:?}", path, backup_path);
        Some(backup_path)
    } else {
        None
    };

    // Configs and backups can contain API keys. NamedTempFile creates them
    // owner-private on Unix before the first byte is written, and cleans up
    // failed writes automatically. Same-directory persistence is atomic.
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("Failed to create private temp file: {}", e))?;
    temp.write_all(data)
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|e| format!("Failed to write temp file: {}", e))?;
    temp.persist(path)
        .map_err(|e| format!("Failed to replace {:?}: {}", path, e.error))?;

    // Prune only after the replacement succeeds so a failed edit cannot
    // destroy recovery points while leaving the original config unchanged.
    if backup_path.is_some() {
        cleanup_old_backups(backup_dir, 10);
    }
    Ok(backup_path)
}

/// Remove old backups, keeping only the most recent `keep` files.
fn cleanup_old_backups(backup_dir: &Path, keep: usize) {
    let mut entries: Vec<_> = match fs::read_dir(backup_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".bak")))
            .collect(),
        Err(_) => return,
    };

    if entries.len() <= keep {
        return;
    }

    // Sort by modification time descending — newest first
    entries.sort_by(|a, b| {
        let a_time = a.metadata().and_then(|m| m.modified()).ok();
        let b_time = b.metadata().and_then(|m| m.modified()).ok();
        b_time.cmp(&a_time)
    });

    for entry in entries.into_iter().skip(keep) {
        let path = entry.path();
        if let Err(e) = fs::remove_file(&path) {
            tracing::debug!("Failed to remove old backup {:?}: {}", path, e);
        } else {
            tracing::debug!("Removed old backup: {:?}", path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn rapid_backups_preserve_each_previous_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let backup_dir = dir.path().join("backups");
        fs::write(&path, "first").unwrap();
        let first = write_with_backup_in(&path, b"second", &backup_dir)
            .unwrap()
            .unwrap();
        let second = write_with_backup_in(&path, b"third", &backup_dir)
            .unwrap()
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read_to_string(first).unwrap(), "first");
        assert_eq!(fs::read_to_string(second).unwrap(), "second");
        assert_eq!(fs::read_to_string(path).unwrap(), "third");
    }

    #[cfg(unix)]
    #[test]
    fn configs_and_backups_are_owner_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.env");
        let backup_dir = dir.path().join("backups");
        write_with_backup_in(&path, b"SECRET=old", &backup_dir).unwrap();
        let backup = write_with_backup_in(&path, b"SECRET=new", &backup_dir)
            .unwrap()
            .unwrap();
        for file in [&path, &backup] {
            assert_eq!(
                fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn unreadable_destination_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("directory");
        fs::create_dir(&path).unwrap();
        let result = write_with_backup_in(&path, b"replacement", &dir.path().join("backups"));
        assert!(result.is_err());
        assert!(path.is_dir());
        assert!(!dir.path().join("backups").exists());
    }

    #[test]
    fn test_write_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.json");

        let result = write_with_backup_in(&path, b"hello", &dir.path().join("backups")).unwrap();
        assert!(result.is_none(), "no backup for new file");
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn test_write_same_content_no_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("same.json");
        fs::write(&path, b"hello").unwrap();

        let result = write_with_backup_in(&path, b"hello", &dir.path().join("backups")).unwrap();
        assert!(result.is_none(), "no backup when content unchanged");
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn test_write_different_content_creates_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("changed.json");
        fs::write(&path, b"old content").unwrap();

        let result =
            write_with_backup_in(&path, b"new content", &dir.path().join("backups")).unwrap();
        assert!(result.is_some(), "backup should be created");

        let backup_path = result.unwrap();
        assert!(backup_path.exists());
        assert_eq!(fs::read_to_string(&backup_path).unwrap(), "old content");
        assert_eq!(fs::read_to_string(&path).unwrap(), "new content");
    }

    #[test]
    fn test_write_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("c").join("file.json");

        let result = write_with_backup_in(&path, b"nested", &dir.path().join("backups"));
        assert!(result.is_ok());
        assert_eq!(fs::read_to_string(&path).unwrap(), "nested");
    }

    #[test]
    fn test_cleanup_old_backups_keeps_newest() {
        let dir = tempfile::tempdir().unwrap();
        let backup_dir = dir.path();

        // Create 15 fake backup files. We write them sequentially so
        // filesystem mtime increases monotonically. A small sleep
        // ensures distinct mtimes on filesystems with coarse resolution.
        for i in 0..15 {
            let name = format!("config.json.20260101_{:06}.bak", i);
            fs::write(backup_dir.join(&name), format!("backup {}", i)).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        cleanup_old_backups(backup_dir, 10);

        let remaining: Vec<_> = fs::read_dir(backup_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(remaining.len(), 10, "should keep exactly 10 backups");

        // The 5 oldest (000000..000004) should be gone
        for i in 0..5 {
            let name = format!("config.json.20260101_{:06}.bak", i);
            assert!(
                !backup_dir.join(&name).exists(),
                "old backup {} should be removed",
                name
            );
        }
        // The 10 newest (000005..000014) should remain
        for i in 5..15 {
            let name = format!("config.json.20260101_{:06}.bak", i);
            assert!(
                backup_dir.join(&name).exists(),
                "new backup {} should remain",
                name
            );
        }
    }

    #[test]
    fn test_cleanup_ignores_non_bak_files() {
        let dir = tempfile::tempdir().unwrap();
        let backup_dir = dir.path();

        // Create some .bak files and a non-.bak file
        for i in 0..5 {
            let name = format!("config.json.20260101_{:06}.bak", i);
            fs::write(backup_dir.join(&name), "data").unwrap();
        }
        fs::write(backup_dir.join("readme.txt"), "not a backup").unwrap();

        cleanup_old_backups(backup_dir, 3);

        // Non-bak file should survive
        assert!(backup_dir.join("readme.txt").exists());
        // Only 3 .bak files should remain
        let bak_count = fs::read_dir(backup_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".bak")))
            .count();
        assert_eq!(bak_count, 3);
    }
}
