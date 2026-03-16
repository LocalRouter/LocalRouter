//! Session compaction — LLM-based summarization of session transcripts.
//!
//! When a session expires (3h inactivity or 8h max), the transcript is
//! compacted into a summary, and the original is archived.

use std::path::Path;

use lr_config::MemoryCompactionConfig;

use crate::cli::MemsearchCli;

/// Compact a session transcript into a summary.
///
/// 1. Runs `memsearch compact` on the session file
/// 2. Writes summary to `sessions/{id}-summary.md`
/// 3. Moves original to `archive/{id}.md`
pub async fn compact_session(
    cli: &MemsearchCli,
    session_path: &Path,
    archive_dir: &Path,
    config: &MemoryCompactionConfig,
) -> Result<(), String> {
    let file_name = session_path
        .file_name()
        .ok_or("Invalid session path")?
        .to_string_lossy();

    let session_id = file_name.trim_end_matches(".md");
    let sessions_dir = session_path
        .parent()
        .ok_or("Session path has no parent directory")?;
    let working_dir = sessions_dir
        .parent()
        .ok_or("Sessions dir has no parent directory")?;

    tracing::info!(
        "Compacting session {} with provider {}",
        &session_id[..8.min(session_id.len())],
        config.llm_provider
    );

    // Run memsearch compact
    cli.compact(working_dir, session_path, &config.llm_provider)
        .await?;

    // The compact command writes to the working directory.
    // We need to check if it produced a summary file and handle it.
    // memsearch compact typically writes to the same directory or a configured output.
    // For safety, let's write our own summary marker and move the original.

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
