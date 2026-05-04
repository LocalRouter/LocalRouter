//! Model downloader from HuggingFace for all-MiniLM-L6-v2

use crate::progress::DownloadProgress;
use hf_hub::api::tokio::Api;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::{debug, info, warn};

static DOWNLOAD_LOCK: once_cell::sync::Lazy<Arc<Mutex<()>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(())));

const DOWNLOAD_TIMEOUT_SECS: u64 = 600;
const MAX_RETRIES: usize = 3;
const RETRY_DELAY_MS: u64 = 2000;

/// HuggingFace repo for the embedding model
pub const REPO_ID: &str = "sentence-transformers/all-MiniLM-L6-v2";

/// Files needed for the embedding model
const MODEL_FILES: &[&str] = &["model.safetensors", "tokenizer.json"];

/// Download embedding model from HuggingFace.
///
/// `progress` is invoked at file-boundary events: once before the
/// first file with `(0, total)`, again after each file copy with
/// `(completed, total)`, and finally `on_complete` after all files
/// are present. Pass `None` for silent download (headless / non-UI
/// consumers).
pub async fn download_model(
    model_dir: &Path,
    progress: Option<&dyn DownloadProgress>,
) -> Result<(), String> {
    let lock_result = DOWNLOAD_LOCK.try_lock();
    if lock_result.is_err() {
        return Err("Another embedding download is already in progress".to_string());
    }
    let _lock = lock_result.unwrap();

    info!("Downloading embedding model from HuggingFace");
    info!("  Repo: {}", REPO_ID);
    info!("  Dir: {:?}", model_dir);

    tokio::fs::create_dir_all(model_dir)
        .await
        .map_err(|e| format!("Failed to create model directory: {}", e))?;

    let total = MODEL_FILES.len() as u32;
    if let Some(p) = progress {
        p.on_progress("model.safetensors", 0, total);
    }

    let api = Api::new().map_err(|e| format!("Failed to initialize HuggingFace API: {}", e))?;
    let repo = api.model(REPO_ID.to_string());

    for (idx, file) in MODEL_FILES.iter().enumerate() {
        info!("Downloading {}...", file);
        let downloaded = download_with_retry(&repo, file).await?;
        let dest = model_dir.join(file);
        tokio::fs::copy(&downloaded, &dest)
            .await
            .map_err(|e| format!("Failed to copy {}: {}", file, e))?;

        if let Some(p) = progress {
            p.on_progress(file, (idx + 1) as u32, total);
        }
    }

    if let Some(p) = progress {
        p.on_complete();
    }

    info!("Embedding model downloaded successfully");
    Ok(())
}

/// Check if model files are present
pub fn is_downloaded(model_dir: &Path) -> bool {
    MODEL_FILES.iter().all(|f| model_dir.join(f).exists())
}

/// Download a file with retry logic
async fn download_with_retry(
    repo: &hf_hub::api::tokio::ApiRepo,
    filename: &str,
) -> Result<std::path::PathBuf, String> {
    let mut last_error = None;

    for attempt in 1..=MAX_RETRIES {
        let result = timeout(
            Duration::from_secs(DOWNLOAD_TIMEOUT_SECS),
            repo.get(filename),
        )
        .await;

        match result {
            Ok(Ok(path)) => {
                debug!("Downloaded {} on attempt {}", filename, attempt);
                return Ok(path);
            }
            Ok(Err(e)) => {
                warn!("Download {} attempt {} failed: {}", filename, attempt, e);
                last_error = Some(format!("{}", e));
            }
            Err(_) => {
                warn!("Download {} attempt {} timed out", filename, attempt);
                last_error = Some("Timed out".to_string());
            }
        }

        if attempt < MAX_RETRIES {
            tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
        }
    }

    Err(format!(
        "Failed to download {} after {} attempts: {}",
        filename,
        MAX_RETRIES,
        last_error.unwrap_or_else(|| "Unknown".to_string())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct RecordingReporter {
        events: StdMutex<Vec<(String, u32, u32)>>,
        completed: StdMutex<bool>,
    }

    impl DownloadProgress for RecordingReporter {
        fn on_progress(&self, current_file: &str, completed_files: u32, total_files: u32) {
            self.events.lock().unwrap().push((
                current_file.to_string(),
                completed_files,
                total_files,
            ));
        }
        fn on_complete(&self) {
            *self.completed.lock().unwrap() = true;
        }
    }

    /// Trait shape lock — calling the methods on a `&dyn DownloadProgress`
    /// must dispatch to the impl, and `Send + Sync` must hold (otherwise
    /// `download_model`'s future stops being `Send`).
    #[test]
    fn reporter_records_calls() {
        fn assert_send_sync<T: Send + Sync>(_: &T) {}
        let r = RecordingReporter::default();
        assert_send_sync(&r);

        let p: &dyn DownloadProgress = &r;
        p.on_progress("model.safetensors", 0, 2);
        p.on_progress("model.safetensors", 1, 2);
        p.on_progress("tokenizer.json", 2, 2);
        p.on_complete();

        let events = r.events.lock().unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], ("model.safetensors".to_string(), 0, 2));
        assert_eq!(events[2], ("tokenizer.json".to_string(), 2, 2));
        assert!(*r.completed.lock().unwrap());
    }
}
