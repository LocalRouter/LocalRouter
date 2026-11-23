//! Model downloader from HuggingFace for LLMLingua-2

use hf_hub::api::tokio::Api;
use hf_hub::{Repo, RepoType};
use lr_utils::model_manifest::{self, ManifestFile, ModelManifest};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::{debug, info, warn};

static DOWNLOAD_LOCK: once_cell::sync::Lazy<Arc<Mutex<()>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(())));

const DOWNLOAD_TIMEOUT_SECS: u64 = 600;
const MAX_RETRIES: usize = 3;
const RETRY_DELAY_MS: u64 = 2000;

/// Get the HuggingFace repo ID for a model size
pub fn repo_id_for_model(model_size: &str) -> &'static str {
    match model_size {
        "xlm-roberta" => "microsoft/llmlingua-2-xlm-roberta-large-meetingbank",
        _ => "microsoft/llmlingua-2-bert-base-multilingual-cased-meetingbank",
    }
}

/// Download LLMLingua-2 model from HuggingFace
pub async fn download_model(
    model_path: &Path,
    tokenizer_path: &Path,
    model_size: &str,
    app_handle: Option<AppHandle>,
) -> Result<(), String> {
    let lock_result = DOWNLOAD_LOCK.try_lock();
    if lock_result.is_err() {
        return Err("Another download is already in progress".to_string());
    }
    let _lock = lock_result.unwrap();

    let repo_id = repo_id_for_model(model_size);
    info!("Downloading LLMLingua-2 model from HuggingFace");
    info!("  Repo: {}", repo_id);
    info!("  Model dir: {:?}", model_path);
    info!("  Tokenizer dir: {:?}", tokenizer_path);

    // Create directories
    tokio::fs::create_dir_all(model_path)
        .await
        .map_err(|e| format!("Failed to create model directory: {}", e))?;
    tokio::fs::create_dir_all(tokenizer_path)
        .await
        .map_err(|e| format!("Failed to create tokenizer directory: {}", e))?;

    if let Some(ref handle) = app_handle {
        let _ = handle.emit(
            "compression-download-progress",
            serde_json::json!({ "progress": 0.0, "current_file": "model.safetensors" }),
        );
    }

    let api = Api::new().map_err(|e| format!("Failed to initialize HuggingFace API: {}", e))?;
    let mut repo = api.model(repo_id.to_string());

    // Everything is fetched into the Hugging Face cache first, so a failed
    // download leaves an installed model as it was.
    // Download model.safetensors (BERT ~709 MB, XLM-RoBERTa ~2.2 GB)
    info!("Downloading model.safetensors...");
    let model_downloaded = download_with_retry(&repo, MODEL_FILES[0]).await?;
    // The tokenizer files are pinned to the commit the weights came from.
    let revision = model_manifest::revision_of_cached_file(&model_downloaded);
    if let Some(revision) = &revision {
        repo = api.repo(Repo::with_revision(
            repo_id.to_string(),
            RepoType::Model,
            revision.clone(),
        ));
    }
    let new_manifest = || ModelManifest {
        repo: repo_id.to_string(),
        revision: revision.clone(),
        files: Default::default(),
    };
    if let Some(ref handle) = app_handle {
        let _ = handle.emit(
            "compression-download-progress",
            serde_json::json!({ "progress": 0.7, "current_file": "tokenizer.json" }),
        );
    }

    let tokenizer_files = tokenizer_files_for(model_size);
    let mut tokenizer_downloads = Vec::with_capacity(tokenizer_files.len());
    for (idx, file) in tokenizer_files.iter().enumerate() {
        debug!("Downloading {}", file);
        tokenizer_downloads.push((*file, download_with_retry(&repo, file).await?));

        let progress = 0.7 + 0.3 * (idx + 1) as f32 / tokenizer_files.len() as f32;
        if let Some(ref handle) = app_handle {
            let _ = handle.emit(
                "compression-download-progress",
                serde_json::json!({ "progress": progress, "current_file": file }),
            );
        }
    }

    // From here until the new manifests are written, both directories count
    // as incomplete.
    for dir in [model_path, tokenizer_path] {
        model_manifest::begin_update(dir)
            .map_err(|e| format!("Failed to prepare {}: {}", dir.display(), e))?;
    }
    let mut model_manifest_data = new_manifest();
    let entry = install_file(model_downloaded, model_path.join(MODEL_FILES[0]))
        .await
        .map_err(|e| format!("Failed to copy model: {}", e))?;
    model_manifest_data
        .files
        .insert(MODEL_FILES[0].to_string(), entry);
    let mut tokenizer_manifest = new_manifest();
    for (file, cached) in tokenizer_downloads {
        let entry = install_file(cached, tokenizer_path.join(file))
            .await
            .map_err(|e| format!("Failed to copy {}: {}", file, e))?;
        tokenizer_manifest.files.insert(file.to_string(), entry);
    }

    // Written last: only now do the directories count as downloaded.
    model_manifest::write(model_path, &model_manifest_data)
        .and_then(|()| model_manifest::write(tokenizer_path, &tokenizer_manifest))
        .map_err(|e| format!("Failed to record the downloaded model: {}", e))?;

    if let Some(ref handle) = app_handle {
        let _ = handle.emit("compression-download-complete", ());
    }

    info!("LLMLingua-2 model downloaded successfully");
    Ok(())
}

/// Files of the model directory.
const MODEL_FILES: &[&str] = &["model.safetensors"];

/// Files of the tokenizer directory (BERT has vocab.txt, XLM-RoBERTa embeds
/// its vocabulary in tokenizer.json).
fn tokenizer_files_for(model_size: &str) -> &'static [&'static str] {
    if model_size == "xlm-roberta" {
        &[
            "tokenizer.json",
            "tokenizer_config.json",
            "special_tokens_map.json",
        ]
    } else {
        &[
            "tokenizer.json",
            "tokenizer_config.json",
            "vocab.txt",
            "special_tokens_map.json",
        ]
    }
}

/// Copy a cached download to its final path without ever exposing a
/// truncated file there, and describe what was written for the manifest.
async fn install_file(
    cached: std::path::PathBuf,
    dest: std::path::PathBuf,
) -> Result<ManifestFile, String> {
    tokio::task::spawn_blocking(move || {
        lr_utils::fs::copy_atomically(&cached, &dest)?;
        model_manifest::describe_file(&dest)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

/// The two directories' states, and whether together they are one usable
/// download: both complete from the same revision, or both legacy.
fn combined_state(
    model_path: &Path,
    tokenizer_path: &Path,
    model_size: &str,
) -> Result<(), String> {
    use model_manifest::ModelState;
    let repo = repo_id_for_model(model_size);
    let model = model_manifest::state(model_path, repo, MODEL_FILES);
    let tokenizer = model_manifest::state(tokenizer_path, repo, tokenizer_files_for(model_size));
    match (&model, &tokenizer) {
        (ModelState::Complete(m), ModelState::Complete(t)) if m.revision != t.revision => {
            Err("the weights and the tokenizer come from different revisions".to_string())
        }
        (ModelState::Complete(_), ModelState::Complete(_)) => Ok(()),
        (ModelState::Incomplete(reason), _) | (_, ModelState::Incomplete(reason)) => {
            Err(reason.clone())
        }
        // Downloads made before manifests existed (the files are all
        // present), possibly one directory adopted already.
        _ => Ok(()),
    }
}

/// Whether both directories hold one whole download (cheap: manifests and
/// file sizes; a download made before manifests existed counts too).
pub fn is_downloaded(model_path: &Path, tokenizer_path: &Path, model_size: &str) -> bool {
    combined_state(model_path, tokenizer_path, model_size).is_ok()
}

/// Check every file against its manifest before loading (hashes once per
/// process; adopts a pre-manifest download). Blocking.
pub fn verify_for_load(
    model_path: &Path,
    tokenizer_path: &Path,
    model_size: &str,
) -> Result<(), String> {
    combined_state(model_path, tokenizer_path, model_size).map_err(|reason| {
        format!("The compression model download is incomplete or damaged ({reason}). Download it again.")
    })?;
    let repo = repo_id_for_model(model_size);
    let model = model_manifest::verify_for_load(model_path, repo, MODEL_FILES)?;
    let tokenizer =
        model_manifest::verify_for_load(tokenizer_path, repo, tokenizer_files_for(model_size))?;
    if model.revision != tokenizer.revision {
        return Err(
            "The compression model's weights and tokenizer come from different revisions. \
             Download it again."
                .to_string(),
        );
    }
    Ok(())
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
    use std::path::PathBuf;

    struct TestDir(PathBuf);
    impl TestDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "lr-compression-dl-{}-{}",
                std::process::id(),
                name
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("model")).unwrap();
            std::fs::create_dir_all(path.join("tokenizer")).unwrap();
            Self(path)
        }
        fn model(&self) -> PathBuf {
            self.0.join("model")
        }
        fn tokenizer(&self) -> PathBuf {
            self.0.join("tokenizer")
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const SIZE: &str = "bert";

    fn put(dir: &Path, files: &[&str], revision: Option<&str>) {
        model_manifest::begin_update(dir).unwrap();
        let mut manifest = ModelManifest {
            repo: repo_id_for_model(SIZE).to_string(),
            revision: revision.map(str::to_string),
            files: Default::default(),
        };
        for file in files {
            std::fs::write(dir.join(file), file.as_bytes()).unwrap();
            manifest.files.insert(
                file.to_string(),
                model_manifest::describe_file(&dir.join(file)).unwrap(),
            );
        }
        model_manifest::write(dir, &manifest).unwrap();
    }

    #[test]
    fn both_directories_must_come_from_one_revision() {
        let dir = TestDir::new("revisions");
        put(&dir.model(), MODEL_FILES, Some("aaa"));
        put(&dir.tokenizer(), tokenizer_files_for(SIZE), Some("aaa"));
        assert!(is_downloaded(&dir.model(), &dir.tokenizer(), SIZE));
        verify_for_load(&dir.model(), &dir.tokenizer(), SIZE).unwrap();

        put(&dir.tokenizer(), tokenizer_files_for(SIZE), Some("bbb"));
        assert!(!is_downloaded(&dir.model(), &dir.tokenizer(), SIZE));
        assert!(verify_for_load(&dir.model(), &dir.tokenizer(), SIZE)
            .unwrap_err()
            .contains("different revisions"));
    }

    #[test]
    fn an_interrupted_download_is_not_downloaded() {
        let dir = TestDir::new("interrupted");
        put(&dir.model(), MODEL_FILES, Some("aaa"));
        put(&dir.tokenizer(), tokenizer_files_for(SIZE), Some("aaa"));
        // A new download replaced the weights and stopped.
        model_manifest::begin_update(&dir.model()).unwrap();
        model_manifest::begin_update(&dir.tokenizer()).unwrap();
        std::fs::write(dir.model().join("model.safetensors"), b"newer").unwrap();
        assert!(!is_downloaded(&dir.model(), &dir.tokenizer(), SIZE));
    }

    #[test]
    fn a_download_from_before_manifests_is_adopted() {
        let dir = TestDir::new("legacy");
        std::fs::write(dir.model().join("model.safetensors"), b"weights").unwrap();
        for file in tokenizer_files_for(SIZE) {
            std::fs::write(dir.tokenizer().join(file), b"{}").unwrap();
        }
        assert!(is_downloaded(&dir.model(), &dir.tokenizer(), SIZE));
        verify_for_load(&dir.model(), &dir.tokenizer(), SIZE).unwrap();
        assert!(model_manifest::read(&dir.model()).unwrap().is_some());
        assert!(model_manifest::read(&dir.tokenizer()).unwrap().is_some());
        assert!(is_downloaded(&dir.model(), &dir.tokenizer(), SIZE));

        // A legacy download missing a tokenizer file is not one.
        let partial = TestDir::new("legacy-partial");
        std::fs::write(partial.model().join("model.safetensors"), b"weights").unwrap();
        std::fs::write(partial.tokenizer().join("tokenizer.json"), b"{}").unwrap();
        assert!(!is_downloaded(&partial.model(), &partial.tokenizer(), SIZE));
    }
}
