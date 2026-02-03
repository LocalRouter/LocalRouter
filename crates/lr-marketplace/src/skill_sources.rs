//! Skill sources client
//!
//! Browses GitHub repos via Contents API to discover skills.
//! Downloads skill files via raw.githubusercontent.com URLs.

use crate::types::{tokenize_name, MarketplaceCache, MarketplaceError, SkillFileInfo, SkillListing};
use lr_config::MarketplaceSkillSource;
use parking_lot::RwLock;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Skill sources client
pub struct SkillSourcesClient {
    http_client: reqwest::Client,
    sources: Arc<RwLock<Vec<MarketplaceSkillSource>>>,
    /// In-memory cache for current session: source_url -> (timestamp, listings)
    memory_cache: Arc<RwLock<HashMap<String, (Instant, Vec<SkillListing>)>>>,
}

impl SkillSourcesClient {
    /// Create a new skill sources client
    pub fn new(sources: Vec<MarketplaceSkillSource>) -> Self {
        Self {
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .user_agent("LocalRouter/1.0")
                .build()
                .expect("Failed to create HTTP client"),
            sources: Arc::new(RwLock::new(sources)),
            memory_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Update the skill sources
    pub fn set_sources(&self, sources: Vec<MarketplaceSkillSource>) {
        *self.sources.write() = sources;
        // Clear memory cache when sources change
        self.memory_cache.write().clear();
    }

    /// Search for skills (for backward compatibility - doesn't use persistent cache)
    pub async fn search(
        &self,
        query: Option<&str>,
        source_filter: Option<&str>,
    ) -> Result<Vec<SkillListing>, MarketplaceError> {
        let sources = self.sources.read().clone();
        let mut all_skills = Vec::new();

        for source in &sources {
            // Skip if source filter doesn't match
            if let Some(filter) = source_filter {
                if !source.label.to_lowercase().contains(&filter.to_lowercase()) {
                    continue;
                }
            }

            // Fetch skills from this source
            match self.fetch_skills_from_source(source).await {
                Ok(skills) => {
                    all_skills.extend(skills);
                }
                Err(e) => {
                    warn!("Failed to fetch skills from {}: {}", source.label, e);
                }
            }
        }

        // Filter by query if provided
        Self::filter_skills(&mut all_skills, query);

        Ok(all_skills)
    }

    /// Search for skills using persistent cache
    pub async fn search_with_cache<F>(
        &self,
        query: Option<&str>,
        source_filter: Option<&str>,
        cache: &RwLock<MarketplaceCache>,
        save_cache: F,
    ) -> Result<Vec<SkillListing>, MarketplaceError>
    where
        F: Fn(),
    {
        let sources = self.sources.read().clone();
        let mut all_skills = Vec::new();

        for source in &sources {
            // Skip if source filter doesn't match
            if let Some(filter) = source_filter {
                if !source.label.to_lowercase().contains(&filter.to_lowercase()) {
                    continue;
                }
            }

            let cache_key = source.repo_url.clone();

            // Check persistent cache first
            {
                let cache_guard = cache.read();
                if let Some(cached) = cache_guard.get_skills(&cache_key) {
                    debug!("Using cached skills for {}", source.label);
                    all_skills.extend(cached.clone());
                    continue;
                }
            }

            // Fetch skills from this source
            match self.fetch_skills_from_source(source).await {
                Ok(skills) => {
                    // Cache the results
                    {
                        let mut cache_guard = cache.write();
                        cache_guard.cache_skills(cache_key, skills.clone());
                    }
                    save_cache();
                    all_skills.extend(skills);
                }
                Err(e) => {
                    warn!("Failed to fetch skills from {}: {}", source.label, e);
                }
            }
        }

        // Filter by query if provided
        Self::filter_skills(&mut all_skills, query);

        Ok(all_skills)
    }

    /// Search for skills bypassing cache (force refresh)
    pub async fn search_fresh<F>(
        &self,
        query: Option<&str>,
        source_filter: Option<&str>,
        cache: &RwLock<MarketplaceCache>,
        save_cache: F,
    ) -> Result<Vec<SkillListing>, MarketplaceError>
    where
        F: Fn(),
    {
        let sources = self.sources.read().clone();
        let mut all_skills = Vec::new();

        for source in &sources {
            // Skip if source filter doesn't match
            if let Some(filter) = source_filter {
                if !source.label.to_lowercase().contains(&filter.to_lowercase()) {
                    continue;
                }
            }

            let cache_key = source.repo_url.clone();

            // Fetch skills from this source
            match self.fetch_skills_from_source(source).await {
                Ok(skills) => {
                    // Cache the results
                    {
                        let mut cache_guard = cache.write();
                        cache_guard.cache_skills(cache_key, skills.clone());
                    }
                    save_cache();
                    all_skills.extend(skills);
                }
                Err(e) => {
                    warn!("Failed to fetch skills from {}: {}", source.label, e);
                }
            }
        }

        // Filter by query if provided
        Self::filter_skills(&mut all_skills, query);

        Ok(all_skills)
    }

    /// Filter skills by query
    fn filter_skills(skills: &mut Vec<SkillListing>, query: Option<&str>) {
        if let Some(query) = query {
            let query_lower = query.to_lowercase();
            skills.retain(|s| {
                s.name.to_lowercase().contains(&query_lower)
                    || s.description
                        .as_ref()
                        .map(|d| d.to_lowercase().contains(&query_lower))
                        .unwrap_or(false)
                    || s.tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&query_lower))
            });
        }
    }

    /// Fetch skills from a single source
    async fn fetch_skills_from_source(
        &self,
        source: &MarketplaceSkillSource,
    ) -> Result<Vec<SkillListing>, MarketplaceError> {
        let (owner, repo) = parse_github_url(&source.repo_url)?;

        info!(
            "Fetching skills from {}/{} (path: {}, branch: {})",
            owner, repo, source.path, source.branch
        );

        // Get directory listing via GitHub Contents API
        let contents_url = format!(
            "https://api.github.com/repos/{}/{}/contents/{}?ref={}",
            owner,
            repo,
            urlencoding::encode(&source.path),
            source.branch
        );

        debug!("Fetching contents: {}", contents_url);

        let response = self
            .http_client
            .get(&contents_url)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(MarketplaceError::SkillSourceError(format!(
                "GitHub API returned {}: {}",
                status, body
            )));
        }

        let contents: Vec<GitHubContent> = response.json().await?;

        // Filter to directories only (skills are in subdirectories)
        let skill_dirs: Vec<_> = contents
            .iter()
            .filter(|c| c.content_type == "dir")
            .collect();

        let mut skills = Vec::new();

        for dir in skill_dirs {
            // Check if this directory has a SKILL.md
            match self
                .fetch_skill_from_dir(source, &owner, &repo, &dir.name)
                .await
            {
                Ok(skill) => skills.push(skill),
                Err(e) => {
                    debug!("Directory {} doesn't appear to be a skill: {}", dir.name, e);
                }
            }
        }

        info!("Found {} skills from {}", skills.len(), source.label);

        Ok(skills)
    }

    /// Fetch a single skill from a directory
    async fn fetch_skill_from_dir(
        &self,
        source: &MarketplaceSkillSource,
        owner: &str,
        repo: &str,
        skill_dir: &str,
    ) -> Result<SkillListing, MarketplaceError> {
        let skill_path = if source.path.is_empty() {
            skill_dir.to_string()
        } else {
            format!("{}/{}", source.path, skill_dir)
        };

        // Try to fetch SKILL.md
        let skill_md_url = format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}/SKILL.md",
            owner, repo, source.branch, skill_path
        );

        debug!("Fetching SKILL.md from {}", skill_md_url);

        let response = self.http_client.get(&skill_md_url).send().await?;

        if !response.status().is_success() {
            return Err(MarketplaceError::SkillSourceError(format!(
                "No SKILL.md found in {}",
                skill_dir
            )));
        }

        let skill_md_content = response.text().await?;

        // Parse SKILL.md frontmatter
        let metadata = parse_skill_frontmatter(&skill_md_content)?;

        // Check for multi-file skill (scripts/, references/, etc.)
        let (is_multi_file, files) = self
            .check_skill_files(source, owner, repo, &skill_path)
            .await
            .unwrap_or((false, vec![]));

        Ok(SkillListing {
            name: metadata.name.unwrap_or_else(|| skill_dir.to_string()),
            description: metadata.description,
            source_id: tokenize_name(&source.label),
            author: metadata.author,
            version: metadata.version,
            tags: metadata.tags.unwrap_or_default(),
            source_label: source.label.clone(),
            source_repo: source.repo_url.clone(),
            source_path: skill_path,
            source_branch: source.branch.clone(),
            skill_md_url,
            is_multi_file,
            files,
        })
    }

    /// Check for additional files in skill directory
    async fn check_skill_files(
        &self,
        source: &MarketplaceSkillSource,
        owner: &str,
        repo: &str,
        skill_path: &str,
    ) -> Result<(bool, Vec<SkillFileInfo>), MarketplaceError> {
        let contents_url = format!(
            "https://api.github.com/repos/{}/{}/contents/{}?ref={}",
            owner,
            repo,
            urlencoding::encode(skill_path),
            source.branch
        );

        let response = self
            .http_client
            .get(&contents_url)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Ok((false, vec![]));
        }

        let contents: Vec<GitHubContent> = response.json().await?;

        let mut files = Vec::new();
        let mut has_subdirs = false;

        for item in &contents {
            if item.content_type == "file" && item.name != "SKILL.md" {
                if let Some(ref download_url) = item.download_url {
                    files.push(SkillFileInfo {
                        path: item.name.clone(),
                        url: download_url.clone(),
                    });
                }
            } else if item.content_type == "dir" {
                has_subdirs = true;
                // Get files from subdirectory (one level only)
                let subdir_files = self
                    .get_files_from_subdir(source, owner, repo, skill_path, &item.name)
                    .await
                    .unwrap_or_default();
                files.extend(subdir_files);
            }
        }

        Ok((has_subdirs || files.len() > 1, files))
    }

    /// Get files from a subdirectory (non-recursive, one level deep only)
    async fn get_files_from_subdir(
        &self,
        source: &MarketplaceSkillSource,
        owner: &str,
        repo: &str,
        skill_path: &str,
        subdir: &str,
    ) -> Result<Vec<SkillFileInfo>, MarketplaceError> {
        let subdir_path = format!("{}/{}", skill_path, subdir);
        let contents_url = format!(
            "https://api.github.com/repos/{}/{}/contents/{}?ref={}",
            owner,
            repo,
            urlencoding::encode(&subdir_path),
            source.branch
        );

        let response = self
            .http_client
            .get(&contents_url)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Ok(vec![]);
        }

        let contents: Vec<GitHubContent> = response.json().await?;

        let mut files = Vec::new();

        for item in &contents {
            if item.content_type == "file" {
                if let Some(ref download_url) = item.download_url {
                    files.push(SkillFileInfo {
                        path: format!("{}/{}", subdir, item.name),
                        url: download_url.clone(),
                    });
                }
            }
            // Don't recurse into nested directories to avoid infinite recursion
            // Skills typically only have one level of subdirectories (scripts/, references/, etc.)
        }

        Ok(files)
    }

    /// Download a skill's files to a directory
    pub async fn download_skill(
        &self,
        listing: &SkillListing,
        target_dir: &std::path::Path,
    ) -> Result<(), MarketplaceError> {
        std::fs::create_dir_all(target_dir).map_err(|e| {
            MarketplaceError::InstallError(format!("Failed to create directory: {}", e))
        })?;

        // Download SKILL.md
        let skill_md = self
            .http_client
            .get(&listing.skill_md_url)
            .send()
            .await?
            .text()
            .await?;

        std::fs::write(target_dir.join("SKILL.md"), skill_md).map_err(|e| {
            MarketplaceError::InstallError(format!("Failed to write SKILL.md: {}", e))
        })?;

        // Download additional files
        for file in &listing.files {
            let file_path = target_dir.join(&file.path);

            // Create parent directories
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    MarketplaceError::InstallError(format!("Failed to create directory: {}", e))
                })?;
            }

            let content = self
                .http_client
                .get(&file.url)
                .send()
                .await?
                .bytes()
                .await?;

            std::fs::write(&file_path, content).map_err(|e| {
                MarketplaceError::InstallError(format!("Failed to write {}: {}", file.path, e))
            })?;
        }

        info!("Downloaded skill '{}' to {:?}", listing.name, target_dir);

        Ok(())
    }
}

impl Clone for SkillSourcesClient {
    fn clone(&self) -> Self {
        Self {
            http_client: self.http_client.clone(),
            sources: self.sources.clone(),
            memory_cache: self.memory_cache.clone(),
        }
    }
}

impl Default for SkillSourcesClient {
    fn default() -> Self {
        Self::new(vec![])
    }
}

/// GitHub Contents API response item
#[derive(Debug, Deserialize)]
struct GitHubContent {
    name: String,
    #[serde(rename = "type")]
    content_type: String,
    #[serde(default)]
    download_url: Option<String>,
}

/// Parsed SKILL.md frontmatter
#[derive(Debug, Default)]
struct SkillMetadata {
    name: Option<String>,
    description: Option<String>,
    author: Option<String>,
    version: Option<String>,
    tags: Option<Vec<String>>,
}

/// Parse a GitHub repo URL to extract owner and repo
fn parse_github_url(url: &str) -> Result<(String, String), MarketplaceError> {
    // Handle various GitHub URL formats:
    // - https://github.com/owner/repo
    // - https://github.com/owner/repo.git
    // - github.com/owner/repo

    let url = url.trim_end_matches('/').trim_end_matches(".git");

    let parts: Vec<&str> = url.split('/').collect();

    // Find "github.com" in the path and extract owner/repo after it
    let github_idx = parts.iter().position(|&p| p == "github.com");

    match github_idx {
        Some(idx) if parts.len() > idx + 2 => {
            Ok((parts[idx + 1].to_string(), parts[idx + 2].to_string()))
        }
        _ => Err(MarketplaceError::ParseError(format!(
            "Invalid GitHub URL: {}",
            url
        ))),
    }
}

/// Parse SKILL.md frontmatter (YAML between --- markers)
fn parse_skill_frontmatter(content: &str) -> Result<SkillMetadata, MarketplaceError> {
    let lines: Vec<&str> = content.lines().collect();

    // Find frontmatter boundaries
    if lines.first() != Some(&"---") {
        return Err(MarketplaceError::ParseError(
            "SKILL.md must start with ---".to_string(),
        ));
    }

    let end_idx = lines
        .iter()
        .skip(1)
        .position(|&l| l == "---")
        .ok_or_else(|| {
            MarketplaceError::ParseError("SKILL.md frontmatter not closed".to_string())
        })?;

    let frontmatter = lines[1..=end_idx].join("\n");

    // Parse YAML
    let yaml: serde_yaml::Value = serde_yaml::from_str(&frontmatter)
        .map_err(|e| MarketplaceError::ParseError(format!("Invalid YAML frontmatter: {}", e)))?;

    Ok(SkillMetadata {
        name: yaml.get("name").and_then(|v| v.as_str()).map(String::from),
        description: yaml
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from),
        author: yaml
            .get("author")
            .and_then(|v| v.as_str())
            .map(String::from),
        version: yaml
            .get("version")
            .and_then(|v| v.as_str())
            .map(String::from),
        tags: yaml.get("tags").and_then(|v| {
            v.as_sequence().map(|seq| {
                seq.iter()
                    .filter_map(|t| t.as_str().map(String::from))
                    .collect()
            })
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_url() {
        let (owner, repo) = parse_github_url("https://github.com/anthropics/skills").unwrap();
        assert_eq!(owner, "anthropics");
        assert_eq!(repo, "skills");

        let (owner, repo) = parse_github_url("https://github.com/owner/repo.git").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");

        let (owner, repo) = parse_github_url("github.com/owner/repo/").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_skill_frontmatter() {
        let content = r#"---
name: test-skill
description: A test skill
author: Test Author
version: 1.0.0
tags:
  - test
  - example
---
# Test Skill

This is the body.
"#;

        let metadata = parse_skill_frontmatter(content).unwrap();
        assert_eq!(metadata.name, Some("test-skill".to_string()));
        assert_eq!(metadata.description, Some("A test skill".to_string()));
        assert_eq!(metadata.author, Some("Test Author".to_string()));
        assert_eq!(metadata.version, Some("1.0.0".to_string()));
        assert_eq!(
            metadata.tags,
            Some(vec!["test".to_string(), "example".to_string()])
        );
    }

    #[test]
    fn test_parse_skill_frontmatter_minimal() {
        let content = r#"---
name: minimal
---
Body
"#;

        let metadata = parse_skill_frontmatter(content).unwrap();
        assert_eq!(metadata.name, Some("minimal".to_string()));
        assert_eq!(metadata.description, None);
    }
}
