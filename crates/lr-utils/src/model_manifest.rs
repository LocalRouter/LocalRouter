//! Manifests that tie a downloaded model's files to one upstream revision.
//!
//! A model directory holds several files fetched one by one (weights,
//! tokenizer, …). Without a record of what belongs together, an interrupted
//! update leaves a mix of old and new files that still "looks downloaded".
//! The downloader therefore removes the manifest before it touches any file,
//! and writes a new one — revision plus size and SHA-256 of every file —
//! only after the last file is in place. A directory is complete only when
//! its manifest is present and the files match it.
//!
//! Directories downloaded before manifests existed hold the files but no
//! manifest; they are reported as [`ModelState::Legacy`] and adopted (a
//! manifest is written for the files as they are) the first time they are
//! loaded, rather than forcing a multi-hundred-megabyte download.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

/// Name of the manifest inside a model directory.
pub const MANIFEST_FILE: &str = ".localrouter-model.json";

/// Present while a download is replacing a directory's files; removed only
/// after the new manifest is written. Tells an interrupted update apart from
/// a download made before manifests existed.
pub const PENDING_FILE: &str = ".localrouter-model.pending";

/// What a complete model directory holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelManifest {
    /// Upstream repository, e.g. `sentence-transformers/all-MiniLM-L6-v2`.
    pub repo: String,
    /// Commit the files were downloaded from; `None` for a directory adopted
    /// from a download made before manifests existed.
    pub revision: Option<String>,
    /// File name (relative to the directory) -> what it must be.
    pub files: BTreeMap<String, ManifestFile>,
}

/// One file of a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    pub size: u64,
    /// Lowercase hex SHA-256 of the content.
    pub sha256: String,
}

/// What a model directory holds, as far as a cheap check can tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelState {
    /// The manifest is present, names every expected file, and every file has
    /// the recorded size.
    Complete(ModelManifest),
    /// Every expected file is present but there is no manifest: a download
    /// made before manifests existed.
    Legacy,
    /// Not usable: missing files, no manifest after an interrupted update, or
    /// files that do not match the manifest. The text says which.
    Incomplete(String),
}

impl ModelState {
    /// Whether the files can be loaded (after adoption, for `Legacy`).
    pub fn is_usable(&self) -> bool {
        !matches!(self, Self::Incomplete(_))
    }

    /// The revision recorded, if any.
    pub fn revision(&self) -> Option<&str> {
        match self {
            Self::Complete(manifest) => manifest.revision.as_deref(),
            _ => None,
        }
    }
}

/// Size and SHA-256 of the file at `path`. Blocking.
pub fn describe_file(path: &Path) -> io::Result<ManifestFile> {
    let mut file = std::fs::File::open(path)?;
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut size = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size += read as u64;
        context.update(&buffer[..read]);
    }
    let sha256 = context
        .finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(ManifestFile { size, sha256 })
}

/// Read the manifest of `dir`: `Ok(None)` when there is none.
pub fn read(dir: &Path) -> Result<Option<ModelManifest>, String> {
    match std::fs::read(dir.join(MANIFEST_FILE)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("the model manifest is unreadable: {e}")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("the model manifest cannot be read: {e}")),
    }
}

/// Write the manifest of `dir`, replacing any previous one atomically.
pub fn write(dir: &Path, manifest: &ModelManifest) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(manifest).map_err(io::Error::other)?;
    let part = dir.join(format!("{MANIFEST_FILE}.part"));
    std::fs::write(&part, bytes)?;
    std::fs::rename(&part, dir.join(MANIFEST_FILE)).inspect_err(|_| {
        let _ = std::fs::remove_file(&part);
    })?;
    forget_verified(dir);
    remove_if_present(&dir.join(PENDING_FILE))
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Mark `dir` as being updated: it counts as incomplete until [`write`]
/// records the new manifest. Call before replacing any of its files.
pub fn begin_update(dir: &Path) -> io::Result<()> {
    forget_verified(dir);
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(PENDING_FILE), b"")?;
    remove_if_present(&dir.join(MANIFEST_FILE))
}

/// Cheap check of `dir` against the files a model of `repo` needs: reads the
/// manifest and the files' sizes, never their content.
pub fn state(dir: &Path, repo: &str, files: &[&str]) -> ModelState {
    if dir.join(PENDING_FILE).exists() {
        return ModelState::Incomplete("an earlier download did not finish".into());
    }
    let manifest = match read(dir) {
        Ok(Some(manifest)) => manifest,
        Ok(None) => {
            let missing: Vec<&str> = files
                .iter()
                .copied()
                .filter(|f| !dir.join(f).is_file())
                .collect();
            return if missing.is_empty() {
                ModelState::Legacy
            } else {
                ModelState::Incomplete(format!("missing {}", missing.join(", ")))
            };
        }
        Err(e) => return ModelState::Incomplete(e),
    };
    if manifest.repo != repo {
        return ModelState::Incomplete(format!("the files are from {}, not {repo}", manifest.repo));
    }
    for file in files {
        let Some(expected) = manifest.files.get(*file) else {
            return ModelState::Incomplete(format!("the manifest does not list {file}"));
        };
        match std::fs::metadata(dir.join(file)) {
            Ok(metadata) if metadata.is_file() && metadata.len() == expected.size => {}
            Ok(_) => return ModelState::Incomplete(format!("{file} does not match the manifest")),
            Err(_) => return ModelState::Incomplete(format!("missing {file}")),
        }
    }
    ModelState::Complete(manifest)
}

/// Directories this process hashed and found to match their manifest.
static VERIFIED: LazyLock<Mutex<HashMap<PathBuf, ModelManifest>>> = LazyLock::new(Default::default);

fn forget_verified(dir: &Path) {
    if let Ok(mut verified) = VERIFIED.lock() {
        verified.remove(dir);
    }
}

/// Make sure `dir` holds a whole model of `repo` before it is loaded:
/// hashes every file against the manifest (once per process per manifest),
/// and adopts a [`ModelState::Legacy`] directory by writing a manifest for
/// its files. Blocking. Returns the manifest.
pub fn verify_for_load(dir: &Path, repo: &str, files: &[&str]) -> Result<ModelManifest, String> {
    let manifest = match state(dir, repo, files) {
        ModelState::Complete(manifest) => manifest,
        ModelState::Legacy => {
            let mut adopted = ModelManifest {
                repo: repo.to_string(),
                revision: None,
                files: BTreeMap::new(),
            };
            for file in files {
                let entry = describe_file(&dir.join(file))
                    .map_err(|e| format!("cannot read {file}: {e}"))?;
                adopted.files.insert(file.to_string(), entry);
            }
            write(dir, &adopted).map_err(|e| format!("cannot record the model files: {e}"))?;
            tracing::info!("Recorded a manifest for the model files in {:?}", dir);
            if let Ok(mut verified) = VERIFIED.lock() {
                verified.insert(dir.to_path_buf(), adopted.clone());
            }
            return Ok(adopted);
        }
        ModelState::Incomplete(reason) => return Err(incomplete_message(&reason)),
    };
    if VERIFIED
        .lock()
        .ok()
        .and_then(|verified| verified.get(dir).cloned())
        .is_some_and(|known| known == manifest)
    {
        return Ok(manifest);
    }
    for file in files {
        let expected = &manifest.files[*file];
        let found =
            describe_file(&dir.join(file)).map_err(|e| format!("cannot read {file}: {e}"))?;
        if found != *expected {
            return Err(incomplete_message(&format!(
                "{file} does not match the manifest"
            )));
        }
    }
    if let Ok(mut verified) = VERIFIED.lock() {
        verified.insert(dir.to_path_buf(), manifest.clone());
    }
    Ok(manifest)
}

fn incomplete_message(reason: &str) -> String {
    format!("The model download is incomplete or damaged ({reason}). Download it again.")
}

/// The commit a file returned by the Hugging Face cache belongs to: the
/// cache lays files out as `.../snapshots/<commit>/<file>`.
pub fn revision_of_cached_file(path: &Path) -> Option<String> {
    let snapshot = path.parent()?;
    if snapshot.parent()?.file_name()? != "snapshots" {
        return None;
    }
    let revision = snapshot.file_name()?.to_str()?;
    (!revision.is_empty() && revision.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| revision.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);
    impl TestDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "localrouter-utils-manifest-{}-{}",
                std::process::id(),
                name
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const REPO: &str = "org/model";
    const FILES: &[&str] = &["model.safetensors", "tokenizer.json"];

    fn download(dir: &Path, weights: &[u8], revision: &str) -> ModelManifest {
        begin_update(dir).unwrap();
        std::fs::write(dir.join("model.safetensors"), weights).unwrap();
        std::fs::write(dir.join("tokenizer.json"), b"{}").unwrap();
        let mut manifest = ModelManifest {
            repo: REPO.into(),
            revision: Some(revision.into()),
            files: BTreeMap::new(),
        };
        for file in FILES {
            manifest
                .files
                .insert(file.to_string(), describe_file(&dir.join(file)).unwrap());
        }
        write(dir, &manifest).unwrap();
        manifest
    }

    #[test]
    fn a_complete_download_is_complete_and_verifies() {
        let dir = TestDir::new("complete");
        let manifest = download(&dir.0, b"weights", "abc123");
        assert_eq!(
            state(&dir.0, REPO, FILES),
            ModelState::Complete(manifest.clone())
        );
        assert_eq!(state(&dir.0, REPO, FILES).revision(), Some("abc123"));
        assert_eq!(verify_for_load(&dir.0, REPO, FILES).unwrap(), manifest);
        assert_eq!(
            manifest.files["tokenizer.json"].sha256,
            "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
        assert!(!dir.0.join(format!("{MANIFEST_FILE}.part")).exists());
    }

    #[test]
    fn an_interrupted_update_is_incomplete() {
        let dir = TestDir::new("interrupted");
        download(&dir.0, b"old weights", "old");
        // The update marks the directory and dies after one file. All files
        // are there (a presence-only check would call this downloaded), but
        // they are a mix of two revisions.
        begin_update(&dir.0).unwrap();
        std::fs::write(dir.0.join("model.safetensors"), b"new weights!").unwrap();
        let found = state(&dir.0, REPO, FILES);
        assert!(!found.is_usable(), "{found:?}");
        assert!(verify_for_load(&dir.0, REPO, FILES).is_err());

        // Finishing the download makes it complete again.
        let manifest = download(&dir.0, b"new weights!", "new");
        assert_eq!(state(&dir.0, REPO, FILES), ModelState::Complete(manifest));
        assert!(!dir.0.join(PENDING_FILE).exists());
    }

    #[test]
    fn a_file_replaced_behind_the_manifest_is_detected() {
        let dir = TestDir::new("replaced");
        download(&dir.0, b"weights", "abc");
        // Different size: the cheap check sees it.
        std::fs::write(dir.0.join("model.safetensors"), b"other weights").unwrap();
        assert!(!state(&dir.0, REPO, FILES).is_usable());
        assert!(verify_for_load(&dir.0, REPO, FILES)
            .unwrap_err()
            .contains("Download it again"));

        // Same size, different content: only hashing sees it.
        let dir = TestDir::new("replaced-same-size");
        download(&dir.0, b"weights", "abc");
        std::fs::write(dir.0.join("model.safetensors"), b"WEIGHTS").unwrap();
        assert!(state(&dir.0, REPO, FILES).is_usable());
        let error = verify_for_load(&dir.0, REPO, FILES).unwrap_err();
        assert!(error.contains("model.safetensors"), "{error}");
    }

    #[test]
    fn missing_files_and_foreign_manifests_are_incomplete() {
        let dir = TestDir::new("missing");
        assert_eq!(
            state(&dir.0, REPO, FILES),
            ModelState::Incomplete("missing model.safetensors, tokenizer.json".into())
        );
        download(&dir.0, b"weights", "abc");
        assert!(!state(&dir.0, "other/repo", FILES).is_usable());
        assert!(!state(&dir.0, REPO, &["model.safetensors", "vocab.txt"]).is_usable());
        std::fs::write(dir.0.join(MANIFEST_FILE), b"{not json").unwrap();
        assert!(!state(&dir.0, REPO, FILES).is_usable());
    }

    #[test]
    fn a_legacy_download_is_adopted_on_load() {
        let dir = TestDir::new("legacy");
        std::fs::write(dir.0.join("model.safetensors"), b"weights").unwrap();
        std::fs::write(dir.0.join("tokenizer.json"), b"{}").unwrap();
        assert_eq!(state(&dir.0, REPO, FILES), ModelState::Legacy);
        let adopted = verify_for_load(&dir.0, REPO, FILES).unwrap();
        assert_eq!(adopted.revision, None);
        assert_eq!(adopted.files.len(), 2);
        assert_eq!(read(&dir.0).unwrap(), Some(adopted.clone()));
        assert_eq!(state(&dir.0, REPO, FILES), ModelState::Complete(adopted));
    }

    #[test]
    fn revisions_are_read_from_hugging_face_cache_paths() {
        let path = Path::new("/c/models--org--m/snapshots/0123abcd/model.safetensors");
        assert_eq!(revision_of_cached_file(path).as_deref(), Some("0123abcd"));
        assert_eq!(
            revision_of_cached_file(Path::new("/c/blobs/0123/model.safetensors")),
            None
        );
        assert_eq!(
            revision_of_cached_file(Path::new("/c/snapshots/../x/model.safetensors")),
            None
        );
    }
}
