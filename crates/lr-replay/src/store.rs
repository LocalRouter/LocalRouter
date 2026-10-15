//! Explicit, atomic local persistence; reads never start or resume inference.
use crate::{source::validate_cases, types::*};
use chrono::Utc;
use parking_lot::Mutex;
use serde::Deserialize;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;
const MAX_FILE: u64 = 32 * 1024 * 1024;
const MAX_TOTAL: u64 = 256 * 1024 * 1024;
const MAX_FILES: usize = 128;
pub struct ReplayStore {
    root: PathBuf,
    lock: Mutex<()>,
}
impl ReplayStore {
    /// Construction does not create directories or persist payloads.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            lock: Mutex::new(()),
        }
    }
    fn directory(&self, kind: ArtifactKind) -> PathBuf {
        self.root.join(match kind {
            ArtifactKind::Dataset => "datasets",
            ArtifactKind::Run => "runs",
        })
    }
    fn check_directory(&self, kind: ArtifactKind) -> Result<(), String> {
        for path in [&self.root, &self.directory(kind)] {
            match fs::symlink_metadata(path) {
                Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
                    return Err("Experiment directories must be real local directories".into())
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(())
    }
    fn path(&self, kind: ArtifactKind, id: &str) -> Result<PathBuf, String> {
        let id = Uuid::parse_str(id).map_err(|_| "Invalid artifact id")?;
        Ok(self.directory(kind).join(format!("{id}.json")))
    }
    fn read_unlocked(&self, kind: ArtifactKind, id: &str) -> Result<SavedArtifact, String> {
        self.check_directory(kind)?;
        let path = self.path(kind, id)?;
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_FILE {
            return Err("Artifact is not a regular bounded file".into());
        }
        let mut bytes = Vec::new();
        fs::File::open(&path)
            .map_err(|e| e.to_string())?
            .take(MAX_FILE + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_FILE {
            return Err("Artifact exceeds its size limit".into());
        }
        let mut artifact: SavedArtifact =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if artifact.schema_version != 1
            || artifact.summary.kind != kind
            || Uuid::parse_str(&artifact.summary.id).ok() != Uuid::parse_str(id).ok()
        {
            return Err("Artifact schema, ownership id or kind is invalid".into());
        }
        if kind == ArtifactKind::Run {
            let mut run: ReplayRun =
                serde_json::from_value(artifact.value).map_err(|e| e.to_string())?;
            if run.id != artifact.summary.id || run.revision != artifact.summary.revision {
                return Err("Run payload does not match its artifact header".into());
            }
            validate_cases(&run.cases)?;
            run.mark_interrupted();
            artifact.value = serde_json::to_value(run).map_err(|e| e.to_string())?;
        } else {
            let dataset: ReplayDataset =
                serde_json::from_value(artifact.value.clone()).map_err(|e| e.to_string())?;
            if dataset.id != artifact.summary.id || dataset.revision != artifact.summary.revision {
                return Err("Dataset payload does not match its artifact header".into());
            }
            validate_cases(&dataset.cases)?;
        }
        Ok(artifact)
    }
    pub fn read(&self, kind: ArtifactKind, id: &str) -> Result<SavedArtifact, String> {
        let _guard = self.lock.lock();
        self.read_unlocked(kind, id)
    }
    pub fn list(&self) -> Result<ArtifactList, String> {
        #[derive(Deserialize)]
        struct Header {
            schema_version: u32,
            summary: ArtifactSummary,
        }
        let _guard = self.lock.lock();
        let mut result = ArtifactList {
            items: vec![],
            issues: vec![],
        };
        for kind in [ArtifactKind::Dataset, ArtifactKind::Run] {
            self.check_directory(kind)?;
            let directory = self.directory(kind);
            let entries = match fs::read_dir(&directory) {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.to_string()),
            };
            for entry in entries.take(MAX_FILES + 1) {
                let entry = entry.map_err(|e| e.to_string())?;
                let id = entry
                    .path()
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let header = (|| -> Result<Header, String> {
                    let metadata = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
                    if !metadata.is_file()
                        || metadata.file_type().is_symlink()
                        || metadata.len() > MAX_FILE
                    {
                        return Err("Not a regular bounded artifact".into());
                    }
                    let file = fs::File::open(entry.path()).map_err(|e| e.to_string())?;
                    let header: Header = serde_json::from_reader(file.take(MAX_FILE + 1))
                        .map_err(|e| e.to_string())?;
                    if header.schema_version != 1
                        || header.summary.kind != kind
                        || Uuid::parse_str(&id).ok() != Uuid::parse_str(&header.summary.id).ok()
                        || Uuid::parse_str(&id).is_err()
                    {
                        return Err("Invalid schema, id or artifact kind".into());
                    }
                    Ok(header)
                })();
                match header {
                    Ok(header) => result.items.push(header.summary),
                    Err(error) => result.issues.push(ArtifactIssue { id, kind, error }),
                }
            }
        }
        result
            .items
            .sort_by_key(|item| std::cmp::Reverse(item.saved_at));
        Ok(result)
    }
    fn save(&self, artifact: &SavedArtifact, expected_revision: u64) -> Result<(), String> {
        let _guard = self.lock.lock();
        let kind = artifact.summary.kind;
        let id = &artifact.summary.id;
        self.check_directory(kind)?;
        let path = self.path(kind, id)?;
        let old_revision = match fs::symlink_metadata(&path) {
            Ok(_) => self.read_unlocked(kind, id)?.summary.revision,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(e) => return Err(e.to_string()),
        };
        if old_revision != expected_revision {
            return Err("Artifact changed or was deleted; reload before saving".into());
        }
        let bytes = serde_json::to_vec_pretty(artifact).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_FILE {
            return Err("Saved artifact exceeds 32 MiB".into());
        }
        let mut total = 0u64;
        let mut count = 0;
        for kind in [ArtifactKind::Dataset, ArtifactKind::Run] {
            self.check_directory(kind)?;
            if let Ok(entries) = fs::read_dir(self.directory(kind)) {
                for entry in entries {
                    let entry = entry.map_err(|e| e.to_string())?;
                    let meta = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
                    if meta.is_file() && entry.path() != path {
                        total = total.saturating_add(meta.len());
                        count += 1;
                    }
                }
            }
        }
        if total.saturating_add(bytes.len() as u64) > MAX_TOTAL || count >= MAX_FILES {
            return Err("Experiment storage is full (128 artifacts or 256 MiB). Delete or export saved data first".into());
        }
        let directory = self.directory(kind);
        create_private_dir(&directory)?;
        let root = fs::canonicalize(&self.root).map_err(|e| e.to_string())?;
        if !fs::canonicalize(&directory)
            .map_err(|e| e.to_string())?
            .starts_with(root)
        {
            return Err("Artifact directory escapes experiment storage".into());
        }
        let mut temporary =
            tempfile::NamedTempFile::new_in(&directory).map_err(|e| e.to_string())?;
        temporary.write_all(&bytes).map_err(|e| e.to_string())?;
        temporary.as_file().sync_all().map_err(|e| e.to_string())?;
        temporary.persist(path).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn save_dataset(&self, mut dataset: ReplayDataset) -> Result<SavedArtifact, String> {
        for case in &mut dataset.cases {
            case.input = crate::source::sanitize_input(&case.input)?;
        }
        validate_cases(&dataset.cases)?;
        validate_name(&dataset.name)?;
        let revision = dataset.revision;
        dataset.revision = revision.checked_add(1).ok_or("Revision overflow")?;
        let saved_at = Utc::now();
        dataset.saved_at = Some(saved_at);
        let artifact = SavedArtifact {
            schema_version: 1,
            summary: ArtifactSummary {
                id: dataset.id.clone(),
                name: dataset.name.clone(),
                kind: ArtifactKind::Dataset,
                revision: dataset.revision,
                saved_at,
                case_count: dataset.cases.len(),
                result_count: 0,
                client_id: None,
            },
            value: serde_json::to_value(dataset).map_err(|e| e.to_string())?,
        };
        self.save(&artifact, revision)?;
        Ok(artifact)
    }
    pub fn save_run(&self, mut run: ReplayRun) -> Result<SavedArtifact, String> {
        for case in &mut run.cases {
            case.input = crate::source::sanitize_input(&case.input)?;
        }
        validate_cases(&run.cases)?;
        validate_name(&run.name)?;
        let revision = run.revision;
        run.revision = revision.checked_add(1).ok_or("Revision overflow")?;
        let artifact = SavedArtifact {
            schema_version: 1,
            summary: ArtifactSummary {
                id: run.id.clone(),
                name: run.name.clone(),
                kind: ArtifactKind::Run,
                revision: run.revision,
                saved_at: Utc::now(),
                case_count: run.cases.len(),
                result_count: run.results.len(),
                client_id: Some(run.client_id.clone()),
            },
            value: serde_json::to_value(run).map_err(|e| e.to_string())?,
        };
        self.save(&artifact, revision)?;
        Ok(artifact)
    }
    pub fn delete(
        &self,
        kind: ArtifactKind,
        id: &str,
        expected_revision: u64,
    ) -> Result<(), String> {
        let _guard = self.lock.lock();
        if self.read_unlocked(kind, id)?.summary.revision != expected_revision {
            return Err("Artifact changed; reload before deleting".into());
        }
        fs::remove_file(self.path(kind, id)?).map_err(|e| e.to_string())
    }
}
fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > 128 {
        Err("Name must contain 1–128 bytes".into())
    } else {
        Ok(())
    }
}
fn create_private_dir(path: &Path) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn dataset() -> ReplayDataset {
        let mut d = ReplayDataset::empty();
        d.cases = vec![ReplayCase {
            id: "case".into(),
            name: "Case".into(),
            input: ReplayInput {
                endpoint: ReplayEndpoint::ChatCompletions,
                body: json!({"messages":[{"role":"user","content":"private input"}],"api_key":"discard-me"}),
                source_event_id: None,
            },
            checks: vec![],
        }];
        d
    }
    #[test]
    fn construction_and_listing_do_not_persist_payloads() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("replay");
        let store = ReplayStore::new(root.clone());
        assert!(store.list().unwrap().items.is_empty());
        assert!(!root.exists());
    }
    #[test]
    fn atomic_revisions_strip_credentials_and_do_not_resurrect_deleted_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let store = ReplayStore::new(dir.path().join("replay"));
        let original = dataset();
        let a = store.save_dataset(original.clone()).unwrap();
        assert_eq!(a.summary.revision, 1);
        assert!(!serde_json::to_string(&a).unwrap().contains("discard-me"));
        assert!(store.save_dataset(original).is_err());
        let d: ReplayDataset = serde_json::from_value(a.value).unwrap();
        assert!(store.delete(ArtifactKind::Dataset, &d.id, 0).is_err());
        store.delete(ArtifactKind::Dataset, &d.id, 1).unwrap();
        assert!(store.save_dataset(d).is_err());
    }
    #[test]
    fn corrupt_headers_and_payloads_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let store = ReplayStore::new(dir.path().join("replay"));
        let a = store.save_dataset(dataset()).unwrap();
        let path = store.path(ArtifactKind::Dataset, &a.summary.id).unwrap();
        let mut bad = a.clone();
        bad.value["id"] = json!(Uuid::new_v4().to_string());
        fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
        assert!(store.read(ArtifactKind::Dataset, &a.summary.id).is_err());
        fs::write(path, b"broken").unwrap();
        assert_eq!(store.list().unwrap().issues.len(), 1);
        assert!(store.read(ArtifactKind::Dataset, "../../secrets").is_err());
    }
    #[cfg(unix)]
    #[test]
    fn private_files_and_symlink_directories_are_rejected() {
        use std::os::unix::{fs::symlink, fs::PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("replay");
        let store = ReplayStore::new(root.clone());
        let a = store.save_dataset(dataset()).unwrap();
        assert_eq!(
            fs::metadata(store.path(ArtifactKind::Dataset, &a.summary.id).unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
        fs::remove_dir_all(root.join("datasets")).unwrap();
        let external = dir.path().join("external");
        fs::create_dir(&external).unwrap();
        symlink(&external, root.join("datasets")).unwrap();
        assert!(store.list().is_err());
        assert!(store.save_dataset(dataset()).is_err());
    }
}
