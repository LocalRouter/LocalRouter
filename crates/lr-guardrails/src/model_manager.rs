//! Model manager for ML-based guardrail classification
//!
//! Manages lifecycle (download, load, unload, classify) for ML models.
//! Feature-gated behind `ml-models`.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::{Mutex, RwLock};
use tracing::{debug, error, info, warn};

use crate::sources::model_source::{
    GuardrailClassifier, GuardrailModelInfo, LabelMapping, ModelArchitecture,
    ModelDownloadProgress, ModelDownloadState,
};
use crate::text_extractor::ExtractedText;
use crate::types::{GuardrailMatch, SourceCheckSummary};

// Download configuration constants
const DOWNLOAD_TIMEOUT_SECS: u64 = 600;
const MAX_RETRIES: usize = 3;
const RETRY_DELAY_MS: u64 = 2000;
const MIN_DISK_SPACE_GB: u64 = 1;

/// Manages ML model lifecycle for guardrail classification
pub struct ModelManager {
    /// Base cache directory for model files
    cache_dir: PathBuf,
    /// Loaded classifiers by source_id
    classifiers: Arc<RwLock<HashMap<String, GuardrailClassifier>>>,
    /// Label mappings by source_id
    label_mappings: Arc<RwLock<HashMap<String, LabelMapping>>>,
    /// Human-readable source labels by source_id
    source_labels: Arc<RwLock<HashMap<String, String>>>,
    /// Download states by source_id
    download_states: Arc<Mutex<HashMap<String, ModelDownloadState>>>,
    /// Last access time for idle unloading
    last_access: Arc<RwLock<HashMap<String, Instant>>>,
    /// Known model metadata (source_id -> hf_repo_id)
    model_repos: Arc<RwLock<HashMap<String, String>>>,
    /// Download lock to prevent concurrent downloads
    download_lock: Arc<tokio::sync::Mutex<()>>,
    /// Progress callback (no Tauri dependency)
    #[allow(clippy::type_complexity)]
    progress_callback: Arc<RwLock<Option<Box<dyn Fn(ModelDownloadProgress) + Send + Sync>>>>,
}

impl ModelManager {
    /// Create a new model manager
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            classifiers: Arc::new(RwLock::new(HashMap::new())),
            label_mappings: Arc::new(RwLock::new(HashMap::new())),
            source_labels: Arc::new(RwLock::new(HashMap::new())),
            download_states: Arc::new(Mutex::new(HashMap::new())),
            last_access: Arc::new(RwLock::new(HashMap::new())),
            model_repos: Arc::new(RwLock::new(HashMap::new())),
            download_lock: Arc::new(tokio::sync::Mutex::new(())),
            progress_callback: Arc::new(RwLock::new(None)),
        }
    }

    /// Register a model source (source_id -> hf_repo_id mapping)
    pub fn register_model(&self, source_id: &str, hf_repo_id: &str) {
        self.model_repos
            .write()
            .insert(source_id.to_string(), hf_repo_id.to_string());
    }

    /// Register source label for display
    pub fn register_source_label(&self, source_id: &str, label: &str) {
        self.source_labels
            .write()
            .insert(source_id.to_string(), label.to_string());
    }

    /// Set progress callback for download notifications
    pub fn set_progress_callback(
        &self,
        callback: impl Fn(ModelDownloadProgress) + Send + Sync + 'static,
    ) {
        *self.progress_callback.write() = Some(Box::new(callback));
    }

    fn emit_progress(&self, progress: ModelDownloadProgress) {
        if let Some(ref cb) = *self.progress_callback.read() {
            cb(progress);
        }
    }

    /// Download a model from HuggingFace
    pub async fn download_model(
        &self,
        source_id: &str,
        hf_repo_id: &str,
        hf_token: Option<&str>,
    ) -> Result<(), String> {
        // Acquire download lock
        let _lock = self.download_lock.try_lock().map_err(|_| {
            "Another model download is already in progress".to_string()
        })?;

        info!(
            "Starting model download: source={}, repo={}, has_token={}",
            source_id, hf_repo_id, hf_token.is_some()
        );

        self.download_states
            .lock()
            .insert(source_id.to_string(), ModelDownloadState::Downloading);

        self.emit_progress(ModelDownloadProgress {
            source_id: source_id.to_string(),
            current_file: Some("Initializing...".to_string()),
            progress: 0.0,
            bytes_downloaded: 0,
            total_bytes: 0,
            bytes_per_second: 0,
        });

        let download_start = Instant::now();
        let mut cumulative_bytes: u64 = 0;

        let model_dir = self.model_dir(source_id);
        let tokenizer_dir = self.tokenizer_dir(source_id);

        // Use temp directories for atomic download
        let temp_model_dir = self.cache_dir.join(source_id).join("model.tmp");
        let temp_tokenizer_dir = self.cache_dir.join(source_id).join("tokenizer.tmp");

        // Clean up old temps
        let _ = tokio::fs::remove_dir_all(&temp_model_dir).await;
        let _ = tokio::fs::remove_dir_all(&temp_tokenizer_dir).await;

        tokio::fs::create_dir_all(&temp_model_dir)
            .await
            .map_err(|e| format!("Failed to create temp model dir: {}", e))?;
        tokio::fs::create_dir_all(&temp_tokenizer_dir)
            .await
            .map_err(|e| format!("Failed to create temp tokenizer dir: {}", e))?;

        // Check disk space
        let available = check_disk_space(&self.cache_dir);
        if available < MIN_DISK_SPACE_GB * 1_073_741_824 {
            let msg = format!(
                "Insufficient disk space. Need {}GB, have {:.1}GB",
                MIN_DISK_SPACE_GB,
                available as f64 / 1_073_741_824.0
            );
            self.download_states
                .lock()
                .insert(source_id.to_string(), ModelDownloadState::Error);
            return Err(msg);
        }

        // Initialize HuggingFace API (with optional token)
        let api = if let Some(token) = hf_token {
            hf_hub::api::tokio::ApiBuilder::new()
                .with_token(Some(token.to_string()))
                .build()
                .map_err(|e| format!("Failed to initialize HuggingFace API with token: {}", e))?
        } else {
            hf_hub::api::tokio::Api::new()
                .map_err(|e| format!("Failed to initialize HuggingFace API: {}", e))?
        };
        let repo = api.model(hf_repo_id.to_string());

        // Download model weights — try safetensors first, fall back to .bin
        self.emit_progress(ModelDownloadProgress {
            source_id: source_id.to_string(),
            current_file: Some("model.safetensors".to_string()),
            progress: 0.1,
            bytes_downloaded: 0,
            total_bytes: 0,
            bytes_per_second: 0,
        });

        let (model_filename, model_path) =
            match download_file_with_retry(&repo, "model.safetensors").await {
                Ok(path) => ("model.safetensors", path),
                Err(safetensors_err) => {
                    debug!(
                        "model.safetensors not found ({}), trying pytorch_model.bin",
                        safetensors_err
                    );
                    self.emit_progress(ModelDownloadProgress {
                        source_id: source_id.to_string(),
                        current_file: Some("pytorch_model.bin".to_string()),
                        progress: 0.1,
                        bytes_downloaded: 0,
                        total_bytes: 0,
                        bytes_per_second: 0,
                    });
                    let path = download_file_with_retry(&repo, "pytorch_model.bin")
                        .await
                        .map_err(|bin_err| {
                            format!(
                                "No model weights found. SafeTensors: {}. PyTorch: {}",
                                safetensors_err, bin_err
                            )
                        })?;
                    ("pytorch_model.bin", path)
                }
            };

        let dest = temp_model_dir.join(model_filename);
        tokio::fs::copy(&model_path, &dest)
            .await
            .map_err(|e| format!("Failed to copy model file: {}", e))?;

        // Track model weights size
        if let Ok(meta) = tokio::fs::metadata(&dest).await {
            cumulative_bytes += meta.len();
        }
        let elapsed_secs = download_start.elapsed().as_secs_f64().max(0.1);
        let bytes_per_sec = (cumulative_bytes as f64 / elapsed_secs) as u64;

        self.emit_progress(ModelDownloadProgress {
            source_id: source_id.to_string(),
            current_file: Some("tokenizer.json".to_string()),
            progress: 0.7,
            bytes_downloaded: cumulative_bytes,
            total_bytes: 0,
            bytes_per_second: bytes_per_sec,
        });

        // Download tokenizer files (config.json is required for architecture config)
        let tokenizer_files = [
            "tokenizer.json",
            "tokenizer_config.json",
            "special_tokens_map.json",
            "config.json",
        ];

        for (idx, file) in tokenizer_files.iter().enumerate() {
            debug!("Downloading {}", file);
            match download_file_with_retry(&repo, file).await {
                Ok(path) => {
                    let dest = temp_tokenizer_dir.join(file);
                    tokio::fs::copy(&path, &dest)
                        .await
                        .map_err(|e| format!("Failed to copy {}: {}", file, e))?;
                }
                Err(e) => {
                    // tokenizer.json and config.json are required, others are optional
                    if *file == "tokenizer.json" || *file == "config.json" {
                        self.download_states
                            .lock()
                            .insert(source_id.to_string(), ModelDownloadState::Error);
                        return Err(format!("Failed to download {}: {}", file, e));
                    }
                    warn!("Optional file {} not available: {}", file, e);
                }
            }

            // Track tokenizer file size
            let dest_path = temp_tokenizer_dir.join(file);
            if let Ok(meta) = tokio::fs::metadata(&dest_path).await {
                cumulative_bytes += meta.len();
            }
            let elapsed_secs = download_start.elapsed().as_secs_f64().max(0.1);
            let bytes_per_sec = (cumulative_bytes as f64 / elapsed_secs) as u64;

            let progress = 0.7 + (0.2 * (idx + 1) as f32 / tokenizer_files.len() as f32);
            self.emit_progress(ModelDownloadProgress {
                source_id: source_id.to_string(),
                current_file: Some(file.to_string()),
                progress,
                bytes_downloaded: cumulative_bytes,
                total_bytes: 0,
                bytes_per_second: bytes_per_sec,
            });
        }

        // Verify model loads — detect architecture from config.json
        let elapsed_secs = download_start.elapsed().as_secs_f64().max(0.1);
        let bytes_per_sec = (cumulative_bytes as f64 / elapsed_secs) as u64;
        self.emit_progress(ModelDownloadProgress {
            source_id: source_id.to_string(),
            current_file: Some("Verifying...".to_string()),
            progress: 0.95,
            bytes_downloaded: cumulative_bytes,
            total_bytes: 0,
            bytes_per_second: bytes_per_sec,
        });

        let temp_model_clone = temp_model_dir.clone();
        let temp_tok_clone = temp_tokenizer_dir.clone();
        let src_id = source_id.to_string();
        let verify_result = tokio::task::spawn_blocking(move || {
            // Try to detect architecture from config.json
            let arch = detect_architecture(&temp_tok_clone)?;
            GuardrailClassifier::load(&temp_model_clone, &temp_tok_clone, &src_id, &arch)
        })
        .await
        .map_err(|e| format!("Verification task failed: {}", e))?;

        if let Err(e) = verify_result {
            let _ = tokio::fs::remove_dir_all(&temp_model_dir).await;
            let _ = tokio::fs::remove_dir_all(&temp_tokenizer_dir).await;
            self.download_states
                .lock()
                .insert(source_id.to_string(), ModelDownloadState::Error);
            return Err(format!("Model verification failed: {}", e));
        }

        // Atomic move to final location
        if model_dir.exists() {
            tokio::fs::remove_dir_all(&model_dir)
                .await
                .map_err(|e| format!("Failed to remove old model dir: {}", e))?;
        }
        if tokenizer_dir.exists() {
            tokio::fs::remove_dir_all(&tokenizer_dir)
                .await
                .map_err(|e| format!("Failed to remove old tokenizer dir: {}", e))?;
        }

        tokio::fs::rename(&temp_model_dir, &model_dir)
            .await
            .map_err(|e| format!("Failed to move model to final location: {}", e))?;
        tokio::fs::rename(&temp_tokenizer_dir, &tokenizer_dir)
            .await
            .map_err(|e| format!("Failed to move tokenizer to final location: {}", e))?;

        self.download_states
            .lock()
            .insert(source_id.to_string(), ModelDownloadState::Ready);

        let elapsed_secs = download_start.elapsed().as_secs_f64().max(0.1);
        let bytes_per_sec = (cumulative_bytes as f64 / elapsed_secs) as u64;
        self.emit_progress(ModelDownloadProgress {
            source_id: source_id.to_string(),
            current_file: None,
            progress: 1.0,
            bytes_downloaded: cumulative_bytes,
            total_bytes: cumulative_bytes,
            bytes_per_second: bytes_per_sec,
        });

        info!("Model download complete: {}", source_id);
        Ok(())
    }

    /// Load a model into memory for inference
    pub fn load_model(
        &self,
        source_id: &str,
        architecture: &ModelArchitecture,
    ) -> Result<(), String> {
        let model_dir = self.model_dir(source_id);
        let tokenizer_dir = self.tokenizer_dir(source_id);

        if !self.is_model_downloaded(source_id) {
            return Err("Model not downloaded. Download it first.".to_string());
        }

        info!("Loading model: {} (arch={:?})", source_id, architecture);
        let classifier =
            GuardrailClassifier::load(&model_dir, &tokenizer_dir, source_id, architecture)?;

        // Parse label mapping from config.json
        let id2label = crate::sources::model_source::parse_id2label(&tokenizer_dir)?;
        let label_mapping = LabelMapping::from_id2label(&id2label);
        info!(
            "Loaded label mapping for {}: {} classes",
            source_id, label_mapping.num_classes
        );

        self.classifiers
            .write()
            .insert(source_id.to_string(), classifier);
        self.label_mappings
            .write()
            .insert(source_id.to_string(), label_mapping);
        self.last_access
            .write()
            .insert(source_id.to_string(), Instant::now());

        info!("Model loaded: {}", source_id);
        Ok(())
    }

    /// Unload a model from memory
    pub fn unload_model(&self, source_id: &str) {
        if self.classifiers.write().remove(source_id).is_some() {
            self.label_mappings.write().remove(source_id);
            self.last_access.write().remove(source_id);
            info!("Model unloaded: {}", source_id);
        }
    }

    /// Check if a model is loaded
    pub fn is_model_loaded(&self, source_id: &str) -> bool {
        self.classifiers.read().contains_key(source_id)
    }

    /// Check if model files exist on disk
    pub fn is_model_downloaded(&self, source_id: &str) -> bool {
        let model_dir = self.model_dir(source_id);
        let has_weights = model_dir.join("model.safetensors").exists()
            || model_dir.join("pytorch_model.bin").exists();
        let has_tokenizer = self.tokenizer_dir(source_id).join("tokenizer.json").exists();
        has_weights && has_tokenizer
    }

    /// Get model info for UI display
    pub fn get_model_info(&self, source_id: &str) -> GuardrailModelInfo {
        let hf_repo_id = self
            .model_repos
            .read()
            .get(source_id)
            .cloned()
            .unwrap_or_default();

        let download_state = if self.is_model_downloaded(source_id) {
            self.download_states
                .lock()
                .get(source_id)
                .cloned()
                .unwrap_or(ModelDownloadState::Ready)
        } else {
            self.download_states
                .lock()
                .get(source_id)
                .cloned()
                .unwrap_or(ModelDownloadState::NotDownloaded)
        };

        // Detect architecture from config or default to Bert
        let architecture = if self.is_model_downloaded(source_id) {
            detect_architecture(&self.tokenizer_dir(source_id)).unwrap_or(ModelArchitecture::Bert)
        } else {
            ModelArchitecture::Bert
        };

        // Estimate size from files on disk
        let size_bytes = if self.is_model_downloaded(source_id) {
            dir_size(&self.cache_dir.join(source_id))
        } else {
            0
        };

        GuardrailModelInfo {
            source_id: source_id.to_string(),
            hf_repo_id,
            architecture,
            download_state,
            size_bytes,
            loaded: self.is_model_loaded(source_id),
            error_message: None,
        }
    }

    /// Classify extracted texts using all loaded models
    ///
    /// Returns (matches, per-source summaries)
    pub fn classify_texts(
        &self,
        texts: &[ExtractedText],
        threshold: f32,
    ) -> (Vec<GuardrailMatch>, Vec<SourceCheckSummary>) {
        let classifiers = self.classifiers.read();
        if classifiers.is_empty() {
            return (vec![], vec![]);
        }

        let label_mappings = self.label_mappings.read();
        let source_labels_map = self.source_labels.read();
        let mut all_matches = Vec::new();
        let mut summaries = Vec::new();

        for (source_id, classifier) in classifiers.iter() {
            // Update last access
            self.last_access
                .write()
                .insert(source_id.clone(), Instant::now());

            let label_mapping = match label_mappings.get(source_id) {
                Some(m) => m,
                None => {
                    error!("No label mapping for source {}", source_id);
                    continue;
                }
            };

            let source_label = source_labels_map
                .get(source_id)
                .cloned()
                .unwrap_or_else(|| source_id.clone());

            let mut source_matches = 0;

            for extracted in texts {
                match classifier.classify(
                    &extracted.text,
                    threshold,
                    source_id,
                    &source_label,
                    label_mapping,
                ) {
                    Ok(mut matches) => {
                        // Set message_index from extracted text
                        for m in &mut matches {
                            m.message_index = extracted.message_index;
                        }
                        source_matches += matches.len();
                        all_matches.extend(matches);
                    }
                    Err(e) => {
                        error!(
                            "Model classification error (source={}): {}",
                            source_id, e
                        );
                    }
                }
            }

            summaries.push(SourceCheckSummary {
                source_id: source_id.clone(),
                source_label,
                rules_checked: label_mapping.num_classes,
                match_count: source_matches,
            });
        }

        debug!(
            "ML classification: {} models, {} texts, {} matches",
            classifiers.len(),
            texts.len(),
            all_matches.len()
        );

        (all_matches, summaries)
    }

    /// Unload models that have been idle for too long
    pub fn unload_idle_models(&self, idle_timeout_secs: u64) {
        let now = Instant::now();
        let mut to_unload = Vec::new();

        {
            let accesses = self.last_access.read();
            for (source_id, last) in accesses.iter() {
                if now.duration_since(*last).as_secs() > idle_timeout_secs {
                    to_unload.push(source_id.clone());
                }
            }
        }

        for source_id in to_unload {
            info!(
                "Unloading idle model: {} (idle > {}s)",
                source_id, idle_timeout_secs
            );
            self.unload_model(&source_id);
        }
    }

    fn model_dir(&self, source_id: &str) -> PathBuf {
        self.cache_dir.join(source_id).join("model")
    }

    fn tokenizer_dir(&self, source_id: &str) -> PathBuf {
        self.cache_dir.join(source_id).join("tokenizer")
    }
}

impl Clone for ModelManager {
    fn clone(&self) -> Self {
        Self {
            cache_dir: self.cache_dir.clone(),
            classifiers: self.classifiers.clone(),
            label_mappings: self.label_mappings.clone(),
            source_labels: self.source_labels.clone(),
            download_states: self.download_states.clone(),
            last_access: self.last_access.clone(),
            model_repos: self.model_repos.clone(),
            download_lock: self.download_lock.clone(),
            progress_callback: self.progress_callback.clone(),
        }
    }
}

/// Detect model architecture from config.json
fn detect_architecture(
    tokenizer_dir: &std::path::Path,
) -> Result<ModelArchitecture, String> {
    let config_path = tokenizer_dir.join("config.json");
    if !config_path.exists() {
        return Err("config.json not found".to_string());
    }

    let config_str = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read config.json: {}", e))?;
    let raw: serde_json::Value = serde_json::from_str(&config_str)
        .map_err(|e| format!("Failed to parse config.json: {}", e))?;

    let model_type = raw
        .get("model_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match model_type {
        "deberta-v2" | "deberta_v2" => Ok(ModelArchitecture::DebertaV2),
        "bert" => Ok(ModelArchitecture::Bert),
        other => {
            // Fallback heuristic: check for deberta-specific fields
            if raw.get("relative_attention").is_some() || raw.get("pos_att_type").is_some() {
                Ok(ModelArchitecture::DebertaV2)
            } else {
                Err(format!("Unknown model_type '{}' in config.json", other))
            }
        }
    }
}

/// Download a file from HuggingFace with retry logic
async fn download_file_with_retry(
    repo: &hf_hub::api::tokio::ApiRepo,
    filename: &str,
) -> Result<std::path::PathBuf, String> {
    let mut last_error = None;

    for attempt in 1..=MAX_RETRIES {
        debug!("Downloading {} (attempt {}/{})", filename, attempt, MAX_RETRIES);

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS),
            repo.get(filename),
        )
        .await;

        match result {
            Ok(Ok(path)) => {
                debug!("Downloaded {} on attempt {}", filename, attempt);
                return Ok(path);
            }
            Ok(Err(e)) => {
                warn!("Download attempt {} failed for {}: {}", attempt, filename, e);
                last_error = Some(format!("{}", e));
            }
            Err(_) => {
                warn!(
                    "Download timed out for {} (attempt {})",
                    filename, attempt
                );
                last_error = Some("Download timed out".to_string());
            }
        }

        if attempt < MAX_RETRIES {
            tokio::time::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS)).await;
        }
    }

    Err(format!(
        "Failed to download {} after {} attempts: {}",
        filename,
        MAX_RETRIES,
        last_error.unwrap_or_else(|| "Unknown error".to_string())
    ))
}

/// Check available disk space (best effort)
fn check_disk_space(path: &std::path::Path) -> u64 {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let check_path = if path.exists() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        if let Ok(output) = Command::new("df").arg("-k").arg(check_path).output() {
            if output.status.success() {
                let output_str = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = output_str.lines().nth(1) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 4 {
                        if let Ok(kb) = parts[3].parse::<u64>() {
                            return kb * 1024;
                        }
                    }
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        let check_path = if path.exists() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        if let Ok(output) = Command::new("df").arg("-B1").arg(check_path).output() {
            if output.status.success() {
                let output_str = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = output_str.lines().nth(1) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 4 {
                        if let Ok(bytes) = parts[3].parse::<u64>() {
                            return bytes;
                        }
                    }
                }
            }
        }
    }
    // Default: assume enough space
    u64::MAX
}

/// Calculate total size of a directory
fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let metadata = entry.metadata();
            if let Ok(meta) = metadata {
                if meta.is_file() {
                    total += meta.len();
                } else if meta.is_dir() {
                    total += dir_size(&entry.path());
                }
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_manager_new() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelManager::new(dir.path().to_path_buf());
        assert!(!manager.is_model_loaded("test"));
        assert!(!manager.is_model_downloaded("test"));
    }

    #[test]
    fn test_model_info_not_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelManager::new(dir.path().to_path_buf());
        manager.register_model("pg2", "meta-llama/Prompt-Guard-86M");

        let info = manager.get_model_info("pg2");
        assert_eq!(info.source_id, "pg2");
        assert_eq!(info.download_state, ModelDownloadState::NotDownloaded);
        assert!(!info.loaded);
    }

    #[test]
    fn test_register_model() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelManager::new(dir.path().to_path_buf());
        manager.register_model("pg2", "meta-llama/Prompt-Guard-86M");

        let info = manager.get_model_info("pg2");
        assert_eq!(info.hf_repo_id, "meta-llama/Prompt-Guard-86M");
    }

    #[test]
    fn test_unload_model_not_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelManager::new(dir.path().to_path_buf());
        // Should not panic
        manager.unload_model("nonexistent");
    }

    #[test]
    fn test_classify_texts_no_models() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelManager::new(dir.path().to_path_buf());
        let texts = vec![ExtractedText {
            text: "Hello world".to_string(),
            message_index: Some(0),
            label: "test".to_string(),
        }];
        let (matches, summaries) = manager.classify_texts(&texts, 0.7);
        assert!(matches.is_empty());
        assert!(summaries.is_empty());
    }
}
