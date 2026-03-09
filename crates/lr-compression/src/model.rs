//! Candle-based BERT token classifier for LLMLingua-2 prompt compression.
//!
//! Loads a BertForTokenClassification model from SafeTensors and classifies
//! each token as keep (label 1) or drop (label 0).

use candle_core::{Device, Module, Tensor};
use candle_nn::{linear, Linear, VarBuilder};
use candle_transformers::models::bert::{BertModel, Config, HiddenAct, DTYPE};
use std::path::Path;
use tokenizers::Tokenizer;
use tracing::{debug, info};

/// BERT token classifier for LLMLingua-2
pub struct CompressorModel {
    model: BertModel,
    classifier: Linear, // hidden_size → 2 (drop, keep)
    tokenizer: Tokenizer,
    device: Device,
}

impl CompressorModel {
    /// Load model from SafeTensors files
    pub fn new(model_path: &Path, tokenizer_path: &Path, model_size: &str) -> Result<Self, String> {
        info!("Loading LLMLingua-2 compression model ({})", model_size);

        let device = select_device();
        debug!("Device: {:?}", device);

        let config = model_config(model_size);

        let model_file = model_path.join("model.safetensors");
        if !model_file.exists() {
            return Err(format!("Model file not found at {:?}", model_file));
        }

        debug!("Loading model weights from {:?}", model_file);
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[model_file], DTYPE, &device)
                .map_err(|e| format!("Failed to load SafeTensors: {}", e))?
        };

        // Load BERT encoder (weight prefix: "bert." for BERT, "roberta." for XLM-RoBERTa)
        let weight_prefix = if model_size == "xlm-roberta" {
            "roberta"
        } else {
            "bert"
        };
        let model = BertModel::load(vb.pp(weight_prefix), &config)
            .map_err(|e| format!("Failed to load BERT model: {}", e))?;

        // Load token classification head: Linear(hidden_size, 2)
        let classifier = linear(config.hidden_size, 2, vb.pp("classifier"))
            .map_err(|e| format!("Failed to load classifier: {}", e))?;

        // Load tokenizer
        let tokenizer_file = tokenizer_path.join("tokenizer.json");
        if !tokenizer_file.exists() {
            return Err(format!("Tokenizer not found at {:?}", tokenizer_file));
        }
        let tokenizer = Tokenizer::from_file(&tokenizer_file)
            .map_err(|e| format!("Failed to load tokenizer: {}", e))?;

        info!("LLMLingua-2 model loaded successfully");
        Ok(Self {
            model,
            classifier,
            tokenizer,
            device,
        })
    }

    /// Compress text by keeping tokens with highest keep probability.
    ///
    /// `rate` is the fraction of tokens to keep (0.0-1.0). Lower = more compression.
    /// Returns (compressed_text, original_token_count, compressed_token_count, kept_word_indices).
    pub fn compress_text(
        &self,
        text: &str,
        rate: f32,
    ) -> Result<(String, usize, usize, Vec<usize>), String> {
        if text.trim().is_empty() {
            return Ok((text.to_string(), 0, 0, vec![]));
        }

        // Split into words for word-level compression (cleaner output)
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.is_empty() {
            return Ok((text.to_string(), 0, 0, vec![]));
        }

        let original_word_count = words.len();
        let keep_count = (original_word_count as f32 * rate).ceil().max(1.0) as usize;

        if keep_count >= original_word_count {
            let all_indices: Vec<usize> = (0..original_word_count).collect();
            return Ok((
                text.to_string(),
                original_word_count,
                original_word_count,
                all_indices,
            ));
        }

        // Tokenize each word separately to map subwords → words
        let mut all_input_ids: Vec<u32> = vec![101]; // [CLS]
        let mut word_boundaries: Vec<(usize, usize)> = Vec::new(); // (start, end) in token sequence

        for word in &words {
            let encoding = self
                .tokenizer
                .encode(*word, false)
                .map_err(|e| format!("Tokenization failed: {}", e))?;
            let ids = encoding.get_ids();
            let start = all_input_ids.len();
            all_input_ids.extend_from_slice(ids);
            let end = all_input_ids.len();
            word_boundaries.push((start, end));
        }
        all_input_ids.push(102); // [SEP]

        // Truncate to max 512 tokens
        if all_input_ids.len() > 512 {
            all_input_ids.truncate(512);
            // Fix boundaries that go beyond truncation
            for boundary in &mut word_boundaries {
                if boundary.0 >= 512 {
                    boundary.0 = 511;
                    boundary.1 = 511;
                } else if boundary.1 > 512 {
                    boundary.1 = 512;
                }
            }
        }

        let seq_len = all_input_ids.len();
        let attention_mask: Vec<u32> = vec![1u32; seq_len];
        let token_type_ids: Vec<u32> = vec![0u32; seq_len];

        // Create tensors
        let input_ids_t = Tensor::new(&all_input_ids[..], &self.device)
            .and_then(|t| t.unsqueeze(0))
            .map_err(|e| format!("Failed to create input tensor: {}", e))?;
        let attention_mask_t = Tensor::new(&attention_mask[..], &self.device)
            .and_then(|t| t.unsqueeze(0))
            .map_err(|e| format!("Failed to create attention mask: {}", e))?;
        let token_type_ids_t = Tensor::new(&token_type_ids[..], &self.device)
            .and_then(|t| t.unsqueeze(0))
            .map_err(|e| format!("Failed to create token_type_ids: {}", e))?;

        // Forward pass: [1, seq_len] → [1, seq_len, 768]
        let hidden_states = self
            .model
            .forward(&input_ids_t, &token_type_ids_t, Some(&attention_mask_t))
            .map_err(|e| format!("BERT forward pass failed: {}", e))?;

        // Apply classifier: [1, seq_len, 768] → [1, seq_len, 2]
        let logits = self
            .classifier
            .forward(&hidden_states)
            .map_err(|e| format!("Classifier forward pass failed: {}", e))?;

        // Extract logits: [seq_len, 2]
        let logits = logits
            .squeeze(0)
            .map_err(|e| format!("Failed to squeeze: {}", e))?;
        let logits_vec: Vec<Vec<f32>> = logits
            .to_vec2()
            .map_err(|e| format!("Failed to extract logits: {}", e))?;

        // Compute per-word keep probability (mean of subword token probs)
        let mut word_keep_probs: Vec<(usize, f32)> = Vec::new();
        for (word_idx, (start, end)) in word_boundaries.iter().enumerate() {
            if start >= end || *start >= logits_vec.len() {
                word_keep_probs.push((word_idx, 0.0));
                continue;
            }
            let mut sum_prob = 0.0;
            let mut count = 0;
            for logits in logits_vec
                .iter()
                .take(*end.min(&logits_vec.len()))
                .skip(*start)
            {
                let logit_keep = logits[1];
                let logit_drop = logits[0];
                // Softmax for 2 classes: P(keep) = exp(keep) / (exp(keep) + exp(drop))
                let max_logit = logit_keep.max(logit_drop);
                let p_keep = (logit_keep - max_logit).exp()
                    / ((logit_keep - max_logit).exp() + (logit_drop - max_logit).exp());
                sum_prob += p_keep;
                count += 1;
            }
            let avg_prob = if count > 0 {
                sum_prob / count as f32
            } else {
                0.0
            };
            word_keep_probs.push((word_idx, avg_prob));
        }

        // Sort by probability (descending) and keep top N
        let mut ranked = word_keep_probs.clone();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut keep_indices: Vec<usize> = ranked
            .iter()
            .take(keep_count)
            .map(|(idx, _)| *idx)
            .collect();
        keep_indices.sort(); // Restore original order

        let compressed: Vec<&str> = keep_indices.iter().map(|&idx| words[idx]).collect();
        let compressed_text = compressed.join(" ");

        debug!(
            "Compressed {} → {} words (rate={:.2})",
            original_word_count,
            keep_indices.len(),
            rate
        );

        Ok((
            compressed_text,
            original_word_count,
            keep_indices.len(),
            keep_indices,
        ))
    }
}

/// Get BERT config for the given model size
fn model_config(model_size: &str) -> Config {
    match model_size {
        "xlm-roberta" => Config {
            vocab_size: 250102,
            hidden_size: 1024,
            num_hidden_layers: 24,
            num_attention_heads: 16,
            intermediate_size: 4096,
            hidden_act: HiddenAct::Gelu,
            hidden_dropout_prob: 0.1,
            max_position_embeddings: 514,
            type_vocab_size: 1,
            initializer_range: 0.02,
            layer_norm_eps: 1e-5,
            pad_token_id: 1,
            position_embedding_type:
                candle_transformers::models::bert::PositionEmbeddingType::Absolute,
            use_cache: false,
            classifier_dropout: None,
            model_type: None,
        },
        _ => Config {
            // BERT Base Multilingual Cased
            vocab_size: 119647,
            hidden_size: 768,
            num_hidden_layers: 12,
            num_attention_heads: 12,
            intermediate_size: 3072,
            hidden_act: HiddenAct::Gelu,
            hidden_dropout_prob: 0.1,
            max_position_embeddings: 512,
            type_vocab_size: 2,
            initializer_range: 0.02,
            layer_norm_eps: 1e-12,
            pad_token_id: 0,
            position_embedding_type:
                candle_transformers::models::bert::PositionEmbeddingType::Absolute,
            use_cache: false,
            classifier_dropout: None,
            model_type: None,
        },
    }
}

/// Select the best available compute device
fn select_device() -> Device {
    #[cfg(target_os = "macos")]
    {
        match Device::new_metal(0) {
            Ok(device) => {
                info!("Using Metal GPU acceleration for compression");
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
                info!("Using CUDA GPU acceleration for compression");
                return device;
            }
        }
    }
    Device::Cpu
}

unsafe impl Send for CompressorModel {}
unsafe impl Sync for CompressorModel {}
