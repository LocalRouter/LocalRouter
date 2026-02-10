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
}
