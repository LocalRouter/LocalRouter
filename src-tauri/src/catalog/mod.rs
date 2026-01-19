// Model catalog module
//
// Provides offline-capable model metadata and pricing from OpenRouter.
// All data is embedded at build time - no runtime network requests.

pub mod matcher;
pub mod types;

pub use types::{CatalogMetadata, CatalogModel, CatalogPricing, Modality};

// Include the generated catalog data
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/catalog/catalog.rs"));

// Lazy-initialized matcher
use once_cell::sync::Lazy;
use matcher::ModelMatcher;

pub static MATCHER: Lazy<ModelMatcher> = Lazy::new(|| {
    ModelMatcher::new(CATALOG_MODELS)
});

/// Find a model by provider and model ID
pub fn find_model(provider: &str, model_id: &str) -> Option<&'static CatalogModel> {
    MATCHER.find_model(provider, model_id)
}

/// Get catalog metadata
pub fn metadata() -> &'static CatalogMetadata {
    &CATALOG_METADATA
}

/// Get all catalog models
pub fn models() -> &'static [CatalogModel] {
    CATALOG_MODELS
}
