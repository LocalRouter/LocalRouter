//! Background task for aggregating metrics
//!
//! Runs hourly and daily aggregations to create time-rolled metrics.

use chrono::{DateTime, DurationRound, Timelike, Utc};
use std::sync::Arc;
use tokio::time::{interval, Duration};

use super::storage::MetricsDatabase;

/// Background task for aggregating metrics
pub struct AggregationTask {
    db: Arc<MetricsDatabase>,
}

impl AggregationTask {
    /// Create a new aggregation task
    pub fn new(db: Arc<MetricsDatabase>) -> Self {
        Self { db }
    }

    /// Run the aggregation task loop
    pub async fn run(self) {
        tracing::info!("Starting metrics aggregation task");

        // Run every 10 minutes to check if we need to aggregate
        let mut timer = interval(Duration::from_secs(600)); // 10 minutes

        let mut last_hourly_aggregation: Option<DateTime<Utc>> = None;
        let mut last_daily_aggregation: Option<DateTime<Utc>> = None;

        loop {
            timer.tick().await;

            let now = Utc::now();

            // Check if we should run hourly aggregation
            // Run at the start of each hour (e.g., 12:00, 13:00, etc.)
            if should_run_hourly_aggregation(now, last_hourly_aggregation) {
                self.aggregate_hourly(now).await;
                last_hourly_aggregation = Some(now);
            }

            // Check if we should run daily aggregation
            // Run once per day at midnight
            if should_run_daily_aggregation(now, last_daily_aggregation) {
                self.aggregate_daily(now).await;
                self.cleanup().await;
                last_daily_aggregation = Some(now);
            }
        }
    }

    /// Aggregate the previous hour's minute data into hourly data
    async fn aggregate_hourly(&self, now: DateTime<Utc>) {
        tracing::info!("Starting hourly aggregation");

        // Aggregate the previous hour (not the current hour, as it's still ongoing)
        let hour_start =
            now.duration_trunc(chrono::Duration::hours(1)).unwrap() - chrono::Duration::hours(1);

        match self.db.aggregate_to_hourly(hour_start) {
            Ok(count) => {
                tracing::info!(
                    "Aggregated {} metric types for hour starting at {}",
                    count,
                    hour_start
                );
            }
            Err(e) => {
                tracing::error!("Failed hourly aggregation: {}", e);
            }
        }
    }

    /// Aggregate the previous day's hourly data into daily data
    async fn aggregate_daily(&self, now: DateTime<Utc>) {
        tracing::info!("Starting daily aggregation");

        // Aggregate the previous day (not today, as it's still ongoing)
        let day_start =
            now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc() - chrono::Duration::days(1);

        match self.db.aggregate_to_daily(day_start) {
            Ok(count) => {
                tracing::info!(
                    "Aggregated {} metric types for day starting at {}",
                    count,
                    day_start
                );
            }
            Err(e) => {
                tracing::error!("Failed daily aggregation: {}", e);
            }
        }
    }

    /// Clean up old metrics data
    async fn cleanup(&self) {
        tracing::info!("Cleaning up old metrics");

        match self.db.cleanup_old_data() {
            Ok(()) => {
                tracing::info!("Successfully cleaned up old metrics");
            }
            Err(e) => {
                tracing::error!("Failed cleanup: {}", e);
            }
        }
    }
}

/// Check if hourly aggregation should run
/// Runs once per hour, at the top of the hour
fn should_run_hourly_aggregation(now: DateTime<Utc>, last_run: Option<DateTime<Utc>>) -> bool {
    // If we've never run, check if we're at the start of an hour
    if last_run.is_none() {
        return now.minute() < 10; // Run in the first 10 minutes of the hour
    }

    // If we've run before, check if we're in a new hour
    let last_run = last_run.unwrap();
    let current_hour = now.hour();
    let last_hour = last_run.hour();

    current_hour != last_hour
}

/// Check if daily aggregation should run
/// Runs once per day, around midnight
fn should_run_daily_aggregation(now: DateTime<Utc>, last_run: Option<DateTime<Utc>>) -> bool {
    // If we've never run, check if we're near midnight
    if last_run.is_none() {
        return now.hour() == 0 && now.minute() < 10;
    }

    // If we've run before, check if we're in a new day
    let last_run = last_run.unwrap();
    let current_day = now.date_naive();
    let last_day = last_run.date_naive();

    current_day != last_day && now.hour() == 0
}

/// Spawn the aggregation task in the background
pub fn spawn_aggregation_task(db: Arc<MetricsDatabase>) -> tokio::task::JoinHandle<()> {
    let task = AggregationTask::new(db);
    tokio::spawn(async move {
        task.run().await;
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_run_hourly_aggregation() {
        // First run at the start of an hour
        let now = Utc::now()
            .date_naive()
            .and_hms_opt(14, 5, 0)
            .unwrap()
            .and_utc();
        assert!(should_run_hourly_aggregation(now, None));

        // First run in the middle of an hour should not trigger
        let now = Utc::now()
            .date_naive()
            .and_hms_opt(14, 30, 0)
            .unwrap()
            .and_utc();
        assert!(!should_run_hourly_aggregation(now, None));

        // Second run in same hour should not trigger
        let last_run = Utc::now()
            .date_naive()
            .and_hms_opt(14, 5, 0)
            .unwrap()
            .and_utc();
        let now = Utc::now()
            .date_naive()
            .and_hms_opt(14, 20, 0)
            .unwrap()
            .and_utc();
        assert!(!should_run_hourly_aggregation(now, Some(last_run)));

        // Second run in next hour should trigger
        let last_run = Utc::now()
            .date_naive()
            .and_hms_opt(14, 5, 0)
            .unwrap()
            .and_utc();
        let now = Utc::now()
            .date_naive()
            .and_hms_opt(15, 5, 0)
            .unwrap()
            .and_utc();
        assert!(should_run_hourly_aggregation(now, Some(last_run)));
    }

    #[test]
    fn test_should_run_daily_aggregation() {
        // First run at midnight
        let now = Utc::now()
            .date_naive()
            .and_hms_opt(0, 5, 0)
            .unwrap()
            .and_utc();
        assert!(should_run_daily_aggregation(now, None));

        // First run during the day should not trigger
        let now = Utc::now()
            .date_naive()
            .and_hms_opt(14, 30, 0)
            .unwrap()
            .and_utc();
        assert!(!should_run_daily_aggregation(now, None));

        // Second run on same day should not trigger
        let last_run = Utc::now()
            .date_naive()
            .and_hms_opt(0, 5, 0)
            .unwrap()
            .and_utc();
        let now = last_run + chrono::Duration::hours(12);
        assert!(!should_run_daily_aggregation(now, Some(last_run)));

        // Second run on next day at midnight should trigger
        let last_run = Utc::now()
            .date_naive()
            .and_hms_opt(0, 5, 0)
            .unwrap()
            .and_utc();
        let now = last_run + chrono::Duration::hours(24);
        assert!(should_run_daily_aggregation(now, Some(last_run)));
    }
}
