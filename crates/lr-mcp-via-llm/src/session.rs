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
pub struct PendingMixedExecution {
    /// The full assistant message containing ALL tool calls (MCP + client)
    pub full_assistant_message: ChatMessage,
    /// Background handles for MCP tool executions: (tool_call_id, Result<content, error>)
    pub mcp_handles: Vec<JoinHandle<(String, Result<String, String>)>>,
    /// Tool call IDs that were sent to the client
    pub client_tool_call_ids: Vec<String>,
    /// Messages as they were before the mixed tool call (for history reconstruction)
    pub messages_before_mixed: Vec<ChatMessage>,
    /// When the mixed execution started
    pub started_at: Instant,
    /// What the tool loop did before it stopped for the client's tools
    /// (iterations, tokens, gateway tools), so the resumed loop reports the
    /// whole turn.
    pub prior: PriorTurnTotals,
    /// `McpViaLlmSession::id` of the session that created this pending execution
    pub session_id: String,
}

/// Totals of a tool loop carried across a mixed tool call: the loop stops
/// to let the client run its tools and resumes on the client's next request.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PriorTurnTotals {
    /// LLM calls made before the stop
    pub iterations: u32,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Gateway tools called before the stop, including those that run in
    /// the background while the client answers
    pub mcp_tools_called: Vec<String>,
    /// Per-iteration usage of those calls
    pub usage_entries: Vec<lr_providers::TokenUsage>,
}

impl Drop for PendingMixedExecution {
    fn drop(&mut self) {
        // Abort any still-running background MCP tasks when the pending execution is dropped
        for handle in &self.mcp_handles {
            handle.abort();
        }
    }
}

/// A pending mixed execution as stored, so the turn can resume after the
/// in-memory one is gone (the app restarted). Holds the gateway calls'
/// results that finished before it was written; a call without a result
/// is never run again on resume (its outcome is unknown).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredPendingMixed {
    /// The assistant message with every call, the client's and the
    /// gateway's
    pub full_assistant_message: ChatMessage,
    pub client_tool_call_ids: Vec<String>,
    /// The conversation before the call, without the turn's `instructions`
    /// and the gateway's instructions (both are added fresh on resume)
    pub messages_before_mixed: Vec<ChatMessage>,
    /// Results of the gateway calls that finished, in finishing order
    pub mcp_results: Vec<StoredToolResult>,
    pub prior: PriorTurnTotals,
}

/// One finished gateway call of a [`StoredPendingMixed`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StoredToolResult {
    pub tool_call_id: String,
    /// The result content, or the error the call failed with
    pub result: Result<String, String>,
}

/// Where a pending mixed execution is written: called when the turn stops
/// for the client's tools and again as each gateway call finishes, with
/// everything known so far. Calls are serialized; the last is the latest.
#[derive(Clone)]
pub struct PendingPersistence(pub std::sync::Arc<dyn Fn(&StoredPendingMixed) + Send + Sync>);

impl std::fmt::Debug for PendingPersistence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PendingPersistence")
    }
}

/// Error of a gateway call whose pending execution was restored from
/// storage without its result: the call was running when the app stopped.
pub(crate) const UNKNOWN_OUTCOME: &str = "its outcome is unknown: LocalRouter stopped before \
     the call finished, and it may or may not have taken effect";

impl PendingMixedExecution {
    /// Write this execution through `persist` now, and again as each
    /// background gateway call finishes. `instructions` (the turn's own)
    /// and `gateway_instructions` are left out of the stored messages.
    pub(crate) fn persist_with(
        &mut self,
        persist: PendingPersistence,
        instructions: Option<&str>,
        gateway_instructions: Option<&str>,
    ) {
        let mut messages: &[ChatMessage] = &self.messages_before_mixed;
        if let (Some(stored), Some(first)) = (instructions, messages.first()) {
            if first.role == "system" && first.content.as_text() == stored {
                messages = &messages[1..];
            }
        }
        let stored = StoredPendingMixed {
            full_assistant_message: self.full_assistant_message.clone(),
            client_tool_call_ids: self.client_tool_call_ids.clone(),
            messages_before_mixed: messages
                .iter()
                .filter(|m| {
                    !(m.role == "system"
                        && gateway_instructions.is_some_and(|g| m.content.as_text() == g))
                })
                .cloned()
                .collect(),
            mcp_results: Vec::new(),
            prior: self.prior.clone(),
        };
        let stored = std::sync::Arc::new(parking_lot::Mutex::new(stored));
        (persist.0)(&stored.lock());
        // Each call reports its result once it has one. The wrapper owns
        // the call: aborting the wrapper (dropping this execution) aborts
        // the call too.
        self.mcp_handles = std::mem::take(&mut self.mcp_handles)
            .into_iter()
            .map(|call| {
                let stored = stored.clone();
                let persist = persist.clone();
                lr_types::spawn_traced(async move {
                    struct AbortCall(tokio::task::AbortHandle);
                    impl Drop for AbortCall {
                        fn drop(&mut self) {
                            self.0.abort();
                        }
                    }
                    let _abort = AbortCall(call.abort_handle());
                    let outcome = match call.await {
                        Ok(outcome) => outcome,
                        Err(e) => ("unknown".to_string(), Err(format!("Task panicked: {e}"))),
                    };
                    let mut stored = stored.lock();
                    stored.mcp_results.push(StoredToolResult {
                        tool_call_id: outcome.0.clone(),
                        result: outcome.1.clone(),
                    });
                    (persist.0)(&stored);
                    outcome
                })
            })
            .collect();
    }

    /// The execution `stored` describes, for the session `session_id`: the
    /// gateway calls resolve at once to their stored results, or to an
    /// unknown-outcome error for those that had none.
    pub(crate) fn from_stored(stored: StoredPendingMixed, session_id: String) -> Self {
        let mcp_handles = stored
            .full_assistant_message
            .tool_calls
            .iter()
            .flatten()
            .filter(|call| !stored.client_tool_call_ids.contains(&call.id))
            .map(|call| {
                let result = stored
                    .mcp_results
                    .iter()
                    .find(|r| r.tool_call_id == call.id)
                    .map(|r| r.result.clone())
                    .unwrap_or_else(|| Err(UNKNOWN_OUTCOME.to_string()));
                let id = call.id.clone();
                lr_types::spawn_traced(async move { (id, result) })
            })
            .collect();
        Self {
            full_assistant_message: stored.full_assistant_message,
            mcp_handles,
            client_tool_call_ids: stored.client_tool_call_ids,
            messages_before_mixed: stored.messages_before_mixed,
            started_at: Instant::now(),
            prior: stored.prior,
            session_id,
        }
    }
}

/// Tracks the full conversation history for an MCP via LLM session,
/// including messages the client never sees (injected tool calls/results).
pub struct SessionHistory {
    /// Complete history including injected tool call/result messages
    pub full_messages: Vec<ChatMessage>,
    /// The text of the final assistant message as the client received it,
    /// when that differs from the last message of `full_messages`: chat
    /// completions joins the text of every assistant message of a tool loop
    /// into its one message, while the history keeps them apart. The
    /// client sends this text back on its next turn, so it is the anchor.
    pub reply_text: Option<String>,
    /// The history ends with a turn that finished its loop (its final
    /// assistant message is the last message), not with one still running,
    /// failed, or stopped for a mixed tool call.
    pub finished: bool,
}

impl SessionHistory {
    pub fn new() -> Self {
        Self {
            full_messages: Vec::new(),
            reply_text: None,
            finished: false,
        }
    }

    /// Replace the full message history
    pub fn set_messages(&mut self, messages: Vec<ChatMessage>) {
        self.full_messages = messages;
        self.reply_text = None;
        self.finished = false;
    }

    /// Record a finished turn: its history, its final assistant message,
    /// and the text the client got for that message (`reply_text`).
    pub fn finish_turn(
        &mut self,
        messages: Vec<ChatMessage>,
        final_message: ChatMessage,
        reply_text: Option<String>,
    ) {
        let reply_text = reply_text.filter(|text| *text != final_message.content.as_text());
        self.full_messages = messages;
        self.full_messages.push(final_message);
        self.reply_text = reply_text;
        self.finished = true;
    }
}

/// Separator between the texts of a tool loop's assistant messages when
/// they are reported as one chat completions message.
pub const JOINED_MESSAGE_SEPARATOR: &str = "\n\n";

/// The text of a tool loop's assistant messages as one message: the
/// non-empty texts in order, separated by [`JOINED_MESSAGE_SEPARATOR`].
pub fn join_message_texts<'a>(texts: impl IntoIterator<Item = &'a str>) -> String {
    texts
        .into_iter()
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(JOINED_MESSAGE_SEPARATOR)
}

/// A single MCP via LLM session tied to one client
pub struct McpViaLlmSession {
    /// Unique id of this session. Forks share the gateway session but get
    /// their own id.
    pub id: String,
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
    /// Deterministic session key: the id of the response whose history
    /// this session holds (`TurnOptions::session_key`), which a later
    /// Responses API turn names as `previous_response_id`. Stamped only
    /// once the turn succeeded. `None` for chat completions sessions and
    /// for keyed sessions whose turn has not finished (or failed).
    pub explicit_key: Option<String>,
    /// Created for a keyed (Responses API) turn. Keyed sessions are found
    /// only by `explicit_key`, never by message hashes.
    pub keyed: bool,
    /// The `instructions` of the turn that wrote `history`: when set,
    /// `history.full_messages` starts with them as a system message. They
    /// apply to that turn only and are dropped when a later turn rebuilds
    /// its history from this session.
    pub instructions_head: Option<String>,
}

impl McpViaLlmSession {
    #[cfg(test)]
    pub fn new(session_id: String, client_id: String) -> Self {
        Self::new_with_key(session_id, client_id, None)
    }

    pub fn new_with_key(
        session_id: String,
        client_id: String,
        explicit_key: Option<String>,
    ) -> Self {
        let gateway_session_key = format!("mcp-via-llm-{}", session_id);
        Self {
            id: session_id,
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
            explicit_key,
            keyed: false,
            instructions_head: None,
        }
    }

    /// A copy that continues from this session's state (same gateway
    /// session, same history) under a new id and without a key, so a turn
    /// can run on it while this session keeps the history of the response
    /// it is keyed to. Lets two turns continue from one response.
    pub fn fork(&self) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            client_id: self.client_id.clone(),
            gateway_session_key: self.gateway_session_key.clone(),
            gateway_initialized: self.gateway_initialized,
            pending_gateway_instructions: self.pending_gateway_instructions.clone(),
            gateway_instructions: self.gateway_instructions.clone(),
            history: SessionHistory {
                full_messages: self.history.full_messages.clone(),
                reply_text: self.history.reply_text.clone(),
                finished: self.history.finished,
            },
            last_activity: Instant::now(),
            transcript_path: self.transcript_path.clone(),
            memory_folder: self.memory_folder.clone(),
            client_message_hashes: self.client_message_hashes.clone(),
            explicit_key: None,
            keyed: self.keyed,
            instructions_head: self.instructions_head.clone(),
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
#[cfg(test)]
pub fn reconstruct_history(
    full_messages: &[ChatMessage],
    incoming: &[ChatMessage],
    gateway_instructions: Option<&str>,
) -> Vec<ChatMessage> {
    reconstruct_history_with_reply(full_messages, None, incoming, gateway_instructions)
}

/// [`reconstruct_history`] for a history whose final assistant message
/// reached the client as `reply_text` (see `SessionHistory::reply_text`).
pub fn reconstruct_history_with_reply(
    full_messages: &[ChatMessage],
    reply_text: Option<&str>,
    incoming: &[ChatMessage],
    gateway_instructions: Option<&str>,
) -> Vec<ChatMessage> {
    if full_messages.is_empty() {
        return incoming.to_vec();
    }

    // Hash the last message in full_messages (the anchor — typically the final
    // assistant response that was returned to the client), as the client saw it
    let anchor = full_messages.last().unwrap();
    let anchor_hash = match reply_text {
        Some(text) => compute_message_hash(&anchor.role, &ChatMessageContent::Text(text.into())),
        None => compute_message_hash(&anchor.role, &anchor.content),
    };

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

    // Use client's current system message (handles changes between turns,
    // including a client that only starts sending one on a later turn)
    if let Some(client_sys) = incoming.first().filter(|m| m.role == "system") {
        match result.iter_mut().find(|m| m.role == "system") {
            Some(first_sys) => *first_sys = client_sys.clone(),
            None => result.insert(0, client_sys.clone()),
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

/// Keyed (Responses API) variant of [`reconstruct_history`].
///
/// `full_messages` is the history of exactly the response the turn
/// continues from. `instructions` apply to one request only: the stored
/// turn's instructions (`stored_instructions`, its leading system message)
/// are dropped, and this turn's own (`incoming[0]` when
/// `leading_instructions`) lead the result. Other system messages of the
/// conversation are kept as they are.
pub fn reconstruct_keyed_history(
    full_messages: &[ChatMessage],
    incoming: &[ChatMessage],
    gateway_instructions: Option<&str>,
    stored_instructions: Option<&str>,
    leading_instructions: bool,
) -> Vec<ChatMessage> {
    let mut history: &[ChatMessage] = full_messages;
    if let (Some(stored), Some(first)) = (stored_instructions, history.first()) {
        if first.role == "system" && first.content.as_text() == stored {
            history = &history[1..];
        }
    }
    let history: Vec<ChatMessage> = history
        .iter()
        .filter(|m| {
            !(m.role == "system" && gateway_instructions.is_some_and(|g| m.content.as_text() == g))
        })
        .cloned()
        .collect();
    let Some(anchor) = history.last() else {
        return incoming.to_vec();
    };
    let anchor_hash = compute_message_hash(&anchor.role, &anchor.content);
    let Some(pos) = incoming
        .iter()
        .rposition(|m| compute_message_hash(&m.role, &m.content) == anchor_hash)
    else {
        tracing::debug!(
            "MCP via LLM: keyed history reconstruction skipped — anchor not found in incoming"
        );
        return incoming.to_vec();
    };
    let mut result = Vec::with_capacity(history.len() + incoming.len() - pos);
    if leading_instructions {
        if let Some(current) = incoming.first().filter(|m| m.role == "system") {
            result.push(current.clone());
        }
    }
    result.extend(history);
    result.extend_from_slice(&incoming[pos + 1..]);
    result
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    use lr_providers::{FunctionCall, ToolCall};
    use std::sync::Arc;

    fn msg(role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            annotations: None,
            role: role.into(),
            content: ChatMessageContent::Text(text.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }
    }

    fn call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            tool_type: "function".into(),
            function: FunctionCall {
                name: name.into(),
                arguments: "{}".into(),
            },
        }
    }

    fn pending(
        mcp_handles: Vec<JoinHandle<(String, Result<String, String>)>>,
    ) -> PendingMixedExecution {
        let mut assistant = msg("assistant", "");
        assistant.tool_calls = Some(vec![
            call("gw-1", "fs__read"),
            call("gw-2", "fs__write"),
            call("client-1", "weather"),
        ]);
        PendingMixedExecution {
            full_assistant_message: assistant,
            mcp_handles,
            client_tool_call_ids: vec!["client-1".into()],
            messages_before_mixed: vec![
                msg("system", "Turn instructions"),
                msg("system", "Gateway instructions"),
                msg("system", "Another system message"),
                msg("user", "Do it"),
            ],
            started_at: Instant::now(),
            prior: PriorTurnTotals {
                iterations: 2,
                ..Default::default()
            },
            session_id: "s".into(),
        }
    }

    /// Written when the turn stops and again as each call finishes, without
    /// the instructions; restored, finished calls give their stored result
    /// and the rest an unknown outcome.
    #[tokio::test]
    async fn stored_execution_tracks_finished_calls_and_restores() {
        let written: Arc<parking_lot::Mutex<Vec<StoredPendingMixed>>> = Default::default();
        let sink = written.clone();
        let persist = PendingPersistence(Arc::new(move |p| sink.lock().push(p.clone())));
        let (release, wait) = tokio::sync::oneshot::channel::<()>();
        let mut execution = pending(vec![
            tokio::spawn(async { ("gw-1".to_string(), Ok("contents".to_string())) }),
            tokio::spawn(async move {
                let _ = wait.await;
                ("gw-2".to_string(), Err("never".to_string()))
            }),
        ]);
        execution.persist_with(
            persist,
            Some("Turn instructions"),
            Some("Gateway instructions"),
        );
        let first = written.lock()[0].clone();
        assert!(first.mcp_results.is_empty());
        assert_eq!(
            first
                .messages_before_mixed
                .iter()
                .map(|m| m.content.as_text())
                .collect::<Vec<_>>(),
            ["Another system message", "Do it"]
        );
        assert_eq!(first.prior.iterations, 2);

        // The first call finishes: written again with its result.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while written.lock().len() < 2 {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let latest = written.lock().last().unwrap().clone();
        assert_eq!(
            latest.mcp_results,
            [StoredToolResult {
                tool_call_id: "gw-1".into(),
                result: Ok("contents".into()),
            }]
        );

        // Dropping the execution stops the call still running: nothing more
        // is written.
        drop(execution);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let _ = release.send(());
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(written.lock().len(), 2);

        // Restored: the stored result, an unknown outcome for the other,
        // nothing for the client's call.
        let mut restored = PendingMixedExecution::from_stored(latest, String::new());
        let mut results = Vec::new();
        for handle in &mut restored.mcp_handles {
            results.push(handle.await.unwrap());
        }
        assert_eq!(
            results,
            [
                ("gw-1".to_string(), Ok("contents".to_string())),
                ("gw-2".to_string(), Err(UNKNOWN_OUTCOME.to_string())),
            ]
        );
        assert_eq!(restored.client_tool_call_ids, ["client-1"]);
    }
}
