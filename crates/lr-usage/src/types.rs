//! Readings parsed from provider responses, and the account identity they
//! belong to.

use serde::{Deserialize, Serialize};

/// Whether an account is a flat-rate subscription or pay-as-you-go API usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    Subscription,
    Api,
}

impl AccountKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AccountKind::Subscription => "subscription",
            AccountKind::Api => "api",
        }
    }
}

/// A usage account: one provider family billed one way. Several LocalRouter
/// providers and proxied tools can feed the same account (Claude Code through
/// the proxy and a LocalRouter Claude provider share one Claude subscription).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountRef {
    /// Provider family, e.g. `anthropic`, `openai`, `github-copilot`.
    pub provider: String,
    pub kind: AccountKind,
}

impl AccountRef {
    pub fn new(provider: impl Into<String>, kind: AccountKind) -> Self {
        Self {
            provider: provider.into(),
            kind,
        }
    }

    pub fn subscription(provider: impl Into<String>) -> Self {
        Self::new(provider, AccountKind::Subscription)
    }

    pub fn api(provider: impl Into<String>) -> Self {
        Self::new(provider, AccountKind::Api)
    }

    /// Stable id, e.g. `anthropic:subscription`.
    pub fn id(&self) -> String {
        format!("{}:{}", self.provider, self.kind.as_str())
    }

    /// Parse an id produced by [`AccountRef::id`].
    pub fn from_id(id: &str) -> Option<Self> {
        let (provider, kind) = id.rsplit_once(':')?;
        let kind = match kind {
            "subscription" => AccountKind::Subscription,
            "api" => AccountKind::Api,
            _ => return None,
        };
        (!provider.is_empty()).then(|| Self::new(provider, kind))
    }
}

/// Where a reading came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DataSource {
    /// Rate-limit headers on traffic through the HTTPS inspection proxy.
    ProxyHeaders,
    /// A usage response (e.g. Claude Code's `/usage`) seen by the proxy.
    ProxyUsageResponse,
    /// Rate-limit headers on LocalRouter's own provider calls.
    GatewayHeaders,
    /// LocalRouter asked a connected provider's usage endpoint.
    ProviderApi,
    /// LocalRouter asked the usage endpoint with a CLI tool's saved login.
    CliLogin,
}

/// A percentage-of-limit window (e.g. Claude's weekly cap).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowReading {
    /// Stable id: `five_hour`, `seven_day`, `seven_day_fable`, `monthly`, …
    pub id: String,
    /// Human label, e.g. "Weekly".
    pub label: String,
    /// Used share of the limit, 0–100 (may exceed 100 when over).
    pub used_percent: f64,
    /// When the window resets (unix seconds).
    pub resets_at: Option<i64>,
    /// Window length in seconds, when known.
    pub window_secs: Option<i64>,
}

/// A count-based limit from rate-limit headers (requests, tokens, …).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct QuotaReading {
    /// e.g. `requests`, `tokens`, `input_tokens`, `requests_day`.
    pub id: String,
    pub limit: Option<f64>,
    pub remaining: Option<f64>,
    /// Unix seconds.
    pub resets_at: Option<i64>,
}

/// Prepaid credit, balance or overage spend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CreditsReading {
    /// e.g. "Extra usage", "Credits", "Key limit".
    pub label: String,
    pub balance_usd: Option<f64>,
    pub limit_usd: Option<f64>,
    pub used_usd: Option<f64>,
    #[serde(default)]
    pub unlimited: bool,
    /// `USD` when the amounts are dollars; `None` for provider credit units.
    #[serde(default)]
    pub currency: Option<String>,
}

/// Everything one response or usage endpoint told us about an account.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageReport {
    /// Raw plan identifier (e.g. `default_claude_max_20x`, `pro`).
    pub plan: Option<String>,
    pub windows: Vec<WindowReading>,
    pub quotas: Vec<QuotaReading>,
    pub credits: Option<CreditsReading>,
    /// Provider's own verdict: `allowed`, `allowed_warning`, `rejected`.
    pub status: Option<String>,
}

impl UsageReport {
    pub fn is_empty(&self) -> bool {
        self.plan.is_none()
            && self.windows.is_empty()
            && self.quotas.is_empty()
            && self.credits.is_none()
            && self.status.is_none()
    }
}

/// Labels for the well-known window ids.
pub fn window_label(id: &str) -> String {
    match id {
        "five_hour" => "5-hour session".to_string(),
        "seven_day" => "Weekly".to_string(),
        "monthly" => "Monthly".to_string(),
        "extra_usage" => "Extra usage".to_string(),
        other => {
            if let Some(model) = other.strip_prefix("seven_day_") {
                format!("Weekly · {}", title_case(model))
            } else if let Some(model) = other.strip_prefix("five_hour_") {
                format!("5-hour · {}", title_case(model))
            } else {
                title_case(other)
            }
        }
    }
}

/// Typical window lengths for well-known ids.
pub fn window_secs_for(id: &str) -> Option<i64> {
    if id == "five_hour" || id.starts_with("five_hour_") {
        Some(5 * 3600)
    } else if id == "seven_day" || id.starts_with("seven_day_") {
        Some(7 * 86_400)
    } else {
        None
    }
}

fn title_case(s: &str) -> String {
    s.split(['_', '-', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Turn a free-form name into a window-id fragment (`Claude Fable` →
/// `claude_fable`).
pub fn id_fragment(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    out.trim_end_matches('_').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_id_round_trips() {
        let a = AccountRef::subscription("github-copilot");
        assert_eq!(a.id(), "github-copilot:subscription");
        assert_eq!(AccountRef::from_id(&a.id()), Some(a));
        assert_eq!(AccountRef::from_id("openai:other"), None);
        assert_eq!(AccountRef::from_id(":api"), None);
    }

    #[test]
    fn labels_for_known_and_scoped_windows() {
        assert_eq!(window_label("seven_day"), "Weekly");
        assert_eq!(window_label("seven_day_fable"), "Weekly · Fable");
        assert_eq!(window_label("codex_other_primary"), "Codex Other Primary");
        assert_eq!(window_secs_for("seven_day_opus"), Some(604_800));
        assert_eq!(window_secs_for("monthly"), None);
    }

    #[test]
    fn id_fragment_normalizes() {
        assert_eq!(id_fragment("Claude Fable 5"), "claude_fable_5");
        assert_eq!(id_fragment("--x--"), "x");
    }
}
