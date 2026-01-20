//! Integration tests for metrics storage and aggregation

use chrono::{DateTime, DurationRound, Utc};
use localrouter_ai::monitoring::metrics::{MetricsCollector, RequestMetrics};
use localrouter_ai::monitoring::storage::{Granularity, MetricRow, MetricsDatabase};
use std::sync::Arc;
use tempfile::tempdir;

/// Create a test database
fn create_test_db() -> Arc<MetricsDatabase> {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    Arc::new(MetricsDatabase::new(db_path).unwrap())
}

/// Create a test metrics collector
fn create_test_collector() -> MetricsCollector {
    let db = create_test_db();
    MetricsCollector::new(db)
}

#[test]
fn test_database_persistence() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");

    // Create database and insert data
    {
        let db = MetricsDatabase::new(db_path.clone()).unwrap();
        let now = Utc::now();
        let minute_timestamp = now.duration_trunc(chrono::Duration::minutes(1)).unwrap();

        let row = MetricRow {
            timestamp: minute_timestamp,
            granularity: Granularity::Minute,
            requests: 10,
            successful_requests: 9,
            failed_requests: 1,
            avg_latency_ms: 150.5,
            input_tokens: Some(1000),
            output_tokens: Some(500),
            cost_usd: Some(0.05),
            method_counts: None,
            p50_latency_ms: Some(140.0),
            p95_latency_ms: Some(200.0),
            p99_latency_ms: Some(250.0),
        };

        db.upsert_metric("llm_global", &row).unwrap();
    }

    // Reopen database and verify data persists
    {
        let db = MetricsDatabase::new(db_path).unwrap();
        let now = Utc::now();
        let start = now - chrono::Duration::hours(1);
        let end = now + chrono::Duration::hours(1);

        let results = db.query_metrics("llm_global", start, end).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].requests, 10);
        assert_eq!(results[0].successful_requests, 9);
        assert_eq!(results[0].failed_requests, 1);
    }
}

#[test]
fn test_metrics_collector_persistence() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");

    // Record metrics
    {
        let db = Arc::new(MetricsDatabase::new(db_path.clone()).unwrap());
        let collector = MetricsCollector::new(db);

        collector.record_success(&RequestMetrics {
            api_key_name: "key1",
            provider: "openai",
            model: "gpt-4",
            input_tokens: 100,
            output_tokens: 200,
            cost_usd: 0.05,
            latency_ms: 1000,
        });

        collector.record_success(&RequestMetrics {
            api_key_name: "key1",
            provider: "openai",
            model: "gpt-4",
            input_tokens: 150,
            output_tokens: 250,
            cost_usd: 0.07,
            latency_ms: 1200,
        });
    }

    // Reopen and verify
    {
        let db = Arc::new(MetricsDatabase::new(db_path).unwrap());
        let collector = MetricsCollector::new(db);

        let now = Utc::now();
        let start = now - chrono::Duration::hours(1);
        let end = now + chrono::Duration::hours(1);

        let global = collector.get_global_range(start, end);
        assert_eq!(global.len(), 1);
        assert_eq!(global[0].requests, 2);
        assert_eq!(global[0].input_tokens, 250);
        assert_eq!(global[0].output_tokens, 450);
    }
}

#[test]
fn test_hourly_aggregation() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let db = Arc::new(MetricsDatabase::new(db_path).unwrap());

    let now = Utc::now();
    let hour_start = now.duration_trunc(chrono::Duration::hours(1)).unwrap();

    // Insert minute data for the previous hour
    let prev_hour_start = hour_start - chrono::Duration::hours(1);

    for i in 0..60 {
        let timestamp = prev_hour_start + chrono::Duration::minutes(i);
        let row = MetricRow {
            timestamp,
            granularity: Granularity::Minute,
            requests: 1,
            successful_requests: 1,
            failed_requests: 0,
            avg_latency_ms: 100.0,
            input_tokens: Some(10),
            output_tokens: Some(5),
            cost_usd: Some(0.001),
            method_counts: None,
            p50_latency_ms: Some(100.0),
            p95_latency_ms: Some(150.0),
            p99_latency_ms: Some(200.0),
        };

        db.upsert_metric("llm_global", &row).unwrap();
    }

    // Aggregate to hourly
    let count = db.aggregate_to_hourly(prev_hour_start).unwrap();
    assert_eq!(count, 1); // One metric type aggregated

    // Query hourly data
    let results = db
        .query_metrics(
            "llm_global",
            prev_hour_start - chrono::Duration::days(1),
            prev_hour_start + chrono::Duration::hours(2),
        )
        .unwrap();

    // Find the hourly record
    let hourly = results
        .iter()
        .find(|r| r.granularity == Granularity::Hour)
        .expect("Should have hourly data");

    assert_eq!(hourly.requests, 60); // 60 minutes
    assert_eq!(hourly.successful_requests, 60);
    assert_eq!(hourly.input_tokens, Some(600)); // 60 * 10
    assert_eq!(hourly.output_tokens, Some(300)); // 60 * 5
}

#[test]
fn test_daily_aggregation() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let db = Arc::new(MetricsDatabase::new(db_path).unwrap());

    let now = Utc::now();
    let day_start = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();

    // Insert hourly data for the previous day
    let prev_day_start = day_start - chrono::Duration::days(1);

    for i in 0..24 {
        let timestamp = prev_day_start + chrono::Duration::hours(i);
        let row = MetricRow {
            timestamp,
            granularity: Granularity::Hour,
            requests: 60,
            successful_requests: 58,
            failed_requests: 2,
            avg_latency_ms: 100.0,
            input_tokens: Some(600),
            output_tokens: Some(300),
            cost_usd: Some(0.06),
            method_counts: None,
            p50_latency_ms: Some(100.0),
            p95_latency_ms: Some(150.0),
            p99_latency_ms: Some(200.0),
        };

        db.upsert_metric("llm_global", &row).unwrap();
    }

    // Aggregate to daily
    let count = db.aggregate_to_daily(prev_day_start).unwrap();
    assert_eq!(count, 1); // One metric type aggregated

    // Query daily data
    let results = db
        .query_metrics(
            "llm_global",
            prev_day_start - chrono::Duration::days(7),
            prev_day_start + chrono::Duration::days(2),
        )
        .unwrap();

    // Find the daily record
    let daily = results
        .iter()
        .find(|r| r.granularity == Granularity::Day)
        .expect("Should have daily data");

    assert_eq!(daily.requests, 1440); // 24 hours * 60 requests
    assert_eq!(daily.successful_requests, 1392); // 24 * 58
    assert_eq!(daily.failed_requests, 48); // 24 * 2
    assert_eq!(daily.input_tokens, Some(14400)); // 24 * 600
    assert_eq!(daily.output_tokens, Some(7200)); // 24 * 300
}

#[test]
fn test_cleanup_retention() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let db = Arc::new(MetricsDatabase::new(db_path).unwrap());

    let now = Utc::now();

    // Insert old minute data (> 24 hours old)
    let old_minute = now - chrono::Duration::hours(25);
    let old_row = MetricRow {
        timestamp: old_minute,
        granularity: Granularity::Minute,
        requests: 1,
        successful_requests: 1,
        failed_requests: 0,
        avg_latency_ms: 100.0,
        input_tokens: Some(10),
        output_tokens: Some(5),
        cost_usd: Some(0.001),
        method_counts: None,
        p50_latency_ms: Some(100.0),
        p95_latency_ms: Some(150.0),
        p99_latency_ms: Some(200.0),
    };
    db.upsert_metric("llm_global", &old_row).unwrap();

    // Insert recent minute data (< 24 hours old)
    let recent_minute = (now - chrono::Duration::hours(1))
        .duration_trunc(chrono::Duration::minutes(1))
        .unwrap();
    let recent_row = MetricRow {
        timestamp: recent_minute,
        granularity: Granularity::Minute,
        requests: 1,
        successful_requests: 1,
        failed_requests: 0,
        avg_latency_ms: 100.0,
        input_tokens: Some(10),
        output_tokens: Some(5),
        cost_usd: Some(0.001),
        method_counts: None,
        p50_latency_ms: Some(100.0),
        p95_latency_ms: Some(150.0),
        p99_latency_ms: Some(200.0),
    };
    db.upsert_metric("llm_global", &recent_row).unwrap();

    // Insert old hourly data (> 7 days old)
    let old_hour = now - chrono::Duration::days(8);
    let old_hour_row = MetricRow {
        timestamp: old_hour,
        granularity: Granularity::Hour,
        requests: 60,
        successful_requests: 60,
        failed_requests: 0,
        avg_latency_ms: 100.0,
        input_tokens: Some(600),
        output_tokens: Some(300),
        cost_usd: Some(0.06),
        method_counts: None,
        p50_latency_ms: Some(100.0),
        p95_latency_ms: Some(150.0),
        p99_latency_ms: Some(200.0),
    };
    db.upsert_metric("llm_global", &old_hour_row).unwrap();

    // Insert old daily data (> 90 days old)
    let old_day = now - chrono::Duration::days(91);
    let old_day_row = MetricRow {
        timestamp: old_day,
        granularity: Granularity::Day,
        requests: 1440,
        successful_requests: 1440,
        failed_requests: 0,
        avg_latency_ms: 100.0,
        input_tokens: Some(14400),
        output_tokens: Some(7200),
        cost_usd: Some(1.44),
        method_counts: None,
        p50_latency_ms: Some(100.0),
        p95_latency_ms: Some(150.0),
        p99_latency_ms: Some(200.0),
    };
    db.upsert_metric("llm_global", &old_day_row).unwrap();

    // Run cleanup
    db.cleanup_old_data().unwrap();

    // Verify old data is deleted - query with minute granularity range (< 24 hours)
    let minute_results = db
        .query_metrics(
            "llm_global",
            now - chrono::Duration::hours(12),
            now + chrono::Duration::hours(1),
        )
        .unwrap();

    // Should only have recent minute data
    assert_eq!(
        minute_results.len(),
        1,
        "Should have exactly 1 recent minute record"
    );
    assert_eq!(minute_results[0].granularity, Granularity::Minute);
    assert_eq!(minute_results[0].timestamp, recent_minute);

    // Verify no old data exists in any granularity
    let hourly_results = db
        .query_metrics(
            "llm_global",
            now - chrono::Duration::days(10),
            now - chrono::Duration::days(5),
        )
        .unwrap();
    assert_eq!(hourly_results.len(), 0, "Old hourly data should be deleted");

    let daily_results = db
        .query_metrics(
            "llm_global",
            now - chrono::Duration::days(100),
            now - chrono::Duration::days(50),
        )
        .unwrap();
    assert_eq!(daily_results.len(), 0, "Old daily data should be deleted");
}

#[test]
fn test_granularity_selection() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let db = Arc::new(MetricsDatabase::new(db_path).unwrap());
    let collector = MetricsCollector::new(db);

    let now = Utc::now();

    // Insert data at different timestamps
    let metrics = RequestMetrics {
        api_key_name: "key1",
        provider: "openai",
        model: "gpt-4",
        input_tokens: 100,
        output_tokens: 200,
        cost_usd: 0.05,
        latency_ms: 1000,
    };

    collector.record_success(&metrics);

    // Query last 1 hour (should use minute granularity)
    let minute_range = collector.get_global_range(
        now - chrono::Duration::minutes(30),
        now + chrono::Duration::minutes(30),
    );
    assert!(
        !minute_range.is_empty(),
        "Should have data in minute granularity"
    );

    // The actual granularity selection happens in the database query_metrics method
    // This test verifies that data is returned correctly
}

#[test]
fn test_four_tier_metrics_persistence() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");

    // Record metrics
    {
        let db = Arc::new(MetricsDatabase::new(db_path.clone()).unwrap());
        let collector = MetricsCollector::new(db);

        collector.record_success(&RequestMetrics {
            api_key_name: "key1",
            provider: "openai",
            model: "gpt-4",
            input_tokens: 100,
            output_tokens: 200,
            cost_usd: 0.05,
            latency_ms: 1000,
        });
    }

    // Reopen and verify all four tiers
    {
        let db = Arc::new(MetricsDatabase::new(db_path).unwrap());
        let collector = MetricsCollector::new(db);

        let now = Utc::now();
        let start = now - chrono::Duration::hours(1);
        let end = now + chrono::Duration::hours(1);

        // Global
        let global = collector.get_global_range(start, end);
        assert_eq!(global.len(), 1);
        assert_eq!(global[0].requests, 1);

        // API Key
        let key = collector.get_key_range("key1", start, end);
        assert_eq!(key.len(), 1);
        assert_eq!(key[0].requests, 1);

        // Provider
        let provider = collector.get_provider_range("openai", start, end);
        assert_eq!(provider.len(), 1);
        assert_eq!(provider[0].requests, 1);

        // Model
        let model = collector.get_model_range("gpt-4", start, end);
        assert_eq!(model.len(), 1);
        assert_eq!(model[0].requests, 1);
    }
}

#[test]
fn test_memory_efficiency() {
    // This test verifies that the new implementation doesn't keep data in memory
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let db = Arc::new(MetricsDatabase::new(db_path).unwrap());
    let collector = MetricsCollector::new(db);

    // Record many metrics
    for i in 0..1000 {
        collector.record_success(&RequestMetrics {
            api_key_name: &format!("key{}", i % 10),
            provider: "openai",
            model: "gpt-4",
            input_tokens: 100,
            output_tokens: 200,
            cost_usd: 0.05,
            latency_ms: 1000,
        });
    }

    // All data should be in SQLite, not in-memory
    // We can't directly measure memory, but we can verify the data is retrievable
    let now = Utc::now();
    let global = collector.get_global_range(
        now - chrono::Duration::minutes(30),
        now + chrono::Duration::minutes(30),
    );

    assert!(!global.is_empty(), "Should have data stored");
    assert_eq!(
        global[0].requests, 1000,
        "Should have all 1000 requests aggregated"
    );
}
