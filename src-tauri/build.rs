// Build script for LocalRouter AI
//
// This script runs at compile time to:
// 1. Fetch OpenRouter model catalog (with 7-day caching)
// 2. Generate Rust code with embedded static data
// 3. Enable offline-capable pricing and model metadata
//
// Environment variables:
// - LOCALROUTER_REBUILD_CATALOG=1  : Force fresh fetch (ignore cache)
// - LOCALROUTER_SKIP_CATALOG_FETCH=1 : Use cache only (no network)
//
// Privacy guarantee: Network requests ONLY at build time, never at runtime

mod buildtools;

fn main() {
    // Generate model catalog from OpenRouter
    if let Err(e) = generate_model_catalog() {
        println!("cargo:warning=Failed to generate model catalog: {}", e);
        println!("cargo:warning=Continuing build without fresh catalog data");
        // Don't fail the build - we can work with cached/committed catalog
    }

    // Run Tauri build
    tauri_build::build()
}

fn generate_model_catalog() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=catalog/.last_fetch");
    println!("cargo:rerun-if-env-changed=LOCALROUTER_REBUILD_CATALOG");
    println!("cargo:rerun-if-env-changed=LOCALROUTER_SKIP_CATALOG_FETCH");

    // Fetch catalog (uses cache if fresh)
    let models = buildtools::fetch_openrouter_catalog()?;

    // Generate Rust code
    buildtools::generate_catalog_code(&models)?;

    Ok(())
}
