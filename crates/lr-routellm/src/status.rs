//! RouteLLM status tracking
//!
//! Types in this module are returned from Tauri commands.
//! TypeScript mirror: src/types/tauri-commands.ts (RouteLLMStatus, RouteLLMTestResult)

use serde::{Deserialize, Serialize};

/// RouteLLM runtime state
/// TypeScript: RouteLLMState in src/types/tauri-commands.ts
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
/// TypeScript: RouteLLMStatus in src/types/tauri-commands.ts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteLLMStatus {
    pub state: RouteLLMState,
    pub memory_usage_mb: Option<u64>,
    pub last_access_secs_ago: Option<u64>,
}

/// Test prediction result from routellm_test_prediction command
/// TypeScript: RouteLLMTestResult in src/types/tauri-commands.ts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteLLMTestResult {
    pub is_strong: bool,
    pub win_rate: f32,
    pub latency_ms: u64,
}
