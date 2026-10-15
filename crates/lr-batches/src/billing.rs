//! Aggregate native accounting, separate from per-row result distribution.
use super::jobs::{decode, encode, load, save};
use super::*;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BatchCostSource {
    ProviderReported,
    ByokFeeAndInferenceEstimate,
    NativePriceEstimate,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct BatchBilling {
    pub source: BatchCostSource,
    pub quota_usage: BatchUsage,
    pub actual_input_tokens: Option<u64>,
    pub actual_output_tokens: Option<u64>,
    pub reported_charge_usd: Option<f64>,
    pub upstream_inference_estimate_usd: Option<f64>,
    pub is_byok: Option<bool>,
    pub successful_requests: u64,
    pub failed_requests: u64,
}
impl BatchBilling {
    fn validate(&self, rows: usize) -> Result<()> {
        self.quota_usage.validate()?;
        if self.quota_usage.requests > rows as u64
            || self.successful_requests.checked_add(self.failed_requests)
                != Some(self.quota_usage.requests)
            || self
                .actual_input_tokens
                .is_some_and(|v| v != self.quota_usage.input_tokens)
            || self
                .actual_output_tokens
                .is_some_and(|v| v != self.quota_usage.output_tokens)
            || [
                self.reported_charge_usd,
                self.upstream_inference_estimate_usd,
            ]
            .into_iter()
            .flatten()
            .any(|v| !v.is_finite() || !(0.0..=1e9).contains(&v))
        {
            return Err(StoreError::Invalid(
                "Invalid aggregate native accounting".into(),
            ));
        }
        let expected = match self.source {
            BatchCostSource::ProviderReported if self.is_byok != Some(true) => {
                self.reported_charge_usd
            }
            BatchCostSource::ByokFeeAndInferenceEstimate if self.is_byok == Some(true) => self
                .reported_charge_usd
                .zip(self.upstream_inference_estimate_usd)
                .map(|(fee, inference)| fee + inference),
            BatchCostSource::NativePriceEstimate if self.reported_charge_usd.is_none() => {
                self.upstream_inference_estimate_usd
            }
            BatchCostSource::Unknown => None,
            _ => {
                return Err(StoreError::Invalid(
                    "Native accounting cost provenance mismatch".into(),
                ))
            }
        };
        if expected != self.quota_usage.cost_usd {
            return Err(StoreError::Invalid(
                "Native accounting must preserve reported/estimated/unknown cost scope".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct BatchBillingRecord {
    pub billing: BatchBilling,
    pub fingerprint: String,
    pub observed_at: i64,
}
pub(super) fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS batch_billing (
        job_id TEXT PRIMARY KEY REFERENCES batch_jobs(id), fingerprint TEXT NOT NULL,
        observed_at INTEGER NOT NULL, record TEXT NOT NULL
    );",
    )?;
    Ok(())
}
impl BatchStore {
    /// Totals are evidence from the known owned terminal job, not an invented
    /// allocation across individual rows. Unknown cost may later become known;
    /// a previously known charge is immutable and conflicts remain visible.
    pub fn record_batch_billing(
        &self,
        owner: &str,
        id: &str,
        fingerprint: &str,
        billing: BatchBilling,
    ) -> Result<bool> {
        if fingerprint.len() != 64 || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(StoreError::Invalid(
                "Invalid native billing fingerprint".into(),
            ));
        }
        let _guard = self.lock.lock();
        let mut connection = self.open()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut job = load(&tx, owner, id)?;
        if !job.status.is_terminal() || job.remote_batch_id.is_none() {
            return Err(StoreError::Conflict);
        }
        billing.validate(job.rows)?;
        let old: Option<(String, i64, String)> = tx
            .query_row(
                "SELECT fingerprint,observed_at,record FROM batch_billing WHERE job_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let observed_at = if let Some((known, observed_at, record)) = old {
            let previous: BatchBilling = decode(&record)?;
            if known == fingerprint {
                if previous != billing {
                    return Err(StoreError::Conflict);
                }
                tx.commit()?;
                return Ok(false);
            }
            let estimate_replaced_by_report = previous.source
                == BatchCostSource::NativePriceEstimate
                && billing.source == BatchCostSource::ProviderReported
                && billing.reported_charge_usd.is_some();
            let compatible = previous.quota_usage.requests == billing.quota_usage.requests
                && previous.successful_requests == billing.successful_requests
                && previous.failed_requests == billing.failed_requests
                && previous.quota_usage.cost_usd.is_none_or(|v| {
                    Some(v) == billing.quota_usage.cost_usd || estimate_replaced_by_report
                })
                && previous
                    .actual_input_tokens
                    .is_none_or(|v| Some(v) == billing.actual_input_tokens)
                && previous
                    .actual_output_tokens
                    .is_none_or(|v| Some(v) == billing.actual_output_tokens)
                && previous
                    .reported_charge_usd
                    .is_none_or(|v| Some(v) == billing.reported_charge_usd)
                && previous
                    .upstream_inference_estimate_usd
                    .is_none_or(|v| Some(v) == billing.upstream_inference_estimate_usd);
            let gained = estimate_replaced_by_report
                || (previous.quota_usage.cost_usd.is_none()
                    && billing.quota_usage.cost_usd.is_some())
                || (previous.actual_input_tokens.is_none()
                    && billing.actual_input_tokens.is_some())
                || (previous.actual_output_tokens.is_none()
                    && billing.actual_output_tokens.is_some());
            if !compatible || !gained {
                return Err(StoreError::Conflict);
            }
            observed_at
        } else {
            chrono::Utc::now().timestamp()
        };
        tx.execute("INSERT INTO batch_billing VALUES (?1,?2,?3,?4) ON CONFLICT(job_id) DO UPDATE SET fingerprint=excluded.fingerprint,record=excluded.record",params![id,fingerprint,observed_at,encode(&billing)?])?;
        job.last_error = None;
        save(&tx, &mut job)?;
        tx.commit()?;
        Ok(true)
    }
    pub fn batch_billing(&self, owner: &str, id: &str) -> Result<Option<BatchBillingRecord>> {
        let _guard = self.lock.lock();
        let connection = self.open()?;
        load(&connection, owner, id)?;
        let row: Option<(String, i64, String)> = connection
            .query_row(
                "SELECT fingerprint,observed_at,record FROM batch_billing WHERE job_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        row.map(|(fingerprint, observed_at, record)| {
            Ok(BatchBillingRecord {
                billing: decode(&record)?,
                fingerprint,
                observed_at,
            })
        })
        .transpose()
    }
}
/// Native quota checks prefer proven job totals where per-row distribution is
/// unavailable. The timestamp is the first local observation, never caller input.
pub(super) fn aggregate(
    connection: &Connection,
    id: &str,
    window_start: i64,
    include_pending: bool,
) -> Result<Option<BatchUsage>> {
    let row: Option<(String, i64)> = connection
        .query_row(
            "SELECT record,observed_at FROM batch_billing WHERE job_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    row.map(|(record, observed_at)| {
        let billing: BatchBilling = decode(&record)?;
        Ok(if include_pending || observed_at >= window_start {
            billing.quota_usage
        } else {
            BatchUsage::default()
        })
    })
    .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn billing() -> BatchBilling {
        BatchBilling {
            source: BatchCostSource::ProviderReported,
            quota_usage: BatchUsage {
                requests: 1,
                input_tokens: 20,
                output_tokens: 10,
                cost_usd: Some(0.01),
            },
            actual_input_tokens: Some(20),
            actual_output_tokens: Some(10),
            reported_charge_usd: Some(0.01),
            upstream_inference_estimate_usd: None,
            is_byok: Some(false),
            successful_requests: 1,
            failed_requests: 0,
        }
    }
    #[test]
    fn estimates_can_be_replaced_by_a_proven_invoice_and_summary_stays_owner_scoped() {
        let (_dir, store, job) = super::super::artifacts::tests::fixture(1024);
        let job = super::super::artifacts::tests::terminal(&store, job);
        let mut estimate = billing();
        estimate.source = BatchCostSource::NativePriceEstimate;
        estimate.reported_charge_usd = None;
        estimate.upstream_inference_estimate_usd = Some(0.02);
        estimate.quota_usage.cost_usd = Some(0.02);
        store
            .record_batch_billing("alice", &job.id, &"d".repeat(64), estimate.clone())
            .unwrap();
        store.settle_batch_accounting("alice", &job.id).unwrap();
        let before = store
            .batch_billing("alice", &job.id)
            .unwrap()
            .unwrap()
            .observed_at;
        let mut report = estimate;
        report.source = BatchCostSource::ProviderReported;
        report.reported_charge_usd = Some(0.01);
        report.quota_usage.cost_usd = Some(0.01);
        store
            .record_batch_billing("alice", &job.id, &"e".repeat(64), report.clone())
            .unwrap();
        assert_eq!(
            store
                .batch_billing("alice", &job.id)
                .unwrap()
                .unwrap()
                .observed_at,
            before
        );
        assert_eq!(
            store
                .batch_usage_summary(Some("alice"), None)
                .unwrap()
                .known_reported_charge_usd,
            Some(0.01)
        );
        assert_eq!(
            store.batch_usage_summary(Some("bob"), None).unwrap().jobs,
            0
        );
        assert_eq!(
            store
                .batch_usage_summary(None, Some("unrelated"))
                .unwrap()
                .jobs,
            0
        );
        report.quota_usage.cost_usd = Some(0.03);
        report.reported_charge_usd = Some(0.03);
        assert!(store
            .record_batch_billing("alice", &job.id, &"f".repeat(64), report)
            .is_err());
    }

    #[test]
    fn aggregate_without_per_row_distribution_settles_once_with_provenance() {
        let (_dir, store, job) = super::super::artifacts::tests::fixture(1024);
        let job = super::super::artifacts::tests::terminal(&store, job);
        let mut report = billing();
        report.source = BatchCostSource::ByokFeeAndInferenceEstimate;
        report.is_byok = Some(true);
        report.reported_charge_usd = Some(0.001);
        report.upstream_inference_estimate_usd = Some(0.02);
        report.quota_usage.cost_usd = Some(0.001 + 0.02);
        assert!(store
            .record_batch_billing("alice", &job.id, &"d".repeat(64), report.clone())
            .unwrap());
        assert!(!store
            .record_batch_billing("alice", &job.id, &"d".repeat(64), report.clone())
            .unwrap());
        assert_eq!(store.batch_usage("alice", 0).unwrap().cost_usd, Some(0.1));
        store.settle_batch_accounting("alice", &job.id).unwrap();
        assert_eq!(
            store.batch_usage("alice", 0).unwrap().cost_usd,
            Some(0.001 + 0.02)
        );
        assert_eq!(store.batch_usage("alice", 0).unwrap().input_tokens, 20);
        assert!(matches!(
            store.batch_billing("bob", &job.id),
            Err(StoreError::NotFound)
        ));
        let record = store.batch_billing("alice", &job.id).unwrap().unwrap();
        assert_eq!(record.billing, report);
        assert_eq!(
            store.batch_usage("alice", i64::MAX).unwrap(),
            BatchUsage::default()
        );
    }
    #[test]
    fn unknown_cost_upgrades_without_retiming_or_duplicate_tokens() {
        let (_dir, store, job) = super::super::artifacts::tests::fixture(1024);
        let job = super::super::artifacts::tests::terminal(&store, job);
        let mut unknown = billing();
        unknown.source = BatchCostSource::Unknown;
        unknown.reported_charge_usd = None;
        unknown.quota_usage.cost_usd = None;
        store
            .record_batch_billing("alice", &job.id, &"d".repeat(64), unknown)
            .unwrap();
        store.settle_batch_accounting("alice", &job.id).unwrap();
        assert_eq!(store.batch_usage("alice", 0).unwrap().cost_usd, None);
        let before = store
            .batch_billing("alice", &job.id)
            .unwrap()
            .unwrap()
            .observed_at;
        store
            .record_batch_billing("alice", &job.id, &"e".repeat(64), billing())
            .unwrap();
        assert_eq!(store.batch_usage("alice", 0).unwrap().requests, 1);
        assert_eq!(store.batch_usage("alice", 0).unwrap().cost_usd, Some(0.01));
        assert_eq!(
            store
                .batch_billing("alice", &job.id)
                .unwrap()
                .unwrap()
                .observed_at,
            before
        );
        let mut changed = billing();
        changed.reported_charge_usd = Some(0.02);
        changed.quota_usage.cost_usd = Some(0.02);
        assert!(matches!(
            store.record_batch_billing("alice", &job.id, &"f".repeat(64), changed),
            Err(StoreError::Conflict)
        ));
    }
    #[test]
    fn malformed_counts_and_byok_scope_cannot_become_fake_total_charges() {
        let (_dir, store, job) = super::super::artifacts::tests::fixture(1024);
        let job = super::super::artifacts::tests::terminal(&store, job);
        let mut invalid = billing();
        invalid.successful_requests = u64::MAX;
        invalid.failed_requests = 1;
        assert!(matches!(
            store.record_batch_billing("alice", &job.id, &"d".repeat(64), invalid),
            Err(StoreError::Invalid(_))
        ));
        let mut invalid = billing();
        invalid.is_byok = Some(true);
        assert!(matches!(
            store.record_batch_billing("alice", &job.id, &"d".repeat(64), invalid),
            Err(StoreError::Invalid(_))
        ));
        assert!(store.batch_billing("alice", &job.id).unwrap().is_none());
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct BatchUsageSummary {
    pub jobs: u64,
    pub terminal_jobs: u64,
    pub active_jobs: u64,
    pub unknown_submission_jobs: u64,
    pub requested_rows: u64,
    pub known_completed_requests: u64,
    pub known_failed_requests: u64,
    pub known_input_tokens: Option<u64>,
    pub known_output_tokens: Option<u64>,
    pub known_reported_charge_usd: Option<f64>,
    pub known_inference_estimate_usd: Option<f64>,
    pub known_budget_usage_usd: Option<f64>,
    pub jobs_with_unknown_input_usage: u64,
    pub jobs_with_unknown_output_usage: u64,
    pub jobs_with_unknown_cost: u64,
    pub reserved_cost_usd: Option<f64>,
    pub jobs_with_unknown_reservation_cost: u64,
}
impl BatchStore {
    /// Native asynchronous usage has its own durable metrics. No synchronous
    /// latency, fake zero charge or per-row distribution is fabricated. An
    /// owner=None query is for the trusted desktop administrator only; SDK
    /// handlers must always supply their authenticated owner.
    pub fn batch_usage_summary(
        &self,
        owner: Option<&str>,
        provider: Option<&str>,
    ) -> Result<BatchUsageSummary> {
        let mut summary = BatchUsageSummary {
            jobs: 0,
            terminal_jobs: 0,
            active_jobs: 0,
            unknown_submission_jobs: 0,
            requested_rows: 0,
            known_completed_requests: 0,
            known_failed_requests: 0,
            known_input_tokens: None,
            known_output_tokens: None,
            known_reported_charge_usd: None,
            known_inference_estimate_usd: None,
            known_budget_usage_usd: None,
            jobs_with_unknown_input_usage: 0,
            jobs_with_unknown_output_usage: 0,
            jobs_with_unknown_cost: 0,
            reserved_cost_usd: None,
            jobs_with_unknown_reservation_cost: 0,
        };
        if !self.root.join("batches.sqlite3").exists() {
            return Ok(summary);
        }
        let _guard = self.lock.lock();
        let connection = self.open()?;
        let mut statement=connection.prepare("SELECT j.record,b.record,r.usage FROM batch_jobs j LEFT JOIN batch_billing b ON b.job_id=j.id LEFT JOIN batch_reservations r ON r.job_id=j.id WHERE (?1 IS NULL OR j.owner=?1)")?;
        let rows = statement.query_map([owner], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (job, billing, reservation) = row?;
            let job: BatchJob = decode(&job)?;
            if provider.is_some_and(|p| p != job.provider_instance) {
                continue;
            }
            summary.jobs += 1;
            summary.requested_rows += job.rows as u64;
            if job.status.is_terminal() {
                summary.terminal_jobs += 1
            } else {
                summary.active_jobs += 1
            }
            if matches!(
                job.phase,
                BatchPhase::UploadPendingOutcome | BatchPhase::SubmitPendingOutcome
            ) {
                summary.unknown_submission_jobs += 1;
            }
            if let Some(billing) = billing {
                let billing: BatchBilling = decode(&billing)?;
                summary.known_completed_requests += billing.successful_requests;
                summary.known_failed_requests += billing.failed_requests;
                if let Some(v) = billing.actual_input_tokens {
                    summary.known_input_tokens = Some(summary.known_input_tokens.unwrap_or(0) + v)
                } else {
                    summary.jobs_with_unknown_input_usage += 1
                }
                if let Some(v) = billing.actual_output_tokens {
                    summary.known_output_tokens = Some(summary.known_output_tokens.unwrap_or(0) + v)
                } else {
                    summary.jobs_with_unknown_output_usage += 1
                }
                if let Some(v) = billing.reported_charge_usd {
                    summary.known_reported_charge_usd =
                        Some(summary.known_reported_charge_usd.unwrap_or(0.0) + v)
                }
                if let Some(v) = billing.upstream_inference_estimate_usd {
                    summary.known_inference_estimate_usd =
                        Some(summary.known_inference_estimate_usd.unwrap_or(0.0) + v)
                }
                if let Some(v) = billing.quota_usage.cost_usd {
                    summary.known_budget_usage_usd =
                        Some(summary.known_budget_usage_usd.unwrap_or(0.0) + v)
                } else {
                    summary.jobs_with_unknown_cost += 1
                }
            } else if job.phase != BatchPhase::RejectedBeforeSubmission
                && !(job.accounting_settled && job.remote_batch_id.is_none())
            {
                summary.jobs_with_unknown_input_usage += 1;
                summary.jobs_with_unknown_output_usage += 1;
                summary.jobs_with_unknown_cost += 1;
            }
            if let Some(reservation) = reservation {
                let reservation: BatchUsage = decode(&reservation)?;
                if let Some(v) = reservation.cost_usd {
                    summary.reserved_cost_usd = Some(summary.reserved_cost_usd.unwrap_or(0.0) + v)
                } else {
                    summary.jobs_with_unknown_reservation_cost += 1
                }
            }
        }
        Ok(summary)
    }
}
