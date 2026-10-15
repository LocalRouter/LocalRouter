//! Durable ownership and mutation journal. These methods never perform IO to a
//! provider. A recorded create intent cannot be issued a second time on restart.
use super::*;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BatchPhase {
    Draft,
    Prepared,
    UploadPendingOutcome,
    Uploaded,
    SubmitPendingOutcome,
    Submitted,
    CancelPendingOutcome,
    Terminal,
    RejectedBeforeSubmission,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    Validating,
    InProgress,
    Finalizing,
    Cancelling,
    Completed,
    Failed,
    Expired,
    Cancelled,
}
impl BatchStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Expired | Self::Cancelled
        )
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BatchMutation {
    Upload,
    Submit,
    Cancel,
}
impl BatchMutation {
    fn label(self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Submit => "submit",
            Self::Cancel => "cancel",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BatchUsage {
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Unknown spend is never coerced to zero.
    pub cost_usd: Option<f64>,
}
impl Default for BatchUsage {
    fn default() -> Self {
        Self {
            requests: 0,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: Some(0.0),
        }
    }
}
impl BatchUsage {
    fn validate(&self) -> Result<()> {
        if self.requests > MAX_ROWS as u64
            || self.input_tokens > 9_000_000_000_000
            || self.output_tokens > 9_000_000_000_000
            || self
                .cost_usd
                .is_some_and(|n| !n.is_finite() || !(0.0..=1e9).contains(&n))
        {
            return Err(StoreError::Invalid(
                "Invalid or unbounded batch usage".into(),
            ));
        }
        Ok(())
    }
    fn add(&mut self, other: &Self) {
        self.requests = self.requests.saturating_add(other.requests);
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cost_usd = self.cost_usd.zip(other.cost_usd).map(|(a, b)| a + b);
    }
    fn maximum(&mut self, other: &Self) {
        self.requests = self.requests.max(other.requests);
        self.input_tokens = self.input_tokens.max(other.input_tokens);
        self.output_tokens = self.output_tokens.max(other.output_tokens);
        self.cost_usd = self.cost_usd.zip(other.cost_usd).map(|(a, b)| a.max(b));
    }
}
#[derive(Clone, Copy, Debug)]
pub enum BatchMetric {
    Requests,
    InputTokens,
    OutputTokens,
    TotalTokens,
    Cost,
}
impl BatchMetric {
    pub fn amount(self, usage: &BatchUsage) -> Option<f64> {
        Some(match self {
            Self::Requests => usage.requests as f64,
            Self::InputTokens => usage.input_tokens as f64,
            Self::OutputTokens => usage.output_tokens as f64,
            Self::TotalTokens => usage.input_tokens.saturating_add(usage.output_tokens) as f64,
            Self::Cost => return usage.cost_usd,
        })
    }
}
#[derive(Clone, Debug)]
pub struct BatchBudget {
    pub metric: BatchMetric,
    pub window_start: i64,
    pub limit: f64,
    /// Current ordinary synchronous usage; batch ledger amounts are added here.
    pub ordinary_usage: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BatchJob {
    pub id: String,
    pub owner: String,
    pub input_file_id: String,
    pub prepared_snapshot_file_id: Option<String>,
    pub snapshot_fingerprint: Option<String>,
    pub provider_instance: String,
    /// Durable salted fingerprint of configured origin and credential identity.
    pub credential_binding: String,
    pub endpoint: String,
    pub model: String,
    pub request_fingerprint: String,
    pub policy_fingerprint: String,
    pub revision: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub rows: usize,
    pub phase: BatchPhase,
    pub status: BatchStatus,
    pub remote_input_file_id: Option<String>,
    pub remote_batch_id: Option<String>,
    pub accounting_settled: bool,
}
pub struct BatchReceipt {
    pub custom_id: String,
    pub fingerprint: String,
    pub usage: BatchUsage,
}

pub struct NewBatchJob {
    pub owner: String,
    pub input_file_id: String,
    pub provider_instance: String,
    pub credential_binding: String,
    pub endpoint: String,
    pub model: String,
    pub request_fingerprint: String,
    pub policy_fingerprint: String,
    /// Already hashed by the API; scoped to the owner, never globally shared.
    pub idempotency_key_hash: Option<String>,
    pub custom_ids: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MutationIntent {
    pub id: String,
    pub job_id: String,
    pub kind: BatchMutation,
}

pub(super) fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS batch_jobs (
        id TEXT PRIMARY KEY, owner TEXT NOT NULL, idempotency_key_hash TEXT,
        request_fingerprint TEXT NOT NULL, revision INTEGER NOT NULL, active INTEGER NOT NULL,
        record TEXT NOT NULL, UNIQUE(owner,idempotency_key_hash)
    );
    CREATE INDEX IF NOT EXISTS batch_jobs_owner ON batch_jobs(owner,active);
    CREATE TABLE IF NOT EXISTS batch_mutations (
        id TEXT PRIMARY KEY, job_id TEXT NOT NULL REFERENCES batch_jobs(id),
        kind TEXT NOT NULL, completed INTEGER NOT NULL DEFAULT 0,
        remote_id TEXT
    );
    CREATE UNIQUE INDEX IF NOT EXISTS batch_create_once ON batch_mutations(job_id,kind) WHERE kind IN ('upload','submit');
    CREATE TABLE IF NOT EXISTS batch_rows (
        job_id TEXT NOT NULL REFERENCES batch_jobs(id), custom_id TEXT NOT NULL,
        receipt_fingerprint TEXT, usage TEXT, observed_at INTEGER,
        PRIMARY KEY(job_id,custom_id)
    );
    CREATE TABLE IF NOT EXISTS batch_reservations (
        job_id TEXT PRIMARY KEY REFERENCES batch_jobs(id), usage TEXT NOT NULL
    );",
    )?;
    Ok(())
}
fn status_rank(status: BatchStatus) -> u8 {
    match status {
        BatchStatus::Validating => 0,
        BatchStatus::InProgress => 1,
        BatchStatus::Finalizing => 2,
        BatchStatus::Cancelling => 3,
        _ => 4,
    }
}
fn encode<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|_| StoreError::Storage("Invalid journal data".into()))
}
fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T> {
    serde_json::from_str(value).map_err(|_| StoreError::Storage("Invalid journal data".into()))
}
fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn remote_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn load(connection: &Connection, owner: &str, id: &str) -> Result<BatchJob> {
    if !id
        .strip_prefix("batch-lr-")
        .is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(StoreError::NotFound);
    }
    let record: String = connection
        .query_row(
            "SELECT record FROM batch_jobs WHERE id=?1 AND owner=?2",
            params![id, owner],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(StoreError::NotFound)?;
    let job: BatchJob = decode(&record)?;
    if job.owner != owner || job.id != id {
        return Err(StoreError::Storage("Invalid job ownership record".into()));
    }
    Ok(job)
}
fn save(connection: &Connection, job: &mut BatchJob) -> Result<()> {
    let previous = job.revision;
    job.revision = job.revision.checked_add(1).ok_or(StoreError::Conflict)?;
    job.updated_at = chrono::Utc::now().timestamp();
    let active = !(job.status.is_terminal() && job.accounting_settled);
    let changed = connection.execute(
        "UPDATE batch_jobs SET revision=?1,active=?2,record=?3 WHERE id=?4 AND revision=?5",
        params![job.revision, active, encode(job)?, job.id, previous],
    )?;
    if changed != 1 {
        return Err(StoreError::Conflict);
    }
    Ok(())
}

impl BatchStore {
    /// Durable salt is created only by an explicit job action. Provider keys are
    /// hashed with it outside this crate and never written to the journal.
    pub fn binding_salt(&self) -> Result<String> {
        let _guard = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS batch_settings (key TEXT PRIMARY KEY,value TEXT NOT NULL)",
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO batch_settings VALUES ('binding_salt',?1)",
            [format!(
                "{}{}",
                Uuid::new_v4().simple(),
                Uuid::new_v4().simple()
            )],
        )?;
        let salt = tx.query_row(
            "SELECT value FROM batch_settings WHERE key='binding_salt'",
            [],
            |r| r.get::<_, String>(0),
        )?;
        if !digest(&salt) {
            return Err(StoreError::Storage("Invalid binding salt".into()));
        }
        tx.commit()?;
        Ok(salt)
    }

    pub fn create_job(&self, new: NewBatchJob) -> Result<(BatchJob, bool)> {
        if new.owner.is_empty()
            || new.owner.len() > 256
            || new.provider_instance.is_empty()
            || new.provider_instance.len() > 256
            || new.model.is_empty()
            || new.model.len() > 512
            || !["/v1/chat/completions", "/v1/responses", "/v1/embeddings"]
                .contains(&new.endpoint.as_str())
            || !digest(&new.credential_binding)
            || !digest(&new.request_fingerprint)
            || !digest(&new.policy_fingerprint)
            || new
                .idempotency_key_hash
                .as_ref()
                .is_some_and(|s| !digest(s))
        {
            return Err(StoreError::Invalid("Invalid native job binding".into()));
        }
        if new.custom_ids.is_empty() || new.custom_ids.len() > MAX_ROWS {
            return Err(StoreError::Invalid("Invalid batch row count".into()));
        }
        let mut seen = HashSet::new();
        if new.custom_ids.iter().any(|id| {
            id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) || !seen.insert(id)
        }) {
            return Err(StoreError::Invalid(
                "Custom IDs must be unique, bounded and nonempty".into(),
            ));
        }
        let _guard = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(key) = &new.idempotency_key_hash {
            let existing:Option<(String,String)>=tx.query_row("SELECT id,request_fingerprint FROM batch_jobs WHERE owner=?1 AND idempotency_key_hash=?2",params![new.owner,key],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            if let Some((id, fingerprint)) = existing {
                if fingerprint != new.request_fingerprint {
                    return Err(StoreError::Conflict);
                }
                let job = load(&tx, &new.owner, &id)?;
                tx.commit()?;
                return Ok((job, false));
            }
        }
        Self::expire(&tx, chrono::Utc::now().timestamp())?;
        lookup(&tx, &new.owner, &new.input_file_id)?;
        let (owned, active): (i64, i64) = tx.query_row(
            "SELECT count(*),coalesce(sum(active),0) FROM batch_jobs WHERE owner=?1",
            [&new.owner],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let (global, global_active): (i64, i64) = tx.query_row(
            "SELECT count(*),coalesce(sum(active),0) FROM batch_jobs",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if owned >= 128 || global >= 512 || active >= 8 || global_active >= 16 {
            return Err(StoreError::Quota);
        }
        let now = chrono::Utc::now().timestamp();
        let job = BatchJob {
            id: format!("batch-lr-{}", Uuid::new_v4().simple()),
            owner: new.owner,
            input_file_id: new.input_file_id,
            prepared_snapshot_file_id: None,
            snapshot_fingerprint: None,
            provider_instance: new.provider_instance,
            credential_binding: new.credential_binding,
            endpoint: new.endpoint,
            model: new.model,
            request_fingerprint: new.request_fingerprint,
            policy_fingerprint: new.policy_fingerprint,
            revision: 0,
            created_at: now,
            updated_at: now,
            rows: new.custom_ids.len(),
            phase: BatchPhase::Draft,
            status: BatchStatus::Validating,
            remote_input_file_id: None,
            remote_batch_id: None,
            accounting_settled: false,
        };
        tx.execute(
            "INSERT INTO batch_jobs VALUES (?1,?2,?3,?4,0,1,?5)",
            params![
                job.id,
                job.owner,
                new.idempotency_key_hash,
                job.request_fingerprint,
                encode(&job)?
            ],
        )?;
        tx.execute(
            "INSERT INTO file_references VALUES (?1,?2)",
            params![job.input_file_id, job.id],
        )?;
        {
            let mut statement =
                tx.prepare("INSERT INTO batch_rows (job_id,custom_id) VALUES (?1,?2)")?;
            for custom_id in new.custom_ids {
                statement.execute(params![job.id, custom_id])?;
            }
        }
        tx.commit()?;
        Ok((job, true))
    }

    /// Seal the exact provider-shaped snapshot after row policy validation.
    /// No mutation intent is allowed for an unprepared draft. The server must
    /// run current safety/permission checks before invoking this method, then
    /// recheck the same policy fingerprint immediately before external IO.
    pub fn prepare_job(
        &self,
        owner: &str,
        id: &str,
        revision: u64,
        snapshot_file_id: &str,
        policy_fingerprint: &str,
    ) -> Result<BatchJob> {
        use sha2::{Digest, Sha256};
        let _guard = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::expire(&tx, chrono::Utc::now().timestamp())?;
        let mut job = load(&tx, owner, id)?;
        if job.revision != revision
            || job.phase != BatchPhase::Draft
            || job.policy_fingerprint != policy_fingerprint
        {
            return Err(StoreError::Conflict);
        }
        lookup(&tx, owner, snapshot_file_id)?;
        let content: Vec<u8> = tx
            .query_row(
                "SELECT content FROM files WHERE owner=?1 AND id=?2 AND bytes<=?3",
                params![owner, snapshot_file_id, MAX_FILE_BYTES as i64],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        if validate_jsonl(&content)? != job.rows {
            return Err(StoreError::Invalid("Snapshot row count changed".into()));
        }
        let text = std::str::from_utf8(&content)
            .map_err(|_| StoreError::Invalid("Invalid snapshot UTF-8".into()))?;
        let mut seen = HashSet::new();
        for line in text.lines() {
            let row: serde_json::Value = serde_json::from_str(line)
                .map_err(|_| StoreError::Invalid("Invalid snapshot JSON".into()))?;
            // Prepared snapshots are generated by LocalRouter. Requiring the
            // compact reserialization rejects duplicate keys and alternate
            // parser interpretations before raw upload to another service.
            if serde_json::to_string(&row)
                .map_err(|_| StoreError::Invalid("Invalid snapshot".into()))?
                != line
            {
                return Err(StoreError::Invalid(
                    "Snapshot must use canonical compact JSON without duplicate keys".into(),
                ));
            }
            let custom_id = row["custom_id"]
                .as_str()
                .ok_or_else(|| StoreError::Invalid("Snapshot requires custom_id".into()))?;
            if !seen.insert(custom_id.to_owned())
                || row["method"] != "POST"
                || row["url"] != job.endpoint
                || row["body"]["model"] != job.model
            {
                return Err(StoreError::Invalid(
                    "Snapshot must preserve the manifest, model and endpoint".into(),
                ));
            }
            let known: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM batch_rows WHERE job_id=?1 AND custom_id=?2)",
                params![id, custom_id],
                |r| r.get(0),
            )?;
            if !known {
                return Err(StoreError::Invalid(
                    "Snapshot custom ID is not in the manifest".into(),
                ));
            }
        }
        job.prepared_snapshot_file_id = Some(snapshot_file_id.into());
        job.snapshot_fingerprint = Some(format!("{:x}", Sha256::digest(&content)));
        job.phase = BatchPhase::Prepared;
        tx.execute(
            "INSERT OR IGNORE INTO file_references VALUES (?1,?2)",
            params![snapshot_file_id, id],
        )?;
        save(&tx, &mut job)?;
        tx.commit()?;
        Ok(job)
    }

    /// Resolve a retried create before opening its input or consulting changed
    /// provider credentials. The original immutable binding remains in force.
    pub fn find_idempotent_job(
        &self,
        owner: &str,
        key_hash: &str,
        request_fingerprint: &str,
    ) -> Result<Option<BatchJob>> {
        if !digest(key_hash) || !digest(request_fingerprint) {
            return Err(StoreError::Invalid(
                "Invalid idempotency fingerprint".into(),
            ));
        }
        let _guard = self.lock.lock();
        let connection = self.open()?;
        let row:Option<(String,String)>=connection.query_row("SELECT id,request_fingerprint FROM batch_jobs WHERE owner=?1 AND idempotency_key_hash=?2",params![owner,key_hash],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        match row {
            Some((id, fingerprint)) if fingerprint == request_fingerprint => {
                Ok(Some(load(&connection, owner, &id)?))
            }
            Some(_) => Err(StoreError::Conflict),
            None => Ok(None),
        }
    }

    pub fn get_job(&self, owner: &str, id: &str) -> Result<BatchJob> {
        let _guard = self.lock.lock();
        load(&self.open()?, owner, id)
    }

    pub fn pending_mutations(&self, owner: &str, id: &str) -> Result<Vec<MutationIntent>> {
        let _guard = self.lock.lock();
        let connection = self.open()?;
        load(&connection, owner, id)?;
        let mut statement = connection.prepare(
            "SELECT id,kind FROM batch_mutations WHERE job_id=?1 AND completed=0 ORDER BY rowid",
        )?;
        let rows = statement
            .query_map([id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(operation_id, kind)| {
                Ok(MutationIntent {
                    id: operation_id,
                    job_id: id.into(),
                    kind: match kind.as_str() {
                        "upload" => BatchMutation::Upload,
                        "submit" => BatchMutation::Submit,
                        "cancel" => BatchMutation::Cancel,
                        _ => return Err(StoreError::Storage("Invalid journal operation".into())),
                    },
                })
            })
            .collect()
    }

    pub fn list_jobs(&self, owner: &str) -> Result<Vec<BatchJob>> {
        let _guard = self.lock.lock();
        let connection = self.open()?;
        let mut statement = connection.prepare(
            "SELECT record FROM batch_jobs WHERE owner=?1 ORDER BY rowid DESC LIMIT 128",
        )?;
        let records = statement
            .query_map([owner], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        records.into_iter().map(|s| decode(&s)).collect()
    }

    /// Record intent before crossing a network mutation boundary. Submit is
    /// unique for the job for its entire lifetime, including failed HTTP calls.
    pub fn begin_mutation(
        &self,
        owner: &str,
        id: &str,
        revision: u64,
        kind: BatchMutation,
    ) -> Result<MutationIntent> {
        let _guard = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = load(&tx, owner, id)?;
        if job.revision != revision {
            return Err(StoreError::Conflict);
        }
        let permitted = match kind {
            BatchMutation::Upload => job.phase == BatchPhase::Prepared,
            BatchMutation::Submit => {
                matches!(job.phase, BatchPhase::Prepared | BatchPhase::Uploaded)
            }
            BatchMutation::Cancel => {
                matches!(
                    job.phase,
                    BatchPhase::Submitted | BatchPhase::CancelPendingOutcome
                ) && !job.status.is_terminal()
            }
        };
        if !permitted {
            return Err(StoreError::Conflict);
        }
        if kind == BatchMutation::Cancel {
            let attempts: i64 = tx.query_row(
                "SELECT count(*) FROM batch_mutations WHERE job_id=?1 AND kind='cancel'",
                [id],
                |r| r.get(0),
            )?;
            if attempts >= 8 {
                return Err(StoreError::Quota);
            }
        }
        // External inference or upload cannot start without durable quota intent.
        let reserved: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM batch_reservations WHERE job_id=?1)",
            [id],
            |r| r.get(0),
        )?;
        if !reserved {
            return Err(StoreError::Conflict);
        }
        let intent = MutationIntent {
            id: Uuid::new_v4().simple().to_string(),
            job_id: id.into(),
            kind,
        };
        tx.execute(
            "INSERT INTO batch_mutations (id,job_id,kind) VALUES (?1,?2,?3)",
            params![intent.id, id, kind.label()],
        )?;
        job.phase = match kind {
            BatchMutation::Upload => BatchPhase::UploadPendingOutcome,
            BatchMutation::Submit => BatchPhase::SubmitPendingOutcome,
            BatchMutation::Cancel => BatchPhase::CancelPendingOutcome,
        };
        save(&tx, &mut job)?;
        tx.commit()?;
        Ok(intent)
    }

    /// Persist the known remote identity atomically with completion of intent.
    /// A duplicate identical acknowledgement is safe; a changed ID is a conflict.
    pub fn complete_mutation(
        &self,
        owner: &str,
        intent: &MutationIntent,
        remote: &str,
        status: BatchStatus,
    ) -> Result<BatchJob> {
        if !remote_id(remote) {
            return Err(StoreError::Invalid("Invalid remote identity".into()));
        }
        if intent.kind == BatchMutation::Cancel
            && status != BatchStatus::Cancelling
            && !status.is_terminal()
        {
            return Err(StoreError::Invalid(
                "Cancellation acknowledgement must be cancelling or terminal".into(),
            ));
        }
        let _guard = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = load(&tx, owner, &intent.job_id)?;
        let row: Option<(String, bool, Option<String>)> = tx
            .query_row(
                "SELECT kind,completed,remote_id FROM batch_mutations WHERE id=?1 AND job_id=?2",
                params![intent.id, intent.job_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (kind, completed, known) = row.ok_or(StoreError::NotFound)?;
        if kind != intent.kind.label() {
            return Err(StoreError::Conflict);
        }
        if completed {
            if known.as_deref() != Some(remote) {
                return Err(StoreError::Conflict);
            }
            tx.commit()?;
            return Ok(job);
        }
        match intent.kind {
            BatchMutation::Upload if job.phase == BatchPhase::UploadPendingOutcome => {
                job.remote_input_file_id = Some(remote.into());
                job.phase = BatchPhase::Uploaded;
            }
            BatchMutation::Submit if job.phase == BatchPhase::SubmitPendingOutcome => {
                job.remote_batch_id = Some(remote.into());
                job.status = status;
                job.phase = if status.is_terminal() {
                    BatchPhase::Terminal
                } else {
                    BatchPhase::Submitted
                };
            }
            BatchMutation::Cancel
                if matches!(
                    job.phase,
                    BatchPhase::CancelPendingOutcome | BatchPhase::Terminal
                ) && job.remote_batch_id.as_deref() == Some(remote) =>
            {
                if !job.status.is_terminal() {
                    job.status = status;
                    job.phase = if status.is_terminal() {
                        BatchPhase::Terminal
                    } else {
                        BatchPhase::Submitted
                    };
                } else if status.is_terminal() && status != job.status {
                    return Err(StoreError::Conflict);
                }
            }
            _ => return Err(StoreError::Conflict),
        }
        tx.execute(
            "UPDATE batch_mutations SET completed=1,remote_id=?1 WHERE id=?2",
            params![remote, intent.id],
        )?;
        save(&tx, &mut job)?;
        tx.commit()?;
        Ok(job)
    }

    /// Polling is read-only upstream. It can reconcile cancellation after a
    /// lost response, but can never invent an identity for an uncertain submit.
    pub fn observe_status(
        &self,
        owner: &str,
        id: &str,
        remote: &str,
        status: BatchStatus,
    ) -> Result<BatchJob> {
        let _guard = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = load(&tx, owner, id)?;
        if job.remote_batch_id.as_deref() != Some(remote)
            || !matches!(
                job.phase,
                BatchPhase::Submitted | BatchPhase::CancelPendingOutcome | BatchPhase::Terminal
            )
            || (job.status.is_terminal() && job.status != status)
        {
            return Err(StoreError::Conflict);
        }
        if !status.is_terminal() && status_rank(status) < status_rank(job.status) {
            tx.commit()?;
            return Ok(job);
        }
        if status == BatchStatus::Cancelling || status.is_terminal() {
            tx.execute("UPDATE batch_mutations SET completed=1,remote_id=?1 WHERE job_id=?2 AND kind='cancel' AND completed=0", params![remote,id])?;
        }
        job.status = status;
        job.phase = if status.is_terminal() {
            BatchPhase::Terminal
        } else {
            job.phase
        };
        save(&tx, &mut job)?;
        tx.commit()?;
        Ok(job)
    }

    pub fn reject_unsubmitted(&self, owner: &str, id: &str) -> Result<BatchJob> {
        let _guard = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = load(&tx, owner, id)?;
        if !matches!(job.phase, BatchPhase::Draft | BatchPhase::Prepared) {
            return Err(StoreError::Conflict);
        }
        job.phase = BatchPhase::RejectedBeforeSubmission;
        job.status = BatchStatus::Failed;
        job.accounting_settled = true;
        tx.execute("DELETE FROM batch_reservations WHERE job_id=?1", [id])?;
        tx.execute("DELETE FROM file_references WHERE job_id=?1", [id])?;
        save(&tx, &mut job)?;
        tx.commit()?;
        Ok(job)
    }

    pub fn reserve_batch(
        &self,
        owner: &str,
        id: &str,
        usage: BatchUsage,
        budgets: &[BatchBudget],
    ) -> Result<BatchJob> {
        usage.validate()?;
        let _guard = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = load(&tx, owner, id)?;
        if job.phase != BatchPhase::Prepared || usage.requests != job.rows as u64 {
            return Err(StoreError::Conflict);
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM batch_reservations WHERE job_id=?1)",
            [id],
            |r| r.get(0),
        )?;
        if exists {
            return Err(StoreError::Conflict);
        }
        for budget in budgets {
            if !budget.limit.is_finite()
                || budget.limit < 0.0
                || !budget.ordinary_usage.is_finite()
                || budget.ordinary_usage < 0.0
            {
                return Err(StoreError::Invalid("Invalid quota budget".into()));
            }
            let mut current = ledger(&tx, owner, budget.window_start)?;
            current.add(&usage);
            let amount = budget.metric.amount(&current).ok_or(StoreError::Quota)?;
            if amount + budget.ordinary_usage > budget.limit {
                return Err(StoreError::Quota);
            }
        }
        tx.execute(
            "INSERT INTO batch_reservations VALUES (?1,?2)",
            params![id, encode(&usage)?],
        )?;
        save(&tx, &mut job)?;
        tx.commit()?;
        Ok(job)
    }

    pub fn batch_usage(&self, owner: &str, window_start: i64) -> Result<BatchUsage> {
        // Ordinary synchronous traffic must not initialize a payload database.
        if !self.root.join("batches.sqlite3").exists() {
            return Ok(BatchUsage::default());
        }
        let _guard = self.lock.lock();
        ledger(&self.open()?, owner, window_start)
    }

    /// Receipts are restricted to the immutable custom-ID manifest. The same
    /// result cannot be charged twice, even after restart or concurrent polls.
    pub fn record_batch_receipt(
        &self,
        owner: &str,
        id: &str,
        custom_id: &str,
        fingerprint: &str,
        usage: BatchUsage,
    ) -> Result<bool> {
        Ok(self.record_batch_receipts(
            owner,
            id,
            &[BatchReceipt {
                custom_id: custom_id.into(),
                fingerprint: fingerprint.into(),
                usage,
            }],
        )? != 0)
    }

    /// Ingest a bounded result set in one atomic transaction, rather than
    /// synchronously committing once per row. A conflicting row rolls back all
    /// new receipts in the set; the outstanding reservation remains intact.
    pub fn record_batch_receipts(
        &self,
        owner: &str,
        id: &str,
        receipts: &[BatchReceipt],
    ) -> Result<usize> {
        if receipts.is_empty() || receipts.len() > MAX_ROWS {
            return Err(StoreError::Invalid("Invalid receipt count".into()));
        }
        for receipt in receipts {
            receipt.usage.validate()?;
            if !digest(&receipt.fingerprint)
                || receipt.usage.requests > 1
                || receipt.custom_id.len() > 256
            {
                return Err(StoreError::Invalid("Invalid receipt".into()));
            }
        }
        let _guard = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = load(&tx, owner, id)?;
        if job.remote_batch_id.is_none() {
            return Err(StoreError::Conflict);
        }
        let mut inserted = 0;
        let observed_at = chrono::Utc::now().timestamp();
        for receipt in receipts {
            let previous: Option<(Option<String>, Option<String>)> = tx.query_row(
                "SELECT receipt_fingerprint,usage FROM batch_rows WHERE job_id=?1 AND custom_id=?2",
                params![id, receipt.custom_id], |r| Ok((r.get(0)?,r.get(1)?)),
            ).optional()?;
            let (known, body) = previous.ok_or(StoreError::NotFound)?;
            let serialized = encode(&receipt.usage)?;
            if let Some(known) = known {
                if known != receipt.fingerprint || body.as_deref() != Some(&serialized) {
                    return Err(StoreError::Conflict);
                }
                continue;
            }
            if job.accounting_settled {
                return Err(StoreError::Conflict);
            }
            tx.execute("UPDATE batch_rows SET receipt_fingerprint=?1,usage=?2,observed_at=?3 WHERE job_id=?4 AND custom_id=?5",params![receipt.fingerprint,serialized,observed_at,id,receipt.custom_id])?;
            inserted += 1;
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Release reservations only after a proven terminal result for every row.
    /// Unexecuted rows need explicit zero-usage receipts from verified status,
    /// never an assumption based on missing output or a request timeout.
    pub fn settle_batch_accounting(&self, owner: &str, id: &str) -> Result<BatchJob> {
        let _guard = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = load(&tx, owner, id)?;
        if !job.status.is_terminal() || job.remote_batch_id.is_none() {
            return Err(StoreError::Conflict);
        }
        let missing: i64 = tx.query_row(
            "SELECT count(*) FROM batch_rows WHERE job_id=?1 AND receipt_fingerprint IS NULL",
            [id],
            |r| r.get(0),
        )?;
        if missing != 0 {
            return Err(StoreError::Conflict);
        }
        if !job.accounting_settled {
            job.accounting_settled = true;
            tx.execute("DELETE FROM batch_reservations WHERE job_id=?1", [id])?;
            tx.execute("DELETE FROM file_references WHERE job_id=?1", [id])?;
            save(&tx, &mut job)?;
        }
        tx.commit()?;
        Ok(job)
    }
}

fn ledger(connection: &Connection, owner: &str, window_start: i64) -> Result<BatchUsage> {
    let mut statement=connection.prepare("SELECT j.id,r.usage FROM batch_jobs j LEFT JOIN batch_reservations r ON r.job_id=j.id WHERE j.owner=?1")?;
    let jobs = statement
        .query_map([owner], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut total = BatchUsage::default();
    let mut receipts=connection.prepare("SELECT usage FROM batch_rows WHERE job_id=?1 AND usage IS NOT NULL AND (?2 OR observed_at>=?3)")?;
    for (id, reservation) in jobs {
        let mut actual = BatchUsage::default();
        let rows = receipts.query_map(params![id, reservation.is_some(), window_start], |r| {
            r.get::<_, String>(0)
        })?;
        for row in rows {
            actual.add(&decode::<BatchUsage>(&row?)?);
        }
        if let Some(reserved) = reservation {
            actual.maximum(&decode::<BatchUsage>(&reserved)?);
        }
        total.add(&actual);
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, BatchStore, String) {
        let root = tempfile::tempdir().unwrap();
        let store = BatchStore::new(root.path().join("store"));
        let file = store
            .upload(
                "alice",
                "input.jsonl",
                "batch",
                b"{\"custom_id\":\"one\",\"method\":\"POST\",\"url\":\"/v1/chat/completions\",\"body\":{\"model\":\"explicit-model\",\"messages\":[]}}\n{\"custom_id\":\"two\",\"method\":\"POST\",\"url\":\"/v1/chat/completions\",\"body\":{\"model\":\"explicit-model\",\"messages\":[]}}\n",
                3600,
            )
            .unwrap();
        (root, store, file.id)
    }
    fn new(file: &str) -> NewBatchJob {
        NewBatchJob {
            owner: "alice".into(),
            input_file_id: file.into(),
            provider_instance: "openai-instance".into(),
            credential_binding: "a".repeat(64),
            endpoint: "/v1/chat/completions".into(),
            model: "explicit-model".into(),
            request_fingerprint: "b".repeat(64),
            policy_fingerprint: "c".repeat(64),
            idempotency_key_hash: Some("d".repeat(64)),
            custom_ids: vec!["one".into(), "two".into()],
        }
    }
    fn usage() -> BatchUsage {
        BatchUsage {
            requests: 2,
            input_tokens: 200,
            output_tokens: 100,
            cost_usd: Some(0.02),
        }
    }
    fn prepare(store: &BatchStore, job: BatchJob) -> BatchJob {
        store
            .prepare_job(
                &job.owner,
                &job.id,
                job.revision,
                &job.input_file_id,
                &job.policy_fingerprint,
            )
            .unwrap()
    }
    fn reserved(store: &BatchStore, file: &str) -> BatchJob {
        let (job, created) = store.create_job(new(file)).unwrap();
        assert!(created);
        let job = prepare(store, job);
        store.reserve_batch("alice", &job.id, usage(), &[]).unwrap()
    }
    fn submitted(store: &BatchStore, file: &str) -> BatchJob {
        let job = reserved(store, file);
        let intent = store
            .begin_mutation("alice", &job.id, job.revision, BatchMutation::Submit)
            .unwrap();
        store
            .complete_mutation("alice", &intent, "batch_remote", BatchStatus::InProgress)
            .unwrap()
    }
    #[test]
    fn idempotency_and_input_references_are_owner_scoped_and_durable() {
        let (_dir, store, file) = fixture();
        let (job, created) = store.create_job(new(&file)).unwrap();
        assert!(created);
        assert!(matches!(
            store.delete("alice", &file),
            Err(StoreError::Conflict)
        ));
        let restart = BatchStore::new(store.root.clone());
        let (again, created) = restart.create_job(new(&file)).unwrap();
        assert!(!created);
        assert_eq!(again.id, job.id);
        assert!(matches!(
            restart.get_job("bob", &job.id),
            Err(StoreError::NotFound)
        ));
        assert!(restart.list_jobs("bob").unwrap().is_empty());
        assert!(matches!(
            restart.reject_unsubmitted("bob", &job.id),
            Err(StoreError::NotFound)
        ));
        let mut changed = new(&file);
        changed.request_fingerprint = "e".repeat(64);
        assert!(matches!(
            restart.create_job(changed),
            Err(StoreError::Conflict)
        ));
        let mut unowned = new(&file);
        unowned.owner = "bob".into();
        assert!(matches!(
            restart.create_job(unowned),
            Err(StoreError::NotFound)
        ));
        restart.reject_unsubmitted("alice", &job.id).unwrap();
        restart.delete("alice", &file).unwrap();
        // Resolve before loading a deleted input or consulting a rotated key.
        assert_eq!(
            restart
                .find_idempotent_job("alice", &"d".repeat(64), &"b".repeat(64))
                .unwrap()
                .unwrap()
                .id,
            job.id
        );
        assert!(restart
            .find_idempotent_job("bob", &"d".repeat(64), &"b".repeat(64))
            .unwrap()
            .is_none());
        assert!(matches!(
            restart.find_idempotent_job("alice", &"d".repeat(64), &"e".repeat(64)),
            Err(StoreError::Conflict)
        ));
        // A known idempotent response survives input deletion after settlement.
        assert_eq!(restart.create_job(new(&file)).unwrap().0.id, job.id);
    }
    #[test]
    fn concurrent_idempotent_creates_have_one_manifest_and_reference() {
        let (_dir, store, file) = fixture();
        let threads = (0..4)
            .map(|_| {
                let store = BatchStore::new(store.root.clone());
                let file = file.clone();
                std::thread::spawn(move || store.create_job(new(&file)).unwrap())
            })
            .collect::<Vec<_>>();
        let jobs = threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(jobs.iter().filter(|(_, created)| *created).count(), 1);
        assert!(jobs.iter().all(|(job, _)| job.id == jobs[0].0.id));
        let connection = store.open().unwrap();
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM batch_rows", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM file_references", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn lost_upload_and_submit_responses_never_allow_another_create() {
        let (_dir, store, file) = fixture();
        let job = reserved(&store, &file);
        store
            .begin_mutation("alice", &job.id, job.revision, BatchMutation::Upload)
            .unwrap();
        let restart = BatchStore::new(store.root.clone());
        let unknown = restart.get_job("alice", &job.id).unwrap();
        assert_eq!(unknown.phase, BatchPhase::UploadPendingOutcome);
        let upload = restart
            .pending_mutations("alice", &job.id)
            .unwrap()
            .remove(0);
        assert!(matches!(
            restart.pending_mutations("bob", &job.id),
            Err(StoreError::NotFound)
        ));
        assert!(matches!(
            restart.begin_mutation("alice", &job.id, unknown.revision, BatchMutation::Upload),
            Err(StoreError::Conflict)
        ));
        assert!(matches!(
            restart.begin_mutation("alice", &job.id, unknown.revision, BatchMutation::Submit),
            Err(StoreError::Conflict)
        ));
        assert!(matches!(
            restart.reject_unsubmitted("alice", &job.id),
            Err(StoreError::Conflict)
        ));
        let uploaded = restart
            .complete_mutation("alice", &upload, "file_remote", BatchStatus::Validating)
            .unwrap();
        assert_eq!(
            uploaded.remote_input_file_id.as_deref(),
            Some("file_remote")
        );
        assert_eq!(
            restart
                .complete_mutation("alice", &upload, "file_remote", BatchStatus::Validating)
                .unwrap()
                .revision,
            uploaded.revision
        );
        assert!(matches!(
            restart.complete_mutation("alice", &upload, "different_file", BatchStatus::Validating),
            Err(StoreError::Conflict)
        ));
        let submit = restart
            .begin_mutation("alice", &job.id, uploaded.revision, BatchMutation::Submit)
            .unwrap();
        let restart = BatchStore::new(store.root.clone());
        let unknown = restart.get_job("alice", &job.id).unwrap();
        assert_eq!(unknown.phase, BatchPhase::SubmitPendingOutcome);
        assert!(matches!(
            restart.observe_status("alice", &job.id, "arbitrary_remote", BatchStatus::Completed),
            Err(StoreError::Conflict)
        ));
        assert!(matches!(
            restart.begin_mutation("alice", &job.id, unknown.revision, BatchMutation::Submit),
            Err(StoreError::Conflict)
        ));
        let done = restart
            .complete_mutation("alice", &submit, "batch_remote", BatchStatus::InProgress)
            .unwrap();
        assert_eq!(done.remote_batch_id.as_deref(), Some("batch_remote"));
        assert!(matches!(
            restart.begin_mutation("alice", &job.id, done.revision, BatchMutation::Submit),
            Err(StoreError::Conflict)
        ));
        assert_eq!(restart.batch_usage("alice", i64::MAX).unwrap(), usage());
    }
    #[test]
    fn pending_usage_never_expires_or_double_counts_partial_receipts() {
        let (_dir, store, file) = fixture();
        let job = submitted(&store, &file);
        let one = BatchUsage {
            requests: 1,
            input_tokens: 50,
            output_tokens: 10,
            cost_usd: Some(0.003),
        };
        assert!(store
            .record_batch_receipt("alice", &job.id, "one", &"1".repeat(64), one.clone())
            .unwrap());
        assert!(!store
            .record_batch_receipt("alice", &job.id, "one", &"1".repeat(64), one.clone())
            .unwrap());
        assert!(matches!(
            store.record_batch_receipt("alice", &job.id, "one", &"2".repeat(64), one),
            Err(StoreError::Conflict)
        ));
        assert!(matches!(
            store.record_batch_receipt(
                "alice",
                &job.id,
                "unowned-row",
                &"1".repeat(64),
                BatchUsage::default()
            ),
            Err(StoreError::NotFound)
        ));
        assert_eq!(store.batch_usage("alice", i64::MAX).unwrap(), usage());
        assert_eq!(store.batch_usage("bob", 0).unwrap(), BatchUsage::default());
        store
            .observe_status("alice", &job.id, "batch_remote", BatchStatus::Cancelled)
            .unwrap();
        assert!(matches!(
            store.settle_batch_accounting("alice", &job.id),
            Err(StoreError::Conflict)
        ));
        assert!(matches!(
            store.delete("alice", &file),
            Err(StoreError::Conflict)
        ));
        store
            .record_batch_receipt(
                "alice",
                &job.id,
                "two",
                &"2".repeat(64),
                BatchUsage::default(),
            )
            .unwrap();
        let settled = store.settle_batch_accounting("alice", &job.id).unwrap();
        assert!(settled.accounting_settled);
        let revision = store
            .settle_batch_accounting("alice", &job.id)
            .unwrap()
            .revision;
        assert_eq!(settled.revision, revision);
        let actual = store.batch_usage("alice", 0).unwrap();
        assert_eq!(actual.requests, 1);
        assert_eq!(actual.cost_usd, Some(0.003));
        assert_eq!(
            store.batch_usage("alice", i64::MAX).unwrap(),
            BatchUsage::default()
        );
        store.delete("alice", &file).unwrap();
    }
    #[test]
    fn reservation_checks_are_atomic_and_include_ordinary_and_pending_usage() {
        let (_dir, store, file) = fixture();
        let (first, _) = store.create_job(new(&file)).unwrap();
        let first = prepare(&store, first);
        let mut second = new(&file);
        second.idempotency_key_hash = None;
        let (second, _) = store.create_job(second).unwrap();
        let second = prepare(&store, second);
        let budget = BatchBudget {
            metric: BatchMetric::Requests,
            window_start: 0,
            limit: 5.0,
            ordinary_usage: 2.0,
        };
        store
            .reserve_batch("alice", &first.id, usage(), std::slice::from_ref(&budget))
            .unwrap();
        assert!(matches!(
            store.reserve_batch("alice", &second.id, usage(), &[budget]),
            Err(StoreError::Quota)
        ));
        assert!(matches!(
            store.begin_mutation("alice", &second.id, second.revision, BatchMutation::Submit),
            Err(StoreError::Conflict)
        ));
        assert!(matches!(
            store.reserve_batch("alice", &first.id, usage(), &[]),
            Err(StoreError::Conflict)
        ));
        store.reject_unsubmitted("alice", &first.id).unwrap();
        let cost = BatchBudget {
            metric: BatchMetric::Cost,
            window_start: 0,
            limit: 0.019,
            ordinary_usage: 0.0,
        };
        assert!(matches!(
            store.reserve_batch("alice", &second.id, usage(), &[cost]),
            Err(StoreError::Quota)
        ));
        let unknown = BatchUsage {
            cost_usd: None,
            ..usage()
        };
        let cost = BatchBudget {
            metric: BatchMetric::Cost,
            window_start: 0,
            limit: 1e8,
            ordinary_usage: 0.0,
        };
        assert!(matches!(
            store.reserve_batch("alice", &second.id, unknown, &[cost]),
            Err(StoreError::Quota)
        ));
        assert_eq!(
            store.batch_usage("alice", 0).unwrap(),
            BatchUsage::default()
        );
    }
    #[test]
    fn unknown_and_above_estimate_spend_remain_visible_after_restart() {
        let (_dir, store, file) = fixture();
        let job = submitted(&store, &file);
        store
            .record_batch_receipt(
                "alice",
                &job.id,
                "one",
                &"1".repeat(64),
                BatchUsage {
                    requests: 1,
                    input_tokens: 500,
                    output_tokens: 200,
                    cost_usd: None,
                },
            )
            .unwrap();
        let restart = BatchStore::new(store.root.clone());
        let actual = restart.batch_usage("alice", 0).unwrap();
        assert_eq!(actual.input_tokens, 500);
        assert_eq!(actual.output_tokens, 200);
        assert_eq!(actual.cost_usd, None);
        restart
            .record_batch_receipt(
                "alice",
                &job.id,
                "two",
                &"2".repeat(64),
                BatchUsage::default(),
            )
            .unwrap();
        restart
            .observe_status("alice", &job.id, "batch_remote", BatchStatus::Completed)
            .unwrap();
        restart.settle_batch_accounting("alice", &job.id).unwrap();
        assert_eq!(restart.batch_usage("alice", 0).unwrap().cost_usd, None);
    }
    #[test]
    fn credential_binding_salt_is_private_durable_and_not_process_scoped() {
        let (_dir, store, _file) = fixture();
        let salt = store.binding_salt().unwrap();
        assert!(digest(&salt));
        assert_eq!(
            BatchStore::new(store.root.clone()).binding_salt().unwrap(),
            salt
        );
    }
    #[test]
    fn manifest_bounds_and_nonfinite_usage_are_rejected() {
        let (_dir, store, file) = fixture();
        let mut invalid = new(&file);
        invalid.custom_ids = vec!["same".into(), "same".into()];
        assert!(matches!(
            store.create_job(invalid),
            Err(StoreError::Invalid(_))
        ));
        let job = store.create_job(new(&file)).unwrap().0;
        for cost in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(matches!(
                store.reserve_batch(
                    "alice",
                    &job.id,
                    BatchUsage {
                        cost_usd: Some(cost),
                        ..usage()
                    },
                    &[]
                ),
                Err(StoreError::Invalid(_))
            ));
        }
        assert_eq!(store.get_job("alice", &job.id).unwrap().revision, 0);
    }
    #[test]
    fn stale_cancel_ack_and_poll_cannot_regress_terminal_status() {
        let (_dir, store, file) = fixture();
        let job = submitted(&store, &file);
        let intent = store
            .begin_mutation("alice", &job.id, job.revision, BatchMutation::Cancel)
            .unwrap();
        let completed = store
            .observe_status("alice", &job.id, "batch_remote", BatchStatus::Completed)
            .unwrap();
        assert_eq!(completed.phase, BatchPhase::Terminal);
        let ack = store
            .complete_mutation("alice", &intent, "batch_remote", BatchStatus::Cancelling)
            .unwrap();
        assert_eq!(ack.status, BatchStatus::Completed);
        assert!(store
            .pending_mutations("alice", &job.id)
            .unwrap()
            .is_empty());
        assert!(matches!(
            store.observe_status("alice", &job.id, "batch_remote", BatchStatus::InProgress),
            Err(StoreError::Conflict)
        ));
    }
    #[test]
    fn bulk_receipts_are_atomic_and_deduplicated() {
        let (_dir, store, file) = fixture();
        let job = submitted(&store, &file);
        let row = || BatchReceipt {
            custom_id: "one".into(),
            fingerprint: "1".repeat(64),
            usage: BatchUsage {
                requests: 1,
                input_tokens: 10,
                output_tokens: 5,
                cost_usd: Some(0.001),
            },
        };
        let invalid = BatchReceipt {
            custom_id: "not-in-manifest".into(),
            fingerprint: "2".repeat(64),
            usage: BatchUsage::default(),
        };
        assert!(matches!(
            store.record_batch_receipts("alice", &job.id, &[row(), invalid]),
            Err(StoreError::NotFound)
        ));
        // The valid first row was rolled back, so it is inserted now.
        assert!(store
            .record_batch_receipt("alice", &job.id, "one", &"1".repeat(64), row().usage)
            .unwrap());
        assert_eq!(
            store
                .record_batch_receipts("alice", &job.id, &[row(), row()])
                .unwrap(),
            0
        );
    }
    #[test]
    fn concurrent_reservations_cannot_overcommit() {
        let (_dir, store, file) = fixture();
        let first = prepare(&store, store.create_job(new(&file)).unwrap().0);
        let mut second = new(&file);
        second.idempotency_key_hash = None;
        let second = prepare(&store, store.create_job(second).unwrap().0);
        let threads = [first, second]
            .into_iter()
            .map(|job| {
                let store = BatchStore::new(store.root.clone());
                std::thread::spawn(move || {
                    store.reserve_batch(
                        "alice",
                        &job.id,
                        usage(),
                        &[BatchBudget {
                            metric: BatchMetric::Requests,
                            window_start: 0,
                            limit: 3.0,
                            ordinary_usage: 0.0,
                        }],
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Err(StoreError::Quota)))
                .count(),
            1
        );
        assert_eq!(store.batch_usage("alice", 0).unwrap().requests, 2);
    }
    #[test]
    fn explicit_cancel_retries_are_bounded_and_never_recreate_inference() {
        let (_dir, store, file) = fixture();
        let job = submitted(&store, &file);
        for _ in 0..8 {
            let latest = store.get_job("alice", &job.id).unwrap();
            store
                .begin_mutation("alice", &job.id, latest.revision, BatchMutation::Cancel)
                .unwrap();
        }
        let latest = store.get_job("alice", &job.id).unwrap();
        assert!(matches!(
            store.begin_mutation("alice", &job.id, latest.revision, BatchMutation::Cancel),
            Err(StoreError::Quota)
        ));
        assert!(matches!(
            store.begin_mutation("alice", &job.id, latest.revision, BatchMutation::Submit),
            Err(StoreError::Conflict)
        ));
        store
            .observe_status("alice", &job.id, "batch_remote", BatchStatus::Cancelled)
            .unwrap();
        assert!(store
            .pending_mutations("alice", &job.id)
            .unwrap()
            .is_empty());
    }
    #[test]
    fn draft_cannot_reserve_or_dispatch_and_snapshot_must_match_manifest() {
        let (_dir, store, file) = fixture();
        let job = store.create_job(new(&file)).unwrap().0;
        assert_eq!(job.phase, BatchPhase::Draft);
        assert!(matches!(
            store.reserve_batch("alice", &job.id, usage(), &[]),
            Err(StoreError::Conflict)
        ));
        assert!(matches!(
            store.begin_mutation("alice", &job.id, job.revision, BatchMutation::Submit),
            Err(StoreError::Conflict)
        ));
        let wrong = store
            .upload(
                "alice",
                "wrong.jsonl",
                "batch",
                b"{\"custom_id\":\"other\"}\n{}\n",
                3600,
            )
            .unwrap();
        assert!(matches!(
            store.prepare_job(
                "alice",
                &job.id,
                job.revision,
                &wrong.id,
                &job.policy_fingerprint
            ),
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            store.prepare_job("alice", &job.id, job.revision, &file, &"e".repeat(64)),
            Err(StoreError::Conflict)
        ));
        let original = String::from_utf8(store.content("alice", &file).unwrap()).unwrap();
        let duplicate = original.replace(
            "\"model\":\"explicit-model\"",
            "\"model\":\"other\",\"model\":\"explicit-model\"",
        );
        let duplicate = store
            .upload(
                "alice",
                "duplicate.jsonl",
                "batch",
                duplicate.as_bytes(),
                3600,
            )
            .unwrap();
        assert!(matches!(
            store.prepare_job(
                "alice",
                &job.id,
                job.revision,
                &duplicate.id,
                &job.policy_fingerprint
            ),
            Err(StoreError::Invalid(_))
        ));
        let prepared = prepare(&store, job);
        assert_eq!(prepared.prepared_snapshot_file_id, Some(file));
        assert!(prepared.snapshot_fingerprint.as_deref().is_some_and(digest));
    }
    #[test]
    fn active_job_limits_are_global_and_owner_scoped() {
        let (_dir, store, file) = fixture();
        let content = store.content("alice", &file).unwrap();
        for owner in ["alice", "bob"] {
            let input = if owner == "alice" {
                file.clone()
            } else {
                store
                    .upload(owner, "input.jsonl", "batch", &content, 3600)
                    .unwrap()
                    .id
            };
            for _ in 0..8 {
                let mut request = new(&input);
                request.owner = owner.into();
                request.idempotency_key_hash = None;
                store.create_job(request).unwrap();
            }
            let mut ninth = new(&input);
            ninth.owner = owner.into();
            ninth.idempotency_key_hash = None;
            assert!(matches!(store.create_job(ninth), Err(StoreError::Quota)));
        }
        let input = store
            .upload("carol", "input.jsonl", "batch", &content, 3600)
            .unwrap()
            .id;
        let mut global = new(&input);
        global.owner = "carol".into();
        global.idempotency_key_hash = None;
        assert!(matches!(store.create_job(global), Err(StoreError::Quota)));
        assert_eq!(store.list_jobs("alice").unwrap().len(), 8);
        assert_eq!(store.list_jobs("bob").unwrap().len(), 8);
    }
}
