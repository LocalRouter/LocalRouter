//! Memsearch CLI wrapper — shells out to the `memsearch` binary.
//!
//! All embedding/search/index calls are routed through LocalRouter's
//! `/v1/embeddings` endpoint using memsearch's OpenAI-compatible provider.
//! Each client directory gets its own isolated Milvus Lite DB file.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;

/// Wrapper around the `memsearch` CLI binary.
///
/// Routes all embedding calls through LocalRouter's OpenAI-compatible
/// `/v1/embeddings` endpoint using a transient bearer token.
pub struct MemsearchCli {
    /// LocalRouter base URL (e.g., "http://localhost:33625/v1")
    pub base_url: String,
    /// Bearer token for LocalRouter auth
    pub api_key: String,
    /// Embedding model routed through LocalRouter (e.g., "ollama/nomic-embed-text").
    /// Behind RwLock so it can be updated when config changes.
    pub embedding_model: parking_lot::RwLock<String>,
}

impl Default for MemsearchCli {
    fn default() -> Self {
        Self::new()
    }
}

impl MemsearchCli {
    pub fn new() -> Self {
        Self {
            base_url: "http://localhost:3625/v1".to_string(),
            api_key: String::new(),
            embedding_model: parking_lot::RwLock::new(String::new()),
        }
    }

    /// Get the Milvus DB URI for a given working directory.
    fn milvus_uri(working_dir: &Path) -> String {
        working_dir.join("milvus.db").to_string_lossy().to_string()
    }

    /// Common embedding provider args for all commands.
    fn embedding_args(&self) -> Vec<String> {
        vec![
            "--provider".to_string(),
            "openai".to_string(),
            "--base-url".to_string(),
            self.base_url.clone(),
            "--api-key".to_string(),
            self.api_key.clone(),
            "--model".to_string(),
            self.embedding_model.read().clone(),
        ]
    }

    /// Get the current embedding model name.
    pub fn get_embedding_model(&self) -> String {
        self.embedding_model.read().clone()
    }

    /// Update the embedding model.
    pub fn set_embedding_model(&self, model: String) {
        *self.embedding_model.write() = model;
    }

    /// Check if memsearch is installed and return its version.
    pub async fn check_installed(&self) -> Result<String, String> {
        let output = Command::new("memsearch")
            .arg("--version")
            .output()
            .await
            .map_err(|e| format!("memsearch not found: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "memsearch --version failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Check if Python is available and return its version.
    pub async fn check_python(&self) -> Result<String, String> {
        let output = Command::new("python3")
            .arg("--version")
            .output()
            .await
            .map_err(|e| format!("python3 not found: {}", e))?;

        if !output.status.success() {
            return Err("python3 --version failed".to_string());
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Search indexed memories in the given directory.
    pub async fn search(
        &self,
        sessions_dir: &Path,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>, String> {
        let working_dir = sessions_dir.parent().unwrap_or(sessions_dir);
        let mut cmd = Command::new("memsearch");
        cmd.arg("search")
            .arg(query)
            .arg("--top-k")
            .arg(top_k.to_string())
            .arg("--json-output")
            .arg("--milvus-uri")
            .arg(Self::milvus_uri(working_dir))
            .args(self.embedding_args())
            .current_dir(working_dir);

        let output = tokio::time::timeout(Duration::from_secs(30), cmd.output())
            .await
            .map_err(|_| "memsearch search timed out (30s)".to_string())?
            .map_err(|e| format!("memsearch search failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("no collection") || stderr.contains("empty") {
                return Ok(Vec::new());
            }
            return Err(format!("memsearch search failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() {
            return Ok(Vec::new());
        }

        serde_json::from_str::<Vec<SearchResult>>(&stdout)
            .or_else(|_| {
                serde_json::from_str::<SearchResultWrapper>(&stdout).map(|w| w.results)
            })
            .map_err(|e| format!("Failed to parse memsearch search output: {}", e))
    }

    /// Expand a chunk to get the full markdown section.
    pub async fn expand(
        &self,
        working_dir: &Path,
        chunk_hash: &str,
    ) -> Result<String, String> {
        let output = tokio::time::timeout(
            Duration::from_secs(10),
            Command::new("memsearch")
                .arg("expand")
                .arg(chunk_hash)
                .arg("--milvus-uri")
                .arg(Self::milvus_uri(working_dir))
                .current_dir(working_dir)
                .output(),
        )
        .await
        .map_err(|_| "memsearch expand timed out (10s)".to_string())?
        .map_err(|e| format!("memsearch expand failed: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "memsearch expand failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Index (or re-index) markdown files in the given directory.
    pub async fn index(&self, dir: &Path) -> Result<(), String> {
        let working_dir = dir.parent().unwrap_or(dir);
        let mut cmd = Command::new("memsearch");
        cmd.arg("index")
            .arg(dir.to_string_lossy().as_ref())
            .arg("--milvus-uri")
            .arg(Self::milvus_uri(working_dir))
            .args(self.embedding_args())
            .current_dir(working_dir);

        let output = tokio::time::timeout(Duration::from_secs(120), cmd.output())
            .await
            .map_err(|_| "memsearch index timed out (120s)".to_string())?
            .map_err(|e| format!("memsearch index failed: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "memsearch index failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(())
    }

    /// Compact a session file using LLM summarization.
    /// Both embedding and LLM calls are routed through LocalRouter.
    pub async fn compact(
        &self,
        working_dir: &Path,
        source: &Path,
        compaction_model: &str,
    ) -> Result<(), String> {
        let mut cmd = Command::new("memsearch");
        cmd.arg("compact")
            .arg("--source")
            .arg(source.to_string_lossy().as_ref())
            .arg("--milvus-uri")
            .arg(Self::milvus_uri(working_dir))
            // Embedding provider args (for re-indexing the summary)
            .args(self.embedding_args())
            // LLM provider args (for summarization)
            .arg("--llm-provider")
            .arg("openai")
            .arg("--llm-base-url")
            .arg(&self.base_url)
            .arg("--llm-api-key")
            .arg(&self.api_key)
            .arg("--llm-model")
            .arg(compaction_model)
            .current_dir(working_dir);

        let output = tokio::time::timeout(Duration::from_secs(120), cmd.output())
            .await
            .map_err(|_| "memsearch compact timed out (120s)".to_string())?
            .map_err(|e| format!("memsearch compact failed: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "memsearch compact failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(())
    }
}

/// A single search result from memsearch.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchResult {
    #[serde(alias = "source")]
    pub source: String,
    #[serde(alias = "heading", default)]
    pub heading: Option<String>,
    #[serde(alias = "content")]
    pub content: String,
    #[serde(alias = "chunk_hash", default)]
    pub chunk_hash: Option<String>,
    #[serde(alias = "score", default)]
    pub score: Option<f64>,
}

#[derive(Deserialize)]
struct SearchResultWrapper {
    results: Vec<SearchResult>,
}
