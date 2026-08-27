//! Text formatting shared by the tray title, tooltip and menu usage lines.

use lr_config::{TrayStatsConfig, TrayUsageMetric, TrayUsagePeriod};
use lr_monitoring::metrics::UsageTotals;

/// Format `v` (≥ 0) with a strict two-digit budget — the leading zero
/// counts — choosing the unit so the result reads `0.9`, `1.2`, `12`,
/// `0.1k`, `1.2k`, `12k`, `0.1M`, … (never more than two digits).
fn compact_two_digits(v: f64) -> String {
    // Up to exa so any u64 (≤ 1.8e19) still fits.
    const UNITS: [&str; 7] = ["", "k", "M", "B", "T", "P", "E"];
    let v = if v.is_finite() { v.max(0.0) } else { 0.0 };

    let mut scaled = v;
    for (i, suffix) in UNITS.iter().enumerate() {
        // One decimal below 10, whole numbers below 100; anything that
        // rounds to 100 or more moves up a unit (99.6 → 0.1k).
        let (rounded, decimals) = if scaled < 9.95 {
            ((scaled * 10.0).round() / 10.0, 1)
        } else {
            (scaled.round(), 0)
        };
        if rounded < 100.0 || i == UNITS.len() - 1 {
            return format!("{:.*}{}", decimals, rounded, suffix);
        }
        scaled /= 1000.0;
    }
    unreachable!("last unit always returns")
}

/// Compact count: exact below 100, otherwise two digits with a unit
/// (`99`, `0.1k`, `1.2k`, `24k`, `0.1M`, `1.2M`).
pub fn compact_number(n: u64) -> String {
    if n < 100 {
        n.to_string()
    } else {
        compact_two_digits(n as f64)
    }
}

/// Compact cost: `$0` when nothing was spent, whole cents below ten cents
/// (`1¢` … `9¢`), otherwise dollars with two digits (`$0.1`, `$0.9`,
/// `$1.2`, `$12`, `$0.1k`).
pub fn compact_cost(usd: f64) -> String {
    let usd = if usd.is_finite() { usd.max(0.0) } else { 0.0 };
    if usd == 0.0 {
        return "$0".to_string();
    }
    if usd < 0.095 {
        return format!("{}¢", (usd * 100.0).round() as u64);
    }
    format!("${}", compact_two_digits(usd))
}

/// The single number shown beside the icon for one item.
pub fn headline_value(usage: &UsageTotals, metric: TrayUsageMetric) -> String {
    match metric {
        TrayUsageMetric::Tokens => compact_number(usage.tokens),
        TrayUsageMetric::Cost => compact_cost(usage.cost_usd),
        TrayUsageMetric::Requests => compact_number(usage.requests),
    }
}

/// Raw magnitude used for the relative usage bar.
pub fn metric_magnitude(usage: &UsageTotals, metric: TrayUsageMetric) -> f64 {
    match metric {
        TrayUsageMetric::Tokens => usage.tokens as f64,
        TrayUsageMetric::Cost => usage.cost_usd,
        TrayUsageMetric::Requests => usage.requests as f64,
    }
}

/// Full usage line for the tray menu / tooltip, e.g.
/// `CLAU   24.1k tok · 31 req · $0.42 · 24h`. The configured usage metric
/// leads; the period the figures cover closes the line.
pub fn usage_line(
    label: &str,
    usage: &UsageTotals,
    metric: TrayUsageMetric,
    period: TrayUsagePeriod,
) -> String {
    let tok = format!("{} tok", compact_number(usage.tokens));
    let req = format!("{} req", compact_number(usage.requests));
    let cost = compact_cost(usage.cost_usd);
    let parts = match metric {
        TrayUsageMetric::Tokens => [tok, req, cost],
        TrayUsageMetric::Cost => [cost, tok, req],
        TrayUsageMetric::Requests => [req, tok, cost],
    };
    format!("{:<4}  {} · {}", label, parts.join(" · "), period.short())
}

/// Header for the tray menu usage section (the period is on every line).
pub fn usage_header(_config: &TrayStatsConfig) -> String {
    "Usage".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_numbers_use_two_digits() {
        assert_eq!(compact_number(0), "0");
        assert_eq!(compact_number(7), "7");
        assert_eq!(compact_number(99), "99");
        assert_eq!(compact_number(100), "0.1k");
        assert_eq!(compact_number(843), "0.8k");
        assert_eq!(compact_number(960), "1.0k");
        assert_eq!(compact_number(1_234), "1.2k");
        assert_eq!(compact_number(9_949), "9.9k");
        assert_eq!(compact_number(9_960), "10k");
        assert_eq!(compact_number(24_100), "24k");
        assert_eq!(compact_number(99_600), "0.1M"); // carries into the next unit
        assert_eq!(compact_number(123_456), "0.1M");
        assert_eq!(compact_number(1_200_000), "1.2M");
        assert_eq!(compact_number(3_400_000_000), "3.4B");
        assert_eq!(compact_number(u64::MAX), "18E");
        for n in [
            0u64,
            9,
            99,
            100,
            999,
            9_999,
            99_999,
            999_999,
            9_999_999,
            u64::MAX / 2,
            u64::MAX,
        ] {
            let digits = compact_number(n)
                .chars()
                .filter(|c| c.is_ascii_digit())
                .count();
            assert!(digits <= 2, "{n} → {}", compact_number(n));
        }
    }

    #[test]
    fn compact_costs() {
        assert_eq!(compact_cost(0.0), "$0");
        assert_eq!(compact_cost(0.0042), "0¢");
        assert_eq!(compact_cost(0.01), "1¢");
        assert_eq!(compact_cost(0.094), "9¢");
        assert_eq!(compact_cost(0.095), "$0.1");
        assert_eq!(compact_cost(0.87), "$0.9");
        assert_eq!(compact_cost(0.42), "$0.4");
        assert_eq!(compact_cost(1.26), "$1.3");
        assert_eq!(compact_cost(9.96), "$10");
        assert_eq!(compact_cost(12.3), "$12");
        assert_eq!(compact_cost(123.4), "$0.1k");
        assert_eq!(compact_cost(1234.0), "$1.2k");
        assert_eq!(compact_cost(-5.0), "$0");
    }

    #[test]
    fn usage_line_leads_with_configured_metric() {
        let usage = UsageTotals {
            requests: 31,
            tokens: 24_100,
            cost_usd: 0.42,
        };
        assert_eq!(
            usage_line(
                "CLAU",
                &usage,
                TrayUsageMetric::Tokens,
                TrayUsagePeriod::Day
            ),
            "CLAU  24k tok · 31 req · $0.4 · 24h"
        );
        assert_eq!(
            usage_line("ALL", &usage, TrayUsageMetric::Cost, TrayUsagePeriod::Week),
            "ALL   $0.4 · 24k tok · 31 req · 7d"
        );
        assert_eq!(
            usage_line(
                "GPT5",
                &usage,
                TrayUsageMetric::Requests,
                TrayUsagePeriod::Hour
            ),
            "GPT5  31 req · 24k tok · $0.4 · 1h"
        );
    }
}
