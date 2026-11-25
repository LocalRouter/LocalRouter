//! Vendor colors for the tray icon's color mode.
//!
//! In [`TrayColorMode::Color`](lr_config::TrayColorMode) request graphs are
//! stacked by vendor and usage gauges are filled in their vendor's color.
//! Only vendors listed here get a color; everything else keeps the
//! foreground (menu-bar) tint.

use image::Rgba;
use lr_config::{ProviderConfig, ProviderType, TraySource};

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

    pub fn color(self) -> Rgba<u8> {
        match self {
            // Claude orange (#D97757)
            TrayVendor::Anthropic => Rgba([217, 119, 87, 255]),
            // Blue (#3B82F6), legible on light and dark menu bars
            TrayVendor::OpenAI => Rgba([59, 130, 246, 255]),
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

/// Vendor of the provider instance named `instance`.
pub fn provider_vendor(providers: &[ProviderConfig], instance: &str) -> Option<TrayVendor> {
    providers
        .iter()
        .find(|p| p.name == instance)
        .and_then(|p| TrayVendor::of_provider_type(&p.provider_type))
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

/// Provider instances whose traffic is stacked as `vendor` in the `All`
/// graph (enabled or not — disabled providers may still have recent traffic).
pub fn vendor_instances(providers: &[ProviderConfig], vendor: TrayVendor) -> Vec<String> {
    providers
        .iter()
        .filter(|p| TrayVendor::of_provider_type(&p.provider_type) == Some(vendor))
        .map(|p| p.name.clone())
        .collect()
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
            vec!["chatgpt".to_string(), "oai".to_string()]
        );
    }
}
