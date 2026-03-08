use serde::{Deserialize, Serialize};

/// Result of compressing chat messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionResult {
    /// The compressed messages array
    pub compressed_messages: Vec<CompressedMessage>,
    /// Original message count
    pub original_count: usize,
    /// Original approximate token count (words)
    pub original_tokens: usize,
    /// Compressed approximate token count (words)
    pub compressed_tokens: usize,
    /// Compression ratio (original / compressed)
    pub ratio: f32,
    /// Compression duration in milliseconds
    pub duration_ms: u64,
}

/// A single compressed message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressedMessage {
    pub role: String,
    pub content: String,
}

/// Status of the compression model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionStatus {
    /// Whether the model files are downloaded
    pub model_downloaded: bool,
    /// Whether the model is loaded in memory
    pub model_loaded: bool,
    /// Model size on disk (bytes) if downloaded
    pub model_size_bytes: Option<u64>,
    /// HuggingFace repo ID
    pub model_repo: String,
}
