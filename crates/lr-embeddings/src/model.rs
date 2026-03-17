//! Candle-based sentence embedding model (all-MiniLM-L6-v2).
//!
//! Loads a BertModel from SafeTensors, runs forward pass, applies mean pooling
//! and L2 normalization to produce 384-dimensional embeddings.

use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config, HiddenAct, DTYPE};
use std::path::Path;
use tokenizers::Tokenizer;
use tracing::{debug, info};

/// Embedding dimension for all-MiniLM-L6-v2.
pub const EMBEDDING_DIM: usize = 384;

/// Sentence embedding model using all-MiniLM-L6-v2.
pub struct SentenceEmbedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

impl SentenceEmbedder {
    /// Load model from SafeTensors files in the given directory.
    pub fn new(model_dir: &Path) -> Result<Self, String> {
        info!("Loading sentence embedding model (all-MiniLM-L6-v2)");

        let device = select_device();
        debug!("Embedding device: {:?}", device);

        let config = model_config();

        let model_file = model_dir.join("model.safetensors");
        if !model_file.exists() {
            return Err(format!(
                "Embedding model file not found at {:?}",
                model_file
            ));
        }

        debug!("Loading embedding model weights from {:?}", model_file);
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[model_file], DTYPE, &device)
                .map_err(|e| format!("Failed to load SafeTensors: {}", e))?
        };

        // sentence-transformers models store weights without a "bert." prefix
        let model = BertModel::load(vb.clone(), &config)
            .or_else(|_| {
                // Fall back to "bert." prefix (standard HuggingFace BERT format)
                BertModel::load(vb.pp("bert"), &config)
            })
            .map_err(|e| format!("Failed to load BERT model: {}", e))?;

        let tokenizer_file = model_dir.join("tokenizer.json");
        if !tokenizer_file.exists() {
            return Err(format!("Tokenizer not found at {:?}", tokenizer_file));
        }
        let tokenizer = Tokenizer::from_file(&tokenizer_file)
            .map_err(|e| format!("Failed to load tokenizer: {}", e))?;

        info!("Sentence embedding model loaded successfully");
        Ok(Self {
            model,
            tokenizer,
            device,
        })
    }

    /// Embed a single text string into a 384-dimensional vector.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let results = self.embed_batch(&[text])?;
        Ok(results.into_iter().next().unwrap())
    }

    /// Embed a batch of texts into 384-dimensional vectors.
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let mut all_embeddings = Vec::with_capacity(texts.len());

        for text in texts {
            let encoding = self
                .tokenizer
                .encode(*text, true)
                .map_err(|e| format!("Tokenization failed: {}", e))?;

            let mut input_ids: Vec<u32> = encoding.get_ids().to_vec();
            let mut attention_mask: Vec<u32> = encoding.get_attention_mask().to_vec();
            let mut token_type_ids: Vec<u32> = encoding.get_type_ids().to_vec();

            // Truncate to max 512 tokens
            if input_ids.len() > 512 {
                input_ids.truncate(512);
                attention_mask.truncate(512);
                token_type_ids.truncate(512);
            }

            let seq_len = input_ids.len();

            let input_ids_t = Tensor::new(&input_ids[..], &self.device)
                .and_then(|t| t.unsqueeze(0))
                .map_err(|e| format!("Failed to create input tensor: {}", e))?;
            let attention_mask_t = Tensor::new(&attention_mask[..], &self.device)
                .and_then(|t| t.unsqueeze(0))
                .map_err(|e| format!("Failed to create attention mask: {}", e))?;
            let token_type_ids_t = Tensor::new(&token_type_ids[..], &self.device)
                .and_then(|t| t.unsqueeze(0))
                .map_err(|e| format!("Failed to create token_type_ids: {}", e))?;

            // Forward pass: [1, seq_len] → [1, seq_len, 384]
            let hidden_states = self
                .model
                .forward(&input_ids_t, &token_type_ids_t, Some(&attention_mask_t))
                .map_err(|e| format!("BERT forward pass failed: {}", e))?;

            // Mean pooling over token dimension with attention mask
            let attention_mask_f = attention_mask_t
                .to_dtype(DTYPE)
                .map_err(|e| format!("Failed to convert mask: {}", e))?;
            let mask_expanded = attention_mask_f
                .unsqueeze(2)
                .and_then(|m| m.broadcast_as((1, seq_len, EMBEDDING_DIM)))
                .map_err(|e| format!("Failed to expand mask: {}", e))?;

            let masked = hidden_states
                .mul(&mask_expanded)
                .map_err(|e| format!("Failed to apply mask: {}", e))?;
            let summed = masked.sum(1).map_err(|e| format!("Failed to sum: {}", e))?;
            let mask_sum = attention_mask_f
                .sum(1)
                .and_then(|s| s.unsqueeze(1))
                .and_then(|s| s.broadcast_as((1, EMBEDDING_DIM)))
                .and_then(|s| s.clamp(1e-9, f64::MAX))
                .map_err(|e| format!("Failed to compute mask sum: {}", e))?;

            let mean_pooled = summed
                .div(&mask_sum)
                .map_err(|e| format!("Failed mean pool: {}", e))?;

            // L2 normalize
            let embedding = mean_pooled
                .squeeze(0)
                .map_err(|e| format!("Failed to squeeze: {}", e))?;
            let norm = embedding
                .sqr()
                .and_then(|s| s.sum_all())
                .and_then(|s| s.sqrt())
                .and_then(|s| s.clamp(1e-12, f64::MAX))
                .map_err(|e| format!("Failed to compute norm: {}", e))?;
            let normalized = embedding
                .broadcast_div(&norm)
                .map_err(|e| format!("Failed to normalize: {}", e))?;

            let vec: Vec<f32> = normalized
                .to_vec1()
                .map_err(|e| format!("Failed to extract embedding: {}", e))?;
            all_embeddings.push(vec);
        }

        Ok(all_embeddings)
    }
}

/// BERT config for all-MiniLM-L6-v2
fn model_config() -> Config {
    Config {
        vocab_size: 30522,
        hidden_size: EMBEDDING_DIM,
        num_hidden_layers: 6,
        num_attention_heads: 12,
        intermediate_size: 1536,
        hidden_act: HiddenAct::Gelu,
        hidden_dropout_prob: 0.1,
        max_position_embeddings: 512,
        type_vocab_size: 2,
        initializer_range: 0.02,
        layer_norm_eps: 1e-12,
        pad_token_id: 0,
        position_embedding_type: candle_transformers::models::bert::PositionEmbeddingType::Absolute,
        use_cache: false,
        classifier_dropout: None,
        model_type: None,
    }
}

/// Select the best available compute device.
fn select_device() -> Device {
    #[cfg(target_os = "macos")]
    {
        match Device::new_metal(0) {
            Ok(device) => {
                info!("Using Metal GPU acceleration for embeddings");
                return device;
            }
            Err(e) => {
                info!("Metal not available: {}, using CPU", e);
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        if candle_core::utils::cuda_is_available() {
            if let Ok(device) = Device::new_cuda(0) {
                info!("Using CUDA GPU acceleration for embeddings");
                return device;
            }
        }
    }
    Device::Cpu
}

// SAFETY: SentenceEmbedder contains candle types (BertModel, Device) that hold
// Metal/CUDA resources without implementing Send/Sync. These are safe to move
// across threads, but concurrent access to the GPU command buffer is NOT safe.
// Callers MUST serialize access via a Mutex (see EmbeddingService).
unsafe impl Send for SentenceEmbedder {}
unsafe impl Sync for SentenceEmbedder {}
