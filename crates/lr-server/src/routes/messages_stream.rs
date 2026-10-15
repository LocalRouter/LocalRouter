//! Messages SSE adaptation around the shared router and stream finalizer.
use std::{collections::BTreeMap, convert::Infallible, sync::Arc, time::Instant};

use axum::response::{
    sse::{Event, Sse},
    IntoResponse, Response,
};
use futures::StreamExt;
use parking_lot::Mutex;
use serde_json::{json, Value};

use super::{
    finalize::FinalizeInputs,
    stream_usage::{finalize_stream, AbandonedStream, FinalizeOnDrop, StreamTracker},
};
use crate::{
    middleware::error::{ApiErrorResponse, ApiResult},
    state::{AppState, AuthContext},
    types::ChatCompletionRequest,
};

pub(crate) async fn stream_message(
    state: AppState,
    auth: AuthContext,
    request: ChatCompletionRequest,
    provider_request: lr_providers::CompletionRequest,
    event_id: String,
) -> ApiResult<Response> {
    let model = provider_request.model.clone();
    let estimate = lr_providers::usage_estimate::prompt_tokens(
        &provider_request.messages,
        provider_request.tools.as_deref(),
    );
    let generation_id = format!("gen-{}", uuid::Uuid::new_v4());
    let started_at = Instant::now();
    let created_at = chrono::Utc::now();
    let (mut upstream, routing) = match state
        .router
        .stream_complete(&auth.api_key_id, provider_request.clone())
        .await
    {
        Ok(value) => value,
        Err(lr_types::AppError::FreeTierFallbackAvailable {
            retry_after_secs,
            exhausted_models,
        }) => {
            super::chat::check_free_tier_fallback(
                &state,
                &auth.api_key_id,
                &exhausted_models,
                retry_after_secs,
            )
            .await?;
            state
                .router
                .stream_complete_with_paid_fallback(&auth.api_key_id, provider_request)
                .await
                .map_err(ApiErrorResponse::from)?
        }
        Err(error) => return Err(ApiErrorResponse::from(error)),
    };
    if let Some(routing) = &routing {
        super::monitor_helpers::update_llm_call_routing(&state, &event_id, routing);
    }
    let tracker = Arc::new(Mutex::new(StreamTracker::default()));
    let mut on_drop = FinalizeOnDrop::new(
        AbandonedStream {
            state: state.clone(),
            auth: auth.clone(),
            llm_event_id: event_id.clone(),
            generation_id: generation_id.clone(),
            started_at,
            created_at,
            compression_tokens_saved: 0,
            model: model.clone(),
            prompt_estimate: estimate,
        },
        tracker.clone(),
    );
    let events = async_stream::stream! {
        // Own the drop guard inside the stream even if no bytes are polled.
        let _request=request;
        let mut translated=TranslatedStream::new(model.clone());
        let mut native=false;
        let mut native_error=false;
        let mut failed=false;
        while let Some(result)=upstream.next().await {
            match result {
                Ok(chunk)=> {
                    tracker.lock().observe(&chunk);
                    if let Some(raw)=chunk.extensions.as_ref().and_then(|v|v.get(lr_providers::messages::EVENT)) {
                        native=true;
                        let name=raw["name"].as_str().unwrap_or("error");
                        let data=raw["data"].as_str().unwrap_or("{}");
                        native_error|=name=="error";
                        yield Ok::<Event,Infallible>(Event::default().event(name).data(data));
                    } else if !native {
                        match translated.push(&chunk) {
                            Ok(values)=>for value in values {yield Ok(event(value));},
                            Err(error)=> {yield Ok(error_event(&error));failed=true;break;}
                        }
                    }
                }
                Err(error)=> {
                    if !native_error {yield Ok(error_event(&error.to_string()));}
                    failed=true;
                    break;
                }
            }
        }
        if !native && !failed {
            match translated.finish() {
                Ok(values)=>for value in values {yield Ok(event(value));},
                Err(error)=>yield Ok(error_event(&error)),
            }
        }
        let tracked=std::mem::take(&mut *tracker.lock());
        on_drop.disarm();
        // Complete local accounting even when the consumer disconnects after
        // the terminal event, while the finalizer is awaiting local work.
        let job=lr_types::spawn_traced(async move {
            finalize_stream(FinalizeInputs {state:&state,auth:&auth,llm_event_id:&event_id,generation_id:&generation_id,started_at,created_at,prompt_tokens:0,compression_tokens_saved:0,routing_metadata:routing.as_ref(),user:None,streamed:true,skip_monitor_completion:false},&model,&tracked,estimate).await;
        });
        if let Err(error)=job.await {tracing::warn!("Messages stream finalization failed: {error}");}
    };
    Ok(Sse::new(events).into_response())
}

fn event(value: Value) -> Event {
    Event::default()
        .event(value["type"].as_str().unwrap_or("error"))
        .data(value.to_string())
}
fn error_event(message: &str) -> Event {
    event(json!({"type":"error","error":{"type":"api_error","message":message}}))
}

/// Reuse the existing MCP orchestrator's authorized Chat response and its
/// accounting. Dropping this adapter drops the underlying HTTP body too.
pub(crate) fn translate_chat_response(response: Response) -> Response {
    let mut bytes = response.into_body().into_data_stream();
    let events = async_stream::stream! {
        let mut partial=Vec::new();
        let mut translated=TranslatedStream::new(String::new());
        let mut done=false;
        'reads: while let Some(read)=bytes.next().await {
            let read=match read {Ok(value)=>value,Err(error)=> {yield Ok::<Event,Infallible>(error_event(&error.to_string()));return;}};
            for byte in read {
                if byte!=b'\n' {
                    partial.push(byte);
                    if partial.len()>2*1024*1024 {yield Ok(error_event("Translated Messages SSE frame exceeds local limit"));return;}
                    continue;
                }
                let line=std::mem::take(&mut partial);
                let line=line.strip_suffix(b"\r").unwrap_or(&line);
                let line=match std::str::from_utf8(line) {Ok(value)=>value,Err(_)=> {yield Ok(error_event("Invalid translated SSE UTF-8"));return;}};
                let Some(data)=line.strip_prefix("data:") else {continue;};
                let data=data.trim_start();
                if data=="[DONE]" {done=true;break 'reads;}
                let chunk=match serde_json::from_str::<lr_providers::CompletionChunk>(data) {
                    Ok(value)=>value,Err(_)=> {yield Ok(error_event("Chat orchestrator returned an error or invalid stream chunk"));return;}
                };
                match translated.push(&chunk) {
                    Ok(values)=>for value in values {yield Ok(event(value));},
                    Err(error)=> {yield Ok(error_event(&error));return;}
                }
            }
        }
        if !done {yield Ok(error_event("Chat orchestrator stream ended before completion"));return;}
        match translated.finish() {
            Ok(values)=>for value in values {yield Ok(event(value));},
            Err(error)=>yield Ok(error_event(&error)),
        }
    };
    Sse::new(events).into_response()
}

struct ToolState {
    block: Option<u32>,
    id: String,
    name: String,
}
struct TranslatedStream {
    id: String,
    model: String,
    started: bool,
    next: u32,
    text: Option<u32>,
    tools: BTreeMap<u32, ToolState>,
    usage: Option<lr_providers::TokenUsage>,
    finish: Option<String>,
}
impl TranslatedStream {
    fn new(model: String) -> Self {
        Self {
            id: format!("msg_{}", uuid::Uuid::new_v4().simple()),
            model,
            started: false,
            next: 0,
            text: None,
            tools: BTreeMap::new(),
            usage: None,
            finish: None,
        }
    }
    fn block(&mut self) -> u32 {
        let next = self.next;
        self.next += 1;
        next
    }
    fn push(&mut self, chunk: &lr_providers::CompletionChunk) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        if let Some(usage) = &chunk.usage {
            self.usage = Some(usage.clone());
        }
        if !self.started {
            self.started = true;
            if !chunk.model.is_empty() {
                self.model = chunk.model.clone();
            }
            out.push(json!({"type":"message_start","message":{"id":self.id,"type":"message","role":"assistant","model":self.model,"content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0}}}));
        }
        for choice in &chunk.choices {
            if choice.index != 0 {
                return Err("Messages translation requires a single completion".into());
            }
            if let Some(reason) = &choice.finish_reason {
                self.finish = Some(reason.clone());
            }
            if let Some(text) = &choice.delta.content {
                if !text.is_empty() {
                    let index = if let Some(index) = self.text {
                        index
                    } else {
                        let index = self.block();
                        self.text = Some(index);
                        out.push(json!({"type":"content_block_start","index":index,"content_block":{"type":"text","text":""}}));
                        index
                    };
                    out.push(json!({"type":"content_block_delta","index":index,"delta":{"type":"text_delta","text":text}}));
                }
            }
            for delta in choice.delta.tool_calls.iter().flatten() {
                if !self.tools.contains_key(&delta.index) {
                    if self.tools.len() >= 1024 {
                        return Err("Translated Messages exceeds 1024 tool calls".into());
                    }
                    self.tools.insert(
                        delta.index,
                        ToolState {
                            block: None,
                            id: String::new(),
                            name: String::new(),
                        },
                    );
                }
                let next = &mut self.next;
                let tool = self.tools.get_mut(&delta.index).unwrap();
                if let Some(id) = &delta.id {
                    if tool.block.is_some() && !id.is_empty() && id != &tool.id {
                        return Err("Tool identity changed after native block start".into());
                    }
                    if tool.block.is_none() {
                        tool.id.push_str(id);
                    }
                }
                if let Some(function) = &delta.function {
                    if let Some(name) = &function.name {
                        if tool.block.is_some() && !name.is_empty() {
                            return Err("Tool name changed after native block start".into());
                        }
                        tool.name.push_str(name);
                    }
                    if tool.id.len() > 8192 || tool.name.len() > 8192 {
                        return Err("Translated tool identity exceeds local limit".into());
                    }
                    if let Some(arguments) = &function.arguments {
                        if tool.block.is_none() {
                            if tool.id.is_empty() || tool.name.is_empty() {
                                return Err("Tool arguments arrived before their identity".into());
                            }
                            tool.block = Some(*next);
                            *next += 1;
                            out.push(json!({"type":"content_block_start","index":tool.block,"content_block":{"type":"tool_use","id":tool.id,"name":tool.name,"input":{}}}));
                        }
                        if !arguments.is_empty() {
                            out.push(json!({"type":"content_block_delta","index":tool.block,"delta":{"type":"input_json_delta","partial_json":arguments}}));
                        }
                    }
                }
            }
        }
        Ok(out)
    }
    fn finish(mut self) -> Result<Vec<Value>, String> {
        if !self.started {
            return Err("Translated Messages stream returned no message".into());
        }
        let stop = match self.finish.as_deref() {
            Some("stop") => "end_turn",
            Some("length") => "max_tokens",
            Some("tool_calls") => "tool_use",
            _ => {
                return Err(
                    "Translated Messages stream ended without a supported finish reason".into(),
                )
            }
        };
        let mut out = Vec::new();
        if let Some(index) = self.text {
            out.push(json!({"type":"content_block_stop","index":index}));
        }
        for tool in self.tools.values_mut() {
            if tool.block.is_none() {
                if tool.id.is_empty() || tool.name.is_empty() {
                    return Err("Incomplete translated tool identity".into());
                }
                tool.block = Some(self.next);
                self.next += 1;
                out.push(json!({"type":"content_block_start","index":tool.block,"content_block":{"type":"tool_use","id":tool.id,"name":tool.name,"input":{}}}));
            }
            out.push(json!({"type":"content_block_stop","index":tool.block}));
        }
        let mut usage = json!({});
        if let Some(reported) = self.usage {
            let read = reported
                .prompt_tokens_details
                .as_ref()
                .and_then(|v| v.cache_read_tokens.or(v.cached_tokens))
                .unwrap_or(0);
            let write = reported
                .prompt_tokens_details
                .as_ref()
                .and_then(|v| v.cache_creation_tokens)
                .unwrap_or(0);
            usage = json!({"input_tokens":reported.prompt_tokens.saturating_sub(read).saturating_sub(write),"output_tokens":reported.completion_tokens,"cache_read_input_tokens":read,"cache_creation_input_tokens":write});
        }
        out.push(json!({"type":"message_delta","delta":{"stop_reason":stop,"stop_sequence":null},"usage":usage}));
        out.push(json!({"type":"message_stop"}));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chunk(delta: Value, finish: Value) -> lr_providers::CompletionChunk {
        serde_json::from_value(json!({"id":"chat_x","object":"chat.completion.chunk","created":1,"model":"fixture","choices":[{"index":0,"delta":delta,"finish_reason":finish}]})).unwrap()
    }
    #[test]
    fn deferred_tool_names_do_not_reserve_gaps_before_text_blocks() {
        let mut stream = TranslatedStream::new("fixture".into());
        let first = stream
            .push(&chunk(
                json!({"tool_calls":[{"index":0,"id":"call_a","function":{"name":"lookup"}}]}),
                Value::Null,
            ))
            .unwrap();
        assert!(first
            .iter()
            .all(|value| value["type"] != "content_block_start"));
        let text = stream
            .push(&chunk(json!({"content":"Checking"}), Value::Null))
            .unwrap();
        assert_eq!(text[0]["index"], 0);
        let tool=stream.push(&chunk(json!({"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":\"Toronto\"}"}}]}),json!("tool_calls"))).unwrap();
        assert_eq!(tool[0]["index"], 1);
        assert_eq!(tool[0]["content_block"]["id"], "call_a");
        assert_eq!(tool[1]["delta"]["partial_json"], "{\"city\":\"Toronto\"}");
        let tail = stream.finish().unwrap();
        assert_eq!(tail[tail.len() - 2]["delta"]["stop_reason"], "tool_use");
        assert_eq!(tail.last().unwrap()["type"], "message_stop");
    }
    #[test]
    fn interrupted_translation_does_not_synthesize_success() {
        let mut stream = TranslatedStream::new("fixture".into());
        stream
            .push(&chunk(json!({"content":"partial"}), Value::Null))
            .unwrap();
        assert!(stream.finish().is_err());
        let mut stream = TranslatedStream::new("fixture".into());
        stream.push(&chunk(json!({"tool_calls":[{"index":0,"id":"call_a","function":{"name":"lookup","arguments":""}}]}),Value::Null)).unwrap();
        assert!(stream
            .push(&chunk(
                json!({"tool_calls":[{"index":0,"id":"call_b"}]}),
                Value::Null
            ))
            .is_err());
    }
}
