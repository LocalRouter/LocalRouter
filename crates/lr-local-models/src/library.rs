//! The library of installed local models (`{storage_dir}/library.json`).
//!
//! Downloaded models live under `{storage_dir}/hf/{org}/{repo}/{commit}/…`;
//! imported files are referenced where they are and never copied, moved or
//! deleted.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::classify::{classify, ModelKind};
use crate::download::CompletedDownload;
use crate::gguf::{self, GgufSummary};
use crate::util;

/// Index file format version.
/// Version 2 added [`ModelKind::Unsupported`]; version 1 entries are
/// reclassified from their files once.
const LIBRARY_VERSION: u32 = 2;

/// Where a library entry came from.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntrySource {
    /// Downloaded from the Hub. `revision` is the commit SHA; `files` are the
    /// repo paths that make up this entry.
    HuggingFace {
        repo: String,
        revision: String,
        files: Vec<String>,
    },
    /// A local file referenced in place.
    Imported,
}

/// One installed model.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LibraryEntry {
    /// Stable, unique slug of `[a-z0-9._-]` (e.g. `qwen3-8b-q4_k_m`).
    pub id: String,
    pub display_name: String,
    pub source: EntrySource,
    /// The GGUF to load (the first part of a split model).
    pub model_path: PathBuf,
    /// The remaining parts of a split model, in order.
    pub extra_parts: Vec<PathBuf>,
    /// Multimodal projector (`mmproj`) downloaded with the model.
    pub projector_path: Option<PathBuf>,
    pub kind: ModelKind,
    pub quant: Option<String>,
    pub architecture: Option<String>,
    /// Trained context length.
    pub context_length: Option<u64>,
    pub pooling_type: Option<u32>,
    /// The chat template supports tool calls.
    pub has_tools: bool,
    /// Model parts plus projector.
    pub size_bytes: u64,
    pub installed_at: DateTime<Utc>,
}

/// Library errors.
#[derive(Debug, Clone, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum LibraryError {
    #[error("model not found in the library: {0}")]
    NotFound(String),
    #[error("not a usable GGUF model: {0}")]
    InvalidModel(String),
    #[error("{0}")]
    InvalidInput(String),
    #[error("library storage error: {0}")]
    Io(String),
}

impl From<std::io::Error> for LibraryError {
    fn from(e: std::io::Error) -> Self {
        LibraryError::Io(e.to_string())
    }
}

#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    entries: Vec<LibraryEntry>,
}

/// Default storage directory: `{config_dir}/models` (a temp directory if the
/// config directory cannot be determined).
pub fn default_storage_dir() -> PathBuf {
    lr_utils::paths::config_dir()
        .map(|d| d.join("models"))
        .unwrap_or_else(|_| std::env::temp_dir().join("localrouter-models"))
}

/// Thread-safe model library persisted to `{storage_dir}/library.json`.
pub struct Library {
    storage_dir: PathBuf,
    entries: Mutex<Vec<LibraryEntry>>,
}

/// A downloaded file as reported by [`CompletedDownload::files`].
type DownloadedFile = (String, PathBuf, u64, Option<String>);
/// `(part number, part count, file)`.
type SplitPart<'a> = (u32, u32, &'a DownloadedFile);

/// A GGUF (possibly split) found in a download or on disk.
struct Candidate {
    repo_paths: Vec<String>,
    parts: Vec<PathBuf>,
    size: u64,
    summary: GgufSummary,
    kind: ModelKind,
    /// File stem without `.gguf` and the split suffix.
    stem: String,
}

/// Re-derive each entry's kind from its model file (entries written before
/// a new kind existed). Unreadable files keep their recorded kind.
fn reclassify(mut entries: Vec<LibraryEntry>) -> Vec<LibraryEntry> {
    for entry in &mut entries {
        if entry.kind == ModelKind::Projector {
            continue;
        }
        if let Ok(header) = gguf::read_local_header(&entry.model_path) {
            let summary = GgufSummary::from_header(&header);
            entry.kind = classify(&summary, header.general_type());
        }
    }
    entries
}

impl Library {
    /// Open (or start) the library in `storage_dir`. An unreadable index is
    /// moved aside to `library.json.corrupt-<timestamp>` and an empty library
    /// is used.
    pub fn open(storage_dir: PathBuf) -> Self {
        let path = storage_dir.join("library.json");
        let mut migrated = false;
        let entries = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Index>(&bytes) {
                Ok(index) => {
                    if index.version > LIBRARY_VERSION {
                        tracing::warn!(
                            "library index version {} is newer than supported {}",
                            index.version,
                            LIBRARY_VERSION
                        );
                    }
                    if index.version < 2 {
                        migrated = true;
                        reclassify(index.entries)
                    } else {
                        index.entries
                    }
                }
                Err(e) => {
                    let backup = util::with_suffix(
                        &path,
                        &format!(".corrupt-{}", Utc::now().format("%Y%m%d%H%M%S")),
                    );
                    tracing::warn!(
                        "library index is unreadable ({e}); moving it to {}",
                        backup.display()
                    );
                    let _ = std::fs::rename(&path, &backup);
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        let library = Self {
            storage_dir,
            entries: Mutex::new(entries),
        };
        if migrated {
            let entries = library.entries.lock().clone();
            if let Err(e) = library.persist(&entries) {
                tracing::warn!("could not save the migrated model library: {e}");
            }
        }
        library
    }

    /// The storage directory.
    pub fn storage_dir(&self) -> &Path {
        &self.storage_dir
    }

    /// All entries.
    pub fn list(&self) -> Vec<LibraryEntry> {
        self.entries.lock().clone()
    }

    /// One entry by id.
    pub fn get(&self, id: &str) -> Option<LibraryEntry> {
        self.entries.lock().iter().find(|e| e.id == id).cloned()
    }

    fn persist(&self, entries: &[LibraryEntry]) -> Result<(), LibraryError> {
        let index = Index {
            version: LIBRARY_VERSION,
            entries: entries.to_vec(),
        };
        let bytes =
            serde_json::to_vec_pretty(&index).map_err(|e| LibraryError::Io(e.to_string()))?;
        util::write_atomic(&self.storage_dir.join("library.json"), &bytes)?;
        Ok(())
    }

    /// Add the GGUF models of a completed download. Split parts are grouped,
    /// a projector (`mmproj`, detected from its header) is attached to the
    /// chat/completion models of the same download, and each model is
    /// classified from its header. Re-adding a model that is already in the
    /// library updates it in place. Returns the added/updated entries.
    pub fn add_downloaded(
        &self,
        done: &CompletedDownload,
    ) -> Result<Vec<LibraryEntry>, LibraryError> {
        // File validation and index insertion must share the deletion lock.
        // Otherwise remove() could delete a validated file before it is added.
        let mut entries = self.entries.lock();
        let mut groups: BTreeMap<String, Vec<SplitPart<'_>>> = BTreeMap::new();
        for file in &done.files {
            let name = file.0.rsplit('/').next().unwrap_or(&file.0);
            let stem = gguf::strip_gguf_ext(name);
            if stem.len() == name.len() {
                continue; // not a .gguf
            }
            let dir = file.0.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            match gguf::split_suffix(stem) {
                Some((prefix, part, total)) => groups
                    .entry(format!("{dir}/{prefix}#{total}"))
                    .or_default()
                    .push((part, total, file)),
                None => groups.entry(file.0.clone()).or_default().push((1, 1, file)),
            }
        }
        if groups.is_empty() {
            return Ok(Vec::new());
        }

        let mut errors = Vec::new();
        let mut models = Vec::new();
        let mut projectors = Vec::new();
        for (key, mut parts) in groups {
            parts.sort_by_key(|p| p.0);
            let total = parts[0].1;
            let complete = parts.len() == total as usize
                && parts.iter().enumerate().all(|(i, p)| p.0 == i as u32 + 1);
            if !complete {
                errors.push(format!("{key}: missing split parts"));
                continue;
            }
            let first = parts[0].2;
            let header = match gguf::read_local_header(&first.1) {
                Ok(h) => h,
                Err(e) => {
                    errors.push(format!("{}: {e}", first.0));
                    continue;
                }
            };
            let summary = GgufSummary::from_header(&header);
            let kind = classify(&summary, header.general_type());
            let name = first.0.rsplit('/').next().unwrap_or(&first.0);
            let stem = gguf::strip_gguf_ext(name);
            let stem = gguf::split_suffix(stem).map(|s| s.0).unwrap_or(stem);
            let size = parts
                .iter()
                .map(|p| {
                    std::fs::metadata(&p.2 .1)
                        .map(|m| m.len())
                        .unwrap_or(p.2 .2)
                })
                .sum();
            let cand = Candidate {
                repo_paths: parts.iter().map(|p| p.2 .0.clone()).collect(),
                parts: parts.iter().map(|p| p.2 .1.clone()).collect(),
                size,
                summary,
                kind,
                stem: stem.to_string(),
            };
            if kind == ModelKind::Projector {
                projectors.push(cand);
            } else {
                models.push(cand);
            }
        }

        if models.is_empty() {
            // A projector downloaded on its own is still worth listing.
            models = std::mem::take(&mut projectors);
        }
        if models.is_empty() {
            return Err(LibraryError::InvalidModel(errors.join("; ")));
        }
        for e in &errors {
            tracing::warn!("skipping file from {}: {e}", done.repo);
        }
        let projector = projectors.first();

        let repo_name = done.repo.rsplit('/').next().unwrap_or(&done.repo);
        let repo_base = strip_gguf_suffix(repo_name);
        let multiple = models.len() > 1;
        let now = Utc::now();

        let mut result = Vec::with_capacity(models.len());
        let mut staged = entries.clone();
        for m in &models {
            let quant = m
                .summary
                .quant
                .clone()
                .or_else(|| gguf::quant_from_filename(&m.repo_paths[0]));
            let attach =
                projector.filter(|_| matches!(m.kind, ModelKind::Chat | ModelKind::Completion));
            let mut files = m.repo_paths.clone();
            if let Some(p) = attach {
                files.extend(p.repo_paths.iter().cloned());
            }
            let (base_id, display_name) = if multiple || quant.is_none() {
                (util::slugify(&m.stem), m.stem.clone())
            } else {
                let q = quant.clone().unwrap_or_default();
                (
                    util::slugify(&format!("{repo_base}-{q}")),
                    format!("{repo_base} {q}"),
                )
            };
            let mut entry = LibraryEntry {
                id: String::new(),
                display_name,
                source: EntrySource::HuggingFace {
                    repo: done.repo.clone(),
                    revision: done.revision.clone(),
                    files,
                },
                model_path: m.parts[0].clone(),
                extra_parts: m.parts[1..].to_vec(),
                projector_path: attach.map(|p| p.parts[0].clone()),
                kind: m.kind,
                quant,
                architecture: m.summary.architecture.clone(),
                context_length: m.summary.context_length,
                pooling_type: m.summary.pooling_type,
                has_tools: m.summary.chat_template_mentions_tools,
                size_bytes: m.size + attach.map(|p| p.size).unwrap_or(0),
                installed_at: now,
            };
            if let Some(existing) = staged.iter_mut().find(|e| e.model_path == entry.model_path) {
                entry.id = existing.id.clone();
                entry.display_name = existing.display_name.clone();
                *existing = entry.clone();
            } else {
                entry.id = unique_id(&base_id, &staged);
                staged.push(entry.clone());
            }
            result.push(entry);
        }
        self.persist(&staged)?;
        *entries = staged;
        Ok(result)
    }

    /// Import a local GGUF in place (it is never copied or deleted). The
    /// header is validated first. For split models pass the first part; all
    /// parts must sit next to it. Importing the same file again returns the
    /// existing entry.
    pub fn import_file(&self, path: &Path) -> Result<LibraryEntry, LibraryError> {
        // Keep validation and insertion atomic with respect to remove().
        let mut entries = self.entries.lock();
        let path = path
            .canonicalize()
            .map_err(|e| LibraryError::InvalidInput(format!("{}: {e}", path.display())))?;
        if !path.is_file() {
            return Err(LibraryError::InvalidInput(format!(
                "{} is not a file",
                path.display()
            )));
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let raw_stem = gguf::strip_gguf_ext(&name).to_string();
        let ext = &name[raw_stem.len()..];
        let (stem, extra_parts) = match gguf::split_suffix(&raw_stem) {
            Some((prefix, part, total)) => {
                if part != 1 {
                    return Err(LibraryError::InvalidInput(format!(
                        "{name} is part {part} of a split model; import the first part (-00001-of-{total:05})"
                    )));
                }
                let dir = path.parent().unwrap_or_else(|| Path::new("."));
                let mut extra = Vec::new();
                for i in 2..=total {
                    let p = dir.join(format!("{prefix}-{i:05}-of-{total:05}{ext}"));
                    if !p.is_file() {
                        return Err(LibraryError::InvalidInput(format!(
                            "split part {} is missing",
                            p.display()
                        )));
                    }
                    extra.push(p);
                }
                (prefix.to_string(), extra)
            }
            None => (raw_stem.clone(), Vec::new()),
        };
        let header = gguf::read_local_header(&path)
            .map_err(|e| LibraryError::InvalidModel(e.to_string()))?;
        let summary = GgufSummary::from_header(&header);
        let kind = classify(&summary, header.general_type());

        let mut size = std::fs::metadata(&path)?.len();
        for p in &extra_parts {
            size += std::fs::metadata(p)?.len();
        }

        if let Some(existing) = entries.iter().find(|e| e.model_path == path) {
            return Ok(existing.clone());
        }
        let entry = LibraryEntry {
            id: unique_id(&util::slugify(&stem), &entries),
            display_name: stem.clone(),
            source: EntrySource::Imported,
            model_path: path.clone(),
            extra_parts,
            projector_path: None,
            kind,
            quant: summary
                .quant
                .clone()
                .or_else(|| gguf::quant_from_filename(&name)),
            architecture: summary.architecture.clone(),
            context_length: summary.context_length,
            pooling_type: summary.pooling_type,
            has_tools: summary.chat_template_mentions_tools,
            size_bytes: size,
            installed_at: Utc::now(),
        };
        let mut staged = entries.clone();
        staged.push(entry.clone());
        self.persist(&staged)?;
        *entries = staged;
        Ok(entry)
    }

    /// Remove an entry. With `delete_files`, files of downloaded entries that
    /// live under the storage directory and are not used by another entry
    /// are deleted (empty directories are pruned). Imported files are never
    /// deleted.
    pub fn remove(&self, id: &str, delete_files: bool) -> Result<(), LibraryError> {
        // Keep the index locked through deletion so a concurrent import/add
        // cannot start referencing a file after the sharing check.
        let mut entries = self.entries.lock();
        let idx = entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| LibraryError::NotFound(id.to_string()))?;
        let mut staged = entries.clone();
        let removed = staged.remove(idx);
        self.persist(&staged)?;
        *entries = staged;
        if !delete_files || removed.source == EntrySource::Imported {
            return Ok(());
        }
        let in_use: HashSet<PathBuf> = entries
            .iter()
            .flat_map(entry_paths)
            .map(|path| util::canonicalize_lenient(&path))
            .collect();
        let root = util::canonicalize_lenient(&self.storage_dir);
        for path in entry_paths(&removed) {
            let canonical = util::canonicalize_lenient(&path);
            if in_use.contains(&canonical) {
                continue;
            }
            if !canonical.starts_with(&root) || canonical == root {
                tracing::warn!(
                    "not deleting {} (outside the model storage directory)",
                    path.display()
                );
                continue;
            }
            match std::fs::remove_file(&canonical) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            prune_empty_dirs(canonical.parent(), &root);
        }
        Ok(())
    }

    /// Change an entry's display name.
    pub fn rename(&self, id: &str, display_name: &str) -> Result<(), LibraryError> {
        let name = display_name.trim();
        if name.is_empty() {
            return Err(LibraryError::InvalidInput(
                "the name must not be empty".into(),
            ));
        }
        let mut entries = self.entries.lock();
        let mut staged = entries.clone();
        let entry = staged
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| LibraryError::NotFound(id.to_string()))?;
        entry.display_name = name.to_string();
        self.persist(&staged)?;
        *entries = staged;
        Ok(())
    }

    /// Bytes used on disk by library files inside the storage directory
    /// (distinct files; imported files elsewhere are not counted).
    pub fn disk_usage(&self) -> u64 {
        let root = util::canonicalize_lenient(&self.storage_dir);
        let paths: HashSet<PathBuf> = self
            .entries
            .lock()
            .iter()
            .flat_map(entry_paths)
            .map(|p| util::canonicalize_lenient(&p))
            .filter(|p| p.starts_with(&root))
            .collect();
        paths
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .sum()
    }
}

fn entry_paths(e: &LibraryEntry) -> Vec<PathBuf> {
    let mut v = vec![e.model_path.clone()];
    v.extend(e.extra_parts.iter().cloned());
    v.extend(e.projector_path.iter().cloned());
    v
}

/// Remove empty directories from `dir` upwards, stopping at `root`.
fn prune_empty_dirs(dir: Option<&Path>, root: &Path) {
    let mut current = dir.map(Path::to_path_buf);
    while let Some(d) = current {
        if d == root || !d.starts_with(root) {
            break;
        }
        if std::fs::remove_dir(&d).is_err() {
            break; // not empty (or not removable)
        }
        current = d.parent().map(Path::to_path_buf);
    }
}

/// `Qwen3-8B-GGUF` → `Qwen3-8B`.
fn strip_gguf_suffix(name: &str) -> &str {
    for sep in ['-', '_', '.'] {
        if name.len() > 5 {
            let (head, tail) = name.split_at(name.len() - 5);
            if tail.starts_with(sep) && tail[1..].eq_ignore_ascii_case("gguf") {
                return head;
            }
        }
    }
    name
}

/// `base`, or `base-2`, `base-3`, … if taken.
fn unique_id(base: &str, entries: &[LibraryEntry]) -> String {
    let taken: HashSet<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    if !taken.contains(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|c| !taken.contains(c.as_str()))
        .unwrap_or_else(|| format!("{base}-{}", uuid::Uuid::new_v4().simple()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gguf::test_support::{GgufBuilder, Val};

    struct Env {
        dir: tempfile::TempDir,
        lib: Library,
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path().to_path_buf());
        Env { dir, lib }
    }

    fn chat_model() -> Vec<u8> {
        GgufBuilder::decoder("qwen3")
            .str(
                "tokenizer.chat_template",
                "{% if tools %}{{ tools }}{% endif %}",
            )
            .build_file(1000)
    }

    #[test]
    fn version_1_entries_are_reclassified_once() {
        let dir = tempfile::tempdir().unwrap();
        let diffusion = dir.path().join("image.gguf");
        std::fs::write(
            &diffusion,
            GgufBuilder::new()
                .str("general.architecture", "qwen_image21")
                .u32("general.file_type", 2)
                .tensor("img_in.weight")
                .build_file(400),
        )
        .unwrap();
        let index = serde_json::json!({
            "version": 1,
            "entries": [{
                "id": "qwen-image", "display_name": "Qwen Image",
                "source": {"type": "imported"},
                "model_path": diffusion, "extra_parts": [], "projector_path": null,
                "kind": "completion", "quant": "Q4_0", "architecture": "qwen_image21",
                "context_length": null, "pooling_type": null, "has_tools": false,
                "size_bytes": 400, "installed_at": "2026-09-25T00:00:00Z"
            }]
        });
        std::fs::write(
            dir.path().join("library.json"),
            serde_json::to_vec(&index).unwrap(),
        )
        .unwrap();
        let lib = Library::open(dir.path().to_path_buf());
        assert_eq!(lib.get("qwen-image").unwrap().kind, ModelKind::Unsupported);
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("library.json")).unwrap())
                .unwrap();
        assert_eq!(saved["version"], 2);
        assert_eq!(saved["entries"][0]["kind"], "unsupported");
    }

    fn projector() -> Vec<u8> {
        GgufBuilder::new()
            .str("general.architecture", "clip")
            .str("general.type", "mmproj")
            .u32("general.file_type", 1)
            .build_file(500)
    }

    impl Env {
        /// Write files under the HF layout and describe them as a download.
        fn download(&self, repo: &str, files: &[(&str, Vec<u8>)]) -> CompletedDownload {
            let base = self.dir.path().join("hf").join(repo).join("c0ffee");
            let mut out = Vec::new();
            for (p, bytes) in files {
                let local = base.join(p);
                std::fs::create_dir_all(local.parent().unwrap()).unwrap();
                std::fs::write(&local, bytes).unwrap();
                out.push((p.to_string(), local, bytes.len() as u64, None));
            }
            CompletedDownload {
                repo: repo.to_string(),
                revision: "c0ffee".into(),
                purpose: None,
                files: out,
            }
        }
    }

    #[test]
    fn add_single_chat_model() {
        let env = env();
        let bytes = chat_model();
        let done = env.download(
            "unsloth/Qwen3-8B-GGUF",
            &[
                ("Qwen3-8B-Q4_K_M.gguf", bytes.clone()),
                ("README.md", b"hi".to_vec()),
            ],
        );
        let added = env.lib.add_downloaded(&done).unwrap();
        assert_eq!(added.len(), 1);
        let e = &added[0];
        assert_eq!(e.id, "qwen3-8b-q4_k_m");
        assert_eq!(e.display_name, "Qwen3-8B Q4_K_M");
        assert_eq!(e.kind, ModelKind::Chat);
        assert!(e.has_tools);
        assert_eq!(e.quant.as_deref(), Some("Q4_K_M"));
        assert_eq!(e.architecture.as_deref(), Some("qwen3"));
        assert_eq!(e.context_length, Some(32768));
        assert_eq!(e.size_bytes, bytes.len() as u64);
        assert_eq!(
            e.source,
            EntrySource::HuggingFace {
                repo: "unsloth/Qwen3-8B-GGUF".into(),
                revision: "c0ffee".into(),
                files: vec!["Qwen3-8B-Q4_K_M.gguf".into()],
            }
        );
        assert_eq!(env.lib.get("qwen3-8b-q4_k_m").unwrap(), *e);

        // Re-adding updates in place.
        let again = env.lib.add_downloaded(&done).unwrap();
        assert_eq!(again[0].id, e.id);
        assert_eq!(env.lib.list().len(), 1);

        // Persisted.
        let reopened = Library::open(env.dir.path().to_path_buf());
        assert_eq!(reopened.list().len(), 1);
        assert_eq!(reopened.list()[0].id, "qwen3-8b-q4_k_m");
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(env.dir.path().join("library.json")).unwrap())
                .unwrap();
        assert_eq!(raw["version"], LIBRARY_VERSION);
        assert_eq!(raw["entries"][0]["source"]["type"], "hugging_face");
        assert_eq!(raw["entries"][0]["kind"], "chat");
    }

    #[test]
    fn split_parts_and_projector_are_grouped() {
        let env = env();
        let first = GgufBuilder::decoder("gemma3")
            .str("tokenizer.chat_template", "{{ messages }}")
            .kv("split.count", Val::U32(3))
            .build_file(100);
        let rest = GgufBuilder::new()
            .kv("split.count", Val::U32(3))
            .build_file(100);
        let done = env.download(
            "org/Big-GGUF",
            &[
                ("Q4_K_M/Big-Q4_K_M-00002-of-00003.gguf", rest.clone()),
                ("Q4_K_M/Big-Q4_K_M-00001-of-00003.gguf", first.clone()),
                ("Q4_K_M/Big-Q4_K_M-00003-of-00003.gguf", rest.clone()),
                ("mmproj-F16.gguf", projector()),
            ],
        );
        let added = env.lib.add_downloaded(&done).unwrap();
        assert_eq!(added.len(), 1);
        let e = &added[0];
        assert_eq!(e.kind, ModelKind::Chat);
        assert!(e.model_path.ends_with("Big-Q4_K_M-00001-of-00003.gguf"));
        assert_eq!(e.extra_parts.len(), 2);
        assert!(e.extra_parts[0].ends_with("Big-Q4_K_M-00002-of-00003.gguf"));
        assert!(e.extra_parts[1].ends_with("Big-Q4_K_M-00003-of-00003.gguf"));
        assert!(e
            .projector_path
            .as_ref()
            .unwrap()
            .ends_with("mmproj-F16.gguf"));
        assert_eq!(
            e.size_bytes,
            (first.len() + 2 * rest.len() + projector().len()) as u64
        );
        match &e.source {
            EntrySource::HuggingFace { files, .. } => assert_eq!(files.len(), 4),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn incomplete_split_or_invalid_file_is_an_error() {
        let env = env();
        let first = GgufBuilder::decoder("llama").build_file(10);
        let done = env.download("org/x", &[("m-00001-of-00002.gguf", first)]);
        assert!(matches!(
            env.lib.add_downloaded(&done),
            Err(LibraryError::InvalidModel(_))
        ));
        let done = env.download("org/y", &[("bad.gguf", b"garbage!".to_vec())]);
        assert!(matches!(
            env.lib.add_downloaded(&done),
            Err(LibraryError::InvalidModel(_))
        ));
        let done = env.download("org/z", &[("config.json", b"{}".to_vec())]);
        assert!(env.lib.add_downloaded(&done).unwrap().is_empty());
        assert!(env.lib.list().is_empty());
    }

    #[test]
    fn projector_alone_and_other_kinds() {
        let env = env();
        let done = env.download("org/Vision-GGUF", &[("mmproj-model-f16.gguf", projector())]);
        let added = env.lib.add_downloaded(&done).unwrap();
        assert_eq!(added[0].kind, ModelKind::Projector);
        assert_eq!(added[0].id, "vision-f16");

        let emb = GgufBuilder::decoder("bert")
            .u32("bert.pooling_type", 1)
            .build_file(10);
        let rr = GgufBuilder::decoder("bert")
            .u32("bert.pooling_type", 4)
            .build_file(10);
        let done = env.download(
            "org/Mixed",
            &[("embed-f16.gguf", emb), ("rerank-f16.gguf", rr)],
        );
        let added = env.lib.add_downloaded(&done).unwrap();
        assert_eq!(added.len(), 2);
        let by_id: BTreeMap<_, _> = added.iter().map(|e| (e.id.as_str(), e)).collect();
        assert_eq!(by_id["embed-f16"].kind, ModelKind::Embedding);
        assert_eq!(by_id["embed-f16"].pooling_type, Some(1));
        assert_eq!(by_id["rerank-f16"].kind, ModelKind::Reranker);
        // Projectors are only attached to generative models.
        assert!(added.iter().all(|e| e.projector_path.is_none()));
    }

    #[test]
    fn id_collisions_get_suffixes() {
        let env = env();
        let a = env.download("a/Model-GGUF", &[("Model-Q4_K_M.gguf", chat_model())]);
        let b = env.download("b/Model-GGUF", &[("Model-Q4_K_M.gguf", chat_model())]);
        let c = env.download("c/Model_GGUF", &[("Model-Q4_K_M.gguf", chat_model())]);
        assert_eq!(env.lib.add_downloaded(&a).unwrap()[0].id, "model-q4_k_m");
        assert_eq!(env.lib.add_downloaded(&b).unwrap()[0].id, "model-q4_k_m-2");
        assert_eq!(env.lib.add_downloaded(&c).unwrap()[0].id, "model-q4_k_m-3");
        let ids: HashSet<String> = env.lib.list().into_iter().map(|e| e.id).collect();
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn import_valid_invalid_and_split() {
        let env = env();
        let outside = tempfile::tempdir().unwrap();
        let p = outside.path().join("My Model Q8_0.gguf");
        std::fs::write(&p, chat_model()).unwrap();
        let e = env.lib.import_file(&p).unwrap();
        assert_eq!(e.source, EntrySource::Imported);
        assert_eq!(e.id, "my-model-q8_0");
        assert_eq!(e.kind, ModelKind::Chat);
        assert_eq!(e.model_path, p.canonicalize().unwrap());
        // Same file again → same entry.
        assert_eq!(env.lib.import_file(&p).unwrap().id, e.id);
        assert_eq!(env.lib.list().len(), 1);

        let bad = outside.path().join("bad.gguf");
        std::fs::write(&bad, b"not a gguf at all").unwrap();
        assert!(matches!(
            env.lib.import_file(&bad),
            Err(LibraryError::InvalidModel(_))
        ));
        assert!(matches!(
            env.lib.import_file(&outside.path().join("missing.gguf")),
            Err(LibraryError::InvalidInput(_))
        ));

        let s1 = outside.path().join("s-00001-of-00002.gguf");
        let s2 = outside.path().join("s-00002-of-00002.gguf");
        std::fs::write(&s1, chat_model()).unwrap();
        assert!(matches!(
            env.lib.import_file(&s1),
            Err(LibraryError::InvalidInput(_))
        ));
        std::fs::write(&s2, b"part two").unwrap();
        assert!(matches!(
            env.lib.import_file(&s2),
            Err(LibraryError::InvalidInput(_))
        ));
        let e = env.lib.import_file(&s1).unwrap();
        assert_eq!(e.id, "s");
        assert_eq!(e.extra_parts, vec![s2.canonicalize().unwrap()]);
    }

    #[test]
    fn remove_deletes_only_owned_unshared_files() {
        let env = env();
        let done = env.download(
            "org/Duo-GGUF",
            &[
                ("a-Q4_K_M.gguf", chat_model()),
                ("b-Q8_0.gguf", chat_model()),
                ("mmproj-f16.gguf", projector()),
            ],
        );
        let added = env.lib.add_downloaded(&done).unwrap();
        assert_eq!(added.len(), 2);
        let proj = added[0].projector_path.clone().unwrap();
        assert_eq!(added[1].projector_path.as_ref(), Some(&proj));
        let usage = env.lib.disk_usage();
        assert_eq!(
            usage,
            (2 * chat_model().len() + projector().len()) as u64,
            "shared projector counted once"
        );

        env.lib.remove(&added[0].id, true).unwrap();
        assert!(!added[0].model_path.exists());
        assert!(proj.exists(), "still used by the other entry");
        env.lib.remove(&added[1].id, true).unwrap();
        assert!(!added[1].model_path.exists());
        assert!(!proj.exists());
        assert!(!env.dir.path().join("hf/org").exists(), "empty dirs pruned");
        assert!(env.dir.path().exists());
        assert!(matches!(
            env.lib.remove("nope", true),
            Err(LibraryError::NotFound(_))
        ));

        // Imported files are never deleted.
        let outside = tempfile::tempdir().unwrap();
        let p = outside.path().join("keep.gguf");
        std::fs::write(&p, chat_model()).unwrap();
        let e = env.lib.import_file(&p).unwrap();
        assert_eq!(
            env.lib.disk_usage(),
            0,
            "imported files are not library storage"
        );
        env.lib.remove(&e.id, true).unwrap();
        assert!(p.exists());
        assert!(env.lib.list().is_empty());
    }

    #[test]
    fn remove_preserves_files_referenced_through_equivalent_paths() {
        let env = env();
        let done = env.download("org/Shared-GGUF", &[("shared-Q4_0.gguf", chat_model())]);
        let entry = env.lib.add_downloaded(&done).unwrap().remove(0);
        let mut alias = entry.clone();
        alias.id = "imported-alias".into();
        alias.source = EntrySource::Imported;
        alias.model_path = entry
            .model_path
            .parent()
            .unwrap()
            .join(".")
            .join(entry.model_path.file_name().unwrap());
        env.lib.entries.lock().push(alias);
        env.lib.remove(&entry.id, true).unwrap();
        assert!(
            entry.model_path.exists(),
            "an imported entry still references this file"
        );
        assert_eq!(env.lib.list().len(), 1);
    }

    #[test]
    fn remove_without_delete_keeps_files() {
        let env = env();
        let done = env.download("org/K-GGUF", &[("k-Q4_0.gguf", chat_model())]);
        let e = env.lib.add_downloaded(&done).unwrap().remove(0);
        env.lib.remove(&e.id, false).unwrap();
        assert!(e.model_path.exists());
    }

    #[test]
    fn rename_and_corrupt_index() {
        let env = env();
        let done = env.download("org/R-GGUF", &[("r-Q4_0.gguf", chat_model())]);
        let e = env.lib.add_downloaded(&done).unwrap().remove(0);
        env.lib.rename(&e.id, "  My favourite  ").unwrap();
        assert_eq!(env.lib.get(&e.id).unwrap().display_name, "My favourite");
        assert!(env.lib.rename(&e.id, "  ").is_err());
        assert!(matches!(
            env.lib.rename("missing", "x"),
            Err(LibraryError::NotFound(_))
        ));
        let reopened = Library::open(env.dir.path().to_path_buf());
        assert_eq!(reopened.get(&e.id).unwrap().display_name, "My favourite");

        std::fs::write(env.dir.path().join("library.json"), b"{broken").unwrap();
        let fresh = Library::open(env.dir.path().to_path_buf());
        assert!(fresh.list().is_empty());
        let backups = std::fs::read_dir(env.dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
            .count();
        assert_eq!(backups, 1);
    }

    #[test]
    fn helpers() {
        assert_eq!(strip_gguf_suffix("Qwen3-8B-GGUF"), "Qwen3-8B");
        assert_eq!(strip_gguf_suffix("x_gguf"), "x");
        assert_eq!(strip_gguf_suffix("gguf"), "gguf");
        assert_eq!(strip_gguf_suffix("Model"), "Model");
        assert!(default_storage_dir().ends_with("models"));
    }
}
