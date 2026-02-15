// Build-time models.dev catalog scraper
//
// This module fetches the model catalog from models.dev API during compilation.
// PRIVACY: This ONLY runs at build time, never at runtime.

use crate::buildtools::models::{flatten_models, FlattenedModel, ModelsDevResponse};
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MODELS_DEV_API: &str = "https://models.dev/api.json";
const CACHE_DIR: &str = "catalog";
const CACHE_FILE: &str = "catalog/modelsdev_raw.json";
const TIMESTAMP_FILE: &str = "catalog/.last_fetch";
const CACHE_DURATION_DAYS: u64 = 7;

pub struct CatalogFetcher {
    cache_dir: String,
    cache_file: String,
    timestamp_file: String,
}

impl CatalogFetcher {
    pub fn new() -> Self {
        Self {
            cache_dir: CACHE_DIR.to_string(),
            cache_file: CACHE_FILE.to_string(),
            timestamp_file: TIMESTAMP_FILE.to_string(),
        }
    }

    /// Fetch models.dev catalog with intelligent caching
    pub fn fetch(&self) -> Result<Vec<FlattenedModel>, Box<dyn std::error::Error>> {
        // Check environment variables
        let force_rebuild = std::env::var("LOCALROUTER_REBUILD_CATALOG").is_ok();
        let skip_fetch = std::env::var("LOCALROUTER_SKIP_CATALOG_FETCH").is_ok();

        // Ensure cache directory exists
        fs::create_dir_all(&self.cache_dir)?;

        // If skip_fetch is set, only use cache
        if skip_fetch {
            println!("cargo:warning=LOCALROUTER_SKIP_CATALOG_FETCH set, using cached catalog only");
            return self.load_from_cache();
        }

        // Check if we need to fetch
        let should_fetch = force_rebuild || self.is_cache_stale()?;

        if should_fetch {
            println!("cargo:warning=Fetching fresh model catalog from models.dev...");
            match self.fetch_from_api() {
                Ok(models) => {
                    // Save raw response to cache
                    self.update_timestamp()?;
                    println!(
                        "cargo:warning=Successfully fetched {} models from models.dev",
                        models.len()
                    );
                    Ok(models)
                }
                Err(e) => {
                    println!(
                        "cargo:warning=Failed to fetch from models.dev: {}. Trying cache...",
                        e
                    );
                    self.load_from_cache()
                }
            }
        } else {
            self.load_from_cache()
        }
    }

    /// Fetch fresh data from models.dev API
    fn fetch_from_api(&self) -> Result<Vec<FlattenedModel>, Box<dyn std::error::Error>> {
        // Use blocking reqwest since we're in build.rs context
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("LocalRouter/", env!("CARGO_PKG_VERSION")))
            .build()?;

        let response = client.get(MODELS_DEV_API).send()?;

        if !response.status().is_success() {
            return Err(format!("models.dev API returned status: {}", response.status()).into());
        }

        let body = response.text()?;

        // Save raw response to cache file
        fs::write(&self.cache_file, &body)?;

        let parsed: ModelsDevResponse = serde_json::from_str(&body)?;

        // Flatten the nested structure
        let flattened = flatten_models(parsed);

        Ok(flattened)
    }

    /// Load catalog from cache file
    fn load_from_cache(&self) -> Result<Vec<FlattenedModel>, Box<dyn std::error::Error>> {
        if !Path::new(&self.cache_file).exists() {
            return Err("Cache file does not exist and fetching is disabled".into());
        }

        let content = fs::read_to_string(&self.cache_file)?;
        let parsed: ModelsDevResponse = serde_json::from_str(&content)?;

        // Flatten the nested structure
        let flattened = flatten_models(parsed);

        Ok(flattened)
    }

    /// Check if cache is stale (>7 days old)
    fn is_cache_stale(&self) -> Result<bool, Box<dyn std::error::Error>> {
        if !Path::new(&self.timestamp_file).exists() {
            return Ok(true); // No timestamp = stale
        }

        let timestamp_str = fs::read_to_string(&self.timestamp_file)?;
        let last_fetch: u64 = timestamp_str.trim().parse()?;

        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

        let age_days = (now - last_fetch) / 86400; // seconds per day

        Ok(age_days >= CACHE_DURATION_DAYS)
    }

    /// Update timestamp file
    fn update_timestamp(&self) -> Result<(), Box<dyn std::error::Error>> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

        fs::write(&self.timestamp_file, now.to_string())?;

        Ok(())
    }
}

/// Main entry point for build.rs
pub fn fetch_modelsdev_catalog() -> Result<Vec<FlattenedModel>, Box<dyn std::error::Error>> {
    let fetcher = CatalogFetcher::new();
    fetcher.fetch()
}
