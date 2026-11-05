//! Response restoration shared by the gateway and native HTTP/WebSocket proxy.
use crate::placeholders::{TextStream, Vault};
use lr_types::secret_context::SecretContext;
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
pub const LIMIT: usize = 16 * 1024 * 1024;
pub fn frame_end(bytes: &[u8]) -> Option<(usize, usize)> {
    let lf = bytes.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}
/// Structured JSON must be parsed before restoring values and keys; textual
/// channels remain incremental. JSON buffering is bounded by the body budget.
#[derive(Default)]
struct ContentStream {
    text: TextStream,
    pending: String,
    structured: Option<bool>,
}
impl ContentStream {
    fn push(
        &mut self,
        text: &str,
        finish: bool,
        context: &Arc<dyn SecretContext>,
    ) -> lr_types::AppResult<String> {
        if self.structured == Some(false) {
            return self
                .text
                .push(text, finish, |s| context.restore_text(s, false));
        }
        self.pending.push_str(text);
        if self.pending.len() > LIMIT {
            return Err(lr_types::AppError::InvalidParams(
                "Protected content exceeds limit".into(),
            ));
        }
        if self.structured.is_none() {
            if let Some(ch) = self.pending.chars().find(|ch| !ch.is_whitespace()) {
                self.structured = Some(context.structured_output() || matches!(ch, '{' | '['));
            }
        }
        if self.structured == Some(false) {
            return self
                .text
                .push(&std::mem::take(&mut self.pending), finish, |s| {
                    context.restore_text(s, false)
                });
        }
        if !finish {
            return Ok(String::new());
        }
        let mut content = Value::String(std::mem::take(&mut self.pending));
        // Use the content adapter so JSON-encoded keys and values are restored.
        let mut wrapped = serde_json::json!({"content":content});
        context.restore(&mut wrapped, false)?;
        content = wrapped["content"].take();
        let text = content.as_str().unwrap_or_default();
        context.validate_output(text)?;
        Ok(text.to_owned())
    }
}
#[derive(Default)]
pub struct Events {
    pub done: bool,
    text: BTreeMap<String, (ContentStream, Value, String)>,
    arguments: BTreeMap<String, (String, Value, String)>,
    tool_names: BTreeMap<String, String>,
    template_cost: BTreeMap<String, usize>,
}
fn encode(value: &Value, event: &str) -> String {
    format!(
        "{}data: {}\n\n",
        if event.is_empty() {
            String::new()
        } else {
            format!("event: {event}\n")
        },
        value
    )
}
impl Events {
    pub fn frame(
        &mut self,
        frame: &str,
        context: &Arc<dyn SecretContext>,
    ) -> lr_types::AppResult<Vec<String>> {
        if frame.len() > LIMIT {
            return Err(lr_types::AppError::InvalidParams(
                "Protected response frame exceeds limit".into(),
            ));
        }
        let data = frame
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        let event = frame
            .lines()
            .find_map(|line| line.strip_prefix("event:").map(str::trim))
            .unwrap_or("");
        if data == "[DONE]" {
            self.done = true;
            let mut out = self.finish(context)?;
            out.push("data: [DONE]\n\n".into());
            return Ok(out);
        }
        if data.is_empty() {
            return Ok(vec![format!("{frame}\n\n")]);
        }
        let mut value: Value = serde_json::from_str(&data)
            .map_err(|_| lr_types::AppError::Internal("Invalid protected SSE event".into()))?;
        self.record_tools(&value);
        self.check_budget()?;
        if value.get("done").is_some() {
            if let Some(calls) = value
                .pointer_mut("/message/tool_calls")
                .and_then(Value::as_array_mut)
            {
                for call in calls {
                    if let Some(function) = call.get_mut("function") {
                        if let Some(arguments) = function.get_mut("arguments") {
                            context.restore(arguments, true)?;
                        }
                        if let Some(arguments) = function.get("arguments") {
                            context.validate_tool(
                                function.get("name").and_then(Value::as_str),
                                arguments,
                            )?;
                        }
                    }
                }
            }
        }
        let kind = value["type"].as_str().unwrap_or("").to_owned();
        let mut prefixes = Vec::new();
        let all = matches!(
            kind.as_str(),
            "response.completed"
                | "response.failed"
                | "response.incomplete"
                | "message_stop"
                | "error"
        ) || value["done"] == true;
        if all {
            self.done = true;
        }
        if kind == "content_block_stop" {
            prefixes.push(format!("messages:{}:", value["index"]));
        }
        if kind.ends_with(".done") {
            let identity = value
                .get("item_id")
                .or_else(|| value.pointer("/item/id"))
                .unwrap_or(&Value::Null);
            prefixes.push(format!("responses:{identity}:"));
        }
        if let Some(choices) = value["choices"].as_array() {
            for choice in choices {
                if !choice["finish_reason"].is_null() {
                    prefixes.push(format!("chat:{}:", choice["index"]));
                }
            }
        }
        let mut paths = Vec::new();
        collect_deltas(&value, "", &mut paths);
        for (path, arguments) in paths {
            let arguments = arguments
                || value["type"].as_str().is_some_and(|kind| {
                    (kind.contains("arguments") || kind.contains("custom_tool_call_input"))
                        && kind.ends_with(".delta")
                });
            let Some(text) = value
                .pointer(&path)
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                continue;
            };
            // Include wire identities, not just array position, for interleaved calls.
            let channel = channel_key(&value, &path);
            let mut template = value.clone();
            clear_delta_strings(&mut template);
            if !self.text.contains_key(&channel) && !self.arguments.contains_key(&channel) {
                self.template_cost
                    .insert(channel.clone(), template.to_string().len());
            }
            if arguments {
                let entry = self
                    .arguments
                    .entry(channel)
                    .or_insert_with(|| (String::new(), template, path.clone()));
                entry.0.push_str(&text);
                if entry.0.len() > LIMIT {
                    return Err(lr_types::AppError::Internal(
                        "Protected tool arguments exceed limit".into(),
                    ));
                }
                *value.pointer_mut(&path).unwrap() = Value::String(String::new());
            } else {
                let entry = self
                    .text
                    .entry(channel)
                    .or_insert_with(|| (ContentStream::default(), template, path.clone()));
                let restored = entry.0.push(&text, false, context)?;
                *value.pointer_mut(&path).unwrap() = Value::String(restored);
            }
            self.check_budget()?;
        }
        self.check_budget()?;
        let mut out = self.flush(context, |channel| {
            all || prefixes.iter().any(|prefix| channel.starts_with(prefix))
        })?;
        // Deltas are handled incrementally above; full snapshots need a fresh pass.
        if !is_delta(&value) {
            context.restore(&mut value, false)?;
            if all || kind.ends_with(".done") {
                context.validate_response(&value)?;
            }
        }
        let metadata = frame
            .lines()
            .filter(|line| {
                line.starts_with("id:") || line.starts_with("retry:") || line.starts_with(':')
            })
            .map(|line| format!("{line}\n"))
            .collect::<String>();
        out.push(format!("{metadata}{}", encode(&value, event)));
        Ok(out)
    }
    fn check_budget(&self) -> lr_types::AppResult<()> {
        let buffers = self
            .text
            .values()
            .map(|(stream, _, _)| stream.pending.len() + stream.text.buffered_bytes())
            .sum::<usize>()
            .saturating_add(
                self.arguments
                    .values()
                    .map(|(text, _, _)| text.len())
                    .sum::<usize>(),
            );
        let metadata = self
            .template_cost
            .iter()
            .map(|(key, cost)| key.len() + cost)
            .sum::<usize>()
            .saturating_add(
                self.tool_names
                    .iter()
                    .map(|(key, name)| key.len() + name.len())
                    .sum::<usize>(),
            );
        if self.text.len() + self.arguments.len() > 1024
            || self.tool_names.len() > 1024
            || buffers.saturating_add(metadata) > LIMIT
        {
            return Err(lr_types::AppError::InvalidParams(
                "Protected response exceeds channel or buffer budget".into(),
            ));
        }
        Ok(())
    }
    fn record_tools(&mut self, value: &Value) {
        if let Some(choices) = value["choices"].as_array() {
            for choice in choices {
                if let Some(calls) = choice
                    .pointer("/delta/tool_calls")
                    .and_then(Value::as_array)
                {
                    for call in calls {
                        if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                            self.tool_names.insert(
                                format!("chat:{}:{}:", choice["index"], call["index"]),
                                name.into(),
                            );
                        }
                    }
                }
                if let Some(name) = choice
                    .pointer("/delta/function_call/name")
                    .and_then(Value::as_str)
                {
                    self.tool_names
                        .insert(format!("chat:{}:null:", choice["index"]), name.into());
                }
            }
        }
        if let (Some(id), Some(name)) = (
            value.pointer("/item/id"),
            value.pointer("/item/name").and_then(Value::as_str),
        ) {
            self.tool_names
                .insert(format!("responses:{id}:"), name.into());
        }
        if let Some(name) = value.pointer("/content_block/name").and_then(Value::as_str) {
            self.tool_names
                .insert(format!("messages:{}:", value["index"]), name.into());
        }
    }
    pub fn finish(&mut self, context: &Arc<dyn SecretContext>) -> lr_types::AppResult<Vec<String>> {
        self.flush(context, |_| true)
    }
    fn flush(
        &mut self,
        context: &Arc<dyn SecretContext>,
        selected: impl Fn(&str) -> bool,
    ) -> lr_types::AppResult<Vec<String>> {
        let mut out = Vec::new();
        let keys: Vec<_> = self
            .text
            .keys()
            .filter(|key| selected(key))
            .cloned()
            .collect();
        for key in keys {
            self.template_cost.remove(&key);
            let (mut stream, mut template, path) =
                self.text.remove(&key).expect("selected channel");
            let text = stream.push("", true, context)?;
            if !text.is_empty() {
                *template.pointer_mut(&path).unwrap() = Value::String(text);
                out.push(encode(&template, template["type"].as_str().unwrap_or("")));
            }
        }
        let keys: Vec<_> = self
            .arguments
            .keys()
            .filter(|key| selected(key))
            .cloned()
            .collect();
        for key in keys {
            self.template_cost.remove(&key);
            let (arguments, mut template, path) = self
                .arguments
                .remove(&key)
                .expect("selected argument channel");
            let restored = if key.contains("custom_tool_call_input") {
                context.restore_text(&arguments, true)?
            } else {
                let mut args: Value = serde_json::from_str(&arguments).map_err(|_| {
                    lr_types::AppError::InvalidParams("Incomplete protected tool arguments".into())
                })?;
                context.restore(&mut args, true)?;
                let name = self
                    .tool_names
                    .iter()
                    .find(|(prefix, _)| key.starts_with(prefix.as_str()))
                    .map(|(_, name)| name.as_str());
                context.validate_tool(name, &args)?;
                serde_json::to_string(&args).map_err(|_| {
                    lr_types::AppError::Internal("Could not restore tool arguments".into())
                })?
            };
            *template.pointer_mut(&path).unwrap() = Value::String(restored);
            out.push(encode(&template, template["type"].as_str().unwrap_or("")));
        }
        Ok(out)
    }
}
fn channel_key(value: &Value, path: &str) -> String {
    if path.starts_with("/choices/") {
        let parts: Vec<_> = path.split('/').collect();
        let choice = value
            .pointer(&format!("/choices/{}/index", parts[2]))
            .unwrap_or(&Value::Null);
        let call = parts
            .iter()
            .position(|s| *s == "tool_calls")
            .and_then(|i| parts.get(i + 1))
            .and_then(|i| {
                value.pointer(&format!("/choices/{}/delta/tool_calls/{i}/index", parts[2]))
            })
            .unwrap_or(&Value::Null);
        let field = parts.last().unwrap_or(&"");
        return format!("chat:{choice}:{call}:{field}");
    }
    if value["type"]
        .as_str()
        .is_some_and(|s| s.starts_with("content_block"))
    {
        return format!("messages:{}:{path}", value["index"]);
    }
    format!(
        "responses:{}:{}:{}:{path}",
        value["item_id"], value["content_index"], value["type"]
    )
}
fn clear_delta_strings(value: &mut Value) {
    if let Some(choices) = value.get_mut("choices").and_then(Value::as_array_mut) {
        for choice in choices {
            if choice.get("text").is_some() {
                choice["text"] = Value::String(String::new());
            }
            if let Some(delta) = choice.get_mut("delta") {
                fn clear(v: &mut Value) {
                    match v {
                        Value::String(s) => s.clear(),
                        Value::Array(a) => {
                            for v in a {
                                clear(v)
                            }
                        }
                        Value::Object(m) => {
                            m.remove("role");
                            m.remove("id");
                            m.remove("name");
                            for v in m.values_mut() {
                                clear(v)
                            }
                        }
                        _ => {}
                    }
                }
                clear(delta);
            }
        }
    } else if value.get("done").is_some() {
        if value.get("response").is_some() {
            value["response"] = Value::String(String::new());
        }
        if let Some(message) = value.get_mut("message").and_then(Value::as_object_mut) {
            for key in ["content", "thinking"] {
                if message.contains_key(key) {
                    message.insert(key.into(), Value::String(String::new()));
                }
            }
            message.remove("tool_calls");
        }
    } else if let Some(delta) = value.get_mut("delta") {
        fn clear(v: &mut Value) {
            match v {
                Value::String(s) => s.clear(),
                Value::Object(m) => {
                    for (k, v) in m {
                        if k != "type" {
                            clear(v)
                        }
                    }
                }
                _ => {}
            }
        }
        clear(delta);
    }
}
fn is_delta(value: &Value) -> bool {
    value
        .get("choices")
        .and_then(Value::as_array)
        .is_some_and(|a| {
            a.iter()
                .any(|v| v.get("delta").is_some() || v.get("text").is_some())
        })
        || value.get("done").is_some()
        || value["type"]
            .as_str()
            .is_some_and(|s| s.ends_with(".delta") || s == "content_block_delta")
}
fn collect_deltas(value: &Value, path: &str, out: &mut Vec<(String, bool)>) {
    if value.pointer("/delta/type").and_then(Value::as_str) == Some("thinking_delta") {
        return;
    }
    if !is_delta(value) {
        return;
    }
    fn walk(value: &Value, path: &str, out: &mut Vec<(String, bool)>) {
        match value {
            Value::Object(map) => {
                for (key, v) in map {
                    let p = format!("{path}/{key}");
                    if v.is_string()
                        && matches!(
                            key.as_str(),
                            "content"
                                | "response"
                                | "reasoning_content"
                                | "text"
                                | "thinking"
                                | "refusal"
                                | "delta"
                                | "arguments"
                                | "partial_json"
                        )
                    {
                        out.push((
                            p,
                            matches!(key.as_str(), "arguments" | "partial_json")
                                || value["type"]
                                    .as_str()
                                    .is_some_and(|s| s.contains("arguments")),
                        ))
                    } else {
                        walk(v, &p, out)
                    }
                }
            }
            Value::Array(a) => {
                for (i, v) in a.iter().enumerate() {
                    walk(v, &format!("{path}/{i}"), out)
                }
            }
            _ => {}
        }
    }
    walk(value, path, out)
}

/// Holds the authorization capability until the response finishes or is dropped.
pub struct ResponseRestorer {
    vault: Arc<Vault>,
    context: Arc<dyn SecretContext>,
    events: Events,
}
impl ResponseRestorer {
    pub fn new(vault: Arc<Vault>) -> Self {
        Self {
            context: vault.clone(),
            vault,
            events: Events::default(),
        }
    }
    fn bind(&self, value: &Value) -> lr_types::AppResult<()> {
        self.vault.bind_opaque(value)?;
        if let Some(id) = value
            .pointer("/response/id")
            .or_else(|| value.get("id"))
            .and_then(Value::as_str)
            .filter(|id| id.starts_with("resp_"))
        {
            self.vault.bind_response(id)?;
        }
        Ok(())
    }
    pub fn json(&mut self, bytes: &[u8]) -> lr_types::AppResult<Vec<u8>> {
        let mut value: Value = serde_json::from_slice(bytes).map_err(|_| {
            lr_types::AppError::InvalidParams("Invalid protected response JSON".into())
        })?;
        self.bind(&value)?;
        self.context.restore(&mut value, false)?;
        self.context.validate_response(&value)?;
        let bytes = serde_json::to_vec(&value).map_err(|_| {
            lr_types::AppError::Internal("Could not encode protected response".into())
        })?;
        if bytes.len() > LIMIT {
            return Err(lr_types::AppError::InvalidParams(
                "Restored response exceeds limit".into(),
            ));
        }
        Ok(bytes)
    }
    pub fn frame(&mut self, frame: &str) -> lr_types::AppResult<Vec<String>> {
        let data = frame
            .lines()
            .filter_map(|l| l.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        if let Ok(value) = serde_json::from_str::<Value>(&data) {
            self.bind(&value)?;
        }
        self.events.frame(frame, &self.context)
    }
    /// WebSocket messages carry the same JSON events without the SSE envelope.
    pub fn message(&mut self, text: &str) -> lr_types::AppResult<Vec<String>> {
        self.frame(&format!("data: {text}"))?
            .into_iter()
            .map(|frame| {
                frame
                    .lines()
                    .find_map(|l| l.strip_prefix("data: "))
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        lr_types::AppError::InvalidParams(
                            "Invalid protected WebSocket event".into(),
                        )
                    })
            })
            .collect()
    }
    pub fn finish(&mut self) -> lr_types::AppResult<Vec<String>> {
        if !self.events.done {
            return Err(lr_types::AppError::InvalidParams(
                "Protected stream ended without completion".into(),
            ));
        }
        self.events.finish(&self.context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (Arc<Vault>, String, String) {
        let vault = Arc::new(Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "owner",
            "wire",
            false,
        ));
        let secret = "synthetic \"quoted\"\n🔐\\original".to_owned();
        vault.register(&secret).unwrap();
        let mut value = json!({"content":secret});
        vault.protect(&mut value).unwrap();
        (vault, value["content"].as_str().unwrap().into(), secret)
    }
    fn values(frames: Vec<String>) -> Vec<Value> {
        frames
            .iter()
            .filter_map(|f| {
                f.lines()
                    .find_map(|l| l.strip_prefix("data: "))
                    .and_then(|d| serde_json::from_str(d).ok())
            })
            .collect()
    }
    #[test]
    fn legacy_completion_and_structured_json_every_split() {
        let (vault, tag, secret) = fixture();
        let context: Arc<dyn SecretContext> = vault;
        for structured in [false, true] {
            let content = if structured {
                json!({tag.clone(): {"id":tag,"type":tag,"name":tag}}).to_string()
            } else {
                tag.clone()
            };
            for split in 0..=content.len() {
                if !content.is_char_boundary(split) {
                    continue;
                }
                let mut events = Events::default();
                let mut frames = Vec::new();
                for fragment in [&content[..split], &content[split..]] {
                    frames.extend(events.frame(&format!("data: {}",json!({"choices":[{"index":0,"text":fragment,"finish_reason":null}]})), &context).unwrap());
                }
                frames.extend(events.frame("data: [DONE]", &context).unwrap());
                let actual: String = values(frames)
                    .iter()
                    .filter_map(|v| v.pointer("/choices/0/text").and_then(Value::as_str))
                    .collect();
                if structured {
                    assert_eq!(
                        serde_json::from_str::<Value>(&actual).unwrap(),
                        json!({secret.clone():{"id":secret,"type":secret,"name":secret}})
                    );
                } else {
                    assert_eq!(actual, secret);
                }
            }
        }
    }
    #[test]
    fn escaped_tags_in_markdown_json_are_restored_once_and_escaped() {
        let (vault, tag, secret) = fixture();
        let context: Arc<dyn SecretContext> = vault;
        let content = format!("```json\n{}\n```", json!({"content":tag}));
        let mut stream = ContentStream::default();
        let mut output = String::new();
        for ch in content.chars() {
            output.push_str(&stream.push(&ch.to_string(), false, &context).unwrap());
        }
        output.push_str(&stream.push("", true, &context).unwrap());
        assert_eq!(
            serde_json::from_str::<Value>(
                output
                    .strip_prefix("```json\n")
                    .unwrap()
                    .strip_suffix("\n```")
                    .unwrap()
            )
            .unwrap()["content"],
            secret
        );
    }
    #[test]
    fn tool_schema_is_checked_after_fragmented_restoration() {
        let (vault, tag, secret) = fixture();
        vault.set_request_contract(&json!({"tools":[{"type":"function","function":{"name":"write_file","parameters":{"type":"object","properties":{"content":{"const":secret}},"required":["content"],"additionalProperties":false}}}]})).unwrap();
        let mut restorer = ResponseRestorer::new(vault);
        restorer.message(&json!({"type":"response.output_item.added","item":{"id":"call","type":"function_call","name":"write_file","arguments":""}}).to_string()).unwrap();
        let args = json!({"content":tag}).to_string();
        for fragment in args.as_bytes().chunks(1) {
            restorer.message(&json!({"type":"response.function_call_arguments.delta","item_id":"call","delta":std::str::from_utf8(fragment).unwrap()}).to_string()).unwrap();
        }
        let output = restorer.message(&json!({"type":"response.function_call_arguments.done","item_id":"call","arguments":args}).to_string()).unwrap();
        assert!(output
            .iter()
            .any(|s| serde_json::from_str::<Value>(s).unwrap()["delta"]
                .as_str()
                .is_some_and(|s| serde_json::from_str::<Value>(s).unwrap()["content"] == secret)));
    }
    #[test]
    fn ollama_streams_restore_content_thinking_and_terminal_fragment() {
        let (vault, tag, secret) = fixture();
        let mut codec = ResponseRestorer::new(vault);
        let mut output = Vec::new();
        for (i, fragment) in [&tag[..8], &tag[8..]].iter().enumerate() {
            output.extend(codec.message(&json!({"message":{"role":"assistant","content":fragment,"thinking":fragment},"done":i==1}).to_string()).unwrap());
        }
        codec.finish().unwrap();
        for key in ["content", "thinking"] {
            let text: String = output
                .iter()
                .filter_map(|s| {
                    serde_json::from_str::<Value>(s).unwrap()["message"][key]
                        .as_str()
                        .map(str::to_owned)
                })
                .collect();
            assert_eq!(text, secret);
        }
    }
    #[test]
    fn metadata_and_scope_are_retained_and_incomplete_stream_fails() {
        let (vault, tag, secret) = fixture();
        let mut codec = ResponseRestorer::new(vault.clone());
        let response = json!({"id":"resp_scope","object":"response","output":[{"type":"output_text","text":tag}]});
        assert_eq!(
            serde_json::from_slice::<Value>(&codec.json(response.to_string().as_bytes()).unwrap())
                .unwrap()["output"][0]["text"],
            secret
        );
        let mut events = Events::default();
        let context: Arc<dyn SecretContext> = vault;
        let output = events
            .frame(
                &format!(
                    ": keepalive\nid: event-12\nretry: 1000\ndata: {}",
                    json!({"type":"response.output_text.delta","delta":"hello"})
                ),
                &context,
            )
            .unwrap()
            .join("");
        assert!(output.contains("id: event-12\nretry: 1000\n"));
        assert!(output.contains(": keepalive"));
        assert!(codec.finish().is_err());
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn unlimited_parallel_channels_cannot_accumulate() {
        let vault = Arc::new(Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "budget",
            "scope",
            true,
        ));
        let mut codec = ResponseRestorer::new(vault);
        for index in 0..1024 {
            codec.message(&json!({"type":"response.output_item.added","item":{"id":format!("call-{index}"),"type":"function_call","name":"write","arguments":""}}).to_string()).unwrap();
        }
        assert!(codec.message(&json!({"type":"response.output_item.added","item":{"id":"excess","type":"function_call","name":"write","arguments":""}}).to_string()).is_err());
    }
}
