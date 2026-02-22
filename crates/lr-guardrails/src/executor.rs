//! Inference executors for safety models
//!
//! Two backends:
//! - `ProviderExecutor`: Routes inference through an already-configured LLM provider
//! - `LocalGgufExecutor`: Loads a GGUF model and runs inference locally via llama.cpp
//!
//! Models are cached globally in `MODEL_CACHE` so they persist across engine rebuilds
//! and aren't reloaded on every request. An idle auto-unload task can reclaim memory.

use std::collections::HashMap;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use llama_cpp_2::context::params::{KvCacheType, LlamaContextParams};
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::Special;
use llama_cpp_2::sampling::LlamaSampler;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Global llama.cpp backend — initialized once per process
static LLAMA_BACKEND: OnceCell<LlamaBackend> = OnceCell::new();

// ============================================================================
// Global Model Cache
// ============================================================================

/// A cached GGUF model with access tracking.
/// The model is stored behind `Arc` so multiple concurrent inference calls
/// (e.g. SingleCategory models checking several categories in parallel) can
/// each create their own `LlamaContext` from the shared model weights.
struct CachedModel {
    model: Arc<LlamaModel>,
    last_access: Instant,
}

/// Global cache of loaded GGUF models, keyed by file path.
///
/// Models persist across engine rebuilds so they don't need to be reloaded
/// on every config change. The idle unload task evicts entries that haven't
/// been used within the configured timeout.
struct ModelCache {
    entries: HashMap<PathBuf, CachedModel>,
}

static MODEL_CACHE: OnceCell<Mutex<ModelCache>> = OnceCell::new();

fn global_cache() -> &'static Mutex<ModelCache> {
    MODEL_CACHE.get_or_init(|| {
        Mutex::new(ModelCache {
            entries: HashMap::new(),
        })
    })
}

/// Get a shared reference to a cached model.
/// Returns `None` if not in cache (caller must load from disk).
fn cache_get(path: &PathBuf) -> Option<Arc<LlamaModel>> {
    let mut cache = global_cache().lock().ok()?;
    let entry = cache.entries.get_mut(path)?;
    entry.last_access = Instant::now();
    Some(entry.model.clone())
}

/// Insert a model into the cache.
fn cache_put(path: PathBuf, model: Arc<LlamaModel>) {
    if let Ok(mut cache) = global_cache().lock() {
        cache.entries.insert(
            path,
            CachedModel {
                model,
                last_access: Instant::now(),
            },
        );
    }
}

/// Unload all cached models that have been idle longer than `timeout_secs`.
/// Returns the number of models unloaded.
pub fn unload_idle_models(timeout_secs: u64) -> usize {
    let Ok(mut cache) = global_cache().lock() else {
        return 0;
    };
    let now = Instant::now();
    let before = cache.entries.len();
    cache.entries.retain(|path, entry| {
        let idle = now.duration_since(entry.last_access).as_secs();
        if idle > timeout_secs {
            info!(
                "Unloading idle safety model: {} (idle {}s > {}s)",
                path.display(),
                idle,
                timeout_secs
            );
            false
        } else {
            true
        }
    });
    before - cache.entries.len()
}

/// Unload all cached models immediately.
pub fn unload_all_models() {
    if let Ok(mut cache) = global_cache().lock() {
        let count = cache.entries.len();
        cache.entries.clear();
        if count > 0 {
            info!("Unloaded all {} cached safety models", count);
        }
    }
}

/// Get the number of currently cached (loaded) models.
pub fn loaded_model_count() -> usize {
    global_cache().lock().map(|c| c.entries.len()).unwrap_or(0)
}

/// Completion request sent to a provider
#[derive(Debug, Clone, Serialize)]
pub struct CompletionRequest {
    pub model: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<u32>,
}

/// Response from a provider completion
#[derive(Debug, Clone, Deserialize)]
pub struct CompletionResponse {
    pub text: String,
    pub logprobs: Option<LogprobsResult>,
}

/// Logprobs from a completion response
#[derive(Debug, Clone, Deserialize)]
pub struct LogprobsResult {
    pub tokens: Vec<TokenLogprob>,
}

/// A single token's logprob info
#[derive(Debug, Clone, Deserialize)]
pub struct TokenLogprob {
    pub token: String,
    pub logprob: f64,
    #[serde(default)]
    pub top_logprobs: Vec<TopLogprob>,
}

/// A top logprob candidate
#[derive(Debug, Clone, Deserialize)]
pub struct TopLogprob {
    pub token: String,
    pub logprob: f64,
}

/// Unified executor that can run inference via provider API or local GGUF
pub enum ModelExecutor {
    Provider(ProviderExecutor),
    Local(LocalGgufExecutor),
}

impl ModelExecutor {
    /// Run a completion and return generated text + optional logprobs
    pub async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, String> {
        match self {
            Self::Provider(executor) => executor.complete(request).await,
            Self::Local(executor) => executor.complete(request).await,
        }
    }
}

/// Executor that sends completion requests to an LLM provider API
pub struct ProviderExecutor {
    http_client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    _model_name: String,
    /// Whether to use Ollama's /api/generate endpoint instead of /v1/completions
    use_ollama_api: bool,
}

impl ProviderExecutor {
    pub fn new(
        base_url: String,
        api_key: Option<String>,
        model_name: String,
        use_ollama_api: bool,
    ) -> Self {
        Self {
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
            base_url,
            api_key,
            _model_name: model_name,
            use_ollama_api,
        }
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, String> {
        if self.use_ollama_api {
            self.complete_ollama(request).await
        } else {
            self.complete_openai(request).await
        }
    }

    /// Send completion via OpenAI-compatible /v1/completions endpoint
    async fn complete_openai(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, String> {
        let url = format!("{}/v1/completions", self.base_url.trim_end_matches('/'));

        let mut body = serde_json::json!({
            "model": request.model,
            "prompt": request.prompt,
            "max_tokens": request.max_tokens.unwrap_or(256),
            "temperature": request.temperature.unwrap_or(0.0),
            "stream": false,
        });

        if let Some(logprobs) = request.logprobs {
            body["logprobs"] = serde_json::json!(logprobs);
        }

        let mut req = self.http_client.post(&url).json(&body);
        if let Some(ref key) = self.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Provider request failed: {}", e))?;
        let status = resp.status();
        let resp_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {}", e))?;

        if !status.is_success() {
            return Err(format!("Provider returned {}: {}", status, resp_text));
        }

        let resp_json: serde_json::Value = serde_json::from_str(&resp_text)
            .map_err(|e| format!("Invalid JSON response: {}", e))?;

        // Extract text from choices[0].text
        let text = resp_json["choices"]
            .get(0)
            .and_then(|c| c["text"].as_str())
            .unwrap_or("")
            .to_string();

        // Extract logprobs if present
        let logprobs = parse_openai_logprobs(&resp_json);

        Ok(CompletionResponse { text, logprobs })
    }

    /// Send completion via Ollama's /api/generate endpoint
    async fn complete_ollama(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, String> {
        let url = format!("{}/api/generate", self.base_url.trim_end_matches('/'));

        let body = serde_json::json!({
            "model": request.model,
            "prompt": request.prompt,
            "raw": true,
            "stream": false,
            "options": {
                "temperature": request.temperature.unwrap_or(0.0),
                "num_predict": request.max_tokens.unwrap_or(256),
            }
        });

        let resp = self
            .http_client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Ollama request failed: {}", e))?;

        let status = resp.status();
        let resp_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read Ollama response: {}", e))?;

        if !status.is_success() {
            return Err(format!("Ollama returned {}: {}", status, resp_text));
        }

        let resp_json: serde_json::Value =
            serde_json::from_str(&resp_text).map_err(|e| format!("Invalid Ollama JSON: {}", e))?;

        let text = resp_json["response"].as_str().unwrap_or("").to_string();

        // Ollama doesn't return logprobs in standard /api/generate
        Ok(CompletionResponse {
            text,
            logprobs: None,
        })
    }
}

/// Initialize the global llama.cpp backend (idempotent)
fn init_backend() -> Result<&'static LlamaBackend, String> {
    LLAMA_BACKEND.get_or_try_init(|| {
        let mut backend =
            LlamaBackend::init().map_err(|e| format!("Failed to init llama backend: {e}"))?;
        backend.void_logs();
        info!("llama.cpp backend initialized");
        Ok(backend)
    })
}

// ============================================================================
// GGUF Pre-Validation
// ============================================================================

/// Validate a GGUF file before passing it to llama.cpp.
///
/// Checks:
/// - File size (minimum viable GGUF)
/// - Magic bytes (`GGUF`)
/// - Version (must be 2 or 3)
/// - Sane tensor and metadata counts
/// - Metadata: `expert_used_count <= expert_count` (any arch prefix)
/// - Metadata: `embedding_length % head_count == 0` (any arch prefix)
///
/// This catches corrupt or incompatible files that would otherwise trigger
/// a C `abort()` inside llama.cpp (uncatchable by Rust).
pub fn validate_gguf_file(path: &Path) -> Result<(), String> {
    let file_size = std::fs::metadata(path)
        .map_err(|e| format!("Cannot stat GGUF file '{}': {}", path.display(), e))?
        .len();

    // Minimum: magic(4) + version(4) + tensor_count(8) + kv_count(8) = 24 bytes
    if file_size < 24 {
        return Err(format!(
            "GGUF file too small ({} bytes, minimum 24). Likely a truncated download: {}",
            file_size,
            path.display()
        ));
    }

    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("Cannot open GGUF file '{}': {}", path.display(), e))?;

    // --- Magic bytes (4 bytes: "GGUF") ---
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)
        .map_err(|e| format!("Failed to read GGUF magic bytes: {}", e))?;
    if &magic != b"GGUF" {
        return Err(format!(
            "Not a GGUF file (magic: {:?}, expected: GGUF): {}",
            std::str::from_utf8(&magic).unwrap_or("???"),
            path.display()
        ));
    }

    // --- Version (u32 LE) ---
    let mut buf4 = [0u8; 4];
    file.read_exact(&mut buf4)
        .map_err(|e| format!("Failed to read GGUF version: {}", e))?;
    let version = u32::from_le_bytes(buf4);
    if !(2..=3).contains(&version) {
        return Err(format!(
            "Unsupported GGUF version {} (expected 2 or 3): {}",
            version,
            path.display()
        ));
    }

    // --- Tensor count (u64 LE) ---
    let mut buf8 = [0u8; 8];
    file.read_exact(&mut buf8)
        .map_err(|e| format!("Failed to read GGUF tensor count: {}", e))?;
    let tensor_count = u64::from_le_bytes(buf8);
    if tensor_count > 100_000 {
        return Err(format!(
            "GGUF tensor count {} is unreasonably large. File is likely corrupt: {}",
            tensor_count,
            path.display()
        ));
    }

    // --- Metadata KV count (u64 LE) ---
    file.read_exact(&mut buf8)
        .map_err(|e| format!("Failed to read GGUF metadata count: {}", e))?;
    let kv_count = u64::from_le_bytes(buf8);
    if kv_count > 100_000 {
        return Err(format!(
            "GGUF metadata count {} is unreasonably large. File is likely corrupt: {}",
            kv_count,
            path.display()
        ));
    }

    // Scan metadata for fields that trigger llama.cpp assertions.
    // Key prefixes vary by architecture (llama, granite, gemma, etc.)
    // so we match on suffixes.
    let mut expert_count: Option<u32> = None;
    let mut expert_used_count: Option<u32> = None;
    let mut embedding_length: Option<u32> = None;
    let mut head_count: Option<u32> = None;
    let mut head_count_kv: Option<u32> = None;

    for _ in 0..kv_count {
        // Key: length-prefixed string (u64 LE len + bytes)
        if file.read_exact(&mut buf8).is_err() {
            break;
        }
        let key_len = u64::from_le_bytes(buf8) as usize;
        if key_len > 1_000_000 {
            return Err(format!(
                "GGUF metadata key length {} is unreasonably large. File is likely corrupt: {}",
                key_len,
                path.display()
            ));
        }
        let mut key_buf = vec![0u8; key_len];
        if file.read_exact(&mut key_buf).is_err() {
            break;
        }
        let key = String::from_utf8_lossy(&key_buf);

        // Value type (u32 LE)
        if file.read_exact(&mut buf4).is_err() {
            break;
        }
        let value_type = u32::from_le_bytes(buf4);

        // Match target keys by suffix (architecture-agnostic).
        // All target fields are u32 (GGUF type 4).
        let is_expert_count =
            key.ends_with(".expert_count") && !key.ends_with(".expert_used_count");
        let is_expert_used = key.ends_with(".expert_used_count");
        let is_embedding_length = key.ends_with(".embedding_length");
        let is_head_count =
            key.ends_with(".attention.head_count") && !key.ends_with(".head_count_kv");
        let is_head_count_kv = key.ends_with(".attention.head_count_kv");
        let is_target_key = is_expert_count
            || is_expert_used
            || is_embedding_length
            || is_head_count
            || is_head_count_kv;

        if is_target_key && value_type == 4 {
            // GGUF_TYPE_UINT32 = 4
            if file.read_exact(&mut buf4).is_err() {
                break;
            }
            let val = u32::from_le_bytes(buf4);
            if is_expert_count {
                expert_count = Some(val);
            } else if is_expert_used {
                expert_used_count = Some(val);
            } else if is_embedding_length {
                embedding_length = Some(val);
            } else if is_head_count {
                head_count = Some(val);
            } else if is_head_count_kv {
                head_count_kv = Some(val);
            }
        } else {
            // Skip value based on type
            let skip_size = match value_type {
                0 => 1,  // UINT8
                1 => 1,  // INT8
                2 => 2,  // UINT16
                3 => 2,  // INT16
                4 => 4,  // UINT32
                5 => 4,  // INT32
                6 => 4,  // FLOAT32
                7 => 1,  // BOOL
                10 => 8, // UINT64
                11 => 8, // INT64
                12 => 8, // FLOAT64
                8 => {
                    // STRING: u64 len + bytes
                    if file.read_exact(&mut buf8).is_err() {
                        break;
                    }
                    let slen = u64::from_le_bytes(buf8) as usize;
                    if file.seek(SeekFrom::Current(slen as i64)).is_err() {
                        break;
                    }
                    continue;
                }
                9 => {
                    // ARRAY: element_type (u32) + count (u64) + elements
                    if file.read_exact(&mut buf4).is_err() {
                        break;
                    }
                    let elem_type = u32::from_le_bytes(buf4);
                    if file.read_exact(&mut buf8).is_err() {
                        break;
                    }
                    let count = u64::from_le_bytes(buf8);

                    // Element size for fixed-width types
                    let elem_size: Option<u64> = match elem_type {
                        0 | 1 | 7 => Some(1),  // UINT8/INT8/BOOL
                        2 | 3 => Some(2),       // UINT16/INT16
                        4..=6 => Some(4),   // UINT32/INT32/FLOAT32
                        10..=12 => Some(8), // UINT64/INT64/FLOAT64
                        _ => None,               // STRING/ARRAY — nested, skip rest
                    };

                    if let Some(sz) = elem_size {
                        if file.seek(SeekFrom::Current((count * sz) as i64)).is_err() {
                            break;
                        }
                    } else {
                        // Nested arrays or string arrays: skip element-by-element
                        if elem_type == 8 {
                            // STRING array
                            for _ in 0..count {
                                if file.read_exact(&mut buf8).is_err() {
                                    break;
                                }
                                let slen = u64::from_le_bytes(buf8) as i64;
                                if file.seek(SeekFrom::Current(slen)).is_err() {
                                    break;
                                }
                            }
                        } else {
                            // Nested arrays — too complex, stop scanning
                            break;
                        }
                    }
                    continue;
                }
                _ => break, // Unknown type — stop scanning
            };
            if file.seek(SeekFrom::Current(skip_size)).is_err() {
                break;
            }
        }
    }

    // --- Validate metadata invariants (mirrors llama.cpp GGML_ASSERTs) ---

    // expert_used_count <= expert_count
    if let (Some(used), Some(total)) = (expert_used_count, expert_count) {
        if used > total {
            return Err(format!(
                "Corrupt GGUF file: expert_used_count ({}) > expert_count ({}). \
                 This file will crash llama.cpp. Delete and re-download: {}",
                used, total, path.display()
            ));
        }
    }

    // embedding_length must be divisible by head_count
    if let (Some(embd), Some(heads)) = (embedding_length, head_count) {
        if heads == 0 {
            return Err(format!(
                "Corrupt GGUF file: head_count is 0. \
                 Delete and re-download: {}",
                path.display()
            ));
        }
        if embd % heads != 0 {
            return Err(format!(
                "Corrupt GGUF file: embedding_length ({}) not divisible by head_count ({}). \
                 Delete and re-download: {}",
                embd, heads, path.display()
            ));
        }
    }

    // head_count_kv must be divisible by expert_count (for MoE models)
    if let (Some(hkv), Some(experts)) = (head_count_kv, expert_count) {
        if experts > 0 && hkv % experts != 0 {
            return Err(format!(
                "Corrupt GGUF file: head_count_kv ({}) not divisible by expert_count ({}). \
                 Delete and re-download: {}",
                hkv, experts, path.display()
            ));
        }
    }

    Ok(())
}

/// Executor that loads a GGUF model locally via llama.cpp.
///
/// Models are cached globally behind `Arc` so they persist across engine rebuilds
/// and support parallel inference. Multiple concurrent calls (e.g. SingleCategory
/// models like Granite Guardian checking 7 categories at once) each create their
/// own `LlamaContext` from the shared model weights — no serialization needed.
pub struct LocalGgufExecutor {
    model_path: PathBuf,
    /// Maximum context window size in tokens (default: 512).
    /// The actual context allocated per request is right-sized to the prompt length.
    context_size: u32,
}

impl LocalGgufExecutor {
    /// Create a new executor. Validates the GGUF file and loads it eagerly
    /// into the global cache if not already present.
    ///
    /// Returns `Err` if the file fails pre-validation (corrupt, bad metadata)
    /// or if llama.cpp fails to load it.
    pub fn new(model_path: PathBuf, context_size: u32) -> Result<Self, String> {
        let context_size = context_size.clamp(256, 4096);
        if model_path.exists() {
            // Pre-validate before handing to llama.cpp (which can abort on bad files)
            validate_gguf_file(&model_path)?;
            Self::ensure_cached(&model_path)?;
        } else {
            debug!(
                "GGUF model path does not exist yet: {}",
                model_path.display()
            );
        }
        Ok(Self {
            model_path,
            context_size,
        })
    }

    fn load_model(path: &PathBuf) -> Result<Arc<LlamaModel>, String> {
        let backend = init_backend()?;
        // Offload all layers to GPU (Metal on macOS, CUDA on Linux/Windows)
        // 999 = "all layers" — llama.cpp caps to the actual layer count
        let params = LlamaModelParams::default().with_n_gpu_layers(999);
        info!("Loading GGUF model with GPU offload: {}", path.display());
        let model = LlamaModel::load_from_file(backend, path, &params)
            .map_err(|e| format!("Failed to load GGUF model: {e}"))?;
        Ok(Arc::new(model))
    }

    /// Warm the model into the global cache if not already loaded.
    fn ensure_cached(path: &PathBuf) -> Result<(), String> {
        let in_cache = global_cache()
            .lock()
            .map(|c| c.entries.contains_key(path))
            .unwrap_or(false);
        if in_cache {
            return Ok(());
        }
        let model = Self::load_model(path)?;
        cache_put(path.clone(), model);
        info!("Loaded GGUF model into cache: {}", path.display());
        Ok(())
    }

    /// Get a shared reference to the model, loading from cache or disk.
    fn get_model(&self) -> Result<Arc<LlamaModel>, String> {
        if let Some(model) = cache_get(&self.model_path) {
            return Ok(model);
        }
        // Not in cache — validate before reloading (catches corrupt files
        // that would abort() inside llama.cpp)
        validate_gguf_file(&self.model_path)?;
        let model = Self::load_model(&self.model_path)?;
        cache_put(self.model_path.clone(), model.clone());
        Ok(model)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, String> {
        let max_tokens = request.max_tokens.unwrap_or(128) as usize;
        let want_logprobs = request.logprobs.is_some();
        let prompt = request.prompt.clone();
        let context_size = self.context_size;

        // Get shared model ref — concurrent callers all get the same Arc
        let model = self.get_model()?;

        // Each call creates its own LlamaContext from the shared model,
        // allowing true parallel inference for SingleCategory models.
        tokio::task::spawn_blocking(move || {
            Self::run_inference(&model, &prompt, max_tokens, want_logprobs, context_size)
        })
        .await
        .map_err(|e| format!("Inference task panicked: {e}"))?
    }

    fn run_inference(
        model: &LlamaModel,
        prompt: &str,
        max_tokens: usize,
        want_logprobs: bool,
        max_context_size: u32,
    ) -> Result<CompletionResponse, String> {
        // Tokenize prompt first so we can right-size the context.
        // Use AddBos::Never because model prompt templates (Llama Guard, Nemotron)
        // already include <|begin_of_text|> (the BOS token).
        let mut tokens = model
            .str_to_token(prompt, llama_cpp_2::model::AddBos::Never)
            .map_err(|e| format!("Tokenization failed: {e}"))?;

        // Truncate prompt if it exceeds available context (reserve space for generation).
        // Uses middle truncation: keeps the first half and last half of tokens so that
        // both the system prompt/template and the most recent user messages are preserved.
        let max_prompt_tokens = (max_context_size as usize).saturating_sub(max_tokens);
        if tokens.len() > max_prompt_tokens {
            warn!(
                "Prompt ({} tokens) exceeds context budget ({} - {} gen = {} max prompt), truncating from middle",
                tokens.len(), max_context_size, max_tokens, max_prompt_tokens
            );
            let head = max_prompt_tokens / 2;
            let tail = max_prompt_tokens - head;
            let tail_start = tokens.len() - tail;
            let mut truncated = tokens[..head].to_vec();
            truncated.extend_from_slice(&tokens[tail_start..]);
            tokens = truncated;
        }

        // Right-size the context to actual need: prompt tokens + generation headroom,
        // rounded up to the next multiple of 256 for allocation alignment.
        // Capped at the configured maximum to respect the user's memory budget.
        let needed = (tokens.len() + max_tokens) as u32;
        let context_size = round_up_to(needed, 256).max(256).min(max_context_size);

        debug!(
            "Tokenized prompt: {} tokens (ctx={}, max_ctx={})",
            tokens.len(),
            context_size,
            max_context_size
        );

        // Create context optimized for safety classifiers:
        // - flash_attn=enabled: significant speedup on Metal/CUDA GPU
        // - Q8_0 KV cache: ~2x less memory vs F16 default, negligible quality impact
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(context_size))
            .with_n_batch(context_size)
            .with_flash_attention_policy(1) // LLAMA_FLASH_ATTN_TYPE_ENABLED = 1
            .with_type_k(KvCacheType::Q8_0)
            .with_type_v(KvCacheType::Q8_0);

        let mut ctx = model
            .new_context(init_backend()?, ctx_params)
            .map_err(|e| format!("Failed to create context: {e}"))?;

        if tokens.is_empty() {
            return Ok(CompletionResponse {
                text: String::new(),
                logprobs: None,
            });
        }

        // Feed prompt tokens in batch
        let mut batch = LlamaBatch::new(context_size as usize, 1);
        let last_idx = tokens.len() - 1;
        for (i, &token) in tokens.iter().enumerate() {
            let logits = i == last_idx; // only need logits for the last prompt token
            batch
                .add(token, i as i32, &[0], logits)
                .map_err(|e| format!("Failed to add token to batch: {e}"))?;
        }

        ctx.decode(&mut batch)
            .map_err(|e| format!("Prompt decode failed: {e}"))?;

        // Set up greedy sampler (temperature 0 for safety classifiers)
        let mut sampler =
            LlamaSampler::chain_simple([LlamaSampler::temp(0.0), LlamaSampler::greedy()]);

        let mut output_tokens = Vec::new();
        let mut logprob_entries: Vec<TokenLogprob> = Vec::new();
        let mut n_cur = tokens.len();

        let eos = model.token_eos();

        for _ in 0..max_tokens {
            // Sample next token
            let new_token = sampler.sample(&ctx, (batch.n_tokens() - 1) as i32);
            sampler.accept(new_token);

            // Check for end of generation
            if model.is_eog_token(new_token) {
                break;
            }

            // Extract logprobs if requested (from the logits before sampling)
            if want_logprobs {
                let logits = ctx.get_logits_ith((batch.n_tokens() - 1) as i32);
                let entry = Self::extract_token_logprobs(model, new_token, logits, eos);
                logprob_entries.push(entry);
            }

            output_tokens.push(new_token);

            // Prepare batch for next token
            batch.clear();
            batch
                .add(new_token, n_cur as i32, &[0], true)
                .map_err(|e| format!("Failed to add generated token: {e}"))?;

            ctx.decode(&mut batch)
                .map_err(|e| format!("Decode failed: {e}"))?;

            n_cur += 1;
        }

        // Decode output tokens to text
        let mut text = String::new();
        for &token in &output_tokens {
            let piece = model
                .token_to_str(token, Special::Tokenize)
                .map_err(|e| format!("Token decode failed: {e}"))?;
            text.push_str(&piece);
        }

        debug!(
            "Generated {} tokens: {:?}",
            output_tokens.len(),
            text.trim()
        );

        let logprobs = if want_logprobs && !logprob_entries.is_empty() {
            Some(LogprobsResult {
                tokens: logprob_entries,
            })
        } else {
            None
        };

        Ok(CompletionResponse { text, logprobs })
    }

    /// Extract logprob info for a sampled token from raw logits
    fn extract_token_logprobs(
        model: &LlamaModel,
        sampled_token: llama_cpp_2::token::LlamaToken,
        logits: &[f32],
        _eos: llama_cpp_2::token::LlamaToken,
    ) -> TokenLogprob {
        let n_vocab = logits.len();

        // Compute log-softmax for the sampled token
        let max_logit = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let sum_exp: f32 = logits.iter().map(|&l| (l - max_logit).exp()).sum();
        let log_sum_exp = max_logit + sum_exp.ln();

        let sampled_logprob = logits[sampled_token.0 as usize] - log_sum_exp;

        let token_str = model
            .token_to_str(sampled_token, Special::Tokenize)
            .unwrap_or_else(|_| format!("<token_{}>", sampled_token.0));

        // Find top-5 tokens by logit value for top_logprobs
        let mut indexed: Vec<(usize, f32)> = logits.iter().copied().enumerate().collect();
        indexed.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let top_logprobs: Vec<TopLogprob> = indexed
            .iter()
            .take(5.min(n_vocab))
            .map(|&(idx, logit)| {
                let lp = logit - log_sum_exp;
                let tok = llama_cpp_2::token::LlamaToken(idx as i32);
                let tok_str = model
                    .token_to_str(tok, Special::Tokenize)
                    .unwrap_or_else(|_| format!("<token_{}>", idx));
                TopLogprob {
                    token: tok_str,
                    logprob: lp as f64,
                }
            })
            .collect();

        TokenLogprob {
            token: token_str,
            logprob: sampled_logprob as f64,
            top_logprobs,
        }
    }

    /// Check if the model files exist on disk
    pub fn is_available(&self) -> bool {
        self.model_path.exists()
    }

    /// Get the model file path
    pub fn model_path(&self) -> &PathBuf {
        &self.model_path
    }
}

/// Parse OpenAI-format logprobs from a completion response
fn parse_openai_logprobs(resp: &serde_json::Value) -> Option<LogprobsResult> {
    let logprobs = resp["choices"].get(0)?.get("logprobs")?;
    if logprobs.is_null() {
        return None;
    }

    // OpenAI format: logprobs.tokens, logprobs.token_logprobs, logprobs.top_logprobs
    let tokens = logprobs.get("tokens")?.as_array()?;
    let token_logprobs = logprobs.get("token_logprobs")?.as_array()?;
    let top_logprobs_arr = logprobs.get("top_logprobs").and_then(|t| t.as_array());

    let mut result_tokens = Vec::new();
    for (i, (token, logprob)) in tokens.iter().zip(token_logprobs.iter()).enumerate() {
        let token_str = token.as_str().unwrap_or("").to_string();
        let lp = logprob.as_f64().unwrap_or(f64::NEG_INFINITY);

        let mut top = Vec::new();
        if let Some(arr) = &top_logprobs_arr {
            if let Some(top_map) = arr.get(i).and_then(|v| v.as_object()) {
                for (t, l) in top_map {
                    top.push(TopLogprob {
                        token: t.clone(),
                        logprob: l.as_f64().unwrap_or(f64::NEG_INFINITY),
                    });
                }
            }
        }

        result_tokens.push(TokenLogprob {
            token: token_str,
            logprob: lp,
            top_logprobs: top,
        });
    }

    Some(LogprobsResult {
        tokens: result_tokens,
    })
}

/// Extract Yes/No probability from logprobs
///
/// Used by ShieldGemma and Granite Guardian which output Yes/No tokens.
/// Returns the probability of "Yes" (unsafe) as a value 0.0-1.0.
pub fn extract_yes_probability(logprobs: &LogprobsResult) -> Option<f32> {
    // Look at the first token's top logprobs
    let first = logprobs.tokens.first()?;

    let mut yes_logprob: Option<f64> = None;
    let mut no_logprob: Option<f64> = None;

    // Check the token itself
    let normalized = first.token.trim().to_lowercase();
    if normalized == "yes" {
        yes_logprob = Some(first.logprob);
    } else if normalized == "no" {
        no_logprob = Some(first.logprob);
    }

    // Check top logprobs for the other token
    for top in &first.top_logprobs {
        let t = top.token.trim().to_lowercase();
        if t == "yes" && yes_logprob.is_none() {
            yes_logprob = Some(top.logprob);
        } else if t == "no" && no_logprob.is_none() {
            no_logprob = Some(top.logprob);
        }
    }

    // If we have both, compute softmax
    match (yes_logprob, no_logprob) {
        (Some(y), Some(n)) => {
            // softmax: P(yes) = exp(y) / (exp(y) + exp(n))
            let max = y.max(n);
            let exp_y = (y - max).exp();
            let exp_n = (n - max).exp();
            Some((exp_y / (exp_y + exp_n)) as f32)
        }
        (Some(_), None) => Some(1.0), // Only "Yes" found
        (None, Some(_)) => Some(0.0), // Only "No" found
        (None, None) => None,         // Neither found
    }
}

/// Round `value` up to the next multiple of `multiple`.
fn round_up_to(value: u32, multiple: u32) -> u32 {
    value.div_ceil(multiple) * multiple
}

/// Fallback: determine Yes/No from generated text when logprobs unavailable
pub fn parse_yes_no_text(text: &str) -> Option<bool> {
    let trimmed = text.trim().to_lowercase();
    if trimmed.starts_with("yes") {
        Some(true)
    } else if trimmed.starts_with("no") {
        Some(false)
    } else {
        warn!("Could not parse Yes/No from model output: {}", text);
        None
    }
}
