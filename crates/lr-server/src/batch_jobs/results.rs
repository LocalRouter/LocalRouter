//! Results retain provider envelopes. Aggregates never invent per-row billing.
use crate::{
    middleware::error::{ApiErrorResponse, ApiResult},
    routes::files::blocking,
    state::AppState,
};
use lr_batches::{BatchBilling, BatchCostSource, BatchJob, BatchResultKind, BatchUsage};
use lr_providers::batches::{NativeBatchKind, NativeBatchObject, NativeBatchTransport};
use serde_json::Value;
use std::collections::HashSet;

pub(crate) async fn collect(
    state: &AppState,
    job: &BatchJob,
    transport: &NativeBatchTransport,
    native: &NativeBatchObject,
) -> ApiResult<()> {
    let mut all = Vec::new();
    if transport.kind() == NativeBatchKind::Openrouter {
        if let Some(rows) = &native.results {
            let mut bytes = Vec::new();
            for row in rows {
                serde_json::to_writer(&mut bytes, row)
                    .map_err(|_| ApiErrorResponse::bad_gateway("Invalid native result"))?;
                bytes.push(b'\n');
            }
            store_result(state, job, BatchResultKind::Output, bytes, &mut all).await?;
        }
    } else {
        for (kind, file) in [
            (BatchResultKind::Output, &job.remote_output_file_id),
            (BatchResultKind::Error, &job.remote_error_file_id),
        ] {
            if let Some(file) = file {
                let bytes = transport
                    .download_file(file)
                    .await
                    .map_err(ApiErrorResponse::from)?;
                store_result(state, job, kind, bytes, &mut all).await?;
            }
        }
    }
    let store = state.batch_store.clone();
    let saved = job.clone();
    let (manifest, reservation) = blocking(move || {
        Ok((
            store.manifest(&saved.owner, &saved.id)?,
            store.job_reservation(&saved.owner, &saved.id)?,
        ))
    })
    .await?;
    let expected: HashSet<_> = manifest.iter().map(String::as_str).collect();
    if all.iter().any(|r| {
        !r["custom_id"]
            .as_str()
            .is_some_and(|id| expected.contains(id))
    }) {
        return Err(ApiErrorResponse::bad_gateway(
            "Native result is outside the owned manifest",
        ));
    }
    let billing = aggregate(
        job,
        native,
        &all,
        reservation.as_ref(),
        transport.kind() == NativeBatchKind::Openrouter,
    )?;
    if let Some(billing) = billing {
        let fingerprint = lr_router::explanation::fingerprint(
            serde_json::to_value(&billing)
                .map_err(|_| ApiErrorResponse::bad_gateway("Invalid native accounting"))?,
        );
        let store = state.batch_store.clone();
        let saved = job.clone();
        blocking(move || {
            store.record_batch_billing(&saved.owner, &saved.id, &fingerprint, billing)?;
            store.settle_batch_accounting(&saved.owner, &saved.id)?;
            store.release_result_storage(&saved.owner, &saved.id)
        })
        .await?;
    }
    Ok(())
}
async fn store_result(
    state: &AppState,
    job: &BatchJob,
    kind: BatchResultKind,
    bytes: Vec<u8>,
    all: &mut Vec<Value>,
) -> ApiResult<()> {
    if bytes.len() > job.options.result_limit_bytes as usize {
        return Err(ApiErrorResponse::bad_gateway(
            "Native results exceed this job's reserved result limit",
        ));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| ApiErrorResponse::bad_gateway("Native JSONL is not UTF-8"))?;
    for line in text.lines() {
        if !line.trim().is_empty() {
            all.push(
                serde_json::from_str(line)
                    .map_err(|_| ApiErrorResponse::bad_gateway("Invalid native result JSONL"))?,
            );
        }
    }
    if all.len() > job.rows {
        return Err(ApiErrorResponse::bad_gateway(
            "Native results exceed the submitted manifest",
        ));
    }
    let store = state.batch_store.clone();
    let saved = job.clone();
    blocking(move || store.store_batch_result(&saved.owner, &saved.id, kind, &bytes)).await?;
    Ok(())
}
fn number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0)
}
fn tokens(usage: &Value, embedding: bool) -> Option<(u64, u64)> {
    Some((
        usage
            .get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))?
            .as_u64()?,
        usage
            .get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(Value::as_u64)
            .or_else(|| embedding.then_some(0))?,
    ))
}
fn aggregate(
    job: &BatchJob,
    native: &NativeBatchObject,
    rows: &[Value],
    reservation: Option<&BatchUsage>,
    openrouter: bool,
) -> ApiResult<Option<BatchBilling>> {
    let Some(counts) = &native.request_counts else {
        return Ok(None);
    };
    if counts.total != job.rows || counts.completed.checked_add(counts.failed) != Some(job.rows) {
        return Ok(None);
    };
    let mut input = 0u64;
    let mut output = 0u64;
    let mut seen = HashSet::new();
    let mut usage_known = rows.len() == job.rows;
    for row in rows {
        let id = row["custom_id"]
            .as_str()
            .ok_or_else(|| ApiErrorResponse::bad_gateway("Native result has no custom ID"))?;
        if !seen.insert(id) {
            return Err(ApiErrorResponse::bad_gateway(
                "Duplicate native result custom ID",
            ));
        }
        let body = &row["response"]["body"];
        if let Some((i, o)) = tokens(&body["usage"], job.endpoint == "/v1/embeddings") {
            input = input
                .checked_add(i)
                .ok_or_else(|| ApiErrorResponse::bad_gateway("Invalid token totals"))?;
            output = output
                .checked_add(o)
                .ok_or_else(|| ApiErrorResponse::bad_gateway("Invalid token totals"))?;
        } else if row["error"]["code"] != "batch_expired" {
            usage_known = false;
        }
    }
    if let Some((i, o)) = native
        .usage
        .as_ref()
        .and_then(|u| tokens(u, job.endpoint == "/v1/embeddings"))
    {
        input = i;
        output = o;
        usage_known = true;
    }
    // Missing results never imply zero usage. Preserve conservative token
    // bounds while exposing unknown actuals; cost remains unknown if unpriced.
    if !usage_known {
        if let Some(reserved) = reservation {
            input = reserved.input_tokens;
            output = reserved.output_tokens;
        } else {
            return Ok(None);
        }
    };
    let usage = native.usage.as_ref();
    let byok = usage
        .and_then(|u| u.get("is_byok"))
        .and_then(Value::as_bool);
    let reported = usage.and_then(|u| number(u.get("cost")));
    let inference = usage.and_then(|u| number(u.pointer("/cost_details/upstream_inference_cost")));
    let estimate = job
        .options
        .native_price
        .as_ref()
        .and_then(|p| p.estimate(input, output));
    let source = if openrouter && byok.is_none() {
        BatchCostSource::Unknown
    } else if byok == Some(true) {
        BatchCostSource::ByokFeeAndInferenceEstimate
    } else if reported.is_some() {
        BatchCostSource::ProviderReported
    } else if usage_known && estimate.is_some() {
        BatchCostSource::NativePriceEstimate
    } else {
        BatchCostSource::Unknown
    };
    let cost = if openrouter && byok.is_none() {
        None
    } else if byok == Some(true) {
        reported.zip(inference).map(|(a, b)| a + b)
    } else {
        reported.or_else(|| usage_known.then_some(estimate).flatten())
    };
    Ok(Some(BatchBilling {
        source,
        quota_usage: BatchUsage {
            requests: job.rows as u64,
            input_tokens: input,
            output_tokens: output,
            cost_usd: cost,
        },
        actual_input_tokens: usage_known.then_some(input),
        actual_output_tokens: usage_known.then_some(output),
        reported_charge_usd: reported,
        upstream_inference_estimate_usd: inference
            .or_else(|| usage_known.then_some(estimate).flatten()),
        is_byok: byok,
        successful_requests: counts.completed as u64,
        failed_requests: counts.failed as u64,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn job() -> BatchJob {
        serde_json::from_value(json!({"id":"batch-lr-test","owner":"alice","input_file_id":"file-local","provider_instance":"fixture","credential_binding":"a","endpoint":"/v1/chat/completions","model":"fixture-model","request_fingerprint":"b","policy_fingerprint":"c","revision":1,"created_at":1,"updated_at":1,"rows":1,"phase":"terminal","status":"completed","accounting_settled":false})).unwrap()
    }
    fn native() -> NativeBatchObject {
        serde_json::from_value(json!({"id":"batch-native","object":"batch","endpoint":"/v1/chat/completions","completion_window":"24h","status":"completed","created_at":1,"request_counts":{"total":1,"completed":1,"failed":0}})).unwrap()
    }
    #[test]
    fn byok_fee_and_inference_remain_distinct_and_missing_scope_is_unknown() {
        let mut native = native();
        native.usage = Some(
            json!({"input_tokens":10,"output_tokens":3,"is_byok":true,"cost":0.02,"cost_details":{"upstream_inference_cost":0.20}}),
        );
        let billing = aggregate(&job(), &native, &[], None, true)
            .unwrap()
            .unwrap();
        assert_eq!(billing.source, BatchCostSource::ByokFeeAndInferenceEstimate);
        assert_eq!(billing.reported_charge_usd, Some(0.02));
        assert_eq!(billing.upstream_inference_estimate_usd, Some(0.20));
        assert!((billing.quota_usage.cost_usd.unwrap() - 0.22).abs() < 1e-12);
        native
            .usage
            .as_mut()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("is_byok");
        let billing = aggregate(&job(), &native, &[], None, true)
            .unwrap()
            .unwrap();
        assert_eq!(billing.quota_usage.cost_usd, None);
        assert_eq!(billing.reported_charge_usd, Some(0.02));
    }
    #[test]
    fn missing_usage_preserves_conservative_bounds_and_actual_unknowns() {
        let reserve = BatchUsage {
            requests: 1,
            input_tokens: 200,
            output_tokens: 50,
            cost_usd: Some(0.1),
        };
        let native = native();
        let rows = vec![json!({"custom_id":"one","error":{"code":"unknown_failure"}})];
        let billing = aggregate(&job(), &native, &rows, Some(&reserve), false)
            .unwrap()
            .unwrap();
        assert_eq!(billing.actual_input_tokens, None);
        assert_eq!(billing.actual_output_tokens, None);
        assert_eq!(billing.quota_usage.input_tokens, 200);
        assert_eq!(billing.quota_usage.output_tokens, 50);
        assert_eq!(billing.quota_usage.cost_usd, None);
        assert!(tokens(&json!({"prompt_tokens":3}), false).is_none());
        assert_eq!(tokens(&json!({"prompt_tokens":3}), true), Some((3, 0)));
    }
    #[test]
    fn duplicate_rows_conflict_and_native_estimate_has_visible_provenance() {
        let native = native();
        let row = json!({"custom_id":"one","response":{"body":{"usage":{"prompt_tokens":3,"completion_tokens":2}}}});
        assert!(aggregate(&job(), &native, &[row.clone(), row.clone()], None, false).is_err());
        let mut job = job();
        job.options.native_price = Some(lr_batches::NativeBatchPrice {
            source: "verified test native table".into(),
            checked_at: 1,
            input_ceiling_per_million: 1.0,
            output_ceiling_per_million: 5.0,
        });
        let bill = aggregate(&job, &native, &[row], None, false)
            .unwrap()
            .unwrap();
        assert_eq!(bill.source, BatchCostSource::NativePriceEstimate);
        assert_eq!(bill.reported_charge_usd, None);
        assert_eq!(bill.quota_usage.cost_usd, Some(0.000013));
    }
}
