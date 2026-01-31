// Build-time catalog generation module
//
// This module is used ONLY during compilation to:
// 1. Fetch models.dev model catalog
// 2. Generate Rust code with embedded static data
// 3. Enable offline-capable pricing lookup
//
// PRIVACY: No runtime network requests - all data embedded at build time

pub mod codegen;
pub mod models;
pub mod scraper;

pub use codegen::generate_catalog_code;
pub use scraper::fetch_modelsdev_catalog;
