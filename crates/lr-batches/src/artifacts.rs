//! Bounded owned native result aliases and reserved local output capacity.
use super::jobs::load;
use super::*;
use sha2::{Digest, Sha256};
pub const MAX_RESULT_BYTES: usize = 32 * 1024 * 1024;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BatchResultKind {
    Output,
    Error,
}
impl BatchResultKind {
    fn label(self) -> &'static str {
        match self {
            Self::Output => "output",
            Self::Error => "error",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct BatchResultAlias {
    pub kind: BatchResultKind,
    pub file_id: Option<String>,
    pub fingerprint: String,
}

pub(super) fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS batch_storage_reservations (
        job_id TEXT PRIMARY KEY REFERENCES batch_jobs(id), owner TEXT NOT NULL, bytes INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS batch_artifacts (
        job_id TEXT NOT NULL REFERENCES batch_jobs(id), kind TEXT NOT NULL,
        file_id TEXT REFERENCES files(id) ON DELETE SET NULL, fingerprint TEXT NOT NULL,
        PRIMARY KEY(job_id,kind)
    );")?;
    Ok(())
}
/// Include unused reserved bytes and slots in all public upload quota checks.
pub(super) fn capacity(connection: &Connection, owner: Option<&str>) -> Result<(i64, i64)> {
    let (count, bytes): (i64, i64) = connection.query_row(
        "SELECT count(*),coalesce(sum(bytes),0) FROM files WHERE (?1 IS NULL OR owner=?1)",
        [owner],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (slots,reserved):(i64,i64)=connection.query_row("SELECT coalesce(sum(max(0,2-(SELECT count(*) FROM batch_artifacts a JOIN files f ON f.id=a.file_id WHERE a.job_id=r.job_id))),0),coalesce(sum(max(0,r.bytes-(SELECT coalesce(sum(f.bytes),0) FROM batch_artifacts a JOIN files f ON f.id=a.file_id WHERE a.job_id=r.job_id))),0) FROM batch_storage_reservations r WHERE (?1 IS NULL OR r.owner=?1)",[owner],|r|Ok((r.get(0)?,r.get(1)?)))?;
    let edits:i64=connection.query_row("SELECT coalesce(sum(e.bytes),0) FROM batch_row_edits e JOIN batch_jobs j ON j.id=e.job_id WHERE (?1 IS NULL OR j.owner=?1)",[owner],|r|r.get(0))?;
    Ok((count + slots, bytes + reserved + edits))
}
impl BatchStore {
    /// Explicit terminal payload deletion preserves all usage/reservations and
    /// identity evidence. Incomplete accounting must never become free usage.
    pub fn purge_batch_payloads(&self, owner: &str, id: &str) -> Result<()> {
        let _guard = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = load(&tx, owner, id)?;
        if !job.status.is_terminal() {
            return Err(StoreError::Conflict);
        }
        tx.execute("DELETE FROM file_references WHERE job_id=?1", [id])?;
        tx.execute("DELETE FROM files WHERE owner=?1 AND id IN (SELECT file_id FROM batch_artifacts WHERE job_id=?2 UNION SELECT ?3) AND NOT EXISTS(SELECT 1 FROM file_references r WHERE r.file_id=files.id)",params![owner,id,job.prepared_snapshot_file_id])?;
        tx.execute("DELETE FROM batch_row_edits WHERE job_id=?1", [id])?;
        tx.execute(
            "DELETE FROM batch_storage_reservations WHERE job_id=?1",
            [id],
        )?;
        job.cleanup
            .insert("local_payloads".into(), "deleted".into());
        super::jobs::save(&tx, &mut job)?;
        tx.commit()?;
        Ok(())
    }

    pub fn reserve_result_storage(&self, owner: &str, id: &str) -> Result<()> {
        let _guard = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = load(&tx, owner, id)?;
        Self::expire(&tx, chrono::Utc::now().timestamp())?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM batch_storage_reservations WHERE job_id=?1)",
            [id],
            |r| r.get(0),
        )?;
        if exists {
            tx.commit()?;
            return Ok(());
        }
        let (owned_count, owned_bytes) = capacity(&tx, Some(owner))?;
        let (count, bytes) = capacity(&tx, None)?;
        let (artifacts,used):(i64,i64)=tx.query_row("SELECT count(*),coalesce(sum(f.bytes),0) FROM batch_artifacts a JOIN files f ON f.id=a.file_id WHERE a.job_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let remaining = (job.options.result_limit_bytes as i64 - used).max(0);
        let slots = (2 - artifacts).max(0);
        if owned_count + slots > 128
            || count + slots > 512
            || owned_bytes + remaining > CLIENT_BYTES
            || bytes + remaining > GLOBAL_BYTES
        {
            return Err(StoreError::Quota);
        }
        tx.execute(
            "INSERT INTO batch_storage_reservations VALUES (?1,?2,?3)",
            params![id, owner, job.options.result_limit_bytes],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn release_result_storage(&self, owner: &str, id: &str) -> Result<()> {
        let _guard = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = load(&tx, owner, id)?;
        if !job.status.is_terminal() {
            return Err(StoreError::Conflict);
        }
        tx.execute(
            "DELETE FROM batch_storage_reservations WHERE job_id=?1",
            [id],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn store_batch_result(
        &self,
        owner: &str,
        id: &str,
        kind: BatchResultKind,
        content: &[u8],
    ) -> Result<FileObject> {
        if content.len() > MAX_RESULT_BYTES {
            return Err(StoreError::Invalid(
                "Native results are limited to 32 MiB per job".into(),
            ));
        }
        let fingerprint = format!("{:x}", Sha256::digest(content));
        let _guard = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = load(&tx, owner, id)?;
        if !job.status.is_terminal() || job.remote_batch_id.is_none() {
            return Err(StoreError::Conflict);
        }
        let existing: Option<(Option<String>, String)> = tx
            .query_row(
                "SELECT file_id,fingerprint FROM batch_artifacts WHERE job_id=?1 AND kind=?2",
                params![id, kind.label()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((file_id, known)) = existing {
            if known != fingerprint {
                return Err(StoreError::Conflict);
            }
            if let Some(file_id) = file_id {
                let file = lookup(&tx, owner, &file_id)?;
                tx.commit()?;
                return Ok(file);
            }
        }
        let reserved: i64 = tx
            .query_row(
                "SELECT bytes FROM batch_storage_reservations WHERE job_id=?1 AND owner=?2",
                params![id, owner],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(StoreError::Quota)?;
        let used:i64=tx.query_row("SELECT coalesce(sum(f.bytes),0) FROM batch_artifacts a JOIN files f ON f.id=a.file_id WHERE a.job_id=?1",[id],|r|r.get(0))?;
        if content.len() as i64 > reserved - used {
            return Err(StoreError::Quota);
        }
        let now = chrono::Utc::now().timestamp();
        let file = FileObject {
            id: format!("file-lr-{}", Uuid::new_v4().simple()),
            object: "file".into(),
            bytes: content.len() as i64,
            created_at: now,
            expires_at: now + job.options.output_retention_seconds as i64,
            filename: format!("{id}-{}.jsonl", kind.label()),
            purpose: "batch_output".into(),
            status: "processed".into(),
        };
        tx.execute(
            "INSERT INTO files VALUES (?1,?2,?3,'batch_output',?4,?5,?6,?7)",
            params![
                file.id,
                owner,
                file.filename,
                now,
                file.expires_at,
                file.bytes,
                content
            ],
        )?;
        tx.execute("INSERT INTO batch_artifacts VALUES (?1,?2,?3,?4) ON CONFLICT(job_id,kind) DO UPDATE SET file_id=excluded.file_id",params![id,kind.label(),file.id,fingerprint])?;
        if !job.accounting_settled {
            tx.execute(
                "INSERT INTO file_references VALUES (?1,?2)",
                params![file.id, id],
            )?;
        }
        tx.commit()?;
        Ok(file)
    }
    pub fn batch_results(&self, owner: &str, id: &str) -> Result<Vec<BatchResultAlias>> {
        let _guard = self.lock.lock();
        let connection = self.open()?;
        load(&connection, owner, id)?;
        let mut statement = connection.prepare(
            "SELECT kind,file_id,fingerprint FROM batch_artifacts WHERE job_id=?1 ORDER BY kind",
        )?;
        let rows = statement
            .query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(kind, file_id, fingerprint)| {
                Ok(BatchResultAlias {
                    kind: match kind.as_str() {
                        "output" => BatchResultKind::Output,
                        "error" => BatchResultKind::Error,
                        _ => return Err(StoreError::Storage("Invalid result kind".into())),
                    },
                    file_id,
                    fingerprint,
                })
            })
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn fixture(limit: u64) -> (tempfile::TempDir, BatchStore, BatchJob) {
        let dir = tempfile::tempdir().unwrap();
        let store = BatchStore::new(dir.path().join("store"));
        let body=b"{\"custom_id\":\"one\",\"method\":\"POST\",\"url\":\"/v1/chat/completions\",\"body\":{\"model\":\"fixture-model\",\"messages\":[]}}\n";
        let file = store
            .upload("alice", "input.jsonl", "batch", body, 3600)
            .unwrap();
        let job = store
            .create_job(NewBatchJob {
                options: BatchJobOptions {
                    result_limit_bytes: limit,
                    ..Default::default()
                },
                owner: "alice".into(),
                input_file_id: file.id,
                provider_instance: "fixture".into(),
                credential_binding: "a".repeat(64),
                endpoint: "/v1/chat/completions".into(),
                model: "fixture-model".into(),
                request_fingerprint: "b".repeat(64),
                policy_fingerprint: "c".repeat(64),
                idempotency_key_hash: None,
                custom_ids: vec!["one".into()],
            })
            .unwrap()
            .0;
        (dir, store, job)
    }
    pub(crate) fn terminal(store: &BatchStore, job: BatchJob) -> BatchJob {
        let job = store
            .prepare_job(
                "alice",
                &job.id,
                job.revision,
                &job.input_file_id,
                &job.policy_fingerprint,
            )
            .unwrap();
        let job = store
            .reserve_batch(
                "alice",
                &job.id,
                BatchUsage {
                    requests: 1,
                    input_tokens: 200,
                    output_tokens: 100,
                    cost_usd: Some(0.1),
                },
                &[],
            )
            .unwrap();
        let intent = store
            .begin_mutation("alice", &job.id, job.revision, BatchMutation::Submit)
            .unwrap();
        store
            .complete_mutation("alice", &intent, "batch_remote", BatchStatus::Completed)
            .unwrap()
    }
    #[test]
    fn owned_result_alias_is_immutable_idempotent_and_listed_by_purpose() {
        let (_dir, store, job) = fixture(1024);
        store.reserve_result_storage("alice", &job.id).unwrap();
        assert!(matches!(
            store.store_batch_result("alice", &job.id, BatchResultKind::Output, b"result"),
            Err(StoreError::Conflict)
        ));
        let job = terminal(&store, job);
        let body = b"{\"custom_id\":\"one\",\"response\":{\"body\":\"original\"}}\n";
        let file = store
            .store_batch_result("alice", &job.id, BatchResultKind::Output, body)
            .unwrap();
        assert_eq!(
            store
                .store_batch_result("alice", &job.id, BatchResultKind::Output, body)
                .unwrap()
                .id,
            file.id
        );
        assert!(matches!(
            store.store_batch_result("alice", &job.id, BatchResultKind::Output, b"changed"),
            Err(StoreError::Conflict)
        ));
        assert_eq!(store.content("alice", &file.id).unwrap(), body);
        assert!(matches!(
            store.content("bob", &file.id),
            Err(StoreError::NotFound)
        ));
        assert!(matches!(
            store.batch_results("bob", &job.id),
            Err(StoreError::NotFound)
        ));
        assert_eq!(
            store
                .list("alice", Some("batch"), None, 20, true)
                .unwrap()
                .data
                .len(),
            1
        );
        assert_eq!(
            store
                .list("alice", Some("batch_output"), None, 20, true)
                .unwrap()
                .data
                .len(),
            1
        );
        assert!(store
            .upload("alice", "forged", "batch_output", b"{}", 3600)
            .is_err());
        assert!(matches!(
            store.delete("alice", &file.id),
            Err(StoreError::Conflict)
        ));
        store
            .record_batch_receipt(
                "alice",
                &job.id,
                "one",
                &"d".repeat(64),
                BatchUsage {
                    requests: 1,
                    input_tokens: 2,
                    output_tokens: 1,
                    cost_usd: Some(0.001),
                },
            )
            .unwrap();
        store.settle_batch_accounting("alice", &job.id).unwrap();
        store.release_result_storage("alice", &job.id).unwrap();
        store.delete("alice", &file.id).unwrap();
        let aliases = store.batch_results("alice", &job.id).unwrap();
        assert!(aliases[0].file_id.is_none());
        assert!(!aliases[0].fingerprint.is_empty());
        assert_eq!(store.batch_usage("alice", 0).unwrap().cost_usd, Some(0.001));
    }
    #[test]
    fn output_reservation_protects_capacity_and_combined_artifact_limit() {
        let (_dir, store, job) = fixture(1024);
        store.reserve_result_storage("alice", &job.id).unwrap();
        store.reserve_result_storage("alice", &job.id).unwrap();
        assert!(matches!(
            store.release_result_storage("alice", &job.id),
            Err(StoreError::Conflict)
        ));
        let job = terminal(&store, job);
        let connection = store.open().unwrap();
        connection
            .execute("UPDATE files SET bytes=?1", [CLIENT_BYTES - 1024])
            .unwrap();
        assert!(matches!(
            store.upload("alice", "new.jsonl", "batch", b"{}", 3600),
            Err(StoreError::Quota)
        ));
        store
            .store_batch_result("alice", &job.id, BatchResultKind::Output, &vec![b'x'; 700])
            .unwrap();
        assert!(matches!(
            store.store_batch_result("alice", &job.id, BatchResultKind::Error, &vec![b'x'; 325]),
            Err(StoreError::Quota)
        ));
        store
            .store_batch_result("alice", &job.id, BatchResultKind::Error, &vec![b'x'; 324])
            .unwrap();
        assert_eq!(
            capacity(&store.open().unwrap(), Some("alice")).unwrap().1,
            CLIENT_BYTES
        );
    }
    #[test]
    fn output_larger_than_input_cap_is_preserved_and_unowned_reads_fail() {
        let (_dir, store, job) = fixture(MAX_RESULT_BYTES as u64);
        store.reserve_result_storage("alice", &job.id).unwrap();
        let job = terminal(&store, job);
        let bytes = vec![b'x'; MAX_FILE_BYTES + 1];
        let file = store
            .store_batch_result("alice", &job.id, BatchResultKind::Output, &bytes)
            .unwrap();
        assert_eq!(store.content("alice", &file.id).unwrap(), bytes);
        assert!(matches!(
            store.content("bob", &file.id),
            Err(StoreError::NotFound)
        ));
    }
}

#[cfg(test)]
mod purge_tests {
    #[test]
    fn terminal_payload_purge_keeps_unknown_quota_evidence_and_is_repeatable() {
        let (_directory, store, job) = super::tests::fixture(1024);
        let job = super::tests::terminal(&store, job);
        let owner = job.owner.clone();
        let before = store.job_reservation(&owner, &job.id).unwrap();
        store.purge_batch_payloads(&owner, &job.id).unwrap();
        store.purge_batch_payloads(&owner, &job.id).unwrap();
        assert_eq!(store.job_reservation(&owner, &job.id).unwrap(), before);
        assert!(!store.get_job(&owner, &job.id).unwrap().accounting_settled);
        assert!(store
            .content(&owner, job.prepared_snapshot_file_id.as_deref().unwrap())
            .is_err());
        assert!(!store.manifest(&owner, &job.id).unwrap().is_empty());
    }
}
