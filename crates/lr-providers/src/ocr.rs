//! Explicit page-bounded OCR. Inline bytes only; never a hidden document parser.
use base64::Engine;
use lr_types::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

pub const METADATA: &str = "localrouter.ocr";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrProtocol {
    Mistral,
    LlmGateway,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum OcrDocument {
    DocumentUrl { document_url: String },
    ImageUrl { image_url: String },
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum OcrPages {
    Indices(Vec<u32>),
    Range(String),
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OcrRequest {
    pub model: String,
    pub document: OcrDocument,
    pub pages: OcrPages,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_image_base64: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_limit: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct OcrPage {
    pub index: u32,
    pub markdown: String,
    #[serde(default)]
    #[schema(value_type=Vec<Object>)]
    pub images: Vec<Value>,
    #[serde(default)]
    #[schema(value_type=Option<Object>)]
    pub dimensions: Option<Value>,
    #[serde(flatten)]
    #[schema(ignore)]
    pub extra: std::collections::HashMap<String, Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct OcrUsage {
    pub pages_processed: u32,
    pub doc_size_bytes: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct OcrResponse {
    pub model: String,
    pub pages: Vec<OcrPage>,
    pub usage_info: OcrUsage,
    #[serde(default)]
    pub document_annotation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type=Option<Object>)]
    pub extensions: Option<std::collections::HashMap<String, Value>>,
    #[serde(flatten)]
    #[schema(ignore)]
    pub extra: std::collections::HashMap<String, Value>,
}
fn invalid(message: &str) -> AppError {
    AppError::InvalidParams(message.into())
}
impl OcrPages {
    pub fn indices(&self) -> AppResult<Vec<u32>> {
        let indices = match self {
            Self::Indices(values) => {
                if values.len() > 20 {
                    return Err(invalid("OCR permits at most 20 pages"));
                }
                values.clone()
            }
            Self::Range(text) => {
                if text.len() > 128 {
                    return Err(invalid("OCR page range exceeds 128 bytes"));
                }
                let (start, end) = text.split_once('-').ok_or_else(|| {
                    invalid(
                        "Use an explicit OCR page list or inclusive zero-index range such as 0-4",
                    )
                })?;
                let start = start
                    .parse::<u32>()
                    .map_err(|_| invalid("Invalid OCR range start"))?;
                let end = end
                    .parse::<u32>()
                    .map_err(|_| invalid("Invalid OCR range end"))?;
                if start > end || end - start >= 20 {
                    return Err(invalid("OCR range must contain 1–20 ascending pages"));
                }
                (start..=end).collect()
            }
        };
        let mut unique = std::collections::HashSet::new();
        if indices.is_empty()
            || indices.len() > 20
            || indices
                .iter()
                .any(|index| *index > 9999 || !unique.insert(*index))
        {
            return Err(invalid(
                "OCR requires 1–20 unique zero-index pages between 0 and 9999",
            ));
        }
        Ok(indices)
    }
}
impl OcrRequest {
    pub fn source(&self) -> AppResult<crate::documents::DocumentOrigin> {
        let (url, image) = match &self.document {
            OcrDocument::DocumentUrl { document_url } => (document_url, false),
            OcrDocument::ImageUrl { image_url } => (image_url, true),
        };
        if !image {
            let file = crate::documents::FileInput {
                filename: self.filename.clone(),
                file_data: url.clone(),
            };
            let decoded = file.decode()?;
            if decoded.mime != "application/pdf" {
                return Err(invalid("OCR document_url requires an inline PDF"));
            }
            return Ok(crate::documents::DocumentOrigin::from_file(
                &file, &decoded, 0, 0,
            ));
        }
        if url.len() > crate::documents::MAX_FILE_BYTES.div_ceil(3) * 4 + 128 {
            return Err(invalid("OCR image exceeds 4 MiB"));
        }
        let (mime,data)=url.strip_prefix("data:").and_then(|v|v.split_once(";base64,")).ok_or_else(||invalid("OCR requires bounded inline PDF/PNG/JPEG data; remote URLs and file IDs are unsupported"))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| invalid("Invalid OCR image base64"))?;
        if bytes.is_empty()
            || bytes.len() > crate::documents::MAX_FILE_BYTES
            || !match mime {
                "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
                "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
                _ => false,
            }
        {
            return Err(invalid(
                "OCR image requires valid PNG/JPEG data up to 4 MiB",
            ));
        }
        if self
            .filename
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 256 || s.chars().any(char::is_control))
        {
            return Err(invalid("Invalid OCR filename"));
        }
        let file = crate::documents::FileInput {
            filename: self.filename.clone(),
            file_data: String::new(),
        };
        Ok(crate::documents::DocumentOrigin::from_file(
            &file,
            &crate::documents::DecodedDocument {
                mime: mime.into(),
                bytes,
            },
            0,
            0,
        ))
    }
    pub fn validate(&self) -> AppResult<()> {
        if self.model.is_empty()
            || self.model.len() > 512
            || self.model.chars().any(char::is_whitespace)
        {
            return Err(invalid("OCR requires an explicit model up to 512 bytes"));
        }
        self.pages.indices()?;
        self.source()?;
        if matches!(self.document, OcrDocument::ImageUrl { .. }) && self.pages.indices()? != [0] {
            return Err(invalid("Image OCR requires pages:[0]"));
        }
        if self.image_limit.is_some_and(|n| n > 8) {
            return Err(invalid("OCR image_limit must be between 0 and 8"));
        }
        Ok(())
    }
    pub fn wire_body(&self, protocol: OcrProtocol) -> AppResult<Value> {
        self.validate()?;
        let supported = match protocol {
            OcrProtocol::LlmGateway => self.model == "mistral-ocr-latest",
            OcrProtocol::Mistral => {
                self.model.starts_with("mistral-ocr-")
                    && self
                        .model
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            }
        };
        if !supported {
            return Err(invalid("OCR model has no verified native mapping"));
        }
        Ok(
            json!({"model":self.model,"document":self.document,"pages":self.pages.indices()?,"include_image_base64":self.include_image_base64.unwrap_or(false),"image_limit":self.image_limit.unwrap_or(0)}),
        )
    }
    pub async fn read_response(&self, response: reqwest::Response) -> AppResult<OcrResponse> {
        if !response.status().is_success() {
            return Err(crate::messages::response_error(response).await);
        }
        let value = crate::messages::read_json_response(response, 8 * 1024 * 1024).await?;
        let mut result: OcrResponse = serde_json::from_value(value).map_err(|_| {
            AppError::Provider(
                "Invalid OCR response; remote work may already have been billed".into(),
            )
        })?;
        let indices = self.pages.indices()?;
        let mut seen = std::collections::HashSet::new();
        if (result.usage_info.pages_processed as usize) < result.pages.len()
            || result.pages.len() > indices.len()
            || result.usage_info.pages_processed as usize > indices.len()
            || result.pages.iter().any(|page| {
                !indices.contains(&page.index)
                    || !seen.insert(page.index)
                    || page.markdown.len() > 512 * 1024
            })
        {
            return Err(AppError::Provider("OCR response exceeded requested page/size bounds; charges may have occurred and the dispatch is not retried".into()));
        }
        let source = self.source()?;
        result.extensions=Some([(METADATA.into(),json!({"source": { "filename":source.filename,"mime":source.mime,"bytes":source.bytes,"sha256":source.sha256 },"requested_pages":indices,"pages_processed":result.usage_info.pages_processed,"billing_unit":"page","cost_usd":null,"cost_source":"unknown","content_trust":"untrusted"}))].into_iter().collect());
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> OcrRequest {
        serde_json::from_value(json!({"model":"mistral-ocr-latest","document":{"type":"document_url","document_url":"data:application/pdf;base64,JVBERi0xLjc="},"pages":[0]})).unwrap()
    }
    #[test]
    fn explicit_page_selection_is_bounded_before_native_dispatch() {
        for pages in [
            json!([]),
            json!([0, 0]),
            json!([10000]),
            json!("1-0"),
            json!("0-20"),
            json!("0-4294967295"),
            json!([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20]),
        ] {
            assert!(serde_json::from_value::<OcrPages>(pages)
                .unwrap()
                .indices()
                .is_err());
        }
        assert_eq!(
            OcrPages::Range("0-4".into()).indices().unwrap(),
            [0, 1, 2, 3, 4]
        );
        let mut body = serde_json::to_value(request()).unwrap();
        body.as_object_mut().unwrap().remove("pages");
        assert!(serde_json::from_value::<OcrRequest>(body).is_err());
    }
    #[test]
    fn remote_urls_unknown_fields_and_mismatched_native_models_are_not_guessed() {
        let mut r = request();
        r.document = OcrDocument::DocumentUrl {
            document_url: "https://example.com/private.pdf".into(),
        };
        assert!(r.validate().is_err());
        let mut r = request();
        r.model = "auto".into();
        assert!(r.wire_body(OcrProtocol::LlmGateway).is_err());
        let mut body = serde_json::to_value(request()).unwrap();
        body["plugins"] = json!([]);
        assert!(serde_json::from_value::<OcrRequest>(body).is_err());
        let r = request();
        let wire = r.wire_body(OcrProtocol::LlmGateway).unwrap();
        assert_eq!(wire["pages"], json!([0]));
        assert!(wire.get("filename").is_none());
        assert_eq!(wire["include_image_base64"], false);
    }
}

#[cfg(test)]
mod response_tests {
    use super::*;
    fn request() -> OcrRequest {
        serde_json::from_value(json!({"model":"mistral-ocr-latest","document":{"type":"document_url","document_url":"data:application/pdf;base64,JVBERi0xLjc="},"filename":"source.pdf","pages":[0]})).unwrap()
    }
    #[tokio::test]
    async fn requested_page_indices_and_native_usage_bound_response_provenance() {
        let server = wiremock::MockServer::start().await;
        for body in [
            json!({"model":"mistral-ocr-latest","pages":[{"index":1,"markdown":"unrequested"}],"usage_info":{"pages_processed":1,"doc_size_bytes":null}}),
            json!({"model":"mistral-ocr-latest","pages":[{"index":0,"markdown":"source"}],"usage_info":{"pages_processed":2,"doc_size_bytes":null}}),
            json!({"model":"mistral-ocr-latest","pages":[{"index":0,"markdown":"source"},{"index":0,"markdown":"duplicate"}],"usage_info":{"pages_processed":1,"doc_size_bytes":null}}),
            json!({"model":"mistral-ocr-latest","pages":[{"index":0,"markdown":"missing usage"}]}),
        ] {
            let _scope = wiremock::Mock::given(wiremock::matchers::method("GET"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
                .mount_as_scoped(&server)
                .await;
            let response = reqwest::get(server.uri()).await.unwrap();
            assert!(request().read_response(response).await.is_err());
        }
        let _scope=wiremock::Mock::given(wiremock::matchers::method("GET")).respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"model":"mistral-ocr-latest","pages":[{"index":0,"markdown":"![untrusted](https://example.com/image.png)","images":[],"dimensions":{"dpi":200}}],"usage_info":{"pages_processed":1,"doc_size_bytes":8},"extensions":{"localrouter.ocr":{"cost_usd":0}}}))).mount_as_scoped(&server).await;
        let result = request()
            .read_response(reqwest::get(server.uri()).await.unwrap())
            .await
            .unwrap();
        assert_eq!(result.pages[0].index, 0);
        assert_eq!(result.pages[0].dimensions.as_ref().unwrap()["dpi"], 200);
        assert_eq!(
            result.extensions.as_ref().unwrap()[METADATA]["source"]["filename"],
            "source.pdf"
        );
        assert!(result.extensions.as_ref().unwrap()[METADATA]["cost_usd"].is_null());
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use crate::ModelProvider;
    #[tokio::test]
    async fn direct_mistral_ocr_uses_native_pages_and_never_follows_a_redirect() {
        use wiremock::{
            matchers::{body_partial_json, header, method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let native = MockServer::start().await;
        let target = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&target)
            .await;
        Mock::given(method("POST"))
            .and(path("/ocr"))
            .and(header("authorization", "Bearer fixture"))
            .and(body_partial_json(
                json!({"model":"mistral-ocr-latest","pages":[0]}),
            ))
            .respond_with(ResponseTemplate::new(307).insert_header("location", target.uri()))
            .expect(1)
            .mount(&native)
            .await;
        let provider =
            crate::mistral::MistralProvider::with_base_url("fixture".into(), Some(native.uri()))
                .unwrap();
        let request:OcrRequest=serde_json::from_value(json!({"model":"mistral-ocr-latest","document":{"type":"document_url","document_url":"data:application/pdf;base64,JVBERi0xLjc="},"pages":[0]})).unwrap();
        assert!(provider.ocr(request).await.is_err());
        native.verify().await;
        target.verify().await;
    }
}

#[cfg(test)]
mod image_and_size_tests {
    use super::*;
    #[test]
    fn image_source_headers_pages_and_size_are_checked_without_executing_a_decoder() {
        let mut request:OcrRequest=serde_json::from_value(json!({"model":"mistral-ocr-latest","document":{"type":"image_url","image_url":"data:image/png;base64,iVBORw0KGgo="},"pages":[0]})).unwrap();
        assert_eq!(request.source().unwrap().mime, "image/png");
        assert!(request.validate().is_ok());
        request.pages = OcrPages::Indices(vec![1]);
        assert!(request.validate().is_err());
        request.pages = OcrPages::Indices(vec![0]);
        request.document = OcrDocument::ImageUrl {
            image_url: "data:image/jpeg;base64,iVBORw0KGgo=".into(),
        };
        assert!(request.validate().is_err());
        request.document = OcrDocument::ImageUrl {
            image_url: "data:image/png;base64,invalid!".into(),
        };
        assert!(request.validate().is_err());
    }
    #[tokio::test]
    async fn ocr_response_body_is_bounded_before_json_deserialization() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_string("x".repeat(8 * 1024 * 1024 + 1)),
            )
            .mount(&server)
            .await;
        let request:OcrRequest=serde_json::from_value(json!({"model":"mistral-ocr-latest","document":{"type":"document_url","document_url":"data:application/pdf;base64,JVBERi0xLjc="},"pages":[0]})).unwrap();
        let error = request
            .read_response(reqwest::get(server.uri()).await.unwrap())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("exceeds"));
    }
}
