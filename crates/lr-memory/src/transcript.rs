//! Session transcript file management.
//!
//! Handles creating, appending to, and restoring session markdown files.

use std::path::{Path, PathBuf};

use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

/// Writes session transcripts as markdown files.
pub struct TranscriptWriter;

impl Default for TranscriptWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl TranscriptWriter {
    pub fn new() -> Self {
        Self
    }

    /// Create an empty session file at the given path.
    pub async fn create_session_file(&self, file_path: &Path) -> Result<(), String> {
        fs::write(file_path, "")
            .await
            .map_err(|e| format!("Failed to create session file: {}", e))?;

        tracing::debug!("Created session file: {}", file_path.display());
        Ok(())
    }

    /// Append a user/assistant exchange to the session file.
    pub async fn append_exchange(
        &self,
        path: &Path,
        user_content: &str,
        assistant_content: &str,
        timestamp: &str,
    ) -> Result<(), String> {
        let exchange = format!(
            "<user timestamp=\"{}\">\n{}\n</user>\n<assistant>\n{}\n</assistant>\n",
            timestamp, user_content, assistant_content
        );
        self.append_raw(path, &exchange).await
    }

    /// Restore a session file from the archive directory back to the active directory.
    /// Deletes any existing summary file for that session.
    /// `file_stem` is the filename without extension (e.g., "2026-03-22T14-30-00-topic-x7k2m").
    /// Returns the restored file path.
    pub async fn restore_from_archive(
        &self,
        file_stem: &str,
        active_dir: &Path,
        archive_dir: &Path,
    ) -> Result<PathBuf, String> {
        let archive_path = archive_dir.join(format!("{}.md", file_stem));
        let active_path = active_dir.join(format!("{}.md", file_stem));
        let summary_path = archive_dir.join(format!("{}-summary.md", file_stem));

        if !archive_path.exists() {
            return Err(format!(
                "Archive file not found: {}",
                archive_path.display()
            ));
        }

        // Delete old summary if it exists
        if summary_path.exists() {
            fs::remove_file(&summary_path)
                .await
                .map_err(|e| format!("Failed to delete summary: {}", e))?;
            tracing::debug!("Deleted summary: {}", summary_path.display());
        }

        // Move archive back to active
        fs::rename(&archive_path, &active_path)
            .await
            .map_err(|e| format!("Failed to restore from archive: {}", e))?;

        tracing::info!("Restored session {} from archive", file_stem);
        Ok(active_path)
    }

    /// Build a complete session transcript in memory (no file I/O).
    /// Used by the Try It Out tab to generate realistic sample content
    /// that stays in sync with the real transcript format.
    pub fn build_transcript(
        exchanges: &[(&str, &str)], // (user_content, assistant_content)
    ) -> String {
        let mut out = String::new();

        if !exchanges.is_empty() {
            let ts = chrono::Utc::now().to_rfc3339();

            for (user, assistant) in exchanges {
                out.push_str(&format!(
                    "<user timestamp=\"{}\">\n{}\n</user>\n<assistant>\n{}\n</assistant>\n",
                    ts, user, assistant
                ));
            }
        }

        out
    }

    /// Append raw content to a file.
    async fn append_raw(&self, path: &Path, content: &str) -> Result<(), String> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await
            .map_err(|e| format!("Failed to open transcript file: {}", e))?;

        file.write_all(content.as_bytes())
            .await
            .map_err(|e| format!("Failed to write to transcript: {}", e))?;

        file.flush()
            .await
            .map_err(|e| format!("Failed to flush transcript: {}", e))?;

        Ok(())
    }
}
