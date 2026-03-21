//! Session transcript file management.
//!
//! Handles creating, appending to, and restoring session markdown files.

use std::path::{Path, PathBuf};

use chrono::Utc;
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

    /// Create a new session file with YAML frontmatter.
    /// Returns the path to the created file.
    pub async fn create_session_file(
        &self,
        sessions_dir: &Path,
        session_id: &str,
        client_id: &str,
    ) -> Result<PathBuf, String> {
        let file_path = sessions_dir.join(format!("{}.md", session_id));

        let frontmatter = format!(
            "---\nclient_id: {}\nsession_id: {}\nstarted: {}\n---\n\n",
            client_id,
            session_id,
            Utc::now().to_rfc3339(),
        );

        fs::write(&file_path, frontmatter)
            .await
            .map_err(|e| format!("Failed to create session file: {}", e))?;

        tracing::debug!("Created session file: {}", file_path.display());
        Ok(file_path)
    }

    /// Append a conversation header to the session file.
    pub async fn append_conversation_header(
        &self,
        path: &Path,
        conversation_id: &str,
        timestamp: &str,
    ) -> Result<(), String> {
        let header = format!("\n<!-- conversation {} {} -->\n\n", conversation_id, timestamp);
        self.append_raw(path, &header).await
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
            "<user timestamp=\"{}\">\n{}\n</user>\n\n<assistant>\n{}\n</assistant>\n\n",
            timestamp, user_content, assistant_content
        );
        self.append_raw(path, &exchange).await
    }

    /// Restore a session file from the archive directory back to the sessions directory.
    /// Deletes any existing summary file for that session.
    /// Returns the restored file path.
    pub async fn restore_from_archive(
        &self,
        session_id: &str,
        sessions_dir: &Path,
        archive_dir: &Path,
    ) -> Result<PathBuf, String> {
        let archive_path = archive_dir.join(format!("{}.md", session_id));
        let sessions_path = sessions_dir.join(format!("{}.md", session_id));
        let summary_path = sessions_dir.join(format!("{}-summary.md", session_id));

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

        // Move archive back to sessions
        fs::rename(&archive_path, &sessions_path)
            .await
            .map_err(|e| format!("Failed to restore from archive: {}", e))?;

        tracing::info!(
            "Restored session {} from archive",
            &session_id[..8.min(session_id.len())]
        );
        Ok(sessions_path)
    }

    /// Build a complete session transcript in memory (no file I/O).
    /// Used by the Try It Out tab to generate realistic sample content
    /// that stays in sync with the real transcript format.
    pub fn build_transcript(
        client_id: &str,
        session_id: &str,
        exchanges: &[(&str, &str)], // (user_content, assistant_content)
    ) -> String {
        let mut out = format!(
            "---\nclient_id: {}\nsession_id: {}\nstarted: {}\n---\n\n",
            client_id,
            session_id,
            chrono::Utc::now().to_rfc3339(),
        );

        if !exchanges.is_empty() {
            let ts = chrono::Utc::now().to_rfc3339();
            out.push_str(&format!("\n<!-- conversation 1 {} -->\n\n", ts));

            for (user, assistant) in exchanges {
                out.push_str(&format!(
                    "<user timestamp=\"{}\">\n{}\n</user>\n\n<assistant>\n{}\n</assistant>\n\n",
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
