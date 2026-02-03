//! MCP access log writer
//!
//! Writes MCP request/response logs in JSON Lines format with automatic daily rotation.

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

use lr_types::{AppError, AppResult};

use super::logger::AccessLogger;

/// MCP access log entry in JSON Lines format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpAccessLogEntry {
    /// Timestamp of the request
    pub timestamp: DateTime<Utc>,

    /// Client ID making the request
    pub client_id: String,

    /// MCP server ID
    pub server_id: String,

    /// JSON-RPC method (e.g., "tools/list", "resources/read")
    pub method: String,

    /// Status ("success" or "error")
    pub status: String,

    /// HTTP-like status code (200, 500, etc.)
    pub status_code: u16,

    /// JSON-RPC error code if failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<i32>,

    /// Latency in milliseconds
    pub latency_ms: u64,

    /// Transport type ("stdio", "sse", "websocket")
    pub transport: String,

    /// Unique request ID
    pub request_id: String,

    /// Firewall action taken (if any)
    /// Values: "allowed", "denied", "asked:allowed_once", "asked:allowed_session", "asked:denied", "asked:timeout"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firewall_action: Option<String>,
}

impl McpAccessLogEntry {
    /// Create a new MCP access log entry for a successful request
    pub fn success(
        client_id: impl Into<String>,
        server_id: impl Into<String>,
        method: impl Into<String>,
        latency_ms: u64,
        transport: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            client_id: client_id.into(),
            server_id: server_id.into(),
            method: method.into(),
            status: "success".to_string(),
            status_code: 200,
            error_code: None,
            latency_ms,
            transport: transport.into(),
            request_id: request_id.into(),
            firewall_action: None,
        }
    }

    /// Create a new MCP access log entry for a failed request
    #[allow(clippy::too_many_arguments)]
    pub fn error(
        client_id: impl Into<String>,
        server_id: impl Into<String>,
        method: impl Into<String>,
        status_code: u16,
        error_code: Option<i32>,
        latency_ms: u64,
        transport: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            client_id: client_id.into(),
            server_id: server_id.into(),
            method: method.into(),
            status: "error".to_string(),
            status_code,
            error_code,
            latency_ms,
            transport: transport.into(),
            request_id: request_id.into(),
            firewall_action: None,
        }
    }
}

/// MCP access logger that writes to daily log files
pub struct McpAccessLogger {
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

    /// Whether access logging is enabled
    enabled: Arc<RwLock<bool>>,
}

impl McpAccessLogger {
    /// Create a new MCP access logger
    pub fn new(retention_days: u32, enabled: bool) -> AppResult<Self> {
        let log_dir = Self::get_log_directory()?;

        // Create log directory if it doesn't exist
        fs::create_dir_all(&log_dir).map_err(|e| {
            AppError::Internal(format!("Failed to create MCP log directory: {}", e))
        })?;

        info!(
            "MCP access logger initialized with directory: {:?}, enabled: {}",
            log_dir, enabled
        );

        Ok(Self {
            log_dir,
            writer: Arc::new(Mutex::new(None)),
            current_date: Arc::new(Mutex::new(String::new())),
            retention_days,
            app_handle: Arc::new(RwLock::new(None)),
            enabled: Arc::new(RwLock::new(enabled)),
        })
    }

    /// Check if MCP access logging is enabled
    pub fn is_enabled(&self) -> bool {
        *self.enabled.read()
    }

    /// Set whether MCP access logging is enabled
    pub fn set_enabled(&self, enabled: bool) {
        info!(
            "MCP access logging {}",
            if enabled { "enabled" } else { "disabled" }
        );
        *self.enabled.write() = enabled;
    }

    /// Set the Tauri app handle for event emission
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        *self.app_handle.write() = Some(handle);
    }

    /// Get the OS-specific log directory (shared with LLM logs)
    ///
    /// Delegates to AccessLogger::get_log_directory() to avoid code duplication.
    fn get_log_directory() -> AppResult<PathBuf> {
        AccessLogger::get_log_directory()
    }

    /// Get the log file path for a given date
    fn get_log_file_path(&self, date: &str) -> PathBuf {
        self.log_dir.join(format!("localrouter-mcp-{}.log", date))
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
                .map_err(|e| AppError::Internal(format!("Failed to open MCP log file: {}", e)))?;

            let buf_writer = BufWriter::new(file);

            // Update the writer and current date
            *self.writer.lock() = Some(buf_writer);
            *current_date = today.clone();

            info!("Opened new MCP log file: {:?}", log_path);

            // Clean up old logs
            if let Err(e) = self.cleanup_old_logs() {
                warn!("Failed to cleanup old MCP logs: {}", e);
            }
        }

        Ok(())
    }

    /// Write an MCP access log entry
    pub fn log(&self, entry: &McpAccessLogEntry) -> AppResult<()> {
        // Skip logging if disabled (but still emit events for real-time UI)
        if !self.is_enabled() {
            // Still emit Tauri event for real-time UI even when file logging is disabled
            if let Some(handle) = self.app_handle.read().as_ref() {
                if let Err(e) = handle.emit("mcp-log-entry", entry) {
                    warn!("Failed to emit MCP log event: {}", e);
                }
            }
            return Ok(());
        }

        self.ensure_log_file()?;

        let json = serde_json::to_string(entry)
            .map_err(|e| AppError::Internal(format!("Failed to serialize MCP log entry: {}", e)))?;

        let mut writer_guard = self.writer.lock();
        if let Some(writer) = writer_guard.as_mut() {
            writeln!(writer, "{}", json)
                .map_err(|e| AppError::Internal(format!("Failed to write MCP log entry: {}", e)))?;

            // Flush to ensure data is written
            writer
                .flush()
                .map_err(|e| AppError::Internal(format!("Failed to flush MCP log: {}", e)))?;
        }

        // Emit Tauri event if app handle is available
        if let Some(handle) = self.app_handle.read().as_ref() {
            if let Err(e) = handle.emit("mcp-log-entry", entry) {
                warn!("Failed to emit MCP log event: {}", e);
            }
        }

        Ok(())
    }

    /// Clean up log files older than retention period
    fn cleanup_old_logs(&self) -> AppResult<()> {
        let retention_duration = chrono::Duration::days(self.retention_days as i64);
        let cutoff_date = Utc::now() - retention_duration;

        let entries = fs::read_dir(&self.log_dir)
            .map_err(|e| AppError::Internal(format!("Failed to read MCP log directory: {}", e)))?;

        for entry in entries {
            let entry = entry.map_err(|e| {
                AppError::Internal(format!("Failed to read directory entry: {}", e))
            })?;

            let path = entry.path();

            // Check if this is an MCP log file
            if path.is_file() {
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    // Parse date from filename (localrouter-mcp-YYYY-MM-DD.log)
                    if filename.starts_with("localrouter-mcp-") && filename.ends_with(".log") {
                        let date_str = &filename[16..26]; // Extract YYYY-MM-DD

                        if let Ok(file_date) =
                            chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                        {
                            let file_datetime = file_date
                                .and_hms_opt(0, 0, 0)
                                .ok_or_else(|| AppError::Internal("Invalid time".to_string()))?
                                .and_utc();

                            if file_datetime < cutoff_date {
                                info!("Deleting old MCP log file: {:?}", path);
                                if let Err(e) = fs::remove_file(&path) {
                                    warn!("Failed to delete old MCP log file {:?}: {}", path, e);
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

    /// Log a successful MCP request
    pub fn log_success(
        &self,
        client_id: &str,
        server_id: &str,
        method: &str,
        latency_ms: u64,
        transport: &str,
        request_id: &str,
    ) -> AppResult<()> {
        let entry = McpAccessLogEntry::success(
            client_id, server_id, method, latency_ms, transport, request_id,
        );
        self.log(&entry)
    }

    /// Log a failed MCP request
    #[allow(clippy::too_many_arguments)]
    pub fn log_failure(
        &self,
        client_id: &str,
        server_id: &str,
        method: &str,
        status_code: u16,
        error_code: Option<i32>,
        latency_ms: u64,
        transport: &str,
        request_id: &str,
    ) -> AppResult<()> {
        let entry = McpAccessLogEntry::error(
            client_id,
            server_id,
            method,
            status_code,
            error_code,
            latency_ms,
            transport,
            request_id,
        );
        self.log(&entry)
    }
}

impl Drop for McpAccessLogger {
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

    #[test]
    fn test_mcp_access_log_entry_success() {
        let entry = McpAccessLogEntry::success(
            "client-1",
            "server-1",
            "tools/list",
            100,
            "stdio",
            "req-123",
        );

        assert_eq!(entry.client_id, "client-1");
        assert_eq!(entry.server_id, "server-1");
        assert_eq!(entry.method, "tools/list");
        assert_eq!(entry.status, "success");
        assert_eq!(entry.status_code, 200);
        assert_eq!(entry.error_code, None);
        assert_eq!(entry.latency_ms, 100);
        assert_eq!(entry.transport, "stdio");
        assert_eq!(entry.request_id, "req-123");
    }

    #[test]
    fn test_mcp_access_log_entry_error() {
        let entry = McpAccessLogEntry::error(
            "client-1",
            "server-1",
            "tools/call",
            500,
            Some(-32600),
            150,
            "sse",
            "req-456",
        );

        assert_eq!(entry.client_id, "client-1");
        assert_eq!(entry.server_id, "server-1");
        assert_eq!(entry.method, "tools/call");
        assert_eq!(entry.status, "error");
        assert_eq!(entry.status_code, 500);
        assert_eq!(entry.error_code, Some(-32600));
        assert_eq!(entry.latency_ms, 150);
        assert_eq!(entry.transport, "sse");
        assert_eq!(entry.request_id, "req-456");
    }
}
