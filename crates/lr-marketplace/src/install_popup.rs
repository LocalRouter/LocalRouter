//! Marketplace install popup manager
//!
//! Follows the same oneshot-channel pattern as FirewallManager.
//! Manages pending install requests that await user approval via popup.

use crate::types::{MarketplaceError, McpServerListing, SkillListing};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// JSON-RPC notification for SSE broadcast
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// Install action chosen by user
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallAction {
    /// Proceed with installation using provided config
    Install,
    /// Cancel the installation
    Cancel,
}

/// Response from user for an install request
#[derive(Debug)]
pub struct InstallResponse {
    pub action: InstallAction,
    /// User-provided config (for Install action)
    pub config: Option<Value>,
}

/// Type of install request
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallType {
    McpServer,
    Skill,
}

/// Pending install session
pub struct PendingInstall {
    /// Unique request ID
    pub request_id: String,

    /// Type of install
    pub install_type: InstallType,

    /// Full listing data for the popup to display
    pub listing: Value,

    /// Client ID that requested the install
    pub client_id: String,

    /// Human-readable client name
    pub client_name: String,

    /// Channel to send response back to waiting request
    pub response_sender: Option<oneshot::Sender<InstallResponse>>,

    /// When this request was created
    pub created_at: Instant,

    /// Timeout in seconds
    pub timeout_seconds: u64,
}

impl PendingInstall {
    /// Check if this session has expired
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > Duration::from_secs(self.timeout_seconds)
    }
}

/// Info about a pending install (for UI display, without the oneshot channel)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingInstallInfo {
    pub request_id: String,
    pub install_type: InstallType,
    pub listing: Value,
    pub client_id: String,
    pub client_name: String,
    pub created_at_secs_ago: u64,
    pub timeout_seconds: u64,
}

/// Manages marketplace install popup lifecycle
pub struct MarketplaceInstallManager {
    /// Pending install sessions (request_id -> session)
    pending: Arc<DashMap<String, PendingInstall>>,

    /// Default timeout for install requests (seconds)
    default_timeout_secs: u64,

    /// Broadcast sender for SSE notifications (optional)
    notification_broadcast:
        Option<Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>>,
}

impl MarketplaceInstallManager {
    /// Create a new install manager
    pub fn new(default_timeout_secs: u64) -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            default_timeout_secs,
            notification_broadcast: None,
        }
    }

    /// Create a new install manager with SSE broadcast support
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

    /// Request user approval for MCP server installation
    pub async fn request_mcp_install(
        &self,
        listing: McpServerListing,
        client_id: String,
        client_name: String,
    ) -> Result<InstallResponse, MarketplaceError> {
        let listing_json = serde_json::to_value(&listing)?;
        self.request_install(InstallType::McpServer, listing_json, client_id, client_name)
            .await
    }

    /// Request user approval for skill installation
    pub async fn request_skill_install(
        &self,
        listing: SkillListing,
        client_id: String,
        client_name: String,
    ) -> Result<InstallResponse, MarketplaceError> {
        let listing_json = serde_json::to_value(&listing)?;
        self.request_install(InstallType::Skill, listing_json, client_id, client_name)
            .await
    }

    /// Request user approval for an installation
    async fn request_install(
        &self,
        install_type: InstallType,
        listing: Value,
        client_id: String,
        client_name: String,
    ) -> Result<InstallResponse, MarketplaceError> {
        let request_id = Uuid::new_v4().to_string();
        let timeout = self.default_timeout_secs;

        debug!(
            "Creating marketplace install request {} for client {} (type: {:?}, timeout: {}s)",
            request_id, client_id, install_type, timeout
        );

        // Create response channel
        let (tx, rx) = oneshot::channel();

        // Create session
        let session = PendingInstall {
            request_id: request_id.clone(),
            install_type: install_type.clone(),
            listing: listing.clone(),
            client_id: client_id.clone(),
            client_name: client_name.clone(),
            response_sender: Some(tx),
            created_at: Instant::now(),
            timeout_seconds: timeout,
        };

        // Store session
        self.pending.insert(request_id.clone(), session);

        info!(
            "Marketplace install request {} created: client={}, type={:?}",
            request_id, client_id, install_type
        );

        // Send SSE notification to connected clients
        if let Some(broadcast) = &self.notification_broadcast {
            let notification = JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: "notifications/marketplace/install_request".to_string(),
                params: Some(json!({
                    "request_id": request_id,
                    "install_type": install_type,
                    "listing": listing,
                    "client_id": client_id,
                    "client_name": client_name,
                    "timeout_seconds": timeout,
                })),
            };

            if let Err(e) = broadcast.send(("_marketplace".to_string(), notification)) {
                debug!("Failed to broadcast marketplace install request: {}", e);
            }
        }

        // Wait for response with timeout
        match tokio::time::timeout(Duration::from_secs(timeout), rx).await {
            Ok(Ok(response)) => {
                info!(
                    "Received marketplace install response for request {}: {:?}",
                    request_id, response.action
                );
                self.pending.remove(&request_id);
                Ok(response)
            }
            Ok(Err(_)) => {
                // Channel closed (cancelled)
                warn!("Marketplace install request {} was cancelled", request_id);
                self.pending.remove(&request_id);
                Err(MarketplaceError::InstallCancelled)
            }
            Err(_) => {
                // Timeout
                warn!(
                    "Marketplace install request {} timed out after {}s",
                    request_id, timeout
                );
                self.pending.remove(&request_id);
                Err(MarketplaceError::InstallTimeout)
            }
        }
    }

    /// Submit a user response to a pending install request
    pub fn submit_response(
        &self,
        request_id: &str,
        action: InstallAction,
        config: Option<Value>,
    ) -> Result<(), MarketplaceError> {
        match self.pending.remove(request_id) {
            Some((_, mut session)) => {
                debug!(
                    "Submitting marketplace install response for request {}: {:?}",
                    request_id, action
                );

                if let Some(sender) = session.response_sender.take() {
                    let response = InstallResponse { action, config };
                    sender.send(response).map_err(|_| {
                        MarketplaceError::Internal("Failed to send install response".to_string())
                    })?;
                }

                info!(
                    "Marketplace install response submitted for request {}",
                    request_id
                );

                Ok(())
            }
            None => {
                warn!(
                    "Attempted to submit response for unknown install request {}",
                    request_id
                );
                Err(MarketplaceError::Internal(format!(
                    "Install request {} not found or expired",
                    request_id
                )))
            }
        }
    }

    /// Cancel a pending install request
    pub fn cancel_request(&self, request_id: &str) -> Result<(), MarketplaceError> {
        match self.pending.remove(request_id) {
            Some(_) => {
                info!("Cancelled marketplace install request {}", request_id);
                Ok(())
            }
            None => Err(MarketplaceError::Internal(format!(
                "Install request {} not found",
                request_id
            ))),
        }
    }

    /// List all pending install requests (for UI display)
    pub fn list_pending(&self) -> Vec<PendingInstallInfo> {
        self.pending
            .iter()
            .map(|entry| {
                let session = entry.value();
                PendingInstallInfo {
                    request_id: session.request_id.clone(),
                    install_type: session.install_type.clone(),
                    listing: session.listing.clone(),
                    client_id: session.client_id.clone(),
                    client_name: session.client_name.clone(),
                    created_at_secs_ago: session.created_at.elapsed().as_secs(),
                    timeout_seconds: session.timeout_seconds,
                }
            })
            .collect()
    }

    /// Get details of a specific pending install
    pub fn get_pending(&self, request_id: &str) -> Option<PendingInstallInfo> {
        self.pending.get(request_id).map(|entry| {
            let session = entry.value();
            PendingInstallInfo {
                request_id: session.request_id.clone(),
                install_type: session.install_type.clone(),
                listing: session.listing.clone(),
                client_id: session.client_id.clone(),
                client_name: session.client_name.clone(),
                created_at_secs_ago: session.created_at.elapsed().as_secs(),
                timeout_seconds: session.timeout_seconds,
            }
        })
    }

    /// Get the number of pending requests
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Check if there are any pending requests
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Clean up expired sessions
    pub fn cleanup_expired(&self) {
        let expired: Vec<String> = self
            .pending
            .iter()
            .filter(|entry| entry.value().is_expired())
            .map(|entry| entry.key().clone())
            .collect();

        for request_id in expired {
            warn!(
                "Cleaning up expired marketplace install request {}",
                request_id
            );
            self.pending.remove(&request_id);
        }
    }
}

impl Default for MarketplaceInstallManager {
    fn default() -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            default_timeout_secs: 120,
            notification_broadcast: None,
        }
    }
}

impl Clone for MarketplaceInstallManager {
    fn clone(&self) -> Self {
        Self {
            pending: self.pending.clone(),
            default_timeout_secs: self.default_timeout_secs,
            notification_broadcast: self.notification_broadcast.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_install_manager_creation() {
        let manager = MarketplaceInstallManager::new(60);
        assert_eq!(manager.pending_count(), 0);
        assert_eq!(manager.default_timeout_secs, 60);
    }

    #[test]
    fn test_session_expiry() {
        let session = PendingInstall {
            request_id: "test-123".to_string(),
            install_type: InstallType::McpServer,
            listing: json!({}),
            client_id: "client-1".to_string(),
            client_name: "Test Client".to_string(),
            response_sender: None,
            created_at: Instant::now() - Duration::from_secs(150),
            timeout_seconds: 120,
        };
        assert!(session.is_expired());
    }

    #[test]
    fn test_session_not_expired() {
        let session = PendingInstall {
            request_id: "test-123".to_string(),
            install_type: InstallType::McpServer,
            listing: json!({}),
            client_id: "client-1".to_string(),
            client_name: "Test Client".to_string(),
            response_sender: None,
            created_at: Instant::now(),
            timeout_seconds: 120,
        };
        assert!(!session.is_expired());
    }

    #[tokio::test]
    async fn test_submit_response() {
        let manager = MarketplaceInstallManager::new(120);
        let manager_clone = manager.clone();

        // Start a request in the background
        let handle = tokio::spawn(async move {
            manager_clone
                .request_install(
                    InstallType::McpServer,
                    json!({"name": "test"}),
                    "client-1".to_string(),
                    "Test Client".to_string(),
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
            .submit_response(
                request_id,
                InstallAction::Install,
                Some(json!({"name": "configured"})),
            )
            .unwrap();

        // Should complete successfully
        let result = handle.await.unwrap();
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.action, InstallAction::Install);
        assert!(response.config.is_some());
    }

    #[tokio::test]
    async fn test_timeout() {
        let manager = MarketplaceInstallManager::new(1); // 1 second timeout

        let result = manager
            .request_install(
                InstallType::Skill,
                json!({"name": "test"}),
                "client-1".to_string(),
                "Test Client".to_string(),
            )
            .await;

        assert!(result.is_err());
        match result {
            Err(MarketplaceError::InstallTimeout) => {}
            _ => panic!("Expected InstallTimeout error"),
        }
    }
}
