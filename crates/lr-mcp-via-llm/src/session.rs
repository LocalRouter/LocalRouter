//! Session management for MCP via LLM
//!
//! Each session tracks conversation history including injected tool calls
//! that the client never sees. Sessions are matched to incoming requests
//! via per-message hashing.

use std::time::Instant;

use lr_providers::ChatMessage;
use tokio::task::JoinHandle;

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
    pub session_id: String,
    pub client_id: String,
    /// Key used to identify this session in the MCP gateway
    pub gateway_session_key: String,
    /// Whether the gateway session has been initialized
    pub gateway_initialized: bool,
    /// Conversation history (including injected tool calls)
    pub history: SessionHistory,
    /// Last time this session was active
    pub last_activity: Instant,
}

impl McpViaLlmSession {
    pub fn new(session_id: String, client_id: String) -> Self {
        let gateway_session_key = format!("mcp-via-llm-{}", session_id);
        Self {
            session_id,
            client_id,
            gateway_session_key,
            gateway_initialized: false,
            history: SessionHistory::new(),
            last_activity: Instant::now(),
        }
    }

    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    pub fn is_expired(&self, ttl: std::time::Duration) -> bool {
        self.last_activity.elapsed() > ttl
    }
}
