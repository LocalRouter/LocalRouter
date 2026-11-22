//! Metrics-related Tauri commands
//!
//! Commands for retrieving metrics data for charts and dashboards.

use std::sync::Arc;
use tauri::State;

use lr_config::{ConfigManager, RateLimitType};
use lr_monitoring::graphs::{GraphData, GraphGenerator, MetricType, RateLimitInfo, TimeRange};
use lr_monitoring::metrics::MetricsCollector;
use lr_monitoring::traffic::TrafficTimeline;
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

/// Dashboard traffic chart: finished LLM and MCP requests, errors and the
/// peak number of requests in progress per bucket (60–96 buckets per range).
#[tauri::command]
pub async fn get_traffic_timeline(
    time_range: TimeRange,
    metrics_collector: State<'_, Arc<MetricsCollector>>,
) -> Result<TrafficTimeline, String> {
    metrics_collector
        .traffic_timeline(time_range)
        .map_err(|e| format!("Failed to read traffic: {e}"))
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
