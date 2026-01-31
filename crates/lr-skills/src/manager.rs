//! Skill manager - central coordination for skill discovery and access

use super::discovery;
use super::types::{SkillDefinition, SkillInfo};
use dashmap::DashMap;
use parking_lot::RwLock;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

/// Central skill manager
pub struct SkillManager {
    /// All discovered skills — readers clone the inner Arc for a snapshot
    skills: Arc<RwLock<Arc<Vec<SkillDefinition>>>>,

    /// Optional Tauri app handle for event emission
    app_handle: Option<tauri::AppHandle>,

    /// Extraction directories pending cleanup: path -> time superseded
    pending_cleanup: Arc<DashMap<PathBuf, Instant>>,
}

impl Default for SkillManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillManager {
    /// Create a new skill manager
    pub fn new() -> Self {
        Self {
            skills: Arc::new(RwLock::new(Arc::new(Vec::new()))),
            app_handle: None,
            pending_cleanup: Arc::new(DashMap::new()),
        }
    }

    /// Set the Tauri app handle for event emission
    pub fn set_app_handle(&mut self, handle: tauri::AppHandle) {
        self.app_handle = Some(handle);
    }

    /// Start the background cleanup task for old extraction directories
    pub fn start_cleanup_task(&self) {
        let pending = self.pending_cleanup.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let now = Instant::now();
                let mut to_remove = Vec::new();
                for entry in pending.iter() {
                    if now.duration_since(*entry.value()).as_secs() >= 30 {
                        let path = entry.key().clone();
                        to_remove.push(path);
                    }
                }
                for path in to_remove {
                    if let Err(e) = std::fs::remove_dir_all(&path) {
                        warn!("Failed to clean up old extraction dir {:?}: {}", path, e);
                    } else {
                        info!("Cleaned up old extraction dir: {:?}", path);
                    }
                    pending.remove(&path);
                }
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

        for path_str in paths {
            let path = PathBuf::from(path_str);
            let result = discovery::discover_skills(&path);
            info!(
                "Skill path '{}': found {} skills",
                path_str,
                result.skills.len()
            );
            all_skills.extend(result.skills);
            old_dirs.extend(result.old_extraction_dirs);
        }

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

        // Atomic swap
        *self.skills.write() = Arc::new(deduped);

        // Queue old dirs for cleanup
        let now = Instant::now();
        for dir in old_dirs {
            self.pending_cleanup.insert(dir, now);
        }
    }

    /// Rescan specific paths and atomically update the skill list
    pub fn rescan_paths(
        &self,
        all_paths: &[String],
        disabled_skills: &[String],
    ) -> Vec<SkillInfo> {
        self.initial_scan(all_paths, disabled_skills);
        self.emit_skills_changed();
        self.list()
    }

    /// Legacy rescan method (calls through to rescan_paths)
    pub fn rescan(
        &self,
        paths: &[String],
        disabled_skills: &[String],
    ) -> Vec<SkillInfo> {
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

    /// Get a specific skill by name
    pub fn get(&self, name: &str) -> Option<SkillDefinition> {
        let snapshot = self.snapshot();
        snapshot
            .iter()
            .find(|s| s.metadata.name == name)
            .cloned()
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

        // Build the full path and canonicalize
        let requested = skill.skill_dir.join(relative_path);
        let canonical = requested
            .canonicalize()
            .map_err(|e| format!("Failed to resolve path '{}': {}", relative_path, e))?;

        let skill_dir_canonical = skill
            .skill_dir
            .canonicalize()
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

        std::fs::read_to_string(&canonical)
            .map_err(|e| format!("Failed to read resource '{}': {}", relative_path, e))
    }

    /// Get the skill directory path (for script executor)
    pub fn get_skill_dir(&self, skill_name: &str) -> Option<PathBuf> {
        self.get(skill_name).map(|s| s.skill_dir)
    }

    /// Emit skills-changed event to frontend
    fn emit_skills_changed(&self) {
        if let Some(ref handle) = self.app_handle {
            use tauri::Emitter;
            if let Err(e) = handle.emit("skills-changed", ()) {
                warn!("Failed to emit skills-changed event: {}", e);
            }
        }
    }
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

        let content = manager.get_resource("test-skill", "references/doc.md").unwrap();
        assert_eq!(content, "# Documentation");
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
            &[
                dir_a.display().to_string(),
                dir_b.display().to_string(),
            ],
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
