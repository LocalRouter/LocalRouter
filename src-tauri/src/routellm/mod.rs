//! RouteLLM intelligent routing service
//!
//! Provides ML-based routing to optimize costs while maintaining quality.
//! Uses a BERT classifier to predict whether a prompt needs a "strong"
//! (expensive, high-quality) or "weak" (cheap, good-enough) model.
//!
//! Key features:
//! - 30-60% cost savings with 85-95% quality retention
//! - Fully local (no API calls)
//! - Fast inference (~15-20ms with Candle)
//! - Auto-unload on idle to manage memory (~2.5-3 GB when loaded)
//! - Pure Rust implementation using Candle framework
//! - Downloads SafeTensors directly from HuggingFace

pub mod candle_router;
pub mod downloader;
pub mod errors;
pub mod memory;
pub mod router;
pub mod status;

use crate::config::paths;
use crate::routellm::errors::{RouteLLMError, RouteLLMResult};
use crate::routellm::router::RouterWrapper;
pub use crate::routellm::status::{RouteLLMState, RouteLLMStatus, RouteLLMTestResult};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{RwLock, Mutex};
use tracing::info;

/// Global RouteLLM service state
pub struct RouteLLMService {
    /// Loaded router (if initialized)
    router: Arc<RwLock<Option<RouterWrapper>>>,

    /// Initialization lock (prevents concurrent initialization)
    init_lock: Arc<Mutex<()>>,

    /// Initialization state flag
    is_initializing: Arc<RwLock<bool>>,

    /// Last access time (for idle timeout)
    pub last_access: Arc<RwLock<Option<Instant>>>,

    /// Model paths (directories containing SafeTensors files)
    model_path: PathBuf,
    tokenizer_path: PathBuf,

    /// Idle timeout in seconds (stored in Arc<RwLock> so it can be updated at runtime)
    idle_timeout_secs: Arc<RwLock<u64>>,
}

impl RouteLLMService {
    /// Create a new RouteLLM service with custom paths
    pub fn new(model_path: PathBuf, tokenizer_path: PathBuf, idle_timeout_secs: u64) -> Self {
        Self {
            router: Arc::new(RwLock::new(None)),
            init_lock: Arc::new(Mutex::new(())),
            is_initializing: Arc::new(RwLock::new(false)),
            last_access: Arc::new(RwLock::new(None)),
            model_path,
            tokenizer_path,
            idle_timeout_secs: Arc::new(RwLock::new(idle_timeout_secs)),
        }
    }

    /// Create a new RouteLLM service with default paths
    pub fn new_with_defaults(idle_timeout_secs: u64) -> RouteLLMResult<Self> {
        let config_dir = paths::config_dir()
            .map_err(|e| RouteLLMError::Internal(format!("Failed to get config dir: {}", e)))?;

        let routellm_dir = config_dir.join("routellm");

        // Paths for SafeTensors models (directories, not files)
        let model_path = routellm_dir.join("model");        // Contains model.safetensors
        let tokenizer_path = routellm_dir.join("tokenizer"); // Contains tokenizer.json

        Ok(Self::new(model_path, tokenizer_path, idle_timeout_secs))
    }

    /// Initialize the router (loads models into memory)
    pub async fn initialize(&self) -> RouteLLMResult<()> {
        // Acquire initialization lock to prevent concurrent initialization
        let _lock = self.init_lock.lock().await;

        // Check again if already initialized (another task might have initialized while we waited)
        if self.router.read().await.is_some() {
            return Ok(()); // Already initialized
        }

        // Check if already initializing (shouldn't happen with lock, but defensive)
        if *self.is_initializing.read().await {
            return Ok(()); // Already being initialized
        }

        // Set initializing flag
        *self.is_initializing.write().await = true;

        info!("Initializing RouteLLM Candle router");

        // Check if model files exist
        // After first load, original model.safetensors is deleted and only patched version remains
        let model_file = self.model_path.join("model.safetensors");
        let patched_model_file = self.model_path.join("model.patched.safetensors");
        let tokenizer_file = self.tokenizer_path.join("tokenizer.json");

        if !model_file.exists() && !patched_model_file.exists() {
            *self.is_initializing.write().await = false;
            return Err(RouteLLMError::ModelNotDownloaded(format!(
                "Model not found at {:?} or {:?}",
                model_file, patched_model_file
            )));
        }

        if !tokenizer_file.exists() {
            *self.is_initializing.write().await = false;
            return Err(RouteLLMError::ModelNotDownloaded(format!(
                "Tokenizer not found at {:?}",
                tokenizer_file
            )));
        }

        // Load router (blocking operation, ~1.5-2s with Candle)
        let model_path = self.model_path.clone();
        let tokenizer_path = self.tokenizer_path.clone();

        let result = tokio::task::spawn_blocking(move || {
            RouterWrapper::new(&model_path, &tokenizer_path)
        })
        .await
        .map_err(|e| RouteLLMError::Internal(format!("Task join error: {}", e)))?;

        // Clear initializing flag before checking result
        *self.is_initializing.write().await = false;

        // Handle initialization result
        let router = result?;

        *self.router.write().await = Some(router);
        *self.last_access.write().await = Some(Instant::now());

        info!("RouteLLM Candle router initialized successfully");
        Ok(())
    }

    /// Predict strong model win rate
    ///
    /// Returns: (is_strong, win_rate)
    /// - is_strong: true if win_rate >= 0.5 (should use strong model)
    /// - win_rate: probability between 0.0 and 1.0
    pub async fn predict(&self, prompt: &str) -> RouteLLMResult<(bool, f32)> {
        // Initialize if needed
        if self.router.read().await.is_none() {
            self.initialize().await?;
        }

        // Calculate win rate while holding the read lock
        // We use block_in_place to avoid blocking the async runtime
        let prompt_owned = prompt.to_string();
        let win_rate = {
            let router_guard = self.router.read().await;
            let router = router_guard
                .as_ref()
                .ok_or_else(|| RouteLLMError::Internal("Router not initialized".into()))?;

            // Run the blocking calculation
            // Since this is just ~10ms, we can afford to hold the lock
            router.calculate_strong_win_rate(&prompt_owned)?
        };

        // Update last access time
        *self.last_access.write().await = Some(Instant::now());

        Ok((win_rate >= 0.5, win_rate))
    }

    /// Predict with custom threshold
    ///
    /// Returns: (is_strong, win_rate)
    /// - is_strong: true if win_rate >= threshold
    /// - win_rate: probability between 0.0 and 1.0
    pub async fn predict_with_threshold(
        &self,
        prompt: &str,
        threshold: f32,
    ) -> RouteLLMResult<(bool, f32)> {
        let (_, win_rate) = self.predict(prompt).await?;
        Ok((win_rate >= threshold, win_rate))
    }

    /// Manually unload models from memory
    pub async fn unload(&self) {
        info!("Unloading RouteLLM models from memory");
        *self.router.write().await = None;
        *self.last_access.write().await = None;
        *self.is_initializing.write().await = false; // Reset initialization flag
        info!("RouteLLM models unloaded");
    }

    /// Check if models are loaded
    pub async fn is_loaded(&self) -> bool {
        self.router.read().await.is_some()
    }

    /// Get current status
    pub async fn get_status(&self) -> RouteLLMStatus {
        let is_loaded = self.is_loaded().await;
        let is_initializing = *self.is_initializing.read().await;
        let last_access = *self.last_access.read().await;

        // Check if model files exist (not just directories)
        // After first load, original model.safetensors is deleted and only patched version remains
        let model_file = self.model_path.join("model.safetensors");
        let patched_model_file = self.model_path.join("model.patched.safetensors");
        let tokenizer_file = self.tokenizer_path.join("tokenizer.json");

        let model_exists = model_file.exists() || patched_model_file.exists();

        let state = if is_initializing {
            RouteLLMState::Initializing
        } else if is_loaded {
            RouteLLMState::Started
        } else if model_exists && tokenizer_file.exists() {
            RouteLLMState::DownloadedNotRunning
        } else {
            RouteLLMState::NotDownloaded
        };

        RouteLLMStatus {
            state,
            memory_usage_mb: if is_loaded { Some(2800) } else { None }, // ~2.5-3 GB with Candle
            last_access_secs_ago: last_access.map(|t| t.elapsed().as_secs()),
        }
    }

    /// Get model paths
    pub fn get_paths(&self) -> (PathBuf, PathBuf) {
        (self.model_path.clone(), self.tokenizer_path.clone())
    }

    /// Update idle timeout setting
    pub async fn set_idle_timeout(&self, timeout_secs: u64) {
        *self.idle_timeout_secs.write().await = timeout_secs;
        info!("RouteLLM idle timeout updated to {} seconds", timeout_secs);
    }

    /// Get current idle timeout setting
    pub async fn get_idle_timeout(&self) -> u64 {
        *self.idle_timeout_secs.read().await
    }

    /// Start auto-unload background task
    pub fn start_auto_unload_task(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        memory::start_auto_unload_task(self.clone())
    }
}

// Implement Send + Sync to allow sharing across threads
unsafe impl Send for RouteLLMService {}
unsafe impl Sync for RouteLLMService {}

#[cfg(test)]
mod tests;

// Re-export for tests
#[cfg(test)]
pub use memory::start_auto_unload_task;
#[cfg(test)]
pub use downloader::download_models;
