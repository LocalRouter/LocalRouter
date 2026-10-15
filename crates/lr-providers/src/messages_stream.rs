//! Native Messages SSE framing and policy/accounting projection. The outward
//! event retains the original data string, including signature deltas.
use std::pin::Pin;

use futures::{Stream, StreamExt};
use lr_types::{AppError, AppResult};
use serde_json::{json, Map, Value};

use crate::{messages, ChunkChoice, ChunkDelta, CompletionChunk, CompletionRequest, ModelProvider};

type CompletionStream = Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>;
const MAX_FRAME: usize = 2 * 1024 * 1024;

pub async fn dispatch_stream(
    provider: &dyn ModelProvider,
    request: CompletionRequest,
) -> AppResult<CompletionStream> {
    let Some(context) = request.native_messages.as_ref() else {
        return provider.stream_complete(request).await;
    };
    if !provider.supports_native_messages() {
        return provider
            .stream_complete(context.translated(&request)?)
            .await;
    }
    let native = context.prepared(&request)?;
    let response = provider
        .native_messages(
            &native,
            &context.headers,
            messages::MessagesOperation::Create,
        )
        .await?;
    if !response.status().is_success() {
        return Err(messages::response_error(response).await);
    }
    if !response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("text/event-stream"))
        })
    {
        return Err(AppError::Provider(
            "Native Messages stream requires text/event-stream".into(),
        ));
    }
    let provider = provider.name().to_owned();
    let model = request.model;
    Ok(Box::pin(async_stream::try_stream! {
        let mut bytes=response.bytes_stream();
        let mut partial=Vec::new();
        let mut frame=NativeFrame::default();
        let mut state=NativeState {model,..Default::default()};
        let mut stopped=false;
        'reads: while let Some(read)=bytes.next().await {
            let read=read.map_err(|e|AppError::Provider(crate::http_client::format_stream_error(&e)))?;
            // Consume bytes incrementally: a single network read can contain
            // many frames and must not count as one enormous retained frame.
            for byte in read {
                if byte!=b'\n' {
                    partial.push(byte);
                    if partial.len().saturating_add(frame.bytes)>MAX_FRAME {
                        Err(AppError::Provider("Native Messages SSE frame exceeds 2 MiB".into()))?;
                    }
                    continue;
                }
                let line=std::mem::take(&mut partial);
                let line=line.strip_suffix(b"\r").unwrap_or(&line);
                let line=std::str::from_utf8(line).map_err(|_|AppError::Provider("Invalid native SSE UTF-8".into()))?;
                if let Some((name,data))=frame.line(line)? {
                    let (chunk,end,error)=state.event(&provider,name,data)?;
                    yield chunk;
                    if error { Err(AppError::Provider("Native Messages emitted a terminal error event".into()))?; }
                    if end { stopped=true; break 'reads; }
                }
            }
        }
        if !stopped {
            Err(AppError::Provider("Native Messages stream ended without message_stop".into()))?;
        }
    }))
}

#[derive(Default)]
struct NativeFrame {
    name: Option<String>,
    data: Vec<String>,
    bytes: usize,
}

impl NativeFrame {
    fn line(&mut self, line: &str) -> AppResult<Option<(String, String)>> {
        if line.is_empty() {
            self.bytes = 0;
            let name = self.name.take().unwrap_or_else(|| "message".into());
            let data = std::mem::take(&mut self.data);
            return Ok((!data.is_empty()).then(|| (name, data.join("\n"))));
        }
        self.bytes = self.bytes.saturating_add(line.len() + 1);
        if self.bytes > MAX_FRAME {
            return Err(AppError::Provider(
                "Native Messages SSE frame exceeds 2 MiB".into(),
            ));
        }
        if let Some(value) = line.strip_prefix("event:") {
            let value = value.strip_prefix(' ').unwrap_or(value);
            if value.len() > 128
                || !value
                    .bytes()
                    .all(|v| v.is_ascii_alphanumeric() || v == b'_')
            {
                return Err(AppError::Provider(
                    "Invalid native Messages SSE event name".into(),
                ));
            }
            self.name = Some(value.into());
        } else if let Some(value) = line.strip_prefix("data:") {
            self.data
                .push(value.strip_prefix(' ').unwrap_or(value).into());
        }
        Ok(None)
    }
}

#[derive(Default)]
struct NativeState {
    id: String,
    model: String,
    usage: Map<String, Value>,
    started: bool,
    final_delta: bool,
}

impl NativeState {
    fn event(
        &mut self,
        provider: &str,
        name: String,
        data: String,
    ) -> AppResult<(CompletionChunk, bool, bool)> {
        let value: Value = serde_json::from_str(&data)
            .map_err(|e| AppError::Provider(format!("Invalid native Messages SSE JSON: {e}")))?;
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Provider("Native Messages SSE event requires type".into()))?;
        if name != "message" && name != kind {
            return Err(AppError::Provider(
                "Native Messages SSE event/type mismatch".into(),
            ));
        }
        if kind == "message_start" {
            if self.started {
                return Err(AppError::Provider(
                    "Repeated native Messages message_start".into(),
                ));
            }
            self.id = value["message"]["id"]
                .as_str()
                .filter(|v| !v.is_empty())
                .ok_or_else(|| AppError::Provider("Native message_start requires id".into()))?
                .into();
            self.model = value["message"]["model"]
                .as_str()
                .filter(|v| !v.is_empty())
                .ok_or_else(|| AppError::Provider("Native message_start requires model".into()))?
                .into();
            self.started = true;
        } else if !self.started && !matches!(kind, "ping" | "error") {
            return Err(AppError::Provider(
                "Native Messages event before message_start".into(),
            ));
        }
        let usage = if kind == "message_start" {
            value["message"].get("usage")
        } else {
            value.get("usage")
        };
        if let Some(usage) = usage {
            let usage = usage
                .as_object()
                .ok_or_else(|| AppError::Provider("Invalid native Messages stream usage".into()))?;
            self.usage.extend(
                usage
                    .iter()
                    .filter(|(key, _)| {
                        matches!(
                            key.as_str(),
                            "input_tokens"
                                | "output_tokens"
                                | "cache_creation_input_tokens"
                                | "cache_read_input_tokens"
                                | "cache_creation"
                        )
                    })
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }
        let projected_usage = if self.usage.contains_key("input_tokens")
            && self.usage.contains_key("output_tokens")
        {
            Some(messages::native_usage(&Value::Object(self.usage.clone()))?)
        } else {
            None
        };
        let delta = &value["delta"];
        if kind == "message_delta" && delta.get("stop_reason").and_then(Value::as_str).is_some() {
            self.final_delta = true;
        }
        if kind == "message_stop" && !self.final_delta {
            return Err(AppError::Provider(
                "Native Messages stopped without a final message_delta".into(),
            ));
        }
        let text = if delta["type"] == "text_delta" {
            delta["text"].as_str().map(str::to_owned)
        } else {
            None
        };
        let thinking = if delta["type"] == "thinking_delta" {
            delta["thinking"].as_str().map(str::to_owned)
        } else {
            None
        };
        let finish = delta
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(|reason| {
                match reason {
                    "end_turn" | "stop_sequence" => "stop",
                    "max_tokens" => "length",
                    "tool_use" => "tool_calls",
                    other => other,
                }
                .to_owned()
            });
        let chunk = CompletionChunk {
            id: self.id.clone(),
            object: "chat.completion.chunk".into(),
            created: chrono::Utc::now().timestamp(),
            model: self.model.clone(),
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    role: (kind == "message_start").then(|| "assistant".into()),
                    content: text,
                    tool_calls: None,
                    reasoning_content: thinking,
                },
                finish_reason: finish,
            }],
            extensions: Some(
                [(
                    messages::EVENT.into(),
                    json!({"name":if name=="message" {kind} else {&name},"data":data}),
                )]
                .into_iter()
                .collect(),
            ),
            usage: projected_usage,
            provider: Some(provider.into()),
        };
        Ok((chunk, kind == "message_stop", kind == "error"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_preserves_multiline_data_and_signature_values() {
        let mut frame = NativeFrame::default();
        assert!(frame.line("event: content_block_delta").unwrap().is_none());
        frame
            .line("data: {\"type\":\"content_block_delta\",")
            .unwrap();
        frame
            .line("data: \"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig+/=\"}}")
            .unwrap();
        let (name, data) = frame.line("").unwrap().unwrap();
        assert_eq!(name, "content_block_delta");
        assert!(data.contains('\n'));
        assert_eq!(
            serde_json::from_str::<Value>(&data).unwrap()["delta"]["signature"],
            "sig+/="
        );
    }

    #[test]
    fn stream_usage_overlays_cumulative_fields_and_rejects_restarts() {
        let mut state = NativeState::default();
        state.event("fixture","message_start".into(),json!({"type":"message_start","message":{"id":"msg_x","model":"fixture","usage":{"input_tokens":3,"output_tokens":0,"cache_read_input_tokens":5}}}).to_string()).unwrap();
        let (chunk,_,_)=state.event("fixture","message_delta".into(),json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}).to_string()).unwrap();
        let usage = chunk.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 8);
        assert_eq!(usage.completion_tokens, 7);
        assert!(state
            .event(
                "fixture",
                "message_start".into(),
                json!({"type":"message_start"}).to_string()
            )
            .is_err());
        assert!(NativeState::default()
            .event(
                "fixture",
                "message_stop".into(),
                json!({"type":"message_stop"}).to_string()
            )
            .is_err());
    }
}
