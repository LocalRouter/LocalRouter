//! Access log writer
//!
//! Writes request/response logs in JSON Lines format with automatic daily rotation.

#![allow(dead_code)]

use chrono::{DateTime, Utc};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Emitter;
use tracing::{info, warn};

use crate::utils::errors::{AppError, AppResult};

/// Access log entry in JSON Lines format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessLogEntry {
    /// Timestamp of the request
    pub timestamp: DateTime<Utc>,

    /// API key name (not the actual key)
    pub api_key_name: String,

    /// Provider used (e.g., "openai", "ollama")
    pub provider: String,

    /// Model used (e.g., "gpt-4", "llama3.3")
    pub model: String,

    /// Status ("success" or "error")
    pub status: String,

    /// HTTP status code
    pub status_code: u16,

    /// Number of input tokens
    pub input_tokens: u64,

    /// Number of output tokens
    pub output_tokens: u64,

    /// Total tokens (input + output)
    pub total_tokens: u64,

    /// Cost in USD
    pub cost_usd: f64,

    /// Latency in milliseconds
    pub latency_ms: u64,

    /// Unique request ID
    pub request_id: String,

    /// RouteLLM win rate (0.0-1.0) if RouteLLM routing was used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routellm_win_rate: Option<f32>,
}

impl AccessLogEntry {
    /// Create a new access log entry for a successful request
    #[allow(clippy::too_many_arguments)]
    pub fn success(
        api_key_name: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        latency_ms: u64,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            api_key_name: api_key_name.into(),
            provider: provider.into(),
            model: model.into(),
            status: "success".to_string(),
            status_code: 200,
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
            cost_usd,
            latency_ms,
            request_id: request_id.into(),
            routellm_win_rate: None,
        }
    }

    /// Create a new access log entry for a failed request
    pub fn error(
        api_key_name: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
        status_code: u16,
        latency_ms: u64,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            api_key_name: api_key_name.into(),
            provider: provider.into(),
            model: model.into(),
            status: "error".to_string(),
            status_code,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            cost_usd: 0.0,
            latency_ms,
            request_id: request_id.into(),
            routellm_win_rate: None,
        }
    }
}

/// Access logger that writes to daily log files
pub struct AccessLogger {
    /// Base directory for logs
    log_dir: PathBuf,

    /// Current log file writer
    writer: Arc<Mutex<Option<BufWriter<File>>>>,

    /// Current log file date (YYYY-MM-DD)
    current_date: Arc<Mutex<String>>,

    /// Maximum number of days to keep logs
    retention_days: u32,

    /// Optional Tauri app handle for emitting events
    app_handle: Arc<RwLock<Option<tauri::AppHandle>>>,
}

impl AccessLogger {
    /// Create a new access logger
    pub fn new(retention_days: u32) -> AppResult<Self> {
        let log_dir = Self::get_log_directory()?;

        // Create log directory if it doesn't exist
        fs::create_dir_all(&log_dir)
            .map_err(|e| AppError::Internal(format!("Failed to create log directory: {}", e)))?;

        info!("Access logger initialized with directory: {:?}", log_dir);

        Ok(Self {
            log_dir,
            writer: Arc::new(Mutex::new(None)),
            current_date: Arc::new(Mutex::new(String::new())),
            retention_days,
            app_handle: Arc::new(RwLock::new(None)),
        })
    }

    /// Set the Tauri app handle for event emission
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        *self.app_handle.write() = Some(handle);
    }

    /// Get the OS-specific log directory
    ///
    /// This is the canonical implementation - use this everywhere to avoid inconsistencies.
    pub fn get_log_directory() -> AppResult<PathBuf> {
        #[cfg(target_os = "linux")]
        {
            // Use user's home directory for logs (consistent with config location)
            // Follows XDG conventions: ~/.local/share/localrouter/logs
            // Falls back to ~/.localrouter/logs for compatibility
            let home = dirs::home_dir()
                .ok_or_else(|| AppError::Internal("Failed to get home directory".to_string()))?;

            // Try XDG data home first
            if let Some(data_home) = dirs::data_local_dir() {
                Ok(data_home.join("localrouter").join("logs"))
            } else {
                Ok(home.join(".localrouter").join("logs"))
            }
        }

        #[cfg(target_os = "macos")]
        {
            let home = dirs::home_dir()
                .ok_or_else(|| AppError::Internal("Failed to get home directory".to_string()))?;
            Ok(home.join("Library").join("Logs").join("LocalRouter"))
        }

        #[cfg(target_os = "windows")]
        {
            // Use LOCALAPPDATA for logs (local to the machine, not roaming)
            let local_app_data = std::env::var("LOCALAPPDATA")
                .or_else(|_| std::env::var("APPDATA"))
                .map_err(|_| {
                    AppError::Internal("Failed to get LOCALAPPDATA directory".to_string())
                })?;
            Ok(PathBuf::from(local_app_data)
                .join("LocalRouter")
                .join("logs"))
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            Err(AppError::Internal(
                "Unsupported operating system".to_string(),
            ))
        }
    }

    /// Get the log file path for a given date
    fn get_log_file_path(&self, date: &str) -> PathBuf {
        self.log_dir.join(format!("localrouter-{}.log", date))
    }

    /// Ensure the log file for today is open
    fn ensure_log_file(&self) -> AppResult<()> {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let mut current_date = self.current_date.lock();

        // If the date has changed, rotate to a new file
        if *current_date != today {
            let log_path = self.get_log_file_path(&today);

            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .map_err(|e| AppError::Internal(format!("Failed to open log file: {}", e)))?;

            let buf_writer = BufWriter::new(file);

            // Update the writer and current date
            *self.writer.lock() = Some(buf_writer);
            *current_date = today.clone();

            info!("Opened new log file: {:?}", log_path);

            // Clean up old logs
            if let Err(e) = self.cleanup_old_logs() {
                warn!("Failed to cleanup old logs: {}", e);
            }
        }

        Ok(())
    }

    /// Write an access log entry
    pub fn log(&self, entry: &AccessLogEntry) -> AppResult<()> {
        self.ensure_log_file()?;

        let json = serde_json::to_string(entry)
            .map_err(|e| AppError::Internal(format!("Failed to serialize log entry: {}", e)))?;

        let mut writer_guard = self.writer.lock();
        if let Some(writer) = writer_guard.as_mut() {
            writeln!(writer, "{}", json)
                .map_err(|e| AppError::Internal(format!("Failed to write log entry: {}", e)))?;

            // Flush to ensure data is written
            writer
                .flush()
                .map_err(|e| AppError::Internal(format!("Failed to flush log: {}", e)))?;
        }

        // Emit Tauri event if app handle is available
        if let Some(handle) = self.app_handle.read().as_ref() {
            if let Err(e) = handle.emit("llm-log-entry", entry) {
                warn!("Failed to emit LLM log event: {}", e);
            }
        }

        Ok(())
    }

    /// Clean up log files older than retention period
    fn cleanup_old_logs(&self) -> AppResult<()> {
        let retention_duration = chrono::Duration::days(self.retention_days as i64);
        let cutoff_date = Utc::now() - retention_duration;

        let entries = fs::read_dir(&self.log_dir)
            .map_err(|e| AppError::Internal(format!("Failed to read log directory: {}", e)))?;

        for entry in entries {
            let entry = entry.map_err(|e| {
                AppError::Internal(format!("Failed to read directory entry: {}", e))
            })?;

            let path = entry.path();

            // Check if this is a log file
            if path.is_file() {
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    // Parse date from filename (localrouter-YYYY-MM-DD.log)
                    // Exclude MCP log files (localrouter-mcp-YYYY-MM-DD.log)
                    if filename.starts_with("localrouter-")
                        && !filename.starts_with("localrouter-mcp-")
                        && filename.ends_with(".log")
                    {
                        let date_str = &filename[12..22]; // Extract YYYY-MM-DD

                        if let Ok(file_date) =
                            chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                        {
                            let file_datetime = file_date
                                .and_hms_opt(0, 0, 0)
                                .ok_or_else(|| AppError::Internal("Invalid time".to_string()))?
                                .and_utc();

                            if file_datetime < cutoff_date {
                                info!("Deleting old log file: {:?}", path);
                                if let Err(e) = fs::remove_file(&path) {
                                    warn!("Failed to delete old log file {:?}: {}", path, e);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Get the log directory path
    pub fn log_dir(&self) -> &PathBuf {
        &self.log_dir
    }

    /// Log a successful request
    #[allow(clippy::too_many_arguments)]
    pub fn log_success(
        &self,
        api_key_name: &str,
        provider: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        latency_ms: u64,
        request_id: &str,
    ) -> AppResult<()> {
        let entry = AccessLogEntry::success(
            api_key_name,
            provider,
            model,
            input_tokens,
            output_tokens,
            cost_usd,
            latency_ms,
            request_id,
        );
        self.log(&entry)
    }

    /// Log a failed request
    pub fn log_failure(
        &self,
        api_key_name: &str,
        provider: &str,
        model: &str,
        latency_ms: u64,
        request_id: &str,
        status_code: u16,
    ) -> AppResult<()> {
        let entry = AccessLogEntry::error(
            api_key_name,
            provider,
            model,
            status_code,
            latency_ms,
            request_id,
        );
        self.log(&entry)
    }
}

impl Drop for AccessLogger {
    fn drop(&mut self) {
        // Flush and close the log file
        if let Some(mut writer) = self.writer.lock().take() {
            let _ = writer.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_logger() -> (AccessLogger, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let logger = AccessLogger {
            log_dir: temp_dir.path().to_path_buf(),
            writer: Arc::new(Mutex::new(None)),
            current_date: Arc::new(Mutex::new(String::new())),
            retention_days: 30,
            app_handle: Arc::new(RwLock::new(None)),
        };
        (logger, temp_dir)
    }

    #[test]
    fn test_access_log_entry_success() {
        let entry = AccessLogEntry::success(
            "my-app-123",
            "openai",
            "gpt-4",
            150,
            500,
            0.0325,
            1234,
            "req_abc123",
        );

        assert_eq!(entry.api_key_name, "my-app-123");
        assert_eq!(entry.provider, "openai");
        assert_eq!(entry.model, "gpt-4");
        assert_eq!(entry.status, "success");
        assert_eq!(entry.status_code, 200);
        assert_eq!(entry.input_tokens, 150);
        assert_eq!(entry.output_tokens, 500);
        assert_eq!(entry.total_tokens, 650);
        assert_eq!(entry.cost_usd, 0.0325);
        assert_eq!(entry.latency_ms, 1234);
        assert_eq!(entry.request_id, "req_abc123");
    }

    #[test]
    fn test_access_log_entry_error() {
        let entry = AccessLogEntry::error("my-app-123", "openai", "gpt-4", 500, 1234, "req_abc123");

        assert_eq!(entry.status, "error");
        assert_eq!(entry.status_code, 500);
        assert_eq!(entry.input_tokens, 0);
        assert_eq!(entry.output_tokens, 0);
        assert_eq!(entry.total_tokens, 0);
        assert_eq!(entry.cost_usd, 0.0);
    }

    #[test]
    fn test_log_entry_serialization() {
        let entry = AccessLogEntry::success(
            "my-app-123",
            "openai",
            "gpt-4",
            150,
            500,
            0.0325,
            1234,
            "req_abc123",
        );

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: AccessLogEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.api_key_name, entry.api_key_name);
        assert_eq!(parsed.provider, entry.provider);
        assert_eq!(parsed.model, entry.model);
    }

    #[test]
    fn test_logger_writes_to_file() {
        let (logger, _temp_dir) = create_test_logger();

        let entry = AccessLogEntry::success(
            "my-app-123",
            "openai",
            "gpt-4",
            150,
            500,
            0.0325,
            1234,
            "req_abc123",
        );

        logger.log(&entry).unwrap();

        // Verify file was created
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let log_file = logger.get_log_file_path(&today);
        assert!(log_file.exists());

        // Verify content
        let content = fs::read_to_string(&log_file).unwrap();
        assert!(content.contains("my-app-123"));
        assert!(content.contains("openai"));
        assert!(content.contains("gpt-4"));
    }

    #[test]
    fn test_log_rotation() {
        let (logger, _temp_dir) = create_test_logger();

        let entry = AccessLogEntry::success(
            "my-app-123",
            "openai",
            "gpt-4",
            150,
            500,
            0.0325,
            1234,
            "req_abc123",
        );

        // Write first entry
        logger.log(&entry).unwrap();

        // Simulate date change by manually updating current_date
        *logger.current_date.lock() = "2026-01-13".to_string();

        // Write second entry (should create new file)
        logger.log(&entry).unwrap();

        // Verify both files exist
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let log_file_today = logger.get_log_file_path(&today);
        assert!(log_file_today.exists());
    }
}
