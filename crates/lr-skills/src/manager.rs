//! Skill manager - central coordination for skill discovery and access

use super::archive::{self, ExtractionLease};
use super::discovery;
use super::types::{SkillDefinition, SkillInfo};
use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

type ChangeCallback = Arc<dyn Fn() + Send + Sync>;

/// Central skill manager
pub struct SkillManager {
    /// All discovered skills — readers clone the inner Arc for a snapshot
    skills: Arc<RwLock<Arc<Vec<SkillDefinition>>>>,

    change_callback: RwLock<Option<ChangeCallback>>,

    /// Optional Tauri app handle for event emission
    app_handle: Option<tauri::AppHandle>,

    /// Extraction directories pending cleanup: path -> time superseded
    pending_cleanup: Arc<DashMap<PathBuf, Instant>>,

    /// Source path -> why its last scan could not load it
    source_errors: RwLock<HashMap<String, String>>,

    /// Where skill archives are extracted
    extraction_root: PathBuf,

    /// Leases on the extractions the current skills are served from, so no
    /// LocalRouter process removes them while they are in use
    leases: parking_lot::Mutex<Vec<ExtractionLease>>,
}

/// How long an archive staging directory must sit untouched before startup
/// treats it as abandoned. The extraction root is shared by every running
/// instance, so a younger one may belong to an extraction in progress.
const STALE_STAGING_AGE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

impl Default for SkillManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillManager {
    /// Create a new skill manager extracting archives into the data
    /// directory ([`discovery::extraction_root`]).
    pub fn new() -> Self {
        Self::with_extraction_root(discovery::extraction_root())
    }

    /// Create a skill manager extracting archives under `root`.
    pub fn with_extraction_root(root: PathBuf) -> Self {
        Self {
            skills: Arc::new(RwLock::new(Arc::new(Vec::new()))),
            app_handle: None,
            change_callback: RwLock::new(None),
            pending_cleanup: Arc::new(DashMap::new()),
            source_errors: RwLock::new(HashMap::new()),
            extraction_root: root,
            leases: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Notify MCP sessions as well as the desktop UI when the catalog changes.
    pub fn set_change_callback(&self, callback: ChangeCallback) {
        *self.change_callback.write() = Some(callback);
    }

    /// Set the Tauri app handle for event emission
    pub fn set_app_handle(&mut self, handle: tauri::AppHandle) {
        self.app_handle = Some(handle);
    }

    /// Start the background cleanup task for old extraction directories.
    /// Also removes staging directories abandoned by an earlier run and, once,
    /// the extractions earlier builds left in the OS temp directory. Call
    /// after [`Self::initial_scan`].
    pub fn start_cleanup_task(&self) {
        let root = self.extraction_root.clone();
        archive::cleanup_stale_staging(&root, STALE_STAGING_AGE);
        archive::migrate_legacy_root(&discovery::legacy_extraction_root(), &root);
        let pending = self.pending_cleanup.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let root = root.clone();
                let pending = pending.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    remove_due(&root, &pending, std::time::Duration::from_secs(30))
                })
                .await;
            }
        });
    }

    /// Get a snapshot of skills (cheap Arc clone)
    fn snapshot(&self) -> Arc<Vec<SkillDefinition>> {
        self.skills.read().clone()
    }

    /// Perform initial scan from config paths
    pub fn initial_scan(&self, paths: &[String], disabled_skills: &[String]) {
        let mut all_skills = Vec::new();
        let mut old_dirs = Vec::new();
        let mut errors = HashMap::new();

        let mut leases = Vec::new();
        for path_str in paths {
            let path = PathBuf::from(path_str);
            let result = discovery::discover_skills_in(&path, &self.extraction_root);
            leases.extend(result.leases);
            info!(
                "Skill path '{}': found {} skills",
                path_str,
                result.skills.len()
            );
            all_skills.extend(result.skills);
            old_dirs.extend(result.old_extraction_dirs);
            if let Some(error) = result.error {
                errors.insert(path_str.clone(), error);
            }
        }

        // Extraction directories some skill of this scan is served from,
        // whether or not it survives deduplication.
        let in_use: Vec<PathBuf> = all_skills.iter().map(|s| s.skill_dir.clone()).collect();
        let is_in_use = |dir: &PathBuf| in_use.iter().any(|skill_dir| skill_dir.starts_with(dir));

        // Apply disabled_skills
        for skill in &mut all_skills {
            if disabled_skills.contains(&skill.metadata.name) {
                skill.enabled = false;
            }
        }

        // Deduplicate by name (last one wins)
        let mut seen = std::collections::HashMap::new();
        for skill in all_skills {
            seen.insert(skill.metadata.name.clone(), skill);
        }

        let deduped: Vec<SkillDefinition> = seen.into_values().collect();
        info!("Total skills after dedup: {}", deduped.len());

        // An extraction queued for deletion is in use again when its archive
        // was reverted to that content, or when a failed update fell back to it.
        self.pending_cleanup.retain(|dir, _| !is_in_use(dir));

        // Atomic swap
        *self.skills.write() = Arc::new(deduped);
        *self.source_errors.write() = errors;
        // The new leases are held already; dropping the old ones releases
        // extractions no longer served.
        *self.leases.lock() = leases;

        // Queue old dirs for cleanup, and extractions of archives that are no
        // longer configured at all.
        let now = Instant::now();
        let configured = configured_archives(paths);
        let unconfigured = archive::list_extractions(&self.extraction_root)
            .into_iter()
            .filter(|(_, source)| source.as_ref().is_none_or(|s| !configured.contains(s)))
            .map(|(dir, _)| dir);
        for dir in old_dirs.into_iter().chain(unconfigured) {
            if !is_in_use(&dir) {
                self.pending_cleanup.entry(dir).or_insert(now);
            }
        }
    }

    /// Why each source failed to load in the last scan, keyed by the source
    /// path as configured. Sources that loaded are absent.
    pub fn source_errors(&self) -> HashMap<String, String> {
        self.source_errors.read().clone()
    }

    /// Rescan specific paths and atomically update the skill list
    pub fn rescan_paths(&self, all_paths: &[String], disabled_skills: &[String]) -> Vec<SkillInfo> {
        self.initial_scan(all_paths, disabled_skills);
        self.emit_skills_changed();
        self.list()
    }

    /// Legacy rescan method (calls through to rescan_paths)
    pub fn rescan(&self, paths: &[String], disabled_skills: &[String]) -> Vec<SkillInfo> {
        self.rescan_paths(paths, disabled_skills)
    }

    /// Toggle a skill's enabled state
    pub fn set_skill_enabled(&self, name: &str, enabled: bool) {
        let current = self.snapshot();
        let mut new_skills: Vec<SkillDefinition> = (*current).clone();
        for skill in &mut new_skills {
            if skill.metadata.name == name {
                skill.enabled = enabled;
            }
        }
        *self.skills.write() = Arc::new(new_skills);
        self.emit_skills_changed();
    }

    /// List all discovered skills (lightweight info)
    pub fn list(&self) -> Vec<SkillInfo> {
        let snapshot = self.snapshot();
        snapshot.iter().map(SkillInfo::from).collect()
    }

    /// Get a specific skill by name (exact match only)
    pub fn get(&self, name: &str) -> Option<SkillDefinition> {
        let snapshot = self.snapshot();
        snapshot.iter().find(|s| s.metadata.name == name).cloned()
    }

    /// Find the closest matching skill using layered matching.
    ///
    /// Tries exact → case-insensitive → normalized → fuzzy (Levenshtein).
    /// Only considers enabled skills. Does NOT filter by permissions — that
    /// is the caller's responsibility.
    pub fn find_closest(&self, name: &str) -> Option<(SkillDefinition, crate::fuzzy::MatchKind)> {
        let snapshot = self.snapshot();
        let candidates: Vec<(usize, &str)> = snapshot
            .iter()
            .enumerate()
            .filter(|(_, s)| s.enabled)
            .map(|(i, s)| (i, s.metadata.name.as_str()))
            .collect();

        let (idx, kind) = crate::fuzzy::find_best_match(name, &candidates)?;
        Some((snapshot[idx].clone(), kind))
    }

    /// Get all skill definitions (for MCP tool generation)
    pub fn get_all(&self) -> Arc<Vec<SkillDefinition>> {
        self.snapshot()
    }

    /// Get the content of a resource file within a skill directory
    ///
    /// Performs path traversal validation to ensure the resource stays
    /// within the skill directory.
    pub fn get_resource(&self, skill_name: &str, relative_path: &str) -> Result<String, String> {
        let skill = self
            .get(skill_name)
            .ok_or_else(|| format!("Skill '{}' not found", skill_name))?;

        // Build the full path and canonicalize. Reads retry briefly while
        // the skill directory is missing: where the OS has no atomic
        // directory exchange, a marketplace update replaces it in two renames.
        let requested = skill.skill_dir.join(relative_path);
        let canonical = lr_utils::fs::retry_while_missing(|| requested.canonicalize())
            .map_err(|e| format!("Failed to resolve path '{}': {}", relative_path, e))?;

        let skill_dir_canonical =
            lr_utils::fs::retry_while_missing(|| skill.skill_dir.canonicalize())
                .map_err(|e| format!("Failed to canonicalize skill directory: {}", e))?;

        // Verify the resolved path is within the skill directory
        if !canonical.starts_with(&skill_dir_canonical) {
            return Err(format!(
                "Path traversal denied: '{}' is outside the skill directory",
                relative_path
            ));
        }

        if !canonical.is_file() {
            return Err(format!("Resource '{}' is not a file", relative_path));
        }

        lr_utils::fs::retry_while_missing(|| std::fs::read_to_string(&canonical))
            .map_err(|e| format!("Failed to read resource '{}': {}", relative_path, e))
    }

    /// Emit skills-changed event to frontend
    fn emit_skills_changed(&self) {
        let callback = self.change_callback.read().clone();
        if let Some(callback) = callback {
            callback();
        }
        if let Some(ref handle) = self.app_handle {
            use tauri::Emitter;
            if let Err(e) = handle.emit("skills-changed", ()) {
                warn!("Failed to emit skills-changed event: {}", e);
            }
        }
    }
}

/// Every way a configured source may be recorded as the archive of an
/// extraction: as written and canonicalized.
fn configured_archives(paths: &[String]) -> HashSet<PathBuf> {
    paths
        .iter()
        .flat_map(|path| {
            let path = PathBuf::from(path);
            [archive::archive_identity(&path), path]
        })
        .collect()
}

/// Remove the queued extractions that have waited at least `grace`. One that
/// a LocalRouter process still serves skills from stays queued and is tried
/// again on the next pass. Returns the directories removed.
fn remove_due(
    root: &Path,
    pending: &DashMap<PathBuf, Instant>,
    grace: std::time::Duration,
) -> Vec<PathBuf> {
    let now = Instant::now();
    let due: Vec<PathBuf> = pending
        .iter()
        .filter(|entry| now.duration_since(*entry.value()) >= grace)
        .map(|entry| entry.key().clone())
        .collect();
    let mut removed = Vec::new();
    for path in due {
        match archive::remove_extraction(root, &path) {
            Ok(true) => {
                info!("Cleaned up old extraction dir: {:?}", path);
                pending.remove(&path);
                removed.push(path);
            }
            Ok(false) => {
                info!(
                    "Old extraction dir {:?} is still in use; retrying later",
                    path
                );
            }
            Err(e) => {
                warn!("Failed to clean up old extraction dir {:?}: {}", path, e);
                pending.remove(&path);
            }
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn create_test_skill(dir: &Path, name: &str) {
        fs::write(
            dir.join("SKILL.md"),
            format!(
                "---\nname: {}\ndescription: Test skill\n---\n# {}\nBody content",
                name, name
            ),
        )
        .unwrap();
    }

    #[test]
    fn rescans_and_enable_changes_notify_mcp_observers() {
        let manager = SkillManager::new();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = calls.clone();
        manager.set_change_callback(Arc::new(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }));
        manager.rescan(&[], &[]);
        manager.set_skill_enabled("absent", false);
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    #[test]
    fn test_manager_initial_scan() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("my-skill");
        fs::create_dir(&skill_dir).unwrap();
        create_test_skill(&skill_dir, "my-skill");

        let manager = SkillManager::new();
        manager.initial_scan(&[tmp.path().display().to_string()], &[]);

        let skills = manager.list();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "my-skill");
        assert!(skills[0].enabled);
    }

    #[test]
    fn source_errors_follow_the_latest_scan() {
        let tmp = TempDir::new().unwrap();
        let broken = tmp.path().join("broken.zip");
        fs::write(&broken, b"not a zip").unwrap();
        let source = broken.display().to_string();

        let manager = SkillManager::new();
        manager.initial_scan(std::slice::from_ref(&source), &[]);
        let errors = manager.source_errors();
        assert!(errors[&source].starts_with("not a readable zip archive"));

        manager.initial_scan(&[], &[]);
        assert!(manager.source_errors().is_empty());
    }

    #[test]
    fn extraction_in_use_is_not_left_queued_for_deletion() {
        let tmp = TempDir::new().unwrap();
        let in_use = tmp.path().join("pack-0123");
        fs::create_dir_all(in_use.join("my-skill")).unwrap();
        create_test_skill(&in_use.join("my-skill"), "my-skill");
        let superseded = tmp.path().join("pack-4567");
        fs::create_dir(&superseded).unwrap();

        let manager = SkillManager::new();
        manager
            .pending_cleanup
            .insert(in_use.clone(), Instant::now());
        manager
            .pending_cleanup
            .insert(superseded.clone(), Instant::now());
        manager.initial_scan(&[in_use.display().to_string()], &[]);

        assert_eq!(manager.list().len(), 1);
        assert!(!manager.pending_cleanup.contains_key(&in_use));
        assert!(manager.pending_cleanup.contains_key(&superseded));
    }

    fn zip_with_skill(dir: &Path, file: &str, name: &str) -> PathBuf {
        let path = dir.join(file);
        let mut bytes = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
            writer
                .start_file("SKILL.md", zip::write::SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(
                &mut writer,
                format!("---\nname: {name}\n---\nBody").as_bytes(),
            )
            .unwrap();
            writer.finish().unwrap();
        }
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn extractions_of_archives_no_longer_configured_are_removed() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let a = zip_with_skill(tmp.path(), "a.zip", "a");
        let b = zip_with_skill(tmp.path(), "b.zip", "b");
        let sources = |paths: &[&PathBuf]| -> Vec<String> {
            paths.iter().map(|p| p.display().to_string()).collect()
        };

        let manager = SkillManager::with_extraction_root(root.clone());
        manager.initial_scan(&sources(&[&a, &b]), &[]);
        assert_eq!(manager.list().len(), 2);
        let dir_of = |name: &str| manager.get(name).unwrap().skill_dir;
        let (a_dir, b_dir) = (dir_of("a"), dir_of("b"));
        assert!(manager.pending_cleanup.is_empty());

        // b is removed from the sources: its extraction is released and queued.
        manager.initial_scan(&sources(&[&a]), &[]);
        assert!(manager.pending_cleanup.contains_key(&b_dir));
        assert!(!manager.pending_cleanup.contains_key(&a_dir));

        // Another process still serves b: it stays until released.
        let other = archive::lease_existing(&root, &b_dir).unwrap();
        let grace = std::time::Duration::ZERO;
        assert!(remove_due(&root, &manager.pending_cleanup, grace).is_empty());
        assert!(b_dir.is_dir());
        assert!(manager.pending_cleanup.contains_key(&b_dir));
        drop(other);
        assert_eq!(
            remove_due(&root, &manager.pending_cleanup, grace),
            vec![b_dir.clone()]
        );
        assert!(!b_dir.exists());
        assert!(a_dir.is_dir());
        assert!(manager.pending_cleanup.is_empty());

        // Not yet due: kept.
        manager.initial_scan(&[], &[]);
        assert!(manager.pending_cleanup.contains_key(&a_dir));
        assert!(remove_due(
            &root,
            &manager.pending_cleanup,
            std::time::Duration::from_secs(3600)
        )
        .is_empty());
        assert!(a_dir.is_dir());
    }

    #[test]
    fn test_manager_disabled_skills() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("my-skill");
        fs::create_dir(&skill_dir).unwrap();
        create_test_skill(&skill_dir, "my-skill");

        let manager = SkillManager::new();
        manager.initial_scan(
            &[tmp.path().display().to_string()],
            &["my-skill".to_string()],
        );

        let skills = manager.list();
        assert_eq!(skills.len(), 1);
        assert!(!skills[0].enabled);
    }

    #[test]
    fn test_manager_set_skill_enabled() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("my-skill");
        fs::create_dir(&skill_dir).unwrap();
        create_test_skill(&skill_dir, "my-skill");

        let manager = SkillManager::new();
        manager.initial_scan(&[tmp.path().display().to_string()], &[]);

        assert!(manager.list()[0].enabled);
        manager.set_skill_enabled("my-skill", false);
        assert!(!manager.list()[0].enabled);
        manager.set_skill_enabled("my-skill", true);
        assert!(manager.list()[0].enabled);
    }

    #[test]
    fn test_manager_get_resource() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();
        create_test_skill(skill_dir, "test-skill");

        // Create a reference file
        fs::create_dir(skill_dir.join("references")).unwrap();
        fs::write(skill_dir.join("references/doc.md"), "# Documentation").unwrap();

        let manager = SkillManager::new();
        manager.initial_scan(&[skill_dir.display().to_string()], &[]);

        let content = manager
            .get_resource("test-skill", "references/doc.md")
            .unwrap();
        assert_eq!(content, "# Documentation");
    }

    /// Where the filesystem has no atomic directory exchange, a marketplace
    /// update leaves the skill directory missing between two renames; a read
    /// in that gap waits for the new version instead of failing.
    #[test]
    fn resource_read_waits_out_a_two_rename_update() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("my-skill");
        fs::create_dir(&skill_dir).unwrap();
        create_test_skill(&skill_dir, "test-skill");
        fs::create_dir(skill_dir.join("references")).unwrap();
        fs::write(skill_dir.join("references/doc.md"), "v1").unwrap();
        let manager = SkillManager::new();
        manager.initial_scan(&[skill_dir.display().to_string()], &[]);

        let aside = tmp.path().join("previous");
        let replace = lr_utils::fs::TwoStepReplace::begin();
        fs::rename(&skill_dir, &aside).unwrap();
        let (back, target) = (aside.clone(), skill_dir.clone());
        let mover = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            fs::write(back.join("references/doc.md"), "v2").unwrap();
            fs::rename(&back, &target).unwrap();
            drop(replace);
        });
        let content = manager.get_resource("test-skill", "references/doc.md");
        mover.join().unwrap();
        assert_eq!(content.unwrap(), "v2");
    }

    #[test]
    fn test_manager_path_traversal_blocked() {
        let tmp = TempDir::new().unwrap();
        // Put the skill in a subdirectory so ../secret.txt resolves outside it
        let skill_dir = tmp.path().join("my-skill");
        fs::create_dir(&skill_dir).unwrap();
        create_test_skill(&skill_dir, "test-skill");

        // Create a file outside the skill dir (in the parent temp dir)
        fs::write(tmp.path().join("secret.txt"), "secret data").unwrap();

        let manager = SkillManager::new();
        manager.initial_scan(&[skill_dir.display().to_string()], &[]);

        let result = manager.get_resource("test-skill", "../secret.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("traversal"));
    }

    #[test]
    fn test_manager_deduplicate_by_name() {
        let tmp = TempDir::new().unwrap();

        // Two directories with same-named skill
        let dir_a = tmp.path().join("dir-a");
        fs::create_dir(&dir_a).unwrap();
        create_test_skill(&dir_a, "duplicate-skill");

        let dir_b = tmp.path().join("dir-b");
        fs::create_dir(&dir_b).unwrap();
        create_test_skill(&dir_b, "duplicate-skill");

        let manager = SkillManager::new();
        manager.initial_scan(
            &[dir_a.display().to_string(), dir_b.display().to_string()],
            &[],
        );

        let skills = manager.list();
        assert_eq!(skills.len(), 1);
    }

    #[test]
    fn test_manager_snapshot_isolation() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("my-skill");
        fs::create_dir(&skill_dir).unwrap();
        create_test_skill(&skill_dir, "my-skill");

        let manager = SkillManager::new();
        manager.initial_scan(&[tmp.path().display().to_string()], &[]);

        // Take a snapshot
        let snapshot = manager.get_all();
        assert_eq!(snapshot.len(), 1);

        // Modify the manager's state
        manager.set_skill_enabled("my-skill", false);

        // Snapshot should still show old state
        assert!(snapshot[0].enabled);

        // New snapshot shows updated state
        let new_snapshot = manager.get_all();
        assert!(!new_snapshot[0].enabled);
    }
}
