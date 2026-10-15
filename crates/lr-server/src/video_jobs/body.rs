use crate::middleware::error::ApiErrorResponse;
use axum::{
    extract::{FromRequest, Multipart, Request},
    response::{IntoResponse, Response},
};
use lr_providers::videos::VideoRequest;
pub struct VideoBody(pub VideoRequest);
impl<S: Send + Sync> FromRequest<S> for VideoBody {
    type Rejection = Response;
    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let multipart = request
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|h| h.starts_with("multipart/form-data"));
        if !multipart {
            let crate::routes::gateway_json::GatewayJson(value) =
                crate::routes::gateway_json::GatewayJson::<VideoRequest>::from_request(
                    request, state,
                )
                .await?;
            return Ok(Self(value));
        }
        let mut fields = serde_json::Map::new();
        let mut form = Multipart::from_request(request, state)
            .await
            .map_err(IntoResponse::into_response)?;
        while let Some(mut field) = form.next_field().await.map_err(|_| {
            ApiErrorResponse::bad_request("Invalid video multipart body").into_response()
        })? {
            let name = field.name().unwrap_or("").to_owned();
            if !matches!(
                name.as_str(),
                "model" | "prompt" | "seconds" | "size" | "audio"
            ) || field.file_name().is_some()
                || fields.contains_key(&name)
            {
                return Err(ApiErrorResponse::bad_request("Video multipart supports unique text fields only; reference assets have no enabled mapping").into_response());
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = field.chunk().await.map_err(|_| {
                ApiErrorResponse::bad_request("Invalid video form data").into_response()
            })? {
                if chunk.len() > (32 * 1024usize).saturating_sub(bytes.len()) {
                    return Err(
                        ApiErrorResponse::bad_request("Video form field exceeds 32 KiB")
                            .into_response(),
                    );
                }
                bytes.extend_from_slice(&chunk);
            }
            let text = String::from_utf8(bytes).map_err(|_| {
                ApiErrorResponse::bad_request("Video fields require UTF-8").into_response()
            })?;
            let value = if name == "audio" {
                serde_json::Value::Bool(text.parse::<bool>().map_err(|_| {
                    ApiErrorResponse::bad_request("Invalid video audio flag").into_response()
                })?)
            } else {
                serde_json::Value::String(text)
            };
            fields.insert(name, value);
        }
        serde_json::from_value(serde_json::Value::Object(fields))
            .map(Self)
            .map_err(|_| {
                ApiErrorResponse::bad_request(
                    "Video form requires model, prompt and explicit seconds",
                )
                .into_response()
            })
    }
}
