//! Bounded native realtime controls. No frame contents are retained by the relay.
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
pub const MAX_FRAME: usize = 1024 * 1024;
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_RESPONSES: usize = 20;
pub const MAX_OUTPUT_TOKENS: u64 = 1024;
pub fn initial_update() -> Value {
    json!({"type":"session.update","session":{"type":"realtime","max_output_tokens":MAX_OUTPUT_TOKENS,"audio":{"input":{"turn_detection":null}},"tools":[]}})
}
fn fields(value: &Value, allowed: &[&str]) -> Result<(), &'static str> {
    let object = value.as_object().ok_or("Expected a JSON object")?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("Unsupported realtime fields");
    }
    Ok(())
}
fn text(value: &Value) -> Result<(), &'static str> {
    if !value.as_str().is_some_and(|text| text.len() <= 16 * 1024) {
        return Err("Realtime text must be a string up to 16 KiB");
    }
    Ok(())
}
fn tokens(value: &Value) -> Result<(), &'static str> {
    if !value
        .as_u64()
        .is_some_and(|n| (1..=MAX_OUTPUT_TOKENS).contains(&n))
    {
        return Err("Realtime output token cap is 1024");
    }
    Ok(())
}
fn tools(value: &Value) -> Result<(), &'static str> {
    let tools = value
        .as_array()
        .filter(|t| t.len() <= 16)
        .ok_or("Realtime tools must be a list of up to 16 client functions")?;
    for tool in tools {
        fields(tool, &["type", "name", "description", "parameters"])?;
        if tool["type"] != "function" {
            return Err("Only client-executed function tools are supported");
        }
        let name = tool["name"]
            .as_str()
            .filter(|n| !n.is_empty() && n.len() <= 64)
            .ok_or("Invalid function name")?;
        if !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
        {
            return Err("Invalid function name");
        }
        if let Some(description) = tool.get("description") {
            text(description)?;
        }
        if tool.get("parameters").is_some_and(|p| !p.is_object()) {
            return Err("Function parameters require an inline JSON schema");
        }
    }
    Ok(())
}
fn audio(value: &Value) -> Result<(), &'static str> {
    fields(value, &["input", "output"])?;
    if let Some(input) = value.get("input") {
        fields(input, &["format", "noise_reduction", "turn_detection"])?;
        if let Some(noise) = input.get("noise_reduction").filter(|v| !v.is_null()) {
            fields(noise, &["type"])?;
            if !matches!(noise["type"].as_str(), Some("near_field" | "far_field")) {
                return Err("Invalid native noise reduction");
            }
        }
        if let Some(vad) = input.get("turn_detection").filter(|v| !v.is_null()) {
            fields(
                vad,
                &[
                    "type",
                    "create_response",
                    "interrupt_response",
                    "threshold",
                    "prefix_padding_ms",
                    "silence_duration_ms",
                    "eagerness",
                ],
            )?;
            if !matches!(vad["type"].as_str(), Some("server_vad" | "semantic_vad"))
                || vad["create_response"] != false
            {
                return Err("VAD requires create_response=false and no idle-timeout generation");
            }
        }
    }
    if let Some(output) = value.get("output") {
        fields(output, &["format", "voice", "speed"])?;
        if output.get("voice").is_some_and(|voice| {
            !matches!(
                voice.as_str(),
                Some(
                    "alloy"
                        | "ash"
                        | "ballad"
                        | "coral"
                        | "echo"
                        | "sage"
                        | "shimmer"
                        | "verse"
                        | "marin"
                        | "cedar"
                )
            )
        }) {
            return Err("Only built-in native voices are supported; custom voice references are not owned by LocalRouter");
        }
        if output
            .get("speed")
            .is_some_and(|speed| !speed.as_f64().is_some_and(|s| (0.25..=1.5).contains(&s)))
        {
            return Err("Invalid native audio speed");
        }
    }
    for channel in ["input", "output"] {
        if let Some(format) = value.get(channel).and_then(|c| c.get("format")) {
            fields(format, &["type", "rate"])?;
            match format["type"].as_str() {
                Some("audio/pcm") if format.get("rate").is_none() || format["rate"] == 24000 => {}
                Some("audio/pcmu" | "audio/pcma") if format.get("rate").is_none() => {}
                _ => return Err("Only native 24 kHz PCM or G.711 audio formats are supported"),
            }
        }
    }
    Ok(())
}
fn controls(
    value: &Value,
    model: &str,
    qualified: &str,
    session: bool,
) -> Result<(), &'static str> {
    let allowed: &[&str] = if session {
        &[
            "type",
            "model",
            "instructions",
            "audio",
            "output_modalities",
            "max_output_tokens",
            "tools",
            "tool_choice",
            "truncation",
        ]
    } else {
        &[
            "instructions",
            "audio",
            "output_modalities",
            "max_output_tokens",
            "tools",
            "tool_choice",
            "conversation",
            "metadata",
        ]
    };
    fields(value, allowed)?;
    if session && value.get("type").is_some_and(|v| v != "realtime") {
        return Err("Only realtime conversation sessions are supported");
    }
    if value
        .get("model")
        .is_some_and(|v| v != model && v != qualified)
    {
        return Err("Realtime model is locked for the connection");
    }
    if let Some(v) = value.get("instructions") {
        text(v)?;
    }
    if let Some(v) = value.get("max_output_tokens") {
        tokens(v)?;
    }
    if let Some(v) = value.get("tools") {
        tools(v)?;
    }
    if let Some(v) = value.get("audio") {
        audio(v)?;
    }
    if let Some(choice) = value.get("tool_choice") {
        if !matches!(choice.as_str(), Some("auto" | "none" | "required")) {
            fields(choice, &["type", "name"])?;
            if choice["type"] != "function" || !choice["name"].is_string() {
                return Err("Only client function tool choices are supported");
            }
        }
    }
    if let Some(modes) = value.get("output_modalities") {
        if !modes.as_array().is_some_and(|modes| {
            modes.len() == 1 && matches!(modes[0].as_str(), Some("text" | "audio"))
        }) {
            return Err("Choose one native text or audio output modality");
        }
    }
    Ok(())
}
#[derive(Default)]
pub struct Policy {
    pub ready: bool,
    pub responses: usize,
    bytes: usize,
    text_bytes: usize,
    // At most MAX_RESPONSES native IDs. Repeated usage snapshots replace the same response.
    usage: HashMap<String, (u64, u64)>,
    native_ids: HashSet<String>,
}
impl Policy {
    pub fn traffic(&mut self, bytes: usize) -> Result<(), &'static str> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or("Realtime traffic limit exceeded")?;
        if bytes > MAX_FRAME || self.bytes > MAX_BYTES {
            return Err("Realtime frame or session traffic limit exceeded");
        }
        Ok(())
    }
    pub fn client(
        &mut self,
        value: &mut Value,
        model: &str,
        qualified: &str,
    ) -> Result<bool, &'static str> {
        if !self.ready {
            return Err("Wait for the native bounded session acknowledgement");
        }
        let kind = value["type"]
            .as_str()
            .ok_or("Realtime event requires a type")?
            .to_owned();
        let response = kind == "response.create";
        match kind.as_str() {
            "session.update" => {
                fields(value, &["type", "event_id", "session"])?;
                controls(&value["session"], model, qualified, true)?;
                if value["session"].get("model").is_some() {
                    value["session"]["model"] = json!(model);
                }
                // Recheck the provider's acknowledgement before accepting more input.
                self.ready = false;
            }
            "response.create" => {
                fields(value, &["type", "event_id", "response"])?;
                if let Some(config) = value.get("response") {
                    controls(config, model, qualified, false)?;
                }
                if self.responses >= MAX_RESPONSES {
                    return Err("Realtime response limit reached");
                }
                self.responses += 1;
            }
            "conversation.item.create" => {
                fields(value, &["type", "event_id", "previous_item_id", "item"])?;
                let item = &value["item"];
                match item["type"].as_str() {
                    Some("message") => {
                        fields(item, &["id", "type", "role", "content"])?;
                        if !matches!(item["role"].as_str(), Some("user" | "system" | "assistant")) {
                            return Err("Unsupported conversation role");
                        }
                        let content = item["content"]
                            .as_array()
                            .filter(|v| v.len() <= 16)
                            .ok_or("Invalid conversation content")?;
                        for part in content {
                            fields(part, &["type", "text"])?;
                            if !matches!(
                                part["type"].as_str(),
                                Some("input_text" | "output_text" | "text")
                            ) {
                                return Err("Conversation items support inline text; stream audio through input_audio_buffer.append");
                            }
                            text(&part["text"])?;
                            self.text_bytes += part["text"].as_str().unwrap_or_default().len();
                        }
                    }
                    Some("function_call_output") => {
                        fields(item, &["id", "type", "call_id", "output"])?;
                        text(&item["output"])?;
                        self.text_bytes += item["output"].as_str().unwrap_or_default().len();
                    }
                    _ => return Err(
                        "Unsupported conversation item; no hosted tools or file/image references",
                    ),
                }
            }
            "input_audio_buffer.append" => {
                fields(value, &["type", "event_id", "audio"])?;
                let audio = value["audio"]
                    .as_str()
                    .ok_or("Audio requires base64 JSON text")?;
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .decode(audio)
                    .map_err(|_| "Invalid base64 audio")?;
            }
            "input_audio_buffer.commit" | "input_audio_buffer.clear" => {
                fields(value, &["type", "event_id"])?
            }
            "response.cancel" => fields(value, &["type", "event_id", "response_id"])?,
            "conversation.item.delete" | "conversation.item.retrieve" => {
                fields(value, &["type", "event_id", "item_id"])?
            }
            "conversation.item.truncate" => fields(
                value,
                &[
                    "type",
                    "event_id",
                    "item_id",
                    "content_index",
                    "audio_end_ms",
                ],
            )?,
            _ => return Err("Unsupported realtime event"),
        }
        if self.text_bytes > 64 * 1024 {
            return Err("Realtime conversation text limit exceeded");
        }
        Ok(response)
    }
    pub fn server(&mut self, value: &Value) -> Result<(), &'static str> {
        match value["type"].as_str() {
            Some("session.updated") => {
                let s = &value["session"];
                let vad = s
                    .pointer("/audio/input/turn_detection")
                    .ok_or("Missing native turn detection acknowledgement")?;
                if let Some(t) = s.get("tools") {
                    tools(t)?;
                }
                if s.get("max_output_tokens")
                    .is_none_or(|v| v.as_u64().is_none_or(|n| n > MAX_OUTPUT_TOKENS))
                    || (!vad.is_null()
                        && (vad["create_response"] != false
                            || vad.get("idle_timeout_ms").is_some_and(|v| !v.is_null())))
                {
                    return Err("Provider did not acknowledge bounded manual response settings");
                }
                self.ready = true;
            }
            Some("response.created") => {
                let id = value["response"]["id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 256)
                    .ok_or("Missing bounded native response ID")?;
                if !self.native_ids.contains(id) && self.native_ids.len() >= self.responses {
                    return Err("Unsolicited native response");
                }
                self.native_ids.insert(id.into());
            }
            Some("response.done") => {
                let r = &value["response"];
                let id = r["id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 256)
                    .ok_or("Missing bounded native response ID")?;
                if !self.native_ids.contains(id) {
                    return Err("Native usage did not match an acknowledged response");
                }
                if let Some(u) = r.get("usage").filter(|v| !v.is_null()) {
                    let input = u["input_tokens"]
                        .as_u64()
                        .filter(|n| *n <= 1_000_000_000)
                        .ok_or("Invalid native input usage")?;
                    let output = u["output_tokens"]
                        .as_u64()
                        .filter(|n| *n <= 1_000_000_000)
                        .ok_or("Invalid native output usage")?;
                    self.usage.insert(id.into(), (input, output));
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn totals(&self) -> (u64, u64, usize) {
        self.usage
            .values()
            .fold((0, 0, self.usage.len()), |(i, o, n), (a, b)| {
                (i + a, o + b, n)
            })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn ready() -> Policy {
        let mut p = Policy::default();
        p.server(&json!({"type":"session.updated","session":{"max_output_tokens":1024,"audio":{"input":{"turn_detection":null}},"tools":[]}})).unwrap();
        p
    }
    #[test]
    fn native_session_requires_explicit_manual_generation_acknowledgement() {
        let mut p = Policy::default();
        assert!(p
            .client(
                &mut json!({"type":"response.create"}),
                "native",
                "gateway/native"
            )
            .is_err());
        for s in [
            json!({"max_output_tokens":1024}),
            json!({"max_output_tokens":"inf","audio":{"input":{"turn_detection":null}}}),
            json!({"max_output_tokens":1024,"audio":{"input":{"turn_detection":{"create_response":true}}}}),
        ] {
            assert!(p
                .server(&json!({"type":"session.updated","session":s}))
                .is_err());
        }
        assert!(ready().ready);
    }
    #[test]
    fn model_hosted_tools_automatic_responses_and_external_inputs_cannot_escape_the_session() {
        for event in [
            json!({"type":"session.update","session":{"model":"other"}}),
            json!({"type":"session.update","session":{"audio":{"output":{"voice":{"id":"voice_foreign"}}}}}),
            json!({"type":"session.update","session":{"tools":[{"type":"mcp","server_url":"https://invalid"}]}}),
            json!({"type":"session.update","session":{"audio":{"input":{"turn_detection":{"type":"server_vad","create_response":true}}}}}),
            json!({"type":"response.create","response":{"max_output_tokens":"inf"}}),
            json!({"type":"response.create","response":{"input":[{"type":"input_image","url":"https://invalid"}]}}),
            json!({"type":"conversation.item.create","item":{"type":"message","role":"user","content":[{"type":"input_image","url":"https://invalid"}]}}),
        ] {
            assert!(
                ready()
                    .client(&mut event.clone(), "native", "gateway/native")
                    .is_err(),
                "{event}"
            );
        }
    }
    #[test]
    fn response_caps_and_native_usage_snapshots_do_not_double_count() {
        let mut p = ready();
        for _ in 0..MAX_RESPONSES {
            assert!(p
                .client(
                    &mut json!({"type":"response.create"}),
                    "native",
                    "gateway/native"
                )
                .unwrap());
        }
        assert!(p
            .client(
                &mut json!({"type":"response.create"}),
                "native",
                "gateway/native"
            )
            .is_err());
        p.server(&json!({"type":"response.created","response":{"id":"resp_1"}}))
            .unwrap();
        let snapshot = json!({"type":"response.done","response":{"id":"resp_1","usage":{"input_tokens":10,"output_tokens":20}}});
        p.server(&snapshot).unwrap();
        p.server(&snapshot).unwrap();
        assert_eq!(p.totals(), (10, 20, 1));
        assert!(p.server(&json!({"type":"response.done","response":{"id":"foreign","usage":{"input_tokens":1,"output_tokens":2}}})).is_err());
    }
    #[test]
    fn text_audio_and_traffic_bounds_apply_before_forwarding() {
        assert!(ready()
            .client(
                &mut json!({"type":"input_audio_buffer.append","audio":"%%%"}),
                "native",
                "gateway/native"
            )
            .is_err());
        assert!(ready().traffic(MAX_FRAME + 1).is_err());
        let mut p = ready();
        for _ in 0..MAX_BYTES / MAX_FRAME {
            p.traffic(MAX_FRAME).unwrap();
        }
        assert!(p.traffic(1).is_err());
    }
}
