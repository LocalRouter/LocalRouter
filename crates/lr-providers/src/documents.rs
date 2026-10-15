//! Bounded inline document input. File IDs never bypass local ownership.
use base64::Engine;
use lr_types::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const OPTIONS_EXTENSION: &str = "localrouter.documents";
pub const METADATA_EXTENSION: &str = "localrouter.document_processing";
pub const PROVENANCE_EXTENSION: &str = "_localrouter_document_provenance";
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_FILES: usize = 4;
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FileInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub file_data: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentProtocol {
    OpenRouterNative,
    LlmGatewayGoogle,
    Anthropic,
    Gemini,
    Openai,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DocumentMode {
    #[default]
    Native,
    Text,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DocumentOptions {
    pub mode: DocumentMode,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentOrigin {
    pub message_index: usize,
    pub part_index: usize,
    pub filename: Option<String>,
    pub mime: String,
    pub bytes: usize,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentContext {
    pub mode: DocumentMode,
    pub files: Vec<DocumentOrigin>,
}
impl DocumentOrigin {
    pub fn from_file(
        file: &FileInput,
        decoded: &DecodedDocument,
        message_index: usize,
        part_index: usize,
    ) -> Self {
        use sha2::{Digest, Sha256};
        Self {
            message_index,
            part_index,
            filename: file.filename.clone(),
            mime: decoded.mime.clone(),
            bytes: decoded.bytes.len(),
            sha256: format!("{:x}", Sha256::digest(&decoded.bytes)),
        }
    }
}
impl DocumentContext {
    pub fn from_request(request: &crate::CompletionRequest) -> AppResult<Self> {
        request.validate_files()?;
        let mut files = Vec::new();
        for (message_index, message) in request.messages.iter().enumerate() {
            if let crate::ChatMessageContent::Parts(parts) = &message.content {
                for (part_index, part) in parts.iter().enumerate() {
                    if let crate::ContentPart::File { file } = part {
                        let decoded = file.decode()?;
                        files.push(DocumentOrigin::from_file(
                            file,
                            &decoded,
                            message_index,
                            part_index,
                        ));
                    }
                }
            }
        }
        Ok(Self {
            mode: DocumentMode::Native,
            files,
        })
    }
    pub fn metadata(&self) -> serde_json::Value {
        serde_json::json!({"mode":self.mode,"files":self.files,"page_count":null,"input_usage_reported":self.mode==DocumentMode::Text,"scan_coverage":if self.files.iter().any(|file|file.mime=="application/pdf") {"text_and_metadata_only"} else {"decoded_text"}})
    }
}
pub struct DecodedDocument {
    pub mime: String,
    pub bytes: Vec<u8>,
}
fn invalid(message: &str) -> AppError {
    AppError::InvalidParams(message.into())
}
impl FileInput {
    pub fn decode(&self) -> AppResult<DecodedDocument> {
        if self.filename.as_ref().is_some_and(|name| {
            name.is_empty() || name.len() > 256 || name.chars().any(char::is_control)
        }) {
            return Err(invalid(
                "Document filename must be nonempty, at most 256 bytes and contain no controls",
            ));
        }
        if self.file_data.len() > MAX_FILE_BYTES.div_ceil(3) * 4 + 128 {
            return Err(invalid("Each inline document is limited to 4 MiB"));
        }
        let data=self.file_data.strip_prefix("data:").ok_or_else(||invalid("Documents require an inline base64 data URL; unowned file IDs and remote references are unsupported"))?;
        let (mime, encoded) = data
            .split_once(";base64,")
            .ok_or_else(|| invalid("Document data must use data:<mime>;base64,<data>"))?;
        if !matches!(
            mime,
            "application/pdf"
                | "text/plain"
                | "text/markdown"
                | "text/csv"
                | "text/html"
                | "text/css"
                | "text/javascript"
                | "text/xml"
        ) {
            return Err(invalid("Unsupported document MIME type"));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| invalid("Invalid document base64 data"))?;
        if bytes.is_empty() || bytes.len() > MAX_FILE_BYTES {
            return Err(invalid("Each inline document requires 1 byte to 4 MiB"));
        }
        if mime == "application/pdf" {
            if !bytes.starts_with(b"%PDF-") {
                return Err(invalid("PDF document data has no PDF header"));
            }
        } else if bytes.len() > 512 * 1024 || std::str::from_utf8(&bytes).is_err() {
            return Err(invalid("Text documents require UTF-8 data up to 512 KiB"));
        }
        Ok(DecodedDocument {
            mime: mime.into(),
            bytes,
        })
    }
}
/// Validate the native mapping even for direct provider callers.
pub fn native_input(
    provider: &dyn crate::ModelProvider,
    request: &crate::CompletionRequest,
) -> AppResult<bool> {
    if !request.has_file_inputs() {
        return Ok(false);
    }
    request.validate_files()?;
    let protocol = provider
        .document_protocol(request)
        .ok_or_else(|| invalid("Provider/model has no verified native document mapping"))?;
    if protocol == DocumentProtocol::OpenRouterNative {
        for message in &request.messages {
            if let crate::ChatMessageContent::Parts(parts) = &message.content {
                for part in parts {
                    if let crate::ContentPart::File { file } = part {
                        if file.decode()?.mime != "application/pdf" {
                            return Err(invalid("OpenRouter native document mode currently accepts PDF only; use local text mode for text files"));
                        }
                    }
                }
            }
        }
    }
    Ok(true)
}

impl crate::CompletionRequest {
    pub fn has_document_context(&self) -> bool {
        self.documents.is_some() || self.has_file_inputs()
    }
    pub fn has_file_inputs(&self) -> bool {
        self.messages.iter().any(|message|matches!(&message.content,crate::ChatMessageContent::Parts(parts) if parts.iter().any(|part|matches!(part,crate::ContentPart::File {..}))))
    }
    pub fn validate_files(&self) -> AppResult<()> {
        let mut count = 0;
        let mut total = 0;
        for message in &self.messages {
            if let crate::ChatMessageContent::Parts(parts) = &message.content {
                for part in parts {
                    if let crate::ContentPart::File { file } = part {
                        if message.role != "user" {
                            return Err(invalid(
                                "Document file parts are supported only in user messages",
                            ));
                        }
                        let decoded = file.decode()?;
                        count += 1;
                        total += decoded.bytes.len();
                        if count > MAX_FILES || total > MAX_TOTAL_BYTES {
                            return Err(invalid("A document request permits at most four files and 8 MiB total decoded data"));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
/// Without native input usage, document costs cannot be inferred from text alone.
pub fn input_usage_unknown(
    extensions: Option<&std::collections::HashMap<String, serde_json::Value>>,
) -> bool {
    extensions
        .and_then(|e| e.get(METADATA_EXTENSION))
        .is_some_and(|v| {
            v.get("mode").and_then(serde_json::Value::as_str) == Some("native")
                && v.get("input_usage_reported")
                    .and_then(serde_json::Value::as_bool)
                    != Some(true)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(mime: &str, bytes: &[u8]) -> FileInput {
        FileInput {
            filename: Some("source.pdf".into()),
            file_data: format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ),
        }
    }
    #[test]
    fn document_bounds_and_types_are_checked_without_fetching_or_guessing_content() {
        assert_eq!(
            file("application/pdf", b"%PDF-1.7\nfixture")
                .decode()
                .unwrap()
                .mime,
            "application/pdf"
        );
        assert!(file("application/pdf", b"not a PDF").decode().is_err());
        assert!(file("text/plain", &[0xff]).decode().is_err());
        assert!(file("text/plain", &vec![b'x'; 512 * 1024 + 1])
            .decode()
            .is_err());
        assert!(file("application/pdf", &vec![b'x'; MAX_FILE_BYTES + 1])
            .decode()
            .is_err());
        assert!(
            serde_json::from_value::<FileInput>(serde_json::json!({"file_id":"unowned"})).is_err()
        );
        let mut reference = file("text/plain", b"text");
        reference.file_data = "https://example.com/source.pdf".into();
        assert!(reference.decode().is_err());
        let mut invalid_name = file("text/plain", b"text");
        invalid_name.filename = Some("name\nwith controls".into());
        assert!(invalid_name.decode().is_err());
    }
}
