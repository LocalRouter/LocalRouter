//! Reloading the config file from disk: what the watcher does with the app's
//! own writes, with good outside edits and with bad ones. Everything runs in
//! a temp dir.

use super::*;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;

/// A manager on a freshly saved config in a temp dir, plus a counter of how
/// many times the client manager was synced (once per `update()` and once
/// per applied reload).
async fn manager() -> (TempDir, ConfigManager, Arc<AtomicUsize>) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("settings.yaml");
    save_config(&AppConfig::default(), &path).await.unwrap();
    let mut manager = ConfigManager::load_from_path(path).await.unwrap();
    let synced = Arc::new(AtomicUsize::new(0));
    let counter = synced.clone();
    manager.set_client_sync_callback(Arc::new(move |_clients| {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    (dir, manager, synced)
}

/// Replace the file the way an editor does: write a sibling, rename it over.
fn write_like_an_editor(path: &Path, contents: &[u8]) {
    let tmp = path.with_file_name(".settings.yaml.editor-tmp");
    std::fs::write(&tmp, contents).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

/// An outside edit that changes a setting: the file as it is, with another port.
fn edit_port(path: &Path, port: u16) {
    let mut config: AppConfig =
        serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    config.server.port = port;
    write_like_an_editor(path, serde_yaml::to_string(&config).unwrap().as_bytes());
}

fn backups(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".backup."))
        .count()
}

#[tokio::test]
async fn the_apps_own_save_is_not_reloaded() {
    let (_dir, manager, synced) = manager().await;
    // Freshly loaded: the file is what the app has.
    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::Unchanged
    );

    manager.update(|c| c.server.port = 4001).unwrap();
    manager.save().await.unwrap();
    let after_update = synced.load(Ordering::SeqCst);

    // However many events that save produced, each look finds known content.
    for _ in 0..3 {
        assert_eq!(
            manager.reload_from_disk(false).await.unwrap(),
            ReloadOutcome::Unchanged
        );
    }
    assert_eq!(synced.load(Ordering::SeqCst), after_update);
    assert_eq!(manager.get().server.port, 4001);
}

#[tokio::test]
async fn an_own_save_does_not_roll_back_newer_unsaved_changes() {
    // update → save → update (not yet saved). The save's file events arrive
    // after the second update; reloading on them would undo it.
    let (_dir, manager, _) = manager().await;
    manager.update(|c| c.server.port = 4001).unwrap();
    manager.save().await.unwrap();
    manager.update(|c| c.server.port = 4002).unwrap();

    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::Unchanged
    );
    assert_eq!(manager.get().server.port, 4002);
}

#[tokio::test]
async fn an_outside_edit_is_applied_once() {
    let (_dir, manager, synced) = manager().await;
    edit_port(manager.config_path(), 4100);

    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::Applied
    );
    assert_eq!(manager.get().server.port, 4100);
    assert_eq!(synced.load(Ordering::SeqCst), 1);

    // Duplicate events for the same edit do nothing.
    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::Unchanged
    );
    assert_eq!(synced.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_outside_edit_right_after_an_app_save_is_not_lost() {
    let (_dir, manager, _) = manager().await;
    manager.update(|c| c.server.port = 4001).unwrap();
    manager.save().await.unwrap();
    // No pause: the edit lands before the watcher has looked at the save.
    edit_port(manager.config_path(), 4200);

    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::Applied
    );
    assert_eq!(manager.get().server.port, 4200);

    // And the app's next save keeps it rather than writing the old value back.
    manager
        .update(|c| c.server.enable_cors = !c.server.enable_cors)
        .unwrap();
    manager.save().await.unwrap();
    let on_disk: AppConfig =
        serde_yaml::from_str(&std::fs::read_to_string(manager.config_path()).unwrap()).unwrap();
    assert_eq!(on_disk.server.port, 4200);
}

#[tokio::test]
async fn a_save_over_an_edit_not_yet_loaded_is_noticed_and_the_edit_backed_up() {
    // The other order: the outside edit lands first and the app saves before
    // the watcher's debounce has run. The save wins, but the edit survives in
    // the backup the save makes and the overwrite is logged.
    let (dir, manager, _) = manager().await;
    assert!(!manager.warn_if_overwriting_an_unloaded_edit().await);

    edit_port(manager.config_path(), 4250);
    let edited = std::fs::read(manager.config_path()).unwrap();
    assert!(manager.warn_if_overwriting_an_unloaded_edit().await);

    manager.update(|c| c.server.port = 4251).unwrap();
    manager.save().await.unwrap();
    let kept = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".backup."))
        .any(|e| std::fs::read(e.path()).unwrap() == edited);
    assert!(kept, "the overwritten edit is in a backup");
    assert!(!manager.warn_if_overwriting_an_unloaded_edit().await);
}

#[tokio::test]
async fn applying_an_edit_does_not_write_the_file_so_it_cannot_loop() {
    let (dir, manager, _) = manager().await;
    edit_port(manager.config_path(), 4300);
    let before = std::fs::read(manager.config_path()).unwrap();
    let backups_before = backups(dir.path());

    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::Applied
    );
    assert_eq!(std::fs::read(manager.config_path()).unwrap(), before);
    assert_eq!(backups(dir.path()), backups_before);

    // A subscriber that saves in response writes content the gate then
    // knows: the events from that save end the chain.
    manager.save().await.unwrap();
    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::Unchanged
    );
}

#[tokio::test]
async fn a_comment_only_edit_changes_nothing_and_notifies_nobody() {
    let (_dir, manager, synced) = manager().await;
    let path = manager.config_path().clone();
    let mut contents = std::fs::read(&path).unwrap();
    contents.extend_from_slice(b"\n# a note to self\n");
    write_like_an_editor(&path, &contents);

    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::Equivalent
    );
    assert_eq!(synced.load(Ordering::SeqCst), 0);
    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::Unchanged
    );
}

#[tokio::test]
async fn unparseable_yaml_keeps_the_running_config_and_the_users_file() {
    let (dir, manager, synced) = manager().await;
    manager.update(|c| c.server.port = 4001).unwrap();
    manager.save().await.unwrap();
    let synced_before = synced.load(Ordering::SeqCst);
    let backups_before = backups(dir.path());

    let broken = b"server:\n  port: [this is not\n   a config\n";
    write_like_an_editor(manager.config_path(), broken);

    let error = manager.reload_from_disk(false).await.unwrap_err();
    assert!(error.to_string().contains("parse"), "{error}");
    assert_eq!(manager.get().server.port, 4001);
    assert_eq!(synced.load(Ordering::SeqCst), synced_before);
    // The half-finished edit is still there for the user to fix: no backup
    // was restored over it, and none was made of it.
    assert_eq!(std::fs::read(manager.config_path()).unwrap(), broken);
    assert_eq!(backups(dir.path()), backups_before);

    // Reported once, not once per event.
    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::StillRejected
    );
}

#[tokio::test]
async fn valid_yaml_that_fails_validation_is_rejected_too() {
    let (_dir, manager, _) = manager().await;
    let path = manager.config_path().clone();
    let mut config: AppConfig =
        serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let good_host = config.server.host.clone();
    config.server.host = String::new();
    config.server.port = 4400;
    write_like_an_editor(&path, serde_yaml::to_string(&config).unwrap().as_bytes());

    let error = manager.reload_from_disk(false).await.unwrap_err();
    assert!(error.to_string().contains("host"), "{error}");
    // Nothing of the rejected file leaked into the running config.
    assert_eq!(manager.get().server.host, good_host);
    assert_ne!(manager.get().server.port, 4400);
}

#[tokio::test]
async fn fixing_a_rejected_edit_applies_it() {
    let (_dir, manager, _) = manager().await;
    write_like_an_editor(manager.config_path(), b"clients: {not: a list");
    assert!(manager.reload_from_disk(false).await.is_err());

    edit_port_from_default(manager.config_path(), 4500);
    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::Applied
    );
    assert_eq!(manager.get().server.port, 4500);
}

/// Like [`edit_port`], for when the file on disk is not parseable.
fn edit_port_from_default(path: &Path, port: u16) {
    let mut config = AppConfig::default();
    config.server.port = port;
    write_like_an_editor(path, serde_yaml::to_string(&config).unwrap().as_bytes());
}

#[tokio::test]
async fn reverting_a_rejected_edit_is_recognized_as_the_running_config() {
    let (_dir, manager, synced) = manager().await;
    let path = manager.config_path().clone();
    let good = std::fs::read(&path).unwrap();

    write_like_an_editor(&path, b"server: [");
    assert!(manager.reload_from_disk(false).await.is_err());

    write_like_an_editor(&path, &good);
    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::Unchanged
    );
    assert_eq!(synced.load(Ordering::SeqCst), 0);

    // The same mistake made again later is a fresh report, not a silent one.
    write_like_an_editor(&path, b"server: [");
    assert!(manager.reload_from_disk(false).await.is_err());
}

#[tokio::test]
async fn an_app_save_after_a_rejected_edit_wins_and_is_not_reloaded() {
    let (_dir, manager, _) = manager().await;
    write_like_an_editor(manager.config_path(), b"server: [");
    assert!(manager.reload_from_disk(false).await.is_err());

    manager.update(|c| c.server.port = 4600).unwrap();
    manager.save().await.unwrap();
    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::Unchanged
    );
    assert_eq!(manager.get().server.port, 4600);
}

#[tokio::test]
async fn a_missing_or_empty_file_is_a_save_in_progress_not_a_config() {
    let (_dir, manager, synced) = manager().await;
    let path = manager.config_path().clone();
    manager.update(|c| c.server.port = 4001).unwrap();
    manager.save().await.unwrap();
    let synced_before = synced.load(Ordering::SeqCst);

    // Truncated in place (an empty document must never become "all defaults").
    std::fs::write(&path, b"").unwrap();
    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::Missing
    );
    std::fs::write(&path, b"  \n\n").unwrap();
    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::Missing
    );

    // Removed (the gap in a delete-then-create save).
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::Missing
    );
    assert!(!path.exists(), "a reload must not recreate the file");

    assert_eq!(manager.get().server.port, 4001);
    assert_eq!(synced.load(Ordering::SeqCst), synced_before);

    // The manual reload reports the missing file instead of resetting to defaults.
    assert!(manager.reload().await.is_err());
    assert_eq!(manager.get().server.port, 4001);
}

#[tokio::test]
async fn manual_reload_forces_disk_over_unsaved_memory_and_validates() {
    let (_dir, manager, _) = manager().await;
    manager.update(|c| c.server.port = 4001).unwrap();
    manager.save().await.unwrap();
    manager.update(|c| c.server.port = 4002).unwrap(); // never saved

    // The watcher would leave this alone; an explicit reload does not.
    manager.reload().await.unwrap();
    assert_eq!(manager.get().server.port, 4001);

    // And it refuses a broken file the same way the watcher does.
    write_like_an_editor(manager.config_path(), b"server: [");
    assert!(manager.reload().await.is_err());
    assert_eq!(manager.get().server.port, 4001);
}

#[tokio::test]
async fn clones_share_what_is_known_about_the_file() {
    // Commands save through one clone; the watcher runs on another.
    let (_dir, manager, _) = manager().await;
    let other = manager.clone();
    other.update(|c| c.server.port = 4700).unwrap();
    other.save().await.unwrap();
    assert_eq!(
        manager.reload_from_disk(false).await.unwrap(),
        ReloadOutcome::Unchanged
    );
}

// --- With the real filesystem watcher ---------------------------------------

const FAST: Debounce = Debounce {
    quiet: Duration::from_millis(100),
    max_wait: Duration::from_secs(1),
};

/// Poll until `check` holds; filesystem notification latency varies by OS.
async fn eventually(what: &str, check: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while !check() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for: {what}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watcher_applies_editor_saves_and_ignores_the_apps_own() {
    let (_dir, manager, synced) = manager().await;
    let path = manager.config_path().clone();
    let _watcher = manager.start_watching_with(FAST).unwrap();
    // Let the OS watch settle before the first write.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 1. An editor's rename-based save is picked up, once.
    edit_port(&path, 5100);
    eventually("the outside edit to be applied", || {
        manager.get().server.port == 5100
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(synced.load(Ordering::SeqCst), 1, "one edit, one reload");

    // 2. A second rename-based save still arrives (a watch on the file
    //    itself would have gone deaf after the first rename).
    edit_port(&path, 5200);
    eventually("the second outside edit to be applied", || {
        manager.get().server.port == 5200
    })
    .await;

    // 3. The app's own save is not reloaded: an in-memory change made right
    //    after it would be rolled back if it were.
    manager.update(|c| c.server.port = 5300).unwrap();
    manager.save().await.unwrap();
    manager.update(|c| c.server.port = 5301).unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(manager.get().server.port, 5301);
    manager.save().await.unwrap();

    // 4. A broken edit is ignored and the file left alone...
    write_like_an_editor(&path, b"server: [");
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(manager.get().server.port, 5301);
    assert_eq!(std::fs::read(&path).unwrap(), b"server: [");

    // 5. ...and fixing it is applied.
    edit_port_from_default(&path, 5400);
    eventually("the fixed file to be applied", || {
        manager.get().server.port == 5400
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watcher_applies_an_edit_made_immediately_after_an_app_save() {
    let (_dir, manager, _) = manager().await;
    let path = manager.config_path().clone();
    let _watcher = manager.start_watching_with(FAST).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    manager.update(|c| c.server.port = 5500).unwrap();
    manager.save().await.unwrap();
    // Same burst of events as the save.
    edit_port(&path, 5600);

    eventually("the edit that followed the save to be applied", || {
        manager.get().server.port == 5600
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_the_watcher_stops_reloading() {
    let (_dir, manager, _) = manager().await;
    let path = manager.config_path().clone();
    let watcher = manager.start_watching_with(FAST).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    drop(watcher);

    let before = manager.get().server.port;
    edit_port(&path, 5700);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(manager.get().server.port, before);
}
