//! Upstream token usage on streaming chat completions.
//!
//! OpenAI-shaped upstreams report usage in different places: a usage-only
//! chunk with empty `choices` after the finish chunk (OpenAI with
//! `stream_options.include_usage`), `usage` on the finish chunk
//! (OpenRouter, Together, llama.cpp, vLLM), cumulative `usage` on every
//! chunk (Perplexity), or `x_groq.usage` (Groq). The helpers here parse all
//! of those and normalize the stream so the final cumulative usage arrives
//! exactly once, on an extra chunk with empty `choices` after the upstream
//! stream ends.

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use futures::stream::{Stream, StreamExt};
use serde::Serialize;
use serde_json::Value;

use crate::{CompletionChunk, CompletionTokensDetails, PromptTokensDetails, TokenUsage};
use lr_types::{AppError, AppResult};

/// Provider types (factory `provider_type()` ids) whose chat completions
/// endpoint accepts `stream_options: {"include_usage": true}`. Upstreams not
/// listed here may reject unknown request fields (Mistral answers 422
/// `extra_forbidden`), so their usage is only parsed, never asked for.
/// Cerebras, DeepInfra, xAI, Together and Perplexity do not document the
/// field but report usage on their own.
const INCLUDE_USAGE_PROVIDER_TYPES: &[&str] = &[
    // platform.openai.com/docs/api-reference/chat/create (stream_options)
    "openai",
    // console.groq.com/docs/api-reference (stream_options)
    "groq",
    // openrouter.ai/docs/use-cases/usage-accounting: accepted; usage is
    // always sent in the last SSE message anyway.
    "openrouter",
    // llama.cpp tools/server/server-task.cpp: the usage chunk is only sent
    // when `include_usage` is set.
    "llamacpp",
    // The bundled llama-server (same server as above).
    "llamacpp_embedded",
    // huggingface.co/docs/inference-providers/tasks/chat-completion
    // (stream_options.include_usage)
    "huggingface",
    // docs.digitalocean.com/reference/pydo/reference/inference/create_chat_completion
    "digitalocean",
    // theopenco/llmgateway apps/gateway/src/fallback.spec.ts sends it and
    // expects 200; usage arrives in a final chunk before [DONE].
    "llmgateway",
    // opencode packages/console/app/src/routes/zen/util/provider/
    // openai-compatible.ts forwards the body and sets include_usage itself.
    "opencode_zen",
    "opencode_go",
];

/// Whether `provider_type` accepts `stream_options.include_usage`.
pub(crate) fn accepts_include_usage(provider_type: &str) -> bool {
    INCLUDE_USAGE_PROVIDER_TYPES.contains(&provider_type)
}

/// Serialize a streaming chat request body, adding
/// `stream_options.include_usage: true` when `provider_type` accepts it and
/// the body asks for a stream. An existing `stream_options` object is kept
/// and extended.
pub(crate) fn streaming_body<T: Serialize>(request: &T, provider_type: &str) -> AppResult<Value> {
    let mut body = serde_json::to_value(request)
        .map_err(|e| AppError::Provider(format!("Failed to serialize request: {}", e)))?;
    let streaming = body.get("stream").and_then(Value::as_bool) == Some(true);
    if streaming && accepts_include_usage(provider_type) {
        if let Some(obj) = body.as_object_mut() {
            let options = obj
                .entry("stream_options")
                .or_insert_with(|| Value::Object(Default::default()));
            if !options.is_object() {
                *options = Value::Object(Default::default());
            }
            if let Some(options) = options.as_object_mut() {
                options.insert("include_usage".to_string(), Value::Bool(true));
            }
        }
    }
    Ok(body)
}

fn count(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_u64)
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
}

/// Parse an OpenAI-shaped `usage` object. Returns `None` for `null`, a
/// non-object, or an object with neither `prompt_tokens` nor
/// `completion_tokens`. A missing or zero `total_tokens` is computed.
pub(crate) fn parse_openai_usage(value: &Value) -> Option<TokenUsage> {
    let obj = value.as_object()?;
    let prompt = count(obj.get("prompt_tokens"));
    let completion = count(obj.get("completion_tokens"));
    if prompt.is_none() && completion.is_none() {
        return None;
    }
    let prompt_tokens = prompt.unwrap_or(0);
    let completion_tokens = completion.unwrap_or(0);
    let total_tokens = count(obj.get("total_tokens"))
        .filter(|t| *t > 0)
        .unwrap_or_else(|| prompt_tokens.saturating_add(completion_tokens));

    let cached = obj
        .get("prompt_tokens_details")
        .and_then(|d| count(d.get("cached_tokens")));
    let reasoning = obj
        .get("completion_tokens_details")
        .and_then(|d| count(d.get("reasoning_tokens")));

    Some(TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        prompt_tokens_details: cached.map(|cached| PromptTokensDetails {
            cached_tokens: Some(cached),
            cache_creation_tokens: None,
            cache_read_tokens: None,
        }),
        completion_tokens_details: reasoning.map(|reasoning| CompletionTokensDetails {
            reasoning_tokens: Some(reasoning),
            thinking_tokens: None,
            audio_tokens: None,
        }),
    })
}

/// Usage on an OpenAI-shaped stream chunk: top-level `usage`, else Groq's
/// `x_groq.usage`.
pub(crate) fn chunk_usage(usage: Option<&Value>, x_groq: Option<&Value>) -> Option<TokenUsage> {
    usage.and_then(parse_openai_usage).or_else(|| {
        x_groq
            .and_then(|x| x.get("usage"))
            .and_then(parse_openai_usage)
    })
}

/// Make upstream usage arrive at most once per stream, with the final
/// cumulative numbers.
///
/// `usage` is taken off every chunk that carries it; the latest value wins.
/// Chunks that carried usage and nothing else (empty `choices`, no
/// extensions) are dropped. When the upstream stream ends, one chunk with
/// empty `choices` and the latest `usage` is emitted. All other chunks pass
/// through unchanged.
pub(crate) fn usage_once_at_end<S>(
    stream: S,
) -> Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>
where
    S: Stream<Item = AppResult<CompletionChunk>> + Send + 'static,
{
    let latest: Arc<Mutex<Option<CompletionChunk>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&latest);

    let body = stream.filter_map(move |item| {
        let sink = Arc::clone(&sink);
        async move {
            let mut chunk = match item {
                Ok(chunk) => chunk,
                Err(e) => return Some(Err(e)),
            };
            let Some(usage) = chunk.usage.take() else {
                return Some(Ok(chunk));
            };
            *sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(CompletionChunk {
                id: chunk.id.clone(),
                object: chunk.object.clone(),
                created: chunk.created,
                model: chunk.model.clone(),
                choices: Vec::new(),
                extensions: None,
                usage: Some(usage),
                provider: None,
            });
            if chunk.choices.is_empty() && chunk.extensions.is_none() {
                None
            } else {
                Some(Ok(chunk))
            }
        }
    });

    let tail =
        futures::stream::once(
            async move { latest.lock().unwrap_or_else(|e| e.into_inner()).take() },
        )
        .filter_map(|chunk| async move { chunk.map(Ok) });

    Box::pin(body.chain(tail))
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Shared fixtures for provider streaming-usage tests.

    use super::*;
    use crate::CompletionRequest;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A streaming request for `model` with one user message.
    pub(crate) fn stream_request(model: &str) -> CompletionRequest {
        serde_json::from_value(serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true,
        }))
        .expect("valid request")
    }

    /// SSE body with one `data:` frame per JSON payload, then `[DONE]`.
    pub(crate) fn sse(frames: &[&str]) -> String {
        let mut out = String::new();
        for frame in frames {
            out.push_str("data: ");
            out.push_str(frame);
            out.push_str("\n\n");
        }
        out.push_str("data: [DONE]\n\n");
        out
    }

    /// A server answering `POST route` with `body` as `content_type`.
    pub(crate) async fn stream_server(route: &str, body: String, content_type: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, content_type))
            .mount(&server)
            .await;
        server
    }

    /// A server answering `POST route` with an SSE `body`.
    pub(crate) async fn sse_server(route: &str, body: String) -> MockServer {
        stream_server(route, body, "text/event-stream").await
    }

    /// JSON body of the single request `server` received.
    pub(crate) async fn received_body(server: &MockServer) -> Value {
        let requests = server.received_requests().await.expect("recording on");
        assert_eq!(requests.len(), 1, "exactly one upstream request");
        serde_json::from_slice(&requests[0].body).expect("JSON request body")
    }

    /// Drain a chunk stream, failing on any error item.
    pub(crate) async fn collect(
        mut stream: Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>,
    ) -> Vec<CompletionChunk> {
        let mut out = Vec::new();
        while let Some(item) = stream.next().await {
            out.push(item.expect("stream item"));
        }
        out
    }

    /// Asserts that exactly one chunk carries usage, that it is the last
    /// chunk, and returns it.
    pub(crate) fn single_trailing_usage(chunks: &[CompletionChunk]) -> TokenUsage {
        let with_usage: Vec<usize> = chunks
            .iter()
            .enumerate()
            .filter(|(_, c)| c.usage.is_some())
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            with_usage,
            vec![chunks.len() - 1],
            "usage exactly once, last"
        );
        chunks.last().unwrap().usage.clone().unwrap()
    }

    /// Concatenated `delta.content` of all chunks.
    pub(crate) fn content(chunks: &[CompletionChunk]) -> String {
        chunks
            .iter()
            .flat_map(|c| c.choices.iter())
            .filter_map(|c| c.delta.content.as_deref())
            .collect()
    }

    /// Every `finish_reason` in stream order.
    pub(crate) fn finish_reasons(chunks: &[CompletionChunk]) -> Vec<String> {
        chunks
            .iter()
            .flat_map(|c| c.choices.iter())
            .filter_map(|c| c.finish_reason.clone())
            .collect()
    }

    /// Where an OpenAI-shaped upstream puts its usage.
    pub(crate) enum UsageAt {
        /// OpenAI with `include_usage`: `usage: null` on every chunk, then
        /// a usage-only chunk with empty `choices` after the finish chunk.
        UsageOnlyChunk,
        /// On the finish chunk (OpenRouter, Together, DeepInfra, …).
        FinishChunk,
        /// Cumulative on every chunk (Perplexity, xAI).
        EveryChunk,
        /// Groq: `x_groq.usage` on the finish chunk.
        XGroq,
    }

    const FULL_USAGE: &str = r#"{"prompt_tokens":12,"completion_tokens":5,"total_tokens":17,"prompt_tokens_details":{"cached_tokens":4},"completion_tokens_details":{"reasoning_tokens":2}}"#;

    /// A chat completions stream: role chunk, "Hello", " world", finish
    /// chunk, with usage placed per `at`. Final usage: 12 prompt (4
    /// cached), 5 completion (2 reasoning), 17 total.
    pub(crate) fn openai_stream(at: UsageAt) -> String {
        let partial = |n: u32| {
            format!(
                r#","usage":{{"prompt_tokens":12,"completion_tokens":{n},"total_tokens":{}}}"#,
                12 + n
            )
        };
        let (on_content, on_finish, trailer): (Vec<String>, String, Option<String>) = match at {
            UsageAt::UsageOnlyChunk => (
                vec![r#","usage":null"#.to_string(); 3],
                r#","usage":null"#.to_string(),
                Some(format!(
                    r#"{{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1700000000,"model":"test-model","choices":[],"usage":{FULL_USAGE}}}"#
                )),
            ),
            UsageAt::FinishChunk => (
                vec![String::new(); 3],
                format!(r#","usage":{FULL_USAGE}"#),
                None,
            ),
            UsageAt::EveryChunk => (
                vec![partial(0), partial(1), partial(3)],
                format!(r#","usage":{FULL_USAGE}"#),
                None,
            ),
            UsageAt::XGroq => (
                vec![String::new(); 3],
                format!(r#","x_groq":{{"id":"req_01","usage":{FULL_USAGE}}}"#),
                None,
            ),
        };
        let frame = |delta: &str, finish: &str, tail: &str| {
            format!(
                r#"{{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1700000000,"model":"test-model","choices":[{{"index":0,"delta":{delta},"finish_reason":{finish}}}]{tail}}}"#
            )
        };
        let mut frames = vec![
            frame(
                r#"{"role":"assistant","content":""}"#,
                "null",
                &on_content[0],
            ),
            frame(r#"{"content":"Hello"}"#, "null", &on_content[1]),
            frame(r#"{"content":" world"}"#, "null", &on_content[2]),
            frame("{}", r#""stop""#, &on_finish),
        ];
        frames.extend(trailer);
        let refs: Vec<&str> = frames.iter().map(String::as_str).collect();
        sse(&refs)
    }

    /// The stream from [`openai_stream`] came through with its four
    /// content/finish chunks unchanged, followed by one usage-only chunk.
    pub(crate) fn assert_openai_stream(chunks: &[CompletionChunk]) {
        assert_eq!(chunks.len(), 5, "role, 2 content, finish, usage");
        assert_eq!(
            chunks[0].choices[0].delta.role.as_deref(),
            Some("assistant")
        );
        assert_eq!(content(chunks), "Hello world");
        assert_eq!(finish_reasons(chunks), vec!["stop"]);
        assert!(chunks[..4].iter().all(|c| c.choices.len() == 1));
        let last = chunks.last().unwrap();
        assert!(last.choices.is_empty());
        assert_eq!(last.id, "chatcmpl-1");
        assert_eq!(last.model, "test-model");
        let usage = single_trailing_usage(chunks);
        assert_eq!(
            (
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.total_tokens
            ),
            (12, 5, 17)
        );
        assert_eq!(usage.prompt_tokens_details.unwrap().cached_tokens, Some(4));
        assert_eq!(
            usage.completion_tokens_details.unwrap().reasoning_tokens,
            Some(2)
        );
    }

    /// Whether the upstream request asked for streaming usage.
    pub(crate) fn asked_for_usage(body: &Value) -> bool {
        body.pointer("/stream_options/include_usage") == Some(&Value::Bool(true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn include_usage_is_asked_only_where_documented() {
        for accepted in [
            "openai",
            "groq",
            "openrouter",
            "llamacpp",
            "llamacpp_embedded",
            "huggingface",
            "digitalocean",
            "llmgateway",
            "opencode_zen",
            "opencode_go",
        ] {
            assert!(accepts_include_usage(accepted), "{accepted}");
        }
        // Mistral rejects unknown fields; the rest are undocumented
        for not_asked in [
            "mistral",
            "openai_compatible",
            "nvidia_nim",
            "vercel_ai_gateway",
            "cloudflare_ai",
            "zhipu",
            "lmstudio",
        ] {
            assert!(!accepts_include_usage(not_asked), "{not_asked}");
        }
    }
    use crate::{ChunkChoice, ChunkDelta};
    use serde_json::json;

    fn chunk(
        content: Option<&str>,
        finish: Option<&str>,
        usage: Option<TokenUsage>,
    ) -> CompletionChunk {
        CompletionChunk {
            id: "c1".into(),
            object: "chat.completion.chunk".into(),
            created: 1,
            model: "m".into(),
            choices: if content.is_none() && finish.is_none() {
                vec![]
            } else {
                vec![ChunkChoice {
                    index: 0,
                    delta: ChunkDelta {
                        role: None,
                        content: content.map(str::to_string),
                        tool_calls: None,
                        reasoning_content: None,
                    },
                    finish_reason: finish.map(str::to_string),
                }]
            },
            extensions: None,
            usage,
            provider: None,
        }
    }

    fn usage(prompt: u32, completion: u32) -> TokenUsage {
        parse_openai_usage(&json!({"prompt_tokens": prompt, "completion_tokens": completion}))
            .unwrap()
    }

    async fn run(chunks: Vec<CompletionChunk>) -> Vec<CompletionChunk> {
        let stream = futures::stream::iter(chunks.into_iter().map(Ok));
        test_support::collect(usage_once_at_end(stream)).await
    }

    #[test]
    fn parses_full_openai_usage() {
        let u = parse_openai_usage(&json!({
            "prompt_tokens": 120,
            "completion_tokens": 30,
            "total_tokens": 150,
            "prompt_tokens_details": {"cached_tokens": 100, "audio_tokens": 0},
            "completion_tokens_details": {"reasoning_tokens": 12, "accepted_prediction_tokens": 0}
        }))
        .unwrap();
        assert_eq!(
            (u.prompt_tokens, u.completion_tokens, u.total_tokens),
            (120, 30, 150)
        );
        assert_eq!(u.prompt_tokens_details.unwrap().cached_tokens, Some(100));
        assert_eq!(
            u.completion_tokens_details.unwrap().reasoning_tokens,
            Some(12)
        );
    }

    #[test]
    fn computes_missing_total_and_rejects_non_usage() {
        let u = parse_openai_usage(&json!({"prompt_tokens": 3, "completion_tokens": 4})).unwrap();
        assert_eq!(u.total_tokens, 7);
        assert!(u.prompt_tokens_details.is_none());
        assert!(u.completion_tokens_details.is_none());
        assert!(parse_openai_usage(&Value::Null).is_none());
        assert!(parse_openai_usage(&json!({})).is_none());
        assert!(parse_openai_usage(&json!({"queue_time": 0.1})).is_none());
    }

    #[test]
    fn chunk_usage_falls_back_to_x_groq() {
        let groq = json!({"id": "req_1", "usage": {"prompt_tokens": 9, "completion_tokens": 2, "total_tokens": 11}});
        let u = chunk_usage(None, Some(&groq)).unwrap();
        assert_eq!(u.total_tokens, 11);
        let top = json!({"prompt_tokens": 1, "completion_tokens": 1});
        assert_eq!(
            chunk_usage(Some(&top), Some(&groq)).unwrap().total_tokens,
            2
        );
        assert!(chunk_usage(Some(&Value::Null), None).is_none());
    }

    #[test]
    fn streaming_body_requests_usage_only_for_known_types() {
        let body = json!({"model": "m", "stream": true});
        let with = streaming_body(&body, "openai").unwrap();
        assert_eq!(with["stream_options"], json!({"include_usage": true}));
        let without = streaming_body(&body, "openai_compatible").unwrap();
        assert!(without.get("stream_options").is_none());
        let not_streaming =
            streaming_body(&json!({"model": "m", "stream": false}), "openai").unwrap();
        assert!(not_streaming.get("stream_options").is_none());
        let existing = streaming_body(
            &json!({"stream": true, "stream_options": {"continuous_usage_stats": false}}),
            "groq",
        )
        .unwrap();
        assert_eq!(
            existing["stream_options"],
            json!({"continuous_usage_stats": false, "include_usage": true})
        );
    }

    #[tokio::test]
    async fn usage_only_chunk_moves_to_the_end() {
        let out = run(vec![
            chunk(Some("Hi"), None, None),
            chunk(None, Some("stop"), None),
            chunk(None, None, Some(usage(5, 2))),
        ])
        .await;
        assert_eq!(out.len(), 3);
        assert_eq!(test_support::content(&out), "Hi");
        assert_eq!(test_support::finish_reasons(&out), vec!["stop"]);
        let u = test_support::single_trailing_usage(&out);
        assert_eq!((u.prompt_tokens, u.completion_tokens), (5, 2));
        assert!(out[2].choices.is_empty());
        assert_eq!(out[2].id, "c1");
    }

    #[tokio::test]
    async fn cumulative_usage_on_every_chunk_is_reported_once_with_latest() {
        let out = run(vec![
            chunk(Some("a"), None, Some(usage(5, 1))),
            chunk(Some("b"), None, Some(usage(5, 2))),
            chunk(None, Some("stop"), Some(usage(5, 3))),
        ])
        .await;
        assert_eq!(out.len(), 4, "3 content/finish chunks + 1 usage chunk");
        assert_eq!(test_support::content(&out), "ab");
        let u = test_support::single_trailing_usage(&out);
        assert_eq!(u.completion_tokens, 3);
    }

    #[tokio::test]
    async fn no_usage_means_no_extra_chunk() {
        let out = run(vec![chunk(Some("a"), Some("stop"), None)]).await;
        assert_eq!(out.len(), 1);
        assert!(out[0].usage.is_none());
    }

    #[tokio::test]
    async fn errors_pass_through() {
        let stream = futures::stream::iter(vec![
            Ok(chunk(Some("a"), None, Some(usage(1, 1)))),
            Err(AppError::Provider("boom".into())),
        ]);
        let items: Vec<_> = usage_once_at_end(stream).collect().await;
        assert_eq!(items.len(), 3);
        assert!(items[1].is_err());
        assert!(items[2].as_ref().unwrap().usage.is_some());
    }
}
