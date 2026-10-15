//! Native batch wire transports. The server must validate every row, reserve
//! quota and persist mutation intent before invoking a mutating operation.
//! Credentials are ephemeral and never serializable; no automatic retry exists.
use crate::gateway::GatewayPreferences;
use futures::StreamExt;
use lr_types::{AppError, AppResult};
use reqwest::{
    header::{HeaderMap, HeaderValue},
    Method, Url,
};
use reqwest_middleware::ClientWithMiddleware;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use utoipa::ToSchema;

pub const MAX_BATCH_INPUT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_BATCH_RESULT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_BATCH_ROWS: usize = 50_000;
const MAX_CONTROL_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NativeBatchKind {
    Openai,
    Openrouter,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NativeBatchStatus {
    Validating,
    InProgress,
    Finalizing,
    Cancelling,
    Completed,
    Failed,
    Expired,
    Cancelled,
}
impl NativeBatchStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Expired | Self::Cancelled
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeBatchCounts {
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeBatchObject {
    pub id: String,
    pub object: String,
    pub status: NativeBatchStatus,
    pub endpoint: String,
    pub completion_window: String,
    pub created_at: i64,
    pub input_file_id: Option<String>,
    pub model: Option<String>,
    pub output_file_id: Option<String>,
    pub error_file_id: Option<String>,
    pub request_counts: Option<NativeBatchCounts>,
    pub metadata: Option<Map<String, Value>>,
    pub usage: Option<Value>,
    pub results: Option<Vec<Value>>,
    pub error: Option<Value>,
    pub errors: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
impl NativeBatchObject {
    pub fn validate(&self) -> AppResult<()> {
        response_id(&self.id)?;
        for id in [
            &self.input_file_id,
            &self.output_file_id,
            &self.error_file_id,
        ]
        .into_iter()
        .flatten()
        {
            response_id(id)?;
        }
        if self.object != "batch"
            || !endpoint_supported(&self.endpoint)
            || self.completion_window != "24h"
            || self.created_at < 0
            || self
                .results
                .as_ref()
                .is_some_and(|v| v.len() > MAX_BATCH_ROWS)
            || self.request_counts.as_ref().is_some_and(|c| {
                c.total > MAX_BATCH_ROWS
                    || c.completed > c.total
                    || c.failed > c.total - c.completed
            })
        {
            return Err(AppError::Provider("Malformed native batch response".into()));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeBatchFile {
    pub id: String,
    pub object: String,
    pub bytes: u64,
    pub created_at: i64,
    pub filename: String,
    pub purpose: String,
    pub expires_at: Option<i64>,
}
#[derive(Deserialize)]
pub struct NativeBatchPage<T> {
    pub data: Vec<T>,
    pub has_more: bool,
    pub last_id: Option<String>,
    pub first_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeBatchCleanup {
    pub id: String,
    pub object: String,
    pub deletion: NativeBatchDeletion,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeBatchDeletion {
    pub openrouter: String,
    pub upstream: Option<NativeBatchUpstreamDeletion>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeBatchUpstreamDeletion {
    pub provider: String,
    pub status: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BatchInlineRow {
    pub custom_id: String,
    pub body: Value,
}
#[derive(Serialize)]
struct BatchProviderPin {
    only: Vec<String>,
}
/// Declaration order is a protocol requirement: OpenRouter's streaming parser
/// must see model/endpoint/provider/window before the requests array.
#[derive(Serialize)]
struct OpenRouterCreate<'a> {
    endpoint: &'a str,
    model: &'a str,
    completion_window: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<BatchProviderPin>,
    requests: &'a [BatchInlineRow],
}
#[derive(Clone, Debug, Serialize)]
pub struct BatchOutputExpiry {
    pub anchor: String,
    pub seconds: u32,
}
#[derive(Serialize)]
struct OpenAiCreate<'a> {
    input_file_id: &'a str,
    endpoint: &'a str,
    completion_window: &'static str,
    metadata: &'a HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_expires_after: Option<BatchOutputExpiry>,
}

/// Never derive Serialize or Debug for a transport containing a key.
pub struct NativeBatchTransport {
    kind: NativeBatchKind,
    root: Url,
    key: String,
    headers: HeaderMap,
    preferences: GatewayPreferences,
    client: ClientWithMiddleware,
}
impl std::fmt::Debug for NativeBatchTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeBatchTransport")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
impl NativeBatchTransport {
    pub fn openai(root: &str, key: &str) -> AppResult<Self> {
        Self::new(
            NativeBatchKind::Openai,
            root,
            key,
            GatewayPreferences::default(),
            HeaderMap::new(),
        )
    }
    pub fn openrouter(
        root: &str,
        key: &str,
        preferences: GatewayPreferences,
        app_name: Option<&str>,
        app_url: Option<&str>,
    ) -> AppResult<Self> {
        let mut headers = HeaderMap::new();
        for (name, value) in [("x-title", app_name), ("http-referer", app_url)] {
            if let Some(value) = value {
                if value.len() > 1024 {
                    return Err(AppError::InvalidParams(
                        "Batch attribution header is too long".into(),
                    ));
                }
                headers.insert(
                    name,
                    HeaderValue::from_str(value).map_err(|_| {
                        AppError::InvalidParams("Invalid batch attribution header".into())
                    })?,
                );
            }
        }
        Self::new(NativeBatchKind::Openrouter, root, key, preferences, headers)
    }
    fn new(
        kind: NativeBatchKind,
        root: &str,
        key: &str,
        preferences: GatewayPreferences,
        headers: HeaderMap,
    ) -> AppResult<Self> {
        let mut root = Url::parse(root)
            .map_err(|_| AppError::InvalidParams("Invalid batch API root".into()))?;
        if !["http", "https"].contains(&root.scheme())
            || root.host_str().is_none()
            || !root.username().is_empty()
            || root.password().is_some()
            || root.query().is_some()
            || root.fragment().is_some()
            || key.is_empty()
            || key.len() > 8192
            || HeaderValue::from_str(&format!("Bearer {key}")).is_err()
        {
            return Err(AppError::InvalidParams("Batch API requires a configured HTTP(S) origin without URL credentials, query or fragment".into()));
        }
        preferences.validate()?;
        let path = format!("{}/", root.path().trim_end_matches('/'));
        root.set_path(&path);
        Ok(Self {
            kind,
            root,
            key: key.into(),
            headers,
            preferences,
            client: crate::http_client::native_client()?,
        })
    }
    pub fn kind(&self) -> NativeBatchKind {
        self.kind
    }
    pub fn supports_cancel(&self) -> bool {
        self.kind == NativeBatchKind::Openai
    }
    pub fn supports_remote_batch_delete(&self) -> bool {
        self.kind == NativeBatchKind::Openrouter
    }
    pub fn binding_fingerprint(&self, salt: &str) -> AppResult<String> {
        if salt.len() != 64 || !salt.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(AppError::InvalidParams(
                "Invalid durable batch binding salt".into(),
            ));
        }
        let mut digest = Sha256::new();
        for value in [
            salt,
            self.root.as_str(),
            self.key.as_str(),
            match self.kind {
                NativeBatchKind::Openai => "openai",
                NativeBatchKind::Openrouter => "openrouter",
            },
        ] {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        }
        Ok(format!("{:x}", digest.finalize()))
    }
    fn url(&self, segments: &[&str]) -> AppResult<Url> {
        let mut url = self.root.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| AppError::InvalidParams("Invalid batch API path".into()))?;
            path.pop_if_empty();
            for segment in segments {
                path.push(segment);
            }
        }
        Ok(url)
    }
    fn request(
        &self,
        method: Method,
        segments: &[&str],
    ) -> AppResult<reqwest_middleware::RequestBuilder> {
        Ok(self
            .client
            .request(method, self.url(segments)?)
            .bearer_auth(&self.key)
            .headers(self.headers.clone())
            .header("accept", "application/json"))
    }
    async fn json<T: serde::de::DeserializeOwned>(
        &self,
        request: reqwest_middleware::RequestBuilder,
        bound: usize,
    ) -> AppResult<T> {
        let bytes = read_response(request, bound).await?;
        serde_json::from_slice(&bytes)
            .map_err(|_| AppError::Provider("Malformed native batch JSON response".into()))
    }
    pub async fn upload_input(
        &self,
        content: Vec<u8>,
        filename: &str,
    ) -> AppResult<NativeBatchFile> {
        self.require(NativeBatchKind::Openai)?;
        if content.is_empty()
            || content.len() > MAX_BATCH_INPUT_BYTES
            || filename.is_empty()
            || filename.len() > 128
            || !filename
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        {
            return Err(AppError::InvalidParams(
                "Invalid native batch input size or private filename".into(),
            ));
        }
        let size = content.len() as u64;
        let part = reqwest::multipart::Part::bytes(content)
            .file_name(filename.to_owned())
            .mime_str("application/jsonl")
            .map_err(|_| AppError::InvalidParams("Invalid batch content type".into()))?;
        // Purpose=batch has documented 30-day upstream default retention.
        let form = reqwest::multipart::Form::new()
            .text("purpose", "batch")
            .part("file", part);
        let file: NativeBatchFile = self
            .json(
                self.request(Method::POST, &["files"])?.multipart(form),
                MAX_CONTROL_BYTES,
            )
            .await?;
        response_id(&file.id)?;
        if file.object != "file"
            || file.purpose != "batch"
            || file.filename != filename
            || file.bytes != size
            || file.created_at < 0
        {
            return Err(AppError::Provider(
                "Malformed native batch upload acknowledgement".into(),
            ));
        }
        Ok(file)
    }
    pub async fn submit_file(
        &self,
        input_file_id: &str,
        endpoint: &str,
        metadata: &HashMap<String, String>,
        expiry: Option<BatchOutputExpiry>,
    ) -> AppResult<NativeBatchObject> {
        self.require(NativeBatchKind::Openai)?;
        valid_id(input_file_id)?;
        validate_endpoint(endpoint)?;
        if metadata.len() > 16
            || metadata.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > 64
                    || value.len() > 512
                    || key.chars().any(char::is_control)
            })
        {
            return Err(AppError::InvalidParams(
                "Batch metadata is limited to 16 bounded string pairs".into(),
            ));
        }
        if expiry
            .as_ref()
            .is_some_and(|e| e.anchor != "created_at" || !(3600..=2_592_000).contains(&e.seconds))
        {
            return Err(AppError::InvalidParams(
                "Batch output expiry must use created_at and 3600..2592000 seconds".into(),
            ));
        }
        let body = bounded_json(
            &OpenAiCreate {
                input_file_id,
                endpoint,
                completion_window: "24h",
                metadata,
                output_expires_after: expiry,
            },
            MAX_CONTROL_BYTES,
        )?;
        let batch: NativeBatchObject = self
            .json(
                self.request(Method::POST, &["batches"])?
                    .header("content-type", "application/json")
                    .body(body),
                MAX_CONTROL_BYTES,
            )
            .await?;
        batch.validate()?;
        if batch.input_file_id.as_deref() != Some(input_file_id) || batch.endpoint != endpoint {
            return Err(AppError::Provider(
                "Native batch acknowledgement changed input or endpoint".into(),
            ));
        }
        Ok(batch)
    }
    pub fn effective_inline_pin(
        &self,
        request: &GatewayPreferences,
    ) -> AppResult<Option<Vec<String>>> {
        self.require(NativeBatchKind::Openrouter)?;
        let mut effective = self.preferences.merge(request)?;
        let pin = effective.only.take();
        // This API runs the entire batch on one provider; allow_fallbacks is
        // permission rather than a requirement to perform fallback.
        effective.allow_fallbacks = None;
        if effective != GatewayPreferences::default() {
            return Err(AppError::InvalidParams("Native OpenRouter batches currently support only provider allowlists; other gateway/privacy/price controls have no verified batch mapping".into()));
        }
        Ok(pin)
    }
    pub async fn submit_inline(
        &self,
        endpoint: &str,
        model: &str,
        rows: &[BatchInlineRow],
        preferences: &GatewayPreferences,
    ) -> AppResult<NativeBatchObject> {
        self.require(NativeBatchKind::Openrouter)?;
        validate_endpoint(endpoint)?;
        if model.is_empty() || model.len() > 512 || rows.is_empty() || rows.len() > MAX_BATCH_ROWS {
            return Err(AppError::InvalidParams(
                "Invalid inline batch model or request count".into(),
            ));
        }
        let pin = self.effective_inline_pin(preferences)?;
        let mut seen = HashSet::new();
        for row in rows {
            if row.custom_id.is_empty()
                || row.custom_id.len() > 256
                || row.custom_id.chars().any(char::is_control)
                || !seen.insert(&row.custom_id)
                || !row.body.is_object()
                || row.body.get("model").is_some_and(|v| v != model)
                || row.body.get("stream").is_some_and(|v| v != false)
                || row.body.get("provider").is_some()
            {
                return Err(AppError::InvalidParams("Inline rows require unique custom IDs, an object body, matching model, no streaming and no per-row provider controls".into()));
            }
        }
        let body = bounded_json(
            &OpenRouterCreate {
                endpoint,
                model,
                completion_window: "24h",
                provider: pin.map(|only| BatchProviderPin { only }),
                requests: rows,
            },
            MAX_BATCH_INPUT_BYTES,
        )?;
        let batch: NativeBatchObject = self
            .json(
                self.request(Method::POST, &["batches"])?
                    .header("content-type", "application/json")
                    .body(body),
                MAX_CONTROL_BYTES,
            )
            .await?;
        batch.validate()?;
        if batch.endpoint != endpoint
            || batch.model.as_deref() != Some(model)
            || batch
                .request_counts
                .as_ref()
                .is_some_and(|c| c.total != rows.len())
        {
            return Err(AppError::Provider(
                "Inline batch acknowledgement changed model, endpoint or count".into(),
            ));
        }
        Ok(batch)
    }
    pub async fn retrieve(&self, id: &str) -> AppResult<NativeBatchObject> {
        valid_id(id)?;
        let batch: NativeBatchObject = self
            .json(
                self.request(Method::GET, &["batches", id])?,
                MAX_BATCH_RESULT_BYTES + 1024 * 1024,
            )
            .await?;
        batch.validate()?;
        if batch.id != id {
            return Err(AppError::Provider(
                "Batch retrieval returned another identity".into(),
            ));
        }
        if self.kind == NativeBatchKind::Openrouter
            && batch.status == NativeBatchStatus::Completed
            && batch.results.is_none()
        {
            return Err(AppError::Provider(
                "Completed OpenRouter batch omitted inline results".into(),
            ));
        }
        Ok(batch)
    }
    pub async fn cancel(&self, id: &str) -> AppResult<NativeBatchObject> {
        self.require(NativeBatchKind::Openai)?;
        valid_id(id)?;
        let batch: NativeBatchObject = self
            .json(
                self.request(Method::POST, &["batches", id, "cancel"])?,
                MAX_CONTROL_BYTES,
            )
            .await?;
        batch.validate()?;
        if batch.id != id
            || (batch.status != NativeBatchStatus::Cancelling && !batch.status.is_terminal())
        {
            return Err(AppError::Provider(
                "Invalid native cancellation acknowledgement".into(),
            ));
        }
        Ok(batch)
    }
    pub async fn download_file(&self, id: &str) -> AppResult<Vec<u8>> {
        self.require(NativeBatchKind::Openai)?;
        valid_id(id)?;
        read_response(
            self.request(Method::GET, &["files", id, "content"])?,
            MAX_BATCH_RESULT_BYTES,
        )
        .await
    }
    pub async fn delete_file(&self, id: &str) -> AppResult<()> {
        self.require(NativeBatchKind::Openai)?;
        valid_id(id)?;
        let value: Value = self
            .json(
                self.request(Method::DELETE, &["files", id])?,
                MAX_CONTROL_BYTES,
            )
            .await?;
        if value["id"] != id || value["deleted"] != true || value["object"] != "file" {
            return Err(AppError::Provider(
                "Invalid native file deletion acknowledgement".into(),
            ));
        }
        Ok(())
    }
    pub async fn delete_terminal_batch(
        &self,
        id: &str,
        status: NativeBatchStatus,
    ) -> AppResult<NativeBatchCleanup> {
        self.require(NativeBatchKind::Openrouter)?;
        valid_id(id)?;
        if !status.is_terminal() {
            return Err(AppError::InvalidParams(
                "Deleting an active batch is not cancellation".into(),
            ));
        }
        let deletion: NativeBatchCleanup = self
            .json(
                self.request(Method::DELETE, &["batches", id])?,
                MAX_CONTROL_BYTES,
            )
            .await?;
        if deletion.id != id
            || deletion.object != "batch"
            || deletion.deletion.openrouter != "deleted"
            || deletion.deletion.upstream.as_ref().is_some_and(|u| {
                u.provider.is_empty()
                    || !["deleted", "unsupported", "not_applicable"].contains(&u.status.as_str())
            })
        {
            return Err(AppError::Provider(
                "Invalid native batch cleanup acknowledgement".into(),
            ));
        }
        Ok(deletion)
    }
    /// Private reconciliation only: never return this account/workspace page
    /// as a LocalRouter ownership list. Filter by a proven saved nonce first.
    pub async fn reconciliation_page(
        &self,
        after: Option<&str>,
    ) -> AppResult<NativeBatchPage<Value>> {
        self.require(NativeBatchKind::Openai)?;
        let request = self.request(Method::GET, &["batches"])?;
        let request = page_query(request, after)?;
        let page: NativeBatchPage<Value> = self.json(request, MAX_CONTROL_BYTES).await?;
        validate_page(&page)?;
        Ok(page)
    }
    pub async fn upload_reconciliation_page(
        &self,
        after: Option<&str>,
    ) -> AppResult<NativeBatchPage<NativeBatchFile>> {
        self.require(NativeBatchKind::Openai)?;
        let request = page_query(
            self.request(Method::GET, &["files"])?
                .query(&[("purpose", "batch")]),
            after,
        )?;
        let page: NativeBatchPage<NativeBatchFile> = self.json(request, MAX_CONTROL_BYTES).await?;
        validate_page(&page)?;
        for file in &page.data {
            response_id(&file.id)?;
        }
        Ok(page)
    }
    fn require(&self, kind: NativeBatchKind) -> AppResult<()> {
        if self.kind != kind {
            return Err(AppError::InvalidParams(
                "Native batch operation is unsupported on this gateway".into(),
            ));
        }
        Ok(())
    }
}

fn endpoint_supported(endpoint: &str) -> bool {
    ["/v1/chat/completions", "/v1/responses", "/v1/embeddings"].contains(&endpoint)
}
fn validate_endpoint(endpoint: &str) -> AppResult<()> {
    if !endpoint_supported(endpoint) {
        return Err(AppError::InvalidParams(
            "Unsupported native batch endpoint".into(),
        ));
    }
    Ok(())
}
fn valid_id(id: &str) -> AppResult<()> {
    if id.is_empty()
        || id.len() > 256
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(AppError::InvalidParams(
            "Invalid native artifact identity".into(),
        ));
    }
    Ok(())
}
fn response_id(id: &str) -> AppResult<()> {
    valid_id(id).map_err(|_| AppError::Provider("Malformed native artifact identity".into()))
}
fn page_query(
    request: reqwest_middleware::RequestBuilder,
    after: Option<&str>,
) -> AppResult<reqwest_middleware::RequestBuilder> {
    let request = request.query(&[("limit", 100)]);
    if let Some(id) = after {
        valid_id(id)?;
        Ok(request.query(&[("after", id)]))
    } else {
        Ok(request)
    }
}
fn validate_page<T>(page: &NativeBatchPage<T>) -> AppResult<()> {
    if page.data.len() > 100 || (page.has_more && page.last_id.is_none()) {
        return Err(AppError::Provider(
            "Malformed native reconciliation page".into(),
        ));
    }
    for id in [&page.first_id, &page.last_id].into_iter().flatten() {
        response_id(id)?;
    }
    Ok(())
}
fn bounded_json<T: Serialize>(value: &T, bound: usize) -> AppResult<Vec<u8>> {
    struct Writer {
        bytes: Vec<u8>,
        bound: usize,
    }
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.bound.saturating_sub(self.bytes.len()) {
                return Err(std::io::Error::other(
                    "Native batch JSON exceeds size limit",
                ));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Writer {
        bytes: Vec::new(),
        bound,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| {
        AppError::InvalidParams("Native batch JSON exceeds supported size or format".into())
    })?;
    Ok(writer.bytes)
}
async fn read_response(
    request: reqwest_middleware::RequestBuilder,
    bound: usize,
) -> AppResult<Vec<u8>> {
    // A timeout can mean a mutation succeeded. The caller must retain its
    // durable pending intent and reconcile; this function never retries.
    let response = request.send().await.map_err(|_| {
        AppError::Provider("Native batch request failed; mutation outcome may be unknown".into())
    })?;
    let status = response.status();
    if !status.is_success() {
        // Provider errors can echo private inputs or credentials; do not log,
        // persist or return raw bodies through batch diagnostics.
        let message = format!("Native batch provider returned HTTP {}", status.as_u16());
        let retry_after_secs = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(|v| v.min(3600));
        if let (429, Some(retry_after_secs)) = (status.as_u16(), retry_after_secs) {
            return Err(AppError::RateLimitWithRetryAfter { retry_after_secs });
        }
        return Err(if status.is_client_error() {
            AppError::ProviderStatus {
                status: status.as_u16(),
                message,
            }
        } else if status.is_server_error() {
            AppError::ProviderUnavailable {
                status: status.as_u16(),
                message,
                retry_after_secs,
            }
        } else {
            AppError::Provider("Native batch redirect refused".into())
        });
    }
    if response
        .content_length()
        .is_some_and(|length| length > bound as u64)
    {
        return Err(AppError::Provider(
            "Native batch response exceeds supported size".into(),
        ));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| {
            AppError::Provider(
                "Native batch response interrupted; mutation outcome may be unknown".into(),
            )
        })?;
        if chunk.len() > bound.saturating_sub(bytes.len()) {
            return Err(AppError::Provider(
                "Native batch response exceeds supported size".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "batches_tests.rs"]
mod tests;
