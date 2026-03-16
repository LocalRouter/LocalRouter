//! Session compaction — LLM-based summarization of session transcripts.
//!
//! When a session expires, the transcript is compacted into a summary
//! using an LLM routed through LocalRouter, and the original is archived.

use std::path::Path;

use crate::cli::MemsearchCli;

/// Compact a session transcript into a summary.
///
/// Both embedding (for re-indexing) and LLM (for summarization) calls
/// are routed through LocalRouter via the CLI's configured endpoints.
pub async fn compact_session(
    cli: &MemsearchCli,
    session_path: &Path,
    archive_dir: &Path,
    compaction_model: &str,
) -> Result<(), String> {
    let file_name = session_path
        .file_name()
        .ok_or("Invalid session path")?
        .to_string_lossy();

    let session_id = file_name.trim_end_matches(".md");
    let working_dir = session_path
        .parent()
        .and_then(|p| p.parent())
        .ok_or("Session path has no parent directory")?;

    tracing::info!(
        "Compacting session {} with model {}",
        &session_id[..8.min(session_id.len())],
        compaction_model
    );

    cli.compact(working_dir, session_path, compaction_model)
        .await?;

    // Move original to archive
    let archive_path = archive_dir.join(format!("{}.md", session_id));
    tokio::fs::rename(session_path, &archive_path)
        .await
        .map_err(|e| format!("Failed to archive session: {}", e))?;

    tracing::info!(
        "Session {} compacted and archived",
        &session_id[..8.min(session_id.len())]
    );

    Ok(())
}
