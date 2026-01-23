//! MCP metrics-related Tauri commands
//!
//! Commands for retrieving MCP metrics data for charts and dashboards.

use std::sync::Arc;
use tauri::State;

use crate::monitoring::graphs::{GraphData, TimeRange};
use crate::monitoring::mcp_graphs::{McpGraphGenerator, McpMetricType};
use crate::server::ServerManager;

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
    Ok(McpGraphGenerator::generate_method_breakdown_bucketed(&data_points, time_range))
}

/// List all tracked MCP clients
#[tauri::command]
pub async fn list_tracked_mcp_clients(
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<Vec<String>, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    Ok(app_state.metrics_collector.mcp().get_client_ids())
}

/// List all tracked MCP servers
#[tauri::command]
pub async fn list_tracked_mcp_servers(
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<Vec<String>, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    Ok(app_state.metrics_collector.mcp().get_server_ids())
}

/// Compare multiple clients (multi-line chart)
#[tauri::command]
pub async fn compare_mcp_clients(
    client_ids: Vec<String>,
    time_range: TimeRange,
    metric_type: McpMetricType,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();

    let data_sets: Vec<(String, Vec<_>)> = client_ids
        .iter()
        .map(|id| {
            let points = app_state
                .metrics_collector
                .mcp()
                .get_client_range(id, start, end);
            let label = app_state
                .client_manager
                .get_client(id)
                .map(|c| c.name)
                .unwrap_or_else(|| id.clone());
            (label, points)
        })
        .collect();

    let data_sets_refs: Vec<(&str, &[_])> = data_sets
        .iter()
        .map(|(label, points)| (label.as_str(), points.as_slice()))
        .collect();

    // Use bucketed generation for consistent time intervals
    Ok(McpGraphGenerator::generate_multi_bucketed(
        data_sets_refs,
        metric_type,
        time_range,
    ))
}

/// Compare multiple MCP servers (multi-line chart)
#[tauri::command]
pub async fn compare_mcp_servers(
    server_ids: Vec<String>,
    time_range: TimeRange,
    metric_type: McpMetricType,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();

    let data_sets: Vec<(String, Vec<_>)> = server_ids
        .iter()
        .map(|id| {
            let points = app_state
                .metrics_collector
                .mcp()
                .get_server_range(id, start, end);
            let label = app_state
                .mcp_server_manager
                .get_config(id)
                .map(|s| s.name)
                .unwrap_or_else(|| id.clone());
            (label, points)
        })
        .collect();

    let data_sets_refs: Vec<(&str, &[_])> = data_sets
        .iter()
        .map(|(label, points)| (label.as_str(), points.as_slice()))
        .collect();

    // Use bucketed generation for consistent time intervals
    Ok(McpGraphGenerator::generate_multi_bucketed(
        data_sets_refs,
        metric_type,
        time_range,
    ))
}

/// Get MCP latency percentiles for a scope
#[tauri::command]
pub async fn get_mcp_latency_percentiles(
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

    Ok(McpGraphGenerator::generate_latency_percentiles(
        &data_points,
    ))
}
