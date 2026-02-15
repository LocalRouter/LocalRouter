//! Inference executors for safety models
//!
//! Two backends:
//! - `ProviderExecutor`: Routes inference through an already-configured LLM provider
//! - `LocalGgufExecutor`: Loads a GGUF model and runs inference locally via Candle

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{debug, warn};

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
    async fn complete_openai(&self, request: CompletionRequest) -> Result<CompletionResponse, String> {
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

        let resp = req.send().await.map_err(|e| format!("Provider request failed: {}", e))?;
        let status = resp.status();
        let resp_text = resp.text().await.map_err(|e| format!("Failed to read response: {}", e))?;

        if !status.is_success() {
            return Err(format!("Provider returned {}: {}", status, resp_text));
        }

        let resp_json: serde_json::Value =
            serde_json::from_str(&resp_text).map_err(|e| format!("Invalid JSON response: {}", e))?;

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
    async fn complete_ollama(&self, request: CompletionRequest) -> Result<CompletionResponse, String> {
        let url = format!("{}/api/generate", self.base_url.trim_end_matches('/'));

        let body = serde_json::json!({
            "model": request.model,
            "prompt": request.prompt,
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

        let text = resp_json["response"]
            .as_str()
            .unwrap_or("")
            .to_string();

        // Ollama doesn't return logprobs in standard /api/generate
        Ok(CompletionResponse {
            text,
            logprobs: None,
        })
    }
}

/// Executor that loads a GGUF model locally via Candle
pub struct LocalGgufExecutor {
    model_path: PathBuf,
    /// Whether the model files have been verified to exist
    _verified: bool,
}

impl LocalGgufExecutor {
    pub fn new(model_path: PathBuf) -> Self {
        Self {
            model_path,
            _verified: false,
        }
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, String> {
        // Local GGUF inference via Candle
        // This is a placeholder - full Candle GGUF inference requires loading
        // the model weights, tokenizer, and running the generation loop.
        // For now, return an error indicating local execution is not yet supported.
        if !self.model_path.exists() {
            return Err(format!(
                "GGUF model not found at: {}",
                self.model_path.display()
            ));
        }

        debug!(
            "Local GGUF inference requested for model at: {}",
            self.model_path.display()
        );

        // TODO: Implement Candle GGUF inference
        // 1. Load tokenizer from model directory
        // 2. Load GGUF weights via candle_transformers::models::quantized_llama
        // 3. Tokenize prompt
        // 4. Run generation loop with temperature sampling
        // 5. Decode output tokens
        Err("Local GGUF inference not yet implemented. Use a provider instead.".to_string())
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
        if let Some(ref arr) = top_logprobs_arr {
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
