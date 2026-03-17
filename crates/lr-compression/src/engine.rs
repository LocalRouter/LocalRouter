//! Compression service managing the LLMLingua-2 Candle model lifecycle.

use crate::downloader::{self, repo_id_for_model};
use crate::model::CompressorModel;
use crate::protection;
use crate::types::*;
use lr_config::types::PromptCompressionConfig;
use lr_utils::paths;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::info;

/// Compression service managing model lifecycle.
///
/// Uses a `Mutex` (not `RwLock`) because Metal/CUDA forward passes require
/// exclusive access to the GPU command buffer — concurrent "read" operations
/// would race on the underlying device.
pub struct CompressionService {
    config: PromptCompressionConfig,
    model: Arc<Mutex<Option<CompressorModel>>>,
    model_path: PathBuf,
    tokenizer_path: PathBuf,
}

impl CompressionService {
    /// Create a new compression service
    pub fn new(config: PromptCompressionConfig) -> Result<Self, String> {
        let config_dir =
            paths::config_dir().map_err(|e| format!("Failed to get config dir: {}", e))?;
        let compression_dir = config_dir.join("compression").join(&config.model_size);
        let model_path = compression_dir.join("model");
        let tokenizer_path = compression_dir.join("tokenizer");

        Ok(Self {
            config,
            model: Arc::new(Mutex::new(None)),
            model_path,
            tokenizer_path,
        })
    }

    /// Get current status
    pub async fn get_status(&self) -> CompressionStatus {
        let downloaded = downloader::is_downloaded(&self.model_path, &self.tokenizer_path);
        let loaded = self.model.lock().await.is_some();

        let model_size_bytes = if downloaded {
            std::fs::metadata(self.model_path.join("model.safetensors"))
                .ok()
                .map(|m| m.len())
        } else {
            None
        };

        CompressionStatus {
            model_downloaded: downloaded,
            model_loaded: loaded,
            model_size_bytes,
            model_repo: repo_id_for_model(&self.config.model_size).to_string(),
        }
    }

    /// Download the model from HuggingFace
    pub async fn download(&self, app_handle: Option<tauri::AppHandle>) -> Result<(), String> {
        downloader::download_model(
            &self.model_path,
            &self.tokenizer_path,
            &self.config.model_size,
            app_handle,
        )
        .await
    }

    /// Load the model into memory (blocking — call from spawn_blocking)
    pub async fn load(&self) -> Result<(), String> {
        let mut guard = self.model.lock().await;
        if guard.is_some() {
            return Ok(());
        }

        if !downloader::is_downloaded(&self.model_path, &self.tokenizer_path) {
            return Err("Model not downloaded. Download it first.".to_string());
        }

        let model_path = self.model_path.clone();
        let tokenizer_path = self.tokenizer_path.clone();
        let model_size = self.config.model_size.clone();

        let loaded = tokio::task::spawn_blocking(move || {
            CompressorModel::new(&model_path, &tokenizer_path, &model_size)
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))??;

        *guard = Some(loaded);
        info!("Compression model loaded into memory");
        Ok(())
    }

    /// Unload model from memory
    pub async fn unload(&self) {
        *self.model.lock().await = None;
        info!("Compression model unloaded");
    }

    /// Compress a single text string (for try-it-out)
    pub async fn compress_text(
        &self,
        text: &str,
        rate: f32,
        preserve_quoted: bool,
    ) -> Result<(String, usize, usize, Vec<usize>, Vec<usize>), String> {
        // Lazy-load model if not loaded
        {
            let guard = self.model.lock().await;
            if guard.is_none() {
                drop(guard);
                self.load().await?;
            }
        }

        let text_owned = text.to_string();
        let guard = self.model.lock().await;
        let model = guard.as_ref().ok_or("Model not loaded")?;

        let protected_mask = if preserve_quoted {
            let words: Vec<&str> = text_owned.split_whitespace().collect();
            Some(protection::detect_protected_words(&words))
        } else {
            None
        };

        model.compress_text(&text_owned, rate, protected_mask.as_deref())
    }

    /// Compress chat messages for the pipeline
    #[allow(clippy::too_many_arguments)]
    pub async fn compress_messages(
        &self,
        messages: &[CompressedMessage],
        rate: f32,
        preserve_recent: u32,
        compress_system: bool,
        min_message_words: u32,
        preserve_quoted: bool,
        compression_notice: bool,
    ) -> Result<CompressionResult, String> {
        let start = Instant::now();
        let original_count = messages.len();

        // Lazy-load model
        {
            let guard = self.model.lock().await;
            if guard.is_none() {
                drop(guard);
                self.load().await?;
            }
        }

        let preserve_recent = preserve_recent as usize;

        // Split messages into: system, compressible, preserved (recent + tool)
        let mut compressed_messages: Vec<CompressedMessage> = Vec::new();
        let mut total_original = 0usize;
        let mut total_compressed = 0usize;

        let recent_start = if messages.len() > preserve_recent {
            messages.len() - preserve_recent
        } else {
            messages.len()
        };

        let guard = self.model.lock().await;
        let model = guard.as_ref().ok_or("Model not loaded")?;

        for (idx, msg) in messages.iter().enumerate() {
            let is_recent = idx >= recent_start;
            let is_system = msg.role == "system";
            let is_tool = msg.role == "tool" || msg.role == "function";

            // Preserve: recent messages, tool messages, system (unless compress_system)
            if is_recent || is_tool || (is_system && !compress_system) {
                let word_count = msg.content.split_whitespace().count();
                total_original += word_count;
                total_compressed += word_count;
                compressed_messages.push(msg.clone());
            } else {
                // Compress this message
                let word_count = msg.content.split_whitespace().count();
                total_original += word_count;

                if word_count < min_message_words as usize {
                    // Too short to compress meaningfully
                    total_compressed += word_count;
                    compressed_messages.push(msg.clone());
                } else {
                    let protected_mask = if preserve_quoted {
                        let words: Vec<&str> = msg.content.split_whitespace().collect();
                        Some(protection::detect_protected_words(&words))
                    } else {
                        None
                    };
                    let (compressed_text, _orig, comp, _kept, _protected) =
                        model.compress_text(&msg.content, rate, protected_mask.as_deref())?;
                    let (content, notice_words) = if compression_notice {
                        (format!("[abridged] {}", compressed_text), 1usize)
                    } else {
                        (compressed_text, 0)
                    };
                    total_compressed += comp + notice_words;
                    compressed_messages.push(CompressedMessage {
                        role: msg.role.clone(),
                        content,
                    });
                }
            }
        }

        let ratio = if total_compressed > 0 {
            total_original as f32 / total_compressed as f32
        } else {
            1.0
        };

        Ok(CompressionResult {
            compressed_messages,
            original_count,
            original_tokens: total_original,
            compressed_tokens: total_compressed,
            ratio,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Get the config
    pub fn config(&self) -> &PromptCompressionConfig {
        &self.config
    }

    /// Get model paths
    pub fn get_paths(&self) -> (&PathBuf, &PathBuf) {
        (&self.model_path, &self.tokenizer_path)
    }
}

// SAFETY: CompressionService's internal model Mutex serializes all GPU access.
// The remaining fields (config, paths) are inherently Send+Sync.
unsafe impl Send for CompressionService {}
unsafe impl Sync for CompressionService {}
