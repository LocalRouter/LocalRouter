mod body;
mod helpers;
pub mod types;
use crate::{
    middleware::{
        client_auth::ClientAuthContext,
        error::{ApiErrorResponse, ApiResult},
    },
    state::{AppState, AuthContext},
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    Extension,
};
use futures::StreamExt;
use helpers::*;
use lr_batches::{VideoJob, VideoPhase};
use lr_providers::videos::{VideoProtocol, VideoRequest};
use sha2::{Digest, Sha256};
use types::*;

fn io_permit(state: &AppState) -> ApiResult<tokio::sync::OwnedSemaphorePermit> {
    state.video_io.clone().try_acquire_owned().map_err(|_| {
        ApiErrorResponse::rate_limited_with_retry("Too many active video operations", 1)
    })
}
async fn load(state: &AppState, owner: &str, id: &str) -> ApiResult<VideoJob> {
    let owner = owner.to_owned();
    let id = id.to_owned();
    store(state, move |s| s.video_job(&owner, &id)).await
}

#[utoipa::path(post,path="/v1/videos",tag="video",request_body(content((VideoRequest="application/json"),(VideoRequest="multipart/form-data")),description="JSON or text-only SDK multipart; Idempotency-Key is required"),params(("Idempotency-Key"=String,Header,description="Owner-scoped key for one video creation intent")),responses((status=200,description="Owned native video job or the recorded idempotent outcome",body=VideoObject),(status=400,description="Invalid or unsupported video request",body=crate::types::ErrorResponse),(status=409,description="Idempotency or lifecycle conflict",body=crate::types::ErrorResponse),(status=502,description="Native create outcome may be unknown; do not resubmit with another key automatically",body=crate::types::ErrorResponse)),security(("bearer_auth"=[])))]
pub async fn create(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    headers: HeaderMap,
    body::VideoBody(mut request): body::VideoBody,
) -> ApiResult<Response> {
    let strategy = state
        .router
        .authorize_native_operation(&auth.api_key_id, lr_config::NativeOperation::Video)
        .map_err(ApiErrorResponse::from)?;
    request.normalize().map_err(ApiErrorResponse::from)?;
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 128 && v.bytes().all(|b| b.is_ascii_graphic()))
        .ok_or_else(|| {
            ApiErrorResponse::bad_request(
                "Video creation requires an Idempotency-Key header of 1–128 visible ASCII bytes",
            )
        })?;
    let key_hash = format!("{:x}", Sha256::digest(key.as_bytes()));
    let request_hash = request.fingerprint().map_err(ApiErrorResponse::from)?;
    let (provider, model) = request
        .model
        .split_once('/')
        .filter(|(p, m)| !p.is_empty() && *p != "localrouter" && !m.is_empty())
        .map(|(p, m)| (p.to_owned(), m.to_owned()))
        .ok_or_else(|| {
            ApiErrorResponse::bad_request("Video requires an explicit qualified provider/model")
        })?;
    let instance = state
        .provider_registry
        .get_provider(&provider)
        .ok_or_else(|| ApiErrorResponse::bad_request("Video provider is unavailable"))?;
    let transport = instance
        .video_transport()
        .map_err(ApiErrorResponse::from)?
        .ok_or_else(|| {
            ApiErrorResponse::bad_request("Provider has no verified native video mapping")
        })?;
    let mut native = request.clone();
    native.model = model.clone();
    transport
        .validate(&native)
        .map_err(ApiErrorResponse::from)?;
    if lr_types::is_duplicate_hop()
        || state
            .config_manager
            .get()
            .clients
            .iter()
            .any(|c| c.id == auth.api_key_id && c.is_mcp_via_llm())
    {
        return Err(ApiErrorResponse::bad_request(
            "Video creation cannot use automatic MCP or duplicate gateway hops",
        ));
    }
    let session = uuid::Uuid::new_v4().to_string();
    let started = std::time::Instant::now();
    let mut monitor = crate::routes::monitor_helpers::emit_llm_call(
        &state,
        client_auth.as_ref(),
        Some(&session),
        "/v1/videos",
        &request.model,
        false,
        &serde_json::to_value(&request).unwrap_or_default(),
    );
    let projection:crate::types::ChatCompletionRequest=serde_json::from_value(serde_json::json!({"model":request.model,"messages":[{"role":"user","content":request.prompt}]})).map_err(|_|ApiErrorResponse::bad_request("Invalid video policy projection"))?;
    let expected = serde_json::to_value(&projection).unwrap_or_default();
    let context = crate::routes::pipeline::run_turn_pipeline(
        &state,
        &auth,
        client_auth.as_ref(),
        projection,
        &mut monitor,
        "/v1/videos",
        session,
        crate::routes::pipeline::PipelineCaps {
            allow_compression: false,
            allow_decision_routing: false,
            parallel_guardrails: false,
        },
    )
    .await?;
    if serde_json::to_value(context.chat_req).ok().as_ref() != Some(&expected) {
        return Err(monitor.capture_err(ApiErrorResponse::bad_request(
            "Video prompt/model rewrites require an explicit new video request",
        )));
    }
    // Approval/scanning may wait while provider configuration changes.
    let transport = state
        .provider_registry
        .get_provider(&provider)
        .ok_or_else(|| ApiErrorResponse::bad_request("Video provider changed before dispatch"))?
        .video_transport()
        .map_err(ApiErrorResponse::from)?
        .ok_or_else(|| {
            ApiErrorResponse::bad_request("Video provider no longer supports native jobs")
        })?;
    transport
        .validate(&native)
        .map_err(ApiErrorResponse::from)?;
    let permit = io_permit(&state)?;
    let salt = store(&state, |s| s.binding_salt()).await?;
    let binding = transport.binding(&salt);
    let now = chrono::Utc::now().timestamp();
    let intent = VideoJob {
        id: String::new(),
        owner: auth.api_key_id.clone(),
        provider: provider.clone(),
        model: request.model.clone(),
        protocol: if transport.protocol == VideoProtocol::Openai {
            "openai"
        } else {
            "llmgateway"
        }
        .into(),
        binding,
        request_hash,
        idempotency_hash: key_hash,
        native_id: None,
        phase: VideoPhase::SubmitPendingOutcome,
        status: "unknown".into(),
        progress: 0,
        seconds: request.seconds,
        size: request.size.clone().unwrap_or_default(),
        created_at: now,
        completed_at: None,
        expires_at: None,
        cost_usd: None,
        error_code: None,
        archived: false,
        event_id: Some(monitor.event_id().to_owned()),
        poll_sequence: 0,
        applied_sequence: 0,
        last_poll_ms: 0,
    };
    let (job, dispatch) = store(&state, move |s| s.create_video_intent(intent)).await?;
    if !dispatch {
        monitor.complete(
            &state,
            &provider,
            &model,
            200,
            0,
            0,
            None,
            Some(0.0),
            started.elapsed().as_millis() as u64,
            Some("idempotent_replay"),
            "Recorded video intent; no upstream create repeated",
            false,
        );
        return Ok(json_response(public_job(&job, true)));
    }
    let mut dispatch = match state
        .router
        .begin_native_dispatch(
            &auth.api_key_id,
            lr_config::NativeOperation::Video,
            &provider,
            &model,
            &strategy.id,
        )
        .await
    {
        Ok(dispatch) => dispatch,
        Err(error) => {
            let owner = job.owner.clone();
            let id = job.id.clone();
            let rejected =
                store(&state, move |s| s.reject_video_before_dispatch(&owner, &id)).await?;
            return Err(monitor.capture_err(with_job(ApiErrorResponse::from(error), &rejected)));
        }
    };
    state.record_client_activity(&auth.api_key_id);
    state.emit_event("llm-request", "video");
    let current_transport = match bound_transport(&state, &auth.api_key_id, &job).await {
        Ok(transport) => transport,
        Err(error) => {
            let owner = job.owner.clone();
            let id = job.id.clone();
            let rejected =
                store(&state, move |s| s.reject_video_before_dispatch(&owner, &id)).await?;
            return Err(monitor.capture_err(with_job(error, &rejected)));
        }
    };
    let result = current_transport.create(&native, &job.id).await;
    dispatch.success = result.is_ok();
    drop(dispatch);
    drop(permit);
    let native = match result {
        Ok(result) => result,
        Err(error) => {
            return Err(monitor.capture_err(with_job(ApiErrorResponse::from(error), &job)))
        }
    };
    let owner = job.owner.clone();
    let id = job.id.clone();
    let result = store(&state, move |s| {
        s.submit_video_result(&owner, &id, update(native))
    })
    .await
    .map_err(|e| monitor.capture_err(with_job(e, &job)))?;
    let public = public_job(&result, false);
    crate::routes::monitor_helpers::update_llm_call_response_body(
        &state,
        monitor.event_id(),
        &serde_json::to_value(&public).unwrap_or_default(),
    );
    crate::routes::monitor_helpers::update_llm_call_upstream(
        &state,
        monitor.event_id(),
        Some(lr_types::LlmApi::Video),
        None,
    );
    monitor.complete(
        &state,
        &provider,
        &model,
        200,
        0,
        0,
        None,
        result.cost_usd,
        started.elapsed().as_millis() as u64,
        Some(&result.status),
        "Native video job accepted",
        false,
    );
    Ok(json_response(public))
}
#[utoipa::path(get,path="/v1/videos",tag="video",params(("limit"=Option<usize>,Query),("after"=Option<String>,Query)),responses((status=200,description="Local owner-scoped job list; no provider account enumeration",body=VideoList)),security(("bearer_auth"=[])))]
pub async fn list(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(query): Query<ListQuery>,
) -> ApiResult<Response> {
    if query.order.as_deref().is_some_and(|v| v != "desc") {
        return Err(ApiErrorResponse::bad_request(
            "Video list currently supports descending order only",
        ));
    }
    let owner = auth.api_key_id;
    let limit = query.limit.unwrap_or(20);
    let (jobs, has_more) = store(&state, move |s| {
        s.list_video_jobs(&owner, limit, query.after.as_deref())
    })
    .await?;
    let subtotal: f64 = jobs.iter().filter_map(|j| j.cost_usd).sum();
    let unknown = jobs.iter().filter(|j| j.cost_usd.is_none()).count();
    Ok(json_response(VideoList {
        object: "list".into(),
        first_id: jobs.first().map(|j| j.id.clone()),
        last_id: jobs.last().map(|j| j.id.clone()),
        data: jobs.iter().map(|j| public_job(j, true)).collect(),
        has_more,
        extensions: serde_json::json!({"localrouter.video":{"known_subtotal_usd":subtotal,"unknown_job_count":unknown,"scope":"listed_jobs","source":"native_job_ledger"}}),
    }))
}
#[utoipa::path(get,path="/v1/videos/{video_id}",tag="video",params(("video_id"=String,Path)),responses((status=200,description="Owned cached or refreshed native job",body=VideoObject),(status=404,description="Unknown or foreign local job",body=crate::types::ErrorResponse)),security(("bearer_auth"=[])))]
pub async fn retrieve(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let job = load(&state, &auth.api_key_id, &id).await?;
    owned(&job, &auth.api_key_id)?;
    if job.phase != VideoPhase::Known || job.native_id.is_none() {
        return Ok(json_response(public_job(&job, true)));
    }
    let transport = bound_transport(&state, &auth.api_key_id, &job).await?;
    let _permit = io_permit(&state)?;
    let owner = job.owner.clone();
    let local_id = job.id.clone();
    let (job, sequence) = store(&state, move |s| {
        s.claim_video_poll(&owner, &local_id, chrono::Utc::now().timestamp_millis())
    })
    .await?;
    let Some(sequence) = sequence else {
        return Ok(json_response(public_job(&job, true)));
    };
    let native = transport
        .retrieve(job.native_id.as_deref().unwrap())
        .await
        .map_err(|e| with_job(ApiErrorResponse::from(e), &job))?;
    let owner = job.owner.clone();
    let local_id = job.id.clone();
    let job = store(&state, move |s| {
        s.update_video_status(&owner, &local_id, sequence, update(native))
    })
    .await?;
    completed_monitor(&state, &job);
    Ok(json_response(public_job(&job, false)))
}
#[utoipa::path(get,path="/v1/videos/{video_id}/content",tag="video",params(("video_id"=String,Path),("variant"=Option<String>,Query)),responses((status=200,description="Bounded native MP4 bytes",content_type="video/mp4"),(status=206,description="Native partial MP4 bytes",content_type="video/mp4"),(status=404,description="Unknown or foreign video",body=crate::types::ErrorResponse)),security(("bearer_auth"=[])))]
pub async fn content(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
    Query(query): Query<ContentQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    if query.variant.as_deref().is_some_and(|v| v != "video") {
        return Err(ApiErrorResponse::bad_request(
            "Only the native video content variant is supported",
        ));
    }
    let job = load(&state, &auth.api_key_id, &id).await?;
    if job.phase != VideoPhase::Known || job.status != "completed" {
        return Err(ApiErrorResponse::new(
            StatusCode::CONFLICT,
            "video_not_ready",
            "Video is not completed or its lifecycle outcome is unknown",
        ));
    }
    let transport = bound_transport(&state, &auth.api_key_id, &job).await?;
    let permit = io_permit(&state)?;
    let range = headers
        .get("range")
        .map(|v| v.to_str())
        .transpose()
        .map_err(|_| ApiErrorResponse::bad_request("Invalid video range"))?;
    let response = transport
        .content(
            job.native_id.as_deref().ok_or_else(|| {
                ApiErrorResponse::internal_error("Video native binding is missing")
            })?,
            range,
        )
        .await
        .map_err(|e| with_job(ApiErrorResponse::from(e), &job))?;
    let status = response.status();
    let mut result_headers = HeaderMap::new();
    for key in [
        "content-type",
        "content-length",
        "content-range",
        "accept-ranges",
        "etag",
    ] {
        if let Some(value) = response.headers().get(key) {
            result_headers.insert(axum::http::HeaderName::from_static(key), value.clone());
        }
    }
    result_headers.insert(
        "cache-control",
        axum::http::HeaderValue::from_static("no-store"),
    );
    result_headers.insert(
        "content-disposition",
        axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{}.mp4\"", job.id))
            .unwrap(),
    );
    let bytes: std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<axum::body::Bytes, std::io::Error>> + Send>,
    > = Box::pin(
        async_stream::try_stream! {let _permit=permit;let mut stream=response.bytes_stream();let mut total=0u64;while let Some(chunk)=stream.next().await {let chunk=chunk.map_err(|_|std::io::Error::other("Video content stream failed"))?;total=total.saturating_add(chunk.len() as u64);if total>lr_providers::videos::MAX_MEDIA_BYTES {Err(std::io::Error::other("Video content exceeds 128 MiB"))?;}yield chunk;}},
    );
    let mut response = Response::new(axum::body::Body::from_stream(bytes));
    *response.status_mut() = status;
    *response.headers_mut() = result_headers;
    Ok(response)
}
#[utoipa::path(delete,path="/v1/videos/{video_id}",tag="video",params(("video_id"=String,Path)),responses((status=200,description="Verified deletion of completed/failed native assets",body=VideoDeleted),(status=501,description="Provider has no verified delete mapping",body=crate::types::ErrorResponse)),security(("bearer_auth"=[])))]
pub async fn delete(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let job = load(&state, &auth.api_key_id, &id).await?;
    if job.phase == VideoPhase::Deleted {
        return Ok(json_response(VideoDeleted {
            id,
            object: "video.deleted".into(),
            deleted: true,
        }));
    }
    let transport = bound_transport(&state, &auth.api_key_id, &job).await?;
    if transport.protocol != VideoProtocol::Openai {
        return Err(ApiErrorResponse::new(
            StatusCode::NOT_IMPLEMENTED,
            "unsupported_operation",
            "Provider does not expose verified video asset deletion",
        ));
    }
    let _permit = io_permit(&state)?;
    let owner = job.owner.clone();
    let local_id = id.clone();
    let pending = store(&state, move |s| s.begin_video_delete(&owner, &local_id)).await?;
    transport
        .delete(
            pending.native_id.as_deref().ok_or_else(|| {
                ApiErrorResponse::internal_error("Video native binding is missing")
            })?,
        )
        .await
        .map_err(|e| with_job(ApiErrorResponse::from(e), &pending))?;
    let owner = job.owner;
    let local_id = id.clone();
    store(&state, move |s| s.finish_video_delete(&owner, &local_id)).await?;
    Ok(json_response(VideoDeleted {
        id,
        object: "video.deleted".into(),
        deleted: true,
    }))
}
#[utoipa::path(post,path="/v1/videos/{video_id}/cancel",tag="video",params(("video_id"=String,Path)),responses((status=501,description="Native cancellation is unsupported; local waiting is not remote cancellation",body=crate::types::ErrorResponse)),security(("bearer_auth"=[])))]
pub async fn cancel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    load(&state, &auth.api_key_id, &id).await?;
    Err(ApiErrorResponse::new(StatusCode::NOT_IMPLEMENTED,"unsupported_operation","Selected native video providers expose no verified cancellation API; stopping local waiting does not stop rendering or billing"))
}
#[utoipa::path(post,path="/v1/videos/{video_id}/archive",tag="video",params(("video_id"=String,Path)),responses((status=200,description="Archive local tracking only; does not delete or cancel native work",body=VideoObject)),security(("bearer_auth"=[])))]
pub async fn archive(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let owner = auth.api_key_id;
    let job = store(&state, move |s| s.archive_video(&owner, &id)).await?;
    Ok(json_response(public_job(&job, true)))
}
