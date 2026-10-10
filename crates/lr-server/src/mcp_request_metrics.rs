//! MCP request metrics from the gateway's monitor events.
//!
//! The gateway reports every client→server request (tool call, resource
//! read, prompt get) as a monitor event that goes from pending to complete
//! or error. Recording a metric at that transition puts gateway traffic on
//! the MCP traffic graphs.

use lr_monitor::{EventStatus, MonitorEvent, MonitorEventData};

/// One finished MCP request, ready for `McpMetricsCollector::record`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedMcpRequest {
    pub client_id: String,
    pub server_id: String,
    pub method: &'static str,
    pub latency_ms: u64,
    pub success: bool,
}

impl FinishedMcpRequest {
    pub fn as_metrics(&self) -> lr_monitoring::mcp_metrics::McpRequestMetrics<'_> {
        lr_monitoring::mcp_metrics::McpRequestMetrics {
            client_id: &self.client_id,
            server_id: &self.server_id,
            method: self.method,
            latency_ms: self.latency_ms,
            success: self.success,
            error_code: None,
        }
    }
}

/// The finished request an event describes, if it is a terminal MCP request.
pub fn finished_mcp_request(event: &MonitorEvent) -> Option<FinishedMcpRequest> {
    if event.status == EventStatus::Pending {
        return None;
    }
    let (server_id, method, latency_ms) = match &event.data {
        MonitorEventData::McpToolCall {
            server_id,
            latency_ms,
            ..
        } => (server_id, "tools/call", latency_ms),
        MonitorEventData::McpResourceRead {
            server_id,
            latency_ms,
            ..
        } => (server_id, "resources/read", latency_ms),
        MonitorEventData::McpPromptGet {
            server_id,
            latency_ms,
            ..
        } => (server_id, "prompts/get", latency_ms),
        _ => return None,
    };
    Some(FinishedMcpRequest {
        client_id: event.client_id.clone().unwrap_or_default(),
        server_id: server_id.clone(),
        method,
        latency_ms: latency_ms.or(event.duration_ms).unwrap_or(0),
        success: event.status == EventStatus::Complete,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_monitor::{MonitorEventStore, MonitorEventType};

    fn tool_call(status: EventStatus) -> MonitorEvent {
        let store = MonitorEventStore::new(4);
        let id = store.push(
            MonitorEventType::McpToolCall,
            Some("client-1".into()),
            None,
            None,
            MonitorEventData::McpToolCall {
                tool_name: "fs__read".into(),
                server_id: "fs".into(),
                server_name: None,
                arguments: serde_json::json!({}),
                firewall_action: None,
                latency_ms: Some(42),
                success: Some(status == EventStatus::Complete),
                response_preview: None,
                error: None,
            },
            status,
            Some(42),
        );
        store.get(&id).unwrap()
    }

    #[test]
    fn terminal_tool_calls_become_metrics() {
        assert_eq!(
            finished_mcp_request(&tool_call(EventStatus::Complete)),
            Some(FinishedMcpRequest {
                client_id: "client-1".into(),
                server_id: "fs".into(),
                method: "tools/call",
                latency_ms: 42,
                success: true,
            })
        );
        assert!(
            !finished_mcp_request(&tool_call(EventStatus::Error))
                .unwrap()
                .success
        );
    }

    #[test]
    fn pending_and_non_request_events_are_skipped() {
        assert_eq!(finished_mcp_request(&tool_call(EventStatus::Pending)), None);
        let mut event = tool_call(EventStatus::Complete);
        event.data = MonitorEventData::SseConnection {
            session_id: "s".into(),
            action: "opened".into(),
        };
        assert_eq!(finished_mcp_request(&event), None);
    }
}
