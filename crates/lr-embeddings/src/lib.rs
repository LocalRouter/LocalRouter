//! Sentence embeddings via all-MiniLM-L6-v2 (Candle).
//!
//! Provides `EmbeddingService` for generating 384-dimensional sentence embeddings
//! using a local BERT model. Supports Metal (macOS), CUDA, and CPU backends.

pub mod downloader;
pub mod model;

use model::SentenceEmbedder;
use parking_lot::RwLock;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;

pub use model::EMBEDDING_DIM;

/// Embedding service managing the sentence embedding model lifecycle.
///
/// Shared across all ContentStore instances via `Arc<EmbeddingService>`.
pub struct EmbeddingService {
    model: Arc<RwLock<Option<SentenceEmbedder>>>,
    model_dir: PathBuf,
}

impl EmbeddingService {
    /// Create a new embedding service.
    ///
    /// `config_dir` is the app config root (e.g., `~/.localrouter/`).
    /// Model files are stored in `{config_dir}/embeddings/all-MiniLM-L6-v2/`.
    pub fn new(config_dir: &Path) -> Self {
        let model_dir = config_dir.join("embeddings").join("all-MiniLM-L6-v2");
        Self {
            model: Arc::new(RwLock::new(None)),
            model_dir,
        }
    }

    /// Check if the model files have been downloaded.
    pub fn is_downloaded(&self) -> bool {
        downloader::is_downloaded(&self.model_dir)
    }

    /// Download the model from HuggingFace.
    pub async fn download(&self, app_handle: Option<tauri::AppHandle>) -> Result<(), String> {
        downloader::download_model(&self.model_dir, app_handle).await
    }

    /// Load the model into memory. No-op if already loaded.
    pub fn ensure_loaded(&self) -> Result<(), String> {
        if self.model.read().is_some() {
            return Ok(());
        }

        if !self.is_downloaded() {
            return Err("Embedding model not downloaded. Download it first.".to_string());
        }

        info!("Loading embedding model from {:?}", self.model_dir);
        let embedder = SentenceEmbedder::new(&self.model_dir)?;
        *self.model.write() = Some(embedder);
        Ok(())
    }

    /// Whether the model is currently loaded in memory.
    pub fn is_loaded(&self) -> bool {
        self.model.read().is_some()
    }

    /// Embed a single text into a 384-dimensional vector.
    ///
    /// Automatically loads the model if not yet loaded.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        self.ensure_loaded()?;
        let guard = self.model.read();
        guard.as_ref().unwrap().embed(text)
    }

    /// Embed a batch of texts into 384-dimensional vectors.
    ///
    /// Automatically loads the model if not yet loaded.
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        self.ensure_loaded()?;
        let guard = self.model.read();
        guard.as_ref().unwrap().embed_batch(texts)
    }

    /// Embedding dimension (384 for all-MiniLM-L6-v2).
    pub fn dimension(&self) -> usize {
        EMBEDDING_DIM
    }

    /// Path to the model directory.
    pub fn model_dir(&self) -> &Path {
        &self.model_dir
    }

    /// Model size on disk in bytes (if downloaded).
    pub fn model_size_bytes(&self) -> Option<u64> {
        let model_file = self.model_dir.join("model.safetensors");
        std::fs::metadata(model_file).ok().map(|m| m.len())
    }
}
