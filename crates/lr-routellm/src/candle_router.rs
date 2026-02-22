//! Candle-based BERT classifier for RouteLLM
//!
//! This module implements a pure Rust BERT classifier using the Candle framework.
//! It loads SafeTensors models directly from HuggingFace without requiring conversion.

use crate::errors::{RouteLLMError, RouteLLMResult};
use candle_core::{Device, Module, Tensor};
use candle_nn::{linear, Linear, VarBuilder};
use candle_transformers::models::bert::{BertModel, Config, HiddenAct, DTYPE};
use safetensors::{tensor::TensorView, SafeTensors};
use std::collections::HashMap;
use std::path::Path;
use tokenizers::{Tokenizer, TruncationDirection};
use tracing::{debug, info};

/// Pad RoBERTa's token_type_embeddings from [1, 768] to [2, 768] for Candle compatibility
///
/// RoBERTa only uses one token type (type 0), but Candle's BERT implementation expects
/// at least 2 token types. We duplicate the embedding to satisfy this requirement.
///
/// **Disk Space Requirements**:
/// - Requires ~880 MB free space during patching (both files exist temporarily)
/// - After patching completes, original file is auto-deleted leaving only ~440 MB used
/// - Patched file is created once on first load, then reused
fn pad_token_type_embeddings(input_file: &Path, output_file: &Path) -> RouteLLMResult<()> {
    use std::fs;

    debug!("Reading SafeTensors from {:?}", input_file);
    let data = fs::read(input_file).map_err(|e| {
        RouteLLMError::ModelLoadingFailed(format!("Failed to read input file: {}", e))
    })?;

    let tensors = SafeTensors::deserialize(&data).map_err(|e| {
        RouteLLMError::ModelLoadingFailed(format!("Failed to deserialize SafeTensors: {}", e))
    })?;

    let token_type_key = "roberta.embeddings.token_type_embeddings.weight";

    // Create a map of all tensors, modifying token_type_embeddings
    let mut tensor_data: HashMap<String, (Vec<usize>, Vec<u8>)> = HashMap::new();

    for name in tensors.names() {
        let tensor_view = tensors.tensor(name).map_err(|e| {
            RouteLLMError::ModelLoadingFailed(format!("Failed to get tensor {}: {}", name, e))
        })?;

        if name == token_type_key {
            // Pad this tensor from [1, 768] to [2, 768]
            debug!(
                "Padding {} from {:?} to [2, 768]",
                name,
                tensor_view.shape()
            );

            // Original data is [1, 768] in f32
            let original_data = tensor_view.data();
            let mut padded_data = Vec::with_capacity(original_data.len() * 2);

            // Duplicate the data
            padded_data.extend_from_slice(original_data);
            padded_data.extend_from_slice(original_data);

            tensor_data.insert(name.to_string(), (vec![2, 768], padded_data));
        } else {
            // Copy as-is
            tensor_data.insert(
                name.to_string(),
                (tensor_view.shape().to_vec(), tensor_view.data().to_vec()),
            );
        }
    }

    // Serialize the modified tensors
    debug!("Serializing patched SafeTensors to {:?}", output_file);
    let views: Vec<(&str, TensorView)> = tensor_data
        .iter()
        .map(|(name, (shape, data))| {
            (
                name.as_str(),
                TensorView::new(safetensors::Dtype::F32, shape.clone(), data.as_slice()).unwrap(),
            )
        })
        .collect();

    let serialized = safetensors::serialize(views, &None).map_err(|e| {
        RouteLLMError::ModelLoadingFailed(format!("Failed to serialize SafeTensors: {}", e))
    })?;

    fs::write(output_file, serialized).map_err(|e| {
        RouteLLMError::ModelLoadingFailed(format!("Failed to write patched file: {}", e))
    })?;

    info!("Created patched model at {:?}", output_file);
    Ok(())
}

/// Candle-based XLM-RoBERTa classifier for RouteLLM
///
/// Note: The model is XLM-RoBERTa (not BERT base) from routellm/bert_gpt4_augmented
/// Architecture: 12 layers, 768 hidden size, 250k vocab, 3-class classifier
pub struct CandleRouter {
    model: BertModel,         // Candle's BERT model works for RoBERTa variants
    classifier_dense: Linear, // 768 → 768 (first layer)
    classifier_out: Linear,   // 768 → 3 (output layer for 3-class classification)
    tokenizer: Tokenizer,
    device: Device,
}

impl CandleRouter {
    /// Load model from SafeTensors files
    ///
    /// # Arguments
    /// * `model_path` - Path to the directory containing model.safetensors
    /// * `tokenizer_path` - Path to the directory containing tokenizer.json
    pub fn new(model_path: &Path, tokenizer_path: &Path) -> RouteLLMResult<Self> {
        info!("Loading RouteLLM model from SafeTensors");
        info!("  Model path: {:?}", model_path);
        info!("  Tokenizer path: {:?}", tokenizer_path);

        // Try to use GPU acceleration (Metal on macOS, CUDA on Linux/Windows)
        // Fall back to CPU if GPU is not available
        let device = {
            #[cfg(target_os = "macos")]
            {
                // Try Metal first on macOS
                match Device::new_metal(0) {
                    Ok(metal_device) => {
                        info!("✓ Using Metal GPU acceleration (Apple Silicon)");
                        metal_device
                    }
                    Err(e) => {
                        info!("⚠ Metal GPU not available: {}", e);
                        info!("  Falling back to CPU");
                        Device::Cpu
                    }
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                // Try CUDA on other platforms
                if candle_core::utils::cuda_is_available() {
                    match Device::new_cuda(0) {
                        Ok(cuda_device) => {
                            info!("✓ Using CUDA GPU acceleration (device 0)");
                            cuda_device
                        }
                        Err(e) => {
                            info!("⚠ CUDA available but failed to initialize: {}", e);
                            info!("  Falling back to CPU");
                            Device::Cpu
                        }
                    }
                } else {
                    info!("Using CPU (CUDA not available)");
                    Device::Cpu
                }
            }
        };
        debug!("Device: {:?}", device);

        // XLM-RoBERTa configuration (from routellm/bert_gpt4_augmented)
        let config = Config {
            vocab_size: 250002, // XLM-RoBERTa multilingual vocab
            hidden_size: 768,
            num_hidden_layers: 12,
            num_attention_heads: 12,
            intermediate_size: 3072,
            hidden_act: HiddenAct::Gelu,
            hidden_dropout_prob: 0.1,
            max_position_embeddings: 514, // XLM-RoBERTa uses 514
            type_vocab_size: 2, // Set to 2 for Candle compatibility, even though RoBERTa only uses type 0
            initializer_range: 0.02,
            layer_norm_eps: 1e-5, // Different from BERT's 1e-12
            pad_token_id: 1,      // RoBERTa PAD token
            position_embedding_type:
                candle_transformers::models::bert::PositionEmbeddingType::Absolute,
            use_cache: false,
            classifier_dropout: Some(0.1),
            model_type: None,
        };

        // Load model weights from SafeTensors
        // RoBERTa's token_type_embeddings has shape [1, 768] but Candle's BERT expects [2, 768]
        // We use a patched version of the model with padded embeddings
        let patched_model_file = model_path.join("model.patched.safetensors");
        let model_file = model_path.join("model.safetensors");

        // Check if we need to create the patched file
        if !patched_model_file.exists() {
            // Need to create the patched file from the original
            if !model_file.exists() {
                return Err(RouteLLMError::ModelNotDownloaded(format!(
                    "Model files not found. Expected either:\n  - {:?} (original), or\n  - {:?} (patched)\n\nPlease download the model first.",
                    model_file,
                    patched_model_file
                )));
            }

            info!("Creating patched model (first-time setup)");
            info!("  Note: Requires ~880 MB free disk space during patching");
            debug!("  Patching token_type_embeddings from [1, 768] to [2, 768]");
            pad_token_type_embeddings(&model_file, &patched_model_file)?;

            // Delete the original file to save disk space (we only need the patched version)
            info!("Deleting original model to save disk space...");
            if let Err(e) = std::fs::remove_file(&model_file) {
                // Log warning but don't fail - the patched file works fine
                info!(
                    "  Could not delete original model file (non-critical): {}",
                    e
                );
                info!("  Disk usage: ~880 MB (both original and patched)");
            } else {
                info!("  ✓ Original model deleted successfully");
                info!("  Final disk usage: ~440 MB (patched model only)");
            }
        } else {
            debug!("Using existing patched model at {:?}", patched_model_file);
        }

        debug!("Loading model weights from {:?}", patched_model_file);

        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[patched_model_file], DTYPE, &device).map_err(
                |e| RouteLLMError::ModelLoadingFailed(format!("Failed to load SafeTensors: {}", e)),
            )?
        };

        // Load RoBERTa model (note: prefix is "roberta" not "bert")
        debug!("Initializing RoBERTa model");
        let model = BertModel::load(vb.pp("roberta"), &config).map_err(|e| {
            RouteLLMError::ModelLoadingFailed(format!("Failed to load BERT model: {}", e))
        })?;

        // Load classification head (2-layer architecture)
        // Layer 1: dense (768 → 768)
        // Layer 2: out_proj (768 → 3) for 3-class classification
        debug!("Loading classification head");
        let classifier_vb = vb.pp("classifier");
        let classifier_dense = linear(768, 768, classifier_vb.pp("dense")).map_err(|e| {
            RouteLLMError::ModelLoadingFailed(format!("Failed to load classifier.dense: {}", e))
        })?;
        let classifier_out = linear(768, 3, classifier_vb.pp("out_proj")).map_err(|e| {
            RouteLLMError::ModelLoadingFailed(format!("Failed to load classifier.out_proj: {}", e))
        })?;

        // Load tokenizer
        let tokenizer_file = tokenizer_path.join("tokenizer.json");
        if !tokenizer_file.exists() {
            return Err(RouteLLMError::ModelNotDownloaded(format!(
                "Tokenizer file not found at {:?}",
                tokenizer_file
            )));
        }

        debug!("Loading tokenizer from {:?}", tokenizer_file);
        let tokenizer = Tokenizer::from_file(&tokenizer_file).map_err(|e| {
            RouteLLMError::ModelLoadingFailed(format!("Failed to load tokenizer: {}", e))
        })?;

        info!("RouteLLM Candle model loaded successfully");

        Ok(Self {
            model,
            classifier_dense,
            classifier_out,
            tokenizer,
            device,
        })
    }

    /// Calculate strong model win rate for a prompt
    ///
    /// Returns a value between 0.0 and 1.0:
    /// - Higher values (closer to 1.0) suggest using a strong model
    /// - Lower values (closer to 0.0) suggest using a weak model
    pub fn calculate_strong_win_rate(&self, prompt: &str) -> RouteLLMResult<f32> {
        debug!("Calculating win rate for prompt of length {}", prompt.len());

        // Tokenize input with truncation to prevent performance issues with long text
        // XLM-RoBERTa has max_position_embeddings=514, so we truncate to 512 tokens
        // (leaving room for special tokens like [CLS] and [SEP])
        let mut encoding = self
            .tokenizer
            .encode(prompt, true)
            .map_err(|e| RouteLLMError::PredictionFailed(format!("Tokenization failed: {}", e)))?;

        // Truncate to max 512 tokens if needed
        // This prevents quadratic performance degradation with long inputs
        const MAX_TOKENS: usize = 512;
        let original_len = encoding.get_ids().len();
        if original_len > MAX_TOKENS {
            debug!("Truncating from {} to {} tokens", original_len, MAX_TOKENS);
            encoding.truncate(MAX_TOKENS, 0, TruncationDirection::Right); // Keep beginning, truncate end
        }

        let input_ids = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask();

        debug!(
            "Tokenized to {} tokens (original: {}, truncated: {})",
            input_ids.len(),
            original_len,
            original_len > MAX_TOKENS
        );
        debug!(
            "Input IDs (first 20): {:?}",
            &input_ids[..input_ids.len().min(20)]
        );

        // Convert to tensors
        let input_ids_tensor = Tensor::new(input_ids, &self.device).map_err(|e| {
            RouteLLMError::PredictionFailed(format!("Failed to create input_ids tensor: {}", e))
        })?;
        let attention_mask_tensor = Tensor::new(attention_mask, &self.device).map_err(|e| {
            RouteLLMError::PredictionFailed(format!(
                "Failed to create attention_mask tensor: {}",
                e
            ))
        })?;

        // Add batch dimension
        let input_ids_tensor = input_ids_tensor.unsqueeze(0).map_err(|e| {
            RouteLLMError::PredictionFailed(format!("Failed to unsqueeze input_ids: {}", e))
        })?;
        let attention_mask_tensor = attention_mask_tensor.unsqueeze(0).map_err(|e| {
            RouteLLMError::PredictionFailed(format!("Failed to unsqueeze attention_mask: {}", e))
        })?;

        // Forward pass through RoBERTa (using BERT implementation)
        // RoBERTa doesn't use token_type_ids, but we need to provide a tensor of zeros
        // Create explicitly with u32 zeros matching input_ids shape
        debug!("Running BERT forward pass");
        debug!("Input IDs shape: {:?}", input_ids_tensor.shape());
        debug!("Attention mask shape: {:?}", attention_mask_tensor.shape());

        // Create token_type_ids as zeros with shape matching input_ids
        let seq_len = input_ids.len();
        let token_type_ids_vec = vec![0u32; seq_len];
        let token_type_ids_tensor =
            Tensor::new(&token_type_ids_vec[..], &self.device).map_err(|e| {
                RouteLLMError::PredictionFailed(format!("Failed to create token_type_ids: {}", e))
            })?;
        let token_type_ids_tensor = token_type_ids_tensor.unsqueeze(0).map_err(|e| {
            RouteLLMError::PredictionFailed(format!("Failed to unsqueeze token_type_ids: {}", e))
        })?;
        debug!("Token type IDs shape: {:?}", token_type_ids_tensor.shape());

        // BertModel.forward() signature: (input_ids, token_type_ids, attention_mask: Option)
        let bert_output = self
            .model
            .forward(
                &input_ids_tensor,
                &token_type_ids_tensor,
                Some(&attention_mask_tensor),
            )
            .map_err(|e| {
                RouteLLMError::PredictionFailed(format!("BERT forward pass failed: {}", e))
            })?;

        // Get [CLS] token embedding (first token, shape: [batch_size, seq_len, hidden_size])
        // Extract first element of batch dimension, then first token, then restore batch dimension
        let cls_embedding = bert_output
            .get(0)
            .map_err(|e| RouteLLMError::PredictionFailed(format!("Failed to get batch: {}", e)))?
            .get(0)
            .map_err(|e| {
                RouteLLMError::PredictionFailed(format!("Failed to get CLS token: {}", e))
            })?
            .unsqueeze(0)
            .map_err(|e| {
                RouteLLMError::PredictionFailed(format!("Failed to add batch dimension: {}", e))
            })?;

        // Apply classification head (2-layer: dense + tanh + out_proj)
        debug!("Applying classification head");
        let hidden = self.classifier_dense.forward(&cls_embedding).map_err(|e| {
            RouteLLMError::PredictionFailed(format!("Classifier dense forward pass failed: {}", e))
        })?;

        // Apply tanh activation
        let hidden = hidden.tanh().map_err(|e| {
            RouteLLMError::PredictionFailed(format!("Tanh activation failed: {}", e))
        })?;

        let logits = self.classifier_out.forward(&hidden).map_err(|e| {
            RouteLLMError::PredictionFailed(format!(
                "Classifier out_proj forward pass failed: {}",
                e
            ))
        })?;

        // Remove batch dimension from logits: [1, 3] → [3]
        let logits = logits.squeeze(0).map_err(|e| {
            RouteLLMError::PredictionFailed(format!("Failed to squeeze logits: {}", e))
        })?;

        // Debug: print raw logits
        let logits_vec = logits.to_vec1::<f32>().map_err(|e| {
            RouteLLMError::PredictionFailed(format!("Failed to extract logits for debug: {}", e))
        })?;
        debug!(
            "Raw logits: [{:.4}, {:.4}, {:.4}]",
            logits_vec[0], logits_vec[1], logits_vec[2]
        );

        // Apply softmax to get class probabilities
        // According to Python RouteLLM BERTRouter implementation:
        //   binary_prob = softmax[-2:] = probs[1] + probs[2] (tie + weak wins)
        //   win_rate = 1 - binary_prob = probs[0] (strong model wins)
        // So the labels are:
        //   LABEL_0: Strong model wins (tier 1 wins)
        //   LABEL_1: Tie
        //   LABEL_2: Weak model wins (tier 2 wins)
        let probs = softmax(&logits)?;

        // Return probability of LABEL_0 (strong model needed) as win_rate
        // This matches Python: win_rate = 1 - (probs[1] + probs[2]) = probs[0]
        let win_rate = probs[0];

        debug!(
            "Win rate: {:.3} (class probs: [{:.3}, {:.3}, {:.3}])",
            win_rate, probs[0], probs[1], probs[2]
        );
        Ok(win_rate)
    }
}

/// Softmax activation function
///
/// Computes: softmax(x_i) = e^(x_i - max(x)) / Σ(e^(x_j - max(x)))
///
/// Uses numerically stable formula by subtracting max value before exp
fn softmax(x: &Tensor) -> RouteLLMResult<Vec<f32>> {
    // Extract values
    let values = x
        .to_vec1::<f32>()
        .map_err(|e| RouteLLMError::PredictionFailed(format!("Failed to extract logits: {}", e)))?;

    if values.is_empty() {
        return Err(RouteLLMError::PredictionFailed(
            "Empty logits tensor".to_string(),
        ));
    }

    // Find max for numerical stability (prevents overflow)
    let max_val = values.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));

    // Compute exp(x - max) for each element
    let exp_values: Vec<f32> = values.iter().map(|&v| (v - max_val).exp()).collect();

    // Compute sum of exponentials
    let sum: f32 = exp_values.iter().sum();

    if sum == 0.0 {
        return Err(RouteLLMError::PredictionFailed(
            "Softmax sum is zero".to_string(),
        ));
    }

    // Normalize to get probabilities
    let probs: Vec<f32> = exp_values.iter().map(|&v| v / sum).collect();

    Ok(probs)
}

// Implement Send + Sync to allow sharing across threads
unsafe impl Send for CandleRouter {}
unsafe impl Sync for CandleRouter {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    #[ignore] // Only run when models are available
    fn test_candle_router_load() {
        let model_path = PathBuf::from("/tmp/routellm/model");
        let tokenizer_path = PathBuf::from("/tmp/routellm/tokenizer");

        let result = CandleRouter::new(&model_path, &tokenizer_path);
        assert!(result.is_ok(), "Failed to load model: {:?}", result.err());
    }

    #[test]
    #[ignore] // Only run when models are available
    fn test_candle_router_predict() {
        let model_path = PathBuf::from("/tmp/routellm/model");
        let tokenizer_path = PathBuf::from("/tmp/routellm/tokenizer");

        let router = CandleRouter::new(&model_path, &tokenizer_path).unwrap();
        let win_rate = router.calculate_strong_win_rate("What is 2+2?").unwrap();

        assert!(
            (0.0..=1.0).contains(&win_rate),
            "Win rate out of bounds: {}",
            win_rate
        );
    }

    #[test]
    fn test_softmax() {
        let device = Device::Cpu;

        // Test softmax with equal logits - should give equal probabilities
        let equal_logits = Tensor::new(&[1.0f32, 1.0, 1.0], &device).unwrap();
        let result = softmax(&equal_logits).unwrap();
        assert_eq!(result.len(), 3, "Should return 3 probabilities");
        for prob in &result {
            assert!(
                (prob - 0.333).abs() < 0.01,
                "Equal logits should give ~0.333, got {}",
                prob
            );
        }

        // Test sum equals 1
        let sum: f32 = result.iter().sum();
        assert!(
            (sum - 1.0).abs() < 0.001,
            "Probabilities should sum to 1.0, got {}",
            sum
        );

        // Test softmax with one dominant logit
        let dominant = Tensor::new(&[-10.0f32, 0.0, 10.0], &device).unwrap();
        let result = softmax(&dominant).unwrap();
        assert!(
            result[0] < 0.01,
            "P(LABEL_0) should be near 0, got {}",
            result[0]
        );
        assert!(
            result[2] > 0.99,
            "P(LABEL_2) should be near 1, got {}",
            result[2]
        );

        // Test numerical stability with extreme values
        let extreme = Tensor::new(&[100.0f32, 200.0, 300.0], &device).unwrap();
        let result = softmax(&extreme).unwrap();
        assert!(!result[0].is_nan(), "Softmax should not produce NaN");
        assert!(!result[1].is_nan(), "Softmax should not produce NaN");
        assert!(!result[2].is_nan(), "Softmax should not produce NaN");
        assert!(result[2] > 0.99, "Highest logit should have highest prob");

        // Test softmax with typical BERT logit range
        let typical = Tensor::new(&[-2.0f32, 0.0, 2.0], &device).unwrap();
        let result = softmax(&typical).unwrap();
        assert_eq!(result.len(), 3);
        for prob in &result {
            assert!(
                *prob >= 0.0 && *prob <= 1.0,
                "Probability {} not in [0, 1]",
                prob
            );
        }
        let sum: f32 = result.iter().sum();
        assert!(
            (sum - 1.0).abs() < 0.001,
            "Probabilities should sum to 1.0, got {}",
            sum
        );
    }
}
