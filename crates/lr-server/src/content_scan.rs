//! Secret Scan for the non-inference routes that carry user text to a
//! provider: embeddings, moderations, rerank, search, OCR, image generation,
//! speech, video and token counting (JSON bodies, read by the secret
//! boundary in `secret_placeholders`), and the text fields of multipart
//! uploads (audio transcription/translation and image edit prompts,
//! multipart video), which their handlers scan with [`scan_form_fields`].
//!
//! The policy is the inference routes' policy: Redact masks findings with
//! vault placeholders, Ask asks per finding, Notify notifies and forwards,
//! remembered decisions apply. A detected secret that is still present after
//! masking (a field masking does not reach, or masking that failed) refuses
//! the request in Redact/Ask instead of forwarding it.
use crate::{middleware::error::ApiErrorResponse, state::AppState};
use axum::http::{header, HeaderMap, Method};
use lr_secret_scanner::placeholders::Vault;
use lr_types::secret_context::{self, SecretContext};
use serde_json::{json, Value};
use std::{borrow::Cow, sync::Arc};

tokio::task_local! {
    /// The vault of a multipart request in Redact/Ask, set by the secret
    /// boundary around the handler.
    static FORM_VAULT: Arc<Vault>;
}

/// Run `handler` with `vault` as the vault [`scan_form_fields`] masks with
/// (and as the request's secret context).
pub(crate) async fn with_form_vault<F: std::future::Future>(
    vault: Arc<Vault>,
    handler: F,
) -> F::Output {
    let context: Arc<dyn SecretContext> = vault.clone();
    FORM_VAULT
        .scope(vault, secret_context::scope(Some(context), handler))
        .await
}

/// How the secret boundary handles a POST route that is not inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentRoute {
    /// A JSON body the boundary reads; its text is given by [`projection`].
    Json,
    /// A multipart form; the handler scans its text fields with
    /// [`scan_form_fields`] inside the vault scope the boundary sets up.
    Form,
}

fn is_multipart(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("multipart/form-data")
        })
}

/// The content route `path` (without its `/v1` prefix) is, if any.
pub(crate) fn classify(method: &Method, path: &str, headers: &HeaderMap) -> Option<ContentRoute> {
    if method != Method::POST {
        return None;
    }
    match path {
        "/audio/transcriptions" | "/audio/translations" | "/images/edits" => {
            Some(ContentRoute::Form)
        }
        "/videos" if is_multipart(headers) => Some(ContentRoute::Form),
        "/embeddings"
        | "/moderations"
        | "/rerank"
        | "/search"
        | "/ocr"
        | "/images/generations"
        | "/audio/speech"
        | "/videos"
        | "/messages/count_tokens" => Some(ContentRoute::Json),
        _ => None,
    }
}

/// Whether the handler of the content route `path` runs the turn pipeline
/// (`routes::pipeline::run_turn_pipeline`), whose own Secret Scan covers the
/// same text. In Redact/Ask the boundary masks and marks the request scanned
/// first; in Notify the pipeline's scan is the only one, after access and
/// model approval as on chat (scanning at the boundary too would notify
/// twice).
pub(crate) fn scanned_in_turn_pipeline(path: &str) -> bool {
    matches!(
        path,
        "/rerank" | "/search" | "/ocr" | "/videos" | "/messages/count_tokens"
    )
}

/// Push every non-empty string of `value`: a string, the strings of an
/// array (recursively), or an object's `text`. Token-id arrays, URLs in
/// other fields and binary data are not text.
fn collect(value: Option<&Value>, texts: &mut Vec<Value>) {
    match value {
        Some(Value::String(text)) if !text.is_empty() => {
            texts.push(json!({"role": "user", "content": text}));
        }
        Some(Value::Array(items)) => {
            for item in items {
                collect(Some(item), texts);
            }
        }
        Some(Value::Object(object)) => collect(object.get("text"), texts),
        _ => {}
    }
}

/// The user text of a JSON content route as a chat request
/// (`{"messages":[{"role":"user","content":…},…]}`) the scanner reads, or
/// the body itself where it already has a chat shape (token counting).
pub(crate) fn projection<'a>(path: &str, body: &'a Value) -> Cow<'a, Value> {
    let fields: &[&str] = match path {
        "/messages/count_tokens" => return Cow::Borrowed(body),
        "/embeddings" | "/moderations" => &["input"],
        "/rerank" => &["query", "documents"],
        "/search" => &["query"],
        // The document itself is a URL or base64 binary; URLs are not
        // scanned on chat either.
        "/ocr" => &["filename"],
        "/images/generations" | "/videos" => &["prompt"],
        "/audio/speech" => &["input", "instructions"],
        _ => &[],
    };
    let mut texts = Vec::new();
    for field in fields {
        collect(body.get(*field), &mut texts);
    }
    Cow::Owned(json!({ "messages": texts }))
}

/// Refusal when a detected secret survived masking.
pub(crate) fn unmaskable() -> ApiErrorResponse {
    ApiErrorResponse::forbidden(
        "Secret Scan found a secret this request cannot carry masked; the request was not sent",
    )
    .with_code("secret_scan_unmaskable")
}

/// Refuse a masked request whose text (`projection`) still holds a secret
/// that should have been masked. The scanner runs again over the masked
/// text; a finding survived masking when the vault knows it (or cannot say
/// it does not), or, in Redact, whenever it is not a remembered "ignore":
/// Redact registers every new finding, so one still present was not
/// reached by masking, or could not be registered. In Ask a value the user
/// allowed is not in the vault and passes.
pub(crate) fn verify_masked(
    state: &AppState,
    owner: &str,
    vault: &Vault,
    projection: &Value,
) -> Result<(), ApiErrorResponse> {
    let session = crate::secret_placeholders::monitor_request_session();
    let Some(policy) =
        crate::routes::pipeline::stream_scan::SecretScanPolicy::resolve(state, owner, &session)
    else {
        return Ok(());
    };
    let redact = policy.action == lr_config::SecretScanAction::Redact;
    let texts: Vec<_> = lr_guardrails::text_extractor::extract_request_text(projection)
        .into_iter()
        .enumerate()
        .map(|(index, text)| lr_secret_scanner::ExtractedText {
            label: text.label,
            text: text.text,
            message_index: index,
        })
        .collect();
    if texts.is_empty() {
        return Ok(());
    }
    let survived = policy.scanner.scan(&texts).findings.iter().any(|finding| {
        let remembered = policy.remembered(state, owner, &finding.raw_text);
        remembered != crate::routes::pipeline::stream_scan::Remembered::Ignore
            && (redact || vault.knows(&finding.raw_text).unwrap_or(true))
    });
    if survived {
        Err(unmaskable())
    } else {
        Ok(())
    }
}

/// The Secret Scan action in effect for `owner` (its override, else the
/// global action).
pub(crate) fn effective_action(state: &AppState, owner: &str) -> lr_config::SecretScanAction {
    state
        .client_manager
        .get_client(owner)
        .and_then(|client| client.secret_scanning.action)
        .unwrap_or_else(|| state.config_manager.get().secret_scanning.action)
}

pub(crate) fn masks(action: &lr_config::SecretScanAction) -> bool {
    matches!(
        action,
        lr_config::SecretScanAction::Redact | lr_config::SecretScanAction::Ask
    )
}

/// Scan the text fields of a multipart request before it is dispatched,
/// with the inference routes' policy; in Redact/Ask the fields are masked in
/// place. Redact/Ask without the boundary's vault ([`with_form_vault`]) is
/// refused: the request could not be masked.
pub(crate) async fn scan_form_fields(
    state: &AppState,
    owner: &str,
    model: &str,
    fields: &mut [&mut String],
) -> Result<(), ApiErrorResponse> {
    let action = effective_action(state, owner);
    if action == lr_config::SecretScanAction::Off {
        return Ok(());
    }
    let vault = if masks(&action) {
        match FORM_VAULT.try_with(Arc::clone) {
            Ok(vault) => Some(vault),
            Err(_) => return Err(unmaskable()),
        }
    } else {
        None
    };
    let mut value = json!({
        "messages": fields
            .iter()
            .map(|field| json!({"role": "user", "content": field.as_str()}))
            .collect::<Vec<_>>()
    });
    if fields.iter().all(|field| field.is_empty()) {
        return Ok(());
    }
    let session = crate::secret_placeholders::monitor_request_session();
    match crate::routes::pipeline::scan_request_for_secrets_with_source(
        state,
        owner,
        model,
        &value,
        None,
        Some(&session),
    )
    .await
    {
        crate::routes::pipeline::SecretScanOutcome::Deny(_) => {
            return Err(lr_types::AppError::SecretScanDenied.into())
        }
        crate::routes::pipeline::SecretScanOutcome::Redact(redaction) => {
            redaction.apply(&mut value)
        }
        crate::routes::pipeline::SecretScanOutcome::Allow => {}
    }
    if let Some(vault) = vault {
        secret_context::protect(vault.as_ref(), &mut value).map_err(ApiErrorResponse::from)?;
        verify_masked(state, owner, &vault, &value)?;
        vault.mark_scanned();
    }
    for (field, message) in fields.iter_mut().zip(
        value["messages"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default(),
    ) {
        if let Some(text) = message["content"].as_str() {
            **field = text.to_owned();
        }
    }
    Ok(())
}

/// Largest text field of a multipart upload (prompt, model, options). Image
/// edit prompts may be 32,000 characters.
pub(crate) const FORM_TEXT_LIMIT: usize = 128 * 1024;

/// Read a multipart field chunk by chunk, refusing it once it passes `limit`
/// instead of buffering the rest; the error text completes "Invalid <name>
/// field: …".
pub(crate) async fn form_bytes(
    mut field: axum::extract::multipart::Field<'_>,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(|error| error.to_string())? {
        if chunk.len() > limit - data.len() {
            return Err(format!("exceeds {}", size_label(limit)));
        }
        if data.is_empty() {
            // The first chunk is taken without copying where possible.
            data = Vec::from(chunk);
        } else {
            data.extend_from_slice(&chunk);
        }
    }
    Ok(data)
}

fn size_label(bytes: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = 1024 * 1024;
    if bytes >= MIB && bytes.is_multiple_of(MIB) {
        format!("{} MiB", bytes / MIB)
    } else if bytes >= KIB && bytes.is_multiple_of(KIB) {
        format!("{} KiB", bytes / KIB)
    } else {
        format!("{bytes} bytes")
    }
}

/// A multipart text field of at most [`FORM_TEXT_LIMIT`] bytes.
pub(crate) async fn form_text(
    field: axum::extract::multipart::Field<'_>,
) -> Result<String, String> {
    String::from_utf8(form_bytes(field, FORM_TEXT_LIMIT).await?)
        .map_err(|_| "is not valid UTF-8".to_owned())
}

#[cfg(test)]
#[path = "content_scan_tests.rs"]
mod tests;
