/// Streaming Notifications Module
///
/// Handles emission of synthetic notifications for streaming sessions,
/// particularly for deferred loading tool activation.

use crate::mcp::protocol::{JsonRpcNotification, JsonRpcResponse};
use serde_json::json;

/// Notification types that can be emitted through the streaming session
#[derive(Debug, Clone)]
pub enum StreamingNotificationType {
    /// Tools list has changed
    ToolsListChanged,
    /// Resources list has changed
    ResourcesListChanged,
    /// Prompts list has changed
    PromptsListChanged,
    /// Custom notification
    Custom { method: String, params: serde_json::Value },
}

impl StreamingNotificationType {
    /// Convert to JSON-RPC notification
    pub fn to_notification(&self) -> JsonRpcNotification {
        match self {
            StreamingNotificationType::ToolsListChanged => JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: "notifications/tools/list_changed".to_string(),
                params: None,
            },
            StreamingNotificationType::ResourcesListChanged => JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: "notifications/resources/list_changed".to_string(),
                params: None,
            },
            StreamingNotificationType::PromptsListChanged => JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: "notifications/prompts/list_changed".to_string(),
                params: None,
            },
            StreamingNotificationType::Custom { method, params } => JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: method.clone(),
                params: Some(params.clone()),
            },
        }
    }

    /// Get the method name for this notification
    pub fn method(&self) -> &str {
        match self {
            StreamingNotificationType::ToolsListChanged => "notifications/tools/list_changed",
            StreamingNotificationType::ResourcesListChanged => "notifications/resources/list_changed",
            StreamingNotificationType::PromptsListChanged => "notifications/prompts/list_changed",
            StreamingNotificationType::Custom { method, .. } => method,
        }
    }
}

/// Response to deferred loading tool activation
pub struct ToolActivationResponse {
    /// Number of tools activated
    pub tools_activated: usize,
    /// Number of resources activated
    pub resources_activated: usize,
    /// Number of prompts activated
    pub prompts_activated: usize,
    /// Total activated
    pub total_activated: usize,
    /// Success message
    pub message: String,
}

impl ToolActivationResponse {
    /// Create a new activation response
    pub fn new(tools: usize, resources: usize, prompts: usize) -> Self {
        let total = tools + resources + prompts;
        let message = format!(
            "Activated {} tool(s), {} resource(s), {} prompt(s)",
            tools, resources, prompts
        );

        Self {
            tools_activated: tools,
            resources_activated: resources,
            prompts_activated: prompts,
            total_activated: total,
            message,
        }
    }

    /// Convert to JSON-RPC response
    pub fn to_response(&self, request_id: serde_json::Value) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request_id,
            result: Some(json!({
                "tools_activated": self.tools_activated,
                "resources_activated": self.resources_activated,
                "prompts_activated": self.prompts_activated,
                "total_activated": self.total_activated,
                "message": self.message,
            })),
            error: None,
        }
    }
}

/// Event that triggers notifications in a streaming session
#[derive(Debug, Clone)]
pub enum StreamingSessionEvent {
    /// Tools were activated through deferred loading search
    ToolsActivated {
        tool_names: Vec<String>,
        server_id: String,
    },
    /// Resources were activated
    ResourcesActivated {
        resource_names: Vec<String>,
        server_id: String,
    },
    /// Prompts were activated
    PromptsActivated {
        prompt_names: Vec<String>,
        server_id: String,
    },
    /// Generic event that should trigger a notification
    Custom {
        notification_type: StreamingNotificationType,
        server_id: String,
    },
}

impl StreamingSessionEvent {
    /// Get the notification type for this event
    pub fn notification_type(&self) -> StreamingNotificationType {
        match self {
            StreamingSessionEvent::ToolsActivated { .. } => {
                StreamingNotificationType::ToolsListChanged
            }
            StreamingSessionEvent::ResourcesActivated { .. } => {
                StreamingNotificationType::ResourcesListChanged
            }
            StreamingSessionEvent::PromptsActivated { .. } => {
                StreamingNotificationType::PromptsListChanged
            }
            StreamingSessionEvent::Custom {
                notification_type, ..
            } => notification_type.clone(),
        }
    }

    /// Get the server ID for this event
    pub fn server_id(&self) -> &str {
        match self {
            StreamingSessionEvent::ToolsActivated { server_id, .. } => server_id,
            StreamingSessionEvent::ResourcesActivated { server_id, .. } => server_id,
            StreamingSessionEvent::PromptsActivated { server_id, .. } => server_id,
            StreamingSessionEvent::Custom { server_id, .. } => server_id,
        }
    }

    /// Get item names that were activated
    pub fn activated_items(&self) -> Vec<&str> {
        match self {
            StreamingSessionEvent::ToolsActivated { tool_names, .. } => {
                tool_names.iter().map(|s| s.as_str()).collect()
            }
            StreamingSessionEvent::ResourcesActivated {
                resource_names, ..
            } => resource_names.iter().map(|s| s.as_str()).collect(),
            StreamingSessionEvent::PromptsActivated { prompt_names, .. } => {
                prompt_names.iter().map(|s| s.as_str()).collect()
            }
            StreamingSessionEvent::Custom { .. } => vec![],
        }
    }

    /// Get a summary message for this event
    pub fn summary(&self) -> String {
        match self {
            StreamingSessionEvent::ToolsActivated { tool_names, server_id } => {
                format!(
                    "Activated {} tool(s) on {}: {}",
                    tool_names.len(),
                    server_id,
                    tool_names.join(", ")
                )
            }
            StreamingSessionEvent::ResourcesActivated {
                resource_names,
                server_id,
            } => {
                format!(
                    "Activated {} resource(s) on {}: {}",
                    resource_names.len(),
                    server_id,
                    resource_names.join(", ")
                )
            }
            StreamingSessionEvent::PromptsActivated {
                prompt_names,
                server_id,
            } => {
                format!(
                    "Activated {} prompt(s) on {}: {}",
                    prompt_names.len(),
                    server_id,
                    prompt_names.join(", ")
                )
            }
            StreamingSessionEvent::Custom {
                notification_type,
                server_id,
            } => format!("Notification from {}: {}", server_id, notification_type.method()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_type_to_notification() {
        let notif = StreamingNotificationType::ToolsListChanged.to_notification();
        assert_eq!(notif.method, "notifications/tools/list_changed");
        assert_eq!(notif.jsonrpc, "2.0");
        assert!(notif.params.is_none());
    }

    #[test]
    fn test_tool_activation_response() {
        let response = ToolActivationResponse::new(3, 2, 1);
        assert_eq!(response.total_activated, 6);
        assert_eq!(response.tools_activated, 3);
        assert_eq!(response.resources_activated, 2);
        assert_eq!(response.prompts_activated, 1);
        assert!(response.message.contains("Activated"));
    }

    #[test]
    fn test_session_event_notification_type() {
        let event = StreamingSessionEvent::ToolsActivated {
            tool_names: vec!["read_file".to_string()],
            server_id: "filesystem".to_string(),
        };

        match event.notification_type() {
            StreamingNotificationType::ToolsListChanged => {}
            _ => panic!("Wrong notification type"),
        }
    }

    #[test]
    fn test_session_event_summary() {
        let event = StreamingSessionEvent::ToolsActivated {
            tool_names: vec!["read_file".to_string(), "write_file".to_string()],
            server_id: "filesystem".to_string(),
        };

        let summary = event.summary();
        assert!(summary.contains("Activated 2 tool(s)"));
        assert!(summary.contains("filesystem"));
        assert!(summary.contains("read_file"));
        assert!(summary.contains("write_file"));
    }
}
