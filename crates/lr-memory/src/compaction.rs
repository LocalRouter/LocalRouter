//! Session compaction — archival of expired session transcripts.
//!
//! When a session expires, the transcript is moved to the archive directory.
//! Future: LLM-based summarization will be added (calling LocalRouter's
//! chat endpoint directly from Rust).

use std::path::Path;

/// Archive an expired session transcript.
///
/// Moves the session file to the archive directory.
/// Future: will add LLM summarization before archiving.
pub async fn compact_session(session_path: &Path, archive_dir: &Path) -> Result<(), String> {
    let file_name = session_path
        .file_name()
        .ok_or("Invalid session path")?
        .to_string_lossy();

    let session_id = file_name.trim_end_matches(".md");

    tracing::info!(
        "Archiving session {}",
        &session_id[..8.min(session_id.len())],
    );

    std::fs::create_dir_all(archive_dir)
        .map_err(|e| format!("Failed to create archive dir: {}", e))?;

    // Move original to archive
    let archive_path = archive_dir.join(format!("{}.md", session_id));
    tokio::fs::rename(session_path, &archive_path)
        .await
        .map_err(|e| format!("Failed to archive session: {}", e))?;

    tracing::info!(
        "Session {} archived",
        &session_id[..8.min(session_id.len())]
    );

    Ok(())
}
