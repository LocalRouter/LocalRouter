//! What a streamed completion produced: text for the monitor, every
//! generated character (content, reasoning, tool calls), the finish reason,
//! the upstream's usage and the provider that served it.
//!
//! Usage is the upstream's own when it reports one (usually on the last
//! chunk, or a usage-only chunk after the finish reason); otherwise an
//! estimate from the whole prompt and all generated characters. A stream
//! that reports usage more than once (cumulative counts) keeps the latest;
//! a response stitched from several upstream streams (MCP via LLM tool
//! loops) sums them, one per chunk id.

use lr_providers::{CompletionChunk, TokenUsage};

use super::finalize::{FinalizeInputs, StreamingFinalizeSummary};

#[derive(Default, Debug)]
pub(crate) struct StreamTracker {
    /// Generated text (for monitor previews).
    pub content: String,
    /// Characters of text, reasoning and tool-call names/arguments.
    output_chars: u64,
    finish_reason: Option<String>,
    /// Latest usage reported per upstream stream (chunk id)
    usage: Vec<(String, TokenUsage)>,
    provider: Option<String>,
    model: Option<String>,
}

/// Usage a provider left in `extensions.usage` (OpenAI wire shape).
fn usage_from_extension(v: &serde_json::Value) -> Option<TokenUsage> {
    let count = |key: &str| v.get(key).and_then(|n| n.as_u64()).map(|n| n as u32);
    let prompt_tokens = count("prompt_tokens")?;
    let completion_tokens = count("completion_tokens").unwrap_or(0);
    let reasoning = v
        .get("completion_tokens_details")
        .and_then(|d| d.get("reasoning_tokens"))
        .and_then(|n| n.as_u64());
    Some(TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
        prompt_tokens_details: None,
        completion_tokens_details: reasoning.map(|r| lr_providers::CompletionTokensDetails {
            reasoning_tokens: Some(r as u32),
            thinking_tokens: None,
            audio_tokens: None,
        }),
    })
}

/// Token totals for a finished stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StreamTotals {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub reasoning_tokens: Option<u32>,
    /// True when the upstream reported no usage and these are estimates.
    pub estimated: bool,
}

impl StreamTracker {
    pub fn observe(&mut self, chunk: &CompletionChunk) {
        for choice in &chunk.choices {
            if let Some(text) = &choice.delta.content {
                self.content.push_str(text);
            }
            if let Some(reason) = &choice.finish_reason {
                self.finish_reason = Some(reason.clone());
            }
        }
        self.output_chars += lr_providers::usage_estimate::chunk_output_chars(chunk);
        let usage = chunk.usage.clone().or_else(|| {
            chunk
                .extensions
                .as_ref()
                .and_then(|ext| ext.get("usage"))
                .and_then(usage_from_extension)
        });
        if let Some(usage) = &usage {
            match self.usage.iter_mut().find(|(id, _)| *id == chunk.id) {
                Some((_, latest)) => *latest = usage.clone(),
                None => self.usage.push((chunk.id.clone(), usage.clone())),
            }
        }
        if self.provider.is_none() {
            self.provider = chunk.provider.clone();
        }
        if self.model.is_none() && !chunk.model.is_empty() {
            self.model = Some(chunk.model.clone());
        }
    }

    /// The model the upstream reported, else `requested`.
    pub fn model(&self, requested: &str) -> String {
        self.model.clone().unwrap_or_else(|| requested.to_string())
    }

    /// A chunk that only carries usage (no choices) is bookkeeping for us,
    /// not something to forward as a content chunk.
    pub fn is_usage_only(chunk: &CompletionChunk) -> bool {
        chunk.choices.is_empty() && chunk.usage.is_some()
    }

    pub fn finish_reason(&self) -> String {
        self.finish_reason
            .clone()
            .unwrap_or_else(|| "stop".to_string())
    }

    /// The provider instance that served the stream: stamped by the router,
    /// else the `provider/` prefix of the model, else `unknown`.
    pub fn provider(&self, model: &str) -> String {
        self.provider
            .clone()
            .filter(|p| !p.is_empty())
            .or_else(|| model.split_once('/').map(|(p, _)| p.to_string()))
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// The upstream's usage, summed over its streams; `None` when it
    /// reported none.
    fn upstream_usage(&self) -> Option<TokenUsage> {
        let mut reports = self.usage.iter().map(|(_, u)| u);
        let first = reports.next()?.clone();
        Some(reports.fold(first, |mut sum, u| {
            sum.prompt_tokens = sum.prompt_tokens.saturating_add(u.prompt_tokens);
            sum.completion_tokens = sum.completion_tokens.saturating_add(u.completion_tokens);
            sum.total_tokens = sum.total_tokens.saturating_add(u.total_tokens);
            let reasoning = |u: &TokenUsage| {
                u.completion_tokens_details
                    .as_ref()
                    .and_then(|d| d.reasoning_tokens)
            };
            if let Some(r) = reasoning(u) {
                let details = sum.completion_tokens_details.get_or_insert(
                    lr_providers::CompletionTokensDetails {
                        reasoning_tokens: None,
                        thinking_tokens: None,
                        audio_tokens: None,
                    },
                );
                details.reasoning_tokens = Some(details.reasoning_tokens.unwrap_or(0) + r);
            }
            let cached = |u: &TokenUsage| {
                u.prompt_tokens_details
                    .as_ref()
                    .and_then(|d| d.cached_tokens)
            };
            if let Some(c) = cached(u) {
                let details =
                    sum.prompt_tokens_details
                        .get_or_insert(lr_providers::PromptTokensDetails {
                            cached_tokens: None,
                            cache_creation_tokens: None,
                            cache_read_tokens: None,
                        });
                details.cached_tokens = Some(details.cached_tokens.unwrap_or(0) + c);
            }
            sum
        }))
    }

    pub fn totals(&self, prompt_estimate: u64) -> StreamTotals {
        match self.upstream_usage() {
            Some(u) => StreamTotals {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                reasoning_tokens: u
                    .completion_tokens_details
                    .as_ref()
                    .and_then(|d| d.reasoning_tokens.or(d.thinking_tokens)),
                estimated: false,
            },
            None => StreamTotals {
                prompt_tokens: prompt_estimate.min(u32::MAX as u64) as u32,
                completion_tokens: lr_providers::usage_estimate::chars_to_tokens(self.output_chars)
                    .min(u32::MAX as u64) as u32,
                reasoning_tokens: None,
                estimated: true,
            },
        }
    }

    /// The usage to report to a client that asked for it
    /// (`stream_options.include_usage`).
    pub fn client_usage(&self, prompt_estimate: u64) -> crate::types::TokenUsage {
        let t = self.totals(prompt_estimate);
        let upstream = self.upstream_usage();
        crate::types::TokenUsage {
            prompt_tokens: t.prompt_tokens,
            completion_tokens: t.completion_tokens,
            total_tokens: t.prompt_tokens.saturating_add(t.completion_tokens),
            prompt_tokens_details: upstream
                .as_ref()
                .and_then(|u| u.prompt_tokens_details.clone()),
            completion_tokens_details: upstream.and_then(|u| u.completion_tokens_details),
        }
    }
}

/// Record a finished (or abandoned) stream: cost, metrics, access log,
/// monitor event and generation tracker. `inputs.prompt_tokens` is
/// overwritten with the stream's totals.
pub(crate) async fn finalize_stream(
    mut inputs: FinalizeInputs<'_>,
    requested_model: &str,
    tracked: &StreamTracker,
    prompt_estimate: u64,
) {
    let totals = tracked.totals(prompt_estimate);
    inputs.prompt_tokens = totals.prompt_tokens;
    let finish_reason = tracked.finish_reason();
    // Chunks carry `provider/model`; pricing and metrics use the bare model
    // with the provider alongside, like non-streaming responses
    let provider = tracked.provider(requested_model);
    let model = tracked.model(requested_model);
    let model = model
        .strip_prefix(&format!("{provider}/"))
        .map(str::to_string)
        .unwrap_or(model);
    let wire_body = super::monitor_helpers::build_streaming_response_body(
        inputs.generation_id,
        requested_model,
        &tracked.content,
        &finish_reason,
        totals.prompt_tokens as u64,
        totals.completion_tokens as u64,
        inputs.created_at.timestamp(),
    );
    super::finalize::finalize_streaming_at_end(
        &inputs,
        StreamingFinalizeSummary {
            provider,
            model,
            prompt_tokens: totals.prompt_tokens,
            completion_tokens: totals.completion_tokens,
            reasoning_tokens: totals.reasoning_tokens.map(u64::from),
            finish_reason: Some(finish_reason),
            content_preview: tracked.content.clone(),
        },
        &wire_body,
    )
    .await;
}

/// What `finalize_stream` needs, owned, for a stream whose client may
/// disconnect before the end.
pub(crate) struct AbandonedStream {
    pub state: crate::state::AppState,
    pub auth: crate::state::AuthContext,
    pub llm_event_id: String,
    pub generation_id: String,
    pub started_at: std::time::Instant,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub compression_tokens_saved: u64,
    pub model: String,
    pub prompt_estimate: u64,
}

/// Finalizes the stream from what it produced so far if it is dropped
/// before `disarm` (the client went away mid-stream).
pub(crate) struct FinalizeOnDrop {
    ctx: Option<AbandonedStream>,
    tracker: std::sync::Arc<parking_lot::Mutex<StreamTracker>>,
}

impl FinalizeOnDrop {
    pub fn new(
        ctx: AbandonedStream,
        tracker: std::sync::Arc<parking_lot::Mutex<StreamTracker>>,
    ) -> Self {
        Self {
            ctx: Some(ctx),
            tracker,
        }
    }

    /// The stream finished and was finalized by its owner.
    pub fn disarm(&mut self) {
        self.ctx = None;
    }
}

impl Drop for FinalizeOnDrop {
    fn drop(&mut self) {
        let Some(ctx) = self.ctx.take() else {
            return;
        };
        let tracked = std::mem::take(&mut *self.tracker.lock());
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        runtime.spawn(async move {
            finalize_stream(
                FinalizeInputs {
                    state: &ctx.state,
                    auth: &ctx.auth,
                    llm_event_id: &ctx.llm_event_id,
                    generation_id: &ctx.generation_id,
                    started_at: ctx.started_at,
                    created_at: ctx.created_at,
                    prompt_tokens: 0, // from the stream
                    compression_tokens_saved: ctx.compression_tokens_saved,
                    routing_metadata: None,
                    user: None,
                    streamed: true,
                    skip_monitor_completion: false,
                },
                &ctx.model,
                &tracked,
                ctx.prompt_estimate,
            )
            .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_providers::{ChunkChoice, ChunkDelta, FunctionCallDelta, ToolCallDelta};

    fn chunk(content: Option<&str>, finish: Option<&str>) -> CompletionChunk {
        CompletionChunk {
            id: "c".into(),
            object: "chat.completion.chunk".into(),
            created: 0,
            model: "m".into(),
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    role: None,
                    content: content.map(str::to_string),
                    tool_calls: None,
                    reasoning_content: None,
                },
                finish_reason: finish.map(str::to_string),
            }],
            extensions: None,
            usage: None,
            provider: Some("chatgpt".into()),
        }
    }

    fn usage(prompt: u32, completion: u32) -> TokenUsage {
        TokenUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt + completion,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        }
    }

    #[test]
    fn upstream_usage_after_the_finish_reason_wins() {
        let mut t = StreamTracker::default();
        t.observe(&chunk(Some("Hello"), None));
        t.observe(&chunk(None, Some("stop")));
        let mut tail = chunk(None, None);
        tail.choices.clear();
        tail.usage = Some(usage(1200, 34));
        assert!(StreamTracker::is_usage_only(&tail));
        t.observe(&tail);
        let totals = t.totals(5);
        assert_eq!((totals.prompt_tokens, totals.completion_tokens), (1200, 34));
        assert!(!totals.estimated);
        assert_eq!(t.content, "Hello");
        assert_eq!(t.finish_reason(), "stop");
        assert_eq!(t.client_usage(5).total_tokens, 1234);
    }

    #[test]
    fn cumulative_reports_keep_the_latest_and_streams_add_up() {
        let mut t = StreamTracker::default();
        let mut a = chunk(Some("a"), None);
        a.usage = Some(usage(100, 1));
        t.observe(&a);
        a.usage = Some(usage(100, 5));
        t.observe(&a);
        let mut b = chunk(None, Some("stop"));
        b.id = "second".into();
        b.usage = Some(usage(150, 7));
        t.observe(&b);
        let totals = t.totals(0);
        assert_eq!((totals.prompt_tokens, totals.completion_tokens), (250, 12));
    }

    #[test]
    fn tool_call_only_turn_is_estimated_from_its_arguments() {
        let mut t = StreamTracker::default();
        let mut c = chunk(None, Some("tool_calls"));
        c.choices[0].delta.tool_calls = Some(vec![ToolCallDelta {
            index: 0,
            id: Some("call_1".into()),
            tool_type: Some("function".into()),
            function: Some(FunctionCallDelta {
                name: Some("record".into()),
                arguments: Some("x".repeat(394)),
            }),
        }]);
        t.observe(&c);
        let totals = t.totals(900);
        assert!(totals.estimated);
        assert_eq!(totals.prompt_tokens, 900);
        // (6 + 394) chars / 4
        assert_eq!(totals.completion_tokens, 100);
        assert_eq!(t.finish_reason(), "tool_calls");
    }

    #[test]
    fn model_is_the_upstream_one_else_the_requested_one() {
        let mut t = StreamTracker::default();
        assert_eq!(t.model("auto"), "auto");
        let mut c = chunk(Some("x"), None);
        c.model = "chatgpt/gpt-5.5".into();
        t.observe(&c);
        assert_eq!(t.model("auto"), "chatgpt/gpt-5.5");
    }

    #[test]
    fn extension_usage_counts_when_the_chunk_has_none() {
        let mut t = StreamTracker::default();
        let mut c = chunk(Some("x"), Some("stop"));
        c.extensions = Some(
            [(
                "usage".to_string(),
                serde_json::json!({"prompt_tokens": 40, "completion_tokens": 2,
                    "completion_tokens_details": {"reasoning_tokens": 1}}),
            )]
            .into_iter()
            .collect(),
        );
        t.observe(&c);
        let totals = t.totals(0);
        assert_eq!(
            (
                totals.prompt_tokens,
                totals.completion_tokens,
                totals.reasoning_tokens
            ),
            (40, 2, Some(1))
        );
    }

    #[test]
    fn provider_comes_from_the_router_then_the_model_prefix() {
        let mut t = StreamTracker::default();
        t.observe(&chunk(Some("x"), None));
        assert_eq!(t.provider("gpt-5.5"), "chatgpt");
        let mut none = StreamTracker::default();
        let mut c = chunk(Some("x"), None);
        c.provider = None;
        none.observe(&c);
        assert_eq!(none.provider("openai/gpt-4o"), "openai");
        assert_eq!(none.provider("gpt-4o"), "unknown");
    }
}
