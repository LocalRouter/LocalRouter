//! Local, owner-scoped payload storage. No network access or provider credentials.
mod artifacts;
mod billing;
pub use billing::*;
mod jobs;
pub use artifacts::*;
pub use jobs::*;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    path::PathBuf,
    sync::Arc,
};
use utoipa::ToSchema;
use uuid::Uuid;

pub const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_ROW_BYTES: usize = 1024 * 1024;
pub const MAX_ROWS: usize = 50_000;
pub const DEFAULT_RETENTION: i64 = 30 * 24 * 3600;
const CLIENT_BYTES: i64 = 128 * 1024 * 1024;
const GLOBAL_BYTES: i64 = 512 * 1024 * 1024;

#[derive(Debug)]
pub enum StoreError {
    Invalid(String),
    NotFound,
    Conflict,
    Quota,
    Storage(String),
}
pub type Result<T> = std::result::Result<T, StoreError>;
impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.to_string())
    }
}
impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        Self::Storage(e.to_string())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct FileObject {
    pub id: String,
    pub object: String,
    pub bytes: i64,
    pub created_at: i64,
    pub expires_at: i64,
    pub filename: String,
    pub purpose: String,
    pub status: String,
}
#[derive(Serialize, ToSchema)]
pub struct FileList {
    pub object: String,
    pub data: Vec<FileObject>,
    pub first_id: Option<String>,
    pub last_id: Option<String>,
    pub has_more: bool,
}
#[derive(Serialize, ToSchema)]
pub struct FileDeleted {
    pub id: String,
    pub object: String,
    pub deleted: bool,
}

/// Clones share an in-process lock; SQLite immediate transactions also serialize
/// independent store instances. Opening the app never creates payload storage.
#[derive(Clone)]
pub struct BatchStore {
    root: PathBuf,
    lock: Arc<Mutex<()>>,
}
impl BatchStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            lock: Arc::new(Mutex::new(())),
        }
    }

    fn open(&self) -> Result<Connection> {
        if !self.root.exists() {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&self.root)?;
        }
        let metadata = fs::symlink_metadata(&self.root)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StoreError::Storage(
                "Payload directory must be a real directory".into(),
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))?;
        }
        let path = self.root.join("batches.sqlite3");
        let mut created_database = false;
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
                return Err(StoreError::Storage(
                    "Payload database must be a regular file".into(),
                ))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                match options.open(&path) {
                    Ok(file) => {
                        file.sync_all()?;
                        created_database = true;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        let meta = fs::symlink_metadata(&path)?;
                        if meta.file_type().is_symlink() || !meta.is_file() {
                            return Err(StoreError::Storage("Unsafe database path".into()));
                        }
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            Err(e) => return Err(e.into()),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        #[cfg(unix)]
        if created_database {
            // Persist the new file and directory entries before a later
            // mutation intent can be acknowledged as durable.
            std::fs::File::open(&self.root)?.sync_all()?;
            if let Some(parent) = self.root.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::File::open(parent)?.sync_all()?;
            }
        }
        #[cfg(not(unix))]
        let _ = created_database;
        // Never follow SQLite sidecar symlinks, including a stale WAL from a
        // previous process. The private directory excludes other OS users.
        for suffix in ["-journal", "-wal", "-shm"] {
            let sidecar = self.root.join(format!("batches.sqlite3{suffix}"));
            match fs::symlink_metadata(sidecar) {
                Ok(m) if m.file_type().is_symlink() || !m.is_file() => {
                    return Err(StoreError::Storage("Unsafe database sidecar".into()))
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        let connection = Connection::open(&path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        let _: String = connection.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))?;
        let _: i64 = connection.query_row("PRAGMA secure_delete=ON", [], |r| r.get(0))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS files (
                id TEXT PRIMARY KEY, owner TEXT NOT NULL, filename TEXT NOT NULL,
                purpose TEXT NOT NULL, created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL, bytes INTEGER NOT NULL, content BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS files_owner ON files(owner, created_at, id);
            CREATE TABLE IF NOT EXISTS file_references (
                file_id TEXT NOT NULL REFERENCES files(id) ON DELETE RESTRICT,
                job_id TEXT NOT NULL, PRIMARY KEY(file_id,job_id)
            );",
        )?;
        jobs::initialize(&connection)?;
        Ok(connection)
    }

    fn expire(connection: &Connection, now: i64) -> Result<()> {
        connection.execute("DELETE FROM files WHERE expires_at <= ?1 AND NOT EXISTS (SELECT 1 FROM file_references r WHERE r.file_id=files.id)", [now])?;
        Ok(())
    }

    pub fn upload(
        &self,
        owner: &str,
        filename: &str,
        purpose: &str,
        content: &[u8],
        retention: i64,
    ) -> Result<FileObject> {
        if owner.is_empty() || owner.len() > 256 {
            return Err(StoreError::Invalid("Invalid owner".into()));
        }
        if purpose != "batch" {
            return Err(StoreError::Invalid(
                "Only purpose=batch is supported".into(),
            ));
        }
        if filename.is_empty() || filename.len() > 255 || filename.chars().any(char::is_control) {
            return Err(StoreError::Invalid(
                "Filename must be 1..255 bytes without control characters".into(),
            ));
        }
        if !(3600..=DEFAULT_RETENTION).contains(&retention) {
            return Err(StoreError::Invalid(
                "Retention must be 3600..2592000 seconds".into(),
            ));
        }
        validate_jsonl(content)?;
        let _lock = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = chrono::Utc::now().timestamp();
        Self::expire(&tx, now)?;
        let (client_count, client_bytes) = artifacts::capacity(&tx, Some(owner))?;
        let (count, bytes) = artifacts::capacity(&tx, None)?;
        if client_count >= 128
            || count >= 512
            || client_bytes + content.len() as i64 > CLIENT_BYTES
            || bytes + content.len() as i64 > GLOBAL_BYTES
        {
            return Err(StoreError::Quota);
        }
        let file = FileObject {
            id: format!("file-lr-{}", Uuid::new_v4().simple()),
            object: "file".into(),
            bytes: content.len() as i64,
            created_at: now,
            expires_at: now + retention,
            filename: filename.into(),
            purpose: purpose.into(),
            status: "processed".into(),
        };
        tx.execute(
            "INSERT INTO files VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                file.id,
                owner,
                filename,
                purpose,
                now,
                file.expires_at,
                file.bytes,
                content
            ],
        )?;
        tx.commit()?;
        Ok(file)
    }

    pub fn get(&self, owner: &str, id: &str) -> Result<FileObject> {
        check_id(id)?;
        let _lock = self.lock.lock();
        let connection = self.open()?;
        Self::expire(&connection, chrono::Utc::now().timestamp())?;
        lookup(&connection, owner, id)
    }

    pub fn content(&self, owner: &str, id: &str) -> Result<Vec<u8>> {
        check_id(id)?;
        let _lock = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::expire(&tx, chrono::Utc::now().timestamp())?;
        let file = lookup(&tx, owner, id)?;
        let content: Vec<u8> = tx.query_row(
            "SELECT content FROM files WHERE owner=?1 AND id=?2",
            params![owner, id],
            |r| r.get(0),
        )?;
        let limit = if file.purpose == "batch_output" {
            artifacts::MAX_RESULT_BYTES
        } else {
            MAX_FILE_BYTES
        };
        if content.len() > limit {
            return Err(StoreError::Storage(
                "Stored file exceeds supported bound".into(),
            ));
        }
        tx.commit()?;
        Ok(content)
    }

    pub fn delete(&self, owner: &str, id: &str) -> Result<FileDeleted> {
        check_id(id)?;
        let _lock = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::expire(&tx, chrono::Utc::now().timestamp())?;
        lookup(&tx, owner, id)?;
        let referenced: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM file_references WHERE file_id=?1)",
            [id],
            |r| r.get(0),
        )?;
        if referenced {
            return Err(StoreError::Conflict);
        }
        tx.execute(
            "DELETE FROM files WHERE id=?1 AND owner=?2",
            params![id, owner],
        )?;
        tx.commit()?;
        Ok(FileDeleted {
            id: id.into(),
            object: "file".into(),
            deleted: true,
        })
    }

    pub fn list(
        &self,
        owner: &str,
        purpose: Option<&str>,
        after: Option<&str>,
        limit: usize,
        descending: bool,
    ) -> Result<FileList> {
        if !(1..=100).contains(&limit) {
            return Err(StoreError::Invalid("limit must be 1..100".into()));
        }
        if purpose.is_some_and(|p| !["batch", "batch_output"].contains(&p)) {
            return Err(StoreError::Invalid(
                "Only batch and batch_output file lists are supported".into(),
            ));
        }
        let _lock = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::expire(&tx, chrono::Utc::now().timestamp())?;
        let cursor = after
            .map(|id| {
                check_id(id)?;
                lookup(&tx, owner, id)
            })
            .transpose()?;
        let order = if descending { "DESC" } else { "ASC" };
        let compare = if descending { "<" } else { ">" };
        let sql = format!("SELECT id,filename,purpose,created_at,expires_at,bytes FROM files WHERE owner=?1 AND (?5 IS NULL OR purpose=?5) AND (?2 IS NULL OR (created_at,id) {compare} (?2,?3)) ORDER BY created_at {order},id {order} LIMIT ?4");
        let mut statement = tx.prepare(&sql)?;
        let mut data = statement
            .query_map(
                params![
                    owner,
                    cursor.as_ref().map(|f| f.created_at),
                    after,
                    (limit + 1) as i64,
                    purpose
                ],
                file_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let has_more = data.len() > limit;
        data.truncate(limit);
        let result = FileList {
            object: "list".into(),
            first_id: data.first().map(|f| f.id.clone()),
            last_id: data.last().map(|f| f.id.clone()),
            data,
            has_more,
        };
        drop(statement);
        tx.commit()?;
        Ok(result)
    }
}

fn check_id(id: &str) -> Result<()> {
    if !id
        .strip_prefix("file-lr-")
        .is_some_and(|v| v.len() == 32 && v.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(StoreError::NotFound);
    }
    Ok(())
}
fn file_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileObject> {
    Ok(FileObject {
        id: row.get(0)?,
        filename: row.get(1)?,
        purpose: row.get(2)?,
        created_at: row.get(3)?,
        expires_at: row.get(4)?,
        bytes: row.get(5)?,
        object: "file".into(),
        status: "processed".into(),
    })
}
fn lookup(connection: &Connection, owner: &str, id: &str) -> Result<FileObject> {
    connection.query_row("SELECT id,filename,purpose,created_at,expires_at,bytes FROM files WHERE owner=?1 AND id=?2", params![owner,id], file_row).optional()?.ok_or(StoreError::NotFound)
}

/// Structural validation only. Per-row inference policy is applied later at
/// batch submission; a processed local file is never an inference approval.
pub fn validate_jsonl(content: &[u8]) -> Result<usize> {
    if content.is_empty() || content.len() > MAX_FILE_BYTES {
        return Err(StoreError::Invalid(
            "File must contain 1..16777216 bytes".into(),
        ));
    }
    let text = std::str::from_utf8(content)
        .map_err(|_| StoreError::Invalid("Batch input must be UTF-8 JSONL".into()))?;
    let mut count = 0;
    for line in text.lines() {
        if line.trim().is_empty() || line.len() > MAX_ROW_BYTES {
            return Err(StoreError::Invalid(
                "Each JSONL row must contain 1..1048576 bytes".into(),
            ));
        }
        let row: serde_json::Value = serde_json::from_str(line)
            .map_err(|_| StoreError::Invalid(format!("Invalid JSON on row {}", count + 1)))?;
        if !row.is_object() {
            return Err(StoreError::Invalid(format!(
                "Row {} must be an object",
                count + 1
            )));
        }
        count += 1;
        if count > MAX_ROWS {
            return Err(StoreError::Invalid(
                "At most 50000 rows are supported".into(),
            ));
        }
    }
    if count == 0 {
        return Err(StoreError::Invalid(
            "Batch input must contain a JSON object".into(),
        ));
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, BatchStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = BatchStore::new(dir.path().join("payloads"));
        (dir, store)
    }
    fn upload(store: &BatchStore, owner: &str) -> FileObject {
        store
            .upload(
                owner,
                "../display-only.jsonl",
                "batch",
                b"{\"custom_id\":\"one\"}\n",
                3600,
            )
            .unwrap()
    }
    #[test]
    fn lazy_private_restart_and_owner_isolation() {
        let (dir, store) = fixture();
        assert!(!store.root.exists());
        let file = upload(&store, "alice");
        assert!(!dir.path().join("display-only.jsonl").exists());
        let restarted = BatchStore::new(store.root.clone());
        assert_eq!(
            restarted.content("alice", &file.id).unwrap(),
            b"{\"custom_id\":\"one\"}\n"
        );
        for result in [
            restarted.get("bob", &file.id).map(|_| ()),
            restarted.content("bob", &file.id).map(|_| ()),
            restarted.delete("bob", &file.id).map(|_| ()),
        ] {
            assert!(matches!(result, Err(StoreError::NotFound)));
        }
        assert!(restarted
            .list("bob", None, None, 20, true)
            .unwrap()
            .data
            .is_empty());
        assert!(matches!(
            restarted.list("bob", None, Some(&file.id), 20, true),
            Err(StoreError::NotFound)
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&store.root).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(store.root.join("batches.sqlite3"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        restarted.delete("alice", &file.id).unwrap();
        assert!(matches!(
            store.get("alice", &file.id),
            Err(StoreError::NotFound)
        ));
    }
    #[test]
    fn rejects_invalid_jsonl_and_limits_before_creating_storage() {
        let (_dir, store) = fixture();
        for bytes in [
            b"".as_slice(),
            b"\xff",
            b"{}\n\n{}",
            b"null\n",
            b"[]\n",
            b"{broken}",
            b"\xef\xbb\xbf{}",
        ] {
            assert!(validate_jsonl(bytes).is_err(), "{bytes:?}");
        }
        assert_eq!(validate_jsonl(b"{}\r\n{}\n").unwrap(), 2);
        assert!(validate_jsonl(&vec![b' '; MAX_FILE_BYTES + 1]).is_err());
        let oversized_row = format!("{{\"text\":\"{}\"}}", "x".repeat(MAX_ROW_BYTES));
        assert!(validate_jsonl(oversized_row.as_bytes()).is_err());
        assert!(validate_jsonl("{}\n".repeat(MAX_ROWS + 1).as_bytes()).is_err());
        assert!(store
            .upload("alice", "x", "assistants", b"{}", 3600)
            .is_err());
        assert!(store.upload("alice", "x\n", "batch", b"{}", 3600).is_err());
        assert!(store.upload("alice", "x", "batch", b"{}", 3599).is_err());
        assert!(!store.root.exists());
    }
    #[test]
    fn stable_cursor_pagination_in_both_directions() {
        let (_dir, store) = fixture();
        for _ in 0..5 {
            upload(&store, "alice");
        }
        upload(&store, "bob");
        for descending in [true, false] {
            let all = store
                .list("alice", Some("batch"), None, 100, descending)
                .unwrap();
            let first = store.list("alice", None, None, 2, descending).unwrap();
            assert!(first.has_more);
            let second = store
                .list("alice", None, first.last_id.as_deref(), 2, descending)
                .unwrap();
            let third = store
                .list("alice", None, second.last_id.as_deref(), 2, descending)
                .unwrap();
            assert!(!third.has_more);
            let paged = first
                .data
                .into_iter()
                .chain(second.data)
                .chain(third.data)
                .map(|f| f.id)
                .collect::<Vec<_>>();
            assert_eq!(
                paged,
                all.data.into_iter().map(|f| f.id).collect::<Vec<_>>()
            );
        }
        assert!(store.list("alice", None, None, 0, true).is_err());
        assert!(matches!(
            store.get("alice", "../../elsewhere"),
            Err(StoreError::NotFound)
        ));
    }
    #[test]
    fn expiry_preserves_referenced_inputs_until_release() {
        let (_dir, store) = fixture();
        let file = upload(&store, "alice");
        let connection = store.open().unwrap();
        connection
            .execute(
                "INSERT INTO file_references VALUES (?1,'job-fixture')",
                [&file.id],
            )
            .unwrap();
        connection
            .execute("UPDATE files SET expires_at=0 WHERE id=?1", [&file.id])
            .unwrap();
        assert!(matches!(
            store.delete("alice", &file.id),
            Err(StoreError::Conflict)
        ));
        assert!(store.get("alice", &file.id).is_ok());
        connection
            .execute("DELETE FROM file_references", [])
            .unwrap();
        assert!(matches!(
            store.get("alice", &file.id),
            Err(StoreError::NotFound)
        ));
        assert!(store
            .list("alice", None, None, 20, true)
            .unwrap()
            .data
            .is_empty());
    }
    #[test]
    fn quotas_are_checked_transactionally_across_store_instances() {
        let (_dir, store) = fixture();
        let seed = upload(&store, "alice");
        let connection = store.open().unwrap();
        // Metadata expansion exercises quota arithmetic without large fixtures.
        connection
            .execute(
                "UPDATE files SET bytes=?1 WHERE id=?2",
                params![CLIENT_BYTES, seed.id],
            )
            .unwrap();
        let second = BatchStore::new(store.root.clone());
        assert!(matches!(
            second.upload("alice", "x", "batch", b"{}", 3600),
            Err(StoreError::Quota)
        ));
        assert!(second.upload("bob", "x", "batch", b"{}", 3600).is_ok());
        connection
            .execute(
                "UPDATE files SET bytes=?1 WHERE id=?2",
                params![GLOBAL_BYTES, seed.id],
            )
            .unwrap();
        assert!(matches!(
            second.upload("carol", "x", "batch", b"{}", 3600),
            Err(StoreError::Quota)
        ));
    }
    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_directory_database_and_journal() {
        use std::os::unix::fs::symlink;
        let (dir, store) = fixture();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        symlink(&target, &store.root).unwrap();
        assert!(store.list("alice", None, None, 20, true).is_err());
        fs::remove_file(&store.root).unwrap();
        fs::create_dir(&store.root).unwrap();
        let other = dir.path().join("other");
        fs::write(&other, b"private original").unwrap();
        let db = store.root.join("batches.sqlite3");
        symlink(&other, &db).unwrap();
        assert!(store.list("alice", None, None, 20, true).is_err());
        fs::remove_file(db).unwrap();
        let journal = store.root.join("batches.sqlite3-journal");
        symlink(&other, journal).unwrap();
        assert!(store.list("alice", None, None, 20, true).is_err());
        assert_eq!(fs::read(other).unwrap(), b"private original");
    }
}
