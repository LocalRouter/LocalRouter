//! Text formatting shared by the tray title, tooltip and menu usage lines.

use lr_config::{TrayStatsConfig, TrayUsageMetric};
use lr_monitoring::metrics::UsageTotals;

/// Compact human number: `999`, `1.2k`, `24k`, `1.2M`, `3.4B`.
pub fn compact_number(n: u64) -> String {
    const UNITS: [(f64, &str); 3] = [(1e9, "B"), (1e6, "M"), (1e3, "k")];
    let v = n as f64;
    for (div, suffix) in UNITS {
        if v >= div {
            let scaled = v / div;
            return if scaled < 10.0 {
                format!("{:.1}{}", scaled, suffix)
            } else {
                format!("{:.0}{}", scaled, suffix)
            };
        }
    }
    n.to_string()
}

/// Compact cost: `$0.42`, `$12.30`, `$1.2k`.
pub fn compact_cost(usd: f64) -> String {
    if usd >= 1000.0 {
        format!("${}", compact_number(usd.round() as u64))
    } else if usd >= 100.0 {
        format!("${:.0}", usd)
    } else {
        format!("${:.2}", usd)
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

/// Text beside the icon: one headline value per item, in panel order.
pub fn title_text(values: &[String]) -> Option<String> {
    if values.is_empty() {
        None
    } else {
        Some(values.join(" · "))
    }
}

/// Header for the tray menu usage section, e.g. `Usage · last 24h`.
pub fn usage_header(config: &TrayStatsConfig) -> String {
    format!("Usage · {}", config.usage_period.label())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_numbers() {
        assert_eq!(compact_number(0), "0");
        assert_eq!(compact_number(999), "999");
        assert_eq!(compact_number(1_000), "1.0k");
        assert_eq!(compact_number(1_234), "1.2k");
        assert_eq!(compact_number(24_100), "24k");
        assert_eq!(compact_number(1_200_000), "1.2M");
        assert_eq!(compact_number(3_400_000_000), "3.4B");
    }

    #[test]
    fn compact_costs() {
        assert_eq!(compact_cost(0.0), "$0.00");
        assert_eq!(compact_cost(0.4211), "$0.42");
        assert_eq!(compact_cost(12.3), "$12.30");
        assert_eq!(compact_cost(123.4), "$123");
        assert_eq!(compact_cost(1234.0), "$1.2k");
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
            "CLAU  24k tok · 31 req · $0.42"
        );
        assert_eq!(
            usage_line("ALL", &usage, TrayUsageMetric::Cost),
            "ALL   $0.42 · 24k tok · 31 req"
        );
        assert_eq!(
            usage_line("GPT5", &usage, TrayUsageMetric::Requests),
            "GPT5  31 req · 24k tok · $0.42"
        );
    }

    #[test]
    fn title_joins_values() {
        assert_eq!(title_text(&[]), None);
        assert_eq!(
            title_text(&["1.2M".into(), "24k".into()]).as_deref(),
            Some("1.2M · 24k")
        );
    }
}
