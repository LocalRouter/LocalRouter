//! Firewall manager for MCP Gateway
//!
//! Provides per-client tool call interception with Allow/Ask/Deny policies.
//! When a tool call hits "Ask" policy, it holds the request pending user approval
//! via a Tauri popup window, system tray integration, and SSE notifications.
//!
//! Follows the same oneshot-channel pattern as `ElicitationManager`.

#![allow(dead_code)]

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::protocol::JsonRpcNotification;
use lr_types::{AppError, AppResult};

/// User's response to a firewall approval request
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FirewallApprovalAction {
    /// Deny this tool call
    Deny,
    /// Allow this single tool call
    AllowOnce,
    /// Allow this tool for the rest of the session
    AllowSession,
}

/// Response from the user for a firewall approval request
#[derive(Debug)]
pub struct FirewallApprovalResponse {
    pub action: FirewallApprovalAction,
}

/// Pending firewall approval session
#[derive(Debug)]
pub struct FirewallApprovalSession {
    /// Unique request ID
    pub request_id: String,

    /// Client ID that owns this session
    pub client_id: String,

    /// Human-readable client name
    pub client_name: String,

    /// Namespaced tool name (e.g. "filesystem__write_file")
    pub tool_name: String,

    /// Human-readable server or skill name
    pub server_name: String,

    /// Preview of tool call arguments (truncated JSON)
    pub arguments_preview: String,

    /// Channel to send response back to waiting request
    pub response_sender: Option<oneshot::Sender<FirewallApprovalResponse>>,

    /// When this request was created
    pub created_at: Instant,

    /// Timeout in seconds
    pub timeout_seconds: u64,
}

impl FirewallApprovalSession {
    /// Check if this session has expired
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > Duration::from_secs(self.timeout_seconds)
    }
}

/// Info about a pending approval (for UI display, without the oneshot channel)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApprovalInfo {
    pub request_id: String,
    pub client_id: String,
    pub client_name: String,
    pub tool_name: String,
    pub server_name: String,
    pub arguments_preview: String,
    pub created_at_secs_ago: u64,
    pub timeout_seconds: u64,
}

/// Manages firewall approval lifecycle for MCP gateway
pub struct FirewallManager {
    /// Pending approval sessions (request_id -> session)
    pending: Arc<DashMap<String, FirewallApprovalSession>>,

    /// Default timeout for approval requests (seconds)
    default_timeout_secs: u64,

    /// Broadcast sender for SSE notifications (optional)
    notification_broadcast:
        Option<Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>>,
}

impl FirewallManager {
    /// Create a new firewall manager
    pub fn new(default_timeout_secs: u64) -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            default_timeout_secs,
            notification_broadcast: None,
        }
    }

    /// Create a new firewall manager with SSE broadcast support
    pub fn new_with_broadcast(
        default_timeout_secs: u64,
        notification_broadcast: Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>,
    ) -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            default_timeout_secs,
            notification_broadcast: Some(notification_broadcast),
        }
    }

    /// Request user approval for a tool call
    ///
    /// This is an async operation that waits for the user response.
    /// Returns the user's action, or auto-denies on timeout.
    pub async fn request_approval(
        &self,
        client_id: String,
        client_name: String,
        tool_name: String,
        server_name: String,
        arguments_preview: String,
        timeout_secs: Option<u64>,
    ) -> AppResult<FirewallApprovalResponse> {
        let request_id = Uuid::new_v4().to_string();
        let timeout = timeout_secs.unwrap_or(self.default_timeout_secs);

        debug!(
            "Creating firewall approval request {} for client {} tool {} (timeout: {}s)",
            request_id, client_id, tool_name, timeout
        );

        // Create response channel
        let (tx, rx) = oneshot::channel();

        // Create session
        let session = FirewallApprovalSession {
            request_id: request_id.clone(),
            client_id: client_id.clone(),
            client_name: client_name.clone(),
            tool_name: tool_name.clone(),
            server_name: server_name.clone(),
            arguments_preview: arguments_preview.clone(),
            response_sender: Some(tx),
            created_at: Instant::now(),
            timeout_seconds: timeout,
        };

        // Store session
        self.pending.insert(request_id.clone(), session);

        info!(
            "Firewall approval request {} created: client={}, tool={}",
            request_id, client_id, tool_name
        );

        // Send SSE notification to connected clients
        if let Some(broadcast) = &self.notification_broadcast {
            let notification = JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: "firewall/approvalRequired".to_string(),
                params: Some(json!({
                    "request_id": request_id,
                    "client_id": client_id,
                    "client_name": client_name,
                    "tool_name": tool_name,
                    "server_name": server_name,
                    "arguments_preview": arguments_preview,
                    "timeout_seconds": timeout,
                })),
            };

            if let Err(e) = broadcast.send(("_firewall".to_string(), notification)) {
                debug!("Failed to broadcast firewall approval request: {}", e);
            }
        }

        // Wait for response with timeout
        match tokio::time::timeout(Duration::from_secs(timeout), rx).await {
            Ok(Ok(response)) => {
                info!(
                    "Received firewall approval response for request {}: {:?}",
                    request_id, response.action
                );
                self.pending.remove(&request_id);
                Ok(response)
            }
            Ok(Err(_)) => {
                // Channel closed (cancelled)
                warn!("Firewall approval request {} was cancelled", request_id);
                self.pending.remove(&request_id);
                Ok(FirewallApprovalResponse {
                    action: FirewallApprovalAction::Deny,
                })
            }
            Err(_) => {
                // Timeout — auto-deny
                warn!(
                    "Firewall approval request {} timed out after {}s — auto-denying",
                    request_id, timeout
                );
                self.pending.remove(&request_id);
                Ok(FirewallApprovalResponse {
                    action: FirewallApprovalAction::Deny,
                })
            }
        }
    }

    /// Submit a user response to a pending firewall approval request
    pub fn submit_response(
        &self,
        request_id: &str,
        action: FirewallApprovalAction,
    ) -> AppResult<()> {
        match self.pending.remove(request_id) {
            Some((_, mut session)) => {
                debug!(
                    "Submitting firewall approval response for request {}: {:?}",
                    request_id, action
                );

                if let Some(sender) = session.response_sender.take() {
                    let response = FirewallApprovalResponse { action };
                    sender.send(response).map_err(|_| {
                        AppError::Internal("Failed to send firewall approval response".to_string())
                    })?;
                }

                info!(
                    "Firewall approval response submitted for request {}",
                    request_id
                );

                Ok(())
            }
            None => {
                warn!(
                    "Attempted to submit response for unknown firewall request {}",
                    request_id
                );
                Err(AppError::InvalidParams(format!(
                    "Firewall approval request {} not found or expired",
                    request_id
                )))
            }
        }
    }

    /// Cancel a pending approval request
    pub fn cancel_request(&self, request_id: &str) -> AppResult<()> {
        match self.pending.remove(request_id) {
            Some(_) => {
                info!("Cancelled firewall approval request {}", request_id);
                Ok(())
            }
            None => Err(AppError::InvalidParams(format!(
                "Firewall approval request {} not found",
                request_id
            ))),
        }
    }

    /// List all pending approval requests (for UI display)
    pub fn list_pending(&self) -> Vec<PendingApprovalInfo> {
        self.pending
            .iter()
            .map(|entry| {
                let session = entry.value();
                PendingApprovalInfo {
                    request_id: session.request_id.clone(),
                    client_id: session.client_id.clone(),
                    client_name: session.client_name.clone(),
                    tool_name: session.tool_name.clone(),
                    server_name: session.server_name.clone(),
                    arguments_preview: session.arguments_preview.clone(),
                    created_at_secs_ago: session.created_at.elapsed().as_secs(),
                    timeout_seconds: session.timeout_seconds,
                }
            })
            .collect()
    }

    /// Insert a pre-built pending approval session (for debug/testing)
    pub fn insert_pending(&self, request_id: String, session: FirewallApprovalSession) {
        self.pending.insert(request_id, session);
    }

    /// Get the number of pending requests
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Check if there are any pending requests
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Clean up expired sessions and return the list of expired request IDs
    pub fn cleanup_expired(&self) -> Vec<String> {
        let expired: Vec<String> = self
            .pending
            .iter()
            .filter(|entry| entry.value().is_expired())
            .map(|entry| entry.key().clone())
            .collect();

        for request_id in &expired {
            warn!(
                "Cleaning up expired firewall approval request {}",
                request_id
            );
            self.pending.remove(request_id);
        }

        expired
    }
}

impl Default for FirewallManager {
    fn default() -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            default_timeout_secs: 120,
            notification_broadcast: None,
        }
    }
}

impl Clone for FirewallManager {
    fn clone(&self) -> Self {
        Self {
            pending: self.pending.clone(),
            default_timeout_secs: self.default_timeout_secs,
            notification_broadcast: self.notification_broadcast.clone(),
        }
    }
}

/// Truncate a JSON value to a preview string suitable for display
pub fn truncate_arguments_preview(args: &serde_json::Value, max_len: usize) -> String {
    let s = serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string());
    if s.len() <= max_len {
        s
    } else {
        // Find a valid UTF-8 boundary at or before max_len
        let truncated = match s.char_indices().take_while(|(i, _)| *i < max_len).last() {
            Some((i, c)) => &s[..i + c.len_utf8()],
            None => "",
        };
        format!("{}...", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_firewall_manager_creation() {
        let manager = FirewallManager::new(60);
        assert_eq!(manager.pending_count(), 0);
        assert_eq!(manager.default_timeout_secs, 60);
    }

    #[test]
    fn test_session_expiry() {
        let session = FirewallApprovalSession {
            request_id: "test-123".to_string(),
            client_id: "client-1".to_string(),
            client_name: "Test Client".to_string(),
            tool_name: "filesystem__write_file".to_string(),
            server_name: "filesystem".to_string(),
            arguments_preview: "{}".to_string(),
            response_sender: None,
            created_at: Instant::now() - Duration::from_secs(150),
            timeout_seconds: 120,
        };
        assert!(session.is_expired());
    }

    #[test]
    fn test_session_not_expired() {
        let session = FirewallApprovalSession {
            request_id: "test-123".to_string(),
            client_id: "client-1".to_string(),
            client_name: "Test Client".to_string(),
            tool_name: "filesystem__write_file".to_string(),
            server_name: "filesystem".to_string(),
            arguments_preview: "{}".to_string(),
            response_sender: None,
            created_at: Instant::now(),
            timeout_seconds: 120,
        };
        assert!(!session.is_expired());
    }

    #[tokio::test]
    async fn test_submit_response() {
        let manager = FirewallManager::new(120);

        // Start a request in the background
        let manager_clone = manager.clone();
        let handle = tokio::spawn(async move {
            manager_clone
                .request_approval(
                    "client-1".to_string(),
                    "Test Client".to_string(),
                    "filesystem__write_file".to_string(),
                    "filesystem".to_string(),
                    "{}".to_string(),
                    None,
                )
                .await
        });

        // Give it time to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Should have 1 pending
        assert_eq!(manager.pending_count(), 1);

        // Submit response
        let pending = manager.list_pending();
        let request_id = &pending[0].request_id;
        manager
            .submit_response(request_id, FirewallApprovalAction::AllowOnce)
            .unwrap();

        // Should complete successfully
        let result = handle.await.unwrap();
        assert!(result.is_ok());
        assert_eq!(result.unwrap().action, FirewallApprovalAction::AllowOnce);
    }

    #[tokio::test]
    async fn test_timeout_auto_deny() {
        let manager = FirewallManager::new(1); // 1 second timeout

        let result = manager
            .request_approval(
                "client-1".to_string(),
                "Test Client".to_string(),
                "tool".to_string(),
                "server".to_string(),
                "{}".to_string(),
                None,
            )
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().action, FirewallApprovalAction::Deny);
    }

    #[tokio::test]
    async fn test_cancel_request() {
        let manager = FirewallManager::new(120);

        // Start a request in the background
        let manager_clone = manager.clone();
        let handle = tokio::spawn(async move {
            manager_clone
                .request_approval(
                    "client-1".to_string(),
                    "Test Client".to_string(),
                    "tool".to_string(),
                    "server".to_string(),
                    "{}".to_string(),
                    None,
                )
                .await
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(manager.pending_count(), 1);

        let request_id = manager.list_pending()[0].request_id.clone();
        manager.cancel_request(&request_id).unwrap();
        assert_eq!(manager.pending_count(), 0);

        // Cancelled = auto-deny
        let result = handle.await.unwrap();
        assert!(result.is_ok());
        assert_eq!(result.unwrap().action, FirewallApprovalAction::Deny);
    }

    #[test]
    fn test_truncate_arguments_preview() {
        let short = serde_json::json!({"a": 1});
        assert_eq!(truncate_arguments_preview(&short, 100), "{\"a\":1}");

        let long = serde_json::json!({"content": "a".repeat(200)});
        let preview = truncate_arguments_preview(&long, 50);
        assert!(preview.len() <= 54); // 50 + "..."
        assert!(preview.ends_with("..."));
    }

    #[test]
    fn test_firewall_rules_resolution() {
        use lr_config::{FirewallPolicy, FirewallRules};
        use std::collections::HashMap;

        // Test default policy
        let rules = FirewallRules::default();
        assert_eq!(
            rules.resolve_mcp_tool("any_tool", "any_server"),
            &FirewallPolicy::Allow
        );

        // Test tool-level override
        let mut tool_rules = HashMap::new();
        tool_rules.insert("filesystem__write_file".to_string(), FirewallPolicy::Deny);
        let rules = FirewallRules {
            default_policy: FirewallPolicy::Allow,
            tool_rules,
            ..Default::default()
        };
        assert_eq!(
            rules.resolve_mcp_tool("filesystem__write_file", "server-1"),
            &FirewallPolicy::Deny
        );
        assert_eq!(
            rules.resolve_mcp_tool("filesystem__read_file", "server-1"),
            &FirewallPolicy::Allow
        );

        // Test server-level override
        let mut server_rules = HashMap::new();
        server_rules.insert("server-1".to_string(), FirewallPolicy::Ask);
        let rules = FirewallRules {
            default_policy: FirewallPolicy::Allow,
            server_rules,
            ..Default::default()
        };
        assert_eq!(
            rules.resolve_mcp_tool("any_tool", "server-1"),
            &FirewallPolicy::Ask
        );
        assert_eq!(
            rules.resolve_mcp_tool("any_tool", "server-2"),
            &FirewallPolicy::Allow
        );

        // Test tool overrides server
        let mut tool_rules = HashMap::new();
        tool_rules.insert("filesystem__write_file".to_string(), FirewallPolicy::Deny);
        let mut server_rules = HashMap::new();
        server_rules.insert("server-1".to_string(), FirewallPolicy::Ask);
        let rules = FirewallRules {
            default_policy: FirewallPolicy::Allow,
            tool_rules,
            server_rules,
            ..Default::default()
        };
        // Tool rule takes precedence over server rule
        assert_eq!(
            rules.resolve_mcp_tool("filesystem__write_file", "server-1"),
            &FirewallPolicy::Deny
        );
        // Server rule still applies for other tools
        assert_eq!(
            rules.resolve_mcp_tool("filesystem__read_file", "server-1"),
            &FirewallPolicy::Ask
        );

        // Test skill tool resolution
        let mut skill_tool_rules = HashMap::new();
        skill_tool_rules.insert("skill_deploy_run_script".to_string(), FirewallPolicy::Deny);
        let mut skill_rules = HashMap::new();
        skill_rules.insert("deploy".to_string(), FirewallPolicy::Ask);
        let rules = FirewallRules {
            default_policy: FirewallPolicy::Allow,
            skill_tool_rules,
            skill_rules,
            ..Default::default()
        };
        // Skill tool rule takes precedence
        assert_eq!(
            rules.resolve_skill_tool("skill_deploy_run_script", "deploy"),
            &FirewallPolicy::Deny
        );
        // Skill rule for other tools
        assert_eq!(
            rules.resolve_skill_tool("skill_deploy_get_info", "deploy"),
            &FirewallPolicy::Ask
        );
        // Default for unknown skill
        assert_eq!(
            rules.resolve_skill_tool("skill_other_tool", "other"),
            &FirewallPolicy::Allow
        );
    }
}
