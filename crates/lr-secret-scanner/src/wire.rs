//! Response restoration shared by the gateway and native HTTP/WebSocket proxy.
use crate::placeholders::{TextStream, Vault};
use lr_types::secret_context::SecretContext;
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
pub const LIMIT: usize = 16 * 1024 * 1024;
/// Decode one Responses resource segment without assuming a provider ID prefix.
/// Endpoint suffixes remain structural; encoded slashes belong to the opaque ID.
pub fn response_id(path: &str) -> Option<String> {
    let (_, suffix) = path.split('?').next()?.rsplit_once("/responses/")?;
    let id = suffix
        .strip_suffix("/input_items")
        .or_else(|| suffix.strip_suffix("/cancel"))
        .unwrap_or(suffix);
    let bytes = id.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit) =>
            {
                i += 3
            }
            b if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') => i += 1,
            _ => return None,
        }
    }
    let decoded = percent_encoding::percent_decode_str(id)
        .decode_utf8()
        .ok()?;
    (!decoded.is_empty() && !decoded.chars().any(char::is_control)).then(|| decoded.into_owned())
}

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
        let mut value: Value = match serde_json::from_str(&data) {
            Ok(value) => value,
            Err(_) => {
                context.diagnostic("Provider event was not JSON; delivered unchanged");
                return Ok(vec![format!("{frame}\n\n")]);
            }
        };
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
            value
                .get("item")
                .and_then(crate::schema::qualified_tool_name),
        ) {
            self.tool_names.insert(format!("responses:{id}:"), name);
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
                context.restore_text_at(&arguments, true, "Tool arguments")?
            } else {
                let mut args: Value = match serde_json::from_str(&arguments) {
                    Ok(args) => args,
                    Err(_) => {
                        context.diagnostic(
                            "Tool arguments were not complete JSON; delivered unchanged",
                        );
                        *template.pointer_mut(&path).unwrap() = Value::String(arguments);
                        out.push(encode(&template, template["type"].as_str().unwrap_or("")));
                        continue;
                    }
                };
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
    original: Vec<String>,
    output: Vec<String>,
    held_bytes: usize,
    passthrough: bool,
}
impl ResponseRestorer {
    pub fn new(vault: Arc<Vault>) -> Self {
        Self {
            context: vault.clone(),
            vault,
            events: Events::default(),
            original: Vec::new(),
            output: Vec::new(),
            held_bytes: 0,
            passthrough: false,
        }
    }
    fn bind(&self, value: &Value) -> lr_types::AppResult<()> {
        if self.vault.client_scoped() {
            return Ok(());
        }
        self.vault.bind_opaque(value)?;
        // Bind response objects, never item/call/event IDs. Providers choose
        // their own ID format; no prefix is part of the Responses contract.
        let response = if value
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.starts_with("response."))
        {
            value.get("response")
        } else if matches!(
            value.get("object").and_then(Value::as_str),
            Some("response" | "response.compaction")
        ) || (value.get("type").is_none()
            && value.get("output").is_some_and(Value::is_array))
        {
            Some(value)
        } else {
            None
        };
        if let Some(id) = response
            .and_then(|response| response.get("id"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            self.vault.bind_response(id)?;
        }
        Ok(())
    }
    pub fn json(&mut self, bytes: &[u8]) -> lr_types::AppResult<Vec<u8>> {
        if bytes.len() > LIMIT {
            self.context
                .diagnostic("Response exceeds restoration budget; original content delivered");
            return Ok(bytes.to_vec());
        }
        let candidate = (|| -> lr_types::AppResult<Vec<u8>> {
            let mut value: Value = serde_json::from_slice(bytes)?;
            let original = value.clone();
            self.bind(&value)?;
            self.context.restore(&mut value, false)?;
            self.context.validate_response(&value)?;
            if value == original {
                return Ok(bytes.to_vec());
            }
            let encoded = serde_json::to_vec(&value)?;
            if encoded.len() > LIMIT {
                return Err(lr_types::AppError::InvalidParams(
                    "Restored response exceeds budget".into(),
                ));
            }
            Ok(encoded)
        })();
        match candidate {
            Ok(value) => Ok(value),
            Err(_) => {
                self.context.diagnostic(
                    "Response could not be restored safely; original content delivered",
                );
                Ok(bytes.to_vec())
            }
        }
    }
    /// Release only outputs whose source frames no longer have buffered content.
    /// If adaptation fails, none of these original frames has been emitted yet.
    pub fn frame(&mut self, frame: &str) -> lr_types::AppResult<Vec<String>> {
        let original = if frame_end(frame.as_bytes()).is_some_and(|(at, n)| at + n == frame.len()) {
            frame.to_owned()
        } else {
            format!("{frame}\n\n")
        };
        if self.passthrough {
            return Ok(vec![original]);
        }
        self.held_bytes = self.held_bytes.saturating_add(original.len());
        self.original.push(original);
        if self.held_bytes > LIMIT {
            return Ok(
                self.abandon("Restoration buffering limit reached; original events delivered")
            );
        }
        let parsed = frame
            .strip_suffix("\r\n\r\n")
            .or_else(|| frame.strip_suffix("\n\n"))
            .unwrap_or(frame);
        let data = parsed
            .lines()
            .filter_map(|l| l.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        let result = (|| {
            if let Ok(value) = serde_json::from_str::<Value>(&data) {
                self.bind(&value)?;
            }
            self.events.frame(parsed, &self.context)
        })();
        match result {
            Ok(output) => {
                self.held_bytes = self
                    .held_bytes
                    .saturating_add(output.iter().map(String::len).sum::<usize>());
                self.output.extend(output);
                if self.held_bytes > LIMIT {
                    return Ok(
                        self.abandon("Restoration output limit reached; original events delivered")
                    );
                }
                let pending = self
                    .events
                    .arguments
                    .values()
                    .any(|(s, _, _)| !s.is_empty())
                    || self
                        .events
                        .text
                        .values()
                        .any(|(s, _, _)| !s.pending.is_empty() || s.text.buffered_bytes() > 0);
                if pending {
                    Ok(Vec::new())
                } else {
                    self.original.clear();
                    self.held_bytes = 0;
                    Ok(std::mem::take(&mut self.output))
                }
            }
            Err(_) => {
                Ok(self.abandon("Response adaptation unavailable; original events delivered"))
            }
        }
    }
    pub fn message(&mut self, text: &str) -> lr_types::AppResult<Vec<String>> {
        self.frame(&format!("data: {text}"))?
            .into_iter()
            .map(|frame| Ok(message_data(&frame)))
            .collect()
    }
    pub fn done(&self) -> bool {
        self.events.done
    }
    pub fn abandon_messages(&mut self, reason: &str) -> Vec<String> {
        self.abandon(reason)
            .into_iter()
            .map(|frame| message_data(&frame))
            .collect()
    }
    /// Switch to pass-through without losing or duplicating held original data.
    pub fn abandon(&mut self, reason: &str) -> Vec<String> {
        self.context.diagnostic(reason);
        self.passthrough = true;
        self.output.clear();
        self.held_bytes = 0;
        self.events = Events::default();
        std::mem::take(&mut self.original)
    }
    pub fn finish(&mut self) -> lr_types::AppResult<Vec<String>> {
        if self.passthrough {
            return Ok(Vec::new());
        }
        if !self.events.done || !self.original.is_empty() {
            return Ok(self.abandon("Provider stream ended with incomplete restoration state; original buffered content delivered"));
        }
        Ok(Vec::new())
    }
}

fn message_data(frame: &str) -> String {
    let payload = if let Some(payload) = frame.strip_prefix("data: ") {
        payload
    } else if let Some(at) = frame.find("\ndata: ") {
        &frame[at + 7..]
    } else {
        frame
    };
    payload.strip_suffix("\n\n").unwrap_or(payload).to_owned()
}

#[derive(Clone, Copy)]
pub enum StreamMode {
    Json,
    Sse,
    Ndjson,
}
/// Bounded best-effort restoration. Only errors from the original transport
/// propagate; adaptation failures release original bytes and stop rewriting.
pub fn restore_stream<S, E>(
    stream: S,
    vault: Arc<Vault>,
    mode: StreamMode,
    completed: Arc<std::sync::atomic::AtomicBool>,
) -> std::pin::Pin<Box<dyn futures::Stream<Item = Result<bytes::Bytes, E>> + Send>>
where
    S: futures::Stream<Item = Result<bytes::Bytes, E>> + Send + 'static,
    E: Send + 'static,
{
    use bytes::Bytes;
    use futures::StreamExt;
    Box::pin(async_stream::stream! {
        let mut stream=Box::pin(stream);
        let mut codec=ResponseRestorer::new(vault.clone());
        let mut pending=Vec::new();
        let mut scanned=0;
        let mut passthrough=false;
        let encode = |frame: String| -> Bytes { if matches!(mode, StreamMode::Ndjson) { Bytes::from(format!("{}\n",message_data(&frame))) } else {Bytes::from(frame)} };
        while let Some(chunk)=stream.next().await {
            let chunk=match chunk {
                Ok(chunk)=>chunk,
                Err(error)=>{
                    for frame in codec.abandon("Upstream transport ended; original buffered content released") { yield Ok(encode(frame)); }
                    if !pending.is_empty() { yield Ok(Bytes::from(std::mem::take(&mut pending))); }
                    yield Err(error); return;
                }
            };
            if passthrough { yield Ok(chunk); continue; }
            pending.extend_from_slice(&chunk);
            if pending.len()>LIMIT {
                for frame in codec.abandon("Restoration byte limit reached; original content delivered") { yield Ok(encode(frame)); }
                yield Ok(Bytes::from(std::mem::take(&mut pending))); passthrough=true; continue;
            }
            if matches!(mode,StreamMode::Json) { continue; }
            loop {
                let boundary=if matches!(mode,StreamMode::Ndjson) { pending[scanned..].iter().position(|b|*b==b'\n').map(|i|(i+scanned,1)) } else {frame_end(&pending[scanned..]).map(|(i,n)|(i+scanned,n))};
                let Some((end,sep))=boundary else { scanned=pending.len().saturating_sub(3); break; };
                let Ok(frame)=std::str::from_utf8(&pending[..end+sep]) else {
                    for frame in codec.abandon("Unsupported response bytes; original content delivered") { yield Ok(encode(frame)); }
                    yield Ok(Bytes::from(std::mem::take(&mut pending))); passthrough=true; break;
                };
                let output=if matches!(mode,StreamMode::Ndjson) {
                    if frame.trim().is_empty() { vec![frame.to_owned()] } else { codec.message(frame.strip_suffix('\n').unwrap_or(frame)).unwrap_or_else(|_|vec![frame.into()]).into_iter().map(|s|format!("{s}\n")).collect() }
                } else { codec.frame(frame).unwrap_or_else(|_|vec![frame.into()]) };
                pending.drain(..end+sep); scanned=0;
                let last=output.len().saturating_sub(1);
                for (i,output)in output.into_iter().enumerate() {
                    if i==last && codec.done() { completed.store(true,std::sync::atomic::Ordering::Release); }
                    yield Ok(Bytes::from(output));
                }
            }
        }
        if !passthrough {
            if matches!(mode,StreamMode::Json) {
                let result=codec.json(&pending).unwrap_or(pending);
                completed.store(true,std::sync::atomic::Ordering::Release);
                if !result.is_empty() {yield Ok(Bytes::from(result));}
            } else {
                for frame in codec.finish().unwrap_or_default() { yield Ok(encode(frame)); }
                if !pending.is_empty() { vault.diagnostic("Incomplete response frame delivered unchanged"); yield Ok(Bytes::from(pending)); }
            }
        }
        completed.store(true,std::sync::atomic::Ordering::Release);
    })
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
    fn response_resource_paths_keep_opaque_ids_and_structural_suffixes_separate() {
        for path in [
            "/v1/responses/opaque%2Fid%3AABC",
            "/v1/responses/opaque%2Fid%3AABC/input_items?limit=1",
            "/backend-api/codex/responses/opaque%2Fid%3AABC/cancel",
        ] {
            assert_eq!(response_id(path).as_deref(), Some("opaque/id:ABC"));
        }
        assert_eq!(
            response_id("/v1/responses/resp-1").as_deref(),
            Some("resp-1")
        );
        for path in [
            "/v1/responses/",
            "/v1/responses/id/unknown",
            "/v1/responses/%ZZ",
            "/v1/responses/%00",
            "/v1/responses/%FF",
        ] {
            assert!(response_id(path).is_none(), "{path}");
        }
    }
    #[test]
    fn websocket_code_tool_restores_secret_and_preserves_literal_tag_check() {
        let vault = Arc::new(Vault::for_client(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "code-literal-client",
        ));
        let secret = "synthetic-\"quoted\"-'single'-\\path\n<&>";
        vault.register(secret).unwrap();
        let mut request = json!({"input": secret});
        vault.protect(&mut request).unwrap();
        let tag = request["input"].as_str().unwrap();
        // Code Mode sends JavaScript containing JSON-encoded shell arguments.
        let code = |value: &str| {
            format!(
                "tools.exec_command({});",
                json!({"cmd": format!("expected = {}\nassert '<localrouter-redacted' not in actual", serde_json::to_string(value).unwrap())})
            )
        };
        let input = code(tag);
        let expected = code(secret);
        // Exercise every split point, including both tag names and JSON escapes.
        for split in 0..=input.len() {
            if !input.is_char_boundary(split) {
                continue;
            }
            let mut restorer = ResponseRestorer::new(vault.clone());
            let mut output = Vec::new();
            for delta in [&input[..split], &input[split..]] {
                output.extend(
                    restorer
                        .message(
                            &json!({
                                "type":"response.custom_tool_call_input.delta",
                                "item_id":"tool-1", "output_index":0, "delta":delta
                            })
                            .to_string(),
                        )
                        .unwrap(),
                );
            }
            output.extend(
                restorer
                    .message(
                        &json!({
                            "type":"response.custom_tool_call_input.done",
                            "item_id":"tool-1", "output_index":0, "input":input
                        })
                        .to_string(),
                    )
                    .unwrap(),
            );
            let events: Vec<Value> = output
                .iter()
                .map(|s| serde_json::from_str(s).unwrap())
                .collect();
            let deltas: String = events
                .iter()
                .filter(|v| v["type"] == "response.custom_tool_call_input.delta")
                .map(|v| v["delta"].as_str().unwrap())
                .collect();
            assert_eq!(deltas, expected, "split {split}");
            assert_eq!(events.last().unwrap()["input"], expected);
        }
    }
    #[test]
    fn binds_only_response_objects_across_http_sse_and_websocket() {
        let store = Arc::new(lr_api_keys::MockKeychain::new());
        let vault = Arc::new(Vault::new(store.clone(), "wire-binding", "scope", false));
        let mut restorer = ResponseRestorer::new(vault);
        for event in [
            json!({"type":"response.created", "response":{"id":"warm-up/id"}}),
            json!({"type":"response.completed", "response":{"id":"opaque-final"}}),
        ] {
            restorer.message(&event.to_string()).unwrap();
        }
        restorer
            .frame(&format!(
                "data: {}",
                json!({"type":"response.created","response":{"id":"sse-id"}})
            ))
            .unwrap();
        restorer
            .json(
                json!({"object":"response.compaction", "id":"compaction-id", "output":[]})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        restorer.bind(&json!({"type":"response.output_item.done","id":"resp_not_a_response","item":{"id":"tool-id"}})).unwrap();
        for id in ["warm-up/id", "opaque-final", "sse-id", "compaction-id"] {
            assert!(
                Vault::find_response(store.clone(), "wire-binding", id)
                    .unwrap()
                    .is_some(),
                "{id}"
            );
        }
        for id in ["resp_not_a_response", "tool-id"] {
            assert!(
                Vault::find_response(store.clone(), "wire-binding", id)
                    .unwrap()
                    .is_none(),
                "{id}"
            );
        }
    }
    #[test]
    fn provider_response_ids_are_opaque_and_not_prefix_filtered() {
        let store = Arc::new(lr_api_keys::MockKeychain::new());
        let vault = Arc::new(Vault::new(
            store.clone(),
            "opaque-id-owner",
            "conversation",
            false,
        ));
        let mut restorer = ResponseRestorer::new(vault);
        restorer
            .json(
                json!({"object":"response","id":"opaque-provider-response","output":[]})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        drop(restorer);
        assert!(
            Vault::continue_response(store, "opaque-id-owner", "opaque-provider-response").is_ok()
        );
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
                .filter(|s| !s.is_empty())
                .is_some_and(|s| serde_json::from_str::<Value>(s).unwrap()["content"] == secret)));
    }
    #[test]
    fn namespaced_streamed_tools_keep_schema_identity() {
        let (vault, tag, secret) = fixture();
        vault.set_request_contract(&json!({"tools":[
            {"type":"namespace","name":"alpha","tools":[{"type":"function","name":"write","parameters":{"type":"object","properties":{"content":{"type":"string"}},"required":["content"]}}]},
            {"type":"namespace","name":"beta","tools":[{"type":"function","name":"write","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}]}
        ]})).unwrap();
        let mut restorer = ResponseRestorer::new(vault);
        for (ns, field) in [("alpha", "content"), ("beta", "path")] {
            restorer.message(&json!({"type":"response.output_item.added","item":{"id":ns,"type":"function_call","namespace":ns,"name":"write","arguments":""}}).to_string()).unwrap();
            let arguments = json!({field:tag}).to_string();
            restorer.message(&json!({"type":"response.function_call_arguments.delta","item_id":ns,"delta":arguments}).to_string()).unwrap();
            let output=restorer.message(&json!({"type":"response.function_call_arguments.done","item_id":ns,"arguments":arguments}).to_string()).unwrap();
            assert!(output
                .iter()
                .any(|s| serde_json::from_str::<Value>(s).unwrap()["delta"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .is_some_and(|s| serde_json::from_str::<Value>(s).unwrap()[field] == secret)));
        }
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
    fn metadata_and_scope_are_retained_and_incomplete_stream_passes() {
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
        assert!(codec.finish().is_ok());
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
        assert!(codec.message(&json!({"type":"response.output_item.added","item":{"id":"excess","type":"function_call","name":"write","arguments":""}}).to_string()).is_ok());
    }
}
