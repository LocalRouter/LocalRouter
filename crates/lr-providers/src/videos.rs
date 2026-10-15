//! Native asynchronous text-to-video transports. No retries, redirects or URL media fetches.
use lr_types::{AppError, AppResult};
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

pub const MAX_MEDIA_BYTES: u64 = 128 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoProtocol {
    Openai,
    LlmGateway,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum VideoSeconds {
    Number(u32),
    Text(String),
}
fn seconds<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    match VideoSeconds::deserialize(d)? {
        VideoSeconds::Number(n) => Ok(n),
        VideoSeconds::Text(s) => s.parse().map_err(serde::de::Error::custom),
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VideoRequest {
    pub model: String,
    pub prompt: String,
    #[serde(deserialize_with = "seconds")]
    #[schema(value_type=VideoSeconds)]
    pub seconds: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<bool>,
}
fn invalid(message: &str) -> AppError {
    AppError::InvalidParams(message.into())
}
impl VideoRequest {
    pub fn normalize(&mut self) -> AppResult<()> {
        if self.model.is_empty()
            || self.model.len() > 512
            || self.model.chars().any(char::is_whitespace)
            || matches!(self.model.as_str(), "auto" | "localrouter/auto")
        {
            return Err(invalid("Video requires an explicit provider/model"));
        }
        if self.prompt.trim().is_empty() || self.prompt.len() > 16 * 1024 {
            return Err(invalid("Video prompt requires 1 byte to 16 KiB"));
        }
        if !(4..=30).contains(&self.seconds) {
            return Err(invalid(
                "Video duration must be explicitly selected between 4 and 30 seconds",
            ));
        }
        if self.size.is_none() {
            self.size = Some("1280x720".into());
        }
        if !matches!(self.size.as_deref(), Some("1280x720" | "720x1280")) {
            return Err(invalid(
                "Initial video support is limited to 1280x720 or 720x1280",
            ));
        }
        Ok(())
    }
    pub fn fingerprint(&self) -> AppResult<String> {
        Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(self)?)))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct VideoError {
    pub code: String,
    pub message: String,
}
#[derive(Clone, Debug)]
pub struct NativeVideo {
    pub id: String,
    pub status: String,
    pub progress: u8,
    pub completed_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub cost_usd: Option<f64>,
}
impl NativeVideo {
    pub fn parse(value: Value) -> AppResult<Self> {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| valid_native_id(id))
            .ok_or_else(|| AppError::Provider("Invalid native video ID".into()))?
            .to_owned();
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Provider("Native video status is missing".into()))?;
        let status = if status == "cancelled" {
            "canceled"
        } else {
            status
        };
        if !matches!(
            status,
            "queued" | "in_progress" | "completed" | "failed" | "canceled" | "expired"
        ) {
            return Err(AppError::Provider(
                "Unknown native video status; the create is not retried".into(),
            ));
        }
        let progress = match value.get("progress").filter(|v| !v.is_null()) {
            Some(v) => v
                .as_u64()
                .filter(|n| *n <= 100)
                .ok_or_else(|| AppError::Provider("Invalid native video progress".into()))?
                as u8,
            None => 0,
        };
        let timestamp = |key: &str| -> AppResult<Option<i64>> {
            value
                .get(key)
                .filter(|v| !v.is_null())
                .map(|v| {
                    v.as_i64()
                        .filter(|n| (0..=253402300799).contains(n))
                        .ok_or_else(|| AppError::Provider("Invalid native video timestamp".into()))
                })
                .transpose()
        };
        let cost = value
            .pointer("/usage/cost")
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_f64()
                    .filter(|n| n.is_finite() && (0.0..=1e9).contains(n))
                    .ok_or_else(|| AppError::Provider("Invalid native video charge".into()))
            })
            .transpose()?;
        Ok(Self {
            id,
            status: status.into(),
            progress,
            completed_at: timestamp("completed_at")?,
            expires_at: timestamp("expires_at")?,
            cost_usd: cost,
        })
    }
}
pub fn valid_native_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
fn valid_range(range: &str) -> bool {
    if range.len() > 128 {
        return false;
    }
    let Some((start, end)) = range.strip_prefix("bytes=").and_then(|v| v.split_once('-')) else {
        return false;
    };
    if start.is_empty() {
        return end.parse::<u64>().is_ok_and(|n| n > 0);
    }
    let Ok(start) = start.parse::<u64>() else {
        return false;
    };
    end.is_empty() || end.parse::<u64>().is_ok_and(|end| end >= start)
}
/// Holds credentials only in memory; intentionally has no Debug/Serialize implementation.
pub struct VideoTransport {
    base_url: String,
    headers: HeaderMap,
    pub protocol: VideoProtocol,
}
impl VideoTransport {
    pub fn new(base_url: &str, headers: HeaderMap, protocol: VideoProtocol) -> AppResult<Self> {
        let url =
            reqwest::Url::parse(base_url).map_err(|_| invalid("Invalid video API base URL"))?;
        if !matches!(url.scheme(), "https" | "http")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid(
                "Video base URL must not contain credentials, query or fragment",
            ));
        }
        if url.scheme() == "http"
            && !url.host_str().is_some_and(|host| {
                host == "localhost"
                    || host
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            })
        {
            return Err(invalid("Remote video providers require HTTPS"));
        }
        Ok(Self {
            base_url: base_url.trim_end_matches('/').into(),
            headers,
            protocol,
        })
    }
    pub fn binding(&self, salt: &str) -> String {
        let mut headers: Vec<_> = self
            .headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_bytes()))
            .collect();
        headers.sort_by(|a, b| a.0.cmp(b.0).then_with(|| a.1.cmp(b.1)));
        let mut hash = Sha256::new();
        hash.update(salt);
        hash.update([0]);
        hash.update(self.base_url.as_bytes());
        hash.update([0]);
        hash.update(
            match self.protocol {
                VideoProtocol::Openai => "openai",
                VideoProtocol::LlmGateway => "llmgateway",
            }
            .as_bytes(),
        );
        for (name, value) in headers {
            hash.update([0]);
            hash.update(name.as_bytes());
            hash.update([0]);
            hash.update(value);
        }
        format!("{:x}", hash.finalize())
    }
    pub fn validate(&self, request: &VideoRequest) -> AppResult<()> {
        let mut request = request.clone();
        request.normalize()?;
        if self.protocol == VideoProtocol::Openai
            && (!matches!(request.model.as_str(), "sora-2" | "sora-2-pro")
                || !matches!(request.seconds, 4 | 8 | 12)
                || request.audio.is_some())
        {
            return Err(invalid("Direct OpenAI video requires sora-2/sora-2-pro, 4/8/12 seconds and no unverified audio option"));
        }
        Ok(())
    }
    fn builder(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> AppResult<reqwest_middleware::RequestBuilder> {
        let mut request = crate::http_client::native_client()?
            .request(method, format!("{}{path}", self.base_url))
            .headers(self.headers.clone());
        if self.protocol == VideoProtocol::LlmGateway {
            request = request.header("X-No-Fallback", "true");
        }
        Ok(request)
    }
    async fn json(response: reqwest::Response) -> AppResult<Value> {
        if !response.status().is_success() {
            let status = response.status().as_u16();
            // Do not echo provider diagnostics: they may contain prompts or signed URLs.
            return Err(AppError::ProviderStatus {
                status,
                message: format!("Native video API returned HTTP {status}; no automatic retry"),
            });
        }
        crate::messages::read_json_response(response, 1024 * 1024).await
    }
    pub async fn create(&self, request: &VideoRequest, local_id: &str) -> AppResult<NativeVideo> {
        let mut normalized = request.clone();
        normalized.normalize()?;
        let request = &normalized;
        self.validate(request)?;
        let mut builder = self
            .builder(reqwest::Method::POST, "/videos")?
            .header("Idempotency-Key", local_id);
        builder = match self.protocol {
            VideoProtocol::Openai => builder.multipart(
                reqwest::multipart::Form::new()
                    .text("model", request.model.clone())
                    .text("prompt", request.prompt.clone())
                    .text("seconds", request.seconds.to_string())
                    .text(
                        "size",
                        request.size.clone().unwrap_or_else(|| "1280x720".into()),
                    ),
            ),
            VideoProtocol::LlmGateway => {
                let mut body = json!({"model":request.model,"prompt":request.prompt,"seconds":request.seconds,"size":request.size});
                if let Some(audio) = request.audio {
                    body["audio"] = json!(audio);
                }
                builder.json(&body)
            }
        };
        let response=builder.send().await.map_err(|_|AppError::Provider("Video create outcome is unknown; use the same Idempotency-Key to inspect the recorded intent, not to resubmit".into()))?;
        NativeVideo::parse(Self::json(response).await?)
    }
    pub async fn retrieve(&self, id: &str) -> AppResult<NativeVideo> {
        if !valid_native_id(id) {
            return Err(invalid("Invalid bound native video ID"));
        }
        let response = self
            .builder(reqwest::Method::GET, &format!("/videos/{id}"))?
            .send()
            .await
            .map_err(|_| AppError::Provider("Video status request failed".into()))?;
        let native = NativeVideo::parse(Self::json(response).await?)?;
        if native.id != id {
            return Err(AppError::Provider(
                "Video status response did not match the bound job".into(),
            ));
        }
        Ok(native)
    }
    pub async fn content(&self, id: &str, range: Option<&str>) -> AppResult<reqwest::Response> {
        if !valid_native_id(id) {
            return Err(invalid("Invalid bound native video ID"));
        }
        let mut request = self.builder(reqwest::Method::GET, &format!("/videos/{id}/content"))?;
        if let Some(range) = range {
            if !valid_range(range) {
                return Err(invalid("Only a valid single byte range is supported"));
            }
            request = request.header("Range", range);
        }
        let response = request
            .send()
            .await
            .map_err(|_| AppError::Provider("Video content request failed".into()))?;
        if !matches!(response.status().as_u16(), 200 | 206) {
            return Err(AppError::ProviderStatus {
                status: response.status().as_u16(),
                message: "Native video content is unavailable".into(),
            });
        }
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        if mime != "video/mp4"
            || response
                .headers()
                .get(reqwest::header::CONTENT_ENCODING)
                .is_some_and(|v| v != "identity")
            || response
                .content_length()
                .is_some_and(|size| size > MAX_MEDIA_BYTES)
        {
            return Err(AppError::Provider(
                "Video content must be MP4 up to 128 MiB".into(),
            ));
        }
        Ok(response)
    }
    pub async fn delete(&self, id: &str) -> AppResult<()> {
        if self.protocol != VideoProtocol::Openai {
            return Err(invalid(
                "Provider has no verified video asset-delete mapping",
            ));
        }
        if !valid_native_id(id) {
            return Err(invalid("Invalid bound native video ID"));
        }
        let response = self
            .builder(reqwest::Method::DELETE, &format!("/videos/{id}"))?
            .send()
            .await
            .map_err(|_| {
                AppError::Provider(
                    "Video deletion outcome is unknown; it is not automatically retried".into(),
                )
            })?;
        let body = Self::json(response).await?;
        if body.get("id").and_then(Value::as_str) != Some(id)
            || body.get("deleted").and_then(Value::as_bool) != Some(true)
        {
            return Err(AppError::Provider(
                "Invalid native video deletion confirmation".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> VideoRequest {
        serde_json::from_value(
            json!({"model":"sora-2","prompt":"A locally provided test prompt","seconds":"4"}),
        )
        .unwrap()
    }
    fn transport(base: &str, protocol: VideoProtocol) -> VideoTransport {
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            "Bearer fixture".parse().unwrap(),
        );
        VideoTransport::new(base, headers, protocol).unwrap()
    }
    #[test]
    fn native_request_bounds_and_fingerprints_preserve_semantic_duration_and_reject_unowned_assets()
    {
        let mut a = request();
        a.normalize().unwrap();
        let mut b:VideoRequest=serde_json::from_value(json!({"model":"sora-2","prompt":"A locally provided test prompt","seconds":4,"size":"1280x720"})).unwrap();
        b.normalize().unwrap();
        assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
        for seconds in [0, 3, 31, u32::MAX] {
            let mut r = a.clone();
            r.seconds = seconds;
            assert!(r.normalize().is_err());
        }
        let mut r = a.clone();
        r.prompt = "x".repeat(16 * 1024 + 1);
        assert!(r.normalize().is_err());
        let mut r = a.clone();
        r.size = Some("3840x2160".into());
        assert!(r.normalize().is_err());
        let mut raw = serde_json::to_value(&a).unwrap();
        raw["input_reference"] = json!({"file_id":"unowned"});
        assert!(serde_json::from_value::<VideoRequest>(raw).is_err());
        assert!(VideoTransport::new(
            "https://user:pass@example.com/v1",
            HeaderMap::new(),
            VideoProtocol::Openai
        )
        .is_err());
        assert!(VideoTransport::new(
            "http://example.com/v1",
            HeaderMap::new(),
            VideoProtocol::Openai
        )
        .is_err());
        for range in ["bytes=0-1", "bytes=12-", "bytes=-64"] {
            assert!(valid_range(range));
        }
        for range in [
            "bytes=-",
            "bytes=2-1",
            "bytes=0-1,2-3",
            "bytes=--",
            "garbage",
        ] {
            assert!(!valid_range(range));
        }
    }
    #[tokio::test]
    async fn direct_video_create_is_native_multipart_and_status_is_bound_without_echoing_provider_payloads(
    ) {
        use wiremock::{
            matchers::{body_string_contains, header, method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/videos")).and(header("authorization","Bearer fixture")).and(header("idempotency-key","video_lr_fixture")).and(body_string_contains("name=\"prompt\"")).and(body_string_contains("A locally provided test prompt")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"video_native","status":"queued","progress":0,"prompt":"must not persist","video_url":"https://example.com/signed?secret=x"}))).expect(1).mount(&server).await;
        let result = transport(&server.uri(), VideoProtocol::Openai)
            .create(&request(), "video_lr_fixture")
            .await
            .unwrap();
        assert_eq!(result.id, "video_native");
        assert!(result.cost_usd.is_none());
        server.verify().await;
    }
    #[tokio::test]
    async fn video_redirects_never_receive_credentials_and_status_ids_are_checked() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        let target = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&target)
            .await;
        Mock::given(method("POST"))
            .and(path("/videos"))
            .respond_with(ResponseTemplate::new(307).insert_header("location", target.uri()))
            .expect(1)
            .mount(&server)
            .await;
        assert!(transport(&server.uri(), VideoProtocol::Openai)
            .create(&request(), "video_lr_fixture")
            .await
            .is_err());
        target.verify().await;
        Mock::given(method("GET"))
            .and(path("/videos/video_bound"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id":"video_other","status":"completed"})),
            )
            .mount(&server)
            .await;
        assert!(transport(&server.uri(), VideoProtocol::Openai)
            .retrieve("video_bound")
            .await
            .is_err());
    }
    #[test]
    fn native_status_and_usage_do_not_infer_zero_fees_from_failed_or_canceled_jobs() {
        let failed = NativeVideo::parse(json!({"id":"v_1","status":"failed"})).unwrap();
        assert!(failed.cost_usd.is_none());
        let canceled =
            NativeVideo::parse(json!({"id":"v_1","status":"canceled","usage":{"cost":0}})).unwrap();
        assert_eq!(canceled.cost_usd, Some(0.0));
        for body in [
            json!({"id":"../../foreign","status":"queued"}),
            json!({"id":"v_1","status":"made_up"}),
            json!({"id":"v_1","status":"completed","usage":{"cost":-1}}),
            json!({"id":"v_1","status":"queued","progress":101}),
        ] {
            assert!(NativeVideo::parse(body).is_err());
        }
    }
    #[tokio::test]
    async fn native_video_content_bounds_and_asset_delete_confirmation_are_enforced() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        for (id, response) in [
            (
                "wrong_type",
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html")
                    .set_body_string("not a video"),
            ),
            (
                "encoded",
                ResponseTemplate::new(200)
                    .insert_header("content-type", "video/mp4")
                    .insert_header("content-encoding", "br")
                    .set_body_bytes(vec![0; 4]),
            ),
        ] {
            Mock::given(method("GET"))
                .and(path(format!("/videos/{id}/content")))
                .respond_with(response)
                .mount(&server)
                .await;
            assert!(transport(&server.uri(), VideoProtocol::Openai)
                .content(id, None)
                .await
                .is_err());
        }
        // wiremock/hyper derives Content-Length from its fixture body, so use a
        // raw local response to test rejection before allocating a huge body.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let oversized = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",MAX_MEDIA_BYTES+1).as_bytes()).await.unwrap();
        });
        let result = transport(&format!("http://{addr}"), VideoProtocol::Openai)
            .content("too_large", None)
            .await;
        assert!(
            result.is_err(),
            "Oversized native media must be rejected before downloading"
        );
        oversized.await.unwrap();
        Mock::given(method("DELETE"))
            .and(path("/videos/video_bound"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id":"video_other","deleted":true})),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert!(transport(&server.uri(), VideoProtocol::Openai)
            .delete("video_bound")
            .await
            .is_err());
        assert!(transport(&server.uri(), VideoProtocol::LlmGateway)
            .delete("video_bound")
            .await
            .is_err());
        server.verify().await;
    }
}
