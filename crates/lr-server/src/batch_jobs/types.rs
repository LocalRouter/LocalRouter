use lr_batches::{
    BatchBillingRecord, BatchJobError, BatchPhase, BatchResultAlias, BatchStatus, BatchUsageSummary,
};
use lr_providers::{batches::BatchOutputExpiry, gateway::GatewayPreferences};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct BatchLocalOptions {
    pub result_limit_bytes: Option<u64>,
    pub local_retention_seconds: Option<u32>,
    pub prepare_only: Option<bool>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct BatchCreateExtensions {
    #[serde(rename = "localrouter.gateway")]
    pub gateway: Option<GatewayPreferences>,
    #[serde(rename = "localrouter.batch")]
    pub batch: Option<BatchLocalOptions>,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateBatchRequest {
    pub input_file_id: String,
    pub endpoint: String,
    pub completion_window: String,
    pub metadata: Option<HashMap<String, String>>,
    pub output_expires_after: Option<BatchOutputExpiry>,
    pub extensions: Option<BatchCreateExtensions>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InputRow {
    pub custom_id: String,
    pub method: String,
    pub url: String,
    pub body: serde_json::Value,
}
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BatchRequestCounts {
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
}
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BatchErrorList {
    pub object: String,
    pub data: Vec<BatchJobError>,
}
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BatchLocalState {
    pub phase: BatchPhase,
    pub provider_instance: String,
    pub model: String,
    pub validated_rows: usize,
    pub worker_running: bool,
    pub can_resume_validation: bool,
    pub can_submit_prepared: bool,
    pub reserved_usage: Option<lr_batches::BatchUsage>,
    pub native_price: Option<lr_batches::NativeBatchPrice>,
    pub cancellation_supported: bool,
    pub remote_batch_deletion_supported: bool,
    pub request_counts_known: bool,
    pub prepared_snapshot_file_id: Option<String>,
    pub snapshot_fingerprint: Option<String>,
    pub result_limit_bytes: u64,
    pub local_retention_seconds: u32,
    pub results: Vec<BatchResultAlias>,
    pub accounting: Option<BatchBillingRecord>,
    pub accounting_settled: bool,
    pub last_error: Option<BatchJobError>,
    pub cleanup: std::collections::BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BatchObject {
    pub id: String,
    pub object: String,
    pub endpoint: String,
    pub input_file_id: String,
    pub completion_window: String,
    pub status: BatchStatus,
    pub output_file_id: Option<String>,
    pub error_file_id: Option<String>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub in_progress_at: Option<i64>,
    pub finalizing_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub failed_at: Option<i64>,
    pub expired_at: Option<i64>,
    pub cancelling_at: Option<i64>,
    pub cancelled_at: Option<i64>,
    pub request_counts: BatchRequestCounts,
    pub errors: Option<BatchErrorList>,
    pub metadata: HashMap<String, String>,
    pub localrouter: BatchLocalState,
}
#[derive(Serialize, ToSchema)]
pub struct BatchList {
    pub object: String,
    pub data: Vec<BatchObject>,
    pub first_id: Option<String>,
    pub last_id: Option<String>,
    pub has_more: bool,
    pub localrouter_usage: BatchUsageSummary,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CleanupBatchRequest {
    pub local_payloads: bool,
    pub remote: bool,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReconcileBatchRequest {
    pub after: Option<String>,
}
