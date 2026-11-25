//! Skill discovery - scanning directories and parsing SKILL.md files

use super::archive::{self, ExtractionLease, ExtractionLimits, FileStamp};
use super::types::{SkillDefinition, SkillMetadata};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tracing::{debug, info, warn};

/// Result of discovery from a single path
#[derive(Default)]
pub struct DiscoveryResult {
    /// Discovered skills
    pub skills: Vec<SkillDefinition>,
    /// Old extraction directories that can be cleaned up (from previous zip extractions)
    pub old_extraction_dirs: Vec<PathBuf>,
    /// Why the source could not be (fully) loaded, in words fit for the user
    pub error: Option<String>,
    /// Leases on the archive extractions the skills are served from; hold
    /// them for as long as the skills are in use.
    pub leases: Vec<ExtractionLease>,
}

/// Compute SHA-256 hash of a file, returning first 16 bytes as 32-char hex string
pub fn content_hash_of_file(path: &Path) -> Result<String, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("Failed to read file for hashing: {}", e))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)
        .map_err(|e| format!("Failed to read file for hashing: {}", e))?;
    let hash = hasher.finalize();
    let hex: String = hash.iter().take(16).map(|b| format!("{:02x}", b)).collect();
    Ok(hex)
}

/// Parse a SKILL.md file into metadata and body
///
/// Expected format:
/// ```text
/// ---
/// name: my-skill
/// description: A useful skill
/// ---
/// # Instructions
/// ...markdown body...
/// ```
pub fn parse_skill_md(content: &str) -> Result<(SkillMetadata, String), String> {
    let trimmed = content.trim();

    // Check for frontmatter delimiters
    if !trimmed.starts_with("---") {
        return Err("SKILL.md must start with '---' frontmatter delimiter".to_string());
    }

    // Find the closing delimiter
    let after_first = &trimmed[3..];
    let end_pos = after_first
        .find("\n---")
        .ok_or_else(|| "Missing closing '---' frontmatter delimiter".to_string())?;

    let frontmatter = &after_first[..end_pos].trim();
    let body_start = 3 + end_pos + 4; // skip "---\n---"
    let body = if body_start < trimmed.len() {
        trimmed[body_start..].trim().to_string()
    } else {
        String::new()
    };

    let metadata: SkillMetadata = serde_yaml::from_str(frontmatter)
        .map_err(|e| format!("Failed to parse SKILL.md frontmatter: {}", e))?;

    if metadata.name.is_empty() {
        return Err("SKILL.md frontmatter must include a non-empty 'name' field".to_string());
    }

    Ok((metadata, body))
}

/// List relative file paths in a subdirectory of the skill dir
fn list_subdir_files(skill_dir: &Path, subdir: &str) -> Vec<String> {
    let dir = skill_dir.join(subdir);
    if !dir.is_dir() {
        return Vec::new();
    }

    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    files.push(format!("{}/{}", subdir, name));
                }
            }
        }
    }
    files.sort();
    files
}

/// List non-hidden, non-SKILL.md files directly in the skill root directory.
///
/// Returns bare filenames (e.g., `"sysinfo.sh"`) without any directory prefix.
fn list_root_files(skill_dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(skill_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.eq_ignore_ascii_case("SKILL.md") || name.starts_with('.') {
                    continue;
                }
                files.push(name.to_string());
            }
        }
    }
    files.sort();
    files
}

/// Try to load a skill from a directory containing SKILL.md
fn load_skill_from_dir(skill_dir: &Path, source_path: &str) -> Option<SkillDefinition> {
    // A listed skill directory that is gone may be mid-update: where the OS
    // has no atomic directory exchange, a marketplace update replaces it in
    // two renames. Wait (bounded) for it to come back. A directory that
    // exists without SKILL.md is not a skill and is not waited for.
    if lr_utils::fs::retry_while_missing(|| std::fs::symlink_metadata(skill_dir)).is_err() {
        return None;
    }
    let skill_md_path = skill_dir.join("SKILL.md");
    if !skill_md_path.is_file() {
        return None;
    }

    let content =
        match lr_utils::fs::retry_while_missing(|| std::fs::read_to_string(&skill_md_path)) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read {:?}: {}", skill_md_path, e);
                return None;
            }
        };

    let (metadata, body) = match parse_skill_md(&content) {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to parse {:?}: {}", skill_md_path, e);
            return None;
        }
    };

    let mut scripts = list_subdir_files(skill_dir, "scripts");
    scripts.extend(list_root_files(skill_dir));
    scripts.sort();
    let references = list_subdir_files(skill_dir, "references");
    let assets = list_subdir_files(skill_dir, "assets");

    Some(SkillDefinition {
        metadata,
        body,
        skill_dir: skill_dir.to_path_buf(),
        source_path: source_path.to_string(),
        scripts,
        references,
        assets,
        enabled: true,
        content_hash: None,
    })
}

/// Discover skills from a path
///
/// Handles:
/// - Directory with SKILL.md -> single skill
/// - Directory containing subdirs with SKILL.md -> multiple skills
/// - .zip or .skill file -> extract into [`extraction_root`], then scan
pub fn discover_skills(path: &Path) -> DiscoveryResult {
    discover_skills_in(path, &extraction_root())
}

/// [`discover_skills`] extracting archives under `root`.
pub fn discover_skills_in(path: &Path, root: &Path) -> DiscoveryResult {
    let source = path.display().to_string();

    if !path.exists() {
        warn!("Skill path does not exist: {}", source);
        return DiscoveryResult::default();
    }

    if path.is_file() {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if ext == "zip" || ext == "skill" {
            return discover_from_zip_in(path, &source, root, &ExtractionLimits::default());
        }

        warn!("Unsupported skill file type: {}", source);
        return DiscoveryResult::default();
    }

    if path.is_dir() {
        return DiscoveryResult {
            skills: discover_from_directory(path, &source),
            ..Default::default()
        };
    }

    DiscoveryResult::default()
}

/// Discover skills from a directory
fn discover_from_directory(dir: &Path, source: &str) -> Vec<SkillDefinition> {
    // First check if this directory itself is a skill
    if dir.join("SKILL.md").is_file() {
        debug!("Found skill directory: {}", source);
        if let Some(skill) = load_skill_from_dir(dir, source) {
            return vec![skill];
        }
        return Vec::new();
    }

    // Otherwise scan subdirectories for skills
    let mut skills = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                if let Some(skill) = load_skill_from_dir(&entry_path, source) {
                    debug!("Found skill '{}' in {}", skill.metadata.name, source);
                    skills.push(skill);
                }
            }
        }
    }

    if skills.is_empty() {
        debug!("No skills found in directory: {}", source);
    } else {
        info!("Discovered {} skills from {}", skills.len(), source);
    }

    skills
}

/// Folder name, inside the data directory, of the extraction root.
const EXTRACTION_DIR: &str = "skill-archives";

/// Where archives are extracted: `<data dir>/skill-archives/`
/// (`~/.localrouter/skill-archives`, `~/.localrouter-dev/...` in debug
/// builds). Unit tests of this crate use a folder of their own under the
/// temp directory instead, so they never touch a real data directory.
pub fn extraction_root() -> PathBuf {
    if cfg!(test) {
        return std::env::temp_dir().join(format!("lr-skills-test-{}", std::process::id()));
    }
    lr_utils::paths::config_dir()
        .map(|dir| dir.join(EXTRACTION_DIR))
        .unwrap_or_else(|_| std::env::temp_dir().join("localrouter-skill-archives"))
}

/// Where builds before the data-directory move extracted archives:
/// `<temp>/localrouter-skills/`. Only cleaned up, never extracted into.
pub fn legacy_extraction_root() -> PathBuf {
    std::env::temp_dir().join("localrouter-skills")
}

/// Whether `name` is `<stem>-<hash>`, the directory name of one extraction of
/// the archive called `stem`. The hash must match exactly so that `foo.zip`
/// never claims the extraction of `foo-bar.zip`.
fn is_extraction_of(name: &str, stem: &str) -> bool {
    name.strip_prefix(stem)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|hash| hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Archive hashes this process computed, by archive path, with the file's
/// stamp at the time: an unchanged archive is not read again on every scan.
static ARCHIVE_HASHES: LazyLock<parking_lot::Mutex<HashMap<PathBuf, (FileStamp, String)>>> =
    LazyLock::new(Default::default);

/// The content hash of the archive, without reading it when possible: from
/// this process's cache, or from an extraction whose marker records the
/// archive with the same size and modification time (the extraction's name
/// carries the hash). Otherwise the archive is hashed.
fn archive_hash(
    zip_path: &Path,
    identity: &Path,
    candidates: &[PathBuf],
) -> Result<String, String> {
    let stamp = FileStamp::of(zip_path);
    if let Some(stamp) = stamp {
        if let Some((cached, hash)) = ARCHIVE_HASHES.lock().get(identity) {
            if *cached == stamp {
                return Ok(hash.clone());
            }
        }
        let recorded = candidates.iter().find(|dir| {
            archive::extraction_archive_stamp(dir) == Some(stamp)
                && archive::extraction_source(dir).as_deref() == Some(identity)
        });
        if let Some(hash) = recorded
            .and_then(|dir| dir.file_name()?.to_str()?.rsplit_once('-'))
            .map(|(_, hash)| hash.to_string())
        {
            ARCHIVE_HASHES
                .lock()
                .insert(identity.to_path_buf(), (stamp, hash.clone()));
            return Ok(hash);
        }
    }
    let hash = content_hash_of_file(zip_path)?;
    // Only cached when the file did not change while it was read.
    if let Some(stamp) = stamp.filter(|s| FileStamp::of(zip_path) == Some(*s)) {
        ARCHIVE_HASHES
            .lock()
            .insert(identity.to_path_buf(), (stamp, hash.clone()));
    }
    Ok(hash)
}

/// Discover skills from a zip/skill file.
///
/// Uses content-addressed extraction directories `<root>/{stem}-{hash}/`.
/// The archive is extracted into a staging directory and renamed to
/// `{stem}-{hash}` only when complete, so an existing hash directory is always
/// a whole extraction, which is reused. When the archive cannot be extracted,
/// the most recent earlier extraction of the same archive keeps being served
/// and the reason is reported in [`DiscoveryResult::error`]. Returns earlier
/// extractions of the same archive (other content) for cleanup, and a lease
/// on the extraction in use.
pub(crate) fn discover_from_zip_in(
    zip_path: &Path,
    source: &str,
    root: &Path,
    limits: &ExtractionLimits,
) -> DiscoveryResult {
    let file_stem = zip_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("skill");

    // Extractions of this archive name. One whose marker names another
    // archive (a same-named file in another folder) is that archive's and is
    // left alone; an unmarked one predates markers and is claimed.
    let identity = archive::archive_identity(zip_path);
    let mut same_archive = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let path = entry.path();
            if is_extraction_of(&name.to_string_lossy(), file_stem)
                && path.is_dir()
                && archive::extraction_source(&path).is_none_or(|from| from == identity)
            {
                same_archive.push(path);
            }
        }
    }

    let hash = match archive_hash(zip_path, &identity, &same_archive) {
        Ok(h) => h,
        Err(e) => {
            warn!("Failed to hash zip file {:?}: {}", zip_path, e);
            return DiscoveryResult {
                error: Some(e),
                ..Default::default()
            };
        }
    };
    let extract_dir = root.join(format!("{}-{}", file_stem, hash));
    let old_dirs: Vec<PathBuf> = same_archive
        .into_iter()
        .filter(|path| *path != extract_dir)
        .collect();

    // Returns at once when a whole extraction of this content already exists.
    let lease = match archive::extract_archive_atomically(zip_path, &extract_dir, limits) {
        Ok(lease) => lease,
        Err(e) => {
            warn!("Failed to extract skill archive {:?}: {}", zip_path, e);
            return fall_back_to_previous_extraction(
                root,
                source,
                &identity,
                old_dirs,
                e.to_string(),
            );
        }
    };
    debug!(
        "Using extraction {:?} for {} (hash: {})",
        extract_dir, source, hash
    );

    let mut skills = discover_from_directory(&extract_dir, source);
    for skill in &mut skills {
        skill.content_hash = Some(hash.clone());
    }
    DiscoveryResult {
        skills,
        old_extraction_dirs: old_dirs,
        error: None,
        leases: vec![lease],
    }
}

/// The archive could not be extracted: keep the newest earlier extraction of
/// this same archive in service (and off the cleanup list) rather than
/// dropping its skills.
fn fall_back_to_previous_extraction(
    root: &Path,
    source: &str,
    identity: &Path,
    mut old_dirs: Vec<PathBuf>,
    reason: String,
) -> DiscoveryResult {
    let modified = |dir: &PathBuf| std::fs::metadata(dir).and_then(|m| m.modified()).ok();
    let newest = (0..old_dirs.len())
        .filter(|&i| archive::extraction_source(&old_dirs[i]).as_deref() == Some(identity))
        .max_by_key(|&i| modified(&old_dirs[i]));
    let lease = newest.and_then(|i| archive::lease_existing(root, &old_dirs[i]));
    let (Some(newest), Some(lease)) = (newest, lease) else {
        return DiscoveryResult {
            error: Some(reason),
            ..Default::default()
        };
    };
    let previous = old_dirs.swap_remove(newest);
    DiscoveryResult {
        skills: discover_from_directory(&previous, source),
        old_extraction_dirs: old_dirs,
        error: Some(format!(
            "{reason}. The previously extracted version is still in use."
        )),
        leases: vec![lease],
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_skill_md_valid() {
        let content = r#"---
name: test-skill
description: A test skill
version: "1.0"
tags:
  - testing
  - example
---
# Instructions

This is the skill body."#;

        let (metadata, body) = parse_skill_md(content).unwrap();
        assert_eq!(metadata.name, "test-skill");
        assert_eq!(metadata.description.as_deref(), Some("A test skill"));
        assert_eq!(metadata.version.as_deref(), Some("1.0"));
        assert_eq!(metadata.tags, vec!["testing", "example"]);
        assert!(body.contains("# Instructions"));
        assert!(body.contains("This is the skill body."));
    }

    #[test]
    fn test_parse_skill_md_no_frontmatter() {
        let content = "# Just a regular markdown file";
        assert!(parse_skill_md(content).is_err());
    }

    #[test]
    fn test_parse_skill_md_missing_name() {
        let content = r#"---
description: No name field
---
Body text"#;

        // serde_yaml should fail because name is required
        assert!(parse_skill_md(content).is_err());
    }

    #[test]
    fn test_parse_skill_md_empty_name() {
        let content = r#"---
name: ""
---
Body text"#;

        assert!(parse_skill_md(content).is_err());
    }

    #[test]
    fn test_discover_single_skill_dir() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();

        fs::write(
            skill_dir.join("SKILL.md"),
            r#"---
name: my-skill
description: Test
---
Body"#,
        )
        .unwrap();

        fs::create_dir(skill_dir.join("scripts")).unwrap();
        fs::write(skill_dir.join("scripts/run.sh"), "#!/bin/bash\necho hi").unwrap();

        let result = discover_skills(skill_dir);
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].metadata.name, "my-skill");
        assert_eq!(result.skills[0].scripts, vec!["scripts/run.sh"]);
        assert!(result.skills[0].enabled);
    }

    #[test]
    fn test_discover_multi_skill_dir() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();

        // Skill A
        let skill_a = parent.join("skill-a");
        fs::create_dir(&skill_a).unwrap();
        fs::write(skill_a.join("SKILL.md"), "---\nname: skill-a\n---\nBody A").unwrap();

        // Skill B
        let skill_b = parent.join("skill-b");
        fs::create_dir(&skill_b).unwrap();
        fs::write(skill_b.join("SKILL.md"), "---\nname: skill-b\n---\nBody B").unwrap();

        // Not a skill
        let not_skill = parent.join("not-a-skill");
        fs::create_dir(&not_skill).unwrap();
        fs::write(not_skill.join("README.md"), "not a skill").unwrap();

        let result = discover_skills(parent);
        assert_eq!(result.skills.len(), 2);

        let names: Vec<&str> = result
            .skills
            .iter()
            .map(|s| s.metadata.name.as_str())
            .collect();
        assert!(names.contains(&"skill-a"));
        assert!(names.contains(&"skill-b"));
    }

    #[test]
    fn test_discover_nonexistent_path() {
        let result = discover_skills(Path::new("/nonexistent/path/to/skills"));
        assert!(result.skills.is_empty());
    }

    #[test]
    fn test_path_traversal_in_resource_listing() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();

        fs::write(skill_dir.join("SKILL.md"), "---\nname: test\n---\nBody").unwrap();

        // Create references dir with a normal file
        fs::create_dir(skill_dir.join("references")).unwrap();
        fs::write(skill_dir.join("references/doc.md"), "doc").unwrap();

        let result = discover_skills(skill_dir);
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].references, vec!["references/doc.md"]);
    }

    #[test]
    fn test_discover_root_level_scripts() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();

        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: sysinfo\n---\nRun ./sysinfo.sh",
        )
        .unwrap();
        fs::write(skill_dir.join("sysinfo.sh"), "#!/bin/bash\nuname -a").unwrap();

        let result = discover_skills(skill_dir);
        assert_eq!(result.skills.len(), 1);
        assert!(result.skills[0].scripts.contains(&"sysinfo.sh".to_string()));
    }

    #[test]
    fn test_discover_root_and_subdir_scripts() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();

        fs::write(skill_dir.join("SKILL.md"), "---\nname: mixed\n---\nBody").unwrap();
        fs::write(skill_dir.join("run.sh"), "#!/bin/bash").unwrap();
        fs::create_dir(skill_dir.join("scripts")).unwrap();
        fs::write(skill_dir.join("scripts/build.sh"), "#!/bin/bash").unwrap();

        let result = discover_skills(skill_dir);
        assert_eq!(result.skills.len(), 1);
        let scripts = &result.skills[0].scripts;
        assert!(scripts.contains(&"run.sh".to_string()));
        assert!(scripts.contains(&"scripts/build.sh".to_string()));
        assert_eq!(scripts.len(), 2);
    }

    #[test]
    fn test_root_files_exclude_skillmd_and_hidden() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();

        fs::write(skill_dir.join("SKILL.md"), "---\nname: test\n---\nBody").unwrap();
        fs::write(skill_dir.join(".gitignore"), "*.log").unwrap();
        fs::write(skill_dir.join(".DS_Store"), "").unwrap();
        fs::write(skill_dir.join("run.sh"), "#!/bin/bash").unwrap();

        let result = discover_skills(skill_dir);
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].scripts, vec!["run.sh".to_string()]);
    }

    #[test]
    fn test_root_files_skip_directories() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();

        fs::write(skill_dir.join("SKILL.md"), "---\nname: test\n---\nBody").unwrap();
        fs::create_dir(skill_dir.join("some-dir")).unwrap();
        fs::write(skill_dir.join("run.sh"), "#!/bin/bash").unwrap();

        let result = discover_skills(skill_dir);
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].scripts, vec!["run.sh".to_string()]);
    }

    use crate::archive::test_support::{write_zip, Entry};

    const SKILL_V1: &[u8] = b"---\nname: zipped\ndescription: one\n---\nBody one";
    const SKILL_V2: &[u8] = b"---\nname: zipped\ndescription: two\n---\nBody two";

    /// Names under `dir`, without the root lock file.
    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .map(|entries| {
                entries
                    .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
                    .filter(|name| name != ".lock")
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn zip_skill_is_extracted_and_reused() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let zip = write_zip(
            tmp.path(),
            "pack.zip",
            &[
                Entry::File("zipped/SKILL.md", SKILL_V1),
                Entry::File("zipped/scripts/run.sh", b"echo hi"),
            ],
        );
        let limits = ExtractionLimits::default();
        let first = discover_from_zip_in(&zip, "src", &root, &limits);
        assert!(first.error.is_none());
        assert_eq!(first.skills.len(), 1);
        assert_eq!(first.skills[0].scripts, vec!["scripts/run.sh"]);
        assert!(first.skills[0].content_hash.is_some());
        assert!(first.old_extraction_dirs.is_empty());
        assert_eq!(names_in(&root).len(), 1, "only the finished extraction");

        let again = discover_from_zip_in(&zip, "src", &root, &limits);
        assert_eq!(again.skills[0].skill_dir, first.skills[0].skill_dir);
    }

    #[test]
    fn failed_zip_extraction_installs_nothing_and_names_the_limit() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        // The manifest is written before the oversized file is reached.
        let zip = write_zip(
            tmp.path(),
            "pack.zip",
            &[
                Entry::File("SKILL.md", SKILL_V1),
                Entry::File("assets/big.bin", &[0u8; 8192]),
            ],
        );
        let limits = ExtractionLimits {
            max_total_bytes: 4096,
            ..Default::default()
        };
        let result = discover_from_zip_in(&zip, "src", &root, &limits);
        assert!(result.skills.is_empty());
        assert_eq!(
            result.error.as_deref(),
            Some("the archive expands to more than the 4 KiB limit")
        );
        assert!(names_in(&root).is_empty(), "no partial skill, no staging");

        // The failure is not cached: the same archive loads under a larger budget.
        let result = discover_from_zip_in(&zip, "src", &root, &ExtractionLimits::default());
        assert_eq!(result.skills.len(), 1);
    }

    #[test]
    fn failed_zip_update_keeps_the_previous_version() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let limits = ExtractionLimits::default();
        let zip = write_zip(tmp.path(), "pack.zip", &[Entry::File("SKILL.md", SKILL_V1)]);
        let v1 = discover_from_zip_in(&zip, "src", &root, &limits);
        let v1_dir = v1.skills[0].skill_dir.clone();

        // The update holds a valid manifest followed by a link.
        write_zip(
            tmp.path(),
            "pack.zip",
            &[
                Entry::File("SKILL.md", SKILL_V2),
                Entry::Link("references", "/etc"),
            ],
        );
        let failed = discover_from_zip_in(&zip, "src", &root, &limits);
        let error = failed.error.expect("the failure is reported");
        assert!(error.contains("links are not allowed"), "{error}");
        assert!(error.contains("previously extracted version"), "{error}");
        assert_eq!(failed.skills.len(), 1);
        assert_eq!(failed.skills[0].skill_dir, v1_dir);
        assert_eq!(failed.skills[0].body, "Body one");
        assert!(
            !failed.old_extraction_dirs.contains(&v1_dir),
            "the version in use must not be queued for deletion"
        );
        assert_eq!(names_in(&root).len(), 1);
        assert_eq!(names_in(&v1_dir), [".localrouter-extraction", "SKILL.md"]);
    }

    #[test]
    fn successful_zip_update_is_a_complete_new_tree() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let limits = ExtractionLimits::default();
        let zip = write_zip(
            tmp.path(),
            "pack.zip",
            &[
                Entry::File("SKILL.md", SKILL_V1),
                Entry::File("scripts/old.sh", b"old"),
            ],
        );
        let v1 = discover_from_zip_in(&zip, "src", &root, &limits);
        let v1_dir = v1.skills[0].skill_dir.clone();

        write_zip(
            tmp.path(),
            "pack.zip",
            &[
                Entry::File("SKILL.md", SKILL_V2),
                Entry::File("scripts/new.sh", b"new"),
            ],
        );
        // Both versions have the same length, and two writes can share one
        // filesystem timestamp tick: give the rewrite a distinct mtime so
        // the change is seen the way a real edit would be.
        let written = std::fs::metadata(&zip).unwrap().modified().unwrap();
        std::fs::File::options()
            .write(true)
            .open(&zip)
            .unwrap()
            .set_modified(written + std::time::Duration::from_secs(2))
            .unwrap();
        let v2 = discover_from_zip_in(&zip, "src", &root, &limits);
        assert!(v2.error.is_none());
        assert_eq!(v2.skills[0].body, "Body two");
        // No file of the old version leaks into the new one.
        assert_eq!(v2.skills[0].scripts, vec!["scripts/new.sh"]);
        assert_ne!(v2.skills[0].skill_dir, v1_dir);
        assert_eq!(v2.old_extraction_dirs, vec![v1_dir.clone()]);
        // The old version stays intact until the manager's delayed cleanup.
        assert!(v1_dir.join("scripts/old.sh").is_file());
    }

    #[test]
    fn cleanup_never_claims_another_archives_extraction() {
        let hash = "0123456789abcdef0123456789abcdef";
        assert!(is_extraction_of(&format!("foo-{hash}"), "foo"));
        assert!(!is_extraction_of(&format!("foo-bar-{hash}"), "foo"));
        assert!(!is_extraction_of(&format!("foo-{}", &hash[1..]), "foo"));
        assert!(!is_extraction_of(".staging-foo", "foo"));
        assert!(!is_extraction_of(&format!("foo{hash}"), "foo"));

        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let limits = ExtractionLimits::default();
        let longer = write_zip(
            tmp.path(),
            "foo-bar.zip",
            &[Entry::File("SKILL.md", SKILL_V1)],
        );
        discover_from_zip_in(&longer, "a", &root, &limits);
        let shorter = write_zip(tmp.path(), "foo.zip", &[Entry::File("SKILL.md", SKILL_V2)]);
        let result = discover_from_zip_in(&shorter, "b", &root, &limits);
        assert!(result.old_extraction_dirs.is_empty());
    }

    #[test]
    fn same_named_archives_in_different_folders_keep_their_extractions() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let limits = ExtractionLimits::default();
        for folder in ["a", "b"] {
            fs::create_dir(tmp.path().join(folder)).unwrap();
        }
        let a = write_zip(
            &tmp.path().join("a"),
            "pack.zip",
            &[Entry::File("SKILL.md", SKILL_V1)],
        );
        let b = write_zip(
            &tmp.path().join("b"),
            "pack.zip",
            &[Entry::File("SKILL.md", SKILL_V2)],
        );
        let a_dir = discover_from_zip_in(&a, "a", &root, &limits).skills[0]
            .skill_dir
            .clone();
        let from_b = discover_from_zip_in(&b, "b", &root, &limits);
        assert!(
            from_b.old_extraction_dirs.is_empty(),
            "claimed a's extraction"
        );

        // A broken update of b never falls back to a's skills.
        fs::write(&b, b"not a zip").unwrap();
        let broken = discover_from_zip_in(&b, "b", &root, &limits);
        assert_eq!(broken.skills.len(), 1);
        assert_eq!(broken.skills[0].body, "Body two");
        assert_ne!(broken.skills[0].skill_dir, a_dir);
        assert!(!broken.old_extraction_dirs.contains(&a_dir));
    }

    #[test]
    fn unreadable_zip_reports_an_error() {
        let tmp = TempDir::new().unwrap();
        let zip = tmp.path().join("broken.skill");
        fs::write(&zip, b"not a zip").unwrap();
        let result = discover_from_zip_in(
            &zip,
            "src",
            &tmp.path().join("root"),
            &ExtractionLimits::default(),
        );
        assert!(result.skills.is_empty());
        assert!(result
            .error
            .unwrap()
            .starts_with("not a readable zip archive"));
        assert!(names_in(&tmp.path().join("root")).is_empty());
    }

    #[test]
    fn the_served_extraction_is_leased() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let zip = write_zip(tmp.path(), "pack.zip", &[Entry::File("SKILL.md", SKILL_V1)]);
        let result = discover_from_zip_in(&zip, "src", &root, &ExtractionLimits::default());
        let dir = result.skills[0].skill_dir.clone();
        assert_eq!(result.leases.len(), 1);
        assert_eq!(result.leases[0].dir(), dir);
        assert!(!archive::remove_extraction(&root, &dir).unwrap());
        drop(result);
        // The lease is an flock on the marker's open file description. A
        // child process another test spawns at that moment holds a copy of
        // the descriptor until it execs, so the release can lag briefly;
        // removal then says "in use, try again later", which the cleanup
        // pass retries. Give it the same chance here.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !archive::remove_extraction(&root, &dir).unwrap() {
            assert!(
                std::time::Instant::now() < deadline,
                "the extraction stayed leased after its lease was dropped"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn a_failed_update_leases_the_version_it_falls_back_to() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let limits = ExtractionLimits::default();
        let zip = write_zip(tmp.path(), "pack.zip", &[Entry::File("SKILL.md", SKILL_V1)]);
        let v1_dir = discover_from_zip_in(&zip, "src", &root, &limits).skills[0]
            .skill_dir
            .clone();
        fs::write(&zip, b"not a zip").unwrap();
        let failed = discover_from_zip_in(&zip, "src", &root, &limits);
        assert_eq!(failed.leases.len(), 1);
        assert_eq!(failed.leases[0].dir(), v1_dir);
        assert!(!archive::remove_extraction(&root, &v1_dir).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn an_unchanged_archive_is_found_without_reading_it() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let limits = ExtractionLimits::default();
        let zip = write_zip(tmp.path(), "pack.zip", &[Entry::File("SKILL.md", SKILL_V1)]);
        let first = discover_from_zip_in(&zip, "src", &root, &limits);
        let dir = first.skills[0].skill_dir.clone();
        let hash = first.skills[0].content_hash.clone();
        drop(first);

        // A new process: nothing cached. The archive cannot even be read,
        // yet the marker's record of its size and time finds the extraction.
        ARCHIVE_HASHES.lock().clear();
        fs::set_permissions(&zip, fs::Permissions::from_mode(0o000)).unwrap();
        let again = discover_from_zip_in(&zip, "src", &root, &limits);
        fs::set_permissions(&zip, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(again.error.is_none(), "{:?}", again.error);
        assert_eq!(again.skills[0].skill_dir, dir);
        assert_eq!(again.skills[0].content_hash, hash);
    }

    #[test]
    fn a_changed_archive_is_hashed_again() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let limits = ExtractionLimits::default();
        let zip = write_zip(tmp.path(), "pack.zip", &[Entry::File("SKILL.md", SKILL_V1)]);
        let v1 = discover_from_zip_in(&zip, "src", &root, &limits);
        // Same length, different bytes, and a different modification time.
        let mut bytes = fs::read(&zip).unwrap();
        let v2_bytes =
            crate::archive::test_support::zip_bytes(&[Entry::File("SKILL.md", SKILL_V2)]);
        assert_eq!(bytes.len(), v2_bytes.len());
        bytes.copy_from_slice(&v2_bytes);
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&zip, &bytes).unwrap();
        let v2 = discover_from_zip_in(&zip, "src", &root, &limits);
        assert_ne!(v2.skills[0].skill_dir, v1.skills[0].skill_dir);
        assert_eq!(v2.skills[0].body, "Body two");
    }
}
