//! MCP-related types shared across crates

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// MCP Tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}
