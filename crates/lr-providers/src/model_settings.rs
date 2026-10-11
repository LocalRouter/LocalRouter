//! Per-provider model settings applied on top of a provider's listing.
//!
//! The effective model list of a provider instance is its discovered models
//! plus its manual entries, with per-model overrides applied, then filtered
//! by visibility. This module is the pure part of that: no I/O, no locks.

use lr_config::{ModelEntry, ProviderModelSettings};
use serde::{Deserialize, Serialize};

use crate::{Capability, ModelInfo};

/// Context window assumed for a manual model nothing else describes.
pub const DEFAULT_CONTEXT_WINDOW: u32 = 4096;

/// Where a managed model comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    /// Listed by the provider
    Discovered,
    /// Added by hand, not listed by the provider
    Manual,
    /// Added by hand and also listed by the provider
    ManualAndDiscovered,
    /// Has settings but is neither listed nor manual (kept so its settings
    /// are not lost while the provider does not list it)
    Missing,
}

/// One model of a provider instance with its settings applied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedModel {
    /// The model as offered: overrides applied
    pub info: ModelInfo,
    pub source: ModelSource,
    /// Whether it is offered (in `/v1/models`, pickers and routing)
    pub enabled: bool,
    /// Whether the user changed its name, context, capabilities or price
    pub has_overrides: bool,
}

impl ManagedModel {
    /// Whether this model is part of the effective list.
    pub fn is_offered(&self) -> bool {
        self.enabled && self.source != ModelSource::Missing
    }
}

/// Whether discovered models without an entry are shown.
pub fn shows_new_models(settings: &ProviderModelSettings, new_models_hidden: bool) -> bool {
    settings.show_new_models.unwrap_or(!new_models_hidden)
}

/// Apply `settings` to the `discovered` models of provider `instance`.
///
/// Order: discovered models in listing order, then manual models that the
/// provider does not list, then entries for models that are gone.
pub fn apply(
    discovered: &[ModelInfo],
    settings: &ProviderModelSettings,
    instance: &str,
    new_models_hidden: bool,
) -> Vec<ManagedModel> {
    let show_new = shows_new_models(settings, new_models_hidden);
    let mut out = Vec::with_capacity(discovered.len() + settings.entries.len());
    let mut seen = std::collections::HashSet::new();

    for model in discovered {
        if !seen.insert(model.id.as_str()) {
            continue; // a listing that repeats an id
        }
        let entry = settings.entry(&model.id);
        let mut info = model.clone();
        info.provider = instance.to_string();
        let (source, enabled, has_overrides) = match entry {
            Some(e) => {
                apply_overrides(&mut info, e);
                let source = if e.manual {
                    ModelSource::ManualAndDiscovered
                } else {
                    ModelSource::Discovered
                };
                // A model the user added by hand is shown unless hidden.
                let default = show_new || e.manual;
                (source, e.enabled.unwrap_or(default), e.has_overrides())
            }
            None => (ModelSource::Discovered, show_new, false),
        };
        out.push(ManagedModel {
            info,
            source,
            enabled,
            has_overrides,
        });
    }

    for entry in &settings.entries {
        if seen.contains(entry.id.as_str()) {
            continue;
        }
        seen.insert(entry.id.as_str());
        let mut info = manual_model_info(&entry.id, instance);
        apply_overrides(&mut info, entry);
        let source = if entry.manual {
            ModelSource::Manual
        } else {
            ModelSource::Missing
        };
        out.push(ManagedModel {
            info,
            source,
            enabled: entry.enabled.unwrap_or(entry.manual),
            has_overrides: entry.has_overrides(),
        });
    }
    out
}

/// The effective (offered) models of a managed list.
pub fn offered(managed: &[ManagedModel]) -> Vec<ModelInfo> {
    managed
        .iter()
        .filter(|m| m.is_offered())
        .map(|m| m.info.clone())
        .collect()
}

/// A model added by hand, described from the catalog when it knows the id.
fn manual_model_info(id: &str, instance: &str) -> ModelInfo {
    ModelInfo {
        id: id.to_string(),
        name: id.to_string(),
        provider: instance.to_string(),
        parameter_count: None,
        context_window: DEFAULT_CONTEXT_WINDOW,
        supports_streaming: true,
        capabilities: vec![Capability::Chat, Capability::Completion],
        detailed_capabilities: None,
    }
    .enrich_with_catalog_by_name()
}

fn apply_overrides(info: &mut ModelInfo, entry: &ModelEntry) {
    if let Some(name) = entry
        .display_name
        .as_deref()
        .filter(|n| !n.trim().is_empty())
    {
        info.name = name.trim().to_string();
    }
    if let Some(ctx) = entry.context_window.filter(|c| *c > 0) {
        info.context_window = ctx;
    }
    if let Some(caps) = &entry.capabilities {
        let mut set: Vec<Capability> = Vec::new();
        for cap in caps.iter().filter_map(|k| Capability::from_kind(*k)) {
            if !set.contains(&cap) {
                set.push(cap);
            }
        }
        info.capabilities = set;
        // The detailed flags were derived from the replaced set.
        info.detailed_capabilities = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_config::{ModelCapabilityKind as K, ModelPricingOverride};

    fn model(id: &str) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            name: id.into(),
            provider: "upstream".into(),
            parameter_count: None,
            context_window: 4096,
            supports_streaming: true,
            capabilities: vec![Capability::Chat, Capability::Completion],
            detailed_capabilities: None,
        }
    }

    fn entry(id: &str) -> ModelEntry {
        ModelEntry {
            id: id.into(),
            ..Default::default()
        }
    }

    fn ids(models: &[ModelInfo]) -> Vec<&str> {
        models.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn no_settings_offers_every_discovered_model() {
        let managed = apply(
            &[model("a"), model("b")],
            &ProviderModelSettings::default(),
            "p",
            false,
        );
        assert_eq!(ids(&offered(&managed)), vec!["a", "b"]);
        assert!(managed.iter().all(|m| m.source == ModelSource::Discovered));
        // The instance name replaces whatever the provider put there.
        assert!(managed.iter().all(|m| m.info.provider == "p"));
    }

    #[test]
    fn hidden_models_leave_the_offered_list() {
        let settings = ProviderModelSettings {
            show_new_models: None,
            entries: vec![ModelEntry {
                enabled: Some(false),
                ..entry("b")
            }],
        };
        let managed = apply(&[model("a"), model("b")], &settings, "p", false);
        assert_eq!(ids(&offered(&managed)), vec!["a"]);
        assert!(!managed[1].enabled);
    }

    #[test]
    fn show_new_models_off_offers_only_enabled_and_manual() {
        let settings = ProviderModelSettings {
            show_new_models: Some(false),
            entries: vec![
                ModelEntry {
                    enabled: Some(true),
                    ..entry("b")
                },
                ModelEntry {
                    manual: true,
                    ..entry("m")
                },
            ],
        };
        let managed = apply(&[model("a"), model("b")], &settings, "p", false);
        assert_eq!(ids(&offered(&managed)), vec!["b", "m"]);
    }

    #[test]
    fn untrusted_listing_hides_new_models_unless_the_user_shows_them() {
        let discovered = [model("a")];
        let hidden = apply(&discovered, &ProviderModelSettings::default(), "p", true);
        assert!(offered(&hidden).is_empty());
        let shown = ProviderModelSettings {
            show_new_models: Some(true),
            entries: vec![],
        };
        assert_eq!(
            ids(&offered(&apply(&discovered, &shown, "p", true))),
            vec!["a"]
        );
    }

    #[test]
    fn manual_models_are_added_and_merge_with_discovered_ones() {
        let settings = ProviderModelSettings {
            show_new_models: None,
            entries: vec![
                ModelEntry {
                    manual: true,
                    ..entry("a")
                },
                ModelEntry {
                    manual: true,
                    ..entry("extra")
                },
            ],
        };
        let managed = apply(&[model("a")], &settings, "p", false);
        assert_eq!(managed.len(), 2);
        assert_eq!(managed[0].source, ModelSource::ManualAndDiscovered);
        assert_eq!(managed[1].source, ModelSource::Manual);
        assert_eq!(managed[1].info.provider, "p");
        assert_eq!(ids(&offered(&managed)), vec!["a", "extra"]);
        // Manual models stay offered while discovery has nothing.
        assert_eq!(
            ids(&offered(&apply(&[], &settings, "p", false))),
            vec!["a", "extra"]
        );
    }

    #[test]
    fn manual_model_can_be_hidden() {
        let settings = ProviderModelSettings {
            show_new_models: None,
            entries: vec![ModelEntry {
                manual: true,
                enabled: Some(false),
                ..entry("m")
            }],
        };
        let managed = apply(&[], &settings, "p", false);
        assert!(offered(&managed).is_empty());
        assert_eq!(managed[0].source, ModelSource::Manual);
    }

    #[test]
    fn settings_for_a_gone_model_are_kept_but_not_offered() {
        let settings = ProviderModelSettings {
            show_new_models: None,
            entries: vec![ModelEntry {
                display_name: Some("Old".into()),
                ..entry("gone")
            }],
        };
        let managed = apply(&[model("a")], &settings, "p", false);
        assert_eq!(managed[1].source, ModelSource::Missing);
        assert!(!managed[1].is_offered());
        assert_eq!(ids(&offered(&managed)), vec!["a"]);
    }

    #[test]
    fn overrides_replace_name_context_and_capabilities() {
        let settings = ProviderModelSettings {
            show_new_models: None,
            entries: vec![ModelEntry {
                display_name: Some("  Nice name ".into()),
                context_window: Some(131_072),
                capabilities: Some(vec![K::Chat, K::FunctionCalling, K::Chat, K::Unknown]),
                pricing: Some(ModelPricingOverride {
                    input_per_million: 1.0,
                    output_per_million: 2.0,
                }),
                ..entry("a")
            }],
        };
        let managed = apply(&[model("a")], &settings, "p", false);
        let info = &managed[0].info;
        assert_eq!(info.name, "Nice name");
        assert_eq!(info.context_window, 131_072);
        assert_eq!(
            info.capabilities,
            vec![Capability::Chat, Capability::FunctionCalling]
        );
        assert!(managed[0].has_overrides);
    }

    #[test]
    fn empty_overrides_leave_the_listing_alone() {
        let settings = ProviderModelSettings {
            show_new_models: None,
            entries: vec![ModelEntry {
                display_name: Some("  ".into()),
                context_window: Some(0),
                ..entry("a")
            }],
        };
        let info = &apply(&[model("a")], &settings, "p", false)[0].info;
        assert_eq!(info.name, "a");
        assert_eq!(info.context_window, 4096);
    }

    #[test]
    fn repeated_listing_ids_appear_once() {
        let managed = apply(
            &[model("a"), model("a")],
            &ProviderModelSettings::default(),
            "p",
            false,
        );
        assert_eq!(managed.len(), 1);
    }

    #[test]
    fn capability_kinds_round_trip() {
        for cap in [
            Capability::Chat,
            Capability::Completion,
            Capability::Embedding,
            Capability::Vision,
            Capability::FunctionCalling,
            Capability::Audio,
            Capability::TextToSpeech,
            Capability::Decision,
            Capability::ImageGeneration,
        ] {
            assert_eq!(Capability::from_kind(cap.kind()), Some(cap));
        }
        assert_eq!(Capability::from_kind(K::Unknown), None);
    }
}
