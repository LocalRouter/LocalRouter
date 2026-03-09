//! Session management for MCP via LLM
//!
//! Each session tracks conversation history including injected tool calls
//! that the client never sees. Sessions are matched to incoming requests
//! via per-message hashing.

use std::time::Instant;

use lr_providers::ChatMessage;

/// Tracks the full conversation history for an MCP via LLM session,
/// including messages the client never sees (injected tool calls/results).
#[allow(dead_code)]
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

    /// Append messages to the history
    pub fn extend(&mut self, messages: impl IntoIterator<Item = ChatMessage>) {
        self.full_messages.extend(messages);
    }
}

/// A single MCP via LLM session tied to one client
#[allow(dead_code)]
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
