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
    /// Deny this single tool call
    Deny,
    /// Deny this tool for the rest of the session (MCP/Skills)
    DenySession,
    /// Deny permanently by updating client permissions to Off
    DenyAlways,
    /// Allow this single tool call
    AllowOnce,
    /// Allow this tool for the rest of the session (MCP/Skills)
    AllowSession,
    /// Allow this for 1 hour (Models & guardrails: time-based bypass)
    #[serde(rename = "allow_1_hour")]
    Allow1Hour,
    /// Allow permanently by updating client permissions to Allow
    AllowPermanent,
    /// Block flagged categories (guardrail-specific: set categories to "block" action)
    BlockCategories,
    /// Allow flagged categories (guardrail-specific: set categories to "allow" action)
    AllowCategories,
    /// Deny all guardrail-flagged content for 1 hour (auto-deny without popup)
    #[serde(rename = "deny_1_hour")]
    Deny1Hour,
}

/// Response from the user for a firewall approval request
#[derive(Debug)]
pub struct FirewallApprovalResponse {
    pub action: FirewallApprovalAction,
    /// Edited tool arguments or model params (from edit mode in the popup)
    pub edited_arguments: Option<serde_json::Value>,
}

/// Guardrail approval details (sent to popup)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailApprovalDetails {
    /// Per-model safety verdicts
    pub verdicts: Vec<serde_json::Value>,
    /// Actions required per flagged category
    pub actions_required: Vec<serde_json::Value>,
    pub total_duration_ms: u64,
    pub scan_direction: String,
    /// The text content that was scanned and triggered the guardrail
    pub flagged_text: String,
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

    /// Namespaced tool name (e.g. "filesystem__write_file") or model name for model requests
    pub tool_name: String,

    /// Human-readable server or skill name, or provider name for model requests
    pub server_name: String,

    /// Preview of tool call arguments (truncated JSON)
    pub arguments_preview: String,

    /// Full arguments for edit mode (complete JSON, not truncated)
    pub full_arguments: Option<serde_json::Value>,

    /// Channel to send response back to waiting request
    pub response_sender: Option<oneshot::Sender<FirewallApprovalResponse>>,

    /// When this request was created
    pub created_at: Instant,

    /// Timeout in seconds
    pub timeout_seconds: u64,

    /// Whether this is a model approval request (vs MCP/skill tool)
    pub is_model_request: bool,

    /// Whether this is a guardrail request
    pub is_guardrail_request: bool,

    /// Guardrail-specific details (matches, severity, etc.)
    pub guardrail_details: Option<GuardrailApprovalDetails>,
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
    /// Full arguments as JSON string (for edit mode, lazy-loaded by UI)
    pub full_arguments: Option<String>,
    pub created_at_secs_ago: u64,
    pub timeout_seconds: u64,
    /// Whether this is a model approval request (vs MCP/skill tool)
    #[serde(default)]
    pub is_model_request: bool,
    /// Whether this is a guardrail request
    #[serde(default)]
    pub is_guardrail_request: bool,
    /// Guardrail-specific details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guardrail_details: Option<GuardrailApprovalDetails>,
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
        full_arguments: Option<serde_json::Value>,
    ) -> AppResult<FirewallApprovalResponse> {
        self.request_approval_internal(
            client_id,
            client_name,
            tool_name,
            server_name,
            arguments_preview,
            timeout_secs,
            false, // MCP/skill tool request
            false, // not guardrail
            full_arguments,
            None,
        )
        .await
    }

    /// Request user approval for a model access
    ///
    /// Similar to `request_approval` but for LLM model access.
    /// The popup shows different options (Allow for 1 Hour instead of Allow for Session).
    pub async fn request_model_approval(
        &self,
        client_id: String,
        client_name: String,
        model_name: String,
        provider_name: String,
        timeout_secs: Option<u64>,
        full_arguments: Option<serde_json::Value>,
    ) -> AppResult<FirewallApprovalResponse> {
        self.request_approval_internal(
            client_id,
            client_name,
            model_name,    // model as "tool_name"
            provider_name, // provider as "server_name"
            String::new(), // no arguments for model requests
            timeout_secs,
            true,  // model request
            false, // not guardrail
            full_arguments,
            None,
        )
        .await
    }

    /// Request user approval for a guardrail detection
    ///
    /// Shows guardrail-specific popup with matched rules, severity badges, etc.
    /// Waits indefinitely for user response (no timeout).
    pub async fn request_guardrail_approval(
        &self,
        client_id: String,
        client_name: String,
        model_name: String,
        provider_name: String,
        guardrail_details: GuardrailApprovalDetails,
        arguments_preview: String,
    ) -> AppResult<FirewallApprovalResponse> {
        self.request_approval_internal(
            client_id,
            client_name,
            model_name,
            provider_name,
            arguments_preview,
            None,  // no custom timeout — will use 24h safety
            false, // not a model request
            true,  // guardrail request
            None,
            Some(guardrail_details),
        )
        .await
    }

    /// Internal approval request handler
    async fn request_approval_internal(
        &self,
        client_id: String,
        client_name: String,
        tool_name: String,
        server_name: String,
        arguments_preview: String,
        timeout_secs: Option<u64>,
        is_model_request: bool,
        is_guardrail_request: bool,
        full_arguments: Option<serde_json::Value>,
        guardrail_details: Option<GuardrailApprovalDetails>,
    ) -> AppResult<FirewallApprovalResponse> {
        let request_id = Uuid::new_v4().to_string();
        // Use 24h safety timeout for all requests (effectively indefinite)
        // The old 120s auto-deny was a bug — users should not have requests silently denied
        let timeout = timeout_secs.unwrap_or(86400);

        debug!(
            "Creating firewall approval request {} for client {} {} {} (timeout: {}s, model: {})",
            request_id,
            client_id,
            if is_model_request { "model" } else { "tool" },
            tool_name,
            timeout,
            is_model_request
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
            full_arguments,
            response_sender: Some(tx),
            created_at: Instant::now(),
            timeout_seconds: timeout,
            is_model_request,
            is_guardrail_request,
            guardrail_details,
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
                    "is_model_request": is_model_request,
                    "is_guardrail_request": is_guardrail_request,
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
                    edited_arguments: None,
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
                    edited_arguments: None,
                })
            }
        }
    }

    /// Submit a user response to a pending firewall approval request
    pub fn submit_response(
        &self,
        request_id: &str,
        action: FirewallApprovalAction,
        edited_arguments: Option<serde_json::Value>,
    ) -> AppResult<()> {
        match self.pending.remove(request_id) {
            Some((_, mut session)) => {
                debug!(
                    "Submitting firewall approval response for request {}: {:?}",
                    request_id, action
                );

                if let Some(sender) = session.response_sender.take() {
                    let response = FirewallApprovalResponse {
                        action,
                        edited_arguments,
                    };
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
                let full_arguments = session
                    .full_arguments
                    .as_ref()
                    .and_then(|v| serde_json::to_string(v).ok());
                PendingApprovalInfo {
                    request_id: session.request_id.clone(),
                    client_id: session.client_id.clone(),
                    client_name: session.client_name.clone(),
                    tool_name: session.tool_name.clone(),
                    server_name: session.server_name.clone(),
                    arguments_preview: session.arguments_preview.clone(),
                    full_arguments,
                    created_at_secs_ago: session.created_at.elapsed().as_secs(),
                    timeout_seconds: session.timeout_seconds,
                    is_model_request: session.is_model_request,
                    is_guardrail_request: session.is_guardrail_request,
                    guardrail_details: session.guardrail_details.clone(),
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
            default_timeout_secs: 86400, // 24 hours — effectively indefinite
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
            full_arguments: None,
            response_sender: None,
            created_at: Instant::now() - Duration::from_secs(150),
            timeout_seconds: 120,
            is_model_request: false,
            is_guardrail_request: false,
            guardrail_details: None,
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
            full_arguments: None,
            response_sender: None,
            created_at: Instant::now(),
            timeout_seconds: 120,
            is_model_request: false,
            is_guardrail_request: false,
            guardrail_details: None,
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
            .submit_response(request_id, FirewallApprovalAction::AllowOnce, None)
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

    #[test]
    fn test_action_serde_roundtrip() {
        // Verify all action variants serialize/deserialize correctly
        let actions = vec![
            (FirewallApprovalAction::Deny, "\"deny\""),
            (FirewallApprovalAction::DenySession, "\"deny_session\""),
            (FirewallApprovalAction::DenyAlways, "\"deny_always\""),
            (FirewallApprovalAction::AllowOnce, "\"allow_once\""),
            (FirewallApprovalAction::AllowSession, "\"allow_session\""),
            (FirewallApprovalAction::Allow1Hour, "\"allow_1_hour\""),
            (FirewallApprovalAction::AllowPermanent, "\"allow_permanent\""),
            (
                FirewallApprovalAction::BlockCategories,
                "\"block_categories\"",
            ),
            (
                FirewallApprovalAction::AllowCategories,
                "\"allow_categories\"",
            ),
            (FirewallApprovalAction::Deny1Hour, "\"deny_1_hour\""),
        ];

        for (action, expected_json) in &actions {
            let serialized = serde_json::to_string(action).unwrap();
            assert_eq!(&serialized, expected_json, "Serialization mismatch for {:?}", action);

            let deserialized: FirewallApprovalAction =
                serde_json::from_str(expected_json).unwrap();
            assert_eq!(
                &deserialized, action,
                "Deserialization mismatch for {}",
                expected_json
            );
        }
    }

    #[tokio::test]
    async fn test_submit_allow_categories() {
        let manager = FirewallManager::new(120);

        let manager_clone = manager.clone();
        let handle = tokio::spawn(async move {
            manager_clone
                .request_guardrail_approval(
                    "client-1".to_string(),
                    "Test Client".to_string(),
                    "model-1".to_string(),
                    "guardrails".to_string(),
                    GuardrailApprovalDetails {
                        verdicts: vec![],
                        actions_required: vec![],
                        total_duration_ms: 100,
                        scan_direction: "request".to_string(),
                        flagged_text: "test text".to_string(),
                    },
                    "test preview".to_string(),
                )
                .await
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(manager.pending_count(), 1);

        let pending = manager.list_pending();
        let request_id = &pending[0].request_id;
        assert!(pending[0].is_guardrail_request);

        manager
            .submit_response(request_id, FirewallApprovalAction::AllowCategories, None)
            .unwrap();

        let result = handle.await.unwrap().unwrap();
        assert_eq!(result.action, FirewallApprovalAction::AllowCategories);
    }

    #[tokio::test]
    async fn test_submit_deny_1_hour() {
        let manager = FirewallManager::new(120);

        let manager_clone = manager.clone();
        let handle = tokio::spawn(async move {
            manager_clone
                .request_guardrail_approval(
                    "client-1".to_string(),
                    "Test Client".to_string(),
                    "model-1".to_string(),
                    "guardrails".to_string(),
                    GuardrailApprovalDetails {
                        verdicts: vec![],
                        actions_required: vec![],
                        total_duration_ms: 100,
                        scan_direction: "request".to_string(),
                        flagged_text: "test text".to_string(),
                    },
                    "test preview".to_string(),
                )
                .await
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(manager.pending_count(), 1);

        let pending = manager.list_pending();
        let request_id = &pending[0].request_id;

        manager
            .submit_response(request_id, FirewallApprovalAction::Deny1Hour, None)
            .unwrap();

        let result = handle.await.unwrap().unwrap();
        assert_eq!(result.action, FirewallApprovalAction::Deny1Hour);
    }

    #[tokio::test]
    async fn test_submit_block_categories() {
        let manager = FirewallManager::new(120);

        let manager_clone = manager.clone();
        let handle = tokio::spawn(async move {
            manager_clone
                .request_guardrail_approval(
                    "client-1".to_string(),
                    "Test Client".to_string(),
                    "model-1".to_string(),
                    "guardrails".to_string(),
                    GuardrailApprovalDetails {
                        verdicts: vec![],
                        actions_required: vec![serde_json::json!({
                            "category": "prompt_injection",
                            "action": "ask",
                            "model_id": "llama-guard-3-8b",
                            "confidence": 0.95,
                        })],
                        total_duration_ms: 200,
                        scan_direction: "request".to_string(),
                        flagged_text: "Ignore all instructions".to_string(),
                    },
                    "test preview".to_string(),
                )
                .await
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let pending = manager.list_pending();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].is_guardrail_request);
        assert!(pending[0].guardrail_details.is_some());

        let details = pending[0].guardrail_details.as_ref().unwrap();
        assert_eq!(details.actions_required.len(), 1);

        let request_id = &pending[0].request_id;
        manager
            .submit_response(request_id, FirewallApprovalAction::BlockCategories, None)
            .unwrap();

        let result = handle.await.unwrap().unwrap();
        assert_eq!(result.action, FirewallApprovalAction::BlockCategories);
    }

    #[tokio::test]
    async fn test_guardrail_approval_preserves_details() {
        let manager = FirewallManager::new(120);

        let details = GuardrailApprovalDetails {
            verdicts: vec![serde_json::json!({
                "model_id": "llama-guard-3-8b",
                "is_safe": false,
                "flagged_categories": [{"category": "violence", "confidence": 0.9, "native_label": "S1"}],
            })],
            actions_required: vec![
                serde_json::json!({"category": "violence", "action": "ask", "model_id": "llama-guard-3-8b", "confidence": 0.9}),
                serde_json::json!({"category": "jailbreak", "action": "ask", "model_id": "llama-guard-3-8b", "confidence": 0.85}),
            ],
            total_duration_ms: 340,
            scan_direction: "request".to_string(),
            flagged_text: "some harmful content".to_string(),
        };

        let manager_clone = manager.clone();
        let _handle = tokio::spawn(async move {
            manager_clone
                .request_guardrail_approval(
                    "client-1".to_string(),
                    "Test Client".to_string(),
                    "gpt-4".to_string(),
                    "guardrails".to_string(),
                    details,
                    "preview".to_string(),
                )
                .await
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let pending = manager.list_pending();
        assert_eq!(pending.len(), 1);

        let info = &pending[0];
        assert!(info.is_guardrail_request);
        assert!(!info.is_model_request);
        assert_eq!(info.client_id, "client-1");
        assert_eq!(info.tool_name, "gpt-4");

        let gd = info.guardrail_details.as_ref().unwrap();
        assert_eq!(gd.verdicts.len(), 1);
        assert_eq!(gd.actions_required.len(), 2);
        assert_eq!(gd.total_duration_ms, 340);
        assert_eq!(gd.scan_direction, "request");
        assert_eq!(gd.flagged_text, "some harmful content");

        // Clean up
        manager.cancel_request(&info.request_id).unwrap();
    }
}
