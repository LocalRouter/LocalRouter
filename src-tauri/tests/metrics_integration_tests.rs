//! Integration tests for metrics system
//!
//! Tests the complete flow of recording metrics, querying data, and generating graphs.

use chrono::{Duration, Utc};
use localrouter_ai::monitoring::graphs::{GraphGenerator, MetricType};
use localrouter_ai::monitoring::metrics::{MetricsCollector, RequestMetrics};
use std::sync::Arc;

/// Test helper: Create MetricsCollector and record sample data
fn create_test_collector_with_data() -> Arc<MetricsCollector> {
    let collector = Arc::new(MetricsCollector::with_default_retention());

    // Record varied metrics across all four tiers
    collector.record_success(&RequestMetrics {
        api_key_name: "api_key_1",
        provider: "openai",
        model: "gpt-4",
        input_tokens: 1000,
        output_tokens: 500,
        cost_usd: 0.05,
        latency_ms: 100,
    });
    collector.record_success(&RequestMetrics {
        api_key_name: "api_key_1",
        provider: "openai",
        model: "gpt-3.5-turbo",
        input_tokens: 500,
        output_tokens: 250,
        cost_usd: 0.005,
        latency_ms: 50,
    });
    collector.record_success(&RequestMetrics {
        api_key_name: "api_key_1",
        provider: "anthropic",
        model: "claude-3.5-sonnet",
        input_tokens: 800,
        output_tokens: 400,
        cost_usd: 0.03,
        latency_ms: 80,
    });

    collector.record_success(&RequestMetrics {
        api_key_name: "api_key_2",
        provider: "openai",
        model: "gpt-4",
        input_tokens: 600,
        output_tokens: 300,
        cost_usd: 0.03,
        latency_ms: 90,
    });
    collector.record_success(&RequestMetrics {
        api_key_name: "api_key_2",
        provider: "groq",
        model: "llama-3.3-70b",
        input_tokens: 2000,
        output_tokens: 1000,
        cost_usd: 0.0,
        latency_ms: 200,
    });

    collector.record_failure("api_key_1", "openai", "gpt-4", 1000);

    collector
}

#[test]
fn test_global_metrics_retrieval() {
    let collector = create_test_collector_with_data();
    let now = Utc::now();

    let data_points = collector.get_global_range(
        now - Duration::minutes(5),
        now + Duration::minutes(5),
    );

    // Verify aggregation
    assert_eq!(data_points.len(), 1);
    assert_eq!(data_points[0].requests, 6); // 5 success + 1 failure
    assert_eq!(data_points[0].successful_requests, 5);
    assert_eq!(data_points[0].failed_requests, 1);

    // Verify token totals
    let total_input = 1000 + 500 + 800 + 600 + 2000;
    let total_output = 500 + 250 + 400 + 300 + 1000;
    assert_eq!(data_points[0].input_tokens, total_input);
    assert_eq!(data_points[0].output_tokens, total_output);

    // Verify cost
    let expected_cost = 0.05 + 0.005 + 0.03 + 0.03 + 0.0;
    assert!((data_points[0].cost_usd - expected_cost).abs() < 0.0001);
}

#[test]
fn test_api_key_metrics_isolation() {
    let collector = create_test_collector_with_data();
    let now = Utc::now();

    // Test key1
    let key1_data = collector.get_key_range(
        "api_key_1",
        now - Duration::minutes(5),
        now + Duration::minutes(5),
    );

    assert_eq!(key1_data.len(), 1);
    assert_eq!(key1_data[0].requests, 4); // 3 success + 1 failure
    assert_eq!(key1_data[0].successful_requests, 3);

    // Test key2
    let key2_data = collector.get_key_range(
        "api_key_2",
        now - Duration::minutes(5),
        now + Duration::minutes(5),
    );

    assert_eq!(key2_data.len(), 1);
    assert_eq!(key2_data[0].requests, 2);
    assert_eq!(key2_data[0].successful_requests, 2);
    assert_eq!(key2_data[0].failed_requests, 0);
}

#[test]
fn test_provider_metrics_isolation() {
    let collector = create_test_collector_with_data();
    let now = Utc::now();

    let openai_data = collector.get_provider_range(
        "openai",
        now - Duration::minutes(5),
        now + Duration::minutes(5),
    );

    // openai has: gpt-4 (2 success + 1 failure), gpt-3.5-turbo (1 success)
    assert_eq!(openai_data.len(), 1);
    assert_eq!(openai_data[0].requests, 4);
    assert_eq!(openai_data[0].successful_requests, 3);
}

#[test]
fn test_model_metrics_isolation() {
    let collector = create_test_collector_with_data();
    let now = Utc::now();

    let gpt4_data = collector.get_model_range(
        "gpt-4",
        now - Duration::minutes(5),
        now + Duration::minutes(5),
    );

    // gpt-4 used by key1 (1 success + 1 failure) and key2 (1 success)
    assert_eq!(gpt4_data.len(), 1);
    assert_eq!(gpt4_data[0].requests, 3);
    assert_eq!(gpt4_data[0].successful_requests, 2);
    assert_eq!(gpt4_data[0].failed_requests, 1);
}

#[test]
fn test_graph_generation_from_metrics() {
    let collector = create_test_collector_with_data();
    let now = Utc::now();

    let data_points = collector.get_global_range(
        now - Duration::minutes(5),
        now + Duration::minutes(5),
    );

    // Generate graph for tokens
    let graph = GraphGenerator::generate(&data_points, MetricType::Tokens, Some("Global"));

    assert_eq!(graph.labels.len(), 1);
    assert_eq!(graph.datasets.len(), 1);
    assert_eq!(graph.datasets[0].label, "Global");

    let expected_tokens = 1000 + 500 + 800 + 600 + 2000 + 500 + 250 + 400 + 300 + 1000;
    assert_eq!(graph.datasets[0].data[0] as u64, expected_tokens);
}

#[test]
fn test_time_range_filtering() {
    let collector = Arc::new(MetricsCollector::with_default_retention());
    let now = Utc::now();

    // Record at current time
    collector.record_success(&RequestMetrics {
        api_key_name: "key1",
        provider: "openai",
        model: "gpt-4",
        input_tokens: 1000,
        output_tokens: 500,
        cost_usd: 0.05,
        latency_ms: 100,
    });

    // Query with narrow range (should find it)
    let data = collector.get_global_range(
        now - Duration::minutes(1),
        now + Duration::minutes(1),
    );
    assert_eq!(data.len(), 1);

    // Query with range in the past (should not find it)
    let data = collector.get_global_range(
        now - Duration::hours(2),
        now - Duration::hours(1),
    );
    assert_eq!(data.len(), 0);
}

#[test]
fn test_cost_calculation_accuracy() {
    let collector = Arc::new(MetricsCollector::with_default_retention());
    let now = Utc::now();

    // Record with specific costs
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
        input_tokens: 2000,
        output_tokens: 1000,
        cost_usd: 0.10,
        latency_ms: 120,
    });

    let data = collector.get_global_range(
        now - Duration::minutes(5),
        now + Duration::minutes(5),
    );

    assert_eq!(data.len(), 1);
    assert!((data[0].cost_usd - 0.15).abs() < 0.0001);
}

#[test]
fn test_success_rate_calculation() {
    let collector = Arc::new(MetricsCollector::with_default_retention());
    let now = Utc::now();

    // 3 success, 1 failure = 75% success rate
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
        input_tokens: 1000,
        output_tokens: 500,
        cost_usd: 0.05,
        latency_ms: 100,
    });
    collector.record_success(&RequestMetrics {
        api_key_name: "key1",
        provider: "openai",
        model: "gpt-4",
        input_tokens: 1000,
        output_tokens: 500,
        cost_usd: 0.05,
        latency_ms: 100,
    });
    collector.record_failure("key1", "openai", "gpt-4", 1000);

    let data = collector.get_global_range(
        now - Duration::minutes(5),
        now + Duration::minutes(5),
    );

    assert_eq!(data.len(), 1);
    assert_eq!(data[0].requests, 4);
    assert_eq!(data[0].successful_requests, 3);
    assert_eq!(data[0].failed_requests, 1);

    // Generate success rate graph
    let graph = GraphGenerator::generate(&data, MetricType::SuccessRate, Some("Global"));
    assert_eq!(graph.datasets[0].data[0], 75.0);
}

#[test]
fn test_latency_aggregation() {
    let collector = Arc::new(MetricsCollector::with_default_retention());
    let now = Utc::now();

    // Record with different latencies
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
        input_tokens: 1000,
        output_tokens: 500,
        cost_usd: 0.05,
        latency_ms: 200,
    });
    collector.record_success(&RequestMetrics {
        api_key_name: "key1",
        provider: "openai",
        model: "gpt-4",
        input_tokens: 1000,
        output_tokens: 500,
        cost_usd: 0.05,
        latency_ms: 300,
    });

    let data = collector.get_global_range(
        now - Duration::minutes(5),
        now + Duration::minutes(5),
    );

    assert_eq!(data.len(), 1);

    // Average latency should be (100 + 200 + 300) / 3 = 200
    let avg_latency = data[0].avg_latency_ms();
    // Use epsilon comparison for floating point
    assert!((avg_latency - 200.0).abs() < 0.001, "Expected ~200.0, got {}", avg_latency);
}

#[test]
fn test_concurrent_metric_recording() {
    use std::thread;

    let collector = Arc::new(MetricsCollector::with_default_retention());
    let mut handles = vec![];

    // Spawn 10 threads, each recording 100 metrics
    for i in 0..10 {
        let collector_clone = Arc::clone(&collector);
        let handle = thread::spawn(move || {
            for _j in 0..100 {
                collector_clone.record_success(&RequestMetrics {
                    api_key_name: &format!("key{}", i),
                    provider: "openai",
                    model: "gpt-4",
                    input_tokens: 1000,
                    output_tokens: 500,
                    cost_usd: 0.05,
                    latency_ms: 100,
                });
            }
        });
        handles.push(handle);
    }

    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }

    let now = Utc::now();
    let data = collector.get_global_range(
        now - Duration::minutes(5),
        now + Duration::minutes(5),
    );

    // Should have exactly 1000 requests (10 threads × 100 requests)
    // Sum across all buckets in case test spans multiple minutes
    let total_requests: u64 = data.iter().map(|p| p.requests).sum();
    assert_eq!(total_requests, 1000, "Expected 1000 total requests across {} bucket(s)", data.len());
}

#[test]
fn test_multi_dataset_generation() {
    let collector = create_test_collector_with_data();
    let now = Utc::now();

    // Get data for multiple API keys
    let key1_data = collector.get_key_range(
        "api_key_1",
        now - Duration::minutes(5),
        now + Duration::minutes(5),
    );
    let key2_data = collector.get_key_range(
        "api_key_2",
        now - Duration::minutes(5),
        now + Duration::minutes(5),
    );

    // Generate multi-dataset graph
    let graph = GraphGenerator::generate_multi(
        vec![
            ("api_key_1", &key1_data[..]),
            ("api_key_2", &key2_data[..]),
        ],
        MetricType::Requests,
    );

    assert_eq!(graph.labels.len(), 1);
    assert_eq!(graph.datasets.len(), 2);
    assert_eq!(graph.datasets[0].label, "api_key_1");
    assert_eq!(graph.datasets[1].label, "api_key_2");
    assert_eq!(graph.datasets[0].data[0] as u64, 4); // key1 has 4 requests
    assert_eq!(graph.datasets[1].data[0] as u64, 2); // key2 has 2 requests
}

#[test]
fn test_empty_data_handling() {
    let collector = Arc::new(MetricsCollector::with_default_retention());
    let now = Utc::now();

    // Query without any data
    let data = collector.get_global_range(
        now - Duration::minutes(5),
        now + Duration::minutes(5),
    );

    assert_eq!(data.len(), 0);

    // Generate graph from empty data
    let graph = GraphGenerator::generate(&data, MetricType::Tokens, Some("Empty"));

    assert_eq!(graph.labels.len(), 0);
    assert_eq!(graph.datasets.len(), 1);
    assert_eq!(graph.datasets[0].data.len(), 0);
}

#[test]
fn test_tracked_entity_lists() {
    let collector = create_test_collector_with_data();

    // Test get_model_names
    let mut models = collector.get_model_names();
    models.sort();
    assert_eq!(
        models,
        vec!["claude-3.5-sonnet", "gpt-3.5-turbo", "gpt-4", "llama-3.3-70b"]
    );

    // Test get_provider_names
    let mut providers = collector.get_provider_names();
    providers.sort();
    assert_eq!(providers, vec!["anthropic", "groq", "openai"]);

    // Test get_api_key_names
    let mut keys = collector.get_api_key_names();
    keys.sort();
    assert_eq!(keys, vec!["api_key_1", "api_key_2"]);
}

/// BUG FIX: Test with ACTUAL time-series data across multiple buckets
/// This tests what the original tests SHOULD have tested!
/// NOW with timestamp injection instead of sleeping!
#[test]
fn test_multi_time_bucket_data_separation() {
    let collector = Arc::new(MetricsCollector::with_default_retention());
    let base_time = Utc::now();

    // Record first batch at T=0
    let time_0 = base_time;
    collector.record_success_at(
        &RequestMetrics {
            api_key_name: "key1",
            provider: "openai",
            model: "gpt-4",
            input_tokens: 1000,
            output_tokens: 500,
            cost_usd: 0.05,
            latency_ms: 100,
        },
        time_0,
    );

    // Record second batch at T+2 minutes (different bucket)
    let time_2min = base_time + Duration::minutes(2);
    collector.record_success_at(
        &RequestMetrics {
            api_key_name: "key1",
            provider: "openai",
            model: "gpt-4",
            input_tokens: 2000,
            output_tokens: 1000,
            cost_usd: 0.10,
            latency_ms: 200,
        },
        time_2min,
    );

    let data = collector.get_global_range(
        base_time - Duration::minutes(1),
        base_time + Duration::minutes(5),
    );

    // CRITICAL: Should have 2 data points, not 1!
    assert_eq!(data.len(), 2, "Data should be in separate minute buckets");

    // Verify first bucket
    assert_eq!(data[0].requests, 1);
    assert_eq!(data[0].input_tokens, 1000);

    // Verify second bucket
    assert_eq!(data[1].requests, 1);
    assert_eq!(data[1].input_tokens, 2000);
}

#[test]
fn test_graph_with_multiple_time_points() {
    let collector = Arc::new(MetricsCollector::with_default_retention());
    let base_time = Utc::now();

    // Record data at T=0
    let time_0 = base_time;
    collector.record_success_at(
        &RequestMetrics {
            api_key_name: "key1",
            provider: "openai",
            model: "gpt-4",
            input_tokens: 1000,
            output_tokens: 500,
            cost_usd: 0.05,
            latency_ms: 100,
        },
        time_0,
    );

    // Record data at T+3 minutes
    let time_3min = base_time + Duration::minutes(3);
    collector.record_success_at(
        &RequestMetrics {
            api_key_name: "key1",
            provider: "openai",
            model: "gpt-4",
            input_tokens: 2000,
            output_tokens: 1000,
            cost_usd: 0.10,
            latency_ms: 200,
        },
        time_3min,
    );

    let data = collector.get_global_range(
        base_time - Duration::minutes(1),
        base_time + Duration::minutes(5),
    );

    // Generate graph
    let graph = GraphGenerator::generate(&data, MetricType::Tokens, Some("Global"));

    // Should have 2 labels (x-axis points)
    assert_eq!(graph.labels.len(), 2, "Graph should have 2 time points");
    assert_eq!(graph.datasets[0].data.len(), 2, "Dataset should have 2 data points");

    // Verify values
    assert_eq!(graph.datasets[0].data[0] as u64, 1500); // 1000 + 500
    assert_eq!(graph.datasets[0].data[1] as u64, 3000); // 2000 + 1000
}

#[test]
fn test_latency_percentiles_graph() {
    let collector = Arc::new(MetricsCollector::with_default_retention());

    // Record multiple requests with varying latencies
    for latency in [50, 100, 150, 200, 250, 300, 350, 400, 450, 500] {
        collector.record_success(&RequestMetrics {
            api_key_name: "key1",
            provider: "openai",
            model: "gpt-4",
            input_tokens: 1000,
            output_tokens: 500,
            cost_usd: 0.05,
            latency_ms: latency,
        });
    }

    let now = Utc::now();
    let data = collector.get_global_range(
        now - Duration::minutes(5),
        now + Duration::minutes(5),
    );

    // Generate percentile graph
    let graph = GraphGenerator::generate_latency_percentiles(&data);

    // Should have 3 datasets: P50, P95, P99
    assert_eq!(graph.datasets.len(), 3);
    assert_eq!(graph.datasets[0].label, "P50");
    assert_eq!(graph.datasets[1].label, "P95");
    assert_eq!(graph.datasets[2].label, "P99");

    // Verify percentiles are calculated
    assert!(graph.datasets[0].data[0] > 0.0, "P50 should be calculated");
    assert!(graph.datasets[1].data[0] > 0.0, "P95 should be calculated");
    assert!(graph.datasets[2].data[0] > 0.0, "P99 should be calculated");
}

#[test]
fn test_token_breakdown_graph() {
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

    let now = Utc::now();
    let data = collector.get_global_range(
        now - Duration::minutes(5),
        now + Duration::minutes(5),
    );

    // Generate token breakdown graph
    let graph = GraphGenerator::generate_token_breakdown(&data);

    // Should have 2 datasets: Input and Output
    assert_eq!(graph.datasets.len(), 2);
    assert_eq!(graph.datasets[0].label, "Input Tokens");
    assert_eq!(graph.datasets[1].label, "Output Tokens");

    // Verify values
    assert_eq!(graph.datasets[0].data[0] as u64, 1000);
    assert_eq!(graph.datasets[1].data[0] as u64, 500);
}

#[test]
#[should_panic(expected = "interval_minutes must be positive")]
fn test_fill_gaps_rejects_zero_interval() {
    use localrouter_ai::monitoring::metrics::MetricDataPoint;

    let now = Utc::now();
    let points = vec![MetricDataPoint {
        timestamp: now,
        requests: 10,
        input_tokens: 100,
        output_tokens: 200,
        total_tokens: 300,
        cost_usd: 0.05,
        total_latency_ms: 1000,
        successful_requests: 10,
        failed_requests: 0,
        latency_samples: vec![100],
    }];

    // This should panic!
    GraphGenerator::fill_gaps(&points, now, now + Duration::minutes(10), 0);
}

#[test]
#[should_panic(expected = "interval_minutes must be positive")]
fn test_fill_gaps_rejects_negative_interval() {
    use localrouter_ai::monitoring::metrics::MetricDataPoint;

    let now = Utc::now();
    let points = vec![MetricDataPoint {
        timestamp: now,
        requests: 10,
        input_tokens: 100,
        output_tokens: 200,
        total_tokens: 300,
        cost_usd: 0.05,
        total_latency_ms: 1000,
        successful_requests: 10,
        failed_requests: 0,
        latency_samples: vec![100],
    }];

    // This should panic!
    GraphGenerator::fill_gaps(&points, now, now + Duration::minutes(10), -1);
}
