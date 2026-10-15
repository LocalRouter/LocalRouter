//! Public file input normalization. No document URL or file ID is dereferenced.
use lr_providers::documents::{
    DocumentContext, DocumentMode, DocumentOptions, DocumentOrigin, FileInput, MAX_FILES,
    MAX_TOTAL_BYTES, OPTIONS_EXTENSION, PROVENANCE_EXTENSION,
};
use lr_types::{AppError, AppResult};
use serde_json::{json, Value};
fn invalid(message: &str) -> AppError {
    AppError::InvalidParams(message.into())
}
pub fn normalize_ingress(value: &mut Value, path: &str) -> AppResult<()> {
    if value
        .get("extensions")
        .and_then(Value::as_object)
        .is_some_and(|extensions| extensions.contains_key(PROVENANCE_EXTENSION))
    {
        return Err(invalid(
            "Document provenance is private and cannot be supplied by callers",
        ));
    }
    let options = value
        .get("extensions")
        .and_then(|extensions| extensions.get(OPTIONS_EXTENSION))
        .map(|options| {
            serde_json::from_value::<DocumentOptions>(options.clone())
                .map_err(|_| invalid("Invalid document options"))
        })
        .transpose()?
        .unwrap_or_default();
    let mut files = Vec::new();
    let mut total = 0;
    let response_path = matches!(path.strip_prefix("/v1").unwrap_or(path), "/responses");
    let messages_key = if response_path { "input" } else { "messages" };
    if let Some(messages) = value.get_mut(messages_key).and_then(Value::as_array_mut) {
        for (message_index, message) in messages.iter_mut().enumerate() {
            if message
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| matches!(kind, "file" | "input_file"))
            {
                return Err(invalid(
                    "File parts must be inside a user message content array",
                ));
            }
            let message_type = message
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let has_role = message.get("role").and_then(Value::as_str).is_some();
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .to_owned();
            if let Some(parts) = message.get_mut("content").and_then(Value::as_array_mut) {
                for (part_index, part) in parts.iter_mut().enumerate() {
                    let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
                    if !matches!(kind, "file" | "input_file") {
                        continue;
                    }
                    if !matches!(
                        path.strip_prefix("/v1").unwrap_or(path),
                        "/chat/completions" | "/responses"
                    ) {
                        return Err(invalid(
                            "File parts currently support Chat Completions and Responses only",
                        ));
                    }
                    if kind != if response_path { "input_file" } else { "file" } {
                        return Err(invalid("File content type does not match this endpoint"));
                    }
                    if response_path
                        && (message_type
                            .as_deref()
                            .is_some_and(|kind| kind != "message")
                            || (!has_role && message_type.is_none()))
                    {
                        return Err(invalid(
                            "Responses files require a message item or an explicit user role",
                        ));
                    }
                    if !has_role || role != "user" {
                        return Err(invalid(
                            "Document file parts are supported only in user messages",
                        ));
                    }
                    if files.len() >= MAX_FILES {
                        return Err(invalid("A document request permits at most four files"));
                    }
                    let allowed: &[&str] = if response_path {
                        &["type", "filename", "file_data"]
                    } else {
                        &["type", "file"]
                    };
                    if part.as_object().is_none_or(|fields| {
                        fields.keys().any(|key| !allowed.contains(&key.as_str()))
                    }) {
                        return Err(invalid("Unsupported file fields; unowned file IDs and remote references have no enabled mapping"));
                    }
                    let raw = if response_path {
                        json!({"filename":part.get("filename"),"file_data":part.get("file_data")})
                    } else {
                        part.get("file")
                            .cloned()
                            .ok_or_else(|| invalid("File content requires a file object"))?
                    };
                    let file: FileInput = serde_json::from_value(raw).map_err(|_| {
                        invalid(
                            "Invalid inline file input; provide file_data and optional filename",
                        )
                    })?;
                    let decoded = file.decode()?;
                    total += decoded.bytes.len();
                    if total > MAX_TOTAL_BYTES {
                        return Err(invalid("Document inputs exceed 8 MiB total decoded data"));
                    }
                    let origin =
                        DocumentOrigin::from_file(&file, &decoded, message_index, part_index);
                    let hash = origin.sha256.clone();
                    files.push(origin);
                    if options.mode == DocumentMode::Text {
                        if decoded.mime == "application/pdf" {
                            return Err(invalid("Local text mode accepts extracted UTF-8 text; extract PDFs through a configured local MCP tool first, or use explicit OCR"));
                        }
                        let text = String::from_utf8(decoded.bytes)
                            .map_err(|_| invalid("Text document is not UTF-8"))?;
                        let text=format!("[Attached document: {}; MIME: {}; SHA-256: {}]\n{}\n[End attached document]",file.filename.as_deref().unwrap_or("unnamed"),decoded.mime,hash,text);
                        *part = if response_path {
                            json!({"type":"input_text","text":text})
                        } else {
                            json!({"type":"text","text":text})
                        };
                    }
                }
            }
        }
    }
    if files.is_empty() {
        if value
            .get("extensions")
            .and_then(|extensions| extensions.get(OPTIONS_EXTENSION))
            .is_some()
        {
            return Err(invalid(
                "Document processing options require supported file input parts",
            ));
        }
        return Ok(());
    }
    if value.get("plugins").is_some() {
        return Err(invalid("Raw document parser plugins are unsupported; native file mode never implicitly chooses paid OCR"));
    }
    if options.mode == DocumentMode::Native && response_path {
        if value
            .get("previous_response_id")
            .is_some_and(|v| !v.is_null())
            || value.get("store").and_then(Value::as_bool) == Some(true)
        {
            return Err(invalid("Native documents require store:false and no previous_response_id; reuse extracted text for continuation"));
        }
        value["store"] = json!(false);
    }
    let extensions = value
        .as_object_mut()
        .ok_or_else(|| invalid("Request must be an object"))?
        .entry("extensions")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| invalid("Document extensions must be an object"))?;
    extensions.remove(OPTIONS_EXTENSION);
    extensions.insert(
        PROVENANCE_EXTENSION.into(),
        serde_json::to_value(DocumentContext {
            mode: options.mode,
            files,
        })?,
    );
    Ok(())
}
pub fn context(
    extensions: Option<&std::collections::HashMap<String, Value>>,
) -> AppResult<Option<Box<DocumentContext>>> {
    extensions
        .and_then(|extensions| extensions.get(PROVENANCE_EXTENSION))
        .map(|context| {
            serde_json::from_value::<DocumentContext>(context.clone())
                .map(Box::new)
                .map_err(|_| invalid("Invalid protected document provenance"))
        })
        .transpose()
}
pub fn public_representation(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(extensions) = value.get_mut("extensions").and_then(Value::as_object_mut) {
        extensions.remove(PROVENANCE_EXTENSION);
    }
    value
}

/// Scan actual decoded text while preserving the original native wire payload.
/// PDFs remain opaque: callers must enforce configured blocking scan policies.
pub fn scan_projection(value: &Value) -> AppResult<(Value, bool)> {
    let mut projected = value.clone();
    let mut opaque = false;
    if let Some(messages) = projected.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages {
            if let Some(parts) = message.get_mut("content").and_then(Value::as_array_mut) {
                for part in parts {
                    if part.get("type").and_then(Value::as_str) != Some("file") {
                        continue;
                    }
                    let file: FileInput =
                        serde_json::from_value(part.get("file").cloned().unwrap_or(Value::Null))?;
                    let decoded = file.decode()?;
                    if decoded.mime == "application/pdf" {
                        opaque = true;
                        *part = json!({"type":"text","text":format!("[Opaque PDF attachment: {}]",file.filename.as_deref().unwrap_or("unnamed"))});
                    } else {
                        *part = json!({"type":"text","text":String::from_utf8(decoded.bytes).map_err(|_|invalid("Invalid document UTF-8"))?});
                    }
                }
            }
        }
    }
    Ok((projected, opaque))
}

pub fn has_native_files(request: &crate::types::ChatCompletionRequest) -> bool {
    request.messages.iter().any(|m|matches!(&m.content,Some(crate::types::MessageContent::Parts(parts)) if parts.iter().any(|p|matches!(p,crate::types::ContentPart::File{..}))))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(kind: &str) -> Value {
        if kind == "input_file" {
            json!({"type":kind,"filename":"notes.txt","file_data":"data:text/plain;base64,c2VjcmV0"})
        } else {
            json!({"type":kind,"file":{"filename":"notes.txt","file_data":"data:text/plain;base64,c2VjcmV0"}})
        }
    }
    #[test]
    fn local_text_preserves_actual_content_and_provenance_on_both_endpoints() {
        for (path, key, kind) in [
            ("/v1/chat/completions", "messages", "file"),
            ("/responses", "input", "input_file"),
        ] {
            let mut body = json!({key:[{"role":"user","content":[input(kind)]}],"extensions":{OPTIONS_EXTENSION:{"mode":"text"}}});
            normalize_ingress(&mut body, path).unwrap();
            let part = &body[key][0]["content"][0];
            assert!(part["text"].as_str().unwrap().contains("\nsecret\n"));
            let metadata = &body["extensions"][PROVENANCE_EXTENSION];
            assert_eq!(metadata["files"][0]["bytes"], 6);
            assert_eq!(metadata["files"][0]["sha256"].as_str().unwrap().len(), 64);
            assert!(public_representation(&body)["extensions"]
                .get(PROVENANCE_EXTENSION)
                .is_none());
            assert!(body["extensions"].get(OPTIONS_EXTENSION).is_none());
        }
    }
    #[test]
    fn opaque_and_text_scan_projections_do_not_mistake_base64_for_inspected_text() {
        let mut body = json!({"messages":[{"role":"user","content":[input("file"),{"type":"file","file":{"file_data":"data:application/pdf;base64,JVBERi0xLjc="}}]}]});
        normalize_ingress(&mut body, "/chat/completions").unwrap();
        let (projected, opaque) = scan_projection(&body).unwrap();
        assert!(opaque);
        assert_eq!(projected["messages"][0]["content"][0]["text"], "secret");
        assert!(projected["messages"][0]["content"][1]["text"]
            .as_str()
            .unwrap()
            .contains("Opaque PDF"));
        assert!(body["messages"][0]["content"][0]["file"].is_object());
    }
    #[test]
    fn invalid_role_ids_continuation_plugins_and_private_metadata_reject_before_dispatch() {
        for mut body in [
            json!({"messages":[{"role":"assistant","content":[input("file")]}]}),
            json!({"messages":[{"role":"user","content":[{"type":"file","file":{"file_id":"unowned"}}]}]}),
            json!({"messages":[{"role":"user","content":[input("file")]}],"plugins":[]}),
            json!({"extensions":{PROVENANCE_EXTENSION:{"mode":"text","files":[]}}}),
            json!({"messages":[{"role":"user","content":[input("input_file")]}]}),
        ] {
            assert!(normalize_ingress(&mut body, "/chat/completions").is_err());
        }
        let mut body = json!({"input":[{"role":"user","content":[input("input_file")]}],"previous_response_id":"old"});
        assert!(normalize_ingress(&mut body, "/responses").is_err());
        body.as_object_mut().unwrap().remove("previous_response_id");
        normalize_ingress(&mut body, "/responses").unwrap();
        assert_eq!(body["store"], false);
    }
    #[test]
    fn responses_document_items_cannot_be_dropped_as_unknown_item_types() {
        for message in [
            json!({"content":[input("input_file")]}),
            json!({"type":"function_call","content":[input("input_file")]}),
            json!({"type":"message","content":[input("input_file")]}),
        ] {
            let mut body = json!({"input":[message]});
            assert!(normalize_ingress(&mut body, "/responses").is_err());
        }
    }
    #[test]
    fn aggregate_decoded_byte_limit_applies_even_below_the_file_count_limit() {
        use base64::Engine;
        let mut bytes = vec![b'x'; 3 * 1024 * 1024];
        bytes[..5].copy_from_slice(b"%PDF-");
        let file = json!({"type":"file","file":{"file_data":format!("data:application/pdf;base64,{}",base64::engine::general_purpose::STANDARD.encode(bytes))}});
        let mut body = json!({"messages":[{"role":"user","content":vec![file;3]}]});
        assert!(normalize_ingress(&mut body, "/chat/completions")
            .unwrap_err()
            .to_string()
            .contains("8 MiB"));
    }
    #[test]
    fn four_file_and_total_limits_apply_before_allocation_of_more_files() {
        let mut body = json!({"messages":[{"role":"user","content":vec![input("file");5]}]});
        assert!(normalize_ingress(&mut body, "/chat/completions").is_err());
        let mut body = json!({"messages":[{"role":"user","content":[{"type":"file","file":{"file_data":"data:application/pdf;base64,JVBERi0xLjc="}}]}],"extensions":{OPTIONS_EXTENSION:{"mode":"text"}}});
        assert!(normalize_ingress(&mut body, "/chat/completions").is_err());
    }
}
