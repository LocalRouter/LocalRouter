//! Tests for metrics Tauri commands
//!
//! Simulates calling Tauri commands directly (without Tauri runtime).

use localrouter_ai::monitoring::graphs::{GraphData, GraphGenerator, MetricType, TimeRange};
use localrouter_ai::monitoring::metrics::{MetricsCollector, RequestMetrics};
use std::sync::Arc;

/// Simulate get_global_metrics command
fn simulate_get_global_metrics(
    collector: &Arc<MetricsCollector>,
    time_range: TimeRange,
    metric_type: MetricType,
) -> Result<GraphData, String> {
    let (start, end) = time_range.get_range();
    let data_points = collector.get_global_range(start, end);
    Ok(GraphGenerator::generate(
        &data_points,
        metric_type,
        Some("Global"),
    ))
}

/// Simulate get_api_key_metrics command
fn simulate_get_api_key_metrics(
    collector: &Arc<MetricsCollector>,
    api_key_id: String,
    time_range: TimeRange,
    metric_type: MetricType,
) -> Result<GraphData, String> {
    let (start, end) = time_range.get_range();
    let data_points = collector.get_key_range(&api_key_id, start, end);
    Ok(GraphGenerator::generate(
        &data_points,
        metric_type,
        Some(&api_key_id),
    ))
}

/// Simulate get_provider_metrics command
fn simulate_get_provider_metrics(
    collector: &Arc<MetricsCollector>,
    provider: String,
    time_range: TimeRange,
    metric_type: MetricType,
) -> Result<GraphData, String> {
    let (start, end) = time_range.get_range();
    let data_points = collector.get_provider_range(&provider, start, end);
    Ok(GraphGenerator::generate(
        &data_points,
        metric_type,
        Some(&provider),
    ))
}

/// Simulate get_model_metrics command
fn simulate_get_model_metrics(
    collector: &Arc<MetricsCollector>,
    model: String,
    time_range: TimeRange,
    metric_type: MetricType,
) -> Result<GraphData, String> {
    let (start, end) = time_range.get_range();
    let data_points = collector.get_model_range(&model, start, end);
    Ok(GraphGenerator::generate(
        &data_points,
        metric_type,
        Some(&model),
    ))
}

/// Simulate compare_providers command
fn simulate_compare_providers(
    collector: &Arc<MetricsCollector>,
    providers: Vec<String>,
    time_range: TimeRange,
    metric_type: MetricType,
) -> Result<GraphData, String> {
    let (start, end) = time_range.get_range();

    let data_sets: Vec<(String, Vec<_>)> = providers
        .iter()
        .map(|provider| {
            let points = collector.get_provider_range(provider, start, end);
            (provider.clone(), points)
        })
        .collect();

    let data_sets_refs: Vec<(&str, &[_])> = data_sets
        .iter()
        .map(|(provider, points)| (provider.as_str(), points.as_slice()))
        .collect();

    Ok(GraphGenerator::generate_multi(data_sets_refs, metric_type))
}

/// Simulate compare_api_keys command
fn simulate_compare_api_keys(
    collector: &Arc<MetricsCollector>,
    api_key_ids: Vec<String>,
    time_range: TimeRange,
    metric_type: MetricType,
) -> Result<GraphData, String> {
    let (start, end) = time_range.get_range();

    let data_sets: Vec<(String, Vec<_>)> = api_key_ids
        .iter()
        .map(|key_id| {
            let points = collector.get_key_range(key_id, start, end);
            (key_id.clone(), points)
        })
        .collect();

    let data_sets_refs: Vec<(&str, &[_])> = data_sets
        .iter()
        .map(|(key_id, points)| (key_id.as_str(), points.as_slice()))
        .collect();

    Ok(GraphGenerator::generate_multi(data_sets_refs, metric_type))
}

#[test]
fn test_get_global_metrics_command() {
    let collector = Arc::new(MetricsCollector::with_default_retention());

    collector.record_success(&RequestMetrics {
        api_key_name: "key1",
        provider: "openai",
        model: "gpt-4",
        input_tokens: 1000,
        output_tokens: 500,
        cost_usd: 0.05,
        latency_ms: 100,
    });

    let result = simulate_get_global_metrics(&collector, TimeRange::Day, MetricType::Tokens);

    assert!(result.is_ok());
    let graph = result.unwrap();
    assert_eq!(graph.datasets.len(), 1);
    assert_eq!(graph.datasets[0].label, "Global");
}

#[test]
fn test_get_api_key_metrics_command() {
    let collector = Arc::new(MetricsCollector::with_default_retention());

    collector.record_success(&RequestMetrics {
        api_key_name: "my_key",
        provider: "openai",
        model: "gpt-4",
        input_tokens: 1000,
        output_tokens: 500,
        cost_usd: 0.05,
        latency_ms: 100,
    });

    let result = simulate_get_api_key_metrics(
        &collector,
        "my_key".to_string(),
        TimeRange::Hour,
        MetricType::Cost,
    );

    assert!(result.is_ok());
    let graph = result.unwrap();
    assert_eq!(graph.datasets[0].label, "my_key");
    assert!((graph.datasets[0].data[0] - 0.05).abs() < 0.0001);
}

#[test]
fn test_get_provider_metrics_command() {
    let collector = Arc::new(MetricsCollector::with_default_retention());

    collector.record_success(&RequestMetrics {
        api_key_name: "key1",
        provider: "anthropic",
        model: "claude-3.5-sonnet",
        input_tokens: 800,
        output_tokens: 400,
        cost_usd: 0.03,
        latency_ms: 80,
    });

    let result = simulate_get_provider_metrics(
        &collector,
        "anthropic".to_string(),
        TimeRange::Day,
        MetricType::Requests,
    );

    assert!(result.is_ok());
    let graph = result.unwrap();
    assert_eq!(graph.datasets[0].label, "anthropic");
    assert_eq!(graph.datasets[0].data[0] as u64, 1);
}

#[test]
fn test_get_model_metrics_command() {
    let collector = Arc::new(MetricsCollector::with_default_retention());

    collector.record_success(&RequestMetrics {
        api_key_name: "key1",
        provider: "openai",
        model: "gpt-4",
        input_tokens: 1000,
        output_tokens: 500,
        cost_usd: 0.05,
        latency_ms: 100,
    });
    collector.record_success(&RequestMetrics {
        api_key_name: "key2",
        provider: "openai",
        model: "gpt-4",
        input_tokens: 500,
        output_tokens: 250,
        cost_usd: 0.025,
        latency_ms: 90,
    });

    let result = simulate_get_model_metrics(
        &collector,
        "gpt-4".to_string(),
        TimeRange::Day,
        MetricType::Requests,
    );

    assert!(result.is_ok());
    let graph = result.unwrap();
    assert_eq!(graph.datasets[0].label, "gpt-4");
    assert_eq!(graph.datasets[0].data[0] as u64, 2);
}

#[test]
fn test_compare_providers_command() {
    let collector = Arc::new(MetricsCollector::with_default_retention());

    collector.record_success(&RequestMetrics {
        api_key_name: "key1",
        provider: "openai",
        model: "gpt-4",
        input_tokens: 1000,
        output_tokens: 500,
        cost_usd: 0.05,
        latency_ms: 100,
    });
    collector.record_success(&RequestMetrics {
        api_key_name: "key1",
        provider: "anthropic",
        model: "claude-3.5-sonnet",
        input_tokens: 800,
        output_tokens: 400,
        cost_usd: 0.03,
        latency_ms: 80,
    });

    let result = simulate_compare_providers(
        &collector,
        vec!["openai".to_string(), "anthropic".to_string()],
        TimeRange::Day,
        MetricType::Requests,
    );

    assert!(result.is_ok());
    let graph = result.unwrap();
    assert_eq!(graph.datasets.len(), 2);
    assert_eq!(graph.datasets[0].label, "openai");
    assert_eq!(graph.datasets[1].label, "anthropic");
}

#[test]
fn test_compare_api_keys_command() {
    let collector = Arc::new(MetricsCollector::with_default_retention());

    collector.record_success(&RequestMetrics {
        api_key_name: "key1",
        provider: "openai",
        model: "gpt-4",
        input_tokens: 1000,
        output_tokens: 500,
        cost_usd: 0.05,
        latency_ms: 100,
    });
    collector.record_success(&RequestMetrics {
        api_key_name: "key1",
        provider: "openai",
        model: "gpt-4",
        input_tokens: 500,
        output_tokens: 250,
        cost_usd: 0.025,
        latency_ms: 90,
    });
    collector.record_success(&RequestMetrics {
        api_key_name: "key2",
        provider: "anthropic",
        model: "claude-3.5-sonnet",
        input_tokens: 800,
        output_tokens: 400,
        cost_usd: 0.03,
        latency_ms: 80,
    });

    let result = simulate_compare_api_keys(
        &collector,
        vec!["key1".to_string(), "key2".to_string()],
        TimeRange::Day,
        MetricType::Cost,
    );

    assert!(result.is_ok());
    let graph = result.unwrap();
    assert_eq!(graph.datasets.len(), 2);
    assert_eq!(graph.datasets[0].label, "key1");
    assert_eq!(graph.datasets[1].label, "key2");
    assert!((graph.datasets[0].data[0] - 0.075).abs() < 0.0001); // key1: 0.05 + 0.025
    assert!((graph.datasets[1].data[0] - 0.03).abs() < 0.0001); // key2: 0.03
}

#[test]
fn test_all_metric_types() {
    let collector = Arc::new(MetricsCollector::with_default_retention());

    collector.record_success(&RequestMetrics {
        api_key_name: "key1",
        provider: "openai",
        model: "gpt-4",
        input_tokens: 1000,
        output_tokens: 500,
        cost_usd: 0.05,
        latency_ms: 100,
    });

    let metric_types = vec![
        MetricType::Tokens,
        MetricType::Cost,
        MetricType::Requests,
        MetricType::Latency,
        MetricType::SuccessRate,
    ];

    for metric_type in metric_types {
        let result = simulate_get_global_metrics(&collector, TimeRange::Day, metric_type);
        assert!(
            result.is_ok(),
            "Failed for metric type: {:?}",
            metric_type
        );
    }
}

#[test]
fn test_all_time_ranges() {
    let collector = Arc::new(MetricsCollector::with_default_retention());

    collector.record_success(&RequestMetrics {
        api_key_name: "key1",
        provider: "openai",
        model: "gpt-4",
        input_tokens: 1000,
        output_tokens: 500,
        cost_usd: 0.05,
        latency_ms: 100,
    });

    let time_ranges = vec![
        TimeRange::Hour,
        TimeRange::Day,
        TimeRange::Week,
        TimeRange::Month,
    ];

    for time_range in time_ranges {
        let result = simulate_get_global_metrics(&collector, time_range, MetricType::Requests);
        assert!(result.is_ok(), "Failed for time range: {:?}", time_range);
    }
}
