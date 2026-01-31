//! Metrics-related Tauri commands
//!
//! Commands for retrieving metrics data for charts and dashboards.

use std::sync::Arc;
use tauri::State;

use lr_config::{ConfigManager, RateLimitType};
use lr_monitoring::graphs::{GraphData, GraphGenerator, MetricType, RateLimitInfo, TimeRange};
use lr_server::ServerManager;

/// Helper function to get rate limits for a client based on metric type
///
/// Returns a vector of RateLimitInfo for the relevant rate limiters that match
/// the given metric type. For example, if metric_type is Requests, it will return
/// the Requests rate limiter. If metric_type is Tokens, it will return InputTokens,
/// OutputTokens, and TotalTokens rate limiters.
fn get_client_rate_limits(
    config_manager: &ConfigManager,
    client_id: &str,
    metric_type: MetricType,
    time_range: TimeRange,
) -> Vec<RateLimitInfo> {
    let config = config_manager.get();

    // Find the client
    let client = match config.clients.iter().find(|c| c.id == client_id) {
        Some(c) => c,
        None => return vec![],
    };

    // Find the strategy
    let strategy = match config
        .strategies
        .iter()
        .find(|s| s.id == client.strategy_id)
    {
        Some(s) => s,
        None => return vec![],
    };

    // Get the time range in seconds for filtering
    let time_range_seconds = time_range.duration().num_seconds();

    // Filter rate limits based on metric type and time range
    strategy
        .rate_limits
        .iter()
        .filter_map(|limit| {
            // Only include limits that match the current time range
            if limit.time_window.to_seconds() != time_range_seconds {
                return None;
            }

            // Map rate limit type to metric type
            let is_match = matches!(
                (metric_type, limit.limit_type),
                (MetricType::Requests, RateLimitType::Requests)
                    | (MetricType::Tokens, RateLimitType::InputTokens)
                    | (MetricType::Tokens, RateLimitType::OutputTokens)
                    | (MetricType::Tokens, RateLimitType::TotalTokens)
                    | (MetricType::Cost, RateLimitType::Cost)
            );

            if is_match {
                Some(RateLimitInfo::new(
                    format!("{:?}", limit.limit_type),
                    limit.value,
                    limit.time_window.to_seconds(),
                ))
            } else {
                None
            }
        })
        .collect()
}

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

    // Use bucketed generation for consistent time intervals
    Ok(GraphGenerator::generate_bucketed(
        &data_points,
        metric_type,
        Some("Global"),
        time_range,
    ))
}

/// Get API key specific metrics
#[tauri::command]
pub async fn get_api_key_metrics(
    api_key_id: String,
    time_range: TimeRange,
    metric_type: MetricType,
    server_manager: State<'_, Arc<ServerManager>>,
    config_manager: State<'_, ConfigManager>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();
    let data_points = app_state
        .metrics_collector
        .get_key_range(&api_key_id, start, end);

    // Look up client name for the label
    let config = config_manager.get();
    let label = config
        .clients
        .iter()
        .find(|c| c.id == api_key_id)
        .map(|c| c.name.as_str())
        .unwrap_or(&api_key_id);

    // Use bucketed generation for consistent time intervals
    let mut graph_data =
        GraphGenerator::generate_bucketed(&data_points, metric_type, Some(label), time_range);

    // Add rate limits if available
    let rate_limits = get_client_rate_limits(&config_manager, &api_key_id, metric_type, time_range);
    if !rate_limits.is_empty() {
        graph_data = graph_data.set_rate_limits(rate_limits);
    }

    Ok(graph_data)
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
    let data_points = app_state
        .metrics_collector
        .get_provider_range(&provider, start, end);

    // Use bucketed generation for consistent time intervals
    Ok(GraphGenerator::generate_bucketed(
        &data_points,
        metric_type,
        Some(&provider),
        time_range,
    ))
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
    let data_points = app_state
        .metrics_collector
        .get_model_range(&model, start, end);

    // Use bucketed generation for consistent time intervals
    Ok(GraphGenerator::generate_bucketed(
        &data_points,
        metric_type,
        Some(&model),
        time_range,
    ))
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

/// List all tracked API keys (clients)
#[tauri::command]
pub async fn list_tracked_api_keys(
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<Vec<String>, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    Ok(app_state.metrics_collector.get_api_key_names())
}

/// Compare multiple API keys (stacked/multi-line chart)
#[tauri::command]
pub async fn compare_api_keys(
    api_key_ids: Vec<String>,
    time_range: TimeRange,
    metric_type: MetricType,
    server_manager: State<'_, Arc<ServerManager>>,
    config_manager: State<'_, ConfigManager>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();

    // Build a map of client id -> client name for label lookup
    let config = config_manager.get();
    let client_names: std::collections::HashMap<String, String> = config
        .clients
        .iter()
        .map(|c| (c.id.clone(), c.name.clone()))
        .collect();

    let data_sets: Vec<(String, Vec<_>)> = api_key_ids
        .iter()
        .map(|id| {
            let points = app_state.metrics_collector.get_key_range(id, start, end);
            // Use client name if available, otherwise fall back to ID
            let label = client_names.get(id).cloned().unwrap_or_else(|| id.clone());
            (label, points)
        })
        .collect();

    let data_sets_refs: Vec<(&str, &[_])> = data_sets
        .iter()
        .map(|(label, points)| (label.as_str(), points.as_slice()))
        .collect();

    // Use bucketed graph generation for consistent time intervals
    let mut graph_data =
        GraphGenerator::generate_multi_bucketed(data_sets_refs, metric_type, time_range);

    // Collect rate limits from all clients
    // We'll show all unique rate limits (deduplicated by value)
    let mut all_rate_limits: Vec<RateLimitInfo> = Vec::new();
    for api_key_id in &api_key_ids {
        let limits = get_client_rate_limits(&config_manager, api_key_id, metric_type, time_range);
        for limit in limits {
            // Only add if we don't already have a limit with the same type and value
            if !all_rate_limits
                .iter()
                .any(|l| l.limit_type == limit.limit_type && l.value == limit.value)
            {
                all_rate_limits.push(limit);
            }
        }
    }

    if !all_rate_limits.is_empty() {
        graph_data = graph_data.set_rate_limits(all_rate_limits);
    }

    Ok(graph_data)
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
            let points = app_state
                .metrics_collector
                .get_provider_range(provider, start, end);
            (provider.clone(), points)
        })
        .collect();

    let data_sets_refs: Vec<(&str, &[_])> = data_sets
        .iter()
        .map(|(provider, points)| (provider.as_str(), points.as_slice()))
        .collect();

    // Use bucketed generation for consistent time intervals
    Ok(GraphGenerator::generate_multi_bucketed(
        data_sets_refs,
        metric_type,
        time_range,
    ))
}

/// Compare multiple models (stacked chart for model usage breakdown)
#[tauri::command]
pub async fn compare_models(
    models: Vec<String>,
    time_range: TimeRange,
    metric_type: MetricType,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();

    let data_sets: Vec<(String, Vec<_>)> = models
        .iter()
        .map(|model| {
            let points = app_state
                .metrics_collector
                .get_model_range(model, start, end);
            (model.clone(), points)
        })
        .collect();

    let data_sets_refs: Vec<(&str, &[_])> = data_sets
        .iter()
        .map(|(model, points)| (model.as_str(), points.as_slice()))
        .collect();

    // Use bucketed generation for consistent time intervals
    Ok(GraphGenerator::generate_multi_bucketed(
        data_sets_refs,
        metric_type,
        time_range,
    ))
}

/// Get strategy-specific metrics
#[tauri::command]
pub async fn get_strategy_metrics(
    strategy_id: String,
    time_range: TimeRange,
    metric_type: MetricType,
    server_manager: State<'_, Arc<ServerManager>>,
    config_manager: State<'_, ConfigManager>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();
    let data_points = app_state
        .metrics_collector
        .get_strategy_range(&strategy_id, start, end);

    // Use bucketed generation for consistent time intervals
    let mut graph_data = GraphGenerator::generate_bucketed(
        &data_points,
        metric_type,
        Some(&strategy_id),
        time_range,
    );

    // Add rate limits for this strategy
    let config = config_manager.get();
    if let Some(strategy) = config.strategies.iter().find(|s| s.id == strategy_id) {
        let time_range_seconds = time_range.duration().num_seconds();

        let rate_limits: Vec<RateLimitInfo> = strategy
            .rate_limits
            .iter()
            .filter_map(|limit| {
                // Only include limits that match the current time range
                if limit.time_window.to_seconds() != time_range_seconds {
                    return None;
                }

                // Map rate limit type to metric type
                let is_match = matches!(
                    (metric_type, limit.limit_type),
                    (MetricType::Requests, RateLimitType::Requests)
                        | (MetricType::Tokens, RateLimitType::InputTokens)
                        | (MetricType::Tokens, RateLimitType::OutputTokens)
                        | (MetricType::Tokens, RateLimitType::TotalTokens)
                        | (MetricType::Cost, RateLimitType::Cost)
                );

                if is_match {
                    Some(RateLimitInfo::new(
                        format!("{:?}", limit.limit_type),
                        limit.value,
                        limit.time_window.to_seconds(),
                    ))
                } else {
                    None
                }
            })
            .collect();

        if !rate_limits.is_empty() {
            graph_data = graph_data.set_rate_limits(rate_limits);
        }
    }

    Ok(graph_data)
}

/// List all tracked strategies (strategies that have recorded metrics)
#[tauri::command]
pub async fn list_tracked_strategies(
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<Vec<String>, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    Ok(app_state.metrics_collector.get_strategy_ids())
}

/// Compare multiple strategies (stacked chart for strategy usage breakdown)
#[tauri::command]
pub async fn compare_strategies(
    strategy_ids: Vec<String>,
    time_range: TimeRange,
    metric_type: MetricType,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<GraphData, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let (start, end) = time_range.get_range();

    let data_sets: Vec<(String, Vec<_>)> = strategy_ids
        .iter()
        .map(|strategy_id| {
            let points = app_state
                .metrics_collector
                .get_strategy_range(strategy_id, start, end);
            (strategy_id.clone(), points)
        })
        .collect();

    let data_sets_refs: Vec<(&str, &[_])> = data_sets
        .iter()
        .map(|(strategy_id, points)| (strategy_id.as_str(), points.as_slice()))
        .collect();

    // Use bucketed generation for consistent time intervals
    Ok(GraphGenerator::generate_multi_bucketed(
        data_sets_refs,
        metric_type,
        time_range,
    ))
}
