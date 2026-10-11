//! Usage readings carried in provider response headers.
//!
//! - Claude subscriptions: `anthropic-ratelimit-unified-{5h,7d,…}-utilization`
//!   (0–1 fraction) with matching `-reset` (unix seconds) and `-status`.
//! - ChatGPT (Codex backend): `x-codex-{primary,secondary}-used-percent`,
//!   `-window-minutes`, `-reset-at`, plus `x-codex-credits-*` and
//!   `x-codex-plan-type`. Other limit families use the same shape under their
//!   own prefix (`x-codex-other-…`, named by `-limit-name`). A slot the plan
//!   does not have is sent empty (`0`, `0`, no reset) — e.g. Pro Lite's
//!   weekly limit is the primary window and its secondary is empty.
//! - API keys: `x-ratelimit-{limit,remaining,reset}-<what>` (OpenAI, Groq,
//!   Cerebras, DigitalOcean, …), `anthropic-ratelimit-<what>-{limit,
//!   remaining,reset}` (plus Anthropic's `anthropic-priority-*` and
//!   `anthropic-fast-*`), and the IETF `RateLimit` / `RateLimit-Policy`
//!   structured fields (LLM Gateway). What a name means differs per
//!   provider (Groq's `-requests` is per day), so each family's quotas are
//!   normalized onto period-named ids (`requests_day`, `tokens_minute`) by
//!   [`parse_headers_for`].

use std::collections::BTreeMap;

use http::HeaderMap;

use crate::providers::HeaderScheme;
use crate::types::{
    id_fragment, period_name, quota_id, window_label, window_secs_for, CreditsReading,
    QuotaReading, UsageReport, WindowReading,
};

const UNIFIED: &str = "anthropic-ratelimit-unified-";
/// Anthropic's per-key rate-limit families → quota id prefix: the standard
/// limits, Priority Tier and fast mode.
const ANTHROPIC_PREFIXES: &[(&str, &str)] = &[
    ("anthropic-ratelimit-", ""),
    ("anthropic-priority-", "priority_"),
    ("anthropic-fast-", "fast_"),
];

/// Parse every usage reading in a response's headers. `now` is unix seconds,
/// used to resolve relative reset times.
pub fn parse_headers(headers: &HeaderMap, now: i64) -> UsageReport {
    parse_header_map(&lowercase_map(headers), now)
}

/// [`parse_headers`] for a provider family whose headers are read as
/// `scheme`: the generic readings, with the family's quotas renamed to what
/// its documentation says they count.
pub fn parse_headers_for(scheme: HeaderScheme, headers: &HeaderMap, now: i64) -> UsageReport {
    parse_header_map_for(scheme, &lowercase_map(headers), now)
}

fn lowercase_map(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(k, v)| {
            Some((
                k.as_str().to_ascii_lowercase(),
                v.to_str().ok()?.trim().to_string(),
            ))
        })
        .collect()
}

/// [`parse_headers`] over lowercase name → value pairs.
pub fn parse_header_map(map: &BTreeMap<String, String>, now: i64) -> UsageReport {
    let mut report = UsageReport::default();
    parse_anthropic_unified(map, &mut report);
    parse_codex(map, &mut report, now);
    report.quotas = parse_quotas(map, now);
    report
}

/// [`parse_headers_for`] over lowercase name → value pairs.
pub fn parse_header_map_for(
    scheme: HeaderScheme,
    map: &BTreeMap<String, String>,
    now: i64,
) -> UsageReport {
    let mut report = parse_header_map(map, now);
    let quotas = std::mem::take(&mut report.quotas);
    report.quotas = match scheme {
        HeaderScheme::None | HeaderScheme::Generic => quotas,
        HeaderScheme::TogetherPerSecond => {
            let mut quotas = rename_quotas(quotas, |id| match id {
                "rate" => Some(quota_id("requests", "second")),
                other => Some(other.to_string()),
            });
            let tokens = QuotaReading {
                id: quota_id("tokens", "second"),
                limit: map.get("x-tokenlimit-limit").and_then(|v| v.parse().ok()),
                remaining: map
                    .get("x-tokenlimit-remaining")
                    .and_then(|v| v.parse().ok()),
                resets_at: None,
            };
            if tokens.limit.is_some() || tokens.remaining.is_some() {
                quotas.push(tokens);
            }
            quotas
        }
        HeaderScheme::GroqDailyRequests => rename_quotas(quotas, |id| match id {
            "requests" => Some(quota_id("requests", "day")),
            "tokens" => Some(quota_id("tokens", "minute")),
            other => Some(other.to_string()),
        }),
        HeaderScheme::DigitalOcean => {
            let quotas = rename_quotas(quotas, |id| match id {
                "requests" => Some(quota_id("requests", "minute")),
                "tokens_per_minute" => Some(quota_id("tokens", "minute")),
                "tokens_per_day" => Some(quota_id("tokens", "day")),
                other => Some(other.to_string()),
            });
            // Resets are refill projections in epoch seconds; `0` means
            // there was capacity to spare (no reset pending).
            quotas
                .into_iter()
                .map(|mut q| {
                    if let Some(raw) = map.get(&format!("x-ratelimit-reset-{}", do_header(&q.id))) {
                        q.resets_at = parse_epoch(raw);
                    }
                    q
                })
                .collect()
        }
        HeaderScheme::IetfStructured => {
            // The unprefixed `x-ratelimit-*` / `ratelimit-*` triples are
            // overwritten by other gates (a spend-cap 429 puts the dollar
            // cap in them); the structured fields are not.
            let mut quotas: Vec<_> = quotas.into_iter().filter(|q| q.id != "rate").collect();
            quotas.extend(parse_structured_ratelimit(map, now));
            quotas
        }
    };
    report
}

/// The DigitalOcean header suffix a normalized quota id came from.
fn do_header(id: &str) -> &str {
    match id {
        "requests_minute" => "requests",
        "tokens_minute" => "tokens-per-minute",
        "tokens_day" => "tokens-per-day",
        other => other,
    }
}

/// Rename quotas by id; `None` drops one.
fn rename_quotas(
    quotas: Vec<QuotaReading>,
    rename: impl Fn(&str) -> Option<String>,
) -> Vec<QuotaReading> {
    quotas
        .into_iter()
        .filter_map(|q| {
            let id = rename(&q.id)?;
            Some(QuotaReading { id, ..q })
        })
        .collect()
}

/// One item of an RFC 8941 list: its (unquoted) name and parameters.
fn structured_items(value: &str) -> Vec<(String, BTreeMap<String, String>)> {
    value
        .split(',')
        .filter_map(|item| {
            let mut parts = item.split(';').map(str::trim);
            let name = parts.next()?.trim_matches('"').to_string();
            if name.is_empty() {
                return None;
            }
            let params = parts
                .filter_map(|p| {
                    let (k, v) = p.split_once('=')?;
                    Some((
                        k.trim().to_ascii_lowercase(),
                        v.trim().trim_matches('"').to_string(),
                    ))
                })
                .collect();
            Some((name, params))
        })
        .collect()
}

/// Quotas from the IETF `RateLimit-Policy` (`"requests";q=600;w=60`) and
/// `RateLimit` (`"requests";r=599;t=60`) structured fields, matched by
/// policy name. A policy's window names the period (`w=60` → per minute);
/// concurrency policies are not rates and are skipped.
fn parse_structured_ratelimit(map: &BTreeMap<String, String>, now: i64) -> Vec<QuotaReading> {
    let Some(policies) = map.get("ratelimit-policy") else {
        return Vec::new();
    };
    let current: BTreeMap<String, BTreeMap<String, String>> = map
        .get("ratelimit")
        .map(|v| structured_items(v).into_iter().collect())
        .unwrap_or_default();
    structured_items(policies)
        .into_iter()
        .filter_map(|(name, params)| {
            let unit = params.get("qu").map(String::as_str).unwrap_or("requests");
            if unit.starts_with("concurrent") || name == "concurrency" {
                return None;
            }
            let limit = params.get("q").and_then(|v| v.parse::<f64>().ok());
            let window = params
                .get("w")
                .and_then(|v| v.parse::<i64>().ok())
                .filter(|w| *w > 0);
            let metric = if unit == "requests" {
                "requests".to_string()
            } else {
                id_fragment(unit)
            };
            let id = match window.and_then(period_name) {
                Some(period) => quota_id(&metric, period),
                None => metric,
            };
            let now_params = current.get(&name);
            let remaining = now_params
                .and_then(|p| p.get("r"))
                .and_then(|v| v.parse::<f64>().ok());
            let resets_at = now_params
                .and_then(|p| p.get("t"))
                .and_then(|v| v.parse::<f64>().ok())
                .and_then(|t| after_secs(now, t));
            (limit.is_some() || remaining.is_some()).then_some(QuotaReading {
                id,
                limit,
                remaining,
                resets_at,
            })
        })
        .collect()
}

fn parse_anthropic_unified(map: &BTreeMap<String, String>, report: &mut UsageReport) {
    for (name, value) in map {
        let Some(rest) = name.strip_prefix(UNIFIED) else {
            continue;
        };
        let Some(abbrev) = rest.strip_suffix("-utilization") else {
            continue;
        };
        let Some(fraction) = value.parse::<f64>().ok().filter(|f| f.is_finite()) else {
            continue;
        };
        let id = unified_window_id(abbrev);
        let resets_at = map
            .get(&format!("{UNIFIED}{abbrev}-reset"))
            .and_then(|v| parse_epoch(v));
        report.windows.push(WindowReading {
            label: window_label(&id),
            window_secs: window_secs_for(&id),
            id,
            used_percent: fraction * 100.0,
            resets_at,
        });
    }
    if let Some(status) = map.get(&format!("{UNIFIED}status")) {
        report.status = Some(status.clone());
    }
}

/// `5h` → `five_hour`, `7d` → `seven_day`, `7d_opus` → `seven_day_opus`,
/// `overage` → `extra_usage`.
fn unified_window_id(abbrev: &str) -> String {
    let (head, tail) = match abbrev.split_once(['_', '-']) {
        Some((h, t)) => (h, Some(t)),
        None => (abbrev, None),
    };
    let base = match head {
        "5h" => "five_hour".to_string(),
        "7d" => "seven_day".to_string(),
        "overage" => "extra_usage".to_string(),
        other => id_fragment(other),
    };
    match tail {
        Some(t) if !t.is_empty() => format!("{base}_{}", id_fragment(t)),
        _ => base,
    }
}

fn parse_codex(map: &BTreeMap<String, String>, report: &mut UsageReport, now: i64) {
    for (name, value) in map {
        let Some(body) = name.strip_prefix("x-") else {
            continue;
        };
        let (limit, slot) = if let Some(l) = body.strip_suffix("-primary-used-percent") {
            (l, "primary")
        } else if let Some(l) = body.strip_suffix("-secondary-used-percent") {
            (l, "secondary")
        } else {
            continue;
        };
        let Some(used) = value.parse::<f64>().ok().filter(|f| f.is_finite()) else {
            continue;
        };
        let prefix = format!("x-{limit}-{slot}");
        let minutes = map
            .get(&format!("{prefix}-window-minutes"))
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|m| m.is_finite() && *m > 0.0)
            .map(|m| (m * 60.0) as i64);
        let reset_at = map
            .get(&format!("{prefix}-reset-at"))
            .and_then(|v| parse_epoch(v));
        let reset_after = map
            .get(&format!("{prefix}-reset-after-seconds"))
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|s| *s > 0.0);
        if !codex_slot_has_data(used, minutes, reset_at.is_some() || reset_after.is_some()) {
            continue;
        }
        let resets_at = reset_at.or_else(|| after_secs(now, reset_after?));
        let (id, mut label) = codex_window(limit, slot, minutes);
        if let Some(name) = map
            .get(&format!("x-{limit}-limit-name"))
            .filter(|n| !n.is_empty())
        {
            if let Some((_, base)) = label.split_once(" · ") {
                label = format!("{name} · {base}");
            }
        }
        report.windows.push(WindowReading {
            id,
            label,
            used_percent: used,
            resets_at,
            window_secs: minutes,
        });
    }
    if let Some(plan) = map.get("x-codex-plan-type").filter(|p| !p.is_empty()) {
        report.plan = Some(plan.clone());
    }
    if map.contains_key("x-codex-rate-limit-reached-type") {
        report.status = Some("rejected".to_string());
    }
    let has_credits = map
        .get("x-codex-credits-has-credits")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let unlimited = map
        .get("x-codex-credits-unlimited")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let balance = map
        .get("x-codex-credits-balance")
        .and_then(|v| v.parse::<f64>().ok());
    if has_credits || unlimited || balance.is_some_and(|b| b > 0.0) {
        report.credits = Some(CreditsReading {
            label: "Credits".to_string(),
            balance_usd: balance,
            unlimited,
            ..Default::default()
        });
    }
}

/// Whether a Codex rate-limit slot carries a window: one the plan does not
/// have arrives as zero usage, zero length and no reset (as Codex itself
/// reads it).
pub(crate) fn codex_slot_has_data(used: f64, window_secs: Option<i64>, has_reset: bool) -> bool {
    used != 0.0 || window_secs.is_some_and(|s| s > 0) || has_reset
}

/// Id + label for a Codex rate-limit window. The `codex` family maps onto
/// the shared `five_hour` / `seven_day` ids so it lines up with
/// `wham/usage` and the Claude windows.
pub(crate) fn codex_window(limit: &str, slot: &str, window_secs: Option<i64>) -> (String, String) {
    let base = match window_secs {
        Some(18_000) => "five_hour".to_string(),
        Some(604_800) => "seven_day".to_string(),
        Some(s) if s > 0 && s % 86_400 == 0 => format!("{}_day", s / 86_400),
        Some(s) if s > 0 && s % 3_600 == 0 => format!("{}_hour", s / 3_600),
        _ if slot == "primary" => "five_hour".to_string(),
        _ => "seven_day".to_string(),
    };
    let base_label = match base.as_str() {
        "five_hour" | "seven_day" => window_label(&base),
        other => other.replace('_', " "),
    };
    let family = id_fragment(limit);
    if family == "codex" || family.is_empty() {
        (base, base_label)
    } else {
        let label = window_label(&family);
        (
            format!("{family}_{base}"),
            format!("{label} · {base_label}"),
        )
    }
}

fn parse_quotas(map: &BTreeMap<String, String>, now: i64) -> Vec<QuotaReading> {
    let mut quotas: BTreeMap<String, QuotaReading> = BTreeMap::new();
    let mut set = |id: String, field: &str, value: &str| {
        let q = quotas.entry(id.clone()).or_insert_with(|| QuotaReading {
            id,
            ..Default::default()
        });
        match field {
            "limit" => q.limit = value.parse().ok(),
            "remaining" => q.remaining = value.parse().ok(),
            "reset" => q.resets_at = parse_reset(value, now),
            _ => {}
        }
    };
    for (name, value) in map {
        if let Some(rest) = name.strip_prefix("x-ratelimit-") {
            for field in ["limit", "remaining", "reset"] {
                if rest == field {
                    set("rate".to_string(), field, value);
                } else if let Some(what) = rest.strip_prefix(&format!("{field}-")) {
                    set(id_fragment(what), field, value);
                }
            }
        } else if let Some((tier, rest)) = ANTHROPIC_PREFIXES
            .iter()
            .find_map(|(prefix, tier)| Some((*tier, name.strip_prefix(prefix)?)))
        {
            if rest.starts_with("unified-") {
                continue;
            }
            for field in ["limit", "remaining", "reset"] {
                if let Some(what) = rest.strip_suffix(&format!("-{field}")) {
                    set(format!("{tier}{}", id_fragment(what)), field, value);
                }
            }
        }
    }
    quotas
        .into_values()
        .filter(|q| q.limit.is_some() || q.remaining.is_some())
        .collect()
}

/// An absolute unix timestamp in seconds (or milliseconds), or RFC 3339.
pub(crate) fn parse_epoch(value: &str) -> Option<i64> {
    let v = value.trim();
    if let Ok(n) = v.parse::<f64>() {
        if !n.is_finite() || n <= 0.0 {
            return None;
        }
        return Some(if n > 1e12 {
            (n / 1000.0) as i64
        } else {
            n as i64
        });
    }
    chrono::DateTime::parse_from_rfc3339(v)
        .ok()
        .map(|d| d.timestamp())
}

/// A reset given as an absolute time (epoch / RFC 3339) or as a delay
/// (`1s`, `6m0s`, `2m59.56s`, `59ms`, or bare seconds).
pub(crate) fn parse_reset(value: &str, now: i64) -> Option<i64> {
    let v = value.trim();
    if let Ok(n) = v.parse::<f64>() {
        if !n.is_finite() || n < 0.0 {
            return None;
        }
        // Small bare numbers are delays; large ones are timestamps.
        return if n > 1e9 {
            parse_epoch(v)
        } else {
            after_secs(now, n)
        };
    }
    if let Some(secs) = parse_go_duration(v) {
        return after_secs(now, secs);
    }
    parse_epoch(v)
}

/// `now` plus a delay in seconds read from a response, rounded up. A
/// negative or non-finite delay gives `None`; a huge one saturates instead
/// of overflowing.
pub(crate) fn after_secs(now: i64, secs: f64) -> Option<i64> {
    if !secs.is_finite() || secs < 0.0 {
        return None;
    }
    // `as` saturates for out-of-range floats.
    Some(now.saturating_add(secs.ceil() as i64))
}

/// Go-style duration (`1h2m3.5s`, `59ms`) in seconds.
fn parse_go_duration(s: &str) -> Option<f64> {
    let mut total = 0.0;
    let mut rest = s;
    let mut any = false;
    while !rest.is_empty() {
        let num_len = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        if num_len == 0 {
            return None;
        }
        let n: f64 = rest[..num_len].parse().ok()?;
        rest = &rest[num_len..];
        let unit_len = rest
            .find(|c: char| c.is_ascii_digit() || c == '.')
            .unwrap_or(rest.len());
        let factor = match &rest[..unit_len] {
            "h" => 3600.0,
            "m" => 60.0,
            "s" => 1.0,
            "ms" => 1e-3,
            "us" | "µs" => 1e-6,
            "ns" => 1e-9,
            _ => return None,
        };
        rest = &rest[unit_len..];
        total += n * factor;
        any = true;
    }
    any.then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn anthropic_unified_windows() {
        let m = map(&[
            ("anthropic-ratelimit-unified-status", "allowed_warning"),
            ("anthropic-ratelimit-unified-5h-utilization", "0.42"),
            ("anthropic-ratelimit-unified-5h-reset", "1760000000"),
            ("anthropic-ratelimit-unified-7d-utilization", "0.9"),
            ("anthropic-ratelimit-unified-7d-reset", "1760500000"),
            ("anthropic-ratelimit-unified-7d_opus-utilization", "0.1"),
            ("anthropic-ratelimit-unified-reset", "1760000000"),
        ]);
        let r = parse_header_map(&m, 0);
        assert_eq!(r.status.as_deref(), Some("allowed_warning"));
        let five = r.windows.iter().find(|w| w.id == "five_hour").unwrap();
        assert!((five.used_percent - 42.0).abs() < 1e-9);
        assert_eq!(five.resets_at, Some(1_760_000_000));
        assert_eq!(five.window_secs, Some(18_000));
        let week = r.windows.iter().find(|w| w.id == "seven_day").unwrap();
        assert!((week.used_percent - 90.0).abs() < 1e-9);
        assert!(r.windows.iter().any(|w| w.id == "seven_day_opus"));
        assert!(r.quotas.is_empty(), "unified headers are not quotas");
    }

    #[test]
    fn delays_saturate_instead_of_overflowing() {
        assert_eq!(after_secs(100, 1.2), Some(102));
        assert_eq!(after_secs(100, -1.0), None);
        assert_eq!(after_secs(100, f64::NAN), None);
        assert_eq!(after_secs(100, f64::INFINITY), None);
        assert_eq!(after_secs(i64::MAX - 1, 1e300), Some(i64::MAX));
        assert_eq!(
            parse_reset("99999999999999999999h", i64::MAX - 5),
            Some(i64::MAX)
        );
        let m = map(&[
            ("x-codex-primary-used-percent", "1"),
            ("x-codex-primary-reset-after-seconds", "1e308"),
        ]);
        let r = parse_header_map(&m, i64::MAX - 10);
        assert_eq!(r.windows[0].resets_at, Some(i64::MAX));
    }

    #[test]
    fn codex_windows_and_credits() {
        let m = map(&[
            ("x-codex-primary-used-percent", "12.5"),
            ("x-codex-primary-window-minutes", "300"),
            ("x-codex-primary-reset-at", "1760000000"),
            ("x-codex-secondary-used-percent", "47"),
            ("x-codex-secondary-window-minutes", "10080"),
            ("x-codex-secondary-reset-at", "1760500000"),
            ("x-codex-other-primary-used-percent", "3"),
            ("x-codex-other-primary-window-minutes", "300"),
            ("x-codex-credits-has-credits", "true"),
            ("x-codex-credits-balance", "9.99"),
        ]);
        let r = parse_header_map(&m, 0);
        let ids: Vec<_> = r.windows.iter().map(|w| w.id.as_str()).collect();
        assert!(ids.contains(&"five_hour"));
        assert!(ids.contains(&"seven_day"));
        assert!(ids.contains(&"codex_other_five_hour"));
        let week = r.windows.iter().find(|w| w.id == "seven_day").unwrap();
        assert_eq!(week.window_secs, Some(604_800));
        assert_eq!(week.resets_at, Some(1_760_500_000));
        let credits = r.credits.unwrap();
        assert_eq!(credits.balance_usd, Some(9.99));
        assert_eq!(credits.currency, None);
    }

    /// Headers the ChatGPT backend sends a Pro Lite account on
    /// `/backend-api/codex/responses` (from Codex's own log): the weekly
    /// limit is the primary window, the secondary slot is empty.
    #[test]
    fn codex_pro_lite_weekly_primary_and_empty_secondary() {
        let now = 1_791_388_033;
        let m = map(&[
            ("x-codex-active-limit", "premium"),
            ("x-codex-plan-type", "prolite"),
            ("x-codex-primary-used-percent", "13"),
            ("x-codex-primary-window-minutes", "10080"),
            ("x-codex-primary-over-secondary-limit-percent", "0"),
            ("x-codex-primary-reset-after-seconds", "597265"),
            ("x-codex-primary-reset-at", "1791985298"),
            ("x-codex-secondary-used-percent", "0"),
            ("x-codex-secondary-window-minutes", "0"),
            ("x-codex-secondary-reset-after-seconds", "0"),
            ("x-codex-secondary-reset-at", ""),
            ("x-codex-credits-has-credits", "False"),
            ("x-codex-credits-balance", "0"),
            ("x-codex-credits-unlimited", "False"),
        ]);
        let r = parse_header_map(&m, now);
        assert_eq!(r.plan.as_deref(), Some("prolite"));
        assert_eq!(r.windows.len(), 1, "the empty secondary is no window");
        let week = &r.windows[0];
        assert_eq!(week.id, "seven_day");
        assert!((week.used_percent - 13.0).abs() < 1e-9);
        assert_eq!(week.window_secs, Some(604_800));
        assert_eq!(week.resets_at, Some(1_791_985_298));
        assert_eq!(r.credits, None);
        assert_eq!(r.status, None);
    }

    #[test]
    fn codex_slot_with_any_data_is_kept() {
        // Zero usage but a known length or reset is a real window.
        let m = map(&[
            ("x-codex-secondary-used-percent", "0"),
            ("x-codex-secondary-window-minutes", "10080"),
            ("x-codex-primary-used-percent", "0"),
            ("x-codex-primary-reset-after-seconds", "60"),
        ]);
        let r = parse_header_map(&m, 1_000);
        let ids: Vec<_> = r.windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, vec!["five_hour", "seven_day"]);
        assert_eq!(r.windows[0].resets_at, Some(1_060));
        assert!(codex_slot_has_data(0.5, None, false));
        assert!(!codex_slot_has_data(0.0, Some(0), false));
        assert!(!codex_slot_has_data(0.0, None, false));
    }

    #[test]
    fn codex_family_named_by_limit_name() {
        let m = map(&[
            ("x-codex-bengalfox-primary-used-percent", "80"),
            ("x-codex-bengalfox-primary-window-minutes", "300"),
            ("x-codex-bengalfox-limit-name", "GPT-5.2-Codex-Sonic"),
        ]);
        let r = parse_header_map(&m, 0);
        assert_eq!(r.windows[0].id, "codex_bengalfox_five_hour");
        assert_eq!(r.windows[0].label, "GPT-5.2-Codex-Sonic · 5-hour session");
    }

    #[test]
    fn api_key_quotas() {
        let now = 1_000;
        let m = map(&[
            ("x-ratelimit-limit-requests", "5000"),
            ("x-ratelimit-remaining-requests", "4999"),
            ("x-ratelimit-reset-requests", "12ms"),
            ("x-ratelimit-limit-tokens", "800000"),
            ("x-ratelimit-remaining-tokens", "799000"),
            ("x-ratelimit-reset-tokens", "6m0s"),
            ("x-ratelimit-limit-requests-day", "14400"),
            ("x-ratelimit-remaining-requests-day", "14000"),
            ("x-ratelimit-reset-requests-day", "33011.38"),
            ("anthropic-ratelimit-input-tokens-limit", "2000000"),
            ("anthropic-ratelimit-input-tokens-remaining", "1999000"),
            (
                "anthropic-ratelimit-input-tokens-reset",
                "2026-10-10T12:00:00Z",
            ),
        ]);
        let r = parse_header_map(&m, now);
        let q = |id: &str| r.quotas.iter().find(|q| q.id == id).unwrap().clone();
        assert_eq!(q("requests").limit, Some(5000.0));
        assert_eq!(q("requests").resets_at, Some(now + 1));
        assert_eq!(q("tokens").resets_at, Some(now + 360));
        assert_eq!(q("requests_day").resets_at, Some(now + 33_012));
        let input = q("input_tokens");
        assert_eq!(input.remaining, Some(1_999_000.0));
        assert_eq!(
            input.resets_at,
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-10-10T12:00:00Z")
                    .unwrap()
                    .timestamp()
            )
        );
    }

    #[test]
    fn duration_and_epoch_parsing() {
        assert_eq!(parse_go_duration("2m59.56s"), Some(179.56));
        assert_eq!(parse_go_duration("1h"), Some(3600.0));
        assert_eq!(parse_go_duration("abc"), None);
        assert_eq!(parse_go_duration(""), None);
        assert_eq!(parse_epoch("1760000000000"), Some(1_760_000_000));
        assert_eq!(parse_epoch("0"), None);
        assert_eq!(parse_reset("1760000000", 5), Some(1_760_000_000));
        assert_eq!(parse_reset("7.66", 5), Some(13));
    }

    #[test]
    fn real_header_map_is_lowercased() {
        let mut h = HeaderMap::new();
        h.insert(
            "Anthropic-Ratelimit-Unified-7d-Utilization",
            "0.5".parse().unwrap(),
        );
        let r = parse_headers(&h, 0);
        assert_eq!(r.windows.len(), 1);
        assert_eq!(r.windows[0].id, "seven_day");
    }

    fn quota<'a>(r: &'a UsageReport, id: &str) -> &'a QuotaReading {
        r.quotas
            .iter()
            .find(|q| q.id == id)
            .unwrap_or_else(|| panic!("no {id} in {:?}", r.quotas))
    }

    /// Groq's documented example: `-requests` is per day, `-tokens` per
    /// minute, resets are Go durations.
    #[test]
    fn groq_requests_per_day_tokens_per_minute() {
        let now = 1_000;
        let m = map(&[
            ("x-ratelimit-limit-requests", "14400"),
            ("x-ratelimit-limit-tokens", "18000"),
            ("x-ratelimit-remaining-requests", "14370"),
            ("x-ratelimit-remaining-tokens", "17997"),
            ("x-ratelimit-reset-requests", "2m59.56s"),
            ("x-ratelimit-reset-tokens", "7.66s"),
        ]);
        let r = parse_header_map_for(HeaderScheme::GroqDailyRequests, &m, now);
        let rpd = quota(&r, "requests_day");
        assert_eq!((rpd.limit, rpd.remaining), (Some(14_400.0), Some(14_370.0)));
        assert_eq!(rpd.resets_at, Some(now + 180));
        let tpm = quota(&r, "tokens_minute");
        assert_eq!((tpm.limit, tpm.remaining), (Some(18_000.0), Some(17_997.0)));
        assert_eq!(tpm.resets_at, Some(now + 8));
        assert_eq!(r.quotas.len(), 2);
    }

    /// Cerebras' documented example: the names carry the period; resets are
    /// float seconds.
    #[test]
    fn cerebras_periods_from_names() {
        let now = 1_000;
        let m = map(&[
            ("x-ratelimit-limit-requests-day", "1000000000"),
            ("x-ratelimit-limit-tokens-minute", "1000000000"),
            ("x-ratelimit-remaining-requests-day", "999999997"),
            ("x-ratelimit-remaining-tokens-minute", "999998298"),
            ("x-ratelimit-reset-requests-day", "33011.382867097855"),
            ("x-ratelimit-reset-tokens-minute", "11.382867097854614"),
        ]);
        let r = parse_header_map_for(HeaderScheme::Generic, &m, now);
        assert_eq!(quota(&r, "requests_day").resets_at, Some(now + 33_012));
        assert_eq!(quota(&r, "tokens_minute").remaining, Some(999_998_298.0));
        assert_eq!(quota(&r, "tokens_minute").resets_at, Some(now + 12));
    }

    /// Together: per-second request and token limits.
    #[test]
    fn together_per_second() {
        let m = map(&[
            ("x-ratelimit-limit", "10"),
            ("x-ratelimit-remaining", "9"),
            ("x-ratelimit-reset", "1"),
            ("x-tokenlimit-limit", "3333"),
            ("x-tokenlimit-remaining", "3000"),
        ]);
        let r = parse_header_map_for(HeaderScheme::TogetherPerSecond, &m, 50);
        let rps = quota(&r, "requests_second");
        assert_eq!(
            (rps.limit, rps.remaining, rps.resets_at),
            (Some(10.0), Some(9.0), Some(51))
        );
        let tps = quota(&r, "tokens_second");
        assert_eq!((tps.limit, tps.remaining), (Some(3333.0), Some(3000.0)));
        // Without token headers only the request limit is reported.
        let r = parse_header_map_for(
            HeaderScheme::TogetherPerSecond,
            &map(&[("x-ratelimit-remaining", "4")]),
            0,
        );
        assert_eq!(r.quotas.len(), 1);
    }

    /// DigitalOcean's documented example: epoch-second resets, `0` = none
    /// pending.
    #[test]
    fn digitalocean_epoch_resets() {
        let now = 1_789_592_400;
        let m = map(&[
            ("x-ratelimit-limit-requests", "1201"),
            ("x-ratelimit-remaining-requests", "1200"),
            ("x-ratelimit-reset-requests", "1789592460"),
            ("x-ratelimit-limit-tokens-per-minute", "15000000"),
            ("x-ratelimit-remaining-tokens-per-minute", "15000000"),
            ("x-ratelimit-reset-tokens-per-minute", "0"),
            ("x-ratelimit-limit-tokens-per-day", "100000000"),
            ("x-ratelimit-remaining-tokens-per-day", "99000000"),
            ("x-ratelimit-reset-tokens-per-day", "0"),
        ]);
        let r = parse_header_map_for(HeaderScheme::DigitalOcean, &m, now);
        assert_eq!(quota(&r, "requests_minute").resets_at, Some(1_789_592_460));
        assert_eq!(quota(&r, "tokens_minute").resets_at, None);
        assert_eq!(quota(&r, "tokens_day").remaining, Some(99_000_000.0));
        assert_eq!(quota(&r, "tokens_day").resets_at, None);
        assert_eq!(r.quotas.len(), 3);
    }

    /// LLM Gateway's documented headers: the structured fields win over the
    /// legacy triples (which spend-cap 429s overwrite with dollar caps);
    /// concurrency is not a rate.
    #[test]
    fn ietf_structured_fields() {
        let now = 1_000;
        let m = map(&[
            (
                "ratelimit-policy",
                r#""requests";q=600;w=60, "concurrency";q=100;qu="concurrent-requests""#,
            ),
            ("ratelimit", r#""requests";r=599;t=60, "concurrency";r=99"#),
            ("ratelimit-limit", "25"),
            ("ratelimit-remaining", "0"),
            ("ratelimit-reset", "60"),
            ("x-ratelimit-limit", "600"),
            ("x-ratelimit-remaining", "599"),
            ("x-ratelimit-reset", "1789592460"),
        ]);
        let r = parse_header_map_for(HeaderScheme::IetfStructured, &m, now);
        assert_eq!(r.quotas.len(), 1, "{:?}", r.quotas);
        let rpm = quota(&r, "requests_minute");
        assert_eq!(
            (rpm.limit, rpm.remaining, rpm.resets_at),
            (Some(600.0), Some(599.0), Some(now + 60))
        );
        // A window with no period name keeps the bare metric.
        let m = map(&[("ratelimit-policy", r#""burst";q=50;w=10"#)]);
        let r = parse_header_map_for(HeaderScheme::IetfStructured, &m, now);
        assert_eq!(quota(&r, "requests").limit, Some(50.0));
        assert_eq!(quota(&r, "requests").remaining, None);
        // Garbage does not panic or produce readings.
        let m = map(&[
            ("ratelimit-policy", ";;,=,\"\";q=x;w=-1"),
            ("ratelimit", "\""),
        ]);
        assert!(parse_header_map_for(HeaderScheme::IetfStructured, &m, now)
            .quotas
            .is_empty());
    }

    /// Anthropic's Priority Tier and fast-mode limits sit next to the
    /// standard ones.
    #[test]
    fn anthropic_priority_and_fast_limits() {
        let m = map(&[
            ("anthropic-ratelimit-requests-limit", "4000"),
            ("anthropic-ratelimit-requests-remaining", "3999"),
            ("anthropic-ratelimit-requests-reset", "2026-10-11T12:00:01Z"),
            ("anthropic-priority-input-tokens-limit", "100000"),
            ("anthropic-priority-input-tokens-remaining", "99000"),
            ("anthropic-fast-output-tokens-limit", "8000"),
            ("anthropic-fast-output-tokens-remaining", "8000"),
        ]);
        let r = parse_header_map_for(HeaderScheme::Generic, &m, 0);
        assert_eq!(quota(&r, "requests").remaining, Some(3_999.0));
        assert_eq!(quota(&r, "priority_input_tokens").limit, Some(100_000.0));
        assert_eq!(quota(&r, "fast_output_tokens").remaining, Some(8_000.0));
    }
}
