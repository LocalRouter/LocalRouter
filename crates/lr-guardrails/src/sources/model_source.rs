//! ML Model source for guardrail classification
//!
//! Provides ML-based guardrail classification using Candle framework.
//! Feature-gated behind the `ml-models` feature flag.
//!
//! Supported models:
//! - Meta Prompt Guard 2 (DeBERTa-v2, 86M params, 3-class: BENIGN/INJECTION/JAILBREAK)
//! - ProtectAI DeBERTa (DeBERTa-v2, 2-class: SAFE/INJECTION)
//! - jackhhao jailbreak-classifier (BERT, 2-class: benign/jailbreak)

use serde::{Deserialize, Serialize};

/// Status of a model download
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelDownloadState {
    NotDownloaded,
    Downloading,
    Ready,
    Error,
}

/// Information about a downloaded model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub source_id: String,
    pub model_name: String,
    pub model_path: String,
    pub tokenizer_path: String,
    pub state: ModelDownloadState,
    pub size_bytes: u64,
    pub error_message: Option<String>,
}

/// Supported model architectures
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelArchitecture {
    /// Standard BERT architecture (used by jackhhao jailbreak-classifier)
    Bert,
    /// DeBERTa-v2 architecture (used by Prompt Guard 2, ProtectAI)
    DebertaV2,
}

/// Extended model info for UI display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailModelInfo {
    pub source_id: String,
    pub hf_repo_id: String,
    pub architecture: ModelArchitecture,
    pub download_state: ModelDownloadState,
    pub size_bytes: u64,
    pub loaded: bool,
    pub error_message: Option<String>,
}

/// Progress of a model download
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDownloadProgress {
    pub source_id: String,
    pub current_file: Option<String>,
    pub progress: f32,
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
    pub bytes_per_second: u64,
}

/// Result of classifying a single text
#[derive(Debug, Clone)]
pub struct ClassificationResult {
    pub class_index: usize,
    pub label_name: String,
    pub confidence: f32,
    pub category: crate::types::GuardrailCategory,
    pub severity: crate::types::GuardrailSeverity,
}

// ─── Candle-based classifier (feature-gated) ───────────────────────────────

#[cfg(feature = "ml-models")]
pub use classifier::GuardrailClassifier;

#[cfg(feature = "ml-models")]
pub use classifier::LabelMapping;

#[cfg(feature = "ml-models")]
pub use classifier::parse_id2label;

#[cfg(feature = "ml-models")]
pub mod classifier {
    use crate::types::{GuardrailCategory, GuardrailMatch, GuardrailSeverity, ScanDirection};
    use candle_core::{Device, Module, Tensor};
    use candle_nn::{linear, Linear, VarBuilder};
    use candle_transformers::models::bert::{BertModel, Config as BertConfig, DTYPE};
    use candle_transformers::models::debertav2::{
        Config as DebertaV2Config, DebertaV2ContextPooler, DebertaV2Model,
    };
    use std::collections::HashMap;
    use std::path::Path;
    use tokenizers::{Tokenizer, TruncationDirection};
    use tracing::{debug, info, warn};

    /// Maps class indices to guardrail categories.
    /// None = benign class (skip), Some = detection match.
    #[derive(Debug, Clone)]
    pub struct LabelMapping {
        /// Maps class index → (label_name, category, severity). None = benign.
        pub mappings: Vec<Option<(String, GuardrailCategory, GuardrailSeverity)>>,
        /// Number of classes
        pub num_classes: usize,
    }

    impl LabelMapping {
        /// Parse label mapping from config.json's id2label field
        pub fn from_id2label(id2label: &HashMap<String, String>) -> Self {
            let max_idx = id2label
                .keys()
                .filter_map(|k| k.parse::<usize>().ok())
                .max()
                .unwrap_or(0);

            let num_classes = max_idx + 1;
            let mut mappings = vec![None; num_classes];

            for (idx_str, label) in id2label {
                let Ok(idx) = idx_str.parse::<usize>() else {
                    continue;
                };
                if idx >= num_classes {
                    continue;
                }

                let lower = label.to_lowercase();
                let mapping = match lower.as_str() {
                    "benign" | "safe" | "legit" | "legitimate" => None,
                    "injection" | "malicious" => Some((
                        label.clone(),
                        GuardrailCategory::PromptInjection,
                        GuardrailSeverity::High,
                    )),
                    "jailbreak" => Some((
                        label.clone(),
                        GuardrailCategory::JailbreakAttempt,
                        GuardrailSeverity::Critical,
                    )),
                    _ => {
                        // Unknown label — treat as potential detection if not obviously benign
                        if lower.contains("inject") || lower.contains("malicious") {
                            Some((
                                label.clone(),
                                GuardrailCategory::PromptInjection,
                                GuardrailSeverity::High,
                            ))
                        } else if lower.contains("jailbreak") {
                            Some((
                                label.clone(),
                                GuardrailCategory::JailbreakAttempt,
                                GuardrailSeverity::Critical,
                            ))
                        } else {
                            warn!("Unknown label '{}' at index {} — treating as benign", label, idx);
                            None
                        }
                    }
                };

                mappings[idx] = mapping;
            }

            Self {
                mappings,
                num_classes,
            }
        }
    }

    /// BERT-based classifier (for jackhhao jailbreak-classifier)
    pub struct BertClassifier {
        model: BertModel,
        classifier: Linear,
        tokenizer: Tokenizer,
        device: Device,
    }

    impl BertClassifier {
        fn load(
            model_dir: &Path,
            tokenizer_dir: &Path,
        ) -> Result<Self, String> {
            let device = select_device();
            debug!("BERT classifier device: {:?}", device);

            // Parse config from config.json
            let config_path = tokenizer_dir.join("config.json");
            let config: BertConfig = if config_path.exists() {
                let config_str = std::fs::read_to_string(&config_path)
                    .map_err(|e| format!("Failed to read config.json: {}", e))?;
                serde_json::from_str(&config_str)
                    .map_err(|e| format!("Failed to parse BERT config.json: {}", e))?
            } else {
                return Err("config.json not found in tokenizer directory".to_string());
            };

            let num_labels = config
                .classifier_dropout
                .map(|_| 2usize) // fallback
                .unwrap_or(2);

            // Determine num_labels from id2label in config
            let num_labels = {
                let config_str = std::fs::read_to_string(&config_path)
                    .map_err(|e| format!("Failed to re-read config.json: {}", e))?;
                let raw: serde_json::Value = serde_json::from_str(&config_str)
                    .map_err(|e| format!("Failed to parse config as JSON: {}", e))?;
                if let Some(id2label) = raw.get("id2label").and_then(|v| v.as_object()) {
                    id2label.len()
                } else {
                    num_labels
                }
            };

            // Load model weights — try safetensors first, then .bin
            let (vb, _format) = load_varbuilder(model_dir, &device)?;

            let model = BertModel::load(vb.pp("bert"), &config)
                .map_err(|e| format!("Failed to load BERT model: {}", e))?;

            let classifier = linear(config.hidden_size, num_labels, vb.pp("classifier"))
                .map_err(|e| format!("Failed to load classifier head: {}", e))?;

            let tokenizer = load_tokenizer(tokenizer_dir)?;

            Ok(Self {
                model,
                classifier,
                tokenizer,
                device,
            })
        }

        fn classify_logits(&self, text: &str) -> Result<Vec<f32>, String> {
            let (input_ids, attention_mask, token_type_ids) =
                tokenize(&self.tokenizer, text, &self.device)?;

            let bert_output = self
                .model
                .forward(&input_ids, &token_type_ids, Some(&attention_mask))
                .map_err(|e| format!("BERT forward pass failed: {}", e))?;

            // Get [CLS] token embedding
            let cls_embedding = bert_output
                .get(0)
                .map_err(|e| format!("Failed to get batch: {}", e))?
                .get(0)
                .map_err(|e| format!("Failed to get CLS token: {}", e))?
                .unsqueeze(0)
                .map_err(|e| format!("Failed to unsqueeze CLS: {}", e))?;

            let logits = self
                .classifier
                .forward(&cls_embedding)
                .map_err(|e| format!("Classifier forward failed: {}", e))?
                .squeeze(0)
                .map_err(|e| format!("Failed to squeeze logits: {}", e))?;

            softmax_vec(&logits)
        }
    }

    /// DeBERTa-v2 classifier (for Prompt Guard 2, ProtectAI)
    pub struct DebertaV2Classifier {
        model: DebertaV2Model,
        pooler: DebertaV2ContextPooler,
        classifier: Linear,
        tokenizer: Tokenizer,
        device: Device,
    }

    impl DebertaV2Classifier {
        fn load(
            model_dir: &Path,
            tokenizer_dir: &Path,
        ) -> Result<Self, String> {
            let device = select_device();
            debug!("DeBERTa-v2 classifier device: {:?}", device);

            // Parse config from config.json
            let config_path = tokenizer_dir.join("config.json");
            let config: DebertaV2Config = if config_path.exists() {
                let config_str = std::fs::read_to_string(&config_path)
                    .map_err(|e| format!("Failed to read config.json: {}", e))?;
                serde_json::from_str(&config_str)
                    .map_err(|e| format!("Failed to parse DeBERTa-v2 config.json: {}", e))?
            } else {
                return Err("config.json not found in tokenizer directory".to_string());
            };

            // Determine num_labels from id2label in config
            let num_labels = {
                let config_str = std::fs::read_to_string(&config_path)
                    .map_err(|e| format!("Failed to re-read config.json: {}", e))?;
                let raw: serde_json::Value = serde_json::from_str(&config_str)
                    .map_err(|e| format!("Failed to parse config as JSON: {}", e))?;
                raw.get("id2label")
                    .and_then(|v| v.as_object())
                    .map(|m| m.len())
                    .unwrap_or(2)
            };

            let (vb, _format) = load_varbuilder(model_dir, &device)?;

            // HF weights: deberta.embeddings.*, deberta.encoder.*, pooler.dense.*, classifier.*
            let model = DebertaV2Model::load(vb.pp("deberta"), &config)
                .map_err(|e| format!("Failed to load DeBERTa-v2 model: {}", e))?;

            let pooler = DebertaV2ContextPooler::load(vb.pp("pooler"), &config)
                .map_err(|e| format!("Failed to load DeBERTa-v2 pooler: {}", e))?;

            let output_dim = pooler
                .output_dim()
                .map_err(|e| format!("Failed to get pooler output dim: {}", e))?;

            let classifier = linear(output_dim, num_labels, vb.pp("classifier"))
                .map_err(|e| format!("Failed to load classifier head: {}", e))?;

            let tokenizer = load_tokenizer(tokenizer_dir)?;

            Ok(Self {
                model,
                pooler,
                classifier,
                tokenizer,
                device,
            })
        }

        fn classify_logits(&self, text: &str) -> Result<Vec<f32>, String> {
            let (input_ids, attention_mask, _token_type_ids) =
                tokenize(&self.tokenizer, text, &self.device)?;

            let encoder_output = self
                .model
                .forward(&input_ids, None, Some(attention_mask))
                .map_err(|e| format!("DeBERTa-v2 forward pass failed: {}", e))?;

            let pooled = self
                .pooler
                .forward(&encoder_output)
                .map_err(|e| format!("Pooler forward failed: {}", e))?;

            let logits = self
                .classifier
                .forward(&pooled)
                .map_err(|e| format!("Classifier forward failed: {}", e))?
                .squeeze(0)
                .map_err(|e| format!("Failed to squeeze logits: {}", e))?;

            softmax_vec(&logits)
        }
    }

    /// Unified guardrail classifier supporting multiple architectures
    pub enum GuardrailClassifier {
        Bert(Box<BertClassifier>),
        DebertaV2(Box<DebertaV2Classifier>),
    }

    impl GuardrailClassifier {
        /// Load a classifier from disk, dispatching by architecture
        pub fn load(
            model_dir: &Path,
            tokenizer_dir: &Path,
            source_id: &str,
            architecture: &super::ModelArchitecture,
        ) -> Result<Self, String> {
            info!(
                "Loading guardrail classifier: source={}, arch={:?}",
                source_id, architecture
            );

            match architecture {
                super::ModelArchitecture::Bert => {
                    let classifier = BertClassifier::load(model_dir, tokenizer_dir)?;
                    info!("BERT classifier loaded: {}", source_id);
                    Ok(Self::Bert(Box::new(classifier)))
                }
                super::ModelArchitecture::DebertaV2 => {
                    let classifier = DebertaV2Classifier::load(model_dir, tokenizer_dir)?;
                    info!("DeBERTa-v2 classifier loaded: {}", source_id);
                    Ok(Self::DebertaV2(Box::new(classifier)))
                }
            }
        }

        /// Classify text and return matches above the confidence threshold
        pub fn classify(
            &self,
            text: &str,
            threshold: f32,
            source_id: &str,
            source_label: &str,
            label_mapping: &LabelMapping,
        ) -> Result<Vec<GuardrailMatch>, String> {
            let probs = match self {
                Self::Bert(c) => c.classify_logits(text)?,
                Self::DebertaV2(c) => c.classify_logits(text)?,
            };

            debug!(
                "Classifier probs (source={}): {:?}",
                source_id,
                probs.iter()
                    .enumerate()
                    .map(|(i, p)| format!("class{}={:.3}", i, p))
                    .collect::<Vec<_>>()
                    .join(", ")
            );

            let mut matches = Vec::new();

            for (idx, prob) in probs.iter().enumerate() {
                if idx >= label_mapping.mappings.len() {
                    continue;
                }

                if let Some((label_name, category, severity)) = &label_mapping.mappings[idx] {
                    if *prob > threshold {
                        let rule_id_suffix = label_name.to_lowercase().replace(' ', "-");
                        matches.push(GuardrailMatch {
                            rule_id: format!("{}-{}", source_id, rule_id_suffix),
                            rule_name: format!("{} (ML)", label_name),
                            source_id: source_id.to_string(),
                            source_label: source_label.to_string(),
                            category: category.clone(),
                            severity: *severity,
                            direction: ScanDirection::Input,
                            matched_text: truncate_text(text, 100),
                            message_index: None,
                            description: format!(
                                "ML model detected {} with {:.1}% confidence",
                                label_name,
                                prob * 100.0
                            ),
                        });
                    }
                }
            }

            Ok(matches)
        }

        /// Get the source ID
        pub fn source_id(&self) -> &str {
            // Source ID is stored externally in the model manager
            ""
        }
    }

    // SAFETY: Candle tensors are thread-safe for inference
    unsafe impl Send for GuardrailClassifier {}
    unsafe impl Sync for GuardrailClassifier {}

    // ─── Shared helpers ─────────────────────────────────────────────────────

    /// Load VarBuilder from model files — tries safetensors first, then .bin
    fn load_varbuilder(
        model_dir: &Path,
        device: &Device,
    ) -> Result<(VarBuilder<'static>, &'static str), String> {
        let safetensors_path = model_dir.join("model.safetensors");
        let bin_path = model_dir.join("pytorch_model.bin");

        if safetensors_path.exists() {
            debug!("Loading weights from model.safetensors");
            let vb = unsafe {
                VarBuilder::from_mmaped_safetensors(&[safetensors_path], DTYPE, device)
                    .map_err(|e| format!("Failed to load SafeTensors: {}", e))?
            };
            Ok((vb, "safetensors"))
        } else if bin_path.exists() {
            debug!("Loading weights from pytorch_model.bin");
            let vb = VarBuilder::from_pth(&bin_path, DTYPE, device)
                .map_err(|e| format!("Failed to load PyTorch .bin: {}", e))?;
            Ok((vb, "bin"))
        } else {
            Err(format!(
                "No model weights found at {:?} (checked model.safetensors and pytorch_model.bin)",
                model_dir
            ))
        }
    }

    /// Load tokenizer from tokenizer.json
    fn load_tokenizer(tokenizer_dir: &Path) -> Result<Tokenizer, String> {
        let tokenizer_file = tokenizer_dir.join("tokenizer.json");
        if !tokenizer_file.exists() {
            return Err(format!(
                "Tokenizer file not found at {:?}",
                tokenizer_file
            ));
        }
        Tokenizer::from_file(&tokenizer_file)
            .map_err(|e| format!("Failed to load tokenizer: {}", e))
    }

    /// Tokenize text for model input (truncates to 512 tokens)
    fn tokenize(
        tokenizer: &Tokenizer,
        text: &str,
        device: &Device,
    ) -> Result<(Tensor, Tensor, Tensor), String> {
        let mut encoding = tokenizer
            .encode(text, true)
            .map_err(|e| format!("Tokenization failed: {}", e))?;

        const MAX_TOKENS: usize = 512;
        if encoding.get_ids().len() > MAX_TOKENS {
            encoding.truncate(MAX_TOKENS, 0, TruncationDirection::Right);
        }

        let input_ids = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask();

        let input_ids_tensor = Tensor::new(input_ids, device)
            .map_err(|e| format!("Failed to create input tensor: {}", e))?
            .unsqueeze(0)
            .map_err(|e| format!("Failed to unsqueeze: {}", e))?;

        let attention_mask_tensor = Tensor::new(attention_mask, device)
            .map_err(|e| format!("Failed to create attention mask: {}", e))?
            .unsqueeze(0)
            .map_err(|e| format!("Failed to unsqueeze: {}", e))?;

        let token_type_ids = vec![0u32; input_ids.len()];
        let token_type_ids_tensor = Tensor::new(&token_type_ids[..], device)
            .map_err(|e| format!("Failed to create token_type_ids: {}", e))?
            .unsqueeze(0)
            .map_err(|e| format!("Failed to unsqueeze: {}", e))?;

        Ok((input_ids_tensor, attention_mask_tensor, token_type_ids_tensor))
    }

    /// Parse id2label from config.json
    pub fn parse_id2label(tokenizer_dir: &Path) -> Result<HashMap<String, String>, String> {
        let config_path = tokenizer_dir.join("config.json");
        if !config_path.exists() {
            return Err("config.json not found".to_string());
        }

        let config_str = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config.json: {}", e))?;
        let raw: serde_json::Value = serde_json::from_str(&config_str)
            .map_err(|e| format!("Failed to parse config.json: {}", e))?;

        let id2label = raw
            .get("id2label")
            .and_then(|v| v.as_object())
            .ok_or_else(|| "No id2label in config.json".to_string())?;

        let mut result = HashMap::new();
        for (k, v) in id2label {
            if let Some(label) = v.as_str() {
                result.insert(k.clone(), label.to_string());
            }
        }

        if result.is_empty() {
            return Err("id2label is empty in config.json".to_string());
        }

        Ok(result)
    }

    /// Select the best available device (Metal > CUDA > CPU)
    fn select_device() -> Device {
        #[cfg(target_os = "macos")]
        {
            match Device::new_metal(0) {
                Ok(device) => {
                    info!("Using Metal GPU for guardrail model");
                    return device;
                }
                Err(e) => {
                    info!("Metal not available: {}, falling back to CPU", e);
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if candle_core::utils::cuda_is_available() {
                match Device::new_cuda(0) {
                    Ok(device) => {
                        info!("Using CUDA GPU for guardrail model");
                        return device;
                    }
                    Err(e) => {
                        info!("CUDA init failed: {}, falling back to CPU", e);
                    }
                }
            }
        }
        Device::Cpu
    }

    /// Numerically stable softmax
    fn softmax_vec(logits: &Tensor) -> Result<Vec<f32>, String> {
        let values = logits
            .to_vec1::<f32>()
            .map_err(|e| format!("Failed to extract logits: {}", e))?;

        if values.is_empty() {
            return Err("Empty logits".to_string());
        }

        let max_val = values.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
        let exp_values: Vec<f32> = values.iter().map(|&v| (v - max_val).exp()).collect();
        let sum: f32 = exp_values.iter().sum();

        if sum == 0.0 {
            return Err("Softmax sum is zero".to_string());
        }

        Ok(exp_values.iter().map(|&v| v / sum).collect())
    }

    /// Truncate text for display
    fn truncate_text(text: &str, max_len: usize) -> String {
        if text.len() <= max_len {
            text.to_string()
        } else {
            format!("{}...", &text[..max_len])
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_softmax_equal() {
            let device = Device::Cpu;
            let logits = Tensor::new(&[1.0f32, 1.0, 1.0], &device).unwrap();
            let probs = softmax_vec(&logits).unwrap();
            assert_eq!(probs.len(), 3);
            for p in &probs {
                assert!((p - 0.333).abs() < 0.01);
            }
        }

        #[test]
        fn test_softmax_dominant() {
            let device = Device::Cpu;
            let logits = Tensor::new(&[-10.0f32, 0.0, 10.0], &device).unwrap();
            let probs = softmax_vec(&logits).unwrap();
            assert!(probs[0] < 0.01);
            assert!(probs[2] > 0.99);
        }

        #[test]
        fn test_softmax_numerical_stability() {
            let device = Device::Cpu;
            let logits = Tensor::new(&[100.0f32, 200.0, 300.0], &device).unwrap();
            let probs = softmax_vec(&logits).unwrap();
            assert!(!probs[0].is_nan());
            assert!(!probs[1].is_nan());
            assert!(!probs[2].is_nan());
        }

        #[test]
        fn test_truncate_text() {
            assert_eq!(truncate_text("short", 10), "short");
            assert_eq!(truncate_text("this is a longer text", 10), "this is a ...");
        }

        #[test]
        fn test_label_mapping_prompt_guard_2() {
            let mut id2label = HashMap::new();
            id2label.insert("0".to_string(), "BENIGN".to_string());
            id2label.insert("1".to_string(), "INJECTION".to_string());
            id2label.insert("2".to_string(), "JAILBREAK".to_string());

            let mapping = LabelMapping::from_id2label(&id2label);
            assert_eq!(mapping.num_classes, 3);
            assert!(mapping.mappings[0].is_none()); // BENIGN
            assert!(mapping.mappings[1].is_some()); // INJECTION
            assert!(mapping.mappings[2].is_some()); // JAILBREAK

            let (_, cat1, sev1) = mapping.mappings[1].as_ref().unwrap();
            assert_eq!(*cat1, GuardrailCategory::PromptInjection);
            assert_eq!(*sev1, GuardrailSeverity::High);

            let (_, cat2, sev2) = mapping.mappings[2].as_ref().unwrap();
            assert_eq!(*cat2, GuardrailCategory::JailbreakAttempt);
            assert_eq!(*sev2, GuardrailSeverity::Critical);
        }

        #[test]
        fn test_label_mapping_binary_safe_injection() {
            let mut id2label = HashMap::new();
            id2label.insert("0".to_string(), "SAFE".to_string());
            id2label.insert("1".to_string(), "INJECTION".to_string());

            let mapping = LabelMapping::from_id2label(&id2label);
            assert_eq!(mapping.num_classes, 2);
            assert!(mapping.mappings[0].is_none()); // SAFE
            assert!(mapping.mappings[1].is_some()); // INJECTION
        }

        #[test]
        fn test_label_mapping_binary_benign_jailbreak() {
            let mut id2label = HashMap::new();
            id2label.insert("0".to_string(), "benign".to_string());
            id2label.insert("1".to_string(), "jailbreak".to_string());

            let mapping = LabelMapping::from_id2label(&id2label);
            assert_eq!(mapping.num_classes, 2);
            assert!(mapping.mappings[0].is_none()); // benign
            assert!(mapping.mappings[1].is_some()); // jailbreak

            let (_, cat, sev) = mapping.mappings[1].as_ref().unwrap();
            assert_eq!(*cat, GuardrailCategory::JailbreakAttempt);
            assert_eq!(*sev, GuardrailSeverity::Critical);
        }
    }
}
