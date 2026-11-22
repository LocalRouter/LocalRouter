//! MCP metrics-related Tauri commands
//!
//! Commands for retrieving MCP metrics data for charts and dashboards.

use std::sync::Arc;
use tauri::State;

use lr_monitoring::graphs::{GraphData, TimeRange};
use lr_monitoring::mcp_graphs::{McpGraphGenerator, McpMetricType};
use lr_server::ServerManager;

/// Get global MCP metrics for dashboard
#[tauri::command]
pub async fn get_global_mcp_metrics(
    time_range: TimeRange,
    metric_type: McpMetricType,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();
    let data_points = app_state
        .metrics_collector
        .mcp()
        .get_global_range(start, end);

    // Use bucketed generation for consistent time intervals
    Ok(McpGraphGenerator::generate_bucketed(
        &data_points,
        metric_type,
        Some("Global MCP"),
        time_range,
    ))
}

/// Get client-specific MCP metrics
#[tauri::command]
pub async fn get_client_mcp_metrics(
    client_id: String,
    time_range: TimeRange,
    metric_type: McpMetricType,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();
    let data_points = app_state
        .metrics_collector
        .mcp()
        .get_client_range(&client_id, start, end);

    // Get client name from client manager (if available) instead of using client_id
    let label = app_state
        .client_manager
        .get_client(&client_id)
        .map(|c| c.name)
        .unwrap_or_else(|| "Client".to_string());

    // Use bucketed generation for consistent time intervals
    Ok(McpGraphGenerator::generate_bucketed(
        &data_points,
        metric_type,
        Some(&label),
        time_range,
    ))
}

/// Get MCP server-specific metrics
#[tauri::command]
pub async fn get_mcp_server_metrics(
    server_id: String,
    time_range: TimeRange,
    metric_type: McpMetricType,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();
    let data_points = app_state
        .metrics_collector
        .mcp()
        .get_server_range(&server_id, start, end);

    // Get server name from mcp_server_manager (if available) instead of using server_id
    let label = app_state
        .mcp_server_manager
        .get_config(&server_id)
        .map(|s| s.name)
        .unwrap_or_else(|| "MCP Server".to_string());

    // Use bucketed generation for consistent time intervals
    Ok(McpGraphGenerator::generate_bucketed(
        &data_points,
        metric_type,
        Some(&label),
        time_range,
    ))
}

/// Get MCP method breakdown for a scope (global, client, or server)
#[tauri::command]
pub async fn get_mcp_method_breakdown(
    scope: String,
    time_range: TimeRange,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();

    let data_points = if scope == "global" {
        app_state
            .metrics_collector
            .mcp()
            .get_global_range(start, end)
    } else if let Some(client_id) = scope.strip_prefix("client:") {
        app_state
            .metrics_collector
            .mcp()
            .get_client_range(client_id, start, end)
    } else if let Some(server_id) = scope.strip_prefix("server:") {
        app_state
            .metrics_collector
            .mcp()
            .get_server_range(server_id, start, end)
    } else {
        return Err(format!(
            "Invalid scope: {}. Expected 'global', 'client:<id>', or 'server:<id>'",
            scope
        ));
    };

    // Use bucketed generation for consistent time intervals
    Ok(McpGraphGenerator::generate_method_breakdown_bucketed(
        &data_points,
        time_range,
    ))
}
