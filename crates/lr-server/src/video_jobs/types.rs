use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
#[derive(Serialize, ToSchema)]
pub struct VideoObject {
    pub id: String,
    pub object: String,
    pub model: String,
    pub status: String,
    pub progress: u8,
    pub seconds: String,
    pub size: String,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub error: Option<lr_providers::videos::VideoError>,
    #[schema(value_type=Object)]
    pub extensions: serde_json::Value,
}
#[derive(Serialize, ToSchema)]
pub struct VideoList {
    pub object: String,
    pub data: Vec<VideoObject>,
    pub first_id: Option<String>,
    pub last_id: Option<String>,
    pub has_more: bool,
    #[schema(value_type=Object)]
    pub extensions: serde_json::Value,
}
#[derive(Serialize, ToSchema)]
pub struct VideoDeleted {
    pub id: String,
    pub object: String,
    pub deleted: bool,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub limit: Option<usize>,
    pub after: Option<String>,
    pub order: Option<String>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ContentQuery {
    pub variant: Option<String>,
}
pub fn public(job: &lr_batches::VideoJob, cached: bool) -> VideoObject {
    use lr_batches::VideoPhase;
    VideoObject {
        id: job.id.clone(),
        object: "video".into(),
        model: job.model.clone(),
        status: job.status.clone(),
        progress: job.progress,
        seconds: job.seconds.to_string(),
        size: job.size.clone(),
        created_at: job.created_at,
        completed_at: job.completed_at,
        expires_at: job.expires_at,
        error: job
            .error_code
            .as_ref()
            .map(|code| lr_providers::videos::VideoError {
                code: code.clone(),
                message: "Video request was not dispatched".into(),
            }),
        extensions: serde_json::json!({"localrouter.video":{"phase":job.phase,"archived":job.archived,"poll_cached":cached,"cost_usd":job.cost_usd,"cost_source":if job.phase==VideoPhase::RejectedBeforeDispatch {"not_dispatched"}else if job.cost_usd.is_some(){"reported"}else{"unknown"},"cost_scope":"job_total","requested_seconds":job.seconds,"cancel_supported":false,"remote_delete_supported":job.protocol=="openai","content_supported":job.phase==VideoPhase::Known&&job.status=="completed","accounting":"video_job_ledger"}}),
    }
}
