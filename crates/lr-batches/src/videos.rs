//! Owner-scoped native video intents. Never stores prompts, credentials or media URLs.
use super::*;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VideoPhase {
    SubmitPendingOutcome,
    Known,
    RejectedBeforeDispatch,
    DeletePendingOutcome,
    Deleted,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VideoJob {
    pub id: String,
    pub owner: String,
    pub provider: String,
    pub model: String,
    pub protocol: String,
    pub binding: String,
    pub request_hash: String,
    pub idempotency_hash: String,
    pub native_id: Option<String>,
    pub phase: VideoPhase,
    pub status: String,
    pub progress: u8,
    pub seconds: u32,
    pub size: String,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub cost_usd: Option<f64>,
    pub error_code: Option<String>,
    pub archived: bool,
    pub event_id: Option<String>,
    pub poll_sequence: u64,
    pub applied_sequence: u64,
    pub last_poll_ms: i64,
}
impl VideoJob {
    pub fn terminal(&self) -> bool {
        matches!(
            self.status.as_str(),
            "completed" | "failed" | "canceled" | "expired"
        )
    }
}
#[derive(Clone, Debug)]
pub struct VideoUpdate {
    pub native_id: String,
    pub status: String,
    pub progress: u8,
    pub completed_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub cost_usd: Option<f64>,
}
fn schema(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS video_jobs(id TEXT PRIMARY KEY,owner TEXT NOT NULL,idempotency_hash TEXT NOT NULL,request_hash TEXT NOT NULL,binding TEXT NOT NULL,native_id TEXT,created_at INTEGER NOT NULL,payload TEXT NOT NULL,UNIQUE(owner,idempotency_hash)); CREATE UNIQUE INDEX IF NOT EXISTS video_native_binding ON video_jobs(binding,native_id) WHERE native_id IS NOT NULL; CREATE INDEX IF NOT EXISTS video_owner_created ON video_jobs(owner,created_at);")?;
    Ok(())
}
fn decode(payload: String) -> Result<VideoJob> {
    serde_json::from_str(&payload)
        .map_err(|_| StoreError::Storage("Invalid video journal row".into()))
}
fn load(conn: &Connection, owner: &str, id: &str) -> Result<VideoJob> {
    let payload = conn
        .query_row(
            "SELECT payload FROM video_jobs WHERE owner=?1 AND id=?2",
            params![owner, id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or(StoreError::NotFound)?;
    let job = decode(payload)?;
    if job.created_at < chrono::Utc::now().timestamp() - DEFAULT_RETENTION {
        return Err(StoreError::NotFound);
    }
    Ok(job)
}
fn save(conn: &Connection, job: &VideoJob) -> Result<()> {
    let payload = serde_json::to_string(job)
        .map_err(|_| StoreError::Storage("Video journal serialization failed".into()))?;
    if payload.len() > 16 * 1024 {
        return Err(StoreError::Invalid(
            "Video journal metadata exceeds 16 KiB".into(),
        ));
    }
    conn.execute(
        "UPDATE video_jobs SET native_id=?1,payload=?2 WHERE owner=?3 AND id=?4",
        params![job.native_id, payload, job.owner, job.id],
    )
    .map_err(|e| match e {
        rusqlite::Error::SqliteFailure(ref error, _)
            if error.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            StoreError::Conflict
        }
        _ => e.into(),
    })?;
    Ok(())
}
impl BatchStore {
    pub fn create_video_intent(&self, mut job: VideoJob) -> Result<(VideoJob, bool)> {
        if job.owner.is_empty()
            || job.owner.len() > 256
            || job.provider.len() > 256
            || job.model.len() > 512
            || job.idempotency_hash.len() != 64
            || job.request_hash.len() != 64
            || job.binding.len() != 64
        {
            return Err(StoreError::Invalid("Invalid video intent metadata".into()));
        }
        let _lock = self.lock.lock();
        let mut conn = self.open()?;
        schema(&conn)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM video_jobs WHERE created_at<?1",
            params![chrono::Utc::now().timestamp() - DEFAULT_RETENTION],
        )?;
        if let Some(payload) = tx
            .query_row(
                "SELECT payload FROM video_jobs WHERE owner=?1 AND idempotency_hash=?2",
                params![job.owner, job.idempotency_hash],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            let previous = decode(payload)?;
            if previous.request_hash != job.request_hash {
                return Err(StoreError::Conflict);
            }
            return Ok((previous, false));
        }
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM video_jobs", [], |row| row.get(0))?;
        let owner_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM video_jobs WHERE owner=?1",
            params![job.owner],
            |row| row.get(0),
        )?;
        if count >= 10000 || owner_count >= 1000 {
            return Err(StoreError::Quota);
        }
        let mut active_owner = 0;
        let mut active_all = 0;
        {
            let mut statement = tx.prepare("SELECT payload FROM video_jobs")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                let other = decode(row?)?;
                if !other.archived && !other.terminal() && other.phase != VideoPhase::Deleted {
                    active_all += 1;
                    if other.owner == job.owner {
                        active_owner += 1;
                    }
                }
            }
        }
        if active_owner >= 8 || active_all >= 64 {
            return Err(StoreError::Quota);
        }
        job.id = format!("video_lr_{}", Uuid::new_v4().simple());
        job.phase = VideoPhase::SubmitPendingOutcome;
        job.status = "unknown".into();
        job.native_id = None;
        let payload = serde_json::to_string(&job)
            .map_err(|_| StoreError::Storage("Video intent serialization failed".into()))?;
        if payload.len() > 16 * 1024 {
            return Err(StoreError::Invalid("Video intent too large".into()));
        }
        tx.execute("INSERT INTO video_jobs(id,owner,idempotency_hash,request_hash,binding,created_at,payload) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![job.id,job.owner,job.idempotency_hash,job.request_hash,job.binding,job.created_at,payload])?;
        tx.commit()?;
        Ok((job, true))
    }
    pub fn video_job(&self, owner: &str, id: &str) -> Result<VideoJob> {
        let _lock = self.lock.lock();
        let conn = self.open()?;
        schema(&conn)?;
        load(&conn, owner, id)
    }
    pub fn list_video_jobs(
        &self,
        owner: &str,
        limit: usize,
        after: Option<&str>,
    ) -> Result<(Vec<VideoJob>, bool)> {
        if !(1..=100).contains(&limit) {
            return Err(StoreError::Invalid("Video list limit must be 1–100".into()));
        }
        let _lock = self.lock.lock();
        let conn = self.open()?;
        schema(&conn)?;
        let cursor = after.map(|id| load(&conn, owner, id)).transpose()?;
        let mut statement=conn.prepare("SELECT payload FROM video_jobs WHERE owner=?1 AND created_at>=?2 ORDER BY created_at DESC,id DESC")?;
        let rows = statement.query_map(
            params![owner, chrono::Utc::now().timestamp() - DEFAULT_RETENTION],
            |row| row.get::<_, String>(0),
        )?;
        let mut result = Vec::new();
        for row in rows {
            let job = decode(row?)?;
            if job.archived || job.phase == VideoPhase::Deleted {
                continue;
            }
            if cursor
                .as_ref()
                .is_some_and(|c| (job.created_at, job.id.as_str()) >= (c.created_at, c.id.as_str()))
            {
                continue;
            }
            result.push(job);
            if result.len() > limit {
                break;
            }
        }
        let more = result.len() > limit;
        result.truncate(limit);
        Ok((result, more))
    }
    pub fn submit_video_result(
        &self,
        owner: &str,
        id: &str,
        update: VideoUpdate,
    ) -> Result<VideoJob> {
        let _lock = self.lock.lock();
        let mut conn = self.open()?;
        schema(&conn)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = load(&tx, owner, id)?;
        if job.phase != VideoPhase::SubmitPendingOutcome || job.native_id.is_some() {
            return Err(StoreError::Conflict);
        }
        validate_update(&update)?;
        job.native_id = Some(update.native_id);
        job.phase = VideoPhase::Known;
        job.status = update.status;
        job.progress = update.progress;
        job.completed_at = update.completed_at;
        job.expires_at = update.expires_at;
        job.cost_usd = update.cost_usd;
        job.error_code = None;
        save(&tx, &job)?;
        tx.commit()?;
        Ok(job)
    }
    pub fn reject_video_before_dispatch(&self, owner: &str, id: &str) -> Result<VideoJob> {
        let _lock = self.lock.lock();
        let mut conn = self.open()?;
        schema(&conn)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = load(&tx, owner, id)?;
        if job.phase != VideoPhase::SubmitPendingOutcome {
            return Err(StoreError::Conflict);
        }
        job.phase = VideoPhase::RejectedBeforeDispatch;
        job.status = "failed".into();
        job.error_code = Some("not_dispatched".into());
        job.cost_usd = Some(0.0);
        save(&tx, &job)?;
        tx.commit()?;
        Ok(job)
    }
    /// Claims a refresh sequence; None returns the cached row during the one-second throttle.
    pub fn claim_video_poll(
        &self,
        owner: &str,
        id: &str,
        now_ms: i64,
    ) -> Result<(VideoJob, Option<u64>)> {
        let _lock = self.lock.lock();
        let mut conn = self.open()?;
        schema(&conn)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = load(&tx, owner, id)?;
        if job.phase != VideoPhase::Known
            || job.native_id.is_none()
            || now_ms.saturating_sub(job.last_poll_ms) < 1000
        {
            return Ok((job, None));
        }
        job.poll_sequence = job
            .poll_sequence
            .checked_add(1)
            .ok_or_else(|| StoreError::Storage("Video poll sequence exhausted".into()))?;
        job.last_poll_ms = now_ms;
        save(&tx, &job)?;
        tx.commit()?;
        let sequence = job.poll_sequence;
        Ok((job, Some(sequence)))
    }
    pub fn update_video_status(
        &self,
        owner: &str,
        id: &str,
        sequence: u64,
        update: VideoUpdate,
    ) -> Result<VideoJob> {
        validate_update(&update)?;
        let _lock = self.lock.lock();
        let mut conn = self.open()?;
        schema(&conn)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = load(&tx, owner, id)?;
        if job.native_id.as_deref() != Some(update.native_id.as_str()) {
            return Err(StoreError::Conflict);
        }
        if job.phase != VideoPhase::Known
            || sequence < job.applied_sequence
            || sequence > job.poll_sequence
        {
            return Ok(job);
        }
        if job.terminal() && job.status != update.status {
            return Ok(job);
        }
        if job.status == "in_progress" && update.status == "queued" {
            return Ok(job);
        }
        job.applied_sequence = sequence;
        job.status = update.status;
        job.progress = job.progress.max(update.progress);
        job.completed_at = update.completed_at.or(job.completed_at);
        job.expires_at = update.expires_at.or(job.expires_at);
        if update.cost_usd.is_some() {
            job.cost_usd = update.cost_usd;
        }
        save(&tx, &job)?;
        tx.commit()?;
        Ok(job)
    }
    pub fn begin_video_delete(&self, owner: &str, id: &str) -> Result<VideoJob> {
        let _lock = self.lock.lock();
        let mut conn = self.open()?;
        schema(&conn)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = load(&tx, owner, id)?;
        if job.phase == VideoPhase::Deleted {
            return Ok(job);
        }
        if job.phase != VideoPhase::Known || !matches!(job.status.as_str(), "completed" | "failed")
        {
            return Err(StoreError::Conflict);
        }
        job.phase = VideoPhase::DeletePendingOutcome;
        save(&tx, &job)?;
        tx.commit()?;
        Ok(job)
    }
    pub fn finish_video_delete(&self, owner: &str, id: &str) -> Result<VideoJob> {
        let _lock = self.lock.lock();
        let mut conn = self.open()?;
        schema(&conn)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = load(&tx, owner, id)?;
        if job.phase != VideoPhase::DeletePendingOutcome {
            return Err(StoreError::Conflict);
        }
        job.phase = VideoPhase::Deleted;
        save(&tx, &job)?;
        tx.commit()?;
        Ok(job)
    }
    pub fn archive_video(&self, owner: &str, id: &str) -> Result<VideoJob> {
        let _lock = self.lock.lock();
        let mut conn = self.open()?;
        schema(&conn)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = load(&tx, owner, id)?;
        if !job.terminal()
            && !(job.phase == VideoPhase::SubmitPendingOutcome
                && chrono::Utc::now()
                    .timestamp()
                    .saturating_sub(job.created_at)
                    >= 600)
        {
            return Err(StoreError::Conflict);
        }
        job.archived = true;
        save(&tx, &job)?;
        tx.commit()?;
        Ok(job)
    }
}
fn validate_update(update: &VideoUpdate) -> Result<()> {
    if update.native_id.is_empty()
        || update.native_id.len() > 256
        || !update
            .native_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        || !matches!(
            update.status.as_str(),
            "queued" | "in_progress" | "completed" | "failed" | "canceled" | "expired"
        )
        || update.progress > 100
        || update
            .cost_usd
            .is_some_and(|n| !n.is_finite() || !(0.0..=1e9).contains(&n))
    {
        return Err(StoreError::Invalid("Invalid native video update".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn intent(owner: &str, key: char) -> VideoJob {
        VideoJob {
            id: String::new(),
            owner: owner.into(),
            provider: "gw".into(),
            model: "gw/model".into(),
            protocol: "llmgateway".into(),
            binding: "c".repeat(64),
            request_hash: "a".repeat(64),
            idempotency_hash: key.to_string().repeat(64),
            native_id: None,
            phase: VideoPhase::SubmitPendingOutcome,
            status: "unknown".into(),
            progress: 0,
            seconds: 4,
            size: "1280x720".into(),
            created_at: chrono::Utc::now().timestamp(),
            completed_at: None,
            expires_at: None,
            cost_usd: None,
            error_code: None,
            archived: false,
            event_id: None,
            poll_sequence: 0,
            applied_sequence: 0,
            last_poll_ms: 0,
        }
    }
    fn update(status: &str, cost: Option<f64>) -> VideoUpdate {
        VideoUpdate {
            native_id: "v_native".into(),
            status: status.into(),
            progress: if status == "completed" { 100 } else { 1 },
            completed_at: None,
            expires_at: None,
            cost_usd: cost,
        }
    }
    #[test]
    fn intent_uniqueness_survives_restart_and_rejects_cross_owner_ids_and_payload_rebinding() {
        let directory = tempfile::tempdir().unwrap();
        let store = BatchStore::new(directory.path().join("store"));
        let (first, dispatch) = store.create_video_intent(intent("a", 'a')).unwrap();
        assert!(dispatch);
        let reopened = BatchStore::new(directory.path().join("store"));
        let (again, dispatch) = reopened.create_video_intent(intent("a", 'a')).unwrap();
        assert!(!dispatch);
        assert_eq!(first.id, again.id);
        assert!(again.native_id.is_none());
        assert_eq!(again.phase, VideoPhase::SubmitPendingOutcome);
        assert!(matches!(
            store.video_job("b", &first.id),
            Err(StoreError::NotFound)
        ));
        let mut changed = intent("a", 'a');
        changed.request_hash = "b".repeat(64);
        assert!(matches!(
            store.create_video_intent(changed),
            Err(StoreError::Conflict)
        ));
        let (other, _) = store.create_video_intent(intent("b", 'a')).unwrap();
        assert_ne!(first.id, other.id);
        assert!(matches!(
            store.list_video_jobs("b", 20, Some(&first.id)),
            Err(StoreError::NotFound)
        ));
    }
    #[test]
    fn concurrent_creates_claim_one_native_dispatch_and_never_share_a_native_job_between_clients() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("store");
        let mut threads = Vec::new();
        for _ in 0..8 {
            let root = root.clone();
            threads.push(std::thread::spawn(move || {
                BatchStore::new(root)
                    .create_video_intent(intent("a", 'a'))
                    .unwrap()
            }));
        }
        let results: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|(_, dispatch)| *dispatch).count(), 1);
        assert!(results.iter().all(|(job, _)| job.id == results[0].0.id));
        let store = BatchStore::new(root);
        let first = &results[0].0;
        store
            .submit_video_result("a", &first.id, update("queued", None))
            .unwrap();
        let (second, _) = store.create_video_intent(intent("b", 'b')).unwrap();
        assert!(matches!(
            store.submit_video_result("b", &second.id, update("queued", None)),
            Err(StoreError::Conflict)
        ));
        assert!(store
            .video_job("b", &second.id)
            .unwrap()
            .native_id
            .is_none());
    }
    #[test]
    fn stale_polls_cannot_regress_state_or_multiply_reported_job_cost_and_delete_intent_is_durable()
    {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("store");
        let store = BatchStore::new(root.clone());
        let (job, _) = store.create_video_intent(intent("a", 'a')).unwrap();
        store
            .submit_video_result("a", &job.id, update("queued", None))
            .unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let (_, first) = store.claim_video_poll("a", &job.id, now).unwrap();
        let (_, throttled) = store.claim_video_poll("a", &job.id, now + 1).unwrap();
        assert!(throttled.is_none());
        let (_, second) = store.claim_video_poll("a", &job.id, now + 1001).unwrap();
        store
            .update_video_status(
                "a",
                &job.id,
                second.unwrap(),
                update("completed", Some(0.4)),
            )
            .unwrap();
        let stale = store
            .update_video_status("a", &job.id, first.unwrap(), update("in_progress", None))
            .unwrap();
        assert_eq!(stale.status, "completed");
        assert_eq!(stale.cost_usd, Some(0.4));
        let repeated = store
            .update_video_status(
                "a",
                &job.id,
                second.unwrap(),
                update("completed", Some(0.4)),
            )
            .unwrap();
        assert_eq!(repeated.cost_usd, Some(0.4));
        store.begin_video_delete("a", &job.id).unwrap();
        let reopened = BatchStore::new(root);
        assert_eq!(
            reopened.video_job("a", &job.id).unwrap().phase,
            VideoPhase::DeletePendingOutcome
        );
        assert!(matches!(
            reopened.begin_video_delete("a", &job.id),
            Err(StoreError::Conflict)
        ));
        reopened.finish_video_delete("a", &job.id).unwrap();
        assert!(reopened
            .list_video_jobs("a", 20, None)
            .unwrap()
            .0
            .is_empty());
    }
    #[test]
    fn rejected_and_archived_records_keep_idempotency_and_only_non_dispatch_is_known_zero_cost() {
        let directory = tempfile::tempdir().unwrap();
        let store = BatchStore::new(directory.path().join("store"));
        let (job, _) = store.create_video_intent(intent("a", 'a')).unwrap();
        let rejected = store.reject_video_before_dispatch("a", &job.id).unwrap();
        assert_eq!(rejected.cost_usd, Some(0.0));
        assert!(rejected.native_id.is_none());
        store.archive_video("a", &job.id).unwrap();
        assert!(store.list_video_jobs("a", 20, None).unwrap().0.is_empty());
        assert!(!store.create_video_intent(intent("a", 'a')).unwrap().1);
        let (pending, _) = store.create_video_intent(intent("a", 'b')).unwrap();
        assert!(pending.cost_usd.is_none());
        assert!(matches!(
            store.archive_video("a", &pending.id),
            Err(StoreError::Conflict)
        ));
    }
}
