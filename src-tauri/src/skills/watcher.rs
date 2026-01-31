//! File watcher for skill sources
//!
//! Uses `notify` (FSEvents/inotify/ReadDirectoryChanges) to watch skill source paths
//! for changes, triggering automatic rescan with debouncing.

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Commands that can be sent to the watcher to manage watched paths
pub enum WatcherCommand {
    /// Add a path to watch
    AddPath(String),
    /// Remove a path from watching
    RemovePath(String),
}

/// File event from notify, forwarded to the async task
struct FileEvent {
    paths: Vec<PathBuf>,
}

/// Skill file watcher
pub struct SkillWatcher {
    command_tx: mpsc::UnboundedSender<WatcherCommand>,
}

impl SkillWatcher {
    /// Start the watcher with initial paths
    ///
    /// Returns the watcher handle and spawns background tasks for:
    /// 1. Receiving notify events and forwarding to async context
    /// 2. Debouncing events and triggering rescan
    pub fn start(
        initial_paths: Vec<String>,
        rescan_callback: Arc<dyn Fn(Vec<String>) + Send + Sync>,
    ) -> Result<Self, String> {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel::<FileEvent>();

        // Create the notify watcher
        let event_tx_clone = event_tx.clone();
        let mut watcher: RecommendedWatcher =
            notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                match res {
                    Ok(event) => {
                        if !event.paths.is_empty() {
                            let _ = event_tx_clone.send(FileEvent {
                                paths: event.paths,
                            });
                        }
                    }
                    Err(e) => {
                        warn!("File watcher error: {}", e);
                    }
                }
            })
            .map_err(|e| format!("Failed to create file watcher: {}", e))?;

        // Watch initial paths
        for path_str in &initial_paths {
            let path = PathBuf::from(path_str);
            if path.is_dir() {
                if let Err(e) = watcher.watch(&path, RecursiveMode::Recursive) {
                    warn!("Failed to watch directory {:?}: {}", path, e);
                } else {
                    debug!("Watching directory: {:?}", path);
                }
            } else if path.is_file() {
                // Watch the parent directory non-recursively, filter events for specific file
                if let Some(parent) = path.parent() {
                    if let Err(e) = watcher.watch(parent, RecursiveMode::NonRecursive) {
                        warn!("Failed to watch parent of {:?}: {}", path, e);
                    } else {
                        debug!("Watching parent directory of file: {:?}", path);
                    }
                }
            }
        }

        // Track watched paths for dynamic management
        let watched_paths: HashSet<String> = initial_paths.iter().cloned().collect();

        // Spawn the debounce + command handling task
        tokio::spawn(Self::run_event_loop(
            watcher,
            command_rx,
            event_rx,
            watched_paths,
            rescan_callback,
        ));

        Ok(Self { command_tx })
    }

    /// Send a command to add a new path
    pub fn add_path(&self, path: String) {
        let _ = self.command_tx.send(WatcherCommand::AddPath(path));
    }

    /// Send a command to remove a path
    pub fn remove_path(&self, path: String) {
        let _ = self.command_tx.send(WatcherCommand::RemovePath(path));
    }

    /// Main event loop: processes commands and debounced file events
    async fn run_event_loop(
        mut watcher: RecommendedWatcher,
        mut command_rx: mpsc::UnboundedReceiver<WatcherCommand>,
        mut event_rx: mpsc::UnboundedReceiver<FileEvent>,
        mut watched_paths: HashSet<String>,
        rescan_callback: Arc<dyn Fn(Vec<String>) + Send + Sync>,
    ) {
        let debounce_duration = std::time::Duration::from_millis(500);
        let mut pending_paths: HashSet<PathBuf> = HashSet::new();
        let mut debounce_timer: Option<tokio::time::Instant> = None;

        loop {
            let timeout = debounce_timer.map(|t| {
                let remaining = t
                    .checked_duration_since(tokio::time::Instant::now())
                    .unwrap_or_default();
                tokio::time::sleep(remaining)
            });

            tokio::select! {
                // Process commands
                Some(cmd) = command_rx.recv() => {
                    match cmd {
                        WatcherCommand::AddPath(path_str) => {
                            let path = PathBuf::from(&path_str);
                            if path.is_dir() {
                                if let Err(e) = watcher.watch(&path, RecursiveMode::Recursive) {
                                    warn!("Failed to watch {:?}: {}", path, e);
                                } else {
                                    info!("Now watching: {:?}", path);
                                }
                            } else if path.is_file() {
                                if let Some(parent) = path.parent() {
                                    if let Err(e) = watcher.watch(parent, RecursiveMode::NonRecursive) {
                                        warn!("Failed to watch parent of {:?}: {}", path, e);
                                    }
                                }
                            }
                            watched_paths.insert(path_str);
                        }
                        WatcherCommand::RemovePath(path_str) => {
                            let path = PathBuf::from(&path_str);
                            let _ = watcher.unwatch(&path);
                            // For files, unwatch the parent
                            if path.is_file() {
                                if let Some(parent) = path.parent() {
                                    let _ = watcher.unwatch(parent);
                                }
                            }
                            watched_paths.remove(&path_str);
                            info!("Stopped watching: {:?}", path);
                        }
                    }
                }

                // Process file events
                Some(event) = event_rx.recv() => {
                    pending_paths.extend(event.paths);
                    debounce_timer = Some(tokio::time::Instant::now() + debounce_duration);
                }

                // Debounce timer fired
                _ = async {
                    if let Some(t) = timeout {
                        t.await;
                    } else {
                        // No timer - wait forever (will be interrupted by other branches)
                        std::future::pending::<()>().await;
                    }
                } => {
                    if !pending_paths.is_empty() {
                        // Determine which configured paths are affected
                        let affected: Vec<String> = watched_paths
                            .iter()
                            .filter(|wp| {
                                let wp_path = PathBuf::from(wp);
                                pending_paths.iter().any(|pp| {
                                    pp.starts_with(&wp_path) || pp == &wp_path
                                })
                            })
                            .cloned()
                            .collect();

                        if !affected.is_empty() {
                            info!(
                                "File changes detected in {} paths, triggering rescan",
                                affected.len()
                            );
                            rescan_callback(affected);
                        }

                        pending_paths.clear();
                        debounce_timer = None;
                    }
                }
            }
        }
    }
}
