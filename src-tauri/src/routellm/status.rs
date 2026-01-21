//! RouteLLM status tracking

use serde::{Deserialize, Serialize};

/// RouteLLM runtime state
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteLLMState {
    NotDownloaded,
    Downloading,
    DownloadedNotRunning,
    Initializing,
    Started,
}

/// RouteLLM status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteLLMStatus {
    pub state: RouteLLMState,
    pub memory_usage_mb: Option<u64>,
    pub last_access_secs_ago: Option<u64>,
}

/// Test prediction result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteLLMTestResult {
    pub is_strong: bool,
    pub win_rate: f32,
    pub latency_ms: u64,
}
