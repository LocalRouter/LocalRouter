use crate::config::StreamingConfig;
use crate::utils::errors::{AppError, AppResult};
use crate::mcp::gateway::session::GatewaySession;
use crate::mcp::gateway::McpGateway;
use crate::mcp::manager::HealthStatus;
use crate::mcp::McpServerManager;
use crate::mcp::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use axum::response::sse::Event;
use dashmap::DashMap;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Streaming event types that flow through the merge channel
#[derive(Debug, Clone)]
pub enum StreamingEvent {
    /// Complete response from a backend server
    Response {
        request_id: String,
        server_id: String,
        response: JsonRpcResponse,
    },

    /// Notification from a backend server
    Notification {
        server_id: String,
        notification: JsonRpcNotification,
    },

    /// Streaming chunk from a backend server
    StreamChunk {
        request_id: String,
        server_id: String,
        chunk: StreamingChunk,
    },

    /// Error event
    Error {
        request_id: Option<String>,
        server_id: Option<String>,
        error: String,
    },

    /// Heartbeat keepalive
    Heartbeat,
}

/// Streaming chunk data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingChunk {
    pub is_final: bool,
    pub data: Value,
}

/// Pending request tracking
#[derive(Debug, Clone)]
pub struct PendingRequest {
    pub request_id: String,
    pub client_request_id: Value,
    pub method: String,
    pub target_servers: Vec<String>,
    pub created_at: Instant,
}

/// Routing mode for requests
#[derive(Debug, Clone)]
pub enum RoutingMode {
    /// Route to a single server
    Direct(String),
    /// Broadcast to all allowed servers
    Broadcast,
}

/// Routing decision
#[derive(Debug, Clone)]
pub struct Routing {
    pub mode: RoutingMode,
    pub servers: Vec<String>,
}

/// A streaming session manages SSE connections to multiple backend MCP servers
/// and multiplexes their events into a single client-facing SSE stream
pub struct StreamingSession {
    /// Unique session ID
    session_id: String,

    /// Client ID (from auth context)
    client_id: String,

    /// Servers this client can access
    allowed_servers: Vec<String>,

    /// Event merge channel (all events flow through here)
    event_tx: mpsc::UnboundedSender<StreamingEvent>,
    event_rx: Arc<Mutex<mpsc::UnboundedReceiver<StreamingEvent>>>,

    /// Track pending requests
    pending_requests: DashMap<String, PendingRequest>,

    /// Shared gateway session (for caching, deferred loading)
    gateway_session: Arc<RwLock<GatewaySession>>,

    /// Server manager reference
    server_manager: Arc<McpServerManager>,

    /// Configuration
    config: StreamingConfig,

    /// Created timestamp
    created_at: Instant,

    /// Last activity timestamp
    last_activity: Arc<RwLock<Instant>>,
}

impl StreamingSession {
    /// Create a new streaming session
    pub fn new(
        session_id: String,
        client_id: String,
        allowed_servers: Vec<String>,
        gateway_session: Arc<RwLock<GatewaySession>>,
        server_manager: Arc<McpServerManager>,
        config: StreamingConfig,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let now = Instant::now();

        Self {
            session_id,
            client_id,
            allowed_servers,
            event_tx,
            event_rx: Arc::new(Mutex::new(event_rx)),
            pending_requests: DashMap::new(),
            gateway_session,
            server_manager,
            config,
            created_at: now,
            last_activity: Arc::new(RwLock::new(now)),
        }
    }

    /// Get session ID
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Get client ID
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// Get allowed servers
    pub fn allowed_servers(&self) -> &[String] {
        &self.allowed_servers
    }

    /// Get session age
    pub fn age(&self) -> Duration {
        self.created_at.elapsed()
    }

    /// Get time since last activity
    pub async fn idle_time(&self) -> Duration {
        self.last_activity.read().await.elapsed()
    }

    /// Check if session is expired
    pub async fn is_expired(&self) -> bool {
        self.idle_time().await > Duration::from_secs(self.config.session_timeout_secs)
    }

    /// Initialize backend servers via SSE - connect and forward events
    pub async fn initialize_backend_servers(
        &self,
        _params: Value,
    ) -> AppResult<Vec<String>> {
        let mut initialized = Vec::new();

        info!(
            "Initializing {} backend servers for streaming session {}",
            self.allowed_servers.len(),
            self.session_id
        );

        for server_id in &self.allowed_servers {
            let health = self.server_manager.get_server_health(server_id).await;
            if health.status == HealthStatus::Healthy {
                initialized.push(server_id.clone());
                info!("Verified backend {} for session {}", server_id, self.session_id);

                // Start forwarding backend notifications to the merge channel
                self.start_backend_notification_forwarding(server_id.clone());
            } else {
                warn!(
                    "Backend {} not available for session {}: {:?}",
                    server_id, self.session_id, health.status
                );
                // Send error event
                let _ = self.event_tx.send(StreamingEvent::Error {
                    request_id: None,
                    server_id: Some(server_id.clone()),
                    error: format!("Server not healthy: {:?}", health.status),
                });
            }
        }

        Ok(initialized)
    }

    /// Start forwarding backend server notifications to the merge channel
    /// Subscribes to the MCP notification broadcast and filters by server_id
    fn start_backend_notification_forwarding(&self, server_id: String) {
        let event_tx = self.event_tx.clone();
        let session_id = self.session_id.clone();

        debug!(
            "Setting up notification forwarding for backend {} in session {}",
            server_id, session_id
        );

        // Register a notification handler for this server
        // Note: Currently handlers cannot be removed when session closes.
        // This is a known limitation - consider adding handler removal to McpServerManager.
        let handler_session_id = session_id.clone();
        let handler_server_id = server_id.clone();
        self.server_manager.on_notification(
            &server_id,
            Arc::new(move |srv_id: String, notification| {
                debug!(
                    "Forwarding notification from {} to streaming session {}",
                    srv_id, handler_session_id
                );
                // Forward notification through the streaming session's event channel
                let _ = event_tx.send(StreamingEvent::Notification {
                    server_id: handler_server_id.clone(),
                    notification,
                });
            }),
        );
    }

    /// Handle incoming request from client
    pub async fn handle_request(&self, mut request: JsonRpcRequest) -> AppResult<String> {
        // Generate internal request ID
        let request_id = Uuid::new_v4().to_string();

        debug!(
            "Handling request {} (client ID: {:?}) in session {}",
            request.method,
            request.id,
            self.session_id
        );

        // Parse routing
        let routing = self.parse_routing(&request.method)?;

        // Store pending request
        self.pending_requests.insert(
            request_id.clone(),
            PendingRequest {
                request_id: request_id.clone(),
                client_request_id: request.id.clone().unwrap_or(json!(null)),
                method: request.method.clone(),
                target_servers: routing.servers.clone(),
                created_at: Instant::now(),
            },
        );

        // Update internal request ID
        let _original_client_id = request.id.clone();
        request.id = Some(json!(request_id.clone()));

        // Send to target servers
        match routing.mode {
            RoutingMode::Direct(server_id) => {
                // Send to single server via server manager
                match self.server_manager.send_request(&server_id, request).await {
                    Ok(response) => {
                        // Forward response through event channel
                        let _ = self.event_tx.send(StreamingEvent::Response {
                            request_id: request_id.clone(),
                            server_id: server_id.clone(),
                            response,
                        });
                    }
                    Err(e) => {
                        self.pending_requests.remove(&request_id);
                        return Err(AppError::Mcp(format!("Failed to send request to {}: {}", server_id, e)));
                    }
                }
            }
            RoutingMode::Broadcast => {
                // Send to all allowed servers
                let mut sent_count = 0;
                for server_id in &self.allowed_servers {
                    let mut req_clone = request.clone();
                    req_clone.id = Some(json!(format!("{}_{}", request_id, server_id)));
                    match self.server_manager.send_request(server_id, req_clone).await {
                        Ok(response) => {
                            sent_count += 1;
                            // Forward response through event channel
                            let _ = self.event_tx.send(StreamingEvent::Response {
                                request_id: request_id.clone(),
                                server_id: server_id.clone(),
                                response,
                            });
                        }
                        Err(e) => {
                            warn!("Failed to send broadcast request to {}: {}", server_id, e);
                        }
                    }
                }
                if sent_count == 0 {
                    self.pending_requests.remove(&request_id);
                    return Err(AppError::Mcp("No servers available for broadcast".to_string()));
                }
            }
        }

        // Update last activity
        *self.last_activity.write().await = Instant::now();

        Ok(request_id)
    }

    /// Parse routing from method name
    fn parse_routing(&self, method: &str) -> AppResult<Routing> {
        // Check for namespace (e.g., "filesystem__tools/call")
        if let Some((server_id, _)) = method.split_once("__") {
            if !self.allowed_servers.contains(&server_id.to_string()) {
                return Err(AppError::Unauthorized);
            }
            return Ok(Routing {
                mode: RoutingMode::Direct(server_id.to_string()),
                servers: vec![server_id.to_string()],
            });
        }

        // Check for broadcast methods
        if matches!(
            method,
            "initialize"
                | "tools/list"
                | "resources/list"
                | "prompts/list"
                | "logging/setLevel"
                | "ping"
        ) {
            return Ok(Routing {
                mode: RoutingMode::Broadcast,
                servers: self.allowed_servers.clone(),
            });
        }

        // Ambiguous - require namespace
        Err(AppError::InvalidParams(
            "Method requires server namespace (e.g., 'filesystem__tools/call')".to_string(),
        ))
    }

    /// Start event stream forwarding
    pub async fn start_event_stream(
        &self,
    ) -> Pin<Box<dyn Stream<Item = Result<Event, AppError>> + Send>> {
        let event_rx = self.event_rx.clone();
        let last_activity = self.last_activity.clone();
        let heartbeat_interval = self.config.heartbeat_interval_secs;

        let stream = async_stream::stream! {
            let mut event_rx = event_rx.lock().await;
            let mut heartbeat_timer = tokio::time::interval(Duration::from_secs(heartbeat_interval));

            loop {
                tokio::select! {
                    Some(event) = event_rx.recv() => {
                        // Update last activity
                        *last_activity.write().await = Instant::now();

                        // Convert to SSE event
                        let sse_event = match event {
                            StreamingEvent::Response { request_id, server_id, response } => {
                                let data = json!({
                                    "request_id": request_id,
                                    "server_id": server_id,
                                    "response": response,
                                });
                                Event::default()
                                    .event("response")
                                    .data(serde_json::to_string(&data).unwrap())
                            }
                            StreamingEvent::Notification { server_id, notification } => {
                                let data = json!({
                                    "server_id": server_id,
                                    "notification": notification,
                                });
                                Event::default()
                                    .event("notification")
                                    .data(serde_json::to_string(&data).unwrap())
                            }
                            StreamingEvent::StreamChunk { request_id, server_id, chunk } => {
                                let data = json!({
                                    "request_id": request_id,
                                    "server_id": server_id,
                                    "chunk": chunk,
                                });
                                Event::default()
                                    .event("chunk")
                                    .data(serde_json::to_string(&data).unwrap())
                            }
                            StreamingEvent::Error { request_id, server_id, error } => {
                                let data = json!({
                                    "request_id": request_id,
                                    "server_id": server_id,
                                    "error": error,
                                });
                                Event::default()
                                    .event("error")
                                    .data(serde_json::to_string(&data).unwrap())
                            }
                            StreamingEvent::Heartbeat => {
                                Event::default()
                                    .event("heartbeat")
                                    .data("")
                            }
                        };

                        yield Ok(sse_event);
                    }
                    _ = heartbeat_timer.tick() => {
                        yield Ok(Event::default().event("heartbeat").data(""));
                    }
                }
            }
        };

        Box::pin(stream)
    }

    /// Cleanup expired requests
    pub fn cleanup_expired_requests(&self) {
        let timeout = Duration::from_secs(self.config.request_timeout_secs);
        let now = Instant::now();

        self.pending_requests.retain(|_, req| {
            if now.duration_since(req.created_at) > timeout {
                warn!("Request {} timed out in session {}", req.request_id, self.session_id);
                false
            } else {
                true
            }
        });
    }
}

/// Manages all active streaming sessions
pub struct StreamingSessionManager {
    /// All active streaming sessions
    sessions: DashMap<String, Arc<StreamingSession>>,

    /// Gateway reference
    gateway: Arc<McpGateway>,

    /// Server manager
    server_manager: Arc<McpServerManager>,

    /// Config
    config: StreamingConfig,
}

impl StreamingSessionManager {
    /// Create a new streaming session manager
    pub fn new(
        gateway: Arc<McpGateway>,
        server_manager: Arc<McpServerManager>,
        config: StreamingConfig,
    ) -> Self {
        Self {
            sessions: DashMap::new(),
            gateway,
            server_manager,
            config,
        }
    }

    /// Create new streaming session
    pub async fn create_session(
        &self,
        client_id: String,
        allowed_servers: Vec<String>,
        gateway_session: Arc<RwLock<GatewaySession>>,
        initialize_params: Value,
    ) -> AppResult<Arc<StreamingSession>> {
        // Check session limit per client
        let current_sessions = self
            .sessions
            .iter()
            .filter(|entry| entry.value().client_id() == client_id)
            .count();

        if current_sessions >= self.config.max_sessions_per_client {
            return Err(AppError::RateLimitExceeded);
        }

        // Generate session ID
        let session_id = Uuid::new_v4().to_string();

        info!(
            "Creating streaming session {} for client {} with {} servers",
            session_id,
            client_id,
            allowed_servers.len()
        );

        // Create session
        let session = Arc::new(StreamingSession::new(
            session_id.clone(),
            client_id,
            allowed_servers,
            gateway_session,
            self.server_manager.clone(),
            self.config.clone(),
        ));

        // Initialize backend servers
        let initialized = session.initialize_backend_servers(initialize_params).await?;

        if initialized.is_empty() {
            return Err(AppError::Mcp("Failed to initialize any backend servers".to_string()));
        }

        info!(
            "Successfully initialized {}/{} backends for session {}",
            initialized.len(),
            session.allowed_servers().len(),
            session_id
        );

        // Store session
        self.sessions.insert(session_id, session.clone());

        Ok(session)
    }

    /// Get existing session
    pub fn get_session(&self, session_id: &str) -> Option<Arc<StreamingSession>> {
        self.sessions.get(session_id).map(|entry| entry.value().clone())
    }

    /// Close session
    pub async fn close_session(&self, session_id: &str) {
        if let Some((_, session)) = self.sessions.remove(session_id) {
            let age = session.age();
            info!(
                "Closed streaming session {} (client: {}, age: {:?})",
                session_id,
                session.client_id(),
                age
            );
        }
    }

    /// Cleanup expired sessions (called periodically)
    pub async fn cleanup_expired_sessions(&self) {
        let expired: Vec<String> = {
            let mut expired = Vec::new();
            for entry in self.sessions.iter() {
                if entry.value().is_expired().await {
                    expired.push(entry.key().clone());
                }
            }
            expired
        };

        for session_id in expired {
            if let Some((_, session)) = self.sessions.remove(&session_id) {
                let idle = session.idle_time().await;
                warn!(
                    "Cleaned up expired streaming session {} (client: {}, idle: {:?})",
                    session_id,
                    session.client_id(),
                    idle
                );
            }
        }
    }

    /// Get active session count
    pub fn active_session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Get sessions for a specific client
    pub fn get_client_sessions(&self, client_id: &str) -> Vec<Arc<StreamingSession>> {
        self.sessions
            .iter()
            .filter(|entry| entry.value().client_id() == client_id)
            .map(|entry| entry.value().clone())
            .collect()
    }
}
