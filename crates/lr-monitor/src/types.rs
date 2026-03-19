use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A monitor event captured from the request pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorEvent {
    /// Unique event ID (e.g., "mon-<uuid>")
    pub id: String,

    /// Monotonic sequence number for stable ordering
    pub sequence: u64,

    /// When the event was created
    pub timestamp: DateTime<Utc>,

    /// Event type discriminator
    pub event_type: MonitorEventType,

    /// Client ID that triggered the event (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,

    /// Human-readable client name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,

    /// Session ID grouping all events from one API request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    /// Type-specific event data
    pub data: MonitorEventData,

    /// Current status (pending for in-flight, complete, or error)
    pub status: EventStatus,

    /// Duration in milliseconds (filled on completion)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Lightweight summary for list views (avoids sending full data).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorEventSummary {
    pub id: String,
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub event_type: MonitorEventType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    pub status: EventStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// One-line summary for display in the list
    pub summary: String,
    /// Session ID grouping all events from one API request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Pending,
    Complete,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorEventType {
    // LLM (combined: request + transform + response/error)
    LlmCall,

    // MCP (combined: request + response)
    McpToolCall,
    McpResourceRead,
    McpPromptGet,
    McpElicitation,
    McpSampling,

    // Security (combined: request + response)
    GuardrailScan,
    GuardrailResponseScan,
    SecretScan,

    // Routing (combined: request + response)
    RouteLlmClassify,

    // Standalone events
    RoutingDecision,
    AuthError,
    AccessDenied,
    RateLimitEvent,
    ValidationError,
    McpServerEvent,
    OAuthEvent,
    InternalError,
    ModerationEvent,
    ConnectionError,
    PromptCompression,
    FirewallDecision,
    SseConnection,
}

impl MonitorEventType {
    /// Human-readable label for display.
    pub fn label(&self) -> &'static str {
        match self {
            Self::LlmCall => "LLM Call",
            Self::McpToolCall => "MCP Tool Call",
            Self::McpResourceRead => "MCP Resource Read",
            Self::McpPromptGet => "MCP Prompt Get",
            Self::McpElicitation => "MCP Elicitation",
            Self::McpSampling => "MCP Sampling",
            Self::GuardrailScan => "Guardrail Scan",
            Self::GuardrailResponseScan => "Guardrail Response Scan",
            Self::SecretScan => "Secret Scan",
            Self::RouteLlmClassify => "RouteLLM Classify",
            Self::RoutingDecision => "Routing Decision",
            Self::AuthError => "Auth Error",
            Self::AccessDenied => "Access Denied",
            Self::RateLimitEvent => "Rate Limit",
            Self::ValidationError => "Validation Error",
            Self::McpServerEvent => "MCP Server Event",
            Self::OAuthEvent => "OAuth Event",
            Self::InternalError => "Internal Error",
            Self::ModerationEvent => "Moderation Event",
            Self::ConnectionError => "Connection Error",
            Self::PromptCompression => "Prompt Compression",
            Self::FirewallDecision => "Firewall Decision",
            Self::SseConnection => "SSE Connection",
        }
    }

    /// Category for grouping/coloring in UI.
    pub fn category(&self) -> &'static str {
        match self {
            Self::LlmCall => "llm",
            Self::McpToolCall
            | Self::McpResourceRead
            | Self::McpPromptGet
            | Self::McpElicitation
            | Self::McpSampling => "mcp",
            Self::GuardrailScan | Self::GuardrailResponseScan | Self::SecretScan => "security",
            Self::RouteLlmClassify | Self::RoutingDecision => "routing",
            Self::AuthError | Self::AccessDenied | Self::OAuthEvent => "auth",
            Self::RateLimitEvent => "rate_limit",
            Self::ValidationError => "validation",
            Self::McpServerEvent => "mcp_server",
            Self::InternalError => "internal",
            Self::ModerationEvent => "moderation",
            Self::ConnectionError | Self::SseConnection => "connection",
            Self::PromptCompression => "optimization",
            Self::FirewallDecision => "firewall",
        }
    }
}

/// Type-specific event payload. Uses serde tag for frontend dispatch.
///
/// Combined events (request+response merged) use `Option<T>` for response fields
/// that are filled via `update()` when the operation completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MonitorEventData {
    // ---- LLM (combined: request + transform + response/error) ----
    LlmCall {
        // Request fields (populated at creation)
        endpoint: String,
        model: String,
        stream: bool,
        message_count: usize,
        has_tools: bool,
        tool_count: usize,
        /// Full request body (may be truncated for very large requests)
        request_body: serde_json::Value,

        // Transformation fields (filled via update when transformations applied)
        #[serde(skip_serializing_if = "Option::is_none")]
        transformed_body: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        transformations_applied: Option<Vec<String>>,

        // Response fields (filled on completion)
        #[serde(skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status_code: Option<u16>,
        #[serde(skip_serializing_if = "Option::is_none")]
        input_tokens: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_tokens: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        total_tokens: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        latency_ms: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        finish_reason: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        content_preview: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        streamed: Option<bool>,

        // Error field (filled only on error)
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    // ---- MCP (combined: request + response) ----
    McpToolCall {
        // Request
        tool_name: String,
        server_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        server_name: Option<String>,
        arguments: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        firewall_action: Option<String>,

        // Response (filled on completion)
        #[serde(skip_serializing_if = "Option::is_none")]
        latency_ms: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        success: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        response_preview: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    McpResourceRead {
        // Request
        uri: String,
        server_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        server_name: Option<String>,

        // Response (filled on completion)
        #[serde(skip_serializing_if = "Option::is_none")]
        latency_ms: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        success: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        content_preview: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    McpPromptGet {
        // Request
        prompt_name: String,
        server_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        server_name: Option<String>,
        arguments: serde_json::Value,

        // Response (filled on completion)
        #[serde(skip_serializing_if = "Option::is_none")]
        latency_ms: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        success: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        content_preview: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    McpElicitation {
        // Request
        server_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        server_name: Option<String>,
        message: String,
        schema: serde_json::Value,

        // Response (filled on completion)
        /// "submitted", "cancelled", "timeout"
        #[serde(skip_serializing_if = "Option::is_none")]
        action: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        latency_ms: Option<u64>,
    },
    McpSampling {
        // Request
        server_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        server_name: Option<String>,
        message_count: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        model_hint: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_tokens: Option<u64>,

        // Response (filled on completion)
        /// "approved", "rejected"
        #[serde(skip_serializing_if = "Option::is_none")]
        action: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model_used: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        content_preview: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        latency_ms: Option<u64>,
    },

    // ---- Security (combined: request + response) ----
    GuardrailScan {
        /// "request" — input safety scan
        direction: String,
        text_preview: String,
        models_used: Vec<String>,

        // Response (filled on completion)
        /// "pass" or "flagged"
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        flagged_categories: Option<Vec<FlaggedCategory>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        action_taken: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        latency_ms: Option<u64>,
    },
    /// Guardrail check on the LLM response (output safety check).
    GuardrailResponseScan {
        /// "response" — output safety scan
        direction: String,
        text_preview: String,
        models_used: Vec<String>,

        // Response (filled on completion)
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        flagged_categories: Option<Vec<FlaggedCategory>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        action_taken: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        latency_ms: Option<u64>,
    },
    SecretScan {
        // Request
        text_preview: String,
        rules_count: usize,

        // Response (filled on completion)
        #[serde(skip_serializing_if = "Option::is_none")]
        findings_count: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        findings: Option<serde_json::Value>,
        /// "notify", "ask", "block"
        #[serde(skip_serializing_if = "Option::is_none")]
        action_taken: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        latency_ms: Option<u64>,
    },

    // ---- Routing (combined: request + response) ----
    RouteLlmClassify {
        // Request
        original_model: String,
        threshold: f64,

        // Response (filled on completion)
        #[serde(skip_serializing_if = "Option::is_none")]
        win_rate: Option<f64>,
        /// "strong" or "weak"
        #[serde(skip_serializing_if = "Option::is_none")]
        selected_tier: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        routed_model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        latency_ms: Option<u64>,
    },

    // ---- Standalone events (unchanged structure) ----
    RoutingDecision {
        /// "auto_router", "model_firewall", "routellm", "direct"
        routing_type: String,
        original_model: String,
        final_model: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        candidate_models: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        firewall_action: Option<String>,
    },

    // ---- Auth & Access Control ----
    AuthError {
        /// "missing_header", "invalid_format", "invalid_key", "verification_error"
        error_type: String,
        endpoint: String,
        message: String,
        status_code: u16,
    },
    AccessDenied {
        /// "client_not_found", "client_disabled", "mcp_only_client_llm", "llm_only_client_mcp",
        /// "mcp_via_llm_direct_mcp", "model_not_allowed", "model_not_found", "strategy_not_found"
        reason: String,
        endpoint: String,
        message: String,
        status_code: u16,
    },

    // ---- Rate Limiting ----
    RateLimitEvent {
        /// "rate_limit_exceeded", "oauth_token_rate_limit", "free_tier_exhausted", "free_tier_fallback"
        reason: String,
        endpoint: String,
        message: String,
        status_code: u16,
        #[serde(skip_serializing_if = "Option::is_none")]
        retry_after_secs: Option<u64>,
    },

    // ---- Validation ----
    ValidationError {
        endpoint: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        field: Option<String>,
        message: String,
        status_code: u16,
    },

    // ---- MCP Server Health ----
    McpServerEvent {
        server_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        server_name: Option<String>,
        /// "connection_failed", "disconnected", "health_changed", "stop_failed",
        /// "path_resolution_failed", "gateway_error"
        action: String,
        message: String,
    },

    // ---- OAuth ----
    OAuthEvent {
        /// "secret_retrieval_failed", "browser_token_failed", "credential_validation_failed",
        /// "token_generation_failed", "rate_limited"
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        client_id_hint: Option<String>,
        message: String,
        status_code: u16,
    },

    // ---- Internal ----
    InternalError {
        /// "storage", "serialization", "crypto", "config", "io"
        error_type: String,
        message: String,
        status_code: u16,
    },

    // ---- Moderation ----
    ModerationEvent {
        /// "endpoint_disabled", "no_models_configured", "no_models_loaded"
        reason: String,
        message: String,
        status_code: u16,
    },

    // ---- Connection / Transport ----
    ConnectionError {
        /// "websocket", "stdio_bridge", "sse"
        transport: String,
        /// "upgrade_failed", "config_not_found", "client_not_found", "no_mcp_servers",
        /// "server_unavailable"
        action: String,
        message: String,
    },

    // ---- Other ----
    PromptCompression {
        original_tokens: u64,
        compressed_tokens: u64,
        reduction_percent: f64,
        duration_ms: u64,
        method: String,
    },
    FirewallDecision {
        /// "tool", "model", "auto_router"
        firewall_type: String,
        item_name: String,
        /// "allow_once", "allow_session", "deny", etc.
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration: Option<String>,
    },
    SseConnection {
        session_id: String,
        /// "opened" or "closed"
        action: String,
    },
}

/// A flagged safety category from guardrails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlaggedCategory {
    pub category: String,
    pub confidence: f64,
    pub action: String,
}

/// Filter criteria for listing monitor events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MonitorEventFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_types: Option<Vec<MonitorEventType>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<EventStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Response for paginated event list queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorEventListResponse {
    pub events: Vec<MonitorEventSummary>,
    pub total: usize,
}

/// Statistics about the monitor store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorStats {
    pub total_events: usize,
    pub max_capacity: usize,
    pub events_by_type: std::collections::HashMap<MonitorEventType, usize>,
}
