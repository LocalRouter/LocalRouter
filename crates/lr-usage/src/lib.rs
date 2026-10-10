//! Subscription and usage-limit tracking.
//!
//! Readings come from three places: rate-limit headers and usage responses
//! on traffic LocalRouter already carries (proxy and gateway), and — when the
//! user allows it — LocalRouter asking providers' usage endpoints. The
//! [`UsageTracker`] merges them per account (provider family × subscription
//! or API) and derives pace, projections and value estimates for the UI.

pub mod bodies;
pub mod classify;
pub mod cli_logins;
pub mod fetch;
pub mod headers;
pub mod plans;
pub mod tracker;
pub mod types;
pub mod view;

use std::sync::{Arc, OnceLock};

use http::HeaderMap;

pub use classify::{
    account_for_provider_type, classify, usage_endpoint, Classified, UsageEndpoint,
};
pub use tracker::{LedgerEntry, PastWindow, UsageTracker};
pub use types::{
    AccountKind, AccountRef, CreditsReading, DataSource, QuotaReading, UsageReport, WindowReading,
};
pub use view::{
    PaceStatus, UsageAccountView, UsageQuotaView, UsageSnapshot, UsageSpendView, UsageWindowView,
};

static GLOBAL: OnceLock<Arc<UsageTracker>> = OnceLock::new();

/// Install the app-wide tracker, read by the provider HTTP clients and the
/// gateway's request finalization. The first install wins.
pub fn install_global(tracker: Arc<UsageTracker>) {
    let _ = GLOBAL.set(tracker);
}

/// The app-wide tracker, once installed.
pub fn global() -> Option<&'static Arc<UsageTracker>> {
    GLOBAL.get()
}

impl UsageTracker {
    /// Observe one upstream response: classify the request, read the
    /// response headers, and return the account (for cost attribution).
    pub fn observe_response(
        &self,
        host: &str,
        path: &str,
        request_headers: &HeaderMap,
        response_headers: &HeaderMap,
        source: DataSource,
    ) -> Option<AccountRef> {
        if !self.is_enabled() {
            return None;
        }
        let classified = classify(host, path, request_headers)?;
        self.note_traffic(&classified.account);
        let mut report = headers::parse_headers(response_headers, tracker::now_secs());
        if report.plan.is_none() {
            report.plan = classified.plan_hint.clone();
        }
        self.apply(&classified.account, &report, source);
        Some(classified.account)
    }

    /// Observe a usage-endpoint response body (e.g. Claude Code's `/usage`
    /// passing through the proxy).
    pub fn observe_usage_body(&self, endpoint: UsageEndpoint, body: &[u8], source: DataSource) {
        if !self.is_enabled() {
            return;
        }
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(body) else {
            return;
        };
        let now = tracker::now_secs();
        let (account, report) = match endpoint {
            UsageEndpoint::ClaudeUsage => (
                AccountRef::subscription("anthropic"),
                bodies::parse_claude_usage(&json),
            ),
            UsageEndpoint::ClaudeProfile => (
                AccountRef::subscription("anthropic"),
                bodies::parse_claude_profile(&json).map(|plan| UsageReport {
                    plan: Some(plan),
                    ..Default::default()
                }),
            ),
            UsageEndpoint::CodexUsage => (
                AccountRef::subscription("openai"),
                bodies::parse_codex_usage(&json, now),
            ),
            UsageEndpoint::CopilotUser => (
                AccountRef::subscription("github-copilot"),
                bodies::parse_copilot_user(&json),
            ),
        };
        if let Some(report) = report {
            self.apply(&account, &report, source);
        }
    }

    /// Observe one in-stream event (SSE `data:` payload or websocket text
    /// frame) from the ChatGPT backend.
    pub fn observe_stream_event(&self, event: &serde_json::Value, source: DataSource) {
        if !self.is_enabled() {
            return;
        }
        if let Some(report) = bodies::parse_codex_rate_limits_event(event, tracker::now_secs()) {
            self.apply(&AccountRef::subscription("openai"), &report, source);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_config::UsageTrackingConfig;

    #[test]
    fn observe_response_classifies_and_records() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let mut req = HeaderMap::new();
        req.insert("authorization", "Bearer sk-ant-oat01-x".parse().unwrap());
        let mut resp = HeaderMap::new();
        resp.insert(
            "anthropic-ratelimit-unified-7d-utilization",
            "0.31".parse().unwrap(),
        );
        resp.insert(
            "anthropic-ratelimit-unified-7d-reset",
            (tracker::now_secs() + 86_400).to_string().parse().unwrap(),
        );
        let account = t
            .observe_response(
                "api.anthropic.com",
                "/v1/messages",
                &req,
                &resp,
                DataSource::ProxyHeaders,
            )
            .unwrap();
        assert_eq!(account, AccountRef::subscription("anthropic"));
        let snap = t.snapshot();
        let w = &snap.accounts[0].windows[0];
        assert_eq!(w.id, "seven_day");
        assert!((w.used_percent - 31.0).abs() < 1e-9);
    }

    #[test]
    fn observe_usage_body_and_stream_event() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        t.observe_usage_body(
            UsageEndpoint::ClaudeUsage,
            br#"{"five_hour":{"utilization":3,"resets_at":"2099-01-01T00:00:00Z"}}"#,
            DataSource::ProxyUsageResponse,
        );
        t.observe_usage_body(
            UsageEndpoint::CodexUsage,
            b"not json",
            DataSource::ProxyUsageResponse,
        );
        t.observe_stream_event(
            &serde_json::json!({"type":"codex.rate_limits","rate_limits":{"secondary":{"used_percent":12,"window_minutes":10080,"reset_at":4102444800i64}}}),
            DataSource::ProxyHeaders,
        );
        let snap = t.snapshot();
        let ids: Vec<_> = snap.accounts.iter().map(|a| a.id.as_str()).collect();
        assert!(ids.contains(&"anthropic:subscription"));
        assert!(ids.contains(&"openai:subscription"));
    }
}
