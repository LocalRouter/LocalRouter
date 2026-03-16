//! Memsearch CLI wrapper — shells out to the `memsearch` binary.
//!
//! All commands pass `--provider` and `--milvus-uri` via CLI args so no
//! `.memsearch.toml` config file is needed. Each client gets its own
//! isolated Milvus Lite DB file to prevent lock conflicts.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;

/// Wrapper around the `memsearch` CLI binary.
///
/// The `provider` field is passed as `--provider` to all embedding commands.
/// Each call also passes `--milvus-uri` pointing to a local DB file in the
/// working directory, preventing conflicts with the global `~/.memsearch/milvus.db`.
pub struct MemsearchCli {
    /// Embedding provider name (e.g., "local", "ollama", "openai")
    pub provider: String,
}

impl Default for MemsearchCli {
    fn default() -> Self {
        Self::new()
    }
}

impl MemsearchCli {
    pub fn new() -> Self {
        Self {
            provider: "local".to_string(),
        }
    }

    /// Create with a specific embedding provider.
    pub fn with_provider(provider: String) -> Self {
        Self { provider }
    }

    /// Get the Milvus DB URI for a given working directory.
    /// Each directory gets its own isolated DB file.
    fn milvus_uri(working_dir: &Path) -> String {
        working_dir.join("milvus.db").to_string_lossy().to_string()
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
        let output = tokio::time::timeout(
            Duration::from_secs(30),
            Command::new("memsearch")
                .arg("search")
                .arg(query)
                .arg("--top-k")
                .arg(top_k.to_string())
                .arg("--json-output")
                .arg("--provider")
                .arg(&self.provider)
                .arg("--milvus-uri")
                .arg(Self::milvus_uri(working_dir))
                .current_dir(working_dir)
                .output(),
        )
        .await
        .map_err(|_| "memsearch search timed out (30s)".to_string())?
        .map_err(|e| format!("memsearch search failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Empty index is not an error
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
                // Try parsing as a wrapper object
                serde_json::from_str::<SearchResultWrapper>(&stdout).map(|w| w.results)
            })
            .map_err(|e| format!("Failed to parse memsearch search output: {}", e))
    }

    /// Expand a chunk to get the full markdown section (progressive disclosure L2).
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
        let output = tokio::time::timeout(
            Duration::from_secs(120),
            Command::new("memsearch")
                .arg("index")
                .arg(dir.to_string_lossy().as_ref())
                .arg("--provider")
                .arg(&self.provider)
                .arg("--milvus-uri")
                .arg(Self::milvus_uri(working_dir))
                .current_dir(working_dir)
                .output(),
        )
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
    pub async fn compact(
        &self,
        working_dir: &Path,
        source: &Path,
        llm_provider: &str,
    ) -> Result<(), String> {
        let output = tokio::time::timeout(
            Duration::from_secs(120),
            Command::new("memsearch")
                .arg("compact")
                .arg("--source")
                .arg(source.to_string_lossy().as_ref())
                .arg("--llm-provider")
                .arg(llm_provider)
                .arg("--milvus-uri")
                .arg(Self::milvus_uri(working_dir))
                .current_dir(working_dir)
                .output(),
        )
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
    /// Source file path
    #[serde(alias = "source")]
    pub source: String,
    /// Section heading
    #[serde(alias = "heading", default)]
    pub heading: Option<String>,
    /// Content preview
    #[serde(alias = "content")]
    pub content: String,
    /// Chunk hash (for expand)
    #[serde(alias = "chunk_hash", default)]
    pub chunk_hash: Option<String>,
    /// Relevance score
    #[serde(alias = "score", default)]
    pub score: Option<f64>,
}

/// Wrapper for search results (memsearch may return { results: [...] })
#[derive(Deserialize)]
struct SearchResultWrapper {
    results: Vec<SearchResult>,
}
