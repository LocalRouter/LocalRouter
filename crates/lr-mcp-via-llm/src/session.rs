//! Session management for MCP via LLM
//!
//! Each session tracks conversation history including injected tool calls
//! that the client never sees. Sessions are matched to incoming requests
//! via per-message hashing.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Instant;

use lr_providers::{ChatMessage, ChatMessageContent};
use tokio::task::JoinHandle;
use unicode_normalization::UnicodeNormalization;

/// Tracks a pending mixed tool execution where MCP tools run in the background
/// while we wait for the client to return its tool results.
#[allow(dead_code)]
pub struct PendingMixedExecution {
    /// The full assistant message containing ALL tool calls (MCP + client)
    pub full_assistant_message: ChatMessage,
    /// Background handles for MCP tool executions: (tool_call_id, Result<content, error>)
    pub mcp_handles: Vec<JoinHandle<(String, Result<String, String>)>>,
    /// Tool call IDs that were sent to the client
    pub client_tool_call_ids: Vec<String>,
    /// Accumulated prompt tokens from iterations before the mixed call
    pub accumulated_prompt_tokens: u64,
    /// Accumulated completion tokens from iterations before the mixed call
    pub accumulated_completion_tokens: u64,
    /// MCP tools called in iterations before the mixed call
    pub mcp_tools_called: Vec<String>,
    /// Messages as they were before the mixed tool call (for history reconstruction)
    pub messages_before_mixed: Vec<ChatMessage>,
    /// When the mixed execution started
    pub started_at: Instant,
    /// Per-iteration token usage entries accumulated before the mixed call
    pub accumulated_usage_entries: Vec<lr_providers::TokenUsage>,
    /// Gateway session key of the session that created this pending execution
    pub gateway_session_key: String,
}

impl Drop for PendingMixedExecution {
    fn drop(&mut self) {
        // Abort any still-running background MCP tasks when the pending execution is dropped
        for handle in &self.mcp_handles {
            handle.abort();
        }
    }
}

/// Tracks the full conversation history for an MCP via LLM session,
/// including messages the client never sees (injected tool calls/results).
pub struct SessionHistory {
    /// Complete history including injected tool call/result messages
    pub full_messages: Vec<ChatMessage>,
}

impl SessionHistory {
    pub fn new() -> Self {
        Self {
            full_messages: Vec::new(),
        }
    }

    /// Replace the full message history
    pub fn set_messages(&mut self, messages: Vec<ChatMessage>) {
        self.full_messages = messages;
    }
}

/// A single MCP via LLM session tied to one client
pub struct McpViaLlmSession {
    pub client_id: String,
    /// Key used to identify this session in the MCP gateway
    pub gateway_session_key: String,
    /// Whether the gateway session has been initialized
    pub gateway_initialized: bool,
    /// Gateway instructions obtained during initialization.
    /// Stored here when `list_tools_for_preview` initializes the gateway
    /// before the orchestrator runs, so the orchestrator can still inject them.
    pub pending_gateway_instructions: Option<String>,
    /// Persisted gateway instructions — re-injected on every request so that
    /// multi-turn conversations always include server instructions.
    pub gateway_instructions: Option<String>,
    /// Conversation history (including injected tool calls)
    pub history: SessionHistory,
    /// Last time this session was active
    pub last_activity: Instant,
    /// Path to the memory transcript file (set when memory is enabled for this client)
    pub transcript_path: Option<PathBuf>,
    /// Memory folder slug (for resolving client dir when indexing transcripts)
    pub memory_folder: Option<String>,
    /// Hashes of client-visible messages from the last request (before any injection).
    /// Used for session matching on subsequent requests.
    pub client_message_hashes: Vec<u64>,
}

impl McpViaLlmSession {
    pub fn new(session_id: String, client_id: String) -> Self {
        let gateway_session_key = format!("mcp-via-llm-{}", session_id);
        Self {
            client_id,
            gateway_session_key,
            gateway_initialized: false,
            pending_gateway_instructions: None,
            gateway_instructions: None,
            history: SessionHistory::new(),
            last_activity: Instant::now(),
            transcript_path: None,
            memory_folder: None,
            client_message_hashes: Vec::new(),
        }
    }

    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    pub fn is_expired(&self, ttl: std::time::Duration) -> bool {
        self.last_activity.elapsed() > ttl
    }
}

// ---------------------------------------------------------------------------
// Normalized hashing for fuzzy-resilient session matching
// ---------------------------------------------------------------------------

/// Normalize text for fuzzy-resilient hash comparison.
/// Handles: leading/trailing whitespace, interior whitespace collapse, Unicode NFC.
pub(crate) fn normalize_for_hash(text: &str) -> String {
    text.trim()
        .nfc()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Compute a normalized hash for a single message (role + text content).
pub fn compute_message_hash(role: &str, content: &ChatMessageContent) -> u64 {
    let text = content.as_text();
    let normalized = normalize_for_hash(&text);
    let mut hasher = DefaultHasher::new();
    role.hash(&mut hasher);
    normalized.hash(&mut hasher);
    hasher.finish()
}

/// Compute normalized hashes for a slice of ChatMessages.
pub fn compute_message_hashes(messages: &[ChatMessage]) -> Vec<u64> {
    messages
        .iter()
        .map(|m| compute_message_hash(&m.role, &m.content))
        .collect()
}

/// Score how well stored hashes match incoming hashes.
/// Returns 0.0..=1.0 where 1.0 is a perfect match.
///
/// Handles:
/// - Exact prefix match (stored is prefix of incoming) → 1.0
/// - Suffix-anchored match (client dropped old messages) → proportion matched
/// - No match → 0.0
pub fn score_session_match(stored: &[u64], incoming: &[u64]) -> f64 {
    if stored.is_empty() || incoming.is_empty() {
        return 0.0;
    }

    // Case 1: stored is a prefix of incoming (standard continuation)
    if stored.len() <= incoming.len() && stored.iter().zip(incoming.iter()).all(|(a, b)| a == b) {
        return 1.0;
    }

    // Case 2: suffix-anchored — client dropped older messages
    // Find the longest suffix of stored that matches a prefix of incoming
    let mut best = 0usize;
    for start in 0..stored.len() {
        let suffix = &stored[start..];
        let match_len = suffix
            .iter()
            .zip(incoming.iter())
            .take_while(|(a, b)| a == b)
            .count();
        // Only count if we matched the entire suffix or entire incoming
        if match_len > 0 && (match_len == suffix.len() || match_len == incoming.len()) {
            best = best.max(match_len);
        }
    }

    best as f64 / stored.len() as f64
}

// ---------------------------------------------------------------------------
// History reconstruction — inject hidden MCP messages into client requests
// ---------------------------------------------------------------------------

/// Reconstruct the full conversation history by splicing hidden MCP messages
/// back into the incoming request messages.
///
/// The session's `full_messages` contains the complete history from previous turns
/// including MCP tool calls/results that the client never saw. The client's incoming
/// messages contain only the "visible" subset plus new messages for this turn.
///
/// Algorithm:
/// 1. Find the anchor: hash the last message in full_messages (the final assistant
///    response returned to the client) and find it in incoming
/// 2. Take full_messages (which includes hidden MCP interactions)
/// 3. Append new messages from incoming after the anchor
/// 4. Strip previously-injected server instructions (they'll be re-injected fresh)
/// 5. Use the client's current system message (in case it changed)
pub fn reconstruct_history(
    full_messages: &[ChatMessage],
    incoming: &[ChatMessage],
    gateway_instructions: Option<&str>,
) -> Vec<ChatMessage> {
    if full_messages.is_empty() {
        return incoming.to_vec();
    }

    // Hash the last message in full_messages (the anchor — typically the final
    // assistant response that was returned to the client)
    let anchor = full_messages.last().unwrap();
    let anchor_hash = compute_message_hash(&anchor.role, &anchor.content);

    // Find anchor in incoming (search from end for robustness)
    let anchor_pos = incoming
        .iter()
        .rposition(|m| compute_message_hash(&m.role, &m.content) == anchor_hash);

    let Some(pos) = anchor_pos else {
        // Can't find anchor — can't reconstruct, use incoming as-is
        tracing::debug!(
            "MCP via LLM: history reconstruction skipped — anchor not found in incoming ({} full, {} incoming)",
            full_messages.len(),
            incoming.len()
        );
        return incoming.to_vec();
    };

    let new_message_count = incoming.len() - pos - 1;
    let mut result = Vec::with_capacity(full_messages.len() + new_message_count);

    // Take full session history (includes hidden MCP messages)
    result.extend_from_slice(full_messages);

    // Append new messages from incoming (after the anchor)
    if pos + 1 < incoming.len() {
        result.extend_from_slice(&incoming[pos + 1..]);
    }

    // Strip previously-injected server instructions (will be re-injected fresh)
    if let Some(instructions) = gateway_instructions {
        result.retain(|m| !(m.role == "system" && m.content.as_text() == instructions));
    }

    // Use client's current system message (handles changes between turns)
    if let Some(client_sys) = incoming.first().filter(|m| m.role == "system") {
        if let Some(first_sys) = result.iter_mut().find(|m| m.role == "system") {
            *first_sys = client_sys.clone();
        }
    }

    tracing::debug!(
        "MCP via LLM: history reconstructed — {} full + {} new → {} total messages",
        full_messages.len(),
        new_message_count,
        result.len()
    );

    result
}
