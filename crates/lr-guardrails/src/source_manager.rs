//! Source manager: download, cache, update, and hot-reload guardrail sources
//!
//! Follows the marketplace/skill_sources.rs pattern for downloading from GitHub.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{debug, error, info, warn};

use crate::compiled_rules::CompiledRuleSet;
use crate::sources::builtin;
use crate::types::{PathDownloadError, RawRule};
use lr_types::{AppError, AppResult};

/// Downloaded file: (file_path, bytes, optional last-modified header)
type DownloadedFile = (String, Vec<u8>, Option<String>);

/// Source configuration (mirrors lr_config::GuardrailSourceConfig)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailSourceConfig {
    pub id: String,
    pub label: String,
    pub source_type: String,
    pub enabled: bool,
    pub url: String,
    pub data_paths: Vec<String>,
    pub branch: String,
    pub predefined: bool,
    #[serde(default = "default_confidence_threshold")]
    pub confidence_threshold: f32,
    #[serde(default)]
    pub model_architecture: Option<String>,
    #[serde(default)]
    pub hf_repo_id: Option<String>,
    #[serde(default)]
    pub requires_auth: bool,
}

fn default_confidence_threshold() -> f32 {
    0.7
}

/// Cached metadata for all sources
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceCacheMetadata {
    pub sources: Vec<SourceCacheEntry>,
}

/// Cache entry for a single source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceCacheEntry {
    pub source_id: String,
    pub last_updated: Option<DateTime<Utc>>,
    pub content_hash: Option<String>,
    pub rule_count: usize,
    pub error_message: Option<String>,
    /// HTTP Last-Modified header from the source server
    #[serde(default)]
    pub source_last_modified: Option<String>,
    /// Per-path download/parse errors
    #[serde(default)]
    pub path_errors: Vec<PathDownloadError>,
}

/// Status of a guardrail source (for UI display)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailSourceStatus {
    pub id: String,
    pub rule_count: usize,
    pub last_updated: Option<String>,
    pub download_state: SourceDownloadState,
    pub error_message: Option<String>,
    /// HTTP Last-Modified header value from the source server
    pub source_last_modified: Option<String>,
    /// Per-path download/parse errors
    #[serde(default)]
    pub path_errors: Vec<PathDownloadError>,
}

/// Download state of a source
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceDownloadState {
    NotDownloaded,
    Downloading,
    Ready,
    Error,
}

/// Detailed information about a guardrail source (for UI detail panel)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailSourceDetails {
    pub id: String,
    pub label: String,
    pub source_type: String,
    pub url: String,
    pub data_paths: Vec<String>,
    pub branch: String,
    pub predefined: bool,
    pub enabled: bool,
    pub cache_dir: Option<String>,
    pub raw_files: Vec<String>,
    pub compiled_rules_count: usize,
    pub error_message: Option<String>,
    pub sample_rules: Vec<RawRule>,
    pub path_errors: Vec<PathDownloadError>,
}

/// Manages downloading, caching, and updating guardrail sources
pub struct SourceManager {
    /// Base directory for caching source data
    cache_dir: PathBuf,
    /// HTTP client for downloads
    http_client: reqwest::Client,
    /// Compiled rule sets (hot-swappable)
    rule_sets: Arc<RwLock<Vec<CompiledRuleSet>>>,
    /// Cache metadata
    cache_metadata: Arc<RwLock<SourceCacheMetadata>>,
    /// Download states (source_id -> state)
    download_states:
        Arc<parking_lot::Mutex<std::collections::HashMap<String, SourceDownloadState>>>,
    /// ML model manager (optional, only when ml-models feature is enabled)
    #[cfg(feature = "ml-models")]
    model_manager: Option<Arc<crate::model_manager::ModelManager>>,
}

impl SourceManager {
    /// Create a new source manager
    pub fn new(cache_dir: PathBuf) -> Self {
        // Compile built-in rules immediately
        let builtin_rules = builtin::builtin_rules();
        let builtin_set = CompiledRuleSet::compile("builtin", "Built-in Rules", &builtin_rules);
        let builtin_count = builtin_set.rule_count;

        info!("Loaded {} built-in guardrail rules", builtin_count);

        Self {
            cache_dir,
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            rule_sets: Arc::new(RwLock::new(vec![builtin_set])),
            cache_metadata: Arc::new(RwLock::new(SourceCacheMetadata::default())),
            download_states: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
            #[cfg(feature = "ml-models")]
            model_manager: None,
        }
    }

    /// Get a reference to the compiled rule sets
    pub fn rule_sets(&self) -> Arc<RwLock<Vec<CompiledRuleSet>>> {
        self.rule_sets.clone()
    }

    /// Set the ML model manager
    #[cfg(feature = "ml-models")]
    pub fn set_model_manager(&mut self, manager: Arc<crate::model_manager::ModelManager>) {
        self.model_manager = Some(manager);
    }

    /// Get the ML model manager
    #[cfg(feature = "ml-models")]
    pub fn model_manager(&self) -> Option<&Arc<crate::model_manager::ModelManager>> {
        self.model_manager.as_ref()
    }

    /// Load cached rules from disk for all sources
    pub async fn load_cached_sources(&self, sources: &[GuardrailSourceConfig]) -> AppResult<()> {
        // Load cache metadata
        let meta_path = self.cache_dir.join("cache.json");
        if meta_path.exists() {
            match tokio::fs::read_to_string(&meta_path).await {
                Ok(data) => {
                    if let Ok(meta) = serde_json::from_str::<SourceCacheMetadata>(&data) {
                        *self.cache_metadata.write() = meta;
                    }
                }
                Err(e) => {
                    warn!("Failed to load guardrails cache metadata: {}", e);
                }
            }
        }

        // Load cached compiled rules for each enabled source
        let mut loaded_sets = Vec::new();

        // Built-in rules always first
        let builtin_rules = builtin::builtin_rules();
        loaded_sets.push(CompiledRuleSet::compile(
            "builtin",
            "Built-in Rules",
            &builtin_rules,
        ));

        for source in sources.iter().filter(|s| s.enabled) {
            let compiled_path = self.cache_dir.join(&source.id).join("compiled_rules.json");
            if compiled_path.exists() {
                match tokio::fs::read_to_string(&compiled_path).await {
                    Ok(data) => {
                        if let Ok(rules) = serde_json::from_str::<Vec<RawRule>>(&data) {
                            let set = CompiledRuleSet::compile(&source.id, &source.label, &rules);
                            info!(
                                "Loaded {} cached rules from source '{}'",
                                set.rule_count, source.id
                            );
                            loaded_sets.push(set);
                        }
                    }
                    Err(e) => {
                        debug!("No cached rules for source '{}': {}", source.id, e);
                    }
                }
            }
        }

        *self.rule_sets.write() = loaded_sets;
        Ok(())
    }

    /// Update a single source: download, parse, compile, cache
    pub async fn update_source(&self, source: &GuardrailSourceConfig) -> AppResult<usize> {
        if source.id == "builtin" {
            return Ok(builtin::builtin_rules().len());
        }

        info!("Updating guardrail source '{}'", source.id);

        // Set download state
        self.download_states
            .lock()
            .insert(source.id.clone(), SourceDownloadState::Downloading);

        let result = self.download_and_compile_source(source).await;

        match &result {
            Ok(count) => {
                self.download_states
                    .lock()
                    .insert(source.id.clone(), SourceDownloadState::Ready);
                info!("Updated source '{}': {} rules", source.id, count);
            }
            Err(e) => {
                self.download_states
                    .lock()
                    .insert(source.id.clone(), SourceDownloadState::Error);
                error!("Failed to update source '{}': {}", source.id, e);
            }
        }

        result
    }

    /// Download, parse, and compile rules from a source
    async fn download_and_compile_source(
        &self,
        source: &GuardrailSourceConfig,
    ) -> AppResult<usize> {
        let source_dir = self.cache_dir.join(&source.id);
        let raw_dir = source_dir.join("raw");
        tokio::fs::create_dir_all(&raw_dir)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to create source dir: {}", e)))?;

        let mut all_rules = Vec::new();
        let mut path_errors: Vec<PathDownloadError> = Vec::new();
        let repo_path = extract_repo_path(&source.url);

        // Download all data_paths in a single tarball request
        let (files, last_modified) = self
            .download_repo_tarball(&repo_path, &source.branch, &source.data_paths, &raw_dir)
            .await?;

        for (file_path, bytes, _) in &files {
            match parse_source_data(bytes, source, file_path) {
                Ok(rules) => {
                    all_rules.extend(rules);
                }
                Err(e) => {
                    let detail = format!("Parse error for '{}': {}", file_path, e);
                    warn!("{}", detail);
                    path_errors.push(PathDownloadError {
                        path: file_path.clone(),
                        error: "parse_error".to_string(),
                        detail,
                    });
                }
            }
        }

        let rule_count = all_rules.len();

        // Cache compiled rules as JSON
        let compiled_path = source_dir.join("compiled_rules.json");
        let json = serde_json::to_string_pretty(&all_rules)
            .map_err(|e| AppError::Internal(format!("Failed to serialize rules: {}", e)))?;
        tokio::fs::write(&compiled_path, &json)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to write compiled rules: {}", e)))?;

        // Compute hash
        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        let hash = format!("{:x}", hasher.finalize());

        // Update cache metadata
        {
            let mut meta = self.cache_metadata.write();
            if let Some(entry) = meta.sources.iter_mut().find(|e| e.source_id == source.id) {
                entry.last_updated = Some(Utc::now());
                entry.content_hash = Some(hash);
                entry.rule_count = rule_count;
                entry.error_message = None;
                entry.path_errors = path_errors;
                if last_modified.is_some() {
                    entry.source_last_modified = last_modified.clone();
                }
            } else {
                meta.sources.push(SourceCacheEntry {
                    source_id: source.id.clone(),
                    last_updated: Some(Utc::now()),
                    content_hash: Some(hash),
                    rule_count,
                    error_message: None,
                    source_last_modified: last_modified.clone(),
                    path_errors,
                });
            }
        }

        // Save cache metadata
        self.save_cache_metadata().await?;

        // Hot-swap: rebuild this source's rule set and replace in the list
        let compiled_set = CompiledRuleSet::compile(&source.id, &source.label, &all_rules);
        {
            let mut sets = self.rule_sets.write();
            // Remove old entry for this source (if any)
            sets.retain(|s| s.source_id != source.id);
            sets.push(compiled_set);
        }

        Ok(rule_count)
    }

    /// Download a repository tarball and extract matching files.
    ///
    /// Downloads `https://github.com/{owner}/{repo}/archive/refs/heads/{branch}.tar.gz`
    /// (CDN-served, no API rate limit) and streams through the archive, extracting only
    /// files whose paths match one of the `data_paths` prefixes.
    ///
    /// Returns `(Vec<DownloadedFile>, Option<last_modified_header>)`.
    async fn download_repo_tarball(
        &self,
        repo_path: &str,
        branch: &str,
        data_paths: &[String],
        raw_dir: &std::path::Path,
    ) -> AppResult<(Vec<DownloadedFile>, Option<String>)> {
        use flate2::read::GzDecoder;
        use std::io::Read;
        use tar::Archive;

        const MAX_FILE_SIZE: u64 = 1_048_576; // 1MB
        const SUPPORTED_EXTENSIONS: &[&str] = &[
            ".json", ".txt", ".md", ".yar", ".yara", ".py", ".yaml", ".yml",
        ];

        let url = format!(
            "https://github.com/{}/archive/refs/heads/{}.tar.gz",
            repo_path, branch
        );

        info!("Downloading tarball from: {}", url);

        let response = self
            .http_client
            .get(&url)
            .header("User-Agent", "LocalRouter-GuardRails/0.1")
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("Tarball download failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::Internal(format!(
                "Tarball download returned HTTP {}",
                response.status()
            )));
        }

        let last_modified = response
            .headers()
            .get("last-modified")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let tarball_bytes = response
            .bytes()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to read tarball body: {}", e)))?;

        info!(
            "Downloaded tarball: {} bytes, extracting matching files",
            tarball_bytes.len()
        );

        // Decompress and iterate through the tar archive
        let gz = GzDecoder::new(&tarball_bytes[..]);
        let mut archive = Archive::new(gz);

        // GitHub tarballs have a root dir like `{repo}-{branch}/`
        // We need to strip this prefix to match against data_paths
        let mut results = Vec::new();

        let entries = archive
            .entries()
            .map_err(|e| AppError::Internal(format!("Failed to read tar entries: {}", e)))?;

        for entry_result in entries {
            let mut entry = match entry_result {
                Ok(e) => e,
                Err(e) => {
                    debug!("Skipping unreadable tar entry: {}", e);
                    continue;
                }
            };

            // Only process regular files
            let entry_type = entry.header().entry_type();
            if !entry_type.is_file() {
                continue;
            }

            let entry_size = entry.header().size().unwrap_or(0);
            if entry_size > MAX_FILE_SIZE {
                continue;
            }

            let entry_path = match entry.path() {
                Ok(p) => p.to_string_lossy().to_string(),
                Err(_) => continue,
            };

            // Strip the root directory (e.g., "presidio-main/") from the path
            let stripped_path = match entry_path.find('/') {
                Some(idx) => &entry_path[idx + 1..],
                None => continue, // No slash means it's the root dir itself
            };

            // Check if this file matches any of the data_paths prefixes
            let matches_data_path = data_paths
                .iter()
                .any(|dp| stripped_path.starts_with(dp.as_str()));

            if !matches_data_path {
                continue;
            }

            // Check file extension
            let file_name = stripped_path.rsplit('/').next().unwrap_or(stripped_path);
            let has_supported_ext = SUPPORTED_EXTENSIONS
                .iter()
                .any(|ext| file_name.ends_with(ext));
            if !has_supported_ext {
                continue;
            }

            // Skip __init__.py, test files, conftest files
            if file_name == "__init__.py"
                || file_name.starts_with("test_")
                || file_name.starts_with("conftest")
            {
                continue;
            }

            // Read the file contents
            let mut contents = Vec::new();
            if let Err(e) = entry.read_to_end(&mut contents) {
                warn!("Failed to read tar entry '{}': {}", stripped_path, e);
                continue;
            }

            // Save to raw_dir (use sync write — we're already in a sync tar loop)
            let filename = stripped_path.replace('/', "_");
            let raw_path = raw_dir.join(&filename);
            let _ = std::fs::write(&raw_path, &contents);

            debug!("Extracted: {} ({} bytes)", stripped_path, contents.len());
            results.push((
                stripped_path.to_string(),
                contents,
                last_modified.clone(),
            ));
        }

        info!(
            "Extracted {} matching files from tarball",
            results.len()
        );

        Ok((results, last_modified))
    }

    /// Get detailed information about a source (for UI detail panel)
    pub fn get_source_details(&self, source: &GuardrailSourceConfig) -> GuardrailSourceDetails {
        let source_dir = self.cache_dir.join(&source.id);
        let raw_dir = source_dir.join("raw");

        // List raw files
        let raw_files = std::fs::read_dir(&raw_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // Read all compiled rules
        let compiled_path = source_dir.join("compiled_rules.json");
        let (compiled_rules_count, sample_rules) = std::fs::read_to_string(&compiled_path)
            .ok()
            .and_then(|data| serde_json::from_str::<Vec<RawRule>>(&data).ok())
            .map(|rules| {
                let count = rules.len();
                (count, rules)
            })
            .unwrap_or((0, vec![]));

        // Get cache entry info
        let meta = self.cache_metadata.read();
        let cache_entry = meta.sources.iter().find(|e| e.source_id == source.id);
        let error_message = cache_entry.and_then(|e| e.error_message.clone());
        let path_errors = cache_entry
            .map(|e| e.path_errors.clone())
            .unwrap_or_default();

        let cache_dir_str = if source_dir.exists() {
            Some(source_dir.to_string_lossy().to_string())
        } else {
            None
        };

        GuardrailSourceDetails {
            id: source.id.clone(),
            label: source.label.clone(),
            source_type: source.source_type.clone(),
            url: source.url.clone(),
            data_paths: source.data_paths.clone(),
            branch: source.branch.clone(),
            predefined: source.predefined,
            enabled: source.enabled,
            cache_dir: cache_dir_str,
            raw_files,
            compiled_rules_count,
            error_message,
            sample_rules,
            path_errors,
        }
    }

    /// Get the cache directory path (for source detail inspection)
    pub fn cache_dir(&self) -> &std::path::Path {
        &self.cache_dir
    }

    /// Get a snapshot of the cache metadata
    pub fn cache_metadata(&self) -> SourceCacheMetadata {
        self.cache_metadata.read().clone()
    }

    /// Save cache metadata to disk
    async fn save_cache_metadata(&self) -> AppResult<()> {
        let meta_path = self.cache_dir.join("cache.json");
        let meta = self.cache_metadata.read().clone();
        let json = serde_json::to_string_pretty(&meta).map_err(|e| {
            AppError::Internal(format!("Failed to serialize cache metadata: {}", e))
        })?;
        tokio::fs::write(&meta_path, json)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to write cache metadata: {}", e)))?;
        Ok(())
    }

    /// Get status for all sources (for UI display)
    pub fn get_sources_status(
        &self,
        sources: &[GuardrailSourceConfig],
    ) -> Vec<GuardrailSourceStatus> {
        let meta = self.cache_metadata.read();
        let states = self.download_states.lock();
        let rule_sets = self.rule_sets.read();

        let mut statuses = Vec::new();

        // Built-in is always first
        let builtin_count = rule_sets
            .iter()
            .find(|s| s.source_id == "builtin")
            .map(|s| s.rule_count)
            .unwrap_or(0);
        statuses.push(GuardrailSourceStatus {
            id: "builtin".to_string(),
            rule_count: builtin_count,
            last_updated: None,
            download_state: SourceDownloadState::Ready,
            error_message: None,
            source_last_modified: None,
            path_errors: vec![],
        });

        for source in sources {
            let cache_entry = meta.sources.iter().find(|e| e.source_id == source.id);
            let rule_count = rule_sets
                .iter()
                .find(|s| s.source_id == source.id)
                .map(|s| s.rule_count)
                .unwrap_or(0);

            let download_state =
                states
                    .get(&source.id)
                    .cloned()
                    .unwrap_or(if cache_entry.is_some() {
                        SourceDownloadState::Ready
                    } else {
                        SourceDownloadState::NotDownloaded
                    });

            statuses.push(GuardrailSourceStatus {
                id: source.id.clone(),
                rule_count,
                last_updated: cache_entry
                    .and_then(|e| e.last_updated)
                    .map(|dt| dt.to_rfc3339()),
                download_state,
                error_message: cache_entry.and_then(|e| e.error_message.clone()),
                source_last_modified: cache_entry.and_then(|e| e.source_last_modified.clone()),
                path_errors: cache_entry
                    .map(|e| e.path_errors.clone())
                    .unwrap_or_default(),
            });
        }

        statuses
    }

    /// Check if a source needs updating based on interval
    pub fn needs_update(&self, source_id: &str, interval_hours: u64) -> bool {
        if interval_hours == 0 {
            return false; // Manual only
        }

        let meta = self.cache_metadata.read();
        if let Some(entry) = meta.sources.iter().find(|e| e.source_id == source_id) {
            if let Some(last_updated) = entry.last_updated {
                let elapsed = Utc::now().signed_duration_since(last_updated);
                return elapsed.num_hours() >= interval_hours as i64;
            }
        }

        true // Never updated
    }

    /// Load custom rules defined by the user into the engine
    ///
    /// Converts enabled CustomGuardrailRule entries into RawRules, compiles them
    /// as a "custom" source, and hot-swaps into the active rule sets.
    pub fn load_custom_rules(&self, custom_rules: &[CustomGuardrailRule]) {
        let raw_rules: Vec<RawRule> = custom_rules
            .iter()
            .filter(|r| r.enabled)
            .filter_map(|r| {
                // Validate the regex compiles before including
                if regex::Regex::new(&r.pattern).is_err() {
                    warn!("Skipping custom rule '{}': invalid regex pattern", r.id);
                    return None;
                }
                Some(RawRule {
                    id: format!("custom-{}", r.id),
                    name: r.name.clone(),
                    pattern: r.pattern.clone(),
                    category: parse_category(&r.category),
                    severity: crate::types::GuardrailSeverity::from_str_lenient(&r.severity),
                    direction: parse_direction(&r.direction),
                    description: format!("Custom rule: {}", r.name),
                })
            })
            .collect();

        let compiled_set = CompiledRuleSet::compile("custom", "Custom Rules", &raw_rules);
        info!("Loaded {} custom guardrail rules", compiled_set.rule_count);

        let mut sets = self.rule_sets.write();
        sets.retain(|s| s.source_id != "custom");
        if compiled_set.rule_count > 0 {
            sets.push(compiled_set);
        }
    }

    /// Remove a source's cached data
    pub async fn remove_source(&self, source_id: &str) -> AppResult<()> {
        // Remove from rule sets
        {
            let mut sets = self.rule_sets.write();
            sets.retain(|s| s.source_id != source_id);
        }

        // Remove from cache metadata
        {
            let mut meta = self.cache_metadata.write();
            meta.sources.retain(|e| e.source_id != source_id);
        }
        self.save_cache_metadata().await?;

        // Remove cached files
        let source_dir = self.cache_dir.join(source_id);
        if source_dir.exists() {
            tokio::fs::remove_dir_all(&source_dir)
                .await
                .map_err(|e| AppError::Internal(format!("Failed to remove source dir: {}", e)))?;
        }

        Ok(())
    }
}

impl Clone for SourceManager {
    fn clone(&self) -> Self {
        Self {
            cache_dir: self.cache_dir.clone(),
            http_client: self.http_client.clone(),
            rule_sets: self.rule_sets.clone(),
            cache_metadata: self.cache_metadata.clone(),
            download_states: self.download_states.clone(),
            #[cfg(feature = "ml-models")]
            model_manager: self.model_manager.clone(),
        }
    }
}

/// A custom guardrail rule (mirrors lr_config::CustomGuardrailRule)
#[derive(Debug, Clone)]
pub struct CustomGuardrailRule {
    pub id: String,
    pub name: String,
    pub pattern: String,
    pub category: String,
    pub severity: String,
    pub direction: String,
    pub enabled: bool,
}

/// Parse a category string into GuardrailCategory
fn parse_category(s: &str) -> crate::types::GuardrailCategory {
    use crate::types::GuardrailCategory;
    match s.to_lowercase().as_str() {
        "prompt_injection" => GuardrailCategory::PromptInjection,
        "jailbreak_attempt" => GuardrailCategory::JailbreakAttempt,
        "pii_leakage" => GuardrailCategory::PiiLeakage,
        "code_injection" => GuardrailCategory::CodeInjection,
        "encoded_payload" => GuardrailCategory::EncodedPayload,
        "sensitive_data" => GuardrailCategory::SensitiveData,
        "malicious_output" => GuardrailCategory::MaliciousOutput,
        "data_leakage" => GuardrailCategory::DataLeakage,
        _ => GuardrailCategory::PromptInjection,
    }
}

/// Parse a direction string into ScanDirection
fn parse_direction(s: &str) -> crate::types::ScanDirection {
    use crate::types::ScanDirection;
    match s.to_lowercase().as_str() {
        "input" => ScanDirection::Input,
        "output" => ScanDirection::Output,
        "both" => ScanDirection::Both,
        _ => ScanDirection::Both,
    }
}

/// Parse source data based on file extension, falling back to source type.
///
/// Routes by extension first (.json, .py, .yar, .yaml, .txt, .md),
/// then falls back to the configured source_type if extension doesn't match.
fn parse_source_data(
    data: &[u8],
    source: &GuardrailSourceConfig,
    file_path: &str,
) -> AppResult<Vec<RawRule>> {
    use crate::sources::{python_source, regex_source, yara_source};
    use crate::types::{GuardrailCategory, ScanDirection};

    let ext = file_path.rsplit('.').next().unwrap_or("").to_lowercase();

    match ext.as_str() {
        "json" => {
            // Try structured JSON first, then plain text
            if let Ok(rules) = regex_source::parse_regex_json(data, &source.id) {
                Ok(rules)
            } else {
                regex_source::parse_pattern_list(
                    data,
                    &source.id,
                    GuardrailCategory::PromptInjection,
                    crate::types::GuardrailSeverity::Medium,
                    ScanDirection::Input,
                )
            }
        }
        "py" => python_source::extract_python_patterns(
            data,
            &source.id,
            file_path,
            GuardrailCategory::PiiLeakage,
            ScanDirection::Both,
        ),
        "yar" | "yara" => yara_source::parse_yara_rules(
            data,
            &source.id,
            GuardrailCategory::PromptInjection,
            ScanDirection::Input,
        ),
        "yaml" | "yml" => {
            // Some YAML files contain regex patterns — try JSON parse first
            if let Ok(rules) = regex_source::parse_regex_json(data, &source.id) {
                Ok(rules)
            } else {
                // Try as plain text pattern list
                regex_source::parse_pattern_list(
                    data,
                    &source.id,
                    GuardrailCategory::PromptInjection,
                    crate::types::GuardrailSeverity::Medium,
                    ScanDirection::Input,
                )
            }
        }
        "txt" | "md" => regex_source::parse_pattern_list(
            data,
            &source.id,
            GuardrailCategory::PromptInjection,
            crate::types::GuardrailSeverity::Medium,
            ScanDirection::Input,
        ),
        _ => {
            // Fall back to source_type-based routing
            match source.source_type.as_str() {
                "regex" => {
                    if let Ok(rules) = regex_source::parse_regex_json(data, &source.id) {
                        Ok(rules)
                    } else {
                        regex_source::parse_pattern_list(
                            data,
                            &source.id,
                            GuardrailCategory::PromptInjection,
                            crate::types::GuardrailSeverity::Medium,
                            ScanDirection::Input,
                        )
                    }
                }
                "yara" => yara_source::parse_yara_rules(
                    data,
                    &source.id,
                    GuardrailCategory::PromptInjection,
                    ScanDirection::Input,
                ),
                "model" => Ok(Vec::new()),
                _ => Err(AppError::Internal(format!(
                    "Unknown source type '{}' for file '{}'",
                    source.source_type, file_path
                ))),
            }
        }
    }
}

/// Extract repo path from a GitHub URL
/// e.g. "https://github.com/microsoft/presidio" → "microsoft/presidio"
fn extract_repo_path(url: &str) -> String {
    url.trim_end_matches('/')
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .unwrap_or(url)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_repo_path() {
        assert_eq!(
            extract_repo_path("https://github.com/microsoft/presidio"),
            "microsoft/presidio"
        );
        assert_eq!(
            extract_repo_path("https://github.com/swisskyrepo/PayloadsAllTheThings/"),
            "swisskyrepo/PayloadsAllTheThings"
        );
    }

    #[tokio::test]
    async fn test_source_manager_new() {
        let dir = tempfile::tempdir().unwrap();
        let manager = SourceManager::new(dir.path().to_path_buf());

        // Should have built-in rules loaded
        let sets = manager.rule_sets.read();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].source_id, "builtin");
        assert!(sets[0].rule_count > 0);
    }

    #[test]
    fn test_needs_update_never_updated() {
        let dir = tempfile::tempdir().unwrap();
        let manager = SourceManager::new(dir.path().to_path_buf());
        assert!(manager.needs_update("presidio", 24));
    }

    #[test]
    fn test_needs_update_manual_only() {
        let dir = tempfile::tempdir().unwrap();
        let manager = SourceManager::new(dir.path().to_path_buf());
        assert!(!manager.needs_update("presidio", 0));
    }
}
