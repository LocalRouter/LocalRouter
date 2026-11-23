//! A strategy's rate limits: whether one more request fits, and when one
//! will if it does not. Shared by the router and the app's proxies, so a
//! strategy is limited (and its `Retry-After` computed) the same way
//! whichever way a client reaches it.
//!
//! A strategy's completed usage is its `llm_strategy:<id>` metrics rows; the
//! requests it has admitted but not finished are held as in-flight
//! reservations ([`crate::rate_limit::RateLimiterManager::strategy_in_flight`]).
//! A new request is projected at one request plus the strategy's average
//! tokens and cost.

use lr_config::{RateLimitTimeWindow, RateLimitType, Strategy};
use lr_monitoring::metrics::MetricsCollector;
use lr_types::AppError;

use crate::rate_limit::ReservedUsage;

/// A strategy limit that refused a request.
#[derive(Debug, Clone, PartialEq)]
pub struct StrategyLimitDenial {
    /// What is limited: `requests`, `tokens` or `cost`.
    pub limit: &'static str,
    pub window: RateLimitTimeWindow,
    /// When the request would fit, as far as recorded usage leaving the
    /// window decides it (see [`MetricsCollector::strategy_retry_after`]):
    /// requests still in flight are counted as usage recorded now. At
    /// least 1.
    pub retry_after_secs: u64,
}

impl StrategyLimitDenial {
    /// `minute`, `hour` or `day`.
    pub fn window_name(&self) -> &'static str {
        match self.window {
            RateLimitTimeWindow::Minute => "minute",
            RateLimitTimeWindow::Hour => "hour",
            RateLimitTimeWindow::Day => "day",
        }
    }

    /// The router's error for this refusal (a 429 with `Retry-After`).
    pub fn into_error(self) -> AppError {
        AppError::StrategyRateLimited {
            limit: self.limit.to_owned(),
            window: self.window_name().to_owned(),
            retry_after_secs: self.retry_after_secs,
        }
    }
}

/// Whether one more request fits every enabled limit of `strategy`, given
/// recorded metrics, the requests in flight, and the averages a new request
/// is projected at. A refusal names the limit that frees last: the request
/// fits only once every limit has room.
pub fn check(
    metrics: &MetricsCollector,
    strategy: &Strategy,
    avg_tokens: f64,
    avg_cost: f64,
    in_flight: &ReservedUsage,
) -> Result<(), StrategyLimitDenial> {
    let in_flight_requests = in_flight.requests as f64;
    let in_flight_tokens = in_flight
        .input_tokens
        .saturating_add(in_flight.output_tokens) as f64;
    let in_flight_cost = in_flight.cost_usd.unwrap_or(0.0);
    let mut denial: Option<StrategyLimitDenial> = None;
    for limit in &strategy.rate_limits {
        if !limit.enabled {
            continue;
        }
        // The projected usage, with `recorded` = (requests, tokens, cost)
        // recorded in the window.
        let (label, projected): (&'static str, Box<dyn Fn(u64, u64, f64) -> f64>) =
            match limit.limit_type {
                RateLimitType::Requests => (
                    "requests",
                    Box::new(move |requests, _, _| requests as f64 + in_flight_requests + 1.0),
                ),
                RateLimitType::TotalTokens => (
                    "tokens",
                    Box::new(move |_, tokens, _| tokens as f64 + in_flight_tokens + avg_tokens),
                ),
                // Free models (an average cost of 0) do not count against a
                // cost limit.
                RateLimitType::Cost if avg_cost == 0.0 => continue,
                RateLimitType::Cost => (
                    "cost",
                    Box::new(move |_, _, cost| cost + in_flight_cost + avg_cost),
                ),
                // Input/output token limits are not checked before a request.
                _ => continue,
            };
        let window_secs = limit.time_window.to_seconds();
        let (requests, tokens, cost) =
            metrics.get_recent_usage_for_strategy(&strategy.id, window_secs);
        let usage = projected(requests, tokens, cost);
        if usage <= limit.value {
            continue;
        }
        tracing::warn!(
            "Strategy rate limit exceeded: {label} per {:?} (projected: {usage:.2}, limit: {:.2})",
            limit.time_window,
            limit.value
        );
        let retry_after_secs =
            metrics.strategy_retry_after(&strategy.id, window_secs, |r, t, c| {
                projected(r, t, c) <= limit.value
            });
        if denial
            .as_ref()
            .is_none_or(|d| retry_after_secs > d.retry_after_secs)
        {
            denial = Some(StrategyLimitDenial {
                limit: label,
                window: limit.time_window,
                retry_after_secs,
            });
        }
    }
    denial.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_monitoring::metrics::RequestMetrics;
    use std::sync::Arc;

    fn collector(dir: &std::path::Path) -> MetricsCollector {
        MetricsCollector::new(Arc::new(
            lr_monitoring::storage::MetricsDatabase::new(dir.join("metrics.db")).unwrap(),
        ))
    }

    fn strategy(limits: &[(RateLimitType, f64, RateLimitTimeWindow)]) -> Strategy {
        let mut strategy = Strategy::new("limited".into());
        strategy.rate_limits = limits
            .iter()
            .map(
                |(limit_type, value, time_window)| lr_config::StrategyRateLimit {
                    limit_type: *limit_type,
                    value: *value,
                    time_window: *time_window,
                    enabled: true,
                },
            )
            .collect();
        strategy
    }

    fn record(metrics: &MetricsCollector, strategy: &Strategy, ago_secs: i64) {
        metrics.record_success_at(
            &RequestMetrics {
                api_key_name: "client",
                provider: "p",
                model: "m",
                strategy_id: &strategy.id,
                input_tokens: 100,
                output_tokens: 0,
                cost_usd: 0.0,
                latency_ms: 1,
            },
            chrono::Utc::now() - chrono::Duration::seconds(ago_secs),
        );
    }

    #[test]
    fn a_request_within_every_limit_fits() {
        let dir = tempfile::tempdir().unwrap();
        let metrics = collector(dir.path());
        let strategy = strategy(&[(RateLimitType::Requests, 2.0, RateLimitTimeWindow::Day)]);
        record(&metrics, &strategy, 10);
        assert_eq!(
            check(&metrics, &strategy, 100.0, 0.0, &ReservedUsage::default()),
            Ok(())
        );
        // An in-flight request takes the last place.
        let in_flight = ReservedUsage {
            requests: 1,
            ..Default::default()
        };
        assert!(check(&metrics, &strategy, 100.0, 0.0, &in_flight).is_err());
    }

    /// A daily request limit filled a few hours ago answers with hours,
    /// not a fixed minute; the limit that frees last decides.
    #[test]
    fn a_daily_limit_answers_when_its_oldest_request_leaves_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let metrics = collector(dir.path());
        let strategy = strategy(&[
            (RateLimitType::Requests, 2.0, RateLimitTimeWindow::Day),
            (RateLimitType::Requests, 5.0, RateLimitTimeWindow::Minute),
        ]);
        record(&metrics, &strategy, 3 * 3_600);
        record(&metrics, &strategy, 2 * 3_600);
        let denial = check(&metrics, &strategy, 100.0, 0.0, &ReservedUsage::default())
            .expect_err("the daily limit is full");
        assert_eq!(denial.limit, "requests");
        assert_eq!(denial.window, RateLimitTimeWindow::Day);
        // The older request leaves the window 21 hours from now (to the
        // minute it was recorded in, plus that minute).
        let expected = 21 * 3_600;
        assert!(
            (expected - 60..=expected + 60).contains(&(denial.retry_after_secs as i64)),
            "{}",
            denial.retry_after_secs
        );
        assert!(matches!(
            denial.clone().into_error(),
            AppError::StrategyRateLimited { ref limit, ref window, retry_after_secs }
                if limit == "requests" && window == "day" && retry_after_secs == denial.retry_after_secs
        ));
    }

    /// Requests in flight become recorded usage that stays a whole window:
    /// when they alone fill a limit, the answer is a whole window.
    #[test]
    fn in_flight_requests_free_a_whole_window_from_now() {
        let dir = tempfile::tempdir().unwrap();
        let metrics = collector(dir.path());
        let strategy = strategy(&[(RateLimitType::Requests, 1.0, RateLimitTimeWindow::Hour)]);
        let in_flight = ReservedUsage {
            requests: 1,
            ..Default::default()
        };
        let denial = check(&metrics, &strategy, 100.0, 0.0, &in_flight).unwrap_err();
        assert!(
            (3_600..=3_660).contains(&denial.retry_after_secs),
            "{}",
            denial.retry_after_secs
        );
    }

    #[test]
    fn token_limits_project_the_average_and_free_cost_is_not_limited() {
        let dir = tempfile::tempdir().unwrap();
        let metrics = collector(dir.path());
        let strategy = strategy(&[
            (
                RateLimitType::TotalTokens,
                250.0,
                RateLimitTimeWindow::Minute,
            ),
            (RateLimitType::Cost, 0.0, RateLimitTimeWindow::Minute),
        ]);
        record(&metrics, &strategy, 0);
        // 100 recorded + 100 projected fits; 200 projected does not.
        assert_eq!(
            check(&metrics, &strategy, 100.0, 0.0, &ReservedUsage::default()),
            Ok(())
        );
        let denial = check(&metrics, &strategy, 200.0, 0.0, &ReservedUsage::default()).unwrap_err();
        assert_eq!(denial.limit, "tokens");
        assert!((1..=120).contains(&denial.retry_after_secs));
        // A disabled limit never refuses.
        let mut disabled = strategy.clone();
        disabled.rate_limits[0].enabled = false;
        assert_eq!(
            check(&metrics, &disabled, 200.0, 0.0, &ReservedUsage::default()),
            Ok(())
        );
    }
}
