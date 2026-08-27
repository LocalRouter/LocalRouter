//! Text formatting shared by the tray title, tooltip and menu usage lines.

use lr_config::{TrayStatsConfig, TrayUsageMetric, TrayUsagePeriod};
use lr_monitoring::metrics::UsageTotals;

/// Significant digits kept in compact values.
const SIG_DIGITS: i32 = 2;

/// Round `x` (> 0) to `SIG_DIGITS` significant digits.
fn round_sig(x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let magnitude = x.log10().floor() as i32;
    let factor = 10f64.powi(SIG_DIGITS - 1 - magnitude);
    (x * factor).round() / factor
}

/// Decimals needed to show `SIG_DIGITS` significant digits of `r` (≥ 0).
fn decimals_for(r: f64) -> usize {
    if r <= 0.0 {
        return 0;
    }
    let magnitude = r.log10().floor() as i32;
    (SIG_DIGITS - 1 - magnitude).max(0) as usize
}

/// Format `v` (≥ 0) with `SIG_DIGITS` significant digits and a unit
/// suffix (`k`, `M`, `B`, `T`, …) so the result never exceeds 4
/// characters (`999`, `1.2k`, `12k`, `120k`, `1.0M`).
fn compact_float(v: f64) -> String {
    // Up to exa so any u64 (≤ 1.8e19) still fits.
    const UNITS: [(f64, &str); 6] = [
        (1e18, "E"),
        (1e15, "P"),
        (1e12, "T"),
        (1e9, "B"),
        (1e6, "M"),
        (1e3, "k"),
    ];
    let v = if v.is_finite() { v.max(0.0) } else { 0.0 };

    for (div, suffix) in UNITS {
        let scaled = round_sig(v / div);
        // Rounding carries into this unit from below (996k → 1.0M) and
        // out of it above (996M → 1.0B, handled by the larger unit).
        if (1.0..1000.0).contains(&scaled) {
            return format!("{:.*}{}", decimals_for(scaled), scaled, suffix);
        }
    }
    let r = round_sig(v);
    format!("{:.*}", decimals_for(r), r)
}

/// Compact count: exact below 1000, else `SIG_DIGITS` significant digits
/// with a unit (`999`, `1.2k`, `24k`, `1.2M`).
pub fn compact_number(n: u64) -> String {
    if n < 1000 {
        n.to_string()
    } else {
        compact_float(n as f64)
    }
}

/// Compact cost: `$0` when nothing was spent, otherwise `SIG_DIGITS`
/// significant digits (`$0.42`, `$0.0042`, `$1.2`, `$12`, `$120`, `$1.2k`).
pub fn compact_cost(usd: f64) -> String {
    let usd = if usd.is_finite() { usd.max(0.0) } else { 0.0 };
    if usd == 0.0 {
        return "$0".to_string();
    }
    if usd < 1.0 {
        let r = round_sig(usd);
        if r >= 1.0 {
            return format!("${}", compact_float(r));
        }
        return format!("${:.*}", decimals_for(r), r);
    }
    format!("${}", compact_float(usd))
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
    fn compact_numbers_have_two_significant_digits() {
        assert_eq!(compact_number(0), "0");
        assert_eq!(compact_number(999), "999");
        assert_eq!(compact_number(1_000), "1.0k");
        assert_eq!(compact_number(1_234), "1.2k");
        assert_eq!(compact_number(24_100), "24k");
        assert_eq!(compact_number(123_456), "120k");
        assert_eq!(compact_number(996_000), "1.0M"); // carries into the next unit
        assert_eq!(compact_number(1_200_000), "1.2M");
        assert_eq!(compact_number(3_400_000_000), "3.4B");
        assert_eq!(compact_number(9_960), "10k"); // no "10.0k"
        assert_eq!(compact_number(u64::MAX), "18E");
        for n in [
            0u64,
            9,
            99,
            999,
            9_999,
            99_999,
            999_999,
            9_999_999,
            u64::MAX / 2,
            u64::MAX,
        ] {
            assert!(compact_number(n).len() <= 4, "{n} → {}", compact_number(n));
        }
    }

    #[test]
    fn compact_costs() {
        assert_eq!(compact_cost(0.0), "$0");
        assert_eq!(compact_cost(0.4211), "$0.42");
        assert_eq!(compact_cost(0.0042), "$0.0042");
        assert_eq!(compact_cost(0.996), "$1.0");
        assert_eq!(compact_cost(1.26), "$1.3");
        assert_eq!(compact_cost(9.99), "$10");
        assert_eq!(compact_cost(12.3), "$12");
        assert_eq!(compact_cost(123.4), "$120");
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
            "CLAU  24k tok · 31 req · $0.42 · 24h"
        );
        assert_eq!(
            usage_line("ALL", &usage, TrayUsageMetric::Cost, TrayUsagePeriod::Week),
            "ALL   $0.42 · 24k tok · 31 req · 7d"
        );
        assert_eq!(
            usage_line(
                "GPT5",
                &usage,
                TrayUsageMetric::Requests,
                TrayUsagePeriod::Hour
            ),
            "GPT5  31 req · 24k tok · $0.42 · 1h"
        );
    }
}
