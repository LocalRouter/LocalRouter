//! Usage endpoint response bodies.
//!
//! - Claude: `GET api.anthropic.com/api/oauth/usage` (what Claude Code's
//!   `/usage` shows) and `/api/oauth/profile` (plan tier).
//! - ChatGPT: `GET chatgpt.com/backend-api/wham/usage` (what Codex's
//!   `/status` shows) and the in-stream `codex.rate_limits` event.
//! - GitHub Copilot: `GET api.github.com/copilot_internal/user`.
//! - API keys (documented, same key as inference): OpenRouter `GET /key`,
//!   LLM Gateway `GET /v1/key`, Vercel AI Gateway `GET /v1/credits`.

use serde_json::Value;

use crate::headers::{codex_slot_has_data, codex_window, parse_epoch};
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

/// A finite, non-negative amount (numbers or decimal strings).
fn amount_of(v: Option<&Value>) -> Option<f64> {
    f64_of(v).filter(|f| f.is_finite() && *f >= 0.0)
}

/// The next UTC midnight after `now`.
fn next_utc_midnight(now: i64) -> i64 {
    now.saturating_sub(now.rem_euclid(86_400))
        .saturating_add(86_400)
}

/// OpenRouter `GET /api/v1/key`: the key's spend limit (with its reset
/// period) or, without one, this month's spend; and the daily request
/// allowance for free models.
pub fn parse_openrouter_key(body: &Value, now: i64) -> Option<UsageReport> {
    let d = body.get("data")?.as_object()?;
    let mut report = UsageReport::default();
    let usage = amount_of(d.get("usage"));
    let limit = amount_of(d.get("limit")).filter(|l| *l > 0.0);
    let limit_remaining = f64_of(d.get("limit_remaining")).filter(|f| f.is_finite());
    let reset = d
        .get("limit_reset")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|r| !r.is_empty());
    report.credits = Some(match limit {
        Some(limit) => {
            let remaining = limit_remaining.or_else(|| Some(limit - usage?));
            CreditsReading {
                label: match reset {
                    Some(r) => format!("Key limit ({})", r.to_ascii_lowercase()),
                    None => "Key limit".to_string(),
                },
                balance_usd: remaining,
                limit_usd: Some(limit),
                used_usd: remaining.map(|r| (limit - r).max(0.0)).or(usage),
                unlimited: false,
                currency: Some("USD".to_string()),
            }
        }
        None => match amount_of(d.get("usage_monthly")) {
            Some(month) => CreditsReading {
                label: "Spent this month".to_string(),
                used_usd: Some(month),
                unlimited: true,
                currency: Some("USD".to_string()),
                ..Default::default()
            },
            None => CreditsReading {
                label: "Spend".to_string(),
                used_usd: usage,
                unlimited: true,
                currency: Some("USD".to_string()),
                ..Default::default()
            },
        },
    });
    if let Some(free) = d.get("free_model_daily_requests") {
        let limit = amount_of(free.get("limit"));
        let remaining = amount_of(free.get("remaining")).or_else(|| {
            let used = amount_of(free.get("used"))?;
            Some((limit? - used).max(0.0))
        });
        if limit.is_some() || remaining.is_some() {
            report.quotas.push(QuotaReading {
                id: "free_requests_day".to_string(),
                limit,
                remaining,
                resets_at: Some(next_utc_midnight(now)),
            });
        }
    }
    (!report.is_empty()).then_some(report)
}

/// LLM Gateway `GET /v1/key`: the key's spend and limit, and — on a DevPass
/// plan — its plan credits and weekly premium allowance.
pub fn parse_llmgateway_key(body: &Value) -> Option<UsageReport> {
    let d = body.get("data")?.as_object()?;
    let mut report = UsageReport::default();
    let usage = amount_of(d.get("usage"));
    let limit = amount_of(d.get("limit")).filter(|l| *l > 0.0);
    if usage.is_some() || limit.is_some() {
        report.credits = Some(CreditsReading {
            label: if limit.is_some() {
                "Key limit"
            } else {
                "Spend"
            }
            .to_string(),
            balance_usd: limit.zip(usage).map(|(l, u)| (l - u).max(0.0)),
            limit_usd: limit,
            used_usd: usage,
            unlimited: limit.is_none(),
            currency: Some("USD".to_string()),
        });
    }
    let plan = d
        .get("devPlan")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|p| !p.is_empty() && !p.eq_ignore_ascii_case("none"));
    if let Some(plan) = plan {
        report.plan = Some(plan.to_string());
        let share = |used: Option<f64>, limit: Option<f64>| match (used, limit) {
            (Some(u), Some(l)) if l > 0.0 => Some(u / l * 100.0),
            _ => None,
        };
        if let Some(used) = share(
            amount_of(d.get("devPlanCreditsUsed")),
            amount_of(d.get("devPlanCreditsLimit")),
        ) {
            report.windows.push(WindowReading {
                id: "dev_plan_credits".to_string(),
                label: "DevPass credits".to_string(),
                used_percent: used,
                resets_at: None,
                window_secs: None,
            });
        }
        if let Some(used) = share(
            amount_of(d.get("devPlanPremiumCreditsUsed")),
            amount_of(d.get("devPlanPremiumWeeklyLimit")),
        ) {
            report.windows.push(WindowReading {
                id: "seven_day_premium".to_string(),
                label: window_label("seven_day_premium"),
                used_percent: used,
                resets_at: time_of(d.get("devPlanPremiumWeekResetsAt")),
                window_secs: Some(7 * 86_400),
            });
        }
    }
    (!report.is_empty()).then_some(report)
}

/// Vercel AI Gateway `GET /v1/credits`: `{"balance":"95.50","total_used":"4.50"}`.
pub fn parse_vercel_credits(body: &Value) -> Option<UsageReport> {
    let balance = f64_of(body.get("balance")).filter(|f| f.is_finite());
    let used = amount_of(body.get("total_used"));
    if balance.is_none() && used.is_none() {
        return None;
    }
    Some(UsageReport {
        credits: Some(CreditsReading {
            label: "Credits".to_string(),
            balance_usd: balance,
            used_usd: used,
            currency: Some("USD".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    })
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
    let used = f64_of(w.get("used_percent")).filter(|u| u.is_finite())?;
    let secs = f64_of(w.get("limit_window_seconds"))
        .map(|s| s as i64)
        .or_else(|| f64_of(w.get("window_minutes")).map(|m| (m * 60.0) as i64))
        .filter(|s| *s > 0);
    let reset_at = time_of(w.get("reset_at"));
    let reset_after = f64_of(w.get("reset_after_seconds"))
        .or_else(|| f64_of(w.get("resets_in_seconds")))
        .filter(|s| *s > 0.0);
    if !codex_slot_has_data(used, secs, reset_at.is_some() || reset_after.is_some()) {
        return None;
    }
    let resets_at =
        reset_at.or_else(|| reset_after.and_then(|s| crate::headers::after_secs(now, s)));
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
        .or_else(|| event.get("limit_name"))
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

    /// Pro Lite: the weekly limit is the primary window; the secondary slot
    /// is absent or empty and must not overwrite it.
    #[test]
    fn codex_weekly_primary_with_empty_secondary() {
        let ev = json!({"type":"codex.rate_limits","plan_type":"prolite",
            "rate_limits":{"allowed":true,"limit_reached":false,
                "primary":{"used_percent":13,"window_minutes":10080,"reset_at":1791985298i64},
                "secondary":{"used_percent":0,"window_minutes":0,"reset_at":null}},
            "credits":{"has_credits":false,"unlimited":false,"balance":"0"}});
        let r = parse_codex_rate_limits_event(&ev, 0).unwrap();
        assert_eq!(r.windows.len(), 1);
        assert_eq!(r.windows[0].id, "seven_day");
        assert_eq!(r.windows[0].used_percent, 13.0);
        assert_eq!(r.windows[0].resets_at, Some(1_791_985_298));
        assert_eq!(r.credits, None);

        let ev = json!({"type":"codex.rate_limits","limit_name":"codex",
            "rate_limits":{"primary":{"used_percent":13,"window_minutes":10080},"secondary":null}});
        let r = parse_codex_rate_limits_event(&ev, 0).unwrap();
        assert_eq!(r.windows.len(), 1);
        assert_eq!(r.windows[0].id, "seven_day");

        let body = json!({"plan_type":"prolite","rate_limit":{"allowed":true,"limit_reached":false,
            "primary_window":{"used_percent":13,"limit_window_seconds":604800,"reset_after_seconds":597265,"reset_at":1791985298i64},
            "secondary_window":{"used_percent":0,"limit_window_seconds":0,"reset_after_seconds":0,"reset_at":0}}});
        let r = parse_codex_usage(&body, 0).unwrap();
        let ids: Vec<_> = r.windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, vec!["seven_day"]);
        assert_eq!(r.windows[0].used_percent, 13.0);
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

    /// The documented example of OpenRouter's `GET /api/v1/key`, with a
    /// monthly reset and the free-model allowance.
    #[test]
    fn openrouter_key_with_limit_and_free_requests() {
        let now = 1_791_720_000; // 2026-10-11T12:00:00Z
        let body = json!({"data": {
            "label": "sk-or-v1-0e6...1c96",
            "limit": 100, "limit_remaining": 74.5, "limit_reset": "monthly",
            "include_byok_in_limit": false,
            "usage": 25.5, "usage_daily": 25.5, "usage_weekly": 25.5, "usage_monthly": 25.5,
            "byok_usage": 17.38, "is_free_tier": false,
            "free_model_daily_requests": {"used": 12, "limit": 1000, "remaining": 988},
            "rate_limit": {"requests": 20, "interval": "10s"}
        }});
        let r = parse_openrouter_key(&body, now).unwrap();
        let c = r.credits.unwrap();
        assert_eq!(c.label, "Key limit (monthly)");
        assert_eq!(c.limit_usd, Some(100.0));
        assert_eq!(c.balance_usd, Some(74.5));
        assert_eq!(c.used_usd, Some(25.5));
        assert_eq!(c.currency.as_deref(), Some("USD"));
        let q = &r.quotas[0];
        assert_eq!(q.id, "free_requests_day");
        assert_eq!((q.limit, q.remaining), (Some(1000.0), Some(988.0)));
        assert_eq!(q.resets_at, Some(1_791_763_200)); // next UTC midnight
    }

    #[test]
    fn openrouter_key_without_limit_shows_this_months_spend() {
        let body = json!({"data": {"limit": null, "usage": 310.2, "usage_monthly": 12.75,
            "limit_remaining": null, "is_free_tier": false}});
        let c = parse_openrouter_key(&body, 0).unwrap().credits.unwrap();
        assert_eq!(c.label, "Spent this month");
        assert_eq!(c.used_usd, Some(12.75));
        assert!(c.unlimited);
        // Older responses without the period fields: lifetime spend.
        let c = parse_openrouter_key(&json!({"data": {"usage": 3.0, "limit": null}}), 0)
            .unwrap()
            .credits
            .unwrap();
        assert_eq!((c.label.as_str(), c.used_usd), ("Spend", Some(3.0)));
        // Used but no remaining given: derived from the limit.
        let c = parse_openrouter_key(&json!({"data": {"usage": 3.0, "limit": 10}}), 0)
            .unwrap()
            .credits
            .unwrap();
        assert_eq!(c.balance_usd, Some(7.0));
        assert!(parse_openrouter_key(&json!({"error": "nope"}), 0).is_none());
    }

    /// The documented example of LLM Gateway's `GET /v1/key` on DevPass Pro.
    #[test]
    fn llmgateway_key_devpass() {
        let body = json!({"data":{"label":"My coding tool","usage":"31.42","limit":null,
            "devPlan":"pro","devPlanCreditsUsed":"25","devPlanCreditsLimit":"237",
            "devPlanCreditsRemaining":"212.00","devPlanPremiumWeeklyLimit":"35.55",
            "devPlanPremiumCreditsUsed":"5.00","devPlanPremiumWeekResetsAt":"2026-08-28T12:00:00.000Z"}});
        let r = parse_llmgateway_key(&body).unwrap();
        assert_eq!(r.plan.as_deref(), Some("pro"));
        let c = r.credits.unwrap();
        assert_eq!(
            (c.label.as_str(), c.used_usd, c.unlimited),
            ("Spend", Some(31.42), true)
        );
        let credits = r
            .windows
            .iter()
            .find(|w| w.id == "dev_plan_credits")
            .unwrap();
        assert!((credits.used_percent - 25.0 / 237.0 * 100.0).abs() < 1e-9);
        let premium = r
            .windows
            .iter()
            .find(|w| w.id == "seven_day_premium")
            .unwrap();
        assert_eq!(premium.label, "Weekly · Premium");
        assert!((premium.used_percent - 5.0 / 35.55 * 100.0).abs() < 1e-9);
        assert_eq!(premium.window_secs, Some(604_800));
        assert_eq!(
            premium.resets_at,
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-08-28T12:00:00Z")
                    .unwrap()
                    .timestamp()
            )
        );
    }

    #[test]
    fn llmgateway_key_pay_as_you_go_with_limit() {
        let body = json!({"data":{"usage":"40","limit":"50","devPlan":"none",
            "devPlanCreditsUsed":"0","devPlanCreditsLimit":"0"}});
        let r = parse_llmgateway_key(&body).unwrap();
        assert!(r.plan.is_none());
        assert!(r.windows.is_empty(), "no DevPass, no plan windows");
        let c = r.credits.unwrap();
        assert_eq!(c.label, "Key limit");
        assert_eq!(
            (c.used_usd, c.limit_usd, c.balance_usd),
            (Some(40.0), Some(50.0), Some(10.0))
        );
    }

    #[test]
    fn vercel_credits() {
        let r = parse_vercel_credits(&json!({"balance":"95.50","total_used":"4.50"})).unwrap();
        let c = r.credits.unwrap();
        assert_eq!((c.balance_usd, c.used_usd), (Some(95.5), Some(4.5)));
        assert_eq!(c.label, "Credits");
        assert!(parse_vercel_credits(&json!({"balance":"NaN"})).is_none());
        assert!(parse_vercel_credits(&json!({})).is_none());
    }
}
