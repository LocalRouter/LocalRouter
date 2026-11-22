//! Picks up edits made to the config file outside the app.
//!
//! Three things make this safe to leave running:
//!
//! - **Events only wake a task.** Editors save by writing a temp file and
//!   renaming it over the target, and one save produces a burst of events, so
//!   the watcher watches the *directory*, keeps events for the config file's
//!   name, and looks at the file once per burst ([`coalesce`]).
//! - **The app's own writes are recognized by content, not by timing.** The
//!   save path records the hash of the bytes it wrote ([`ReloadGate`]); a file
//!   whose hash the gate already knows is not reloaded. An edit made right
//!   after an app save has a different hash and is applied.
//! - **A bad edit changes nothing.** The file is validated before it replaces
//!   the running configuration; see `ConfigManager::reload_from_disk`.

use crate::storage::ContentHash;
use crate::ConfigManager;
use lr_types::{AppError, AppResult};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;
use tracing::{debug, error, info};

/// What the app knows about the config file's content.
///
/// Shared by every clone of the `ConfigManager`, and only read or written
/// while the save mutex is held, so it always describes the file as it is.
#[derive(Debug, Default)]
pub struct ReloadGate {
    /// Hash of the content the running configuration corresponds to: the
    /// app's last write, or the last content it loaded.
    synced: Option<ContentHash>,
    /// Hash of content that failed validation and was reported, so the same
    /// broken file is reported once rather than once per event.
    rejected: Option<ContentHash>,
}

/// How file content relates to what the app already knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen {
    /// The app wrote this, or already loaded it: nothing to do.
    Synced,
    /// Already validated, rejected and reported.
    Rejected,
    /// Content the app has not seen: an outside edit to evaluate.
    New,
}

impl ReloadGate {
    /// Classify content found on disk.
    pub fn check(&self, hash: &ContentHash) -> Seen {
        if self.synced.as_ref() == Some(hash) {
            Seen::Synced
        } else if self.rejected.as_ref() == Some(hash) {
            Seen::Rejected
        } else {
            Seen::New
        }
    }

    /// The running configuration now corresponds to this content (the app
    /// wrote it, or loaded it). Returns whether a rejection was outstanding.
    pub fn record_synced(&mut self, hash: ContentHash) -> bool {
        self.synced = Some(hash);
        self.rejected.take().is_some()
    }

    /// This content was evaluated and refused.
    pub fn record_rejected(&mut self, hash: ContentHash) {
        self.rejected = Some(hash);
    }

    /// The file is back to content the app already has; any earlier rejection
    /// no longer applies. Returns whether one was outstanding.
    pub fn clear_rejected(&mut self) -> bool {
        self.rejected.take().is_some()
    }

    /// Whether anything has been recorded as synced yet.
    pub fn is_seeded(&self) -> bool {
        self.synced.is_some()
    }
}

/// How long to wait for a burst of file events to end.
#[derive(Debug, Clone, Copy)]
pub struct Debounce {
    /// Act once the file has been quiet for this long.
    pub quiet: Duration,
    /// Act at the latest this long after the first event of a burst, even if
    /// events keep arriving (something rewriting the file in a loop must not
    /// postpone the reload forever).
    pub max_wait: Duration,
}

impl Default for Debounce {
    fn default() -> Self {
        Self {
            quiet: Duration::from_millis(250),
            max_wait: Duration::from_secs(2),
        }
    }
}

impl Debounce {
    /// When to act, given when the burst started and when its latest event
    /// arrived.
    pub fn next_deadline(&self, first: Instant, last: Instant) -> Instant {
        (last + self.quiet).min(first + self.max_wait)
    }
}

/// Wait for one burst of events to end. Returns `false` once the sender is
/// gone and nothing is pending (the watcher was dropped).
pub(crate) async fn coalesce(events: &mut UnboundedReceiver<()>, debounce: Debounce) -> bool {
    if events.recv().await.is_none() {
        return false;
    }
    let first = Instant::now();
    let mut last = first;
    loop {
        let deadline = debounce.next_deadline(first, last);
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => return true,
            more = events.recv() => match more {
                Some(()) => last = Instant::now(),
                // Sender gone mid-burst: still handle what was seen.
                None => return true,
            },
        }
    }
}

/// Which filesystem events concern the config file.
#[derive(Debug, Clone)]
pub(crate) struct EventFilter {
    file_name: OsString,
    /// The watched directory as given, and as the OS reports it (macOS hands
    /// back resolved paths, e.g. `/private/var/...` for `/var/...`).
    dirs: Vec<PathBuf>,
}

impl EventFilter {
    pub(crate) fn new(config_path: &Path) -> Option<Self> {
        let file_name = config_path.file_name()?.to_os_string();
        let dir = config_path.parent()?.to_path_buf();
        let mut dirs = vec![dir.clone()];
        if let Ok(canonical) = dir.canonicalize() {
            if canonical != dir {
                dirs.push(canonical);
            }
        }
        Some(Self { file_name, dirs })
    }

    fn is_config_file(&self, path: &Path) -> bool {
        path.file_name() == Some(self.file_name.as_os_str())
            && path
                .parent()
                .is_some_and(|parent| self.dirs.iter().any(|dir| dir == parent))
    }

    /// Whether `event` may have changed the config file's content. Matching
    /// is by exact name, so the app's own temp files (`settings.yaml.tmp.*`),
    /// backups (`settings.yaml.backup.*`) and editor swap files do not count;
    /// a rename *onto* the config file does, because it names the target.
    pub(crate) fn matches(&self, event: &Event) -> bool {
        !matches!(event.kind, EventKind::Access(_))
            && event.paths.iter().any(|path| self.is_config_file(path))
    }

    #[cfg(test)]
    fn file_name(&self) -> &std::ffi::OsStr {
        &self.file_name
    }
}

/// A running config file watcher. Watching stops when this is dropped.
pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for ConfigWatcher {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ConfigWatcher {
    /// Watch `manager`'s config file and reload it on outside edits.
    ///
    /// Must be called inside a Tokio runtime.
    pub(crate) fn start(manager: ConfigManager, debounce: Debounce) -> AppResult<Self> {
        let config_path = manager.config_path().clone();
        let filter = EventFilter::new(&config_path).ok_or_else(|| {
            AppError::Config(format!(
                "Config path {:?} has no parent directory to watch",
                config_path
            ))
        })?;
        let dir = filter.dirs[0].clone();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let mut watcher =
            notify::recommended_watcher(move |result: Result<Event, notify::Error>| match result {
                Ok(event) => {
                    if filter.matches(&event) {
                        // The receiver only goes away with the watcher.
                        let _ = tx.send(());
                    }
                }
                Err(e) => error!("Config file watch error: {}", e),
            })
            .map_err(|e| AppError::Config(format!("Failed to create file watcher: {}", e)))?;

        // The directory, not the file: a rename-based save replaces the file,
        // and a watch on the old file would go deaf after the first one.
        watcher
            .watch(&dir, RecursiveMode::NonRecursive)
            .map_err(|e| AppError::Config(format!("Failed to watch config directory: {}", e)))?;

        let task = tokio::spawn(async move {
            while coalesce(&mut rx, debounce).await {
                match manager.reload_from_disk(false).await {
                    Ok(outcome) => debug!("Config file event handled: {:?}", outcome),
                    // Already logged and reported to the UI by the manager.
                    Err(e) => debug!("Config file edit not applied: {}", e),
                }
            }
        });

        info!("Watching configuration file: {:?}", config_path);
        Ok(Self {
            _watcher: watcher,
            task,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, CreateKind, ModifyKind, RenameMode};
    use std::ffi::OsStr;

    fn hash(byte: u8) -> ContentHash {
        [byte; 32]
    }

    // --- ReloadGate ---------------------------------------------------------

    #[test]
    fn unseeded_gate_treats_everything_as_new() {
        let gate = ReloadGate::default();
        assert!(!gate.is_seeded());
        assert_eq!(gate.check(&hash(1)), Seen::New);
    }

    #[test]
    fn own_write_is_recognized_however_many_events_it_produces() {
        let mut gate = ReloadGate::default();
        gate.record_synced(hash(1));
        // A save emits several events; each one finds the same content.
        for _ in 0..5 {
            assert_eq!(gate.check(&hash(1)), Seen::Synced);
        }
    }

    #[test]
    fn external_edit_right_after_an_own_write_is_not_suppressed() {
        let mut gate = ReloadGate::default();
        gate.record_synced(hash(1)); // app saves
                                     // ...and before the watcher has even looked, someone edits the file.
        assert_eq!(gate.check(&hash(2)), Seen::New);
    }

    #[test]
    fn only_the_latest_own_write_is_synced() {
        let mut gate = ReloadGate::default();
        gate.record_synced(hash(1));
        gate.record_synced(hash(2));
        assert_eq!(gate.check(&hash(2)), Seen::Synced);
        // Someone putting the older content back is an edit like any other.
        assert_eq!(gate.check(&hash(1)), Seen::New);
    }

    #[test]
    fn applied_reload_is_not_applied_again() {
        let mut gate = ReloadGate::default();
        gate.record_synced(hash(1));
        assert_eq!(gate.check(&hash(2)), Seen::New);
        gate.record_synced(hash(2)); // reload applied
        assert_eq!(gate.check(&hash(2)), Seen::Synced);
    }

    #[test]
    fn rejected_content_is_reported_once_and_does_not_replace_synced() {
        let mut gate = ReloadGate::default();
        gate.record_synced(hash(1));
        assert_eq!(gate.check(&hash(9)), Seen::New);
        gate.record_rejected(hash(9));
        assert_eq!(gate.check(&hash(9)), Seen::Rejected);
        // The running config is still the one from hash(1).
        assert_eq!(gate.check(&hash(1)), Seen::Synced);
        // A different broken edit is new and gets its own report.
        assert_eq!(gate.check(&hash(8)), Seen::New);
    }

    #[test]
    fn syncing_or_reverting_clears_an_outstanding_rejection() {
        let mut gate = ReloadGate::default();
        gate.record_synced(hash(1));
        assert!(!gate.clear_rejected());

        gate.record_rejected(hash(9));
        assert!(gate.clear_rejected(), "revert to known content");
        assert!(!gate.clear_rejected());

        gate.record_rejected(hash(9));
        assert!(
            gate.record_synced(hash(2)),
            "fixed, or overwritten by a save"
        );
        assert_eq!(gate.check(&hash(9)), Seen::New);
    }

    // --- Debounce -----------------------------------------------------------

    #[test]
    fn deadline_follows_the_latest_event_until_the_cap() {
        let debounce = Debounce {
            quiet: Duration::from_millis(100),
            max_wait: Duration::from_millis(350),
        };
        let first = Instant::now();
        assert_eq!(
            debounce.next_deadline(first, first),
            first + Duration::from_millis(100)
        );
        let later = first + Duration::from_millis(200);
        assert_eq!(
            debounce.next_deadline(first, later),
            first + Duration::from_millis(300)
        );
        // Events keep coming: the cap wins.
        let much_later = first + Duration::from_millis(300);
        assert_eq!(
            debounce.next_deadline(first, much_later),
            first + Duration::from_millis(350)
        );
    }

    #[tokio::test]
    async fn a_burst_of_events_is_one_wakeup() {
        let debounce = Debounce {
            quiet: Duration::from_millis(60),
            max_wait: Duration::from_secs(5),
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        for _ in 0..20 {
            tx.send(()).unwrap();
        }
        let started = Instant::now();
        assert!(coalesce(&mut rx, debounce).await);
        assert!(started.elapsed() >= Duration::from_millis(60));
        // The whole burst was consumed by that one wakeup.
        assert!(rx.try_recv().is_err());

        // A later event is a new burst.
        tx.send(()).unwrap();
        assert!(coalesce(&mut rx, debounce).await);

        drop(tx);
        assert!(!coalesce(&mut rx, debounce).await);
    }

    #[tokio::test]
    async fn events_that_never_stop_still_get_handled() {
        let debounce = Debounce {
            quiet: Duration::from_millis(80),
            max_wait: Duration::from_millis(300),
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        tx.send(()).unwrap();
        let feeder = tokio::spawn(async move {
            // Faster than the quiet period, for far longer than the cap.
            for _ in 0..200 {
                if tx.send(()).is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        let started = Instant::now();
        assert!(coalesce(&mut rx, debounce).await);
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(300), "waited {waited:?}");
        assert!(waited < Duration::from_secs(3), "waited {waited:?}");
        feeder.abort();
    }

    #[tokio::test]
    async fn a_closed_channel_mid_burst_still_reports_the_burst() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        tx.send(()).unwrap();
        drop(tx);
        assert!(coalesce(&mut rx, Debounce::default()).await);
        assert!(!coalesce(&mut rx, Debounce::default()).await);
    }

    // --- Event filter -------------------------------------------------------

    fn event(kind: EventKind, paths: &[&str]) -> Event {
        let mut event = Event::new(kind);
        for path in paths {
            event = event.add_path(PathBuf::from(path));
        }
        event
    }

    fn filter() -> EventFilter {
        EventFilter::new(Path::new("/nonexistent-lr-config-dir/settings.yaml")).unwrap()
    }

    #[test]
    fn filter_is_built_from_the_config_path() {
        assert_eq!(filter().file_name(), OsStr::new("settings.yaml"));
        assert!(EventFilter::new(Path::new("/")).is_none());
    }

    #[test]
    fn writes_creates_and_removes_of_the_config_file_match() {
        let f = filter();
        let path = "/nonexistent-lr-config-dir/settings.yaml";
        for kind in [
            EventKind::Modify(ModifyKind::Any),
            EventKind::Create(CreateKind::File),
            EventKind::Remove(notify::event::RemoveKind::File),
            EventKind::Any,
        ] {
            assert!(f.matches(&event(kind, &[path])), "{kind:?}");
        }
    }

    #[test]
    fn a_rename_onto_the_config_file_matches() {
        // How editors (and the app itself) save: temp file, then rename.
        let f = filter();
        assert!(f.matches(&event(
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            &[
                "/nonexistent-lr-config-dir/.settings.yaml.swp",
                "/nonexistent-lr-config-dir/settings.yaml",
            ],
        )));
    }

    #[test]
    fn neighbours_of_the_config_file_do_not_match() {
        let f = filter();
        let modify = EventKind::Modify(ModifyKind::Any);
        for path in [
            "/nonexistent-lr-config-dir/settings.yaml.tmp.1700000000000",
            "/nonexistent-lr-config-dir/settings.yaml.backup.20261121-120000",
            "/nonexistent-lr-config-dir/.settings.yaml.swp",
            "/nonexistent-lr-config-dir/settings.yaml~",
            "/nonexistent-lr-config-dir/ui_state.json",
            // Same name, different directory.
            "/nonexistent-lr-config-dir/proxy/settings.yaml",
            "/elsewhere/settings.yaml",
        ] {
            assert!(!f.matches(&event(modify, &[path])), "{path}");
        }
    }

    #[test]
    fn reads_of_the_config_file_do_not_match() {
        // The watcher's own read must not wake the watcher.
        assert!(!filter().matches(&event(
            EventKind::Access(AccessKind::Read),
            &["/nonexistent-lr-config-dir/settings.yaml"],
        )));
    }

    #[test]
    fn the_resolved_directory_matches_too() {
        // A temp dir on macOS is reached through a symlink (`/var` →
        // `/private/var`); events arrive with the resolved path.
        let dir = tempfile::tempdir().unwrap();
        let given = dir.path().join("settings.yaml");
        let f = EventFilter::new(&given).unwrap();
        let resolved = dir.path().canonicalize().unwrap().join("settings.yaml");
        let modify = EventKind::Modify(ModifyKind::Any);
        assert!(f.matches(&Event::new(modify).add_path(given)));
        assert!(f.matches(&Event::new(modify).add_path(resolved)));
    }
}
