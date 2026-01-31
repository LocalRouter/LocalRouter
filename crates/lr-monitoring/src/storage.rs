//! SQLite storage for time-aggregated metrics
//!
//! Implements progressive aggregation:
//! - Per-minute: Last 24 hours (1,440 points)
//! - Per-hour: Last 7 days (168 points)
#![allow(dead_code)]
//! - Per-day: Last 90 days (90 points)

use anyhow::Result;
#[allow(unused_imports)]
use chrono::{DateTime, DurationRound, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::Arc;

/// Time granularity for metrics
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    Minute,
    Hour,
    Day,
}

impl Granularity {
    fn as_str(&self) -> &'static str {
        match self {
            Granularity::Minute => "minute",
            Granularity::Hour => "hour",
            Granularity::Day => "day",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "minute" => Some(Granularity::Minute),
            "hour" => Some(Granularity::Hour),
            "day" => Some(Granularity::Day),
            _ => None,
        }
    }
}

/// A single metric row in the database
#[derive(Debug, Clone)]
pub struct MetricRow {
    pub timestamp: DateTime<Utc>,
    pub granularity: Granularity,
    pub requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub avg_latency_ms: f64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub method_counts: Option<String>, // JSON string for MCP method counts
    pub p50_latency_ms: Option<f64>,
    pub p95_latency_ms: Option<f64>,
    pub p99_latency_ms: Option<f64>,
}

/// SQLite database for metrics storage
pub struct MetricsDatabase {
    conn: Arc<Mutex<Connection>>,
}

impl MetricsDatabase {
    /// Create a new metrics database
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.initialize_schema()?;
        Ok(db)
    }

    /// Initialize database schema
    fn initialize_schema(&self) -> Result<()> {
        let conn = self.conn.lock();

        conn.execute(
            "CREATE TABLE IF NOT EXISTS metrics (
                metric_type TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                granularity TEXT NOT NULL,
                requests INTEGER NOT NULL,
                successful_requests INTEGER NOT NULL,
                failed_requests INTEGER NOT NULL,
                avg_latency_ms REAL NOT NULL,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cost_usd REAL,
                method_counts TEXT,
                p50_latency_ms REAL,
                p95_latency_ms REAL,
                p99_latency_ms REAL,
                PRIMARY KEY (metric_type, timestamp, granularity)
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_type_time_gran
             ON metrics(metric_type, timestamp, granularity)",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_timestamp_gran
             ON metrics(timestamp, granularity)",
            [],
        )?;

        Ok(())
    }

    /// Insert or update a metric row
    pub fn upsert_metric(&self, metric_type: &str, row: &MetricRow) -> Result<()> {
        let conn = self.conn.lock();

        conn.execute(
            "INSERT OR REPLACE INTO metrics VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                metric_type,
                row.timestamp.timestamp(),
                row.granularity.as_str(),
                row.requests as i64,
                row.successful_requests as i64,
                row.failed_requests as i64,
                row.avg_latency_ms,
                row.input_tokens.map(|v| v as i64),
                row.output_tokens.map(|v| v as i64),
                row.cost_usd,
                row.method_counts,
                row.p50_latency_ms,
                row.p95_latency_ms,
                row.p99_latency_ms,
            ],
        )?;

        Ok(())
    }

    /// Atomically increment metrics for a success event
    /// Uses SQL ON CONFLICT to avoid race conditions
    pub fn atomic_record_success(
        &self,
        metric_type: &str,
        timestamp: DateTime<Utc>,
        latency_ms: u64,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
    ) -> Result<()> {
        let conn = self.conn.lock();

        conn.execute(
            "INSERT INTO metrics (metric_type, timestamp, granularity, requests, successful_requests,
                failed_requests, avg_latency_ms, input_tokens, output_tokens, cost_usd, method_counts,
                p50_latency_ms, p95_latency_ms, p99_latency_ms)
             VALUES (?1, ?2, 'minute', 1, 1, 0, ?3, ?4, ?5, ?6, NULL, ?3, ?3, ?3)
             ON CONFLICT(metric_type, timestamp, granularity) DO UPDATE SET
                requests = requests + 1,
                successful_requests = successful_requests + 1,
                avg_latency_ms = (avg_latency_ms * requests + ?3) / (requests + 1),
                input_tokens = COALESCE(input_tokens, 0) + ?4,
                output_tokens = COALESCE(output_tokens, 0) + ?5,
                cost_usd = COALESCE(cost_usd, 0.0) + ?6",
            params![
                metric_type,
                timestamp.timestamp(),
                latency_ms as f64,
                input_tokens as i64,
                output_tokens as i64,
                cost_usd,
            ],
        )?;

        Ok(())
    }

    /// Atomically increment metrics for a failure event
    /// Uses SQL ON CONFLICT to avoid race conditions
    pub fn atomic_record_failure(
        &self,
        metric_type: &str,
        timestamp: DateTime<Utc>,
        latency_ms: u64,
    ) -> Result<()> {
        let conn = self.conn.lock();

        conn.execute(
            "INSERT INTO metrics (metric_type, timestamp, granularity, requests, successful_requests,
                failed_requests, avg_latency_ms, input_tokens, output_tokens, cost_usd, method_counts,
                p50_latency_ms, p95_latency_ms, p99_latency_ms)
             VALUES (?1, ?2, 'minute', 1, 0, 1, ?3, NULL, NULL, NULL, NULL, ?3, ?3, ?3)
             ON CONFLICT(metric_type, timestamp, granularity) DO UPDATE SET
                requests = requests + 1,
                failed_requests = failed_requests + 1,
                avg_latency_ms = (avg_latency_ms * requests + ?3) / (requests + 1)",
            params![metric_type, timestamp.timestamp(), latency_ms as f64,],
        )?;

        Ok(())
    }

    /// Query metrics with automatic granularity selection based on time range
    pub fn query_metrics(
        &self,
        metric_type: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<MetricRow>> {
        let range_hours = (end - start).num_hours();

        // Select granularity based on time range
        let granularity = if range_hours <= 24 {
            Granularity::Minute
        } else if range_hours <= 7 * 24 {
            Granularity::Hour
        } else {
            Granularity::Day
        };

        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT timestamp, granularity, requests, successful_requests,
                    failed_requests, avg_latency_ms, input_tokens, output_tokens,
                    cost_usd, method_counts, p50_latency_ms, p95_latency_ms, p99_latency_ms
             FROM metrics
             WHERE metric_type = ?
               AND timestamp >= ?
               AND timestamp <= ?
               AND granularity = ?
             ORDER BY timestamp ASC",
        )?;

        let rows = stmt.query_map(
            params![
                metric_type,
                start.timestamp(),
                end.timestamp(),
                granularity.as_str()
            ],
            |row| {
                let timestamp_i64: i64 = row.get(0)?;
                let granularity_str: String = row.get(1)?;
                let requests_i64: i64 = row.get(2)?;
                let successful_i64: i64 = row.get(3)?;
                let failed_i64: i64 = row.get(4)?;
                let input_tokens_i64: Option<i64> = row.get(6)?;
                let output_tokens_i64: Option<i64> = row.get(7)?;

                Ok(MetricRow {
                    timestamp: DateTime::from_timestamp(timestamp_i64, 0).unwrap_or_else(Utc::now),
                    granularity: Granularity::from_str(&granularity_str)
                        .unwrap_or(Granularity::Minute),
                    requests: requests_i64 as u64,
                    successful_requests: successful_i64 as u64,
                    failed_requests: failed_i64 as u64,
                    avg_latency_ms: row.get(5)?,
                    input_tokens: input_tokens_i64.map(|v| v as u64),
                    output_tokens: output_tokens_i64.map(|v| v as u64),
                    cost_usd: row.get(8)?,
                    method_counts: row.get(9)?,
                    p50_latency_ms: row.get(10)?,
                    p95_latency_ms: row.get(11)?,
                    p99_latency_ms: row.get(12)?,
                })
            },
        )?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }

        Ok(result)
    }

    /// Get all distinct metric types
    pub fn get_metric_types(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT DISTINCT metric_type FROM metrics")?;

        let rows = stmt.query_map([], |row| row.get(0))?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }

        Ok(result)
    }

    /// Get aggregated usage for a metric type within a time window
    /// Returns (total_requests, total_tokens, total_cost)
    /// Uses SQL aggregation for performance (much faster than Rust-side summing)
    pub fn get_aggregated_usage(
        &self,
        metric_type: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<(u64, u64, f64)> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT
                COALESCE(SUM(requests), 0) as total_requests,
                COALESCE(SUM(input_tokens), 0) as total_input_tokens,
                COALESCE(SUM(output_tokens), 0) as total_output_tokens,
                COALESCE(SUM(cost_usd), 0.0) as total_cost
             FROM metrics
             WHERE metric_type = ?
               AND timestamp >= ?
               AND timestamp <= ?",
        )?;

        let result = stmt.query_row(
            params![metric_type, start.timestamp(), end.timestamp()],
            |row| {
                let total_requests: i64 = row.get(0)?;
                let total_input_tokens: i64 = row.get(1)?;
                let total_output_tokens: i64 = row.get(2)?;
                let total_cost: f64 = row.get(3)?;

                Ok((
                    total_requests as u64,
                    (total_input_tokens + total_output_tokens) as u64,
                    total_cost,
                ))
            },
        )?;

        Ok(result)
    }

    /// Cleanup old data based on retention policies
    /// - Minute data: older than 24 hours
    /// - Hour data: older than 7 days
    /// - Day data: older than 90 days
    pub fn cleanup_old_data(&self) -> Result<()> {
        let now = Utc::now();
        let conn = self.conn.lock();

        // Delete minute data older than 24 hours
        let minute_cutoff = now - chrono::Duration::hours(24);
        let minute_deleted = conn.execute(
            "DELETE FROM metrics WHERE granularity = 'minute' AND timestamp < ?",
            params![minute_cutoff.timestamp()],
        )?;

        // Delete hour data older than 7 days
        let hour_cutoff = now - chrono::Duration::days(7);
        let hour_deleted = conn.execute(
            "DELETE FROM metrics WHERE granularity = 'hour' AND timestamp < ?",
            params![hour_cutoff.timestamp()],
        )?;

        // Delete day data older than 90 days
        let day_cutoff = now - chrono::Duration::days(90);
        let day_deleted = conn.execute(
            "DELETE FROM metrics WHERE granularity = 'day' AND timestamp < ?",
            params![day_cutoff.timestamp()],
        )?;

        tracing::debug!(
            "Cleaned up old metrics: {} minutes, {} hours, {} days",
            minute_deleted,
            hour_deleted,
            day_deleted
        );

        // Reclaim space
        conn.execute("VACUUM", [])?;

        Ok(())
    }

    /// Aggregate minute data into hourly data
    /// Should be called hourly by the aggregation task
    pub fn aggregate_to_hourly(&self, hour_start: DateTime<Utc>) -> Result<usize> {
        let conn = self.conn.lock();

        let hour_end = hour_start + chrono::Duration::hours(1);
        let hour_timestamp = hour_start.timestamp();

        // Get all unique metric types that have minute data in this hour
        let mut stmt = conn.prepare(
            "SELECT DISTINCT metric_type FROM metrics
             WHERE granularity = 'minute'
               AND timestamp >= ?
               AND timestamp < ?",
        )?;

        let metric_types: Vec<String> = stmt
            .query_map(
                params![hour_start.timestamp(), hour_end.timestamp()],
                |row| row.get(0),
            )?
            .collect::<Result<Vec<String>, _>>()?;

        let mut aggregated_count = 0;

        for metric_type in metric_types {
            // Aggregate metrics for this type
            let aggregated = conn.execute(
                "INSERT OR REPLACE INTO metrics (metric_type, timestamp, granularity,
                    requests, successful_requests, failed_requests, avg_latency_ms,
                    input_tokens, output_tokens, cost_usd, method_counts,
                    p50_latency_ms, p95_latency_ms, p99_latency_ms)
                 SELECT
                    ? as metric_type,
                    ? as timestamp,
                    'hour' as granularity,
                    SUM(requests),
                    SUM(successful_requests),
                    SUM(failed_requests),
                    AVG(avg_latency_ms),
                    SUM(input_tokens),
                    SUM(output_tokens),
                    SUM(cost_usd),
                    NULL,
                    AVG(p50_latency_ms),
                    AVG(p95_latency_ms),
                    AVG(p99_latency_ms)
                 FROM metrics
                 WHERE metric_type = ?
                   AND granularity = 'minute'
                   AND timestamp >= ?
                   AND timestamp < ?",
                params![
                    metric_type,
                    hour_timestamp,
                    metric_type,
                    hour_start.timestamp(),
                    hour_end.timestamp()
                ],
            )?;

            if aggregated > 0 {
                aggregated_count += 1;
            }
        }

        Ok(aggregated_count)
    }

    /// Aggregate hourly data into daily data
    /// Should be called daily by the aggregation task
    pub fn aggregate_to_daily(&self, day_start: DateTime<Utc>) -> Result<usize> {
        let conn = self.conn.lock();

        let day_end = day_start + chrono::Duration::days(1);
        let day_timestamp = day_start.timestamp();

        // Get all unique metric types that have hour data in this day
        let mut stmt = conn.prepare(
            "SELECT DISTINCT metric_type FROM metrics
             WHERE granularity = 'hour'
               AND timestamp >= ?
               AND timestamp < ?",
        )?;

        let metric_types: Vec<String> = stmt
            .query_map(params![day_start.timestamp(), day_end.timestamp()], |row| {
                row.get(0)
            })?
            .collect::<Result<Vec<String>, _>>()?;

        let mut aggregated_count = 0;

        for metric_type in metric_types {
            // Aggregate metrics for this type
            let aggregated = conn.execute(
                "INSERT OR REPLACE INTO metrics (metric_type, timestamp, granularity,
                    requests, successful_requests, failed_requests, avg_latency_ms,
                    input_tokens, output_tokens, cost_usd, method_counts,
                    p50_latency_ms, p95_latency_ms, p99_latency_ms)
                 SELECT
                    ? as metric_type,
                    ? as timestamp,
                    'day' as granularity,
                    SUM(requests),
                    SUM(successful_requests),
                    SUM(failed_requests),
                    AVG(avg_latency_ms),
                    SUM(input_tokens),
                    SUM(output_tokens),
                    SUM(cost_usd),
                    NULL,
                    AVG(p50_latency_ms),
                    AVG(p95_latency_ms),
                    AVG(p99_latency_ms)
                 FROM metrics
                 WHERE metric_type = ?
                   AND granularity = 'hour'
                   AND timestamp >= ?
                   AND timestamp < ?",
                params![
                    metric_type,
                    day_timestamp,
                    metric_type,
                    day_start.timestamp(),
                    day_end.timestamp()
                ],
            )?;

            if aggregated > 0 {
                aggregated_count += 1;
            }
        }

        Ok(aggregated_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_database_creation() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = MetricsDatabase::new(db_path).unwrap();

        // Verify schema was created
        let conn = db.conn.lock();
        let result: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='metrics'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(result, 1);
    }

    #[test]
    fn test_upsert_and_query() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = MetricsDatabase::new(db_path).unwrap();

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

        let start = minute_timestamp - chrono::Duration::minutes(5);
        let end = minute_timestamp + chrono::Duration::minutes(5);

        let results = db.query_metrics("llm_global", start, end).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].requests, 10);
        assert_eq!(results[0].successful_requests, 9);
        assert_eq!(results[0].failed_requests, 1);
        assert_eq!(results[0].avg_latency_ms, 150.5);
    }

    #[test]
    fn test_cleanup_old_data() {
        let _dir = tempdir().unwrap();
        let db_path = _dir.path().join("test.db");
        let db = MetricsDatabase::new(db_path).unwrap();

        let now = Utc::now();

        // Insert recent minute data (should NOT be deleted)
        let recent_row = MetricRow {
            timestamp: now - chrono::Duration::hours(1),
            granularity: Granularity::Minute,
            requests: 5,
            successful_requests: 5,
            failed_requests: 0,
            avg_latency_ms: 100.0,
            input_tokens: Some(100),
            output_tokens: Some(50),
            cost_usd: Some(0.01),
            method_counts: None,
            p50_latency_ms: Some(100.0),
            p95_latency_ms: Some(150.0),
            p99_latency_ms: Some(200.0),
        };
        db.upsert_metric("llm_global", &recent_row).unwrap();

        // Insert old minute data (should be deleted)
        let old_row = MetricRow {
            timestamp: now - chrono::Duration::hours(25),
            granularity: Granularity::Minute,
            requests: 3,
            successful_requests: 3,
            failed_requests: 0,
            avg_latency_ms: 100.0,
            input_tokens: Some(100),
            output_tokens: Some(50),
            cost_usd: Some(0.01),
            method_counts: None,
            p50_latency_ms: Some(100.0),
            p95_latency_ms: Some(150.0),
            p99_latency_ms: Some(200.0),
        };
        db.upsert_metric("llm_global", &old_row).unwrap();

        // Cleanup
        db.cleanup_old_data().unwrap();

        // Query minute data (use range <=24 hours to select minute granularity)
        let start = now - chrono::Duration::hours(2);
        let end = now;
        let results = db.query_metrics("llm_global", start, end).unwrap();

        // Only recent data should remain (old data from 25 hours ago should be deleted)
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].requests, 5);
    }

    #[test]
    fn test_aggregate_to_hourly() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = MetricsDatabase::new(db_path).unwrap();

        let now = Utc::now();
        let hour_start = now.duration_trunc(chrono::Duration::hours(1)).unwrap();

        // Insert 3 minute data points in the same hour
        for i in 0..3 {
            let row = MetricRow {
                timestamp: hour_start + chrono::Duration::minutes(i * 20),
                granularity: Granularity::Minute,
                requests: 10,
                successful_requests: 9,
                failed_requests: 1,
                avg_latency_ms: 100.0 + (i as f64 * 10.0),
                input_tokens: Some(100),
                output_tokens: Some(50),
                cost_usd: Some(0.01),
                method_counts: None,
                p50_latency_ms: Some(100.0),
                p95_latency_ms: Some(150.0),
                p99_latency_ms: Some(200.0),
            };
            db.upsert_metric("llm_global", &row).unwrap();
        }

        // Aggregate to hourly
        let count = db.aggregate_to_hourly(hour_start).unwrap();
        assert_eq!(count, 1); // Should aggregate 1 metric type

        // Query hourly data
        let results = db
            .query_metrics(
                "llm_global",
                hour_start - chrono::Duration::days(1),
                hour_start + chrono::Duration::days(1),
            )
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].granularity, Granularity::Hour);
        assert_eq!(results[0].requests, 30); // 3 * 10
        assert_eq!(results[0].successful_requests, 27); // 3 * 9
        assert_eq!(results[0].failed_requests, 3); // 3 * 1
    }
}
