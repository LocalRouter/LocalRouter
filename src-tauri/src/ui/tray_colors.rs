//! Vendor colors for the tray icon's color mode.
//!
//! In [`TrayColorMode::Color`](lr_config::TrayColorMode) request graphs are
//! stacked by vendor and usage gauges are filled in their vendor's color.
//! Only vendors listed here get a color; everything else keeps the
//! foreground (menu-bar) tint.

use image::Rgba;
use lr_config::{parse_hex_color, ProviderConfig, ProviderType, TraySource, TrayVendorColors};

/// A vendor with its own tray color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrayVendor {
    Anthropic,
    OpenAI,
}

impl TrayVendor {
    /// Stacking order in request graphs, bottom first. Traffic from any
    /// other vendor is stacked on top in the foreground tint.
    pub const STACK_ORDER: [TrayVendor; 2] = [TrayVendor::Anthropic, TrayVendor::OpenAI];

    /// Built-in color, used unless the user picked another.
    pub fn default_color(self) -> Rgba<u8> {
        match self {
            // Claude orange (#D97757)
            TrayVendor::Anthropic => Rgba([217, 119, 87, 255]),
            // Blue (#3B82F6), legible on light and dark menu bars
            TrayVendor::OpenAI => Rgba([59, 130, 246, 255]),
        }
    }

    /// Provider name the HTTPS inspection proxy records this vendor's
    /// traffic under (Claude Code, Codex, … going through the proxy).
    pub fn proxy_provider(self) -> &'static str {
        match self {
            TrayVendor::Anthropic => lr_proxy::wire::ANTHROPIC_PROVIDER,
            TrayVendor::OpenAI => lr_proxy::wire::OPENAI_PROVIDER,
        }
    }

    pub fn of_provider_type(provider_type: &ProviderType) -> Option<Self> {
        match provider_type {
            ProviderType::Anthropic => Some(TrayVendor::Anthropic),
            ProviderType::OpenAI | ProviderType::ChatGPTPlus => Some(TrayVendor::OpenAI),
            _ => None,
        }
    }

    /// Vendor of a usage-tracker account id such as `anthropic:subscription`.
    pub fn of_usage_account(account: &str) -> Option<Self> {
        match lr_usage::types::AccountRef::from_id(account)?
            .provider
            .as_str()
        {
            "anthropic" => Some(TrayVendor::Anthropic),
            "openai" => Some(TrayVendor::OpenAI),
            _ => None,
        }
    }
}

/// Vendor colors in effect: the user's picks over the built-in colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VendorPalette {
    anthropic: Rgba<u8>,
    openai: Rgba<u8>,
}

impl VendorPalette {
    pub fn new(colors: &TrayVendorColors) -> Self {
        let pick = |value: &Option<String>, vendor: TrayVendor| {
            value
                .as_deref()
                .and_then(parse_hex_color)
                .map(|[r, g, b]| Rgba([r, g, b, 255]))
                .unwrap_or_else(|| vendor.default_color())
        };
        Self {
            anthropic: pick(&colors.anthropic, TrayVendor::Anthropic),
            openai: pick(&colors.openai, TrayVendor::OpenAI),
        }
    }

    pub fn color(&self, vendor: TrayVendor) -> Rgba<u8> {
        match vendor {
            TrayVendor::Anthropic => self.anthropic,
            TrayVendor::OpenAI => self.openai,
        }
    }
}

impl Default for VendorPalette {
    fn default() -> Self {
        Self::new(&TrayVendorColors::default())
    }
}

/// Vendor of the provider recorded as `instance`: a configured provider by
/// its type, otherwise the proxy's own provider names.
pub fn provider_vendor(providers: &[ProviderConfig], instance: &str) -> Option<TrayVendor> {
    match providers.iter().find(|p| p.name == instance) {
        Some(p) => TrayVendor::of_provider_type(&p.provider_type),
        None => TrayVendor::STACK_ORDER
            .into_iter()
            .find(|v| v.proxy_provider() == instance),
    }
}

/// The single vendor a tray source belongs to, when it has one: a provider
/// instance, a model (`{provider_instance}/{model_id}`) or a usage window.
/// `All` and clients mix vendors and return `None`.
pub fn source_vendor(source: &TraySource, providers: &[ProviderConfig]) -> Option<TrayVendor> {
    match source {
        TraySource::Provider { instance } => provider_vendor(providers, instance),
        TraySource::Model { id } => providers
            .iter()
            .find(|p| {
                id.strip_prefix(p.name.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
            })
            .and_then(|p| TrayVendor::of_provider_type(&p.provider_type)),
        TraySource::Usage { account, .. } => TrayVendor::of_usage_account(account),
        TraySource::All | TraySource::Client { .. } => None,
    }
}

/// Provider names whose traffic is stacked as `vendor` in the `All` graph:
/// configured providers of the vendor (enabled or not — disabled providers
/// may still have recent traffic) and the proxy's name for it, unless a
/// configured provider of another kind already uses that name.
pub fn vendor_instances(providers: &[ProviderConfig], vendor: TrayVendor) -> Vec<String> {
    let mut names: Vec<String> = providers
        .iter()
        .filter(|p| TrayVendor::of_provider_type(&p.provider_type) == Some(vendor))
        .map(|p| p.name.clone())
        .collect();
    let proxy = vendor.proxy_provider();
    if !providers.iter().any(|p| p.name == proxy) {
        names.push(proxy.to_string());
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(name: &str, provider_type: ProviderType) -> ProviderConfig {
        ProviderConfig {
            name: name.into(),
            provider_type,
            ..ProviderConfig::default_ollama()
        }
    }

    #[test]
    fn vendors_of_sources() {
        let providers = vec![
            provider("claude", ProviderType::Anthropic),
            provider("chatgpt", ProviderType::ChatGPTPlus),
            provider("oai", ProviderType::OpenAI),
            provider("groq", ProviderType::Groq),
        ];
        let p = |i: &str| TraySource::Provider { instance: i.into() };
        let m = |i: &str| TraySource::Model { id: i.into() };
        assert_eq!(
            source_vendor(&p("claude"), &providers),
            Some(TrayVendor::Anthropic)
        );
        assert_eq!(
            source_vendor(&p("chatgpt"), &providers),
            Some(TrayVendor::OpenAI)
        );
        assert_eq!(source_vendor(&p("groq"), &providers), None);
        assert_eq!(source_vendor(&p("missing"), &providers), None);
        // Proxy traffic (Claude Code, Codex) is recorded under these names
        assert_eq!(
            source_vendor(&p("anthropic"), &providers),
            Some(TrayVendor::Anthropic)
        );
        assert_eq!(
            source_vendor(&p("openai"), &providers),
            Some(TrayVendor::OpenAI)
        );
        assert_eq!(
            source_vendor(&m("oai/gpt-5"), &providers),
            Some(TrayVendor::OpenAI)
        );
        // A provider name that is only a prefix of another doesn't match
        assert_eq!(source_vendor(&m("claudex/foo"), &providers), None);
        assert_eq!(source_vendor(&TraySource::All, &providers), None);
        let usage = |a: &str| TraySource::Usage {
            account: a.into(),
            window: "seven_day".into(),
        };
        assert_eq!(
            source_vendor(&usage("anthropic:subscription"), &providers),
            Some(TrayVendor::Anthropic)
        );
        assert_eq!(
            source_vendor(&usage("openai:subscription:abc123"), &providers),
            Some(TrayVendor::OpenAI)
        );
        assert_eq!(
            source_vendor(&usage("github-copilot:subscription"), &providers),
            None
        );
        assert_eq!(
            vendor_instances(&providers, TrayVendor::OpenAI),
            vec![
                "chatgpt".to_string(),
                "oai".to_string(),
                "openai".to_string()
            ]
        );
        assert_eq!(
            vendor_instances(&[], TrayVendor::Anthropic),
            vec!["anthropic".to_string()]
        );
        // A configured provider already named like the proxy isn't doubled
        let named = vec![provider("anthropic", ProviderType::Anthropic)];
        assert_eq!(
            vendor_instances(&named, TrayVendor::Anthropic),
            vec!["anthropic".to_string()]
        );
    }

    #[test]
    fn palette_uses_picks_over_defaults() {
        let palette = VendorPalette::new(&TrayVendorColors {
            anthropic: Some("#112233".into()),
            openai: Some("not a color".into()),
        });
        assert_eq!(
            palette.color(TrayVendor::Anthropic),
            Rgba([0x11, 0x22, 0x33, 255])
        );
        assert_eq!(
            palette.color(TrayVendor::OpenAI),
            TrayVendor::OpenAI.default_color()
        );
    }
}
