//! Session compaction — archival and LLM-based summarization of expired session transcripts.
//!
//! When a session expires, the transcript is moved to the archive directory.
//! If a compaction model is configured, an LLM generates a summary that replaces
//! the raw transcript in the search index. The original is preserved for re-compaction.

use std::path::Path;

/// Result of a successful LLM compaction call, carrying full response metadata
/// for monitor event observability.
#[derive(Debug, Clone)]
pub struct CompactionResult {
    pub summary: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub reasoning_tokens: Option<u32>,
    pub finish_reason: Option<String>,
    /// Serialized CompletionRequest (for monitor event)
    pub request_body: Option<serde_json::Value>,
    /// Serialized CompletionResponse (for monitor event)
    pub response_body: Option<serde_json::Value>,
}

/// Trait for calling an LLM to summarize a transcript.
///
/// Implemented at the application level (e.g., via the Router) to avoid
/// circular crate dependencies between lr-memory and lr-router.
#[async_trait::async_trait]
pub trait CompactionLlm: Send + Sync + 'static {
    /// Summarize a conversation transcript using the given model.
    ///
    /// `model` is in "provider/model" format (e.g., "anthropic/claude-haiku-4-5-20251001").
    /// Returns the summary text along with full response metadata.
    async fn summarize(&self, model: &str, transcript: &str)
        -> Result<CompactionResult, String>;
}

/// Outcome of a compaction operation.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum CompactionOutcome {
    /// Session was archived without LLM summarization.
    ArchivedOnly,
    /// LLM responded but returned an empty summary — metadata preserved for debugging.
    ArchivedEmptyResponse(CompactionResult),
    /// Session was archived and an LLM summary was generated.
    ArchivedAndSummarized(CompactionResult),
}

/// Result of a summarize-and-write operation.
enum SummarizeOutcome {
    /// Summary was non-empty and written to disk.
    Written(CompactionResult),
    /// LLM responded but summary was empty — not written to disk.
    Empty(CompactionResult),
    /// LLM call failed entirely.
    Failed(String),
}

/// Shared logic: call LLM, check for empty, write summary file if non-empty.
async fn summarize_and_write(
    llm: &dyn CompactionLlm,
    model: &str,
    content: &str,
    summary_path: &Path,
    short_id: &str,
) -> SummarizeOutcome {
    let result = match llm.summarize(model, content).await {
        Ok(r) => r,
        Err(e) => return SummarizeOutcome::Failed(e),
    };

    if result.summary.trim().is_empty() {
        tracing::warn!(
            "LLM returned empty summary for session {} (output_tokens={}, reasoning_tokens={:?})",
            short_id,
            result.output_tokens,
            result.reasoning_tokens,
        );
        return SummarizeOutcome::Empty(result);
    }

    if let Err(e) = tokio::fs::write(summary_path, &result.summary).await {
        tracing::warn!("Failed to write summary for session {}: {}", short_id, e);
        return SummarizeOutcome::Failed(format!("Failed to write summary: {}", e));
    }

    tracing::info!(
        "Session {} summarized ({} bytes)",
        short_id,
        result.summary.len(),
    );

    SummarizeOutcome::Written(result)
}

const COMPACTION_SYSTEM_PROMPT: &str = "\
You are a memory compaction assistant. Your task is to compress a conversation \
transcript into a structured summary that preserves all important information \
while being significantly shorter than the original.\n\
\n\
## Instructions\n\
\n\
1. **Preserve completely**: decisions made, technical details, code snippets, \
action items, configuration changes, error messages, and their resolutions.\n\
\n\
2. **Use structured markdown**: organize by topic with `##` headers and bullet points. \
Group related items together rather than preserving chronological order.\n\
\n\
3. **Optimize for searchability**: include specific names, function/file names, \
model identifiers, error codes, and domain terms. A future search should be able \
to find any important detail mentioned in the original conversation.\n\
\n\
4. **Compress aggressively**: remove greetings, filler, repeated context, \
and conversational back-and-forth. Keep only the information payload. \
Target 20-30% of the original length.\n\
\n\
5. **Preserve code snippets**: include short code examples, commands, and \
configuration values verbatim \u{2014} do not paraphrase technical content.\n\
\n\
6. **Note unresolved items**: if the conversation ended with open questions \
or incomplete work, add a `## Open Items` section at the end.";

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
    let short_id = &session_id[..8.min(session_id.len())];

    tracing::info!("Archiving session {}", short_id);

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

    tracing::info!("Session {} archived", short_id);

    // Summarize with LLM if available
    if let (Some(llm), Some(model), Some(content)) = (llm, model, content) {
        if content.trim().is_empty() {
            return Ok(CompactionOutcome::ArchivedOnly);
        }

        let summary_path = archive_dir.join(format!("{}-summary.md", session_id));
        match summarize_and_write(llm, model, &content, &summary_path, short_id).await {
            SummarizeOutcome::Written(result) => {
                return Ok(CompactionOutcome::ArchivedAndSummarized(result));
            }
            SummarizeOutcome::Empty(result) => {
                return Ok(CompactionOutcome::ArchivedEmptyResponse(result));
            }
            SummarizeOutcome::Failed(e) => {
                tracing::warn!(
                    "LLM summarization failed for session {}, keeping raw archive: {}",
                    short_id, e,
                );
                // Fall through to ArchivedOnly
            }
        }
    }

    Ok(CompactionOutcome::ArchivedOnly)
}

/// Result of re-compacting a session.
#[allow(clippy::large_enum_variant)]
pub enum RecompactOutcome {
    /// Summary was generated successfully.
    Summarized(CompactionResult),
    /// LLM responded but summary was empty — metadata preserved for debugging.
    EmptyResponse(CompactionResult),
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
) -> Result<RecompactOutcome, String> {
    let short_id = &session_id[..8.min(session_id.len())];
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

    let summary_path = archive_dir.join(format!("{}-summary.md", session_id));
    match summarize_and_write(llm, model, &content, &summary_path, short_id).await {
        SummarizeOutcome::Written(result) => Ok(RecompactOutcome::Summarized(result)),
        SummarizeOutcome::Empty(result) => Ok(RecompactOutcome::EmptyResponse(result)),
        SummarizeOutcome::Failed(e) => Err(e),
    }
}

/// Return the system prompt used for compaction summarization.
pub fn system_prompt() -> &'static str {
    COMPACTION_SYSTEM_PROMPT
}
