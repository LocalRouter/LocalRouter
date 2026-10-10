//! Usage readings carried in provider response headers.
//!
//! - Claude subscriptions: `anthropic-ratelimit-unified-{5h,7d,…}-utilization`
//!   (0–1 fraction) with matching `-reset` (unix seconds) and `-status`.
//! - ChatGPT (Codex backend): `x-codex-{primary,secondary}-used-percent`,
//!   `-window-minutes`, `-reset-at`, plus `x-codex-credits-*`. Other limit
//!   families use the same shape under their own prefix (`x-codex-other-…`).
//! - API keys: `x-ratelimit-{limit,remaining,reset}-<what>` (OpenAI, Groq,
//!   Cerebras, …) and `anthropic-ratelimit-<what>-{limit,remaining,reset}`.

use std::collections::BTreeMap;

use http::HeaderMap;

use crate::types::{
    id_fragment, window_label, window_secs_for, CreditsReading, QuotaReading, UsageReport,
    WindowReading,
};

const UNIFIED: &str = "anthropic-ratelimit-unified-";

/// Parse every usage reading in a response's headers. `now` is unix seconds,
/// used to resolve relative reset times.
pub fn parse_headers(headers: &HeaderMap, now: i64) -> UsageReport {
    let map: BTreeMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| {
            Some((
                k.as_str().to_ascii_lowercase(),
                v.to_str().ok()?.trim().to_string(),
            ))
        })
        .collect();
    parse_header_map(&map, now)
}

/// [`parse_headers`] over lowercase name → value pairs.
pub fn parse_header_map(map: &BTreeMap<String, String>, now: i64) -> UsageReport {
    let mut report = UsageReport::default();
    parse_anthropic_unified(map, &mut report);
    parse_codex(map, &mut report);
    report.quotas = parse_quotas(map, now);
    report
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

fn parse_codex(map: &BTreeMap<String, String>, report: &mut UsageReport) {
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
            .map(|m| (m * 60.0) as i64);
        let resets_at = map
            .get(&format!("{prefix}-reset-at"))
            .and_then(|v| parse_epoch(v))
            .or_else(|| {
                let after = map
                    .get(&format!("{prefix}-reset-after-seconds"))?
                    .parse::<f64>()
                    .ok()?;
                Some(chrono::Utc::now().timestamp() + after as i64)
            });
        let (id, label) = codex_window(limit, slot, minutes);
        report.windows.push(WindowReading {
            id,
            label,
            used_percent: used,
            resets_at,
            window_secs: minutes,
        });
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
        } else if let Some(rest) = name.strip_prefix("anthropic-ratelimit-") {
            if rest.starts_with("unified-") {
                continue;
            }
            for field in ["limit", "remaining", "reset"] {
                if let Some(what) = rest.strip_suffix(&format!("-{field}")) {
                    set(id_fragment(what), field, value);
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
            Some(now + n.ceil() as i64)
        };
    }
    if let Some(secs) = parse_go_duration(v) {
        return Some(now + secs.ceil() as i64);
    }
    parse_epoch(v)
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
}
