//! Text formatting shared by the tray title, tooltip and menu usage lines.

use lr_config::{TrayStatsConfig, TrayUsageMetric};
use lr_monitoring::metrics::UsageTotals;

/// Round `v` (≥ 0) to at most 3 significant digits and format it with a
/// unit suffix (`k`, `M`, `B`, `T`, …) so the result never exceeds 5
/// characters (`999`, `1.23k`, `12.3k`, `123k`, `1.00M`).
fn compact_float(v: f64) -> String {
    // Up to exa so any u64 (≤ 1.8e19) still fits in 5 characters.
    const UNITS: [(f64, &str); 6] = [
        (1e18, "E"),
        (1e15, "P"),
        (1e12, "T"),
        (1e9, "B"),
        (1e6, "M"),
        (1e3, "k"),
    ];
    let v = if v.is_finite() { v.max(0.0) } else { 0.0 };

    // Round to 3 significant digits.
    fn round_sig3(x: f64) -> f64 {
        if x == 0.0 {
            return 0.0;
        }
        let magnitude = x.log10().floor() as i32;
        let factor = 10f64.powi(2 - magnitude);
        (x * factor).round() / factor
    }

    fn decimals_for(scaled: f64) -> usize {
        if scaled < 10.0 {
            2
        } else if scaled < 100.0 {
            1
        } else {
            0
        }
    }

    for (div, suffix) in UNITS {
        let scaled = round_sig3(v / div);
        // Rounding carries into this unit from below (999.6k → 1.00M) and
        // out of it above (999.6M → 1.00B, handled by the larger unit).
        if (1.0..1000.0).contains(&scaled) {
            return format!("{:.*}{}", decimals_for(scaled), scaled, suffix);
        }
    }
    let r = round_sig3(v);
    format!("{:.*}", decimals_for(r), r)
}

/// Compact human number with ≤ 3 significant digits: `999`, `1.23k`,
/// `24.1k`, `1.20M`.
pub fn compact_number(n: u64) -> String {
    if n < 1000 {
        n.to_string()
    } else {
        compact_float(n as f64)
    }
}

/// Compact cost with ≤ 3 significant digits: `$0.42`, `$9.99`, `$12.3`,
/// `$123`, `$1.23k`.
pub fn compact_cost(usd: f64) -> String {
    let usd = if usd.is_finite() { usd.max(0.0) } else { 0.0 };
    if usd < 9.995 {
        format!("${:.2}", usd)
    } else {
        format!("${}", compact_float(usd))
    }
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
/// `CLAU   24.1k tok · 31 req · $0.42`. The configured usage metric leads.
pub fn usage_line(label: &str, usage: &UsageTotals, metric: TrayUsageMetric) -> String {
    let tok = format!("{} tok", compact_number(usage.tokens));
    let req = format!("{} req", compact_number(usage.requests));
    let cost = compact_cost(usage.cost_usd);
    let parts = match metric {
        TrayUsageMetric::Tokens => [tok, req, cost],
        TrayUsageMetric::Cost => [cost, tok, req],
        TrayUsageMetric::Requests => [req, tok, cost],
    };
    format!("{:<4}  {}", label, parts.join(" · "))
}

/// Header for the tray menu usage section, e.g. `Usage · last 24h`.
pub fn usage_header(config: &TrayStatsConfig) -> String {
    format!("Usage · {}", config.usage_period.label())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_numbers_have_three_significant_digits() {
        assert_eq!(compact_number(0), "0");
        assert_eq!(compact_number(999), "999");
        assert_eq!(compact_number(1_000), "1.00k");
        assert_eq!(compact_number(1_234), "1.23k");
        assert_eq!(compact_number(24_100), "24.1k");
        assert_eq!(compact_number(123_456), "123k");
        assert_eq!(compact_number(999_600), "1.00M"); // carries into the next unit
        assert_eq!(compact_number(1_200_000), "1.20M");
        assert_eq!(compact_number(3_400_000_000), "3.40B");
        assert_eq!(compact_number(9_996), "10.0k"); // no "10.00k"
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
        ] {
            assert!(compact_number(n).len() <= 5, "{n} → {}", compact_number(n));
        }
    }

    #[test]
    fn compact_costs() {
        assert_eq!(compact_cost(0.0), "$0.00");
        assert_eq!(compact_cost(0.4211), "$0.42");
        assert_eq!(compact_cost(9.999), "$10.0"); // never "$10.00"
        assert_eq!(compact_cost(12.3), "$12.3");
        assert_eq!(compact_cost(123.4), "$123");
        assert_eq!(compact_cost(1234.0), "$1.23k");
        assert_eq!(compact_cost(-5.0), "$0.00");
    }

    #[test]
    fn usage_line_leads_with_configured_metric() {
        let usage = UsageTotals {
            requests: 31,
            tokens: 24_100,
            cost_usd: 0.42,
        };
        assert_eq!(
            usage_line("CLAU", &usage, TrayUsageMetric::Tokens),
            "CLAU  24.1k tok · 31 req · $0.42"
        );
        assert_eq!(
            usage_line("ALL", &usage, TrayUsageMetric::Cost),
            "ALL   $0.42 · 24.1k tok · 31 req"
        );
        assert_eq!(
            usage_line("GPT5", &usage, TrayUsageMetric::Requests),
            "GPT5  31 req · 24.1k tok · $0.42"
        );
    }
}
