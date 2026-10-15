//! Owner-scoped SDK lifecycle. GET is always local; network work requires POST.
use super::{context, rows, types::*, worker};
use crate::{
    middleware::error::{ApiErrorResponse, ApiResult},
    routes::files::{blocking, owner},
    state::{AppState, AuthContext},
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use lr_batches::{BatchJob, BatchJobOptions, BatchPhase, NewBatchJob};
use lr_providers::{batches::NativeBatchKind, gateway::GatewayPreferences};
use serde::Deserialize;
use serde_json::json;

pub(crate) fn conflict(message: &str) -> ApiErrorResponse {
    ApiErrorResponse::new(StatusCode::CONFLICT, "batch_state_conflict", message)
}
pub(crate) async fn load(state: &AppState, auth: &AuthContext, id: &str) -> ApiResult<BatchJob> {
    let client = owner(state, auth)?;
    let store = state.batch_store.clone();
    let id = id.to_owned();
    blocking(move || store.get_job(&client, &id)).await
}
pub(crate) async fn object(state: &AppState, job: BatchJob) -> ApiResult<BatchObject> {
    // Observe runtime first. If it is already idle, the following durable
    // read includes the worker's final diagnostic/progress before it dropped
    // its lease. Reading these in reverse order can expose stale idle state.
    let worker_running = state.batch_runtime.worker_running(&job.id);
    let store = state.batch_store.clone();
    let snapshot = blocking(move || store.job_snapshot(&job.owner, &job.id)).await?;
    let lr_batches::BatchJobSnapshot {
        job,
        results,
        accounting,
        reserved_usage,
    } = snapshot;
    let kind = state
        .provider_registry
        .get_provider(&job.provider_instance)
        .and_then(|p| p.native_batch_kind());
    let count = job.request_counts.as_ref();
    let output = results
        .iter()
        .find(|r| r.kind == lr_batches::BatchResultKind::Output)
        .and_then(|r| r.file_id.clone());
    let error = results
        .iter()
        .find(|r| r.kind == lr_batches::BatchResultKind::Error)
        .and_then(|r| r.file_id.clone());
    // Native timestamps aren't synthesized from local polling times.
    Ok(BatchObject {
        id: job.id.clone(),
        object: "batch".into(),
        endpoint: job.endpoint.clone(),
        input_file_id: job.input_file_id.clone(),
        completion_window: "24h".into(),
        status: job.status,
        output_file_id: output,
        error_file_id: error,
        created_at: job.created_at,
        expires_at: None,
        in_progress_at: None,
        finalizing_at: None,
        completed_at: None,
        failed_at: None,
        expired_at: None,
        cancelling_at: None,
        cancelled_at: None,
        request_counts: BatchRequestCounts {
            total: job.rows,
            completed: count.map_or(0, |c| c.completed),
            failed: count.map_or(0, |c| c.failed),
        },
        errors: job.last_error.clone().map(|e| BatchErrorList {
            object: "list".into(),
            data: vec![e],
        }),
        metadata: job.options.metadata.clone(),
        localrouter: BatchLocalState {
            phase: job.phase,
            provider_instance: job.provider_instance,
            model: job.model,
            validated_rows: job.validated_rows,
            worker_running,
            can_resume_validation: job.phase == BatchPhase::Draft && !worker_running,
            can_submit_prepared: matches!(job.phase, BatchPhase::Prepared | BatchPhase::Uploaded)
                && !worker_running,
            reserved_usage,
            native_price: job.options.native_price.clone(),
            cancellation_supported: kind == Some(NativeBatchKind::Openai),
            remote_batch_deletion_supported: kind == Some(NativeBatchKind::Openrouter),
            request_counts_known: count.is_some(),
            prepared_snapshot_file_id: job.prepared_snapshot_file_id,
            snapshot_fingerprint: job.snapshot_fingerprint,
            result_limit_bytes: job.options.result_limit_bytes,
            local_retention_seconds: job.options.output_retention_seconds,
            results,
            accounting,
            accounting_settled: job.accounting_settled,
            last_error: job.last_error,
            cleanup: job.cleanup,
        },
    })
}

#[utoipa::path(post,path="/v1/batches",tag="batches",request_body=CreateBatchRequest,params(("Idempotency-Key"=Option<String>,Header)),responses((status=200,description="Durable owned batch; validation and native submission continue asynchronously",body=BatchObject)),security(("bearer_auth"=[])))]
pub async fn create(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    Json(request): Json<CreateBatchRequest>,
) -> ApiResult<Json<BatchObject>> {
    let client = owner(&state, &auth)?;
    if request.completion_window != "24h" {
        return Err(ApiErrorResponse::bad_request(
            "Native batches require completion_window=24h",
        ));
    }
    let fingerprint = lr_router::explanation::fingerprint(
        serde_json::to_value(&request)
            .map_err(|_| ApiErrorResponse::bad_request("Invalid batch request"))?,
    );
    let idempotency = headers
        .get("idempotency-key")
        .map(|v| {
            v.to_str()
                .ok()
                .filter(|v| !v.is_empty() && v.len() <= 256 && !v.chars().any(char::is_control))
                .map(|v| lr_router::explanation::fingerprint(json!(v)))
                .ok_or_else(|| ApiErrorResponse::bad_request("Invalid Idempotency-Key"))
        })
        .transpose()?;
    if let Some(key) = idempotency.clone() {
        let store = state.batch_store.clone();
        let owner = client.clone();
        let fp = fingerprint.clone();
        if let Some(job) = blocking(move || store.find_idempotent_job(&owner, &key, &fp)).await? {
            return Ok(Json(object(&state, job).await?));
        }
    }
    let _slot = state.batch_runtime.try_preflight().ok_or_else(|| {
        ApiErrorResponse::rate_limited("Two native validations are already running")
    })?;
    let store = state.batch_store.clone();
    let owner = client.clone();
    let input = request.input_file_id.clone();
    let bytes = blocking(move || store.content(&owner, &input)).await?;
    let (rows, qualified) = rows::parse_input(&bytes, &request.endpoint)?;
    let context = context::context(&state, &auth, &qualified)?;
    context::verify_model(&context, &request.endpoint).await?;
    let preferences = request
        .extensions
        .as_ref()
        .and_then(|e| e.gateway.clone())
        .unwrap_or_default();
    let pin = if context.transport.kind() == NativeBatchKind::Openrouter {
        if request.output_expires_after.is_some()
            || request.metadata.as_ref().is_some_and(|m| !m.is_empty())
        {
            return Err(ApiErrorResponse::bad_request("OpenRouter inline batches do not expose verified metadata or output-expiry controls"));
        }
        context
            .transport
            .effective_inline_pin(&preferences)
            .map_err(ApiErrorResponse::from)?
    } else {
        if preferences != GatewayPreferences::default() {
            return Err(ApiErrorResponse::bad_request(
                "Gateway provider controls require OpenRouter",
            ));
        }
        None
    };
    if request
        .output_expires_after
        .as_ref()
        .is_some_and(|e| e.anchor != "created_at" || !(3600..=2_592_000).contains(&e.seconds))
    {
        return Err(ApiErrorResponse::bad_request(
            "Invalid native output expiry",
        ));
    }
    for row in &rows {
        context::validate_kind(context.transport.kind(), &request.endpoint, &row.body)?;
    }
    let seal = context::seal(&state, &auth, &context)?;
    let store = state.batch_store.clone();
    let salt = blocking(move || store.binding_salt()).await?;
    let binding = context
        .transport
        .binding_fingerprint(&salt)
        .map_err(ApiErrorResponse::from)?;
    let local = request.extensions.as_ref().and_then(|e| e.batch.as_ref());
    let options = BatchJobOptions {
        prepare_only: local.and_then(|l| l.prepare_only).unwrap_or(false),
        permission_model: Some(context.permission_model.clone()),
        native_price: super::worker::native_price(&context),
        native_output_expiry_seconds: request.output_expires_after.map(|e| e.seconds),
        metadata: request.metadata.unwrap_or_default(),
        output_retention_seconds: local
            .and_then(|l| l.local_retention_seconds)
            .unwrap_or(lr_batches::DEFAULT_RETENTION as u32),
        result_limit_bytes: local
            .and_then(|l| l.result_limit_bytes)
            .unwrap_or(lr_batches::MAX_RESULT_BYTES as u64),
        provider_only: pin,
    };
    let new = NewBatchJob {
        options,
        owner: client,
        input_file_id: request.input_file_id,
        provider_instance: context.provider_instance,
        credential_binding: binding,
        endpoint: request.endpoint,
        model: context.model,
        request_fingerprint: fingerprint,
        policy_fingerprint: seal.fingerprint,
        idempotency_key_hash: idempotency,
        custom_ids: rows.iter().map(|r| r.custom_id.clone()).collect(),
    };
    let store = state.batch_store.clone();
    let (job, created) = blocking(move || store.create_job(new)).await?;
    if created {
        worker::start_admitted(&state, &auth, &job, false, _slot)?;
    }
    Ok(Json(object(&state, job).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    after: Option<String>,
    limit: Option<usize>,
}
#[utoipa::path(get,path="/v1/batches",tag="batches",params(("after"=Option<String>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=BatchList)),security(("bearer_auth"=[])))]
pub async fn list(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(query): Query<ListQuery>,
) -> ApiResult<Json<BatchList>> {
    let owner = owner(&state, &auth)?;
    let store = state.batch_store.clone();
    let (jobs, usage) = blocking(move || {
        Ok((
            store.list_jobs(&owner)?,
            store.batch_usage_summary(Some(&owner), None)?,
        ))
    })
    .await?;
    let start = match query.after {
        Some(id) => jobs
            .iter()
            .position(|j| j.id == id)
            .map(|i| i + 1)
            .ok_or_else(|| ApiErrorResponse::bad_request("Unknown batch cursor"))?,
        None => 0,
    };
    let limit = query.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err(ApiErrorResponse::bad_request("limit must be 1..100"));
    }
    let has_more = jobs.len() > start + limit;
    let mut data = Vec::new();
    for job in jobs.into_iter().skip(start).take(limit) {
        data.push(object(&state, job).await?);
    }
    Ok(Json(BatchList {
        object: "list".into(),
        first_id: data.first().map(|j| j.id.clone()),
        last_id: data.last().map(|j| j.id.clone()),
        data,
        has_more,
        localrouter_usage: usage,
    }))
}
#[utoipa::path(get,path="/v1/batches/{batch_id}",tag="batches",params(("batch_id"=String,Path)),responses((status=200,body=BatchObject)),security(("bearer_auth"=[])))]
pub async fn retrieve(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> ApiResult<Json<BatchObject>> {
    Ok(Json(object(&state, load(&state, &auth, &id).await?).await?))
}
#[utoipa::path(post,path="/v1/batches/{batch_id}/resume",tag="batches",params(("batch_id"=String,Path)),responses((status=200,body=BatchObject)),security(("bearer_auth"=[])))]
pub async fn resume(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> ApiResult<Json<BatchObject>> {
    let job = load(&state, &auth, &id).await?;
    if job.phase != BatchPhase::Draft {
        return Err(conflict("Only an undispatched draft can resume validation"));
    }
    worker::start(&state, &auth, &job, false)?;
    Ok(Json(object(&state, job).await?))
}
#[utoipa::path(post,path="/v1/batches/{batch_id}/submit",tag="batches",params(("batch_id"=String,Path)),responses((status=200,body=BatchObject)),security(("bearer_auth"=[])))]
pub async fn submit(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> ApiResult<Json<BatchObject>> {
    let job = load(&state, &auth, &id).await?;
    if !matches!(job.phase, BatchPhase::Prepared | BatchPhase::Uploaded) {
        return Err(conflict(
            "Only sealed, undispatched inference can be submitted",
        ));
    }
    worker::start(&state, &auth, &job, true)?;
    Ok(Json(object(&state, job).await?))
}
#[utoipa::path(post,path="/v1/batches/{batch_id}/refresh",tag="batches",params(("batch_id"=String,Path)),responses((status=200,body=BatchObject)),security(("bearer_auth"=[])))]
pub async fn refresh(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> ApiResult<Json<BatchObject>> {
    let job = load(&state, &auth, &id).await?;
    let _control = state
        .batch_runtime
        .try_control(&id)
        .ok_or_else(|| conflict("A batch control operation is already running"))?;
    if state.batch_runtime.worker_running(&id) {
        return Err(conflict("Validation/submission is still running"));
    }
    let context = worker::bound_context(&state, &auth, &job, false).await?;
    let remote = job
        .remote_batch_id
        .as_deref()
        .ok_or_else(|| conflict("No known native batch ID; uncertain creation is never retried"))?;
    let native = context
        .transport
        .retrieve(remote)
        .await
        .map_err(ApiErrorResponse::from)?;
    worker::observe(&state, &job, &native).await?;
    let job = load(&state, &auth, &id).await?;
    if job.status.is_terminal()
        && job
            .cleanup
            .get("local_payloads")
            .is_none_or(|v| v != "deleted")
    {
        super::results::collect(&state, &job, &context.transport, &native).await?;
    }
    Ok(Json(object(&state, load(&state, &auth, &id).await?).await?))
}
#[utoipa::path(post,path="/v1/batches/{batch_id}/cancel",tag="batches",params(("batch_id"=String,Path)),responses((status=200,body=BatchObject)),security(("bearer_auth"=[])))]
pub async fn cancel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> ApiResult<Json<BatchObject>> {
    let job = load(&state, &auth, &id).await?;
    if state.batch_runtime.cancel_worker(&id) {
        return Err(conflict(
            "Validation is stopping; inspect the durable phase before cancelling again",
        ));
    }
    let _control = state
        .batch_runtime
        .try_control(&id)
        .ok_or_else(|| conflict("A batch control operation is already running"))?;
    if job.status.is_terminal() {
        return Ok(Json(object(&state, job).await?));
    }
    if matches!(
        job.phase,
        BatchPhase::Draft | BatchPhase::Prepared | BatchPhase::Uploaded
    ) {
        let store = state.batch_store.clone();
        let owner = job.owner.clone();
        let id = job.id.clone();
        let job = blocking(move || {
            let job = store.cancel_unsubmitted(&owner, &id)?;
            store.release_result_storage(&owner, &id)?;
            Ok(job)
        })
        .await?;
        return Ok(Json(object(&state, job).await?));
    }
    let context = worker::bound_context(&state, &auth, &job, false).await?;
    if !context.transport.supports_cancel() {
        return Err(ApiErrorResponse::bad_request(
            "This native API has no verified cancellation operation",
        ));
    }
    let remote = job.remote_batch_id.as_deref().ok_or_else(|| {
        conflict("Uncertain submission cannot be cancelled without a proven native ID")
    })?;
    let store = state.batch_store.clone();
    let saved = job.clone();
    let intent = blocking(move || {
        store.begin_mutation(
            &saved.owner,
            &saved.id,
            saved.revision,
            lr_batches::BatchMutation::Cancel,
        )
    })
    .await?;
    let native = context
        .transport
        .cancel(remote)
        .await
        .map_err(ApiErrorResponse::from)?;
    let store = state.batch_store.clone();
    let owner = job.owner.clone();
    let remote = native.id.clone();
    let status = worker::status(native.status);
    blocking(move || store.complete_mutation(&owner, &intent, &remote, status)).await?;
    worker::observe(&state, &job, &native).await?;
    Ok(Json(object(&state, load(&state, &auth, &id).await?).await?))
}

#[utoipa::path(post,path="/v1/batches/{batch_id}/cleanup",tag="batches",params(("batch_id"=String,Path)),request_body=CleanupBatchRequest,responses((status=200,body=BatchObject)),security(("bearer_auth"=[])))]
pub async fn cleanup(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
    Json(request): Json<CleanupBatchRequest>,
) -> ApiResult<Json<BatchObject>> {
    let job = load(&state, &auth, &id).await?;
    let _control = state
        .batch_runtime
        .try_control(&id)
        .ok_or_else(|| conflict("A worker or control operation is already running"))?;
    if !job.status.is_terminal() {
        return Err(conflict("Only terminal payloads can be cleaned up"));
    }
    if request.remote && !job.accounting_settled {
        return Err(conflict(
            "Collect and reconcile native accounting before deleting remote evidence",
        ));
    }
    if request.remote {
        let context = worker::bound_context(&state, &auth, &job, false).await?;
        if context.transport.kind() == NativeBatchKind::Openai {
            for (kind, file) in [
                ("input_file", &job.remote_input_file_id),
                ("output_file", &job.remote_output_file_id),
                ("error_file", &job.remote_error_file_id),
            ] {
                if let Some(file) = file {
                    cleanup_record(&state, &job, kind, "pending").await?;
                    let outcome = match context.transport.delete_file(file).await {
                        Ok(()) => "deleted",
                        Err(lr_types::AppError::ProviderStatus { status: 404, .. }) => {
                            "already_absent"
                        }
                        Err(error) => return Err(ApiErrorResponse::from(error)),
                    };
                    cleanup_record(&state, &job, kind, outcome).await?;
                } else {
                    cleanup_record(&state, &job, kind, "not_applicable").await?;
                }
            }
            cleanup_record(&state, &job, "batch", "unsupported").await?;
        } else if let Some(remote) = &job.remote_batch_id {
            cleanup_record(&state, &job, "batch", "pending").await?;
            let status = serde_json::from_value(
                serde_json::to_value(job.status)
                    .map_err(|_| conflict("Invalid terminal status"))?,
            )
            .map_err(|_| conflict("Invalid terminal status"))?;
            match context
                .transport
                .delete_terminal_batch(remote, status)
                .await
            {
                Ok(result) => {
                    cleanup_record(&state, &job, "batch", "deleted").await?;
                    if let Some(upstream) = result.deletion.upstream {
                        cleanup_record(&state, &job, "upstream", &upstream.status).await?;
                    }
                }
                Err(lr_types::AppError::ProviderStatus { status: 404, .. }) => {
                    cleanup_record(&state, &job, "batch", "already_absent").await?;
                }
                Err(error) => return Err(ApiErrorResponse::from(error)),
            }
        } else {
            cleanup_record(&state, &job, "batch", "not_applicable").await?;
        }
    }
    if request.local_payloads {
        cleanup_record(&state, &job, "local_payloads", "pending").await?;
        let store = state.batch_store.clone();
        let saved = job.clone();
        blocking(move || store.purge_batch_payloads(&saved.owner, &saved.id)).await?;
    }
    Ok(Json(object(&state, load(&state, &auth, &id).await?).await?))
}
async fn cleanup_record(
    state: &AppState,
    job: &BatchJob,
    artifact: &str,
    outcome: &str,
) -> ApiResult<()> {
    let store = state.batch_store.clone();
    let saved = job.clone();
    let artifact = artifact.to_owned();
    let outcome = outcome.to_owned();
    blocking(move || {
        store.cleanup_outcome(&saved.owner, &saved.id, &artifact, &outcome)?;
        Ok(())
    })
    .await
}

#[utoipa::path(post,path="/v1/batches/{batch_id}/reconcile",tag="batches",params(("batch_id"=String,Path)),request_body=ReconcileBatchRequest,responses((status=200,description="Read one private upstream page; prove original nonce/snapshot before adopting an ID",body=BatchObject)),security(("bearer_auth"=[])))]
pub async fn reconcile(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
    Json(request): Json<ReconcileBatchRequest>,
) -> ApiResult<Json<BatchObject>> {
    let job = load(&state, &auth, &id).await?;
    let _control = state
        .batch_runtime
        .try_control(&id)
        .ok_or_else(|| conflict("A worker or control operation is already running"))?;
    if !matches!(
        job.phase,
        BatchPhase::UploadPendingOutcome | BatchPhase::SubmitPendingOutcome
    ) {
        return Err(conflict(
            "Only an uncertain upload/submit requires reconciliation",
        ));
    }
    let context = worker::bound_context(&state, &auth, &job, false).await?;
    if context.transport.kind() != NativeBatchKind::Openai {
        return Err(conflict(
            "This native API has no verified ownership reconciliation; no create will be retried",
        ));
    }
    let store = state.batch_store.clone();
    let saved = job.clone();
    let mut intents = blocking(move || store.pending_mutations(&saved.owner, &saved.id)).await?;
    if intents.len() != 1 {
        return Err(conflict("Uncertain mutation journal is ambiguous"));
    }
    let intent = intents.remove(0);
    let (remote, status) = if job.phase == BatchPhase::UploadPendingOutcome {
        let page = context
            .transport
            .upload_reconciliation_page(request.after.as_deref())
            .await
            .map_err(ApiErrorResponse::from)?;
        let filename = format!("lr-{}.jsonl", intent.id);
        let candidates = page
            .data
            .into_iter()
            .filter(|f| f.filename == filename && f.purpose == "batch")
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            return Err(conflict("This page has no unique proven upload. No create was retried; use an explicit next-page cursor if needed"));
        }
        let store = state.batch_store.clone();
        let saved = job.clone();
        let expected = blocking(move || {
            store.content(
                &saved.owner,
                saved
                    .prepared_snapshot_file_id
                    .as_deref()
                    .ok_or(lr_batches::StoreError::Conflict)?,
            )
        })
        .await?;
        let file = &candidates[0];
        if file.bytes != expected.len() as u64
            || context
                .transport
                .download_file(&file.id)
                .await
                .map_err(ApiErrorResponse::from)?
                != expected
        {
            return Err(conflict(
                "Uploaded file does not match the exact sealed snapshot",
            ));
        }
        (file.id.clone(), lr_batches::BatchStatus::Validating)
    } else {
        let page = context
            .transport
            .reconciliation_page(request.after.as_deref())
            .await
            .map_err(ApiErrorResponse::from)?;
        let candidates = page
            .data
            .into_iter()
            .filter(|b| {
                b["metadata"]["localrouter_nonce"] == intent.id
                    && b["metadata"]["localrouter_snapshot"].as_str()
                        == job.snapshot_fingerprint.as_deref()
                    && b["input_file_id"].as_str() == job.remote_input_file_id.as_deref()
                    && b["endpoint"] == job.endpoint
            })
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            return Err(conflict(
                "This page has no unique proven native batch. No inference was resubmitted",
            ));
        }
        let batch: lr_providers::batches::NativeBatchObject = serde_json::from_value(
            candidates
                .into_iter()
                .next()
                .ok_or_else(|| conflict("Missing proven batch"))?,
        )
        .map_err(|_| ApiErrorResponse::bad_gateway("Malformed proven native batch"))?;
        batch.validate().map_err(ApiErrorResponse::from)?;
        (batch.id, worker::status(batch.status))
    };
    let store = state.batch_store.clone();
    let owner = job.owner.clone();
    blocking(move || store.complete_mutation(&owner, &intent, &remote, status)).await?;
    Ok(Json(object(&state, load(&state, &auth, &id).await?).await?))
}
