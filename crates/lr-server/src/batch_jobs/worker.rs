use super::{
    api::conflict,
    context::{self, NativeContext},
    policy, rows, BatchWorkerGuard,
};
use crate::{
    middleware::error::{ApiErrorResponse, ApiResult},
    routes::files::{blocking, owner},
    state::{AppState, AuthContext},
};
use lr_batches::{BatchJob, BatchJobError, BatchMutation, BatchPhase, BatchStatus, BatchUsage};
use lr_providers::{
    batches::{
        BatchInlineRow, BatchOutputExpiry, NativeBatchKind, NativeBatchObject, NativeBatchStatus,
    },
    gateway::GatewayPreferences,
};

// Verified native Batch table, 2026-10-15. Text subset uses the maximum
// long-context/cache-write input and output tiers. Savings from cache reads or
// short contexts are deliberately not promised by this conservative estimate.
pub(crate) fn native_price(context: &NativeContext) -> Option<lr_batches::NativeBatchPrice> {
    if !context.transport.uses_public_openai_pricing() {
        return None;
    }
    let (input, output) = match context.model.as_str() {
        "gpt-6-astra" => (12.50, 37.50),
        "gpt-6.1-sol" => (2.50, 7.50),
        "gpt-6-luna" => (0.125, 0.375),
        _ => return None,
    };
    Some(lr_batches::NativeBatchPrice{source:"https://developers.openai.com/api/docs/pricing#flagship-models (batch; long-context/cache-write ceiling; verified 2026-10-15)".into(),checked_at:1792022400,input_ceiling_per_million:input,output_ceiling_per_million:output})
}

pub(crate) fn status(value: NativeBatchStatus) -> BatchStatus {
    match value {
        NativeBatchStatus::Validating => BatchStatus::Validating,
        NativeBatchStatus::InProgress => BatchStatus::InProgress,
        NativeBatchStatus::Finalizing => BatchStatus::Finalizing,
        NativeBatchStatus::Cancelling => BatchStatus::Cancelling,
        NativeBatchStatus::Completed => BatchStatus::Completed,
        NativeBatchStatus::Failed => BatchStatus::Failed,
        NativeBatchStatus::Expired => BatchStatus::Expired,
        NativeBatchStatus::Cancelled => BatchStatus::Cancelled,
    }
}
pub(crate) async fn bound_context(
    state: &AppState,
    auth: &AuthContext,
    job: &BatchJob,
    check_policy: bool,
) -> ApiResult<NativeContext> {
    owner(state, auth)?;
    let context = context::context(
        state,
        auth,
        &format!("{}/{}", job.provider_instance, job.model),
    )?;
    let store = state.batch_store.clone();
    let salt = blocking(move || store.binding_salt()).await?;
    if context
        .transport
        .binding_fingerprint(&salt)
        .map_err(ApiErrorResponse::from)?
        != job.credential_binding
    {
        return Err(conflict("The original native provider account/origin is unavailable; restore it to control this job"));
    }
    if context.permission_model
        != job
            .options
            .permission_model
            .as_deref()
            .unwrap_or(&job.model)
    {
        return Err(conflict("Native model permission binding changed"));
    }
    if check_policy {
        state
            .router
            .check_provider_privacy(&job.owner, &job.provider_instance, false)
            .map_err(ApiErrorResponse::from)?;
        if state.config_manager.client_privacy(&job.owner).no_retention {
            return Err(conflict(
                "No-retention policy does not allow persistent batch submission",
            ));
        }
    }
    if check_policy && context::seal(state, auth, &context)?.fingerprint != job.policy_fingerprint {
        return Err(conflict("Policy changed after preparation; cancel this undispatched job and prepare a new snapshot"));
    }
    Ok(context)
}
pub(crate) fn start(
    state: &AppState,
    auth: &AuthContext,
    job: &BatchJob,
    submit: bool,
) -> ApiResult<()> {
    let permit = state.batch_runtime.try_preflight().ok_or_else(|| {
        ApiErrorResponse::rate_limited("Two native validations/submissions are already running")
    })?;
    start_admitted(state, auth, job, submit, permit)
}
pub(crate) fn start_admitted(
    state: &AppState,
    auth: &AuthContext,
    job: &BatchJob,
    submit: bool,
    permit: tokio::sync::OwnedSemaphorePermit,
) -> ApiResult<()> {
    let guard = state
        .batch_runtime
        .claim_worker(&job.id)
        .ok_or_else(|| conflict("This batch already has a worker"))?;
    let state = state.clone();
    let auth = auth.clone();
    let job = job.clone();
    tokio::spawn(async move {
        let _permit = permit;
        if let Err(error) = run(&state, &auth, job.clone(), submit, &guard).await {
            // Policy dialogs and upstream errors may contain request/provider
            // details. Persist a bounded generic diagnostic, never raw bodies.
            let store = state.batch_store.clone();
            let owner = job.owner;
            let id = job.id;
            let price_unavailable =
                error.error.error.code.as_deref() == Some("native_price_unavailable");
            let diagnostic=BatchJobError{code:if price_unavailable{"native_price_unavailable"}else{"worker_paused"}.into(),message:if price_unavailable{"This cost-limited client requires a verified native price ceiling. The selected native model/account has no verified ceiling; nothing was submitted."}else{"Batch paused. Review permissions, policy, account and quotas. Drafts may resume; uncertain native mutations are never repeated."}.into(),row:None};
            let _ = blocking(move || store.record_job_error(&owner, &id, diagnostic)).await;
        }
        drop(guard);
    });
    Ok(())
}
fn running(guard: &BatchWorkerGuard) -> ApiResult<()> {
    if guard.token.is_cancelled() {
        Err(conflict("Batch worker stopped"))
    } else {
        Ok(())
    }
}
async fn run(
    state: &AppState,
    auth: &AuthContext,
    mut job: BatchJob,
    explicit_submit: bool,
    guard: &BatchWorkerGuard,
) -> ApiResult<()> {
    running(guard)?;
    if state.config_manager.client_privacy(&job.owner).no_retention {
        return Err(conflict(
            "No-retention policy does not allow preparing/submitting persistent jobs",
        ));
    }
    let mut context = bound_context(state, auth, &job, false).await?;
    if job.phase == BatchPhase::Draft {
        context::verify_model(&context, &job.endpoint).await?;
        let mut seal = context::seal(state, auth, &context)?;
        let store = state.batch_store.clone();
        let saved = job.clone();
        let fingerprint = seal.fingerprint.clone();
        job = blocking(move || store.update_draft_policy(&saved.owner, &saved.id, &fingerprint))
            .await?;
        let store = state.batch_store.clone();
        let saved = job.clone();
        let (bytes, edits) = blocking(move || {
            Ok((
                store.content(&saved.owner, &saved.input_file_id)?,
                store.row_edits(&saved.owner, &saved.id)?,
            ))
        })
        .await?;
        let (mut input, _) = rows::parse_input(&bytes, &job.endpoint)?;
        for (index, row) in input.iter_mut().enumerate() {
            running(guard)?;
            // A resume rechecks every row, beginning with the last saved edit.
            if let Some(body) = edits.get(&row.custom_id) {
                row.body = body.clone();
            }
            row.body["model"] = serde_json::Value::String(job.model.clone());
            let current = context::seal(state, auth, &context)?;
            if current.fingerprint != seal.fingerprint {
                return Err(conflict("Policy changed during native validation"));
            }
            let checked = tokio::select! { _=guard.token.cancelled()=>return Err(conflict("Validation stopped")), result=policy::validate_row(state,auth,&job,&row.body,&row.custom_id)=>result? };
            running(guard)?;
            let after = context::seal(state, auth, &context)?;
            if after.fingerprint != seal.fingerprint {
                if !context::approval_relaxation(&seal, &after) {
                    return Err(conflict("Policy changed during native validation"));
                }
                let store = state.batch_store.clone();
                let saved = job.clone();
                let fingerprint = after.fingerprint.clone();
                job = blocking(move || {
                    store.update_draft_policy(&saved.owner, &saved.id, &fingerprint)
                })
                .await?;
                seal = after;
            }
            row.body = checked;
            context::validate_kind(context.transport.kind(), &job.endpoint, &row.body)?;
            let store = state.batch_store.clone();
            let saved = job.clone();
            let row_id = row.custom_id.clone();
            let body = row.body.clone();
            blocking(move || {
                store.save_row_edit(&saved.owner, &saved.id, &row_id, &body)?;
                store.validation_progress(&saved.owner, &saved.id, index + 1)
            })
            .await?;
        }
        running(guard)?;
        let mut snapshot = Vec::new();
        for row in &input {
            serde_json::to_writer(&mut snapshot, row)
                .map_err(|_| ApiErrorResponse::bad_request("Invalid batch snapshot"))?;
            snapshot.push(b'\n');
        }
        if snapshot.len() > lr_batches::MAX_FILE_BYTES {
            return Err(ApiErrorResponse::bad_request(
                "Prepared batch exceeds local input limit",
            ));
        }
        let store = state.batch_store.clone();
        // Row edits/progress increment the durable revision.
        job = super::api::load(state, auth, &job.id).await?;
        let saved = job.clone();
        let fp = seal.fingerprint;
        job = blocking(move || {
            let file = store.upload(
                &saved.owner,
                "prepared-batch.jsonl",
                "batch",
                &snapshot,
                saved.options.output_retention_seconds as i64,
            )?;
            match store.prepare_job(&saved.owner, &saved.id, saved.revision, &file.id, &fp) {
                Ok(job) => Ok(job),
                Err(error) => {
                    let _ = store.delete(&saved.owner, &file.id);
                    Err(error)
                }
            }
        })
        .await?;
    }
    running(guard)?;
    bound_context(state, auth, &job, true).await?;
    let store = state.batch_store.clone();
    let saved = job.clone();
    let bytes = blocking(move || {
        store.content(
            &saved.owner,
            saved
                .prepared_snapshot_file_id
                .as_deref()
                .ok_or(lr_batches::StoreError::Conflict)?,
        )
    })
    .await?;
    let (input, _) = rows::parse_input(&bytes, &job.endpoint)?;
    let store = state.batch_store.clone();
    let saved = job.clone();
    let reservation = blocking(move || store.job_reservation(&saved.owner, &saved.id)).await?;
    if reservation.is_none() {
        let mut usage = BatchUsage {
            requests: job.rows as u64,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: None,
        };
        for row in &input {
            let (i, o) = rows::token_bounds(&job.endpoint, &row.body)?;
            usage.input_tokens += i;
            usage.output_tokens += o;
        }
        usage.cost_usd = job
            .options
            .native_price
            .as_ref()
            .and_then(|p| p.estimate(usage.input_tokens, usage.output_tokens));
        // Native catalog/rate provenance must be verified before reserving a
        // dollar figure. Unknown cost fails closed for cost-limited clients.
        let budgets = state
            .rate_limiter
            .batch_budget_snapshot(&job.owner)
            .await
            .map_err(ApiErrorResponse::from)?;
        if usage.cost_usd.is_none()
            && budgets
                .iter()
                .any(|b| matches!(b.metric, lr_batches::BatchMetric::Cost))
        {
            return Err(ApiErrorResponse::bad_request(
                "Verified native pricing is unavailable for this cost-limited client",
            )
            .with_code("native_price_unavailable"));
        }
        let store = state.batch_store.clone();
        let saved = job.clone();
        job =
            blocking(move || store.reserve_batch(&saved.owner, &saved.id, usage, &budgets)).await?;
    }
    let store = state.batch_store.clone();
    let saved = job.clone();
    blocking(move || store.reserve_result_storage(&saved.owner, &saved.id)).await?;
    if job.options.prepare_only && !explicit_submit {
        return Ok(());
    }
    running(guard)?;
    context = bound_context(state, auth, &job, true).await?;
    if context.transport.kind() == NativeBatchKind::Openai && job.phase == BatchPhase::Prepared {
        let store = state.batch_store.clone();
        let saved = job.clone();
        let intent = blocking(move || {
            store.begin_mutation(
                &saved.owner,
                &saved.id,
                saved.revision,
                BatchMutation::Upload,
            )
        })
        .await?;
        let filename = format!("lr-{}.jsonl", intent.id);
        // From here, a dropped response leaves a durable uncertain outcome.
        let uploaded = native_outcome(
            state,
            &job,
            &intent,
            context
                .transport
                .upload_input(bytes.clone(), &filename)
                .await,
        )
        .await?;
        let store = state.batch_store.clone();
        let owner = job.owner.clone();
        job = blocking(move || {
            store.complete_mutation(&owner, &intent, &uploaded.id, BatchStatus::Validating)
        })
        .await?;
    }
    running(guard)?;
    context = bound_context(state, auth, &job, true).await?;
    let store = state.batch_store.clone();
    let saved = job.clone();
    let intent = blocking(move || {
        store.begin_mutation(
            &saved.owner,
            &saved.id,
            saved.revision,
            BatchMutation::Submit,
        )
    })
    .await?;
    let native = if context.transport.kind() == NativeBatchKind::Openai {
        let mut metadata = job.options.metadata.clone();
        metadata.insert("localrouter_nonce".into(), intent.id.clone());
        metadata.insert(
            "localrouter_snapshot".into(),
            job.snapshot_fingerprint
                .clone()
                .ok_or_else(|| conflict("Missing snapshot binding"))?,
        );
        context
            .transport
            .submit_file(
                job.remote_input_file_id
                    .as_deref()
                    .ok_or_else(|| conflict("Missing uploaded input"))?,
                &job.endpoint,
                &metadata,
                job.options
                    .native_output_expiry_seconds
                    .map(|seconds| BatchOutputExpiry {
                        anchor: "created_at".into(),
                        seconds,
                    }),
            )
            .await
    } else {
        let rows = input
            .into_iter()
            .map(|row| BatchInlineRow {
                custom_id: row.custom_id,
                body: row.body,
            })
            .collect::<Vec<_>>();
        context
            .transport
            .submit_inline(
                &job.endpoint,
                &job.model,
                &rows,
                &GatewayPreferences {
                    only: job.options.provider_only.clone(),
                    ..Default::default()
                },
            )
            .await
    };
    let native = native_outcome(state, &job, &intent, native).await?;
    let store = state.batch_store.clone();
    let owner = job.owner.clone();
    let remote = native.id.clone();
    let native_status = status(native.status);
    job =
        blocking(move || store.complete_mutation(&owner, &intent, &remote, native_status)).await?;
    observe(state, &job, &native).await?;
    if native.status.is_terminal() {
        let job = super::api::load(state, auth, &job.id).await?;
        super::results::collect(state, &job, &context.transport, &native).await?;
    }
    Ok(())
}
pub(crate) async fn observe(
    state: &AppState,
    job: &BatchJob,
    native: &NativeBatchObject,
) -> ApiResult<()> {
    if native.endpoint != job.endpoint
        || native.model.as_deref().is_some_and(|m| m != job.model)
        || native
            .input_file_id
            .as_deref()
            .is_some_and(|f| Some(f) != job.remote_input_file_id.as_deref())
    {
        return Err(conflict(
            "Native status differs from the bound batch identity",
        ));
    }
    let store = state.batch_store.clone();
    let saved = job.clone();
    let native = native.clone();
    blocking(move || {
        store.observe_status(&saved.owner, &saved.id, &native.id, status(native.status))?;
        store.record_remote_details(
            &saved.owner,
            &saved.id,
            &native.id,
            native.output_file_id.as_deref(),
            native.error_file_id.as_deref(),
            native
                .request_counts
                .map(|c| lr_batches::BatchRequestCounts {
                    total: c.total,
                    completed: c.completed,
                    failed: c.failed,
                }),
            None,
        )?;
        Ok(())
    })
    .await
}

async fn native_outcome<T>(
    state: &AppState,
    job: &BatchJob,
    intent: &lr_batches::MutationIntent,
    result: lr_types::AppResult<T>,
) -> ApiResult<T> {
    if let Err(lr_types::AppError::ProviderStatus {
        status: 400 | 401 | 403 | 404 | 405 | 413 | 415 | 422,
        ..
    }) = &result
    {
        let store = state.batch_store.clone();
        let owner = job.owner.clone();
        let intent = intent.clone();
        blocking(move || {
            let job = store.reject_native_mutation(&owner, &intent)?;
            store.release_result_storage(&owner, &job.id)
        })
        .await?;
    }
    result.map_err(ApiErrorResponse::from)
}
