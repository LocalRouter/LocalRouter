//! Per-client limits on the sampling requests upstream MCP servers send
//! through a client's sessions (`Client::mcp_sampling_max_tokens`,
//! `Client::mcp_sampling_rate_limit`).
//!
//! Applied to every `sampling/createMessage` before it is answered, whatever
//! answers it: passthrough to the client, LocalRouter's LLM routing, or the
//! fallback for clients that cannot sample. A refused request uses no slot.

use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::protocol::JsonRpcError;

/// The rate limit counts requests in this sliding window.
pub(crate) const RATE_WINDOW: Duration = Duration::from_secs(3600);

/// A client's configured sampling limits (`None` = unlimited).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SamplingLimits {
    /// Largest `maxTokens` a sampling request may ask for.
    pub max_tokens: Option<u32>,
    /// Sampling requests allowed per hour.
    pub rate_limit_per_hour: Option<u32>,
}

impl SamplingLimits {
    pub fn from_client(client: &lr_config::Client) -> Self {
        Self {
            max_tokens: client.mcp_sampling_max_tokens,
            rate_limit_per_hour: client.mcp_sampling_rate_limit,
        }
    }
}

/// Reads a client's current limits by client id (live config).
pub type SamplingLimitsSource = Arc<dyn Fn(&str) -> SamplingLimits + Send + Sync>;

/// Admission times per client within the last [`RATE_WINDOW`]. A client's
/// queue never holds more than its limit; clients with no recent request
/// are dropped.
#[derive(Default)]
pub(crate) struct SamplingRateTracker {
    windows: parking_lot::Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl SamplingRateTracker {
    /// Take a slot for `client_id`, or say how long until one frees up.
    fn admit(&self, client_id: &str, limit: u32, now: Instant) -> Result<(), Duration> {
        let mut windows = self.windows.lock();
        windows.retain(|_, times| {
            while times
                .front()
                .is_some_and(|t| now.duration_since(*t) >= RATE_WINDOW)
            {
                times.pop_front();
            }
            !times.is_empty()
        });
        if limit == 0 {
            return Err(RATE_WINDOW);
        }
        let times = windows.entry(client_id.to_string()).or_default();
        if times.len() >= limit as usize {
            let oldest = times[times.len() - limit as usize];
            return Err(RATE_WINDOW.saturating_sub(now.duration_since(oldest)));
        }
        times.push_back(now);
        Ok(())
    }

    #[cfg(test)]
    fn tracked_clients(&self) -> usize {
        self.windows.lock().len()
    }
}

/// Check one sampling request (`params` of `sampling/createMessage`)
/// against the client's limits; `Err` is the error for the upstream.
pub(crate) fn check(
    limits: SamplingLimits,
    tracker: &SamplingRateTracker,
    client_id: &str,
    params: Option<&Value>,
) -> Result<(), JsonRpcError> {
    check_at(limits, tracker, client_id, params, Instant::now())
}

fn check_at(
    limits: SamplingLimits,
    tracker: &SamplingRateTracker,
    client_id: &str,
    params: Option<&Value>,
    now: Instant,
) -> Result<(), JsonRpcError> {
    if let Some(max) = limits.max_tokens {
        let requested = params
            .and_then(|p| p.get("maxTokens"))
            .and_then(Value::as_u64);
        if let Some(requested) = requested.filter(|r| *r > u64::from(max)) {
            return Err(JsonRpcError::custom(
                -32602,
                format!(
                    "Sampling request asks for maxTokens {requested}, above this client's limit of {max}"
                ),
                Some(json!({"maxTokensLimit": max, "requested": requested})),
            ));
        }
    }
    if let Some(limit) = limits.rate_limit_per_hour {
        if let Err(retry) = tracker.admit(client_id, limit, now) {
            return Err(JsonRpcError::custom(
                -32000,
                format!(
                    "Sampling rate limit reached: this client allows {limit} sampling requests per hour"
                ),
                Some(json!({
                    "rateLimitPerHour": limit,
                    "retryAfterSecs": retry.as_secs().max(1),
                })),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(max_tokens: u64) -> Value {
        json!({"messages": [], "maxTokens": max_tokens})
    }

    #[test]
    fn unlimited_admits_everything_and_tracks_nothing() {
        let tracker = SamplingRateTracker::default();
        for _ in 0..100 {
            check(
                SamplingLimits::default(),
                &tracker,
                "c",
                Some(&params(1_000_000)),
            )
            .unwrap();
        }
        assert_eq!(tracker.tracked_clients(), 0);
    }

    #[test]
    fn max_tokens_above_the_limit_is_refused_without_a_slot() {
        let tracker = SamplingRateTracker::default();
        let limits = SamplingLimits {
            max_tokens: Some(100),
            rate_limit_per_hour: Some(1),
        };
        let err = check(limits, &tracker, "c", Some(&params(101))).unwrap_err();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("limit of 100"), "{}", err.message);
        assert_eq!(tracker.tracked_clients(), 0, "refused request used no slot");
        // At the limit is fine, and takes the one slot
        check(limits, &tracker, "c", Some(&params(100))).unwrap();
        // A request without maxTokens is not refused for its size
        let err = check(limits, &tracker, "c", Some(&json!({"messages": []}))).unwrap_err();
        assert_eq!(err.code, -32000, "but the slot is gone");
    }

    #[test]
    fn rate_limit_slides_per_client() {
        let tracker = SamplingRateTracker::default();
        let limits = SamplingLimits {
            max_tokens: None,
            rate_limit_per_hour: Some(2),
        };
        let t0 = Instant::now();
        check_at(limits, &tracker, "a", None, t0).unwrap();
        check_at(limits, &tracker, "a", None, t0 + Duration::from_secs(600)).unwrap();
        let err =
            check_at(limits, &tracker, "a", None, t0 + Duration::from_secs(1200)).unwrap_err();
        assert_eq!(err.code, -32000);
        let data = err.data.unwrap();
        assert_eq!(data["rateLimitPerHour"], 2);
        assert_eq!(
            data["retryAfterSecs"], 2400,
            "the first slot frees at t0 + 1h"
        );
        // Another client has its own window
        check_at(limits, &tracker, "b", None, t0 + Duration::from_secs(1200)).unwrap();
        // The first slot frees after an hour
        check_at(limits, &tracker, "a", None, t0 + RATE_WINDOW).unwrap();
        // Idle clients are forgotten
        check_at(
            limits,
            &tracker,
            "c",
            None,
            t0 + Duration::from_secs(10_000),
        )
        .unwrap();
        assert_eq!(tracker.tracked_clients(), 1);
    }

    #[test]
    fn a_lowered_limit_applies_to_the_recent_window() {
        let tracker = SamplingRateTracker::default();
        let t0 = Instant::now();
        let five = SamplingLimits {
            max_tokens: None,
            rate_limit_per_hour: Some(5),
        };
        for i in 0..3 {
            check_at(five, &tracker, "a", None, t0 + Duration::from_secs(i)).unwrap();
        }
        let one = SamplingLimits {
            max_tokens: None,
            rate_limit_per_hour: Some(1),
        };
        let err = check_at(one, &tracker, "a", None, t0 + Duration::from_secs(10)).unwrap_err();
        // The newest admission must leave the window before one more fits
        assert_eq!(err.data.unwrap()["retryAfterSecs"], 3600 - 8);
        let zero = SamplingLimits {
            max_tokens: None,
            rate_limit_per_hour: Some(0),
        };
        assert!(check_at(zero, &tracker, "z", None, t0).is_err());
    }
}
