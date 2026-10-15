//! Owned local Files API. Uploading never contacts an inference provider.
use crate::{
    middleware::error::{ApiErrorResponse, ApiResult},
    state::{AppState, AuthContext},
};
use axum::{
    extract::{Multipart, Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use lr_batches::{
    FileDeleted, FileList, FileObject, StoreError, DEFAULT_RETENTION, MAX_FILE_BYTES,
};
use serde::Deserialize;

pub(crate) fn owner(state: &AppState, auth: &AuthContext) -> ApiResult<String> {
    if auth.credential_scope.is_none() {
        return Err(ApiErrorResponse::forbidden(
            "Persistent files require a real client credential",
        ));
    }
    let client = state
        .client_manager
        .get_client(&auth.api_key_id)
        .filter(|c| c.enabled)
        .ok_or_else(|| {
            ApiErrorResponse::forbidden(
                "Persistent files require a currently enabled client credential",
            )
        })?;
    Ok(client.id)
}
fn error(error: StoreError) -> ApiErrorResponse {
    match error {
        StoreError::Invalid(message) => ApiErrorResponse::bad_request(message),
        StoreError::NotFound => ApiErrorResponse::not_found("File not found"),
        StoreError::Conflict => ApiErrorResponse::new(
            StatusCode::CONFLICT,
            "file_in_use",
            "File is referenced by an active job",
        ),
        StoreError::Quota => ApiErrorResponse::new(
            StatusCode::INSUFFICIENT_STORAGE,
            "storage_limit_error",
            "Local file quota exceeded",
        ),
        StoreError::Storage(detail) => {
            tracing::error!("Owned file storage failed: {detail}");
            ApiErrorResponse::internal_error("Local file storage is unavailable")
        }
    }
}
async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> lr_batches::Result<T> + Send + 'static,
) -> ApiResult<T> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| ApiErrorResponse::internal_error("File storage task failed"))?
        .map_err(error)
}

#[derive(Deserialize, utoipa::ToSchema)]
#[allow(dead_code)]
pub struct FileUpload {
    #[schema(value_type=String, format=Binary)]
    file: Vec<u8>,
    purpose: String,
    #[serde(rename = "expires_after[anchor]")]
    expires_after_anchor: Option<String>,
    #[serde(rename = "expires_after[seconds]")]
    expires_after_seconds: Option<i64>,
}
#[utoipa::path(post,path="/v1/files",tag="files",request_body(content=FileUpload,content_type="multipart/form-data"),
    responses((status=200,description="Owned local JSONL file; no upstream upload",body=FileObject),(status=400,description="Invalid upload"),(status=403,description="Real client credential required"),(status=413,description="Upload too large"),(status=507,description="Local quota exceeded")),security(("bearer_auth"=[])))]
pub async fn upload_file(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    mut multipart: Multipart,
) -> ApiResult<Json<FileObject>> {
    let client = owner(&state, &auth)?;
    let _permit = state
        .file_upload_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiErrorResponse::rate_limited_with_retry("Two uploads are already in progress", 1)
        })?;
    let mut content = None;
    let mut filename = None;
    let mut purpose = None;
    let mut anchor = None;
    let mut seconds = None;
    let mut fields = 0;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|_| ApiErrorResponse::bad_request("Invalid multipart body"))?
    {
        fields += 1;
        if fields > 4 {
            return Err(ApiErrorResponse::bad_request("Too many multipart fields"));
        }
        let name = field.name().unwrap_or("").to_owned();
        if name == "file" {
            if content.is_some() {
                return Err(ApiErrorResponse::bad_request("Duplicate file field"));
            }
            filename = Some(
                field
                    .file_name()
                    .ok_or_else(|| ApiErrorResponse::bad_request("File requires a filename"))?
                    .to_owned(),
            );
            let mut bytes = Vec::new();
            while let Some(chunk) = field
                .chunk()
                .await
                .map_err(|_| ApiErrorResponse::bad_request("Invalid file body"))?
            {
                if chunk.len() > MAX_FILE_BYTES - bytes.len() {
                    return Err(ApiErrorResponse::new(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "request_too_large",
                        "Local files are limited to 16 MiB",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            content = Some(bytes);
        } else {
            let mut bytes = Vec::new();
            while let Some(chunk) = field
                .chunk()
                .await
                .map_err(|_| ApiErrorResponse::bad_request("Invalid multipart field"))?
            {
                if chunk.len() > 128usize.saturating_sub(bytes.len()) {
                    return Err(ApiErrorResponse::bad_request(
                        "Multipart control field exceeds 128 bytes",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            let value = String::from_utf8(bytes)
                .map_err(|_| ApiErrorResponse::bad_request("Control fields must be UTF-8"))?;
            match name.as_str() {
                "purpose" if purpose.is_none() => purpose = Some(value),
                "expires_after[anchor]" if anchor.is_none() => anchor = Some(value),
                "expires_after[seconds]" if seconds.is_none() => {
                    seconds = Some(value.parse::<i64>().map_err(|_| {
                        ApiErrorResponse::bad_request("Expiry seconds must be an integer")
                    })?)
                }
                _ => {
                    return Err(ApiErrorResponse::bad_request(
                        "Unknown or duplicate multipart field",
                    ))
                }
            }
        }
    }
    if anchor.is_some() != seconds.is_some() || anchor.as_ref().is_some_and(|s| s != "created_at") {
        return Err(ApiErrorResponse::bad_request(
            "Expiry requires anchor=created_at and seconds",
        ));
    }
    let content = content.ok_or_else(|| ApiErrorResponse::bad_request("Missing file"))?;
    let filename = filename.ok_or_else(|| ApiErrorResponse::bad_request("Missing filename"))?;
    let purpose = purpose.ok_or_else(|| ApiErrorResponse::bad_request("Missing purpose=batch"))?;
    // A client disabled during a slow upload cannot create persistent data.
    owner(&state, &auth)?;
    let store = state.batch_store.clone();
    Ok(Json(
        blocking(move || {
            let _upload_permit = _permit;
            store.upload(
                &client,
                &filename,
                &purpose,
                &content,
                seconds.unwrap_or(DEFAULT_RETENTION),
            )
        })
        .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesQuery {
    purpose: Option<String>,
    after: Option<String>,
    limit: Option<usize>,
    order: Option<String>,
}
#[utoipa::path(get,path="/v1/files",tag="files",params(("purpose"=Option<String>,Query),("after"=Option<String>,Query),("limit"=Option<usize>,Query,description="1..100, default 20"),("order"=Option<String>,Query,description="asc or desc, default desc")),responses((status=200,description="Only this client's local files",body=FileList)),security(("bearer_auth"=[])))]
pub async fn list_files(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(query): Query<FilesQuery>,
) -> ApiResult<Json<FileList>> {
    let client = owner(&state, &auth)?;
    let descending = match query.order.as_deref().unwrap_or("desc") {
        "desc" => true,
        "asc" => false,
        _ => return Err(ApiErrorResponse::bad_request("order must be asc or desc")),
    };
    let store = state.batch_store.clone();
    Ok(Json(
        blocking(move || {
            store.list(
                &client,
                query.purpose.as_deref(),
                query.after.as_deref(),
                query.limit.unwrap_or(20),
                descending,
            )
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/files/{file_id}",tag="files",params(("file_id"=String,Path)),responses((status=200,description="Owned file metadata",body=FileObject),(status=404,description="Missing, expired or unowned file")),security(("bearer_auth"=[])))]
pub async fn retrieve_file(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> ApiResult<Json<FileObject>> {
    let client = owner(&state, &auth)?;
    let store = state.batch_store.clone();
    Ok(Json(blocking(move || store.get(&client, &id)).await?))
}
#[utoipa::path(get,path="/v1/files/{file_id}/content",tag="files",params(("file_id"=String,Path)),responses((status=200,description="Original immutable JSONL",content_type="application/jsonl",body=String),(status=404,description="Missing, expired or unowned file")),security(("bearer_auth"=[])))]
pub async fn file_content(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let client = owner(&state, &auth)?;
    let store = state.batch_store.clone();
    let content = blocking(move || store.content(&client, &id)).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/jsonl"),
            (header::CACHE_CONTROL, "no-store"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"batch.jsonl\"",
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        content,
    )
        .into_response())
}
#[utoipa::path(delete,path="/v1/files/{file_id}",tag="files",params(("file_id"=String,Path)),responses((status=200,description="Local file deleted",body=FileDeleted),(status=404,description="Missing or unowned file"),(status=409,description="Active job requires this file")),security(("bearer_auth"=[])))]
pub async fn delete_file(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> ApiResult<Json<FileDeleted>> {
    let client = owner(&state, &auth)?;
    let store = state.batch_store.clone();
    Ok(Json(blocking(move || store.delete(&client, &id)).await?))
}

/// File metadata and content must not enter browser/proxy caches.
pub(crate) async fn private_response(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
