//! Shared HTTP client builders for providers
//!
//! Centralizes reqwest client construction with consistent defaults.
//! Uses a generous overall timeout (5 min) so streaming SSE responses
//! are not cut short, plus a fast connect timeout (10 s) for quick
//! failure when a provider is unreachable.

use lr_types::{AppError, AppResult, RequestTrace, TRACE_HEADER};
use reqwest::Client;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware, Middleware, Next};
use std::time::Duration;

tokio::task_local! {
    /// The cross-hop trace to stamp on every upstream request sent while this
    /// task-local is in scope. Set by the router around each provider
    /// dispatch; read by [`TraceHeaderMiddleware`] so no provider has to
    /// thread it through its own request building.
    pub static OUTBOUND_TRACE: Option<RequestTrace>;
}

/// Run `fut` with `trace` as the outbound trace for any upstream HTTP request
/// it sends. `None` sends no header (dedupe disabled).
pub async fn with_outbound_trace<F: std::future::Future>(
    trace: Option<RequestTrace>,
    fut: F,
) -> F::Output {
    OUTBOUND_TRACE.scope(trace, fut).await
}

/// Adds `X-LocalRouter-Trace` to outgoing requests when [`OUTBOUND_TRACE`]
/// is set, so a downstream LocalRouter hop can recognize the request as one
/// this instance already handled.
struct TraceHeaderMiddleware;

#[async_trait::async_trait]
impl Middleware for TraceHeaderMiddleware {
    async fn handle(
        &self,
        mut req: reqwest::Request,
        extensions: &mut http::Extensions,
        next: Next<'_>,
    ) -> reqwest_middleware::Result<reqwest::Response> {
        let trace = OUTBOUND_TRACE.try_with(|t| t.clone()).ok().flatten();
        if let Some(trace) = trace {
            if let Ok(value) = reqwest::header::HeaderValue::from_str(&trace.header_value()) {
                req.headers_mut().insert(TRACE_HEADER, value);
            }
        }
        next.run(req, extensions).await
    }
}

/// Wrap a plain reqwest client with the shared middleware stack.
pub fn with_middleware(client: Client) -> ClientWithMiddleware {
    ClientBuilder::new(client)
        .with(TraceHeaderMiddleware)
        .build()
}

/// Default client for standard API providers.
///
/// * connect_timeout 10 s – fail fast if the host is unreachable
/// * overall timeout 300 s – safety net; long enough for streaming
pub fn default_client() -> ClientWithMiddleware {
    with_middleware(
        Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300))
            .no_gzip()
            .build()
            .unwrap_or_default(),
    )
}

/// Extended-timeout client for providers that may have slower responses.
///
/// Same as `default_client` — the previous 120 s limit was too short for
/// streaming with tool-use payloads. Both now share the 300 s ceiling.
pub fn extended_client() -> AppResult<ClientWithMiddleware> {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .no_gzip()
        .build()
        .map(with_middleware)
        .map_err(|e| AppError::Provider(format!("Failed to create HTTP client: {}", e)))
}

/// Format a reqwest streaming error with the full cause chain.
///
/// `reqwest::Error` Display only shows the top-level kind (e.g.
/// "error decoding response body") but omits the underlying cause.
/// This helper walks the source chain so the log/error message
/// includes the real reason (timeout, connection reset, etc.).
pub fn format_stream_error(e: &reqwest::Error) -> String {
    use std::error::Error;
    let mut msg = format!("Stream error: {}", e);
    let mut source = e.source();
    while let Some(cause) = source {
        msg.push_str(&format!(": {}", cause));
        source = cause.source();
    }
    msg
}

/// Short-timeout client for local service discovery (2s timeout).
pub fn discovery_client() -> ClientWithMiddleware {
    with_middleware(
        Client::builder()
            .timeout(Duration::from_secs(2))
            .no_gzip()
            .build()
            .unwrap_or_default(),
    )
}

/// Parse an OpenAI-style error body and return the best-typed `AppError`.
///
/// OpenAI (and the OpenAI-compatible providers: OpenRouter, DeepInfra,
/// TogetherAI, Groq, openai_compatible) return errors shaped like:
///
/// ```json
/// { "error": { "message": "...", "type": "...", "code": "..." } }
/// ```
///
/// We map `code`/`type` to typed `AppError` variants where possible and
/// parse the numeric context-window out of messages like
/// `"This model's maximum context length is 8192 tokens..."` so callers see
/// the real max instead of a hardcoded zero. When the body is not
/// recognised, falls back to `AppError::Provider("API error (<status>): …")`
/// — the same format the provider code used before, so existing logs/tests
/// continue to match.
pub fn classify_openai_error(status: reqwest::StatusCode, body: &str) -> AppError {
    if let Some(typed) = try_classify_openai_error(status, body) {
        return typed;
    }
    // Status-code fallbacks for bodies we can't parse as JSON.
    match status {
        reqwest::StatusCode::UNAUTHORIZED => AppError::Unauthorized,
        reqwest::StatusCode::TOO_MANY_REQUESTS => AppError::RateLimitExceeded,
        // Preserve an upstream *client* error (4xx) so it passes through to
        // the API response as a 4xx instead of being masked as a generic
        // 502. These are permanent request errors (e.g. an unsupported
        // `max_tokens`/`temperature` param → 400) that clients must not
        // retry as if transient. Server errors (5xx) and everything else
        // stay a generic Provider error → 502.
        s if s.is_client_error() => AppError::ProviderStatus {
            status: s.as_u16(),
            message: format!("API error ({}): {}", s, body),
        },
        _ => AppError::Provider(format!("API error ({}): {}", status, body)),
    }
}

fn try_classify_openai_error(status: reqwest::StatusCode, body: &str) -> Option<AppError> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let err = v.get("error")?;
    let code = err
        .get("code")
        .and_then(|c| c.as_str())
        .map(str::to_ascii_lowercase);
    let etype = err
        .get("type")
        .and_then(|c| c.as_str())
        .map(str::to_ascii_lowercase);
    let message = err.get("message").and_then(|m| m.as_str()).unwrap_or("");

    let is = |s: &str| code.as_deref() == Some(s) || etype.as_deref() == Some(s);

    if is("context_length_exceeded") || is("string_above_max_length") {
        let (max, requested) = parse_context_numbers(message);
        return Some(AppError::ContextLengthExceeded { max, requested });
    }
    if is("model_not_found") || status == reqwest::StatusCode::NOT_FOUND {
        // Try to pull the model id out of the message; fall back to empty.
        let model = extract_model_name(message).unwrap_or_default();
        return Some(AppError::ModelNotFound { model });
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || is("rate_limit_exceeded") {
        return Some(AppError::RateLimitExceeded);
    }
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Some(AppError::Unauthorized);
    }
    None
}

/// Parse `(max, requested)` token counts from OpenAI's context-length error
/// message: `"This model's maximum context length is 8192 tokens. However,
/// your messages resulted in 9100 tokens."`
fn parse_context_numbers(message: &str) -> (Option<u64>, Option<u64>) {
    let lower = message.to_ascii_lowercase();
    let max = find_number_after(&lower, "maximum context length is")
        .or_else(|| find_number_after(&lower, "context length of"))
        .or_else(|| find_number_after(&lower, "context window of"));
    let requested = find_number_after(&lower, "resulted in")
        .or_else(|| find_number_after(&lower, "you requested"))
        .or_else(|| find_number_after(&lower, "input length is"));
    (max, requested)
}

fn find_number_after(haystack: &str, needle: &str) -> Option<u64> {
    let idx = haystack.find(needle)?;
    let tail = &haystack[idx + needle.len()..];
    let digits: String = tail
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Best-effort extraction of a model id from `"The model `foo-bar` does not
/// exist..."`. Returns the string inside backticks, or the token after
/// `"model "` if no backticks are present.
fn extract_model_name(message: &str) -> Option<String> {
    if let Some(start) = message.find('`') {
        let tail = &message[start + 1..];
        if let Some(end) = tail.find('`') {
            return Some(tail[..end].to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn test_classify_openai_context_length() {
        let body = r#"{"error":{"message":"This model's maximum context length is 8192 tokens. However, your messages resulted in 9100 tokens. Please reduce the length of the messages.","type":"invalid_request_error","param":"messages","code":"context_length_exceeded"}}"#;
        let err = classify_openai_error(StatusCode::BAD_REQUEST, body);
        match err {
            AppError::ContextLengthExceeded { max, requested } => {
                assert_eq!(max, Some(8192));
                assert_eq!(requested, Some(9100));
            }
            other => panic!("expected ContextLengthExceeded, got {:?}", other),
        }
    }

    #[test]
    fn test_classify_openai_model_not_found() {
        let body = r#"{"error":{"message":"The model `gpt-99` does not exist or you do not have access to it.","type":"invalid_request_error","code":"model_not_found"}}"#;
        let err = classify_openai_error(StatusCode::NOT_FOUND, body);
        match err {
            AppError::ModelNotFound { model } => assert_eq!(model, "gpt-99"),
            other => panic!("expected ModelNotFound, got {:?}", other),
        }
    }

    #[test]
    fn test_classify_unparseable_body_falls_back() {
        let err = classify_openai_error(StatusCode::INTERNAL_SERVER_ERROR, "gateway down");
        match err {
            AppError::Provider(msg) => assert!(msg.contains("API error (500")),
            other => panic!("expected Provider, got {:?}", other),
        }
    }

    #[test]
    fn test_classify_preserves_upstream_4xx_status() {
        // A 400 for an unsupported param (e.g. `max_tokens`) that we can't
        // map to a more specific typed variant must keep its 400 status via
        // ProviderStatus, so it passes through as a 4xx instead of a 502.
        let err = classify_openai_error(
            StatusCode::BAD_REQUEST,
            "Unsupported parameter: 'max_tokens' is not supported with this model.",
        );
        match err {
            AppError::ProviderStatus { status, message } => {
                assert_eq!(status, 400);
                assert!(message.contains("API error (400"));
            }
            other => panic!("expected ProviderStatus(400), got {:?}", other),
        }

        // 403/422 likewise pass through as client errors.
        assert!(matches!(
            classify_openai_error(StatusCode::FORBIDDEN, "nope"),
            AppError::ProviderStatus { status: 403, .. }
        ));

        // 5xx stays a generic Provider error (→ 502 downstream), NOT a
        // pass-through client status.
        assert!(matches!(
            classify_openai_error(StatusCode::BAD_GATEWAY, "upstream boom"),
            AppError::Provider(_)
        ));
    }

    #[test]
    fn test_classify_rate_limit_by_status() {
        let err = classify_openai_error(StatusCode::TOO_MANY_REQUESTS, "");
        assert!(matches!(err, AppError::RateLimitExceeded));
    }

    #[test]
    fn test_classify_unauthorized_by_status() {
        let err = classify_openai_error(StatusCode::UNAUTHORIZED, "");
        assert!(matches!(err, AppError::Unauthorized));
    }

    #[test]
    fn test_classify_does_not_misclassify_token_auth_error() {
        // A 401 with a message containing the word "token" should NOT be
        // classified as ContextLengthExceeded. This is the exact failure
        // mode of the old substring classifier ("bearer token invalid" →
        // context length) that motivated switching to structured codes.
        let body = r#"{"error":{"message":"Invalid authentication token","type":"invalid_request_error","code":"invalid_api_key"}}"#;
        let err = classify_openai_error(StatusCode::UNAUTHORIZED, body);
        assert!(matches!(err, AppError::Unauthorized));
    }

    #[test]
    fn test_parse_context_numbers_variants() {
        assert_eq!(
            parse_context_numbers("This model's maximum context length is 4096 tokens."),
            (Some(4096), None)
        );
        assert_eq!(
            parse_context_numbers("max context window of 128000 tokens; resulted in 200000"),
            (Some(128_000), Some(200_000))
        );
        assert_eq!(parse_context_numbers("nothing to parse"), (None, None));
    }
}

#[cfg(test)]
mod trace_tests {
    use super::*;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

    /// Minimal one-shot HTTP server that records the trace header it received.
    async fn echo_server() -> (SocketAddr, Arc<Mutex<Vec<Option<String>>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                let seen = seen2.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).to_string();
                    let header = head
                        .lines()
                        .find_map(|l| l.strip_prefix("x-localrouter-trace: "))
                        .map(|v| v.trim().to_string());
                    seen.lock().unwrap().push(header);
                    let _ = sock
                        .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
                        .await;
                });
            }
        });
        (addr, seen)
    }

    #[tokio::test]
    async fn header_is_added_only_inside_scope() {
        let (addr, seen) = echo_server().await;
        let client = default_client();
        let url = format!("http://{addr}/v1/chat/completions");

        client.get(&url).send().await.unwrap();
        let trace = RequestTrace::parse("abc;hop=2").unwrap();
        with_outbound_trace(Some(trace), client.get(&url).send())
            .await
            .unwrap();
        with_outbound_trace(None, client.get(&url).send())
            .await
            .unwrap();

        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen, vec![None, Some("abc;hop=2".to_string()), None]);
    }
}
