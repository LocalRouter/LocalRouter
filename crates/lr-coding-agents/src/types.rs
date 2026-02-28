//! Core types for coding agent sessions.

use chrono::{DateTime, Utc};
use lr_config::{CodingAgentType, CodingPermissionMode};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;

/// Unique session identifier
pub type SessionId = String;

/// A coding agent session
pub struct CodingSession {
    /// Unique session ID
    pub id: SessionId,
    /// Which agent type
    pub agent_type: CodingAgentType,
    /// Owning client ID (immutable)
    pub client_id: String,
    /// Current status
    pub status: SessionStatus,
    /// Working directory
    pub working_directory: PathBuf,
    /// Original config for re-spawning on resume
    pub config: SessionConfig,

    /// Process handle (None when session is done/not yet started)
    pub process: Option<AgentProcess>,

    /// Output ring buffer
    pub output_buffer: VecDeque<String>,
    /// Max buffer size (from config)
    pub output_buffer_max: usize,

    /// Pending question (at most one — agent blocks until answered)
    pub pending_question: Option<PendingQuestion>,

    /// Initial prompt text
    pub initial_prompt: String,
    /// Final result (when done)
    pub result: Option<String>,
    /// Error message (when error)
    pub error: Option<String>,
    /// Estimated cost in USD
    pub cost_usd: Option<f64>,
    /// Number of agent turns
    pub turn_count: Option<u32>,

    pub created_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub exit_code: Option<i32>,
}

impl CodingSession {
    pub fn new(
        id: SessionId,
        agent_type: CodingAgentType,
        client_id: String,
        working_directory: PathBuf,
        config: SessionConfig,
        initial_prompt: String,
        output_buffer_max: usize,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            agent_type,
            client_id,
            status: SessionStatus::Active,
            working_directory,
            config,
            process: None,
            output_buffer: VecDeque::with_capacity(output_buffer_max.min(1000)),
            output_buffer_max,
            pending_question: None,
            initial_prompt,
            result: None,
            error: None,
            cost_usd: None,
            turn_count: None,
            created_at: now,
            last_activity: now,
            exit_code: None,
        }
    }

    /// Append output lines to the ring buffer
    pub fn append_output(&mut self, line: String) {
        if self.output_buffer.len() >= self.output_buffer_max {
            self.output_buffer.pop_front();
        }
        self.output_buffer.push_back(line);
        self.last_activity = Utc::now();
    }

    /// Get recent output lines
    pub fn recent_output(&self, count: usize) -> Vec<String> {
        let start = self.output_buffer.len().saturating_sub(count);
        self.output_buffer.iter().skip(start).cloned().collect()
    }
}

/// Session configuration (stored for resume)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    pub model: Option<String>,
    pub permission_mode: CodingPermissionMode,
    pub env: std::collections::HashMap<String, String>,
}

/// Session status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    AwaitingInput,
    Done,
    Error,
    Interrupted,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Active => write!(f, "active"),
            SessionStatus::AwaitingInput => write!(f, "awaiting_input"),
            SessionStatus::Done => write!(f, "done"),
            SessionStatus::Error => write!(f, "error"),
            SessionStatus::Interrupted => write!(f, "interrupted"),
        }
    }
}

/// Handle to a running agent process
pub struct AgentProcess {
    /// OS child process group
    pub child: command_group::AsyncGroupChild,
    /// Stdin writer for sending messages
    pub stdin: Option<tokio::process::ChildStdin>,
    /// Cancellation token
    pub cancel: tokio_util::sync::CancellationToken,
}

/// A pending question from the agent
pub struct PendingQuestion {
    /// Unique question ID
    pub id: String,
    /// Type of question
    pub question_type: QuestionType,
    /// Individual questions (each with options)
    pub questions: Vec<QuestionItem>,
    /// Channel to send the response back to the blocked executor
    pub resolve: tokio::sync::oneshot::Sender<ApprovalResponse>,
}

/// Type of pending question
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuestionType {
    ToolApproval,
    PlanApproval,
    Question,
}

/// A single question with options
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionItem {
    pub question: String,
    pub options: Vec<String>,
}

/// Response to a pending question
#[derive(Debug, Clone)]
pub enum ApprovalResponse {
    Approved,
    Denied { reason: Option<String> },
    Answered { answers: Vec<String> },
}

// ── MCP tool response types ──

/// Response from {agent}_start
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResponse {
    pub session_id: String,
    pub status: SessionStatus,
}

/// Response from {agent}_say
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SayResponse {
    pub session_id: String,
    pub status: SessionStatus,
}

/// Response from {agent}_status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusResponse {
    pub session_id: String,
    pub status: SessionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    pub recent_output: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_question: Option<PendingQuestionInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_count: Option<u32>,
}

/// Serializable info about a pending question (without the oneshot channel)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingQuestionInfo {
    pub id: String,
    #[serde(rename = "type")]
    pub question_type: QuestionType,
    pub questions: Vec<QuestionItem>,
}

/// Response from {agent}_respond
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RespondResponse {
    pub session_id: String,
    pub status: SessionStatus,
}

/// Response from {agent}_interrupt
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptResponse {
    pub session_id: String,
    pub status: SessionStatus,
}

/// Session summary for {agent}_list
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    pub working_directory: String,
    pub display_text: String,
    pub timestamp: DateTime<Utc>,
    pub status: SessionStatus,
}

/// Response from {agent}_list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse {
    pub sessions: Vec<SessionSummary>,
}
