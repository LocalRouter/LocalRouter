//! Download-progress callback for [`crate::EmbeddingService::download`].
//!
//! Defined as a trait so `lr-embeddings` itself stays free of any UI /
//! framework dependency. Desktop integrations implement it as a thin
//! wrapper around their event surface (e.g. `tauri::AppHandle::emit`);
//! headless daemons can pass `None` or a `tracing`-flavoured impl.
//!
//! Granularity matches the underlying `hf-hub` API today: file-boundary
//! events, not byte-stream. Consumers wanting a `0.0..=1.0` fraction
//! compute it as `completed_files as f64 / total_files as f64`.

/// Hook invoked at file-boundary events during model download.
///
/// `Send + Sync` is required: `download_model` is `async` and the
/// reporter is referenced after every awaited file copy, so the
/// future must remain `Send`.
pub trait DownloadProgress: Send + Sync {
    /// Fired at the start of each file (`completed_files == 0` for
    /// the first call) and again after each file finishes copying.
    /// `current_file` is the file name (e.g. `"model.safetensors"`).
    fn on_progress(&self, current_file: &str, completed_files: u32, total_files: u32);

    /// Fired exactly once after the last file has been copied. Not
    /// called on error paths.
    fn on_complete(&self);
}
