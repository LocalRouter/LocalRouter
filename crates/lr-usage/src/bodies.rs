//! Usage endpoint response bodies.
//!
//! - Claude: `GET api.anthropic.com/api/oauth/usage` (what Claude Code's
//!   `/usage` shows) and `/api/oauth/profile` (plan tier).
//! - ChatGPT: `GET chatgpt.com/backend-api/wham/usage` (what Codex's
//!   `/status` shows) and the in-stream `codex.rate_limits` event.
//! - GitHub Copilot: `GET api.github.com/copilot_internal/user`.

use serde_json::Value;

use crate::headers::{codex_window, parse_epoch};
use crate::types::{
    id_fragment, window_label, window_secs_for, CreditsReading, QuotaReading, UsageReport,
    WindowReading,
};

/// Top-level `seven_day_*` buckets that share the main weekly limit and so
/// would only duplicate it.
const SHARED_CLAUDE_BUCKETS: &[&str] = &["seven_day_design"];

fn f64_of(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn time_of(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_f64().and_then(|f| parse_epoch(&f.to_string())),
        Value::String(s) => parse_epoch(s),
        _ => None,
    }
}

/// Claude `/api/oauth/usage`.
pub fn parse_claude_usage(body: &Value) -> Option<UsageReport> {
    let obj = body.as_object()?;
    let mut report = UsageReport::default();
    for (key, value) in obj {
        let is_window = key == "five_hour"
            || key == "seven_day"
            || key.starts_with("seven_day_")
            || key.starts_with("five_hour_");
        if !is_window || SHARED_CLAUDE_BUCKETS.contains(&key.as_str()) {
            continue;
        }
        let Some(used) = f64_of(value.get("utilization")) else {
            continue;
        };
        report.windows.push(WindowReading {
            id: key.clone(),
            label: window_label(key),
            used_percent: used,
            resets_at: time_of(value.get("resets_at")),
            window_secs: window_secs_for(key),
        });
    }
    // Model-scoped weekly caps now arrive only in `limits[]`.
    for limit in body
        .get("limits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let kind = limit.get("kind").and_then(Value::as_str).unwrap_or("");
        let id = match kind {
            "session" => "five_hour".to_string(),
            "weekly_all" => "seven_day".to_string(),
            "weekly_scoped" => {
                let name = limit
                    .pointer("/scope/model/display_name")
                    .and_then(Value::as_str)
                    .or_else(|| limit.pointer("/scope/model/id").and_then(Value::as_str));
                match name {
                    Some(n) => format!("seven_day_{}", id_fragment(n)),
                    None => continue,
                }
            }
            _ => continue,
        };
        if report.windows.iter().any(|w| w.id == id) {
            continue;
        }
        let Some(used) = f64_of(limit.get("percent")) else {
            continue;
        };
        report.windows.push(WindowReading {
            label: window_label(&id),
            window_secs: window_secs_for(&id),
            id,
            used_percent: used,
            resets_at: time_of(limit.get("resets_at")),
        });
    }
    if let Some(extra) = body.get("extra_usage").filter(|e| {
        e.get("is_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }) {
        // Amounts are in cents.
        report.credits = Some(CreditsReading {
            label: "Extra usage".to_string(),
            limit_usd: f64_of(extra.get("monthly_limit")).map(|c| c / 100.0),
            used_usd: f64_of(extra.get("used_credits")).map(|c| c / 100.0),
            balance_usd: None,
            unlimited: false,
            currency: extra
                .get("currency")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| Some("USD".to_string())),
        });
    }
    (!report.is_empty()).then_some(report)
}

/// Claude `/api/oauth/profile`: only the plan tier.
pub fn parse_claude_profile(body: &Value) -> Option<String> {
    if let Some(tier) = body
        .pointer("/organization/rate_limit_tier")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Some(tier.to_string());
    }
    let account = body.get("account")?;
    if account.get("has_claude_max").and_then(Value::as_bool) == Some(true) {
        return Some("claude_max".to_string());
    }
    if account.get("has_claude_pro").and_then(Value::as_bool) == Some(true) {
        return Some("claude_pro".to_string());
    }
    None
}

fn codex_window_reading(family: &str, slot: &str, w: &Value, now: i64) -> Option<WindowReading> {
    let used = f64_of(w.get("used_percent"))?;
    let secs = f64_of(w.get("limit_window_seconds"))
        .map(|s| s as i64)
        .or_else(|| f64_of(w.get("window_minutes")).map(|m| (m * 60.0) as i64));
    let resets_at = time_of(w.get("reset_at")).or_else(|| {
        f64_of(w.get("reset_after_seconds"))
            .or_else(|| f64_of(w.get("resets_in_seconds")))
            .map(|s| now + s as i64)
    });
    let (id, label) = codex_window(family, slot, secs);
    Some(WindowReading {
        id,
        label,
        used_percent: used,
        resets_at,
        window_secs: secs,
    })
}

fn codex_credits(c: &Value) -> Option<CreditsReading> {
    let has = c
        .get("has_credits")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let unlimited = c.get("unlimited").and_then(Value::as_bool).unwrap_or(false);
    let balance = f64_of(c.get("balance"));
    (has || unlimited || balance.is_some_and(|b| b > 0.0)).then(|| CreditsReading {
        label: "Credits".to_string(),
        balance_usd: balance,
        unlimited,
        ..Default::default()
    })
}

/// ChatGPT `/backend-api/wham/usage`.
pub fn parse_codex_usage(body: &Value, now: i64) -> Option<UsageReport> {
    let mut report = UsageReport {
        plan: body
            .get("plan_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        ..Default::default()
    };
    let push_limits = |family: &str, rl: &Value, report: &mut UsageReport| {
        for (slot, key) in [
            ("primary", "primary_window"),
            ("secondary", "secondary_window"),
        ] {
            if let Some(w) = rl.get(key).filter(|w| w.is_object()) {
                if let Some(r) = codex_window_reading(family, slot, w, now) {
                    report.windows.push(r);
                }
            }
        }
    };
    if let Some(rl) = body.get("rate_limit") {
        push_limits("codex", rl, &mut report);
        let reached = rl.get("limit_reached").and_then(Value::as_bool) == Some(true)
            || rl.get("allowed").and_then(Value::as_bool) == Some(false);
        report.status = Some(if reached { "rejected" } else { "allowed" }.to_string());
    }
    for extra in body
        .get("additional_rate_limits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let family = extra
            .get("limit_name")
            .or_else(|| extra.get("metered_feature"))
            .and_then(Value::as_str)
            .unwrap_or("other");
        if let Some(rl) = extra.get("rate_limit") {
            push_limits(family, rl, &mut report);
        }
    }
    report.credits = body.get("credits").and_then(codex_credits);
    (!report.is_empty()).then_some(report)
}

/// The `codex.rate_limits` event Codex receives in-stream (SSE / websocket).
pub fn parse_codex_rate_limits_event(event: &Value, now: i64) -> Option<UsageReport> {
    if event.get("type").and_then(Value::as_str) != Some("codex.rate_limits") {
        return None;
    }
    let family = event
        .get("metered_limit_name")
        .and_then(Value::as_str)
        .unwrap_or("codex");
    let mut report = UsageReport {
        plan: event
            .get("plan_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        ..Default::default()
    };
    if let Some(rl) = event.get("rate_limits") {
        for slot in ["primary", "secondary"] {
            if let Some(w) = rl.get(slot).filter(|w| w.is_object()) {
                if let Some(r) = codex_window_reading(family, slot, w, now) {
                    report.windows.push(r);
                }
            }
        }
    }
    report.credits = event.get("credits").and_then(codex_credits);
    (!report.is_empty()).then_some(report)
}

/// GitHub Copilot `copilot_internal/user`.
pub fn parse_copilot_user(body: &Value) -> Option<UsageReport> {
    let mut report = UsageReport {
        plan: body
            .get("copilot_plan")
            .or_else(|| body.get("access_type_sku"))
            .and_then(Value::as_str)
            .map(str::to_string),
        ..Default::default()
    };
    let resets_at = body
        .get("quota_reset_date")
        .and_then(Value::as_str)
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|dt| dt.and_utc().timestamp());
    for (key, id, label) in [
        ("premium_interactions", "monthly", "Premium requests"),
        ("chat", "monthly_chat", "Chat"),
        ("completions", "monthly_completions", "Completions"),
    ] {
        let Some(snap) = body.pointer(&format!("/quota_snapshots/{key}")) else {
            continue;
        };
        if snap.get("unlimited").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let entitlement = f64_of(snap.get("entitlement"));
        let remaining = f64_of(snap.get("remaining"));
        let used = f64_of(snap.get("percent_remaining"))
            .map(|p| 100.0 - p)
            .or_else(|| match (entitlement, remaining) {
                (Some(e), Some(r)) if e > 0.0 => Some((e - r) / e * 100.0),
                _ => None,
            });
        if entitlement.is_some_and(|e| e <= 0.0) {
            continue;
        }
        if let Some(used) = used {
            report.windows.push(WindowReading {
                id: id.to_string(),
                label: label.to_string(),
                used_percent: used.max(0.0),
                resets_at,
                window_secs: Some(30 * 86_400),
            });
        }
        if entitlement.is_some() || remaining.is_some() {
            report.quotas.push(QuotaReading {
                id: id_fragment(key),
                limit: entitlement,
                remaining,
                resets_at,
            });
        }
    }
    (!report.is_empty()).then_some(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_usage_windows_scoped_limits_and_extra_usage() {
        let body = json!({
            "five_hour":  {"utilization": 11.0, "resets_at": "2026-07-03T00:30:00.282668+00:00"},
            "seven_day":  {"utilization": 9.0,  "resets_at": "2026-07-08T09:00:00.282694+00:00"},
            "seven_day_opus": null,
            "seven_day_design": {"utilization": 9.0, "resets_at": "2026-07-08T09:00:00+00:00"},
            "seven_day_routines": {"utilization": 18, "resets_at": "2026-07-08T09:00:00+00:00"},
            "extra_usage": {"is_enabled": true, "monthly_limit": 2050, "used_credits": 325, "utilization": 15.8, "currency": "USD"},
            "limits": [
                {"kind":"session","percent":11,"resets_at":"2026-07-03T00:30:00+00:00"},
                {"kind":"weekly_scoped","percent":5,"resets_at":"2026-07-08T09:00:00+00:00",
                 "scope":{"model":{"id":null,"display_name":"Fable"}}}
            ]
        });
        let r = parse_claude_usage(&body).unwrap();
        let ids: Vec<_> = r.windows.iter().map(|w| w.id.as_str()).collect();
        assert!(ids.contains(&"five_hour"));
        assert!(ids.contains(&"seven_day"));
        assert!(ids.contains(&"seven_day_routines"));
        assert!(ids.contains(&"seven_day_fable"));
        assert!(!ids.contains(&"seven_day_design"));
        assert!(!ids.contains(&"seven_day_opus"));
        assert_eq!(ids.iter().filter(|i| **i == "five_hour").count(), 1);
        let five = r.windows.iter().find(|w| w.id == "five_hour").unwrap();
        assert_eq!(five.window_secs, Some(18_000));
        assert!(five.resets_at.is_some());
        let credits = r.credits.unwrap();
        assert_eq!(credits.limit_usd, Some(20.5));
        assert_eq!(credits.used_usd, Some(3.25));
    }

    #[test]
    fn claude_usage_rejects_error_bodies() {
        assert!(parse_claude_usage(&json!({"type": "error", "error": {}})).is_none());
        assert!(parse_claude_usage(&json!([1, 2])).is_none());
    }

    #[test]
    fn claude_profile_plan() {
        let p = json!({"organization": {"rate_limit_tier": "default_claude_max_20x"}});
        assert_eq!(
            parse_claude_profile(&p).as_deref(),
            Some("default_claude_max_20x")
        );
        let p = json!({"account": {"has_claude_pro": true}, "organization": {}});
        assert_eq!(parse_claude_profile(&p).as_deref(), Some("claude_pro"));
        assert_eq!(parse_claude_profile(&json!({})), None);
    }

    #[test]
    fn codex_usage() {
        let body = json!({
            "plan_type": "pro",
            "rate_limit": {"allowed": true, "limit_reached": false,
              "primary_window":   {"used_percent": 42, "limit_window_seconds": 18000,  "reset_after_seconds": 3600,  "reset_at": 1760000000},
              "secondary_window": {"used_percent": 84, "limit_window_seconds": 604800, "reset_after_seconds": 90000, "reset_at": 1760500000}},
            "credits": {"has_credits": true, "unlimited": false, "balance": "9.99"},
            "additional_rate_limits": [{"limit_name":"codex_other","rate_limit":{
              "primary_window": {"used_percent": 1, "limit_window_seconds": 18000, "reset_after_seconds": 10}}}]
        });
        let r = parse_codex_usage(&body, 100).unwrap();
        assert_eq!(r.plan.as_deref(), Some("pro"));
        assert_eq!(r.status.as_deref(), Some("allowed"));
        let week = r.windows.iter().find(|w| w.id == "seven_day").unwrap();
        assert_eq!(week.used_percent, 84.0);
        assert_eq!(week.resets_at, Some(1_760_500_000));
        let other = r
            .windows
            .iter()
            .find(|w| w.id == "codex_other_five_hour")
            .unwrap();
        assert_eq!(other.resets_at, Some(110));
        assert_eq!(r.credits.unwrap().balance_usd, Some(9.99));
    }

    #[test]
    fn codex_rate_limits_event() {
        let ev = json!({"type":"codex.rate_limits","plan_type":"plus",
            "rate_limits":{"primary":{"used_percent":5.0,"window_minutes":300,"reset_at":1760000000},
                           "secondary":{"used_percent":30.0,"window_minutes":10080,"reset_at":1760500000}}});
        let r = parse_codex_rate_limits_event(&ev, 0).unwrap();
        assert_eq!(r.plan.as_deref(), Some("plus"));
        assert_eq!(r.windows.len(), 2);
        assert!(r
            .windows
            .iter()
            .any(|w| w.id == "seven_day" && w.used_percent == 30.0));
        assert!(parse_codex_rate_limits_event(&json!({"type":"response.created"}), 0).is_none());
    }

    #[test]
    fn copilot_user() {
        let body = json!({"copilot_plan":"individual","quota_reset_date":"2026-11-01",
          "quota_snapshots":{
            "premium_interactions":{"entitlement":300,"remaining":250,"percent_remaining":83.3,"unlimited":false},
            "chat":{"entitlement":0,"remaining":0,"percent_remaining":100,"unlimited":true}}});
        let r = parse_copilot_user(&body).unwrap();
        assert_eq!(r.plan.as_deref(), Some("individual"));
        assert_eq!(r.windows.len(), 1);
        let w = &r.windows[0];
        assert_eq!(w.id, "monthly");
        assert!((w.used_percent - 16.7).abs() < 1e-6);
        assert!(w.resets_at.is_some());
        assert_eq!(r.quotas[0].limit, Some(300.0));
    }
}
