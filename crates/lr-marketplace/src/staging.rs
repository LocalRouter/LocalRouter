//! Staged, atomic installation of downloaded skills.
//!
//! A skill is downloaded into `<skills dir>/.staging/<id>/new` (the same
//! filesystem as its destination), checked, and only then renamed to
//! `<skills dir>/<source>/<name>`. When a version is already installed the
//! two directories are exchanged in one atomic step where the OS supports it
//! (macOS, Linux), so the skill directory is never missing. Elsewhere the
//! installed version is first moved to `.staging/<id>/previous` and moved
//! back if the new one cannot take its place, so the destination always
//! holds one whole version (but is briefly absent between the two renames).
//!
//! `.staging/<id>/restore-to` records where `previous` came from. If the
//! process dies between the two renames, [`recover`] puts it back at the next
//! start; everything else left in `.staging` is removed.

use crate::skill_sources::{prepare_skill_file_path, validate_skill_file_path};
use crate::types::MarketplaceError;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const STAGING_DIR: &str = ".staging";
const NEW_DIR: &str = "new";
const PREVIOUS_DIR: &str = "previous";
const RESTORE_FILE: &str = "restore-to";

fn install_error(message: String) -> MarketplaceError {
    MarketplaceError::InstallError(message)
}

/// Whether `path` is a directory itself (not a link to one).
fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

/// One in-progress install. Dropping it removes its staging directory, which
/// covers failed and cancelled downloads.
pub(crate) struct StagedInstall {
    dir: PathBuf,
    /// Set when `previous` could not be moved back and must survive for
    /// [`recover`].
    keep: bool,
    /// Exclusive lock on `<id>/.lock` for as long as the install runs, so the
    /// startup [`recover`] of another LocalRouter instance sharing the data
    /// directory leaves it alone.
    lock: Option<std::fs::File>,
}

/// Lock file inside each staging directory.
const LOCK_FILE: &str = ".lock";

/// Whether a live install (in any process) holds `staged`.
fn is_held(staged: &Path) -> bool {
    match std::fs::File::open(staged.join(LOCK_FILE)) {
        Ok(file) => matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
        Err(_) => false,
    }
}

impl StagedInstall {
    /// Create `<root>/.staging/<id>/new`.
    pub(crate) fn create(root: &Path) -> Result<Self, MarketplaceError> {
        let staging_root = root.join(STAGING_DIR);
        match std::fs::symlink_metadata(&staging_root) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(install_error(format!(
                    "Staging path is not a directory: {}",
                    staging_root.display()
                )))
            }
            Err(e) if e.kind() == ErrorKind::NotFound => match std::fs::create_dir(&staging_root) {
                Ok(()) => {}
                // A concurrent install created it first.
                Err(e) if e.kind() == ErrorKind::AlreadyExists && is_real_dir(&staging_root) => {}
                Err(e) => {
                    return Err(install_error(format!(
                        "Failed to create staging directory: {e}"
                    )))
                }
            },
            Err(e) => {
                return Err(install_error(format!(
                    "Failed to inspect staging directory: {e}"
                )))
            }
        }
        let dir = staging_root.join(uuid::Uuid::new_v4().to_string());
        let lock = std::fs::create_dir(&dir)
            .and_then(|()| std::fs::File::create(dir.join(LOCK_FILE)))
            .and_then(|file| file.lock().map(|()| file))
            .and_then(|file| std::fs::create_dir(dir.join(NEW_DIR)).map(|()| file))
            .map_err(|e| {
                let _ = std::fs::remove_dir_all(&dir);
                install_error(format!("Failed to create staging directory: {e}"))
            })?;
        Ok(Self {
            dir,
            keep: false,
            lock: Some(lock),
        })
    }

    /// The empty directory to download the skill into.
    pub(crate) fn download_dir(&self) -> PathBuf {
        self.dir.join(NEW_DIR)
    }

    /// Swap the downloaded skill into `<root>/<relative>` (`<source>/<name>`).
    ///
    /// On failure the previously installed version, if any, is back in place.
    pub(crate) fn promote(
        mut self,
        root: &Path,
        relative: &str,
    ) -> Result<PathBuf, MarketplaceError> {
        // Creates `<source>/` and refuses symlinks on the way to the target.
        let target = prepare_skill_file_path(root, relative)?;
        let new = self.dir.join(NEW_DIR);
        let previous = self.dir.join(PREVIOUS_DIR);

        // Held from the first rename of a two-step replace to the end of this
        // call (including a rollback), so skill readers in this process wait
        // out the moment the skill directory is missing.
        let mut _gap: Option<lr_utils::fs::TwoStepReplace> = None;
        let had_previous = match std::fs::symlink_metadata(&target) {
            Ok(_) => true,
            Err(e) if e.kind() == ErrorKind::NotFound => false,
            Err(e) => {
                return Err(install_error(format!(
                    "Failed to inspect the skill directory: {e}"
                )))
            }
        };
        if had_previous {
            // Where the OS can exchange the two directories in one step, the
            // skill directory never goes missing; the old version ends up in
            // `new`, which the drop removes with the rest of the staging.
            #[cfg(test)]
            let exchange = if tests::TWO_STEP_ONLY.with(std::cell::Cell::get) {
                Ok(false)
            } else {
                lr_utils::fs::exchange_paths(&new, &target)
            };
            #[cfg(not(test))]
            let exchange = lr_utils::fs::exchange_paths(&new, &target);
            match exchange {
                Ok(true) => return Ok(target),
                Ok(false) => {}
                Err(e) => warn!(
                    "Atomic swap of {:?} failed ({}); replacing it in two steps",
                    target, e
                ),
            }
            _gap = Some(lr_utils::fs::TwoStepReplace::begin());
            std::fs::write(self.dir.join(RESTORE_FILE), relative)
                .and_then(|()| std::fs::rename(&target, &previous))
                .map_err(|e| {
                    install_error(format!(
                        "Failed to set aside the installed version of the skill: {e}. \
                         It was left unchanged."
                    ))
                })?;
        }

        if let Err(e) = std::fs::rename(&new, &target) {
            if !had_previous {
                return Err(install_error(format!(
                    "Failed to move the skill into place: {e}"
                )));
            }
            return Err(match std::fs::rename(&previous, &target) {
                Ok(()) => install_error(format!(
                    "Failed to move the skill into place: {e}. \
                     The previously installed version was kept."
                )),
                Err(rollback) => {
                    self.keep = true;
                    install_error(format!(
                        "Failed to move the skill into place ({e}) and to put the previous \
                         version back ({rollback}). It is saved in {} and will be restored \
                         when LocalRouter next starts.",
                        previous.display()
                    ))
                }
            });
        }
        Ok(target)
    }
}

impl Drop for StagedInstall {
    fn drop(&mut self) {
        // Closed first: Windows cannot remove a directory holding an open file.
        drop(self.lock.take());
        if self.keep {
            return;
        }
        if let Err(e) = std::fs::remove_dir_all(&self.dir) {
            if e.kind() != ErrorKind::NotFound {
                warn!("Failed to remove staging directory {:?}: {}", self.dir, e);
            }
        }
    }
}

/// Where an orphaned `previous` belongs, if its record is intact.
fn restore_target(root: &Path, staged: &Path) -> Option<PathBuf> {
    let relative = std::fs::read_to_string(staged.join(RESTORE_FILE)).ok()?;
    validate_skill_file_path(&relative).ok()?;
    if Path::new(&relative).components().count() != 2 {
        return None;
    }
    prepare_skill_file_path(root, &relative).ok()
}

/// Clean `<root>/.staging` after an unclean shutdown. Call once at startup,
/// before any install runs.
///
/// A `previous` whose skill directory is missing is the only copy of an
/// installed skill (the process died mid-swap) and is moved back. All other
/// staging content is an unfinished download and is removed.
pub(crate) fn recover(root: &Path) {
    let staging_root = root.join(STAGING_DIR);
    if !is_real_dir(&staging_root) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&staging_root) else {
        return;
    };
    for entry in entries.flatten() {
        let staged = entry.path();
        if !is_real_dir(&staged) {
            let _ = std::fs::remove_file(&staged);
            continue;
        }
        // An install running in another instance on the same data directory.
        if is_held(&staged) {
            continue;
        }
        let previous = staged.join(PREVIOUS_DIR);
        if is_real_dir(&previous) {
            if let Some(target) = restore_target(root, &staged) {
                if matches!(
                    std::fs::symlink_metadata(&target),
                    Err(ref e) if e.kind() == ErrorKind::NotFound
                ) {
                    match std::fs::rename(&previous, &target) {
                        Ok(()) => info!("Restored interrupted skill install at {:?}", target),
                        Err(e) => {
                            warn!("Failed to restore skill {:?}: {}; keeping it", target, e);
                            continue;
                        }
                    }
                }
            }
        }
        match std::fs::remove_dir_all(&staged) {
            Ok(()) => info!("Removed stale skill staging directory {:?}", staged),
            Err(e) => warn!("Failed to remove stale staging {:?}: {}", staged, e),
        }
    }
}

/// Check a downloaded skill before it replaces anything: `SKILL.md` must be a
/// regular file that the skill loader accepts.
pub(crate) fn validate_downloaded_skill(dir: &Path) -> Result<(), MarketplaceError> {
    let manifest = dir.join("SKILL.md");
    if !std::fs::symlink_metadata(&manifest).is_ok_and(|m| m.is_file()) {
        return Err(install_error(
            "The download does not contain a SKILL.md file".to_string(),
        ));
    }
    let content = std::fs::read_to_string(&manifest)
        .map_err(|e| install_error(format!("SKILL.md cannot be read as text: {e}")))?;
    lr_skills::discovery::parse_skill_md(&content)
        .map_err(|e| install_error(format!("SKILL.md is not a valid skill manifest: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill_sources::SkillSourcesClient;
    use crate::types::SkillListing;
    use crate::MarketplaceService;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const MANIFEST_V1: &str = "---\nname: example\ndescription: one\n---\nBody one";
    const MANIFEST_V2: &str = "---\nname: example\ndescription: two\n---\nBody two";

    /// Serve fixed responses by request path on a loopback socket.
    async fn serve(routes: &[(&str, u16, &str)]) -> String {
        let routes: Arc<HashMap<String, (u16, String)>> = Arc::new(
            routes
                .iter()
                .map(|(path, status, body)| (path.to_string(), (*status, body.to_string())))
                .collect(),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let routes = routes.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => request.extend_from_slice(&chunk[..read]),
                        }
                    }
                    let request = String::from_utf8_lossy(&request).to_string();
                    let path = request.split_whitespace().nth(1).unwrap_or("/");
                    let (status, body) = routes
                        .get(path)
                        .cloned()
                        .unwrap_or((404, "missing".to_string()));
                    let response = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        base
    }

    fn listing(base: &str, files: &[&str]) -> SkillListing {
        serde_json::from_value(serde_json::json!({
            "name": "example", "source_id": "test", "source_label": "Source",
            "source_repo": "", "source_path": "", "source_branch": "main",
            "skill_md_url": format!("{base}/SKILL.md"), "is_multi_file": !files.is_empty(),
            "files": files.iter().map(|path| serde_json::json!({
                "path": path, "url": format!("{base}/{path}")
            })).collect::<Vec<_>>()
        }))
        .unwrap()
    }

    fn service(data_dir: &Path) -> MarketplaceService {
        let mut service = MarketplaceService::new(Default::default(), data_dir.to_path_buf());
        service.skill_sources_client = SkillSourcesClient::for_tests();
        service
    }

    fn tree(dir: &Path) -> Vec<String> {
        fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                out.push(
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
                if is_real_dir(&path) {
                    walk(base, &path, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    fn install_v1(skills: &Path) -> PathBuf {
        let target = skills.join("Source/example");
        std::fs::create_dir_all(target.join("scripts")).unwrap();
        std::fs::write(target.join("SKILL.md"), MANIFEST_V1).unwrap();
        std::fs::write(target.join("scripts/old.sh"), "old").unwrap();
        target
    }

    #[tokio::test]
    async fn install_is_promoted_and_staging_is_emptied() {
        let data = tempfile::tempdir().unwrap();
        let base = serve(&[
            ("/SKILL.md", 200, MANIFEST_V1),
            ("/scripts/run.sh", 200, "run"),
        ])
        .await;
        let service = service(data.path());
        let target = service
            .download_skill(&listing(&base, &["scripts/run.sh"]))
            .await
            .unwrap();
        let skills = service.skills_data_dir();
        assert_eq!(target, skills.join("Source/example"));
        assert_eq!(
            tree(&skills),
            [
                ".staging",
                "Source",
                "Source/example",
                "Source/example/SKILL.md",
                "Source/example/scripts",
                "Source/example/scripts/run.sh",
            ]
        );
    }

    #[tokio::test]
    async fn failed_install_leaves_no_partial_skill() {
        let data = tempfile::tempdir().unwrap();
        // The manifest and the first file arrive; the second file fails.
        let base = serve(&[
            ("/SKILL.md", 200, MANIFEST_V1),
            ("/scripts/a.sh", 200, "a"),
            ("/scripts/b.sh", 500, "boom"),
        ])
        .await;
        let service = service(data.path());
        let error = service
            .download_skill(&listing(&base, &["scripts/a.sh", "scripts/b.sh"]))
            .await
            .unwrap_err();
        assert!(
            matches!(error, MarketplaceError::NetworkError(_)),
            "{error}"
        );
        assert_eq!(tree(&service.skills_data_dir()), [".staging"]);
    }

    #[tokio::test]
    async fn failed_update_preserves_the_installed_version() {
        let data = tempfile::tempdir().unwrap();
        let service = service(data.path());
        let skills = service.skills_data_dir();
        let target = install_v1(&skills);
        let before = tree(&skills);

        let base = serve(&[
            ("/SKILL.md", 200, MANIFEST_V2),
            ("/scripts/new.sh", 500, "boom"),
        ])
        .await;
        assert!(service
            .download_skill(&listing(&base, &["scripts/new.sh"]))
            .await
            .is_err());
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            MANIFEST_V1
        );
        assert_eq!(
            std::fs::read_to_string(target.join("scripts/old.sh")).unwrap(),
            "old"
        );
        let mut expected = before;
        expected.push(".staging".to_string());
        expected.sort();
        assert_eq!(tree(&skills), expected);
    }

    #[tokio::test]
    async fn invalid_manifest_never_replaces_the_installed_version() {
        let data = tempfile::tempdir().unwrap();
        let service = service(data.path());
        let skills = service.skills_data_dir();
        let target = install_v1(&skills);

        let base = serve(&[("/SKILL.md", 200, "<html>rate limited</html>")]).await;
        let error = service
            .download_skill(&listing(&base, &[]))
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("SKILL.md is not a valid skill manifest"),
            "{error}"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            MANIFEST_V1
        );
        assert!(tree(&skills.join(STAGING_DIR)).is_empty());
    }

    #[tokio::test]
    async fn update_replaces_the_whole_tree() {
        let data = tempfile::tempdir().unwrap();
        let service = service(data.path());
        let skills = service.skills_data_dir();
        install_v1(&skills);

        let base = serve(&[
            ("/SKILL.md", 200, MANIFEST_V2),
            ("/scripts/new.sh", 200, "new"),
        ])
        .await;
        let target = service
            .download_skill(&listing(&base, &["scripts/new.sh"]))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            MANIFEST_V2
        );
        // Nothing of the old version is mixed into the new one.
        assert_eq!(
            tree(&skills),
            [
                ".staging",
                "Source",
                "Source/example",
                "Source/example/SKILL.md",
                "Source/example/scripts",
                "Source/example/scripts/new.sh",
            ]
        );
    }

    #[tokio::test]
    async fn concurrent_installs_of_one_skill_are_serialized() {
        let data = tempfile::tempdir().unwrap();
        let service = Arc::new(service(data.path()));
        let base = serve(&[
            ("/SKILL.md", 200, MANIFEST_V1),
            ("/scripts/run.sh", 200, "run"),
        ])
        .await;
        let listing = listing(&base, &["scripts/run.sh"]);
        let installs: Vec<_> = (0..6)
            .map(|_| {
                let service = service.clone();
                let listing = listing.clone();
                tokio::spawn(async move { service.download_skill(&listing).await })
            })
            .collect();
        for install in installs {
            install.await.unwrap().unwrap();
        }
        assert_eq!(
            tree(&service.skills_data_dir()),
            [
                ".staging",
                "Source",
                "Source/example",
                "Source/example/SKILL.md",
                "Source/example/scripts",
                "Source/example/scripts/run.sh",
            ]
        );
        assert!(Arc::ptr_eq(
            &service.install_lock("Source/Example"),
            &service.install_lock("source/example")
        ));
        assert!(!Arc::ptr_eq(
            &service.install_lock("source/example"),
            &service.install_lock("source/other")
        ));
        // A clone installs into the same directory and shares the locks.
        assert!(Arc::ptr_eq(
            &service.install_lock("source/example"),
            &(*service).clone().install_lock("source/example")
        ));
    }

    #[tokio::test]
    async fn delete_waits_for_an_install_of_the_same_skill() {
        let data = tempfile::tempdir().unwrap();
        let service = Arc::new(service(data.path()));
        let skills = service.skills_data_dir();
        let target = install_v1(&skills).canonicalize().unwrap();

        let lock = service.install_lock("Source/example");
        let held = lock.lock().await;
        let removal = tokio::spawn({
            let service = service.clone();
            let target = target.clone();
            async move { service.remove_installed_skill(&target).await }
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(target.is_dir(), "deleted while an install held the skill");
        drop(held);
        removal.await.unwrap().unwrap();
        assert!(!target.exists());

        // Deleting what is already gone succeeds; paths outside are refused.
        service.remove_installed_skill(&target).await.unwrap();
        let outside = tempfile::tempdir().unwrap();
        assert!(service
            .remove_installed_skill(outside.path())
            .await
            .unwrap_err()
            .contains("Not a marketplace skill directory"));
        assert!(outside.path().is_dir());
    }

    #[test]
    fn failed_swap_rolls_back_to_the_previous_version() {
        let data = tempfile::tempdir().unwrap();
        let skills = data.path();
        let target = install_v1(skills);

        let staged = StagedInstall::create(skills).unwrap();
        // Make the second rename fail after the installed version was set aside.
        std::fs::remove_dir(staged.download_dir()).unwrap();
        let error = staged.promote(skills, "Source/example").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("The previously installed version was kept"),
            "{error}"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            MANIFEST_V1
        );
        assert!(target.join("scripts/old.sh").is_file());
        assert!(tree(&skills.join(STAGING_DIR)).is_empty());
    }

    #[test]
    fn recovery_leaves_an_install_running_elsewhere_alone() {
        let data = tempfile::tempdir().unwrap();
        let running = StagedInstall::create(data.path()).unwrap();
        std::fs::write(running.download_dir().join("SKILL.md"), MANIFEST_V1).unwrap();
        // Another instance starts and cleans up.
        recover(data.path());
        assert!(running.download_dir().join("SKILL.md").is_file());
        running.promote(data.path(), "Source/example").unwrap();
        assert!(data.path().join("Source/example/SKILL.md").is_file());
        assert!(tree(&data.path().join(STAGING_DIR)).is_empty());
    }

    /// Readers never find an installed skill missing while it is updated.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn an_update_never_leaves_the_skill_directory_missing() {
        let data = tempfile::tempdir().unwrap();
        let skills = data.path().to_path_buf();
        let target = install_v1(&skills);
        let done = std::sync::atomic::AtomicBool::new(false);
        let missing = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                while !done.load(std::sync::atomic::Ordering::SeqCst) {
                    if std::fs::symlink_metadata(target.join("SKILL.md")).is_err() {
                        missing.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            });
            for round in 0..200 {
                let staged = StagedInstall::create(&skills).unwrap();
                let manifest = if round % 2 == 0 {
                    MANIFEST_V2
                } else {
                    MANIFEST_V1
                };
                std::fs::write(staged.download_dir().join("SKILL.md"), manifest).unwrap();
                staged.promote(&skills, "Source/example").unwrap();
            }
            done.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        assert_eq!(missing.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            MANIFEST_V1
        );
        // The old versions went with their staging directories.
        assert!(tree(&skills.join(STAGING_DIR)).is_empty());
        assert!(!target.join("scripts").exists());
    }

    thread_local! {
        /// Makes `promote` on this thread skip the atomic exchange, as on a
        /// filesystem without one.
        pub(super) static TWO_STEP_ONLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// Where the filesystem has no atomic exchange, an update replaces the
    /// skill in two renames; readers using `retry_while_missing` wait out the
    /// gap instead of failing, on every platform.
    #[test]
    fn readers_wait_out_a_two_rename_update() {
        let data = tempfile::tempdir().unwrap();
        let skills = data.path().to_path_buf();
        let target = install_v1(&skills);
        let done = std::sync::atomic::AtomicBool::new(false);
        let failed = std::sync::atomic::AtomicUsize::new(0);
        let reads = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                while !done.load(std::sync::atomic::Ordering::SeqCst) {
                    reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if lr_utils::fs::retry_while_missing(|| {
                        std::fs::read_to_string(target.join("SKILL.md"))
                    })
                    .is_err()
                    {
                        failed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            });
            TWO_STEP_ONLY.with(|only| only.set(true));
            for round in 0..100 {
                let staged = StagedInstall::create(&skills).unwrap();
                let manifest = if round % 2 == 0 {
                    MANIFEST_V2
                } else {
                    MANIFEST_V1
                };
                std::fs::write(staged.download_dir().join("SKILL.md"), manifest).unwrap();
                staged.promote(&skills, "Source/example").unwrap();
            }
            TWO_STEP_ONLY.with(|only| only.set(false));
            done.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        assert!(reads.load(std::sync::atomic::Ordering::SeqCst) > 0);
        assert_eq!(failed.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            MANIFEST_V1
        );
        assert!(tree(&skills.join(STAGING_DIR)).is_empty());
    }

    #[test]
    fn failed_first_install_swap_leaves_no_skill() {
        let data = tempfile::tempdir().unwrap();
        let staged = StagedInstall::create(data.path()).unwrap();
        std::fs::remove_dir(staged.download_dir()).unwrap();
        assert!(staged.promote(data.path(), "Source/example").is_err());
        assert!(!data.path().join("Source/example").exists());
        assert!(tree(&data.path().join(STAGING_DIR)).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn promotion_refuses_symlinked_destinations() {
        let data = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), data.path().join("Source")).unwrap();
        let staged = StagedInstall::create(data.path()).unwrap();
        std::fs::write(staged.download_dir().join("SKILL.md"), MANIFEST_V1).unwrap();
        assert!(staged.promote(data.path(), "Source/example").is_err());
        assert!(tree(outside.path()).is_empty());
    }

    #[test]
    fn startup_recovery_restores_an_interrupted_swap_and_removes_the_rest() {
        let data = tempfile::tempdir().unwrap();
        let skills = data.path().join("marketplace-skills");
        let staging = skills.join(STAGING_DIR);

        // Died between the two renames: the only copy is in staging.
        let interrupted = staging.join("interrupted");
        std::fs::create_dir_all(interrupted.join(PREVIOUS_DIR)).unwrap();
        std::fs::write(interrupted.join(PREVIOUS_DIR).join("SKILL.md"), MANIFEST_V1).unwrap();
        std::fs::write(interrupted.join(RESTORE_FILE), "Source/example").unwrap();
        std::fs::create_dir_all(interrupted.join(NEW_DIR)).unwrap();

        // Died mid-download.
        let partial = staging.join("partial");
        std::fs::create_dir_all(partial.join(NEW_DIR)).unwrap();
        std::fs::write(partial.join(NEW_DIR).join("SKILL.md"), "half").unwrap();

        // Died after the swap, before cleanup: the new version is installed.
        let finished = staging.join("finished");
        std::fs::create_dir_all(finished.join(PREVIOUS_DIR)).unwrap();
        std::fs::write(finished.join(PREVIOUS_DIR).join("SKILL.md"), MANIFEST_V1).unwrap();
        std::fs::write(finished.join(RESTORE_FILE), "Source/other").unwrap();
        std::fs::create_dir_all(skills.join("Source/other")).unwrap();
        std::fs::write(skills.join("Source/other/SKILL.md"), MANIFEST_V2).unwrap();

        // A record pointing outside the skills directory is never followed.
        let hostile = staging.join("hostile");
        std::fs::create_dir_all(hostile.join(PREVIOUS_DIR)).unwrap();
        std::fs::write(hostile.join(RESTORE_FILE), "../../escaped").unwrap();
        std::fs::write(staging.join("stray-file"), "x").unwrap();

        // Constructing the service is what runs recovery.
        let _service = MarketplaceService::new(Default::default(), data.path().to_path_buf());

        assert_eq!(
            tree(&skills),
            [
                ".staging",
                "Source",
                "Source/example",
                "Source/example/SKILL.md",
                "Source/other",
                "Source/other/SKILL.md",
            ]
        );
        assert_eq!(
            std::fs::read_to_string(skills.join("Source/example/SKILL.md")).unwrap(),
            MANIFEST_V1
        );
        assert_eq!(
            std::fs::read_to_string(skills.join("Source/other/SKILL.md")).unwrap(),
            MANIFEST_V2
        );
        assert!(!data.path().join("escaped").exists());
    }

    #[test]
    fn recovery_tolerates_a_missing_staging_directory() {
        let data = tempfile::tempdir().unwrap();
        recover(data.path());
        recover(&data.path().join("missing"));
        assert!(tree(data.path()).is_empty());
    }
}
