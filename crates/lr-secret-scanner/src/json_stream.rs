//! Incremental JSON string adapter. Syntax is forwarded as it arrives; only
//! string contents enter the bounded reference recognizer. JSON escapes are
//! decoded at this layer and reapplied to replacements at the same layer.
use crate::placeholders::TextStream;
use lr_types::{secret_context::SecretContext, AppResult};
use std::{collections::VecDeque, sync::Arc};

#[derive(Clone, Default)]
pub(crate) struct JsonStream {
    in_string: bool,
    escape: String,
    text: TextStream,
    containers: Vec<(bool, bool, bool)>,
    key: String,
    is_key: bool,
    skip: bool,
    pub preserve_opaque: bool,
    original: VecDeque<(String, String)>,
    passthrough: bool,
    pub complete: bool,
    pub location: &'static str,
    part: Option<Part>,
}
#[derive(Clone, Default)]
struct Part {
    raw: String,
    depth: usize,
    quoted: bool,
    escaped: bool,
    passthrough: bool,
}
impl JsonStream {
    pub(crate) fn buffered_bytes(&self) -> usize {
        self.escape.len()
            + self.part.as_ref().map_or(0, |part| part.raw.len())
            + self.text.buffered_bytes()
            + self
                .original
                .iter()
                .map(|(_, raw)| raw.len())
                .sum::<usize>()
    }
    pub(crate) fn for_response() -> Self {
        Self {
            preserve_opaque: true,
            ..Default::default()
        }
    }
    pub(crate) fn for_arguments() -> Self {
        Self {
            location: "Tool arguments",
            ..Default::default()
        }
    }
    pub(crate) fn abandon_raw(&mut self) -> String {
        let mut output: String = self.original.drain(..).map(|(_, raw)| raw).collect();
        output.push_str(&std::mem::take(&mut self.escape));
        if let Some(part) = self.part.take() {
            output.push_str(&part.raw);
        }
        self.text = TextStream::default();
        output
    }
    pub(crate) fn push(
        &mut self,
        input: &str,
        finish: bool,
        context: &Arc<dyn SecretContext>,
    ) -> AppResult<String> {
        if self.passthrough {
            return Ok(input.into());
        }
        let checkpoint = self.clone();
        match self.push_inner(input, finish, context) {
            Ok(output) => Ok(output),
            Err(error) => {
                *self = checkpoint;
                Err(error)
            }
        }
    }
    fn push_inner(
        &mut self,
        input: &str,
        finish: bool,
        context: &Arc<dyn SecretContext>,
    ) -> AppResult<String> {
        let mut output = String::new();
        let mut at = 0;
        while at < input.len() {
            if self.part.is_none() && self.in_string && !self.skip && self.escape.is_empty() {
                let plain = input[at..].find(['"', '\\']).unwrap_or(input.len() - at);
                if plain > 0 {
                    let raw = &input[at..at + plain];
                    self.emit(raw, raw, false, context, &mut output)?;
                    at += plain;
                    continue;
                }
            }
            let ch = input[at..].chars().next().expect("remaining char");
            at += ch.len_utf8();
            if let Some(part) = self.part.as_mut() {
                if part.passthrough {
                    output.push(ch);
                } else {
                    part.raw.push(ch);
                }
                if part.quoted {
                    if part.escaped {
                        part.escaped = false;
                    } else if ch == '\\' {
                        part.escaped = true;
                    } else if ch == '"' {
                        part.quoted = false;
                    }
                } else {
                    match ch {
                        '"' => part.quoted = true,
                        '{' | '[' => part.depth += 1,
                        '}' | ']' => part.depth = part.depth.saturating_sub(1),
                        _ => {}
                    }
                }
                if part.raw.len() > 64 * 1024 || (!part.passthrough && part.depth > 128) {
                    output.push_str(&std::mem::take(&mut part.raw));
                    part.passthrough = true;
                    context.diagnostic("Provider content block exceeds signed-content lookahead; block delivered unchanged");
                }
                if part.depth == 0 {
                    let part = self.part.take().expect("part exists");
                    if !part.passthrough {
                        let signed = serde_json::from_str::<serde_json::Value>(&part.raw)
                            .ok()
                            .is_some_and(|value| {
                                ["signature", "thought_signature"].iter().any(|key| {
                                    value
                                        .get(key)
                                        .and_then(serde_json::Value::as_str)
                                        .is_some_and(|v| !v.is_empty())
                                })
                            });
                        if signed {
                            output.push_str(&part.raw);
                        } else {
                            let mut nested = JsonStream::for_response();
                            output.push_str(&nested.push(&part.raw, true, context)?);
                        }
                    }
                }
                continue;
            }
            if self.preserve_opaque
                && !self.in_string
                && ch == '{'
                && self.containers.last().is_some_and(|(_, _, parts)| *parts)
            {
                self.part = Some(Part {
                    raw: "{".into(),
                    depth: 1,
                    ..Default::default()
                });
                continue;
            }
            if !self.in_string {
                output.push(ch);
                if matches!(ch, '{' | '[') && self.containers.len() >= 256 {
                    self.passthrough = true;
                    context.diagnostic("JSON nesting exceeds rolling adapter limit; remaining content delivered unchanged");
                    output.push_str(&input[at..]);
                    return Ok(output);
                }
                match ch {
                    '{' => self.containers.push((true, true, false)),
                    '[' => self.containers.push((
                        false,
                        false,
                        matches!(self.key.as_str(), "content" | "parts"),
                    )),
                    '}' | ']' => {
                        self.containers.pop();
                        if self.containers.is_empty() {
                            self.complete = true;
                        }
                    }
                    ',' => {
                        if let Some((object, key, _)) = self.containers.last_mut() {
                            *key = *object;
                        }
                    }
                    ':' => {
                        if let Some((_, key, _)) = self.containers.last_mut() {
                            *key = false;
                        }
                    }
                    '"' => {
                        self.in_string = true;
                        self.is_key = self
                            .containers
                            .last()
                            .is_some_and(|(object, key, _)| *object && *key);
                        self.skip = self.preserve_opaque
                            && !self.is_key
                            && matches!(
                                self.key.as_str(),
                                "signature"
                                    | "thought_signature"
                                    | "encrypted_content"
                                    | "encrypted_output"
                            );
                        if self.is_key {
                            self.key.clear();
                        }
                    }
                    _ => {}
                }
                continue;
            }
            if self.skip {
                output.push(ch);
                if !self.escape.is_empty() {
                    self.escape.clear();
                } else if ch == '\\' {
                    self.escape.push(ch);
                } else if ch == '"' {
                    self.in_string = false;
                }
                continue;
            }
            if !self.escape.is_empty() || ch == '\\' {
                self.escape.push(ch);
                if self.escape.len() == 1 {
                    continue;
                }
                let wrapped = format!("\"{}\"", self.escape);
                match serde_json::from_str::<String>(&wrapped) {
                    Ok(decoded) => {
                        let raw = std::mem::take(&mut self.escape);
                        self.emit(&decoded, &raw, false, context, &mut output)?;
                    }
                    Err(_) if self.escape.len() < 12 => {}
                    Err(_) => {
                        // Invalid syntax remains the endpoint's concern. Do not
                        // drop bytes or turn adapter trouble into a failed call.
                        self.emit("", "", true, context, &mut output)?;
                        output.push_str(&std::mem::take(&mut self.escape));
                    }
                }
                continue;
            }
            if ch == '"' {
                self.emit("", "", true, context, &mut output)?;
                output.push(ch);
                self.in_string = false;
                if self.containers.is_empty() {
                    self.complete = true;
                }
            } else {
                let mut buf = [0; 4];
                let decoded = ch.encode_utf8(&mut buf);
                self.emit(decoded, decoded, false, context, &mut output)?;
            }
        }
        if finish {
            if let Some(part) = self.part.take() {
                output.push_str(&part.raw);
            }
            self.emit("", "", true, context, &mut output)?;
            output.push_str(&std::mem::take(&mut self.escape));
        }
        Ok(output)
    }
    fn emit(
        &mut self,
        text: &str,
        raw: &str,
        finish: bool,
        context: &Arc<dyn SecretContext>,
        output: &mut String,
    ) -> AppResult<()> {
        if self.is_key && self.key.len() < 256 {
            self.key.extend(text.chars().take(256 - self.key.len()));
        }
        if !text.is_empty() {
            self.original.push_back((text.into(), raw.into()));
        }
        let before = self.text.buffered_bytes() + text.len();
        let location = if !self.location.is_empty() {
            self.location
        } else if matches!(self.key.as_str(), "arguments" | "partial_json") {
            "Tool arguments"
        } else {
            "JSON string"
        };
        let restored = self.text.push(text, finish, |s| {
            context.restore_text_at(s, false, location)
        })?;
        if self.text.take_limited() {
            context.diagnostic(
                "Quoted reference exceeds rolling restoration window; original fragment delivered",
            );
        }
        let consumed = before.saturating_sub(self.text.buffered_bytes());
        let mut decoded = String::new();
        let mut original = String::new();
        while decoded.len() < consumed {
            if let Some((text, raw)) = self.original.pop_front() {
                let remaining = consumed - decoded.len();
                if text.len() > remaining && text == raw {
                    decoded.push_str(&text[..remaining]);
                    original.push_str(&raw[..remaining]);
                    self.original
                        .push_front((text[remaining..].into(), raw[remaining..].into()));
                } else {
                    decoded.push_str(&text);
                    original.push_str(&raw);
                }
            } else {
                break;
            }
        }
        if restored == decoded {
            output.push_str(&original);
        } else if !restored.is_empty() {
            let encoded = serde_json::to_string(&restored)?;
            output.push_str(&encoded[1..encoded.len() - 1]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placeholders::Vault;
    use lr_types::secret_context::SecretContext;
    use serde_json::json;
    fn fixture() -> (Arc<dyn SecretContext>, String, String) {
        let vault = Arc::new(Vault::for_client(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "json-stream-test",
        ));
        let secret = "original-\"quote\"-\\-\n-🔑".to_string();
        vault.register(&secret).unwrap();
        let mut value = json!({"content":secret});
        vault.protect(&mut value).unwrap();
        (vault, value["content"].as_str().unwrap().into(), secret)
    }
    #[test]
    fn unicode_surrogates_and_nested_strings_survive_every_fragment_size() {
        let (vault, tag, secret) = fixture();
        let raw = format!(
            r#"{{"\u006bey":"{}","unicode":"\ud83d\udd11","nested":{{"thinking":"{}"}}}}"#,
            tag, tag
        );
        for size in 1..30 {
            let mut stream = JsonStream::default();
            let mut output = String::new();
            for chunk in raw.as_bytes().chunks(size) {
                output.push_str(
                    &stream
                        .push(std::str::from_utf8(chunk).unwrap(), false, &vault)
                        .unwrap(),
                );
            }
            output.push_str(&stream.push("", true, &vault).unwrap());
            let value: serde_json::Value = serde_json::from_str(&output).unwrap();
            assert_eq!(value["key"], secret);
            assert_eq!(value["unicode"], "🔑");
            assert_eq!(value["nested"]["thinking"], secret);
        }
    }
    #[test]
    fn opaque_response_fields_keep_their_exact_bytes() {
        let (vault, tag, secret) = fixture();
        let raw = json!({"content":tag,"thinking":tag,"signature":format!("{}\\u0041",tag),"encrypted_content":tag}).to_string();
        let mut stream = JsonStream {
            preserve_opaque: true,
            ..Default::default()
        };
        let result = stream.push(&raw, true, &vault).unwrap();
        let actual: serde_json::Value = serde_json::from_str(&result).unwrap();
        let original: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(actual["content"], secret);
        assert_eq!(actual["thinking"], secret);
        for key in ["signature", "encrypted_content"] {
            assert_eq!(actual[key], original[key]);
        }
    }
    #[test]
    fn large_structured_text_is_emitted_with_bounded_retention() {
        let (vault, tag, secret) = fixture();
        let mut stream = JsonStream::default();
        let mut output = stream.push("{\"content\":\"", false, &vault).unwrap();
        for _ in 0..4096 {
            let part = stream.push(&"x".repeat(1024), false, &vault).unwrap();
            assert_eq!(part.len(), 1024);
            assert!(stream.buffered_bytes() < 1024);
            output.push_str(&part);
        }
        output.push_str(&stream.push(&format!("{tag}\"}}"), true, &vault).unwrap());
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert!(value["content"].as_str().unwrap().ends_with(&secret));
    }
}

#[cfg(test)]
mod signed_part_tests {
    use super::*;
    use lr_types::secret_context::SecretContext;
    use serde_json::json;
    #[test]
    fn signed_provider_parts_are_preserved_for_every_property_order_and_split() {
        let vault = Arc::new(crate::placeholders::Vault::for_client(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "signed-stream",
        ));
        vault.register("synthetic-signed-stream-original").unwrap();
        let mut value = json!({"content":"synthetic-signed-stream-original"});
        vault.protect(&mut value).unwrap();
        let tag = value["content"].as_str().unwrap();
        let context: Arc<dyn SecretContext> = vault;
        for key in ["signature", "thought_signature"] {
            for fields in [
                format!("\"text\":{},\"{key}\":\"signature-bytes\"", json!(tag)),
                format!("\"{key}\":\"signature-bytes\",\"text\":{}", json!(tag)),
            ] {
                let raw = format!("{{\"content\":[{{{fields}}},{{\"text\":{}}}]}}", json!(tag));
                for size in [1, 7, 500] {
                    let mut stream = JsonStream::for_response();
                    let mut output = String::new();
                    for fragment in raw.as_bytes().chunks(size) {
                        output.push_str(
                            &stream
                                .push(std::str::from_utf8(fragment).unwrap(), false, &context)
                                .unwrap(),
                        );
                    }
                    output.push_str(&stream.push("", true, &context).unwrap());
                    let actual: serde_json::Value = serde_json::from_str(&output).unwrap();
                    assert_eq!(actual["content"][0]["text"], tag);
                    assert_eq!(
                        actual["content"][1]["text"],
                        "synthetic-signed-stream-original"
                    );
                }
            }
        }
    }
}
