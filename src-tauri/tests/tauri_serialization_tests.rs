//! Tests for Tauri command serialization
//!
//! Verifies that Rust types serialize correctly for JavaScript/TypeScript consumption.

use localrouter::monitoring::graphs::{MetricType, TimeRange};
use serde_json::json;

#[test]
fn test_time_range_serialization() {
    // Verify TimeRange serializes to lowercase strings (as expected by JS)
    assert_eq!(
        serde_json::to_value(TimeRange::Hour).unwrap(),
        json!("hour")
    );
    assert_eq!(serde_json::to_value(TimeRange::Day).unwrap(), json!("day"));
    assert_eq!(
        serde_json::to_value(TimeRange::Week).unwrap(),
        json!("week")
    );
    assert_eq!(
        serde_json::to_value(TimeRange::Month).unwrap(),
        json!("month")
    );
}

#[test]
fn test_time_range_deserialization() {
    // Verify JS values deserialize correctly
    let hour: TimeRange = serde_json::from_value(json!("hour")).unwrap();
    assert_eq!(hour, TimeRange::Hour);

    let day: TimeRange = serde_json::from_value(json!("day")).unwrap();
    assert_eq!(day, TimeRange::Day);

    let week: TimeRange = serde_json::from_value(json!("week")).unwrap();
    assert_eq!(week, TimeRange::Week);

    let month: TimeRange = serde_json::from_value(json!("month")).unwrap();
    assert_eq!(month, TimeRange::Month);
}

#[test]
fn test_metric_type_serialization() {
    // Verify MetricType serializes to lowercase strings
    assert_eq!(
        serde_json::to_value(MetricType::Tokens).unwrap(),
        json!("tokens")
    );
    assert_eq!(
        serde_json::to_value(MetricType::Cost).unwrap(),
        json!("cost")
    );
    assert_eq!(
        serde_json::to_value(MetricType::Requests).unwrap(),
        json!("requests")
    );
    assert_eq!(
        serde_json::to_value(MetricType::Latency).unwrap(),
        json!("latency")
    );
    assert_eq!(
        serde_json::to_value(MetricType::SuccessRate).unwrap(),
        json!("successrate")
    );
}

#[test]
fn test_metric_type_deserialization() {
    // Verify JS values deserialize correctly
    let tokens: MetricType = serde_json::from_value(json!("tokens")).unwrap();
    assert_eq!(tokens, MetricType::Tokens);

    let cost: MetricType = serde_json::from_value(json!("cost")).unwrap();
    assert_eq!(cost, MetricType::Cost);

    let requests: MetricType = serde_json::from_value(json!("requests")).unwrap();
    assert_eq!(requests, MetricType::Requests);

    let latency: MetricType = serde_json::from_value(json!("latency")).unwrap();
    assert_eq!(latency, MetricType::Latency);

    let success_rate: MetricType = serde_json::from_value(json!("successrate")).unwrap();
    assert_eq!(success_rate, MetricType::SuccessRate);
}

#[test]
fn test_tauri_command_args_format() {
    // Simulate what JavaScript sends
    let js_args = json!({
        "timeRange": "day",
        "metricType": "tokens"
    });

    // Verify these deserialize correctly
    let time_range: TimeRange = serde_json::from_value(js_args["timeRange"].clone()).unwrap();
    let metric_type: MetricType = serde_json::from_value(js_args["metricType"].clone()).unwrap();

    assert_eq!(time_range, TimeRange::Day);
    assert_eq!(metric_type, MetricType::Tokens);
}

#[test]
fn test_wrong_case_fails() {
    // Verify that wrong casing (snake_case from JS) fails to deserialize
    let result: Result<TimeRange, _> = serde_json::from_value(json!("time_range"));
    assert!(result.is_err(), "Should reject snake_case variant");

    let result: Result<MetricType, _> = serde_json::from_value(json!("success_rate"));
    assert!(result.is_err(), "Should reject snake_case variant");
}
