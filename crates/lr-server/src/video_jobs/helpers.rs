use super::types::VideoObject;
use crate::{
    middleware::error::{ApiErrorResponse, ApiResult},
    state::AppState,
};
use lr_batches::{VideoJob, VideoUpdate};
use lr_providers::videos::{NativeVideo, VideoTransport};
use serde_json::json;
pub async fn store<T: Send + 'static>(
    state: &AppState,
    work: impl FnOnce(&lr_batches::BatchStore) -> lr_batches::Result<T> + Send + 'static,
) -> ApiResult<T> {
    let store = state.batch_store.clone();
    tokio::task::spawn_blocking(move || work(&store))
        .await
        .map_err(|_| ApiErrorResponse::internal_error("Video journal task failed"))?
        .map_err(|error| match error {
            lr_batches::StoreError::Invalid(message) => ApiErrorResponse::bad_request(message),
            lr_batches::StoreError::NotFound => ApiErrorResponse::not_found("Video not found"),
            lr_batches::StoreError::Conflict => ApiErrorResponse::new(
                axum::http::StatusCode::CONFLICT,
                "video_state_conflict",
                "Video state, idempotency key or native binding conflicts",
            ),
            lr_batches::StoreError::Quota => ApiErrorResponse::new(
                axum::http::StatusCode::INSUFFICIENT_STORAGE,
                "video_storage_limit",
                "Local video tracking quota exceeded",
            ),
            lr_batches::StoreError::Storage(_) => {
                ApiErrorResponse::internal_error("Video journal is unavailable")
            }
        })
}
pub fn update(native: NativeVideo) -> VideoUpdate {
    VideoUpdate {
        native_id: native.id,
        status: native.status,
        progress: native.progress,
        completed_at: native.completed_at,
        expires_at: native.expires_at,
        cost_usd: native.cost_usd,
    }
}
pub fn json_response<T: serde::Serialize>(body: T) -> axum::response::Response {
    use axum::response::IntoResponse;
    let mut response = axum::Json(body).into_response();
    response.headers_mut().insert(
        "cache-control",
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
pub fn with_job(mut error: ApiErrorResponse, job: &VideoJob) -> ApiErrorResponse {
    error.error.extensions.get_or_insert_with(Default::default).insert("localrouter.video".into(),json!({"id":job.id,"phase":job.phase,"status":job.status,"dispatch_outcome":if job.phase==lr_batches::VideoPhase::RejectedBeforeDispatch {"not_dispatched"}else if job.native_id.is_some(){"known"}else{"unknown"}}));
    error.without_retry_if(true)
}
pub async fn bound_transport(
    state: &AppState,
    client: &str,
    job: &VideoJob,
) -> ApiResult<VideoTransport> {
    state
        .router
        .authorize_native_resource(client, lr_config::NativeOperation::Video, &job.model)
        .map_err(ApiErrorResponse::from)?;
    let provider = state
        .provider_registry
        .get_provider(&job.provider)
        .ok_or_else(|| ApiErrorResponse::bad_request("Video provider is unavailable"))?;
    let transport = provider
        .video_transport()
        .map_err(ApiErrorResponse::from)?
        .ok_or_else(|| ApiErrorResponse::bad_request("Provider no longer supports native video"))?;
    let salt = store(state, |store| store.binding_salt()).await?;
    if transport.binding(&salt) != job.binding {
        return Err(ApiErrorResponse::new(
            axum::http::StatusCode::CONFLICT,
            "credential_binding_changed",
            "Video job is bound to a different provider origin or credential",
        ));
    }
    Ok(transport)
}
pub fn owned(job: &VideoJob, owner: &str) -> ApiResult<()> {
    if job.owner != owner {
        return Err(ApiErrorResponse::not_found("Video not found"));
    }
    Ok(())
}
pub fn completed_monitor(state: &AppState, job: &VideoJob) {
    if let Some(id) = &job.event_id {
        crate::routes::monitor_helpers::complete_llm_call(
            state,
            id,
            &job.provider,
            &job.model,
            200,
            0,
            0,
            None,
            job.cost_usd,
            job.completed_at
                .unwrap_or_else(|| chrono::Utc::now().timestamp())
                .saturating_sub(job.created_at)
                .max(0) as u64
                * 1000,
            Some(&job.status),
            &format!("Video job {}", job.status),
            false,
        );
    }
}
pub fn public_job(job: &VideoJob, cached: bool) -> VideoObject {
    super::types::public(job, cached)
}
