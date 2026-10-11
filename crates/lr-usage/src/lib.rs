//! Subscription and usage-limit tracking.
//!
//! Readings come from three places: rate-limit headers and usage responses
//! on traffic LocalRouter already carries (proxy and gateway), and — when the
//! user allows it — LocalRouter asking providers' usage endpoints. The
//! [`UsageTracker`] merges them per account (provider family × subscription
//! or API × login, where logins are told apart) and derives pace,
//! projections and value estimates for the UI.

pub mod bodies;
pub mod classify;
pub mod cli_logins;
pub mod fetch;
pub mod headers;
pub mod plans;
pub mod providers;
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
        let scheme = providers::spec_for(&classified.account.provider)
            .filter(|s| s.kind == classified.account.kind)
            .map_or(providers::HeaderScheme::Generic, |s| s.headers);
        let mut report = headers::parse_headers_for(scheme, response_headers, tracker::now_secs());
        if report.plan.is_none() {
            report.plan = classified.plan_hint.clone();
        }
        report.login_label = classified.login_label.clone();
        self.apply(&classified.account, &report, source);
        Some(classified.account)
    }

    /// Observe a usage-endpoint response body (e.g. Claude Code's `/usage`
    /// passing through the proxy). `account` is what [`observe_response`]
    /// classified the request as; it says which login the body is for.
    ///
    /// [`observe_response`]: Self::observe_response
    pub fn observe_usage_body(
        &self,
        endpoint: UsageEndpoint,
        account: Option<&AccountRef>,
        body: &[u8],
        source: DataSource,
    ) {
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
                chatgpt_account(account),
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
    /// frame) from the ChatGPT backend. `account` is what the request that
    /// opened the stream was classified as.
    pub fn observe_stream_event(
        &self,
        account: Option<&AccountRef>,
        event: &serde_json::Value,
        source: DataSource,
    ) {
        if !self.is_enabled() {
            return;
        }
        if let Some(report) = bodies::parse_codex_rate_limits_event(event, tracker::now_secs()) {
            self.apply(&chatgpt_account(account), &report, source);
        }
    }
}

/// `account` when it is a ChatGPT subscription, else the ChatGPT family's.
fn chatgpt_account(account: Option<&AccountRef>) -> AccountRef {
    account
        .filter(|a| a.provider == "openai" && a.kind == AccountKind::Subscription)
        .cloned()
        .unwrap_or_else(|| AccountRef::chatgpt(None))
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

    /// Groq's headers on LocalRouter's own call land on the Groq API
    /// account as a per-day request limit and a per-minute token limit.
    #[test]
    fn observe_response_reads_provider_specific_headers() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let mut req = HeaderMap::new();
        req.insert("authorization", "Bearer gsk_x".parse().unwrap());
        let mut resp = HeaderMap::new();
        for (k, v) in [
            ("x-ratelimit-limit-requests", "14400"),
            ("x-ratelimit-remaining-requests", "14370"),
            ("x-ratelimit-reset-requests", "2m59.56s"),
            ("x-ratelimit-limit-tokens", "18000"),
            ("x-ratelimit-remaining-tokens", "17997"),
            ("x-ratelimit-reset-tokens", "7.66s"),
        ] {
            resp.insert(k, v.parse().unwrap());
        }
        let account = t
            .observe_response(
                "api.groq.com",
                "/openai/v1/chat/completions",
                &req,
                &resp,
                DataSource::GatewayHeaders,
            )
            .unwrap();
        assert_eq!(account, AccountRef::api("groq"));
        let snap = t.snapshot();
        let labels: Vec<_> = snap.accounts[0]
            .quotas
            .iter()
            .map(|q| q.label.as_str())
            .collect();
        assert_eq!(labels, ["Requests / day", "Tokens / minute"]);
    }

    #[test]
    fn observe_usage_body_and_stream_event() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        t.observe_usage_body(
            UsageEndpoint::ClaudeUsage,
            None,
            br#"{"five_hour":{"utilization":3,"resets_at":"2099-01-01T00:00:00Z"}}"#,
            DataSource::ProxyUsageResponse,
        );
        t.observe_usage_body(
            UsageEndpoint::CodexUsage,
            None,
            b"not json",
            DataSource::ProxyUsageResponse,
        );
        t.observe_stream_event(
            None,
            &serde_json::json!({"type":"codex.rate_limits","rate_limits":{"secondary":{"used_percent":12,"window_minutes":10080,"reset_at":4102444800i64}}}),
            DataSource::ProxyHeaders,
        );
        let snap = t.snapshot();
        let ids: Vec<_> = snap.accounts.iter().map(|a| a.id.as_str()).collect();
        assert!(ids.contains(&"anthropic:subscription"));
        assert!(ids.contains(&"openai:subscription"));
    }

    fn chatgpt_request(account_id: &str, email: Option<&str>) -> HeaderMap {
        let token = classify::chatgpt_jwt(account_id, "prolite", email);
        let mut req = HeaderMap::new();
        req.insert("authorization", format!("Bearer {token}").parse().unwrap());
        req
    }

    fn codex_weekly(used: &str) -> HeaderMap {
        let mut resp = HeaderMap::new();
        resp.insert("x-codex-primary-used-percent", used.parse().unwrap());
        resp.insert("x-codex-primary-window-minutes", "10080".parse().unwrap());
        resp.insert(
            "x-codex-primary-reset-at",
            (tracker::now_secs() + 86_400).to_string().parse().unwrap(),
        );
        resp.insert("x-codex-secondary-used-percent", "0".parse().unwrap());
        resp.insert("x-codex-secondary-window-minutes", "0".parse().unwrap());
        resp.insert("x-codex-secondary-reset-at", "".parse().unwrap());
        resp
    }

    fn weekly_of(t: &UsageTracker, id: &str) -> f64 {
        let snap = t.snapshot();
        let a = snap.accounts.iter().find(|a| a.id == id).unwrap();
        a.windows
            .iter()
            .find(|w| w.id == "seven_day")
            .unwrap()
            .used_percent
    }

    /// The ChatGPT provider (gateway), Codex through the proxy (HTTP and
    /// websocket) and the poller all feed one card per ChatGPT account.
    #[test]
    fn same_chatgpt_account_from_every_source_is_one_account() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let req = chatgpt_request("acc-1", Some("me@example.com"));
        let gateway = t
            .observe_response(
                "chatgpt.com",
                "/backend-api/codex/responses",
                &req,
                &codex_weekly("13"),
                DataSource::GatewayHeaders,
            )
            .unwrap();
        let proxied = t
            .observe_response(
                "chatgpt.com",
                "/backend-api/codex/responses",
                &chatgpt_request("acc-1", None),
                &codex_weekly("14"),
                DataSource::ProxyHeaders,
            )
            .unwrap();
        assert_eq!(gateway, proxied);
        t.observe_stream_event(
            Some(&proxied),
            &serde_json::json!({"type":"codex.rate_limits","rate_limits":{
                "primary":{"used_percent":15,"window_minutes":10080,"reset_at":tracker::now_secs() + 86_400},
                "secondary":null}}),
            DataSource::ProxyHeaders,
        );
        t.record_request(
            &AccountRef::chatgpt(Some("acc-1")),
            LedgerEntry {
                requests: 1,
                ..Default::default()
            },
        );
        let snap = t.snapshot();
        assert_eq!(snap.accounts.len(), 1, "one card: {:?}", snap.accounts);
        let a = &snap.accounts[0];
        assert_eq!(
            a.id, "openai:subscription",
            "the first login keeps the base id"
        );
        assert_eq!(a.title, "ChatGPT Pro Lite");
        assert_eq!(a.login_label.as_deref(), Some("me@example.com"));
        assert_eq!(a.spend.requests_30d, 1);
        assert_eq!(weekly_of(&t, "openai:subscription"), 15.0);
        assert_eq!(a.windows.len(), 1, "the empty secondary adds no window");
    }

    #[test]
    fn two_chatgpt_accounts_are_two_accounts() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        for (account, used) in [("acc-1", "10"), ("acc-2", "70"), ("acc-1", "11")] {
            t.observe_response(
                "chatgpt.com",
                "/backend-api/codex/responses",
                &chatgpt_request(account, None),
                &codex_weekly(used),
                DataSource::ProxyHeaders,
            );
        }
        let second = AccountRef::chatgpt(Some("acc-2"));
        assert_eq!(t.account_id(&second), second.id());
        assert_eq!(t.snapshot().accounts.len(), 2);
        assert_eq!(weekly_of(&t, "openai:subscription"), 11.0);
        assert_eq!(weekly_of(&t, &second.id()), 70.0);
        // A usage body for the second login lands on its account.
        t.observe_usage_body(
            UsageEndpoint::CodexUsage,
            Some(&second),
            br#"{"rate_limit":{"primary_window":{"used_percent":71,"limit_window_seconds":604800}}}"#,
            DataSource::ProxyUsageResponse,
        );
        assert_eq!(weekly_of(&t, &second.id()), 71.0);
        assert_eq!(weekly_of(&t, "openai:subscription"), 11.0);
    }
}
