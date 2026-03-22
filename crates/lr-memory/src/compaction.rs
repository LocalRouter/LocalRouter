//! Session compaction — archival and LLM-based summarization of expired session transcripts.
//!
//! When a session expires, the transcript is moved to the archive directory.
//! If a compaction model is configured, an LLM generates a summary that replaces
//! the raw transcript in the search index. The original is preserved for re-compaction.

use std::path::Path;

/// Trait for calling an LLM to summarize a transcript.
///
/// Implemented at the application level (e.g., via the Router) to avoid
/// circular crate dependencies between lr-memory and lr-router.
#[async_trait::async_trait]
pub trait CompactionLlm: Send + Sync + 'static {
    /// Summarize a conversation transcript using the given model.
    ///
    /// `model` is in "provider/model" format (e.g., "anthropic/claude-haiku-4-5-20251001").
    /// Returns the summary text.
    async fn summarize(&self, model: &str, transcript: &str) -> Result<String, String>;
}

/// Outcome of a compaction operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionOutcome {
    /// Session was archived without LLM summarization.
    ArchivedOnly,
    /// Session was archived and an LLM summary was generated.
    ArchivedAndSummarized,
}

const COMPACTION_SYSTEM_PROMPT: &str = "\
You are a memory compaction assistant. Summarize this conversation transcript \
preserving: decisions made, technical details, action items, code snippets, \
and key context. Include specific terms and names for searchability. \
Format as structured markdown with topic headers.";

/// Archive an expired session transcript and optionally summarize it with an LLM.
///
/// 1. Moves the session file from `session_path` to `archive_dir/{uuid}.md`
/// 2. If `llm` and `model` are provided, generates an LLM summary and saves it
///    as `archive_dir/{uuid}-summary.md`
pub async fn compact_session(
    session_path: &Path,
    archive_dir: &Path,
    llm: Option<&dyn CompactionLlm>,
    model: Option<&str>,
) -> Result<CompactionOutcome, String> {
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

    // Read content before moving (needed for summarization)
    let content = if llm.is_some() && model.is_some() {
        Some(
            tokio::fs::read_to_string(session_path)
                .await
                .map_err(|e| format!("Failed to read session for summarization: {}", e))?,
        )
    } else {
        None
    };

    // Move original to archive
    let archive_path = archive_dir.join(format!("{}.md", session_id));
    tokio::fs::rename(session_path, &archive_path)
        .await
        .map_err(|e| format!("Failed to archive session: {}", e))?;

    tracing::info!(
        "Session {} archived",
        &session_id[..8.min(session_id.len())]
    );

    // Summarize with LLM if available
    if let (Some(llm), Some(model), Some(content)) = (llm, model, content) {
        if content.trim().is_empty() {
            return Ok(CompactionOutcome::ArchivedOnly);
        }

        match llm.summarize(model, &content).await {
            Ok(summary) => {
                let summary_path = archive_dir.join(format!("{}-summary.md", session_id));
                tokio::fs::write(&summary_path, &summary)
                    .await
                    .map_err(|e| format!("Failed to write summary: {}", e))?;

                tracing::info!(
                    "Session {} summarized ({} bytes → {} bytes)",
                    &session_id[..8.min(session_id.len())],
                    content.len(),
                    summary.len(),
                );

                return Ok(CompactionOutcome::ArchivedAndSummarized);
            }
            Err(e) => {
                tracing::warn!(
                    "LLM summarization failed for session {}, keeping raw archive: {}",
                    &session_id[..8.min(session_id.len())],
                    e,
                );
                // Fall through to ArchivedOnly
            }
        }
    }

    Ok(CompactionOutcome::ArchivedOnly)
}

/// Re-compact an already-archived session by regenerating its LLM summary.
///
/// Reads the raw transcript from `archive_dir/{session_id}.md`, calls the LLM
/// to generate a new summary, and writes/overwrites `archive_dir/{session_id}-summary.md`.
pub async fn recompact_session(
    session_id: &str,
    archive_dir: &Path,
    llm: &dyn CompactionLlm,
    model: &str,
) -> Result<(), String> {
    let raw_path = archive_dir.join(format!("{}.md", session_id));

    if !raw_path.exists() {
        return Err(format!("Raw transcript not found: {}", raw_path.display()));
    }

    let content = tokio::fs::read_to_string(&raw_path)
        .await
        .map_err(|e| format!("Failed to read raw transcript: {}", e))?;

    if content.trim().is_empty() {
        return Err("Raw transcript is empty".to_string());
    }

    let summary = llm.summarize(model, &content).await?;

    let summary_path = archive_dir.join(format!("{}-summary.md", session_id));
    tokio::fs::write(&summary_path, &summary)
        .await
        .map_err(|e| format!("Failed to write summary: {}", e))?;

    tracing::info!(
        "Session {} re-compacted ({} bytes → {} bytes)",
        &session_id[..8.min(session_id.len())],
        content.len(),
        summary.len(),
    );

    Ok(())
}

/// Return the system prompt used for compaction summarization.
pub fn system_prompt() -> &'static str {
    COMPACTION_SYSTEM_PROMPT
}
