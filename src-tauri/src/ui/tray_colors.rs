//! Colors for the tray icon's color mode.
//!
//! In [`TrayColorMode::Color`](lr_config::TrayColorMode) every provider has
//! a color: the user's pick, or a built-in one (Anthropic orange, OpenAI
//! blue, and a fixed palette for everything else). The `All` graph stacks
//! one layer per provider; provider, model and usage panes take their
//! provider's color; any item can carry its own color that overrides that.

use std::collections::BTreeMap;

use image::Rgba;
use lr_config::{parse_hex_color, ProviderConfig, ProviderType, TraySource, TrayStatsItem};

/// A vendor with a fixed built-in color and a fixed place in the stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrayVendor {
    Anthropic,
    OpenAI,
}

impl TrayVendor {
    /// Stacking order in the `All` graph, bottom first. Other providers
    /// stack above these, by name.
    pub const STACK_ORDER: [TrayVendor; 2] = [TrayVendor::Anthropic, TrayVendor::OpenAI];

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
}

/// Built-in colors for providers that aren't Anthropic or OpenAI, picked
/// by a stable hash of the provider name. Mid-tones that read on light and
/// dark menu bars; no orange or blue, which belong to the vendors above.
const OTHER_COLORS: [[u8; 3]; 8] = [
    [34, 197, 94],   // green
    [168, 85, 247],  // purple
    [236, 72, 153],  // pink
    [20, 184, 166],  // teal
    [234, 179, 8],   // yellow
    [239, 68, 68],   // red
    [132, 204, 22],  // lime
    [148, 163, 184], // slate
];

fn stable_hash(name: &str) -> u64 {
    // FNV-1a: stable across runs and platforms, unlike `DefaultHasher`.
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Format a color as `#rrggbb`.
pub fn to_hex(color: Rgba<u8>) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

fn parse_color(value: &str) -> Option<Rgba<u8>> {
    parse_hex_color(value).map(|[r, g, b]| Rgba([r, g, b, 255]))
}

/// The colors in effect for one render.
#[derive(Debug, Clone, Copy)]
pub struct TrayPalette<'a> {
    providers: &'a [ProviderConfig],
    overrides: &'a BTreeMap<String, String>,
}

impl<'a> TrayPalette<'a> {
    pub fn new(providers: &'a [ProviderConfig], overrides: &'a BTreeMap<String, String>) -> Self {
        Self {
            providers,
            overrides,
        }
    }

    /// Vendor of the provider recorded as `name`: a configured provider by
    /// its type, otherwise the proxy's own provider names.
    pub fn vendor(&self, name: &str) -> Option<TrayVendor> {
        match self.providers.iter().find(|p| p.name == name) {
            Some(p) => TrayVendor::of_provider_type(&p.provider_type),
            None => TrayVendor::STACK_ORDER
                .into_iter()
                .find(|v| v.proxy_provider() == name),
        }
    }

    /// Built-in color of the provider recorded as `name`.
    pub fn default_color(&self, name: &str) -> Rgba<u8> {
        match self.vendor(name) {
            Some(vendor) => vendor.default_color(),
            None => {
                let i = (stable_hash(name) % OTHER_COLORS.len() as u64) as usize;
                let [r, g, b] = OTHER_COLORS[i];
                Rgba([r, g, b, 255])
            }
        }
    }

    /// Color of the provider recorded as `name`: the user's pick, else the
    /// built-in color.
    pub fn provider_color(&self, name: &str) -> Rgba<u8> {
        self.overrides
            .get(name)
            .and_then(|v| parse_color(v))
            .unwrap_or_else(|| self.default_color(name))
    }

    /// Provider names stacked in the `All` graph, bottom first: Anthropic
    /// providers, then OpenAI providers, then the rest by name. Covers
    /// every configured provider (enabled or not — disabled providers may
    /// still have recent traffic) and the proxy's names, unless a
    /// configured provider already uses one.
    pub fn stack_providers(&self) -> Vec<String> {
        let mut names: Vec<String> = self.providers.iter().map(|p| p.name.clone()).collect();
        for vendor in TrayVendor::STACK_ORDER {
            let proxy = vendor.proxy_provider();
            if !names.iter().any(|n| n == proxy) {
                names.push(proxy.to_string());
            }
        }
        let rank = |name: &str| match self.vendor(name) {
            Some(v) => TrayVendor::STACK_ORDER
                .iter()
                .position(|o| *o == v)
                .unwrap_or(0),
            None => TrayVendor::STACK_ORDER.len(),
        };
        names.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
        names.dedup();
        names
    }

    /// The provider a source belongs to, when it has exactly one: a
    /// provider, a model (`{provider}/{model_id}`) or a usage window
    /// (account `{provider}:{kind}`). `All` and clients mix providers.
    pub fn source_provider(&self, source: &TraySource) -> Option<String> {
        match source {
            TraySource::Provider { instance } => Some(instance.clone()),
            TraySource::Model { id } => self
                .providers
                .iter()
                .map(|p| p.name.as_str())
                .chain(TrayVendor::STACK_ORDER.iter().map(|v| v.proxy_provider()))
                .find(|name| {
                    id.strip_prefix(name)
                        .is_some_and(|rest| rest.starts_with('/'))
                })
                .map(str::to_string),
            TraySource::Usage { account, .. } => {
                lr_usage::types::AccountRef::from_id(account).map(|a| a.provider)
            }
            TraySource::All | TraySource::Client { .. } => None,
        }
    }

    /// Color of a tray item's panel: the item's own color, else its
    /// provider's color. `None` leaves `All` stacked and clients in the
    /// menu-bar tint.
    pub fn item_color(&self, item: &TrayStatsItem) -> Option<Rgba<u8>> {
        item.color.as_deref().and_then(parse_color).or_else(|| {
            self.source_provider(&item.source)
                .map(|name| self.provider_color(&name))
        })
    }
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

    fn providers() -> Vec<ProviderConfig> {
        vec![
            provider("groq", ProviderType::Groq),
            provider("claude", ProviderType::Anthropic),
            provider("chatgpt", ProviderType::ChatGPTPlus),
            provider("Ollama", ProviderType::Ollama),
        ]
    }

    #[test]
    fn stack_order_is_anthropic_openai_then_rest() {
        let providers = providers();
        let overrides = BTreeMap::new();
        let palette = TrayPalette::new(&providers, &overrides);
        assert_eq!(
            palette.stack_providers(),
            vec!["anthropic", "claude", "chatgpt", "openai", "Ollama", "groq"]
        );
        // A configured provider named like the proxy isn't doubled
        let named = vec![provider("anthropic", ProviderType::Anthropic)];
        let palette = TrayPalette::new(&named, &overrides);
        assert_eq!(palette.stack_providers(), vec!["anthropic", "openai"]);
    }

    #[test]
    fn colors_default_by_vendor_and_honor_overrides() {
        let providers = providers();
        let mut overrides = BTreeMap::new();
        overrides.insert("groq".to_string(), "#112233".to_string());
        overrides.insert("chatgpt".to_string(), "bogus".to_string());
        let palette = TrayPalette::new(&providers, &overrides);
        assert_eq!(
            palette.provider_color("claude"),
            TrayVendor::Anthropic.default_color()
        );
        // Proxy traffic (Claude Code, Codex) is recorded under these names
        assert_eq!(
            palette.provider_color("anthropic"),
            TrayVendor::Anthropic.default_color()
        );
        assert_eq!(
            palette.provider_color("openai"),
            TrayVendor::OpenAI.default_color()
        );
        assert_eq!(
            palette.provider_color("chatgpt"),
            TrayVendor::OpenAI.default_color()
        );
        assert_eq!(
            palette.provider_color("groq"),
            Rgba([0x11, 0x22, 0x33, 255])
        );
        // Other providers get a stable palette color, never the vendors'
        let ollama = palette.provider_color("Ollama");
        assert_eq!(ollama, palette.provider_color("Ollama"));
        assert_ne!(ollama, TrayVendor::Anthropic.default_color());
        assert_ne!(ollama, TrayVendor::OpenAI.default_color());
        assert_eq!(to_hex(Rgba([217, 119, 87, 255])), "#d97757");
    }

    #[test]
    fn item_colors() {
        let providers = providers();
        let overrides = BTreeMap::new();
        let palette = TrayPalette::new(&providers, &overrides);
        let item = TrayStatsItem::new;
        assert_eq!(palette.item_color(&item(TraySource::All)), None);
        assert_eq!(
            palette.item_color(&item(TraySource::Client { id: "c".into() })),
            None
        );
        assert_eq!(
            palette.item_color(&item(TraySource::Model {
                id: "claude/opus".into()
            })),
            Some(TrayVendor::Anthropic.default_color())
        );
        assert_eq!(
            palette.item_color(&item(TraySource::Usage {
                account: "openai:subscription:abc".into(),
                window: "seven_day".into(),
            })),
            Some(TrayVendor::OpenAI.default_color())
        );
        // Accounts of providers LocalRouter doesn't know still get a color
        assert!(palette
            .item_color(&item(TraySource::Usage {
                account: "github-copilot:subscription".into(),
                window: "monthly".into(),
            }))
            .is_some());
        // An item's own color wins, for clients too
        let mut client = item(TraySource::Client { id: "c".into() });
        client.color = Some("#00ff00".into());
        assert_eq!(palette.item_color(&client), Some(Rgba([0, 255, 0, 255])));
    }
}
