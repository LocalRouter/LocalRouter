//! Skill discovery - scanning directories and parsing SKILL.md files

use super::types::{SkillDefinition, SkillMetadata};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Result of discovery from a single path
pub struct DiscoveryResult {
    /// Discovered skills
    pub skills: Vec<SkillDefinition>,
    /// Old extraction directories that can be cleaned up (from previous zip extractions)
    pub old_extraction_dirs: Vec<PathBuf>,
}

/// Compute SHA-256 hash of a file, returning first 16 bytes as 32-char hex string
pub fn content_hash_of_file(path: &Path) -> Result<String, String> {
    let data = std::fs::read(path).map_err(|e| format!("Failed to read file for hashing: {}", e))?;
    let hash = Sha256::digest(&data);
    let hex: String = hash.iter().take(16).map(|b| format!("{:02x}", b)).collect();
    Ok(hex)
}

/// Parse a SKILL.md file into metadata and body
///
/// Expected format:
/// ```
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

/// Try to load a skill from a directory containing SKILL.md
fn load_skill_from_dir(skill_dir: &Path, source_path: &str) -> Option<SkillDefinition> {
    let skill_md_path = skill_dir.join("SKILL.md");
    if !skill_md_path.is_file() {
        return None;
    }

    let content = match std::fs::read_to_string(&skill_md_path) {
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

    let scripts = list_subdir_files(skill_dir, "scripts");
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
/// - .zip or .skill file -> extract to temp, then scan
pub fn discover_skills(path: &Path) -> DiscoveryResult {
    let source = path.display().to_string();

    if !path.exists() {
        warn!("Skill path does not exist: {}", source);
        return DiscoveryResult {
            skills: Vec::new(),
            old_extraction_dirs: Vec::new(),
        };
    }

    if path.is_file() {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if ext == "zip" || ext == "skill" {
            return discover_from_zip(path, &source);
        }

        warn!("Unsupported skill file type: {}", source);
        return DiscoveryResult {
            skills: Vec::new(),
            old_extraction_dirs: Vec::new(),
        };
    }

    if path.is_dir() {
        return DiscoveryResult {
            skills: discover_from_directory(path, &source),
            old_extraction_dirs: Vec::new(),
        };
    }

    DiscoveryResult {
        skills: Vec::new(),
        old_extraction_dirs: Vec::new(),
    }
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

/// Discover skills from a zip/skill file
///
/// Uses hash-based extraction directories: `/tmp/localrouter-skills/{stem}-{hash}/`
/// If the hash dir already exists, skips extraction (reuse).
/// Returns old extraction dirs (same stem, different hash) for cleanup.
fn discover_from_zip(zip_path: &Path, source: &str) -> DiscoveryResult {
    // Compute content hash
    let hash = match content_hash_of_file(zip_path) {
        Ok(h) => h,
        Err(e) => {
            warn!("Failed to hash zip file {:?}: {}", zip_path, e);
            return DiscoveryResult {
                skills: Vec::new(),
                old_extraction_dirs: Vec::new(),
            };
        }
    };

    let temp_dir = std::env::temp_dir().join("localrouter-skills");
    let file_stem = zip_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("skill");
    let extract_dir = temp_dir.join(format!("{}-{}", file_stem, hash));

    // Collect old extraction dirs (same stem, different hash) for cleanup
    let mut old_dirs = Vec::new();
    let prefix = format!("{}-", file_stem);
    if let Ok(entries) = std::fs::read_dir(&temp_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(&prefix) && entry.path() != extract_dir && entry.path().is_dir()
            {
                old_dirs.push(entry.path());
            }
        }
    }

    // If hash dir already exists, reuse it
    if extract_dir.is_dir() {
        debug!(
            "Reusing existing extraction for {} (hash: {})",
            source, hash
        );
        let mut skills = discover_from_directory(&extract_dir, source);
        for skill in &mut skills {
            skill.content_hash = Some(hash.clone());
        }
        return DiscoveryResult {
            skills,
            old_extraction_dirs: old_dirs,
        };
    }

    // Need to extract
    let file = match std::fs::File::open(zip_path) {
        Ok(f) => f,
        Err(e) => {
            warn!("Failed to open zip file {:?}: {}", zip_path, e);
            return DiscoveryResult {
                skills: Vec::new(),
                old_extraction_dirs: old_dirs,
            };
        }
    };

    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(e) => {
            warn!("Failed to read zip archive {:?}: {}", zip_path, e);
            return DiscoveryResult {
                skills: Vec::new(),
                old_extraction_dirs: old_dirs,
            };
        }
    };

    if let Err(e) = std::fs::create_dir_all(&extract_dir) {
        warn!("Failed to create extraction directory: {}", e);
        return DiscoveryResult {
            skills: Vec::new(),
            old_extraction_dirs: old_dirs,
        };
    }

    // Extract all files
    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to read zip entry {}: {}", i, e);
                continue;
            }
        };

        let entry_path = match entry.enclosed_name() {
            Some(p) => extract_dir.join(p),
            None => {
                warn!("Skipping zip entry with unsafe path");
                continue;
            }
        };

        if entry.is_dir() {
            let _ = std::fs::create_dir_all(&entry_path);
        } else {
            if let Some(parent) = entry_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(mut outfile) = std::fs::File::create(&entry_path) {
                if let Err(e) = std::io::copy(&mut entry, &mut outfile) {
                    warn!("Failed to extract {:?}: {}", entry_path, e);
                }
            }
        }
    }

    let mut skills = discover_from_directory(&extract_dir, source);
    for skill in &mut skills {
        skill.content_hash = Some(hash.clone());
    }
    DiscoveryResult {
        skills,
        old_extraction_dirs: old_dirs,
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
        fs::write(
            skill_a.join("SKILL.md"),
            "---\nname: skill-a\n---\nBody A",
        )
        .unwrap();

        // Skill B
        let skill_b = parent.join("skill-b");
        fs::create_dir(&skill_b).unwrap();
        fs::write(
            skill_b.join("SKILL.md"),
            "---\nname: skill-b\n---\nBody B",
        )
        .unwrap();

        // Not a skill
        let not_skill = parent.join("not-a-skill");
        fs::create_dir(&not_skill).unwrap();
        fs::write(not_skill.join("README.md"), "not a skill").unwrap();

        let result = discover_skills(parent);
        assert_eq!(result.skills.len(), 2);

        let names: Vec<&str> = result.skills.iter().map(|s| s.metadata.name.as_str()).collect();
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

        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: test\n---\nBody",
        )
        .unwrap();

        // Create references dir with a normal file
        fs::create_dir(skill_dir.join("references")).unwrap();
        fs::write(skill_dir.join("references/doc.md"), "doc").unwrap();

        let result = discover_skills(skill_dir);
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].references, vec!["references/doc.md"]);
    }
}
