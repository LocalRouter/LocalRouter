//! Atomic file write with backup utility
//!
//! Port of Ollama's `writeWithBackup` pattern for safe config file modification.

use std::fs;
use std::path::{Path, PathBuf};

/// Write data to path via temp file + rename, backing up any existing file first.
/// Returns the backup path if one was created.
pub fn write_with_backup(path: &Path, data: &[u8]) -> Result<Option<PathBuf>, String> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory {:?}: {}", parent, e))?;
    }

    // Read existing file content to check if backup is needed
    let existing = fs::read(path).ok();
    let backup_path = if let Some(ref existing_data) = existing {
        if existing_data != data {
            // Content differs - create backup
            let backup_dir = dirs::data_local_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("localrouter-backups");
            fs::create_dir_all(&backup_dir)
                .map_err(|e| format!("Failed to create backup dir: {}", e))?;

            let filename = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            let backup_name = format!("{}.{}.bak", filename, timestamp);
            let backup_path = backup_dir.join(backup_name);

            fs::write(&backup_path, existing_data)
                .map_err(|e| format!("Failed to write backup to {:?}: {}", backup_path, e))?;

            tracing::info!("Backed up {:?} to {:?}", path, backup_path);

            // Self-cleanup: keep only the last 10 backups
            cleanup_old_backups(&backup_dir, 10);

            Some(backup_path)
        } else {
            // Content is the same, no backup needed
            None
        }
    } else {
        None
    };

    // Write to temp file in same directory, then atomic rename
    let parent = path.parent().unwrap_or(Path::new("."));
    let temp_path = parent.join(format!(
        ".localrouter-tmp-{}",
        uuid::Uuid::new_v4().as_simple()
    ));

    fs::write(&temp_path, data)
        .map_err(|e| format!("Failed to write temp file {:?}: {}", temp_path, e))?;

    fs::rename(&temp_path, path).map_err(|e| {
        // Clean up temp file on rename failure
        let _ = fs::remove_file(&temp_path);
        format!("Failed to rename {:?} to {:?}: {}", temp_path, path, e)
    })?;

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
    fn test_write_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.json");

        let result = write_with_backup(&path, b"hello").unwrap();
        assert!(result.is_none(), "no backup for new file");
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn test_write_same_content_no_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("same.json");
        fs::write(&path, b"hello").unwrap();

        let result = write_with_backup(&path, b"hello").unwrap();
        assert!(result.is_none(), "no backup when content unchanged");
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn test_write_different_content_creates_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("changed.json");
        fs::write(&path, b"old content").unwrap();

        let result = write_with_backup(&path, b"new content").unwrap();
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

        let result = write_with_backup(&path, b"nested");
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
