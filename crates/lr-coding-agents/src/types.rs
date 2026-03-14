//! Core types for coding agent sessions.

use chrono::{DateTime, Utc};
use lr_config::{CodingAgentType, CodingPermissionMode};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;

/// Hard cap for output buffer size to prevent excessive memory usage
const MAX_OUTPUT_BUFFER_SIZE: usize = 10_000;

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

    /// The underlying agent's session ID (e.g., Claude Code's conversation ID).
    /// Used for `--resume` on follow-up messages to preserve context.
    pub agent_session_id: Option<String>,
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
        let output_buffer_max = output_buffer_max.min(MAX_OUTPUT_BUFFER_SIZE);
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
            initial_prompt,
            result: None,
            error: None,
            cost_usd: None,
            turn_count: None,
            created_at: now,
            last_activity: now,
            exit_code: None,
            agent_session_id: None,
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
    Done,
    Error,
    Interrupted,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Active => write!(f, "active"),
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
    /// Stdin writer for sending messages (piped for Claude Code control protocol)
    pub stdin: Option<tokio::process::ChildStdin>,
    /// Cancellation token for interrupting the agent
    pub cancel: tokio_util::sync::CancellationToken,
    /// Cancellation token for stopping the output reader task.
    /// Cancelled when the session is resumed with a new process, so the old
    /// reader stops cleanly instead of racing with the new one.
    pub reader_cancel: tokio_util::sync::CancellationToken,
}

// ── MCP tool response types ──

/// Response from start tool
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResponse {
    pub session_id: String,
    pub status: SessionStatus,
}

/// Response from say tool (combined say + interrupt)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SayResponse {
    pub session_id: String,
    pub status: SessionStatus,
    /// True if the session was interrupted before sending the message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupted: Option<bool>,
    /// True if the session was resumed (context preserved via --resume)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resumed: Option<bool>,
}

/// Response from status tool
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusResponse {
    pub session_id: String,
    pub status: SessionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    pub recent_output: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_count: Option<u32>,
}

/// Session summary for list tool
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    pub agent_type: CodingAgentType,
    pub client_id: String,
    pub working_directory: String,
    pub display_text: String,
    pub timestamp: DateTime<Utc>,
    pub status: SessionStatus,
}

/// Response from list tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse {
    pub sessions: Vec<SessionSummary>,
}

/// Detailed session info for admin/UI
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub session_id: String,
    pub agent_type: CodingAgentType,
    pub client_id: String,
    pub working_directory: String,
    pub display_text: String,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub recent_output: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session(buffer_max: usize) -> CodingSession {
        CodingSession::new(
            "test-id".to_string(),
            CodingAgentType::ClaudeCode,
            "client-1".to_string(),
            PathBuf::from("/tmp/test"),
            SessionConfig {
                model: None,
                permission_mode: lr_config::CodingPermissionMode::Supervised,
                env: Default::default(),
            },
            "initial prompt".to_string(),
            buffer_max,
        )
    }

    #[test]
    fn test_session_new_defaults() {
        let session = make_session(100);
        assert_eq!(session.id, "test-id");
        assert_eq!(session.agent_type, CodingAgentType::ClaudeCode);
        assert_eq!(session.client_id, "client-1");
        assert_eq!(session.status, SessionStatus::Active);
        assert!(session.process.is_none());
        assert!(session.output_buffer.is_empty());
        assert_eq!(session.initial_prompt, "initial prompt");
        assert!(session.result.is_none());
        assert!(session.error.is_none());
        assert!(session.cost_usd.is_none());
        assert!(session.turn_count.is_none());
        assert!(session.exit_code.is_none());
        assert!(session.agent_session_id.is_none());
    }

    #[test]
    fn test_append_output() {
        let mut session = make_session(3);
        session.append_output("line 1".to_string());
        session.append_output("line 2".to_string());
        session.append_output("line 3".to_string());
        assert_eq!(session.output_buffer.len(), 3);

        // Adding one more should evict the oldest
        session.append_output("line 4".to_string());
        assert_eq!(session.output_buffer.len(), 3);
        assert_eq!(session.output_buffer[0], "line 2");
        assert_eq!(session.output_buffer[1], "line 3");
        assert_eq!(session.output_buffer[2], "line 4");
    }

    #[test]
    fn test_recent_output() {
        let mut session = make_session(100);
        for i in 1..=10 {
            session.append_output(format!("line {}", i));
        }

        // Request last 3 lines
        let recent = session.recent_output(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0], "line 8");
        assert_eq!(recent[1], "line 9");
        assert_eq!(recent[2], "line 10");

        // Request more than available
        let all = session.recent_output(100);
        assert_eq!(all.len(), 10);

        // Request 0
        let empty = session.recent_output(0);
        assert!(empty.is_empty());
    }

    #[test]
    fn test_output_buffer_ring_behavior() {
        let mut session = make_session(5);
        for i in 1..=20 {
            session.append_output(format!("line {}", i));
        }
        // Only last 5 should remain
        assert_eq!(session.output_buffer.len(), 5);
        let recent = session.recent_output(5);
        assert_eq!(
            recent,
            vec!["line 16", "line 17", "line 18", "line 19", "line 20"]
        );
    }

    #[test]
    fn test_session_status_display() {
        assert_eq!(SessionStatus::Active.to_string(), "active");
        assert_eq!(SessionStatus::Done.to_string(), "done");
        assert_eq!(SessionStatus::Error.to_string(), "error");
        assert_eq!(SessionStatus::Interrupted.to_string(), "interrupted");
    }

    #[test]
    fn test_session_status_serde() {
        let parsed: SessionStatus = serde_json::from_str("\"done\"").unwrap();
        assert_eq!(parsed, SessionStatus::Done);
    }

    #[test]
    fn test_start_response_serde() {
        let resp = StartResponse {
            session_id: "abc-123".to_string(),
            status: SessionStatus::Active,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["sessionId"], "abc-123");
        assert_eq!(json["status"], "active");
    }

    #[test]
    fn test_status_response_serde_skips_none() {
        let resp = StatusResponse {
            session_id: "abc".to_string(),
            status: SessionStatus::Active,
            result: None,
            recent_output: vec!["hello".to_string()],
            cost_usd: None,
            turn_count: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json.get("result").is_none());
        assert!(json.get("costUsd").is_none());
        assert!(json.get("turnCount").is_none());
        assert_eq!(json["recentOutput"][0], "hello");
    }

    #[test]
    fn test_list_response_serde() {
        let resp = ListResponse {
            sessions: vec![SessionSummary {
                session_id: "s1".to_string(),
                agent_type: CodingAgentType::ClaudeCode,
                client_id: "c1".to_string(),
                working_directory: "/tmp".to_string(),
                display_text: "hello".to_string(),
                timestamp: Utc::now(),
                status: SessionStatus::Done,
            }],
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["sessions"][0]["sessionId"], "s1");
        assert_eq!(json["sessions"][0]["agentType"], "claude_code");
        assert_eq!(json["sessions"][0]["clientId"], "c1");
        assert_eq!(json["sessions"][0]["status"], "done");
    }

    #[test]
    fn test_say_response_serde() {
        let resp = SayResponse {
            session_id: "abc".to_string(),
            status: SessionStatus::Active,
            interrupted: Some(true),
            resumed: Some(true),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["interrupted"], true);
        assert_eq!(json["resumed"], true);

        // Without optional fields
        let resp2 = SayResponse {
            session_id: "abc".to_string(),
            status: SessionStatus::Active,
            interrupted: None,
            resumed: None,
        };
        let json2 = serde_json::to_value(&resp2).unwrap();
        assert!(json2.get("interrupted").is_none());
        assert!(json2.get("resumed").is_none());
    }
}
