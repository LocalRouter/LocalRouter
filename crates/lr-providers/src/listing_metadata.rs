//! A provider's model listing as raw JSON: each model's own entry, kept so
//! free-model rules can look at the fields the provider publishes and so
//! listed prices can be used for cost tracking.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::PricingInfo;

/// Raw listing entries by model id.
pub type ModelMetadata = Arc<HashMap<String, Value>>;

/// A listed token price above this (per token, i.e. $1,000 per 1M tokens)
/// is taken to be in another unit (per 1M tokens) and not used for cost.
const MAX_PLAUSIBLE_PRICE_PER_TOKEN: f64 = 0.001;

/// A price as a JSON string (`"0.000000042"`, `"0.042e-6"`) or number.
pub fn price_value(v: &Value) -> Option<f64> {
    let price = match v {
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        Value::Number(n) => n.as_f64()?,
        _ => return None,
    };
    price.is_finite().then_some(price)
}

/// A price given per token, converted to per 1K tokens. Negative values
/// (OpenRouter lists `-1` for variable-priced routers) are not prices.
pub fn per_1k(v: Option<&Value>) -> Option<f64> {
    let per_token = price_value(v?)?;
    (per_token >= 0.0).then_some(per_token * 1000.0)
}

/// The `pricing` object's input and output price fields of a listing entry:
/// `prompt`/`completion` (OpenRouter, LLM Gateway) or `input`/`output`
/// (Vercel, Free.ai).
pub fn listed_price_fields(model: &Value) -> Option<(&Value, Option<&Value>)> {
    let p = model.get("pricing")?;
    let input = p.get("prompt").or_else(|| p.get("input"))?;
    let output = p.get("completion").or_else(|| p.get("output"));
    Some((input, output))
}

/// Pricing from a model's listing entry, when it lists per-token prices
/// (missing output price: $0). `None` when there is no usable price, or
/// one too large to be a per-token price.
pub fn listing_pricing(model: &Value) -> Option<PricingInfo> {
    let (input, output) = listed_price_fields(model)?;
    let input = per_1k(Some(input))?;
    let output = match output {
        Some(v) => per_1k(Some(v))?,
        None => 0.0,
    };
    let limit = MAX_PLAUSIBLE_PRICE_PER_TOKEN * 1000.0;
    if input > limit || output > limit {
        return None;
    }
    Some(PricingInfo {
        input_cost_per_1k: input,
        output_cost_per_1k: output,
        ..PricingInfo::free()
    })
}

/// Raw entries of an OpenAI-style listing (`{"data": [...]}`, a bare array,
/// `{"result": [...]}` or `{"models": [...]}`) by model id.
pub fn entries_by_id(body: &Value) -> HashMap<String, Value> {
    let items = body
        .get("data")
        .or_else(|| body.get("result"))
        .or_else(|| body.get("models"))
        .unwrap_or(body)
        .as_array();
    items
        .into_iter()
        .flatten()
        .filter_map(|m| Some((m.get("id")?.as_str()?.to_string(), m.clone())))
        .collect()
}

/// How long to wait before fetching a listing again for pricing after a
/// failure.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(60);

/// The latest listing a provider fetched, kept so pricing does not need a
/// request of its own.
#[derive(Default)]
pub struct ListingMetadataCache {
    entries: parking_lot::RwLock<Option<ModelMetadata>>,
    failed_at: parking_lot::Mutex<Option<Instant>>,
}

impl ListingMetadataCache {
    /// Remember a freshly fetched listing.
    pub fn store(&self, entries: ModelMetadata) {
        *self.entries.write() = Some(entries);
        *self.failed_at.lock() = None;
    }

    /// Remember that fetching failed (pricing will not retry for a while).
    pub fn mark_failed(&self) {
        *self.failed_at.lock() = Some(Instant::now());
    }

    /// The cached listing, if one was fetched.
    pub fn get(&self) -> Option<ModelMetadata> {
        self.entries.read().clone()
    }

    /// Whether pricing should fetch the listing: nothing cached and no
    /// recent failure.
    pub fn should_fetch(&self) -> bool {
        self.entries.read().is_none()
            && !self
                .failed_at
                .lock()
                .is_some_and(|t| t.elapsed() < RETRY_AFTER_FAILURE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn listing_prices_are_per_token() {
        let m = json!({"pricing": {"prompt": "0.000002", "completion": "0.000008"}});
        let p = listing_pricing(&m).unwrap();
        assert!((p.input_cost_per_1k - 0.002).abs() < 1e-12);
        assert!((p.output_cost_per_1k - 0.008).abs() < 1e-12);

        let m = json!({"pricing": {"input": 0.0000005}});
        let p = listing_pricing(&m).unwrap();
        assert!((p.input_cost_per_1k - 0.0005).abs() < 1e-12);
        assert_eq!(p.output_cost_per_1k, 0.0);

        let free = json!({"pricing": {"prompt": "0", "completion": "0"}});
        assert_eq!(listing_pricing(&free).unwrap().input_cost_per_1k, 0.0);
    }

    #[test]
    fn unusable_listing_prices_are_ignored() {
        // Variable-priced router, per-1M units, no price, not a number.
        for m in [
            json!({"pricing": {"prompt": "-1", "completion": "-1"}}),
            json!({"pricing": {"input": 0.15, "output": 0.6}}),
            json!({"pricing": {"hourly": 2}}),
            json!({"pricing": {"prompt": "free"}}),
            json!({"id": "x"}),
        ] {
            assert!(listing_pricing(&m).is_none(), "{m}");
        }
    }

    #[test]
    fn entries_from_each_envelope() {
        for body in [
            json!({"data": [{"id": "a", "x": 1}]}),
            json!([{"id": "a", "x": 1}]),
            json!({"result": [{"id": "a", "x": 1}]}),
            json!({"models": [{"id": "a", "x": 1}]}),
        ] {
            let entries = entries_by_id(&body);
            assert_eq!(entries["a"]["x"], 1, "{body}");
        }
        assert!(entries_by_id(&json!({"data": "nope"})).is_empty());
    }

    #[test]
    fn cache_backs_off_after_a_failure() {
        let cache = ListingMetadataCache::default();
        assert!(cache.should_fetch());
        cache.mark_failed();
        assert!(!cache.should_fetch());
        cache.store(Arc::new(HashMap::new()));
        assert!(!cache.should_fetch());
        assert!(cache.get().is_some());
    }
}
