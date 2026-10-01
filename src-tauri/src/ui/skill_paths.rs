//! Confinement for destructive operations on managed skill directories.

use std::path::{Path, PathBuf};

/// Resolve an existing skill strictly beneath its managed root. Lexical prefix
/// checks alone accept `root/../outside`; canonicalization also catches parent
/// symlinks. Require a manifest so source directories cannot be deleted as skills.
pub(crate) fn managed_skill_directory(root: &Path, candidate: &Path) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("Cannot resolve skills directory: {e}"))?;
    if std::fs::symlink_metadata(candidate)
        .map_err(|e| format!("Cannot inspect skill directory: {e}"))?
        .file_type()
        .is_symlink()
    {
        return Err("A symbolic link cannot be deleted as a managed skill".to_string());
    }
    let candidate = candidate
        .canonicalize()
        .map_err(|e| format!("Cannot resolve skill directory: {e}"))?;
    if candidate == root || !candidate.starts_with(&root) {
        return Err(
            "Skill directory must be strictly inside its managed skills directory".to_string(),
        );
    }
    if !candidate.is_dir() || !candidate.join("SKILL.md").is_file() {
        return Err("Skill directory must contain a SKILL.md file".to_string());
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "localrouter-skill-paths-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn make_skill(path: &Path) {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(path.join("SKILL.md"), "---\nname: example\n---\n").unwrap();
    }

    #[test]
    fn accepts_user_and_marketplace_skills() {
        let temp = TestDirectory::new();
        for relative in ["user-skill", "source/marketplace-skill"] {
            let target = temp.0.join(relative);
            make_skill(&target);
            assert_eq!(
                managed_skill_directory(&temp.0, &target).unwrap(),
                target.canonicalize().unwrap()
            );
        }
    }

    #[test]
    fn rejects_root_parent_escape_and_non_skill_directories() {
        let temp = TestDirectory::new();
        let root = temp.0.join("skills");
        make_skill(&root);
        make_skill(&temp.0.join("outside"));
        std::fs::create_dir(root.join("source")).unwrap();
        assert!(managed_skill_directory(&root, &root).is_err());
        assert!(managed_skill_directory(&root, &root.join("../outside")).is_err());
        assert!(managed_skill_directory(&root, &root.join("source")).is_err());
        assert!(managed_skill_directory(&root, &root.join("missing")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_targets_and_escaping_parent_symlinks() {
        use std::os::unix::fs::symlink;
        let temp = TestDirectory::new();
        let root = temp.0.join("skills");
        make_skill(&root.join("real"));
        make_skill(&temp.0.join("outside/skill"));
        symlink(root.join("real"), root.join("alias")).unwrap();
        symlink(temp.0.join("outside"), root.join("escape")).unwrap();
        assert!(managed_skill_directory(&root, &root.join("alias")).is_err());
        assert!(managed_skill_directory(&root, &root.join("escape/skill")).is_err());
    }
}
