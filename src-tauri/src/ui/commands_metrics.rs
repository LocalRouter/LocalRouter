//! Metrics-related Tauri commands
//!
//! Commands for retrieving metrics data for charts and dashboards.

use std::sync::Arc;
use tauri::State;

use crate::monitoring::graphs::{GraphData, GraphGenerator, MetricType, TimeRange};
use crate::server::ServerManager;

/// Get global metrics for dashboard
#[tauri::command]
pub async fn get_global_metrics(
    time_range: TimeRange,
    metric_type: MetricType,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();
    let data_points = app_state.metrics_collector.get_global_range(start, end);

    Ok(GraphGenerator::generate(&data_points, metric_type, Some("Global")))
}

/// Get API key specific metrics
#[tauri::command]
pub async fn get_api_key_metrics(
    api_key_id: String,
    time_range: TimeRange,
    metric_type: MetricType,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();
    let data_points = app_state.metrics_collector.get_key_range(&api_key_id, start, end);

    Ok(GraphGenerator::generate(&data_points, metric_type, Some(&api_key_id)))
}

/// Get provider specific metrics
#[tauri::command]
pub async fn get_provider_metrics(
    provider: String,
    time_range: TimeRange,
    metric_type: MetricType,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();
    let data_points = app_state.metrics_collector.get_provider_range(&provider, start, end);

    Ok(GraphGenerator::generate(&data_points, metric_type, Some(&provider)))
}

/// Get model specific metrics
#[tauri::command]
pub async fn get_model_metrics(
    model: String,
    time_range: TimeRange,
    metric_type: MetricType,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();
    let data_points = app_state.metrics_collector.get_model_range(&model, start, end);

    Ok(GraphGenerator::generate(&data_points, metric_type, Some(&model)))
}

/// List all tracked models
#[tauri::command]
pub async fn list_tracked_models(
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<Vec<String>, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    Ok(app_state.metrics_collector.get_model_names())
}

/// List all tracked providers
#[tauri::command]
pub async fn list_tracked_providers(
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<Vec<String>, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    Ok(app_state.metrics_collector.get_provider_names())
}

/// Compare multiple API keys (stacked/multi-line chart)
#[tauri::command]
pub async fn compare_api_keys(
    api_key_ids: Vec<String>,
    time_range: TimeRange,
    metric_type: MetricType,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();

    let data_sets: Vec<(String, Vec<_>)> = api_key_ids
        .iter()
        .map(|id| {
            let points = app_state.metrics_collector.get_key_range(id, start, end);
            (id.clone(), points)
        })
        .collect();

    let data_sets_refs: Vec<(&str, &[_])> = data_sets
        .iter()
        .map(|(id, points)| (id.as_str(), points.as_slice()))
        .collect();

    Ok(GraphGenerator::generate_multi(data_sets_refs, metric_type))
}

/// Compare multiple providers (stacked chart for cost breakdown)
#[tauri::command]
pub async fn compare_providers(
    providers: Vec<String>,
    time_range: TimeRange,
    metric_type: MetricType,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();

    let data_sets: Vec<(String, Vec<_>)> = providers
        .iter()
        .map(|provider| {
            let points = app_state.metrics_collector.get_provider_range(provider, start, end);
            (provider.clone(), points)
        })
        .collect();

    let data_sets_refs: Vec<(&str, &[_])> = data_sets
        .iter()
        .map(|(provider, points)| (provider.as_str(), points.as_slice()))
        .collect();

    Ok(GraphGenerator::generate_multi(data_sets_refs, metric_type))
}
