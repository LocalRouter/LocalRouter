# Test Audit Report - Bugs and Issues Found

## Critical Issues Found

### 1. ❌ **INVALID TESTS: Ollama Health Check and List Models**

**Location**: `ollama_tests.rs:11-28`

**Issue**: Tests claim to test Ollama health checks and model listing, but they **don't actually test the provider code**.

**Why**: The Ollama provider uses the `ollama-rs` SDK client for these operations:
```rust
// OllamaProvider::health_check() - line 168
match self.sdk_client.list_local_models().await {  // Uses SDK, not HTTP!

// OllamaProvider::list_models() - line 194
let local_models = self.sdk_client.list_local_models().await  // Uses SDK!
```

The mock HTTP server created in the tests **cannot intercept SDK calls**. The SDK makes its own HTTP requests that bypass our mocks.

**Test Code (INVALID)**:
```rust
#[tokio::test]
async fn test_ollama_health_check() {
    let mock = OllamaMockBuilder::new().await.mock_list_models().await;
    let provider = OllamaProvider::with_base_url(mock.base_url());
    let health = provider.health_check().await;
    // This test doesn't actually call the mock!
}
```

**Impact**:
- Tests pass but don't validate the actual code path
- False sense of security
- Health check and model listing bugs would not be caught

**Fix Required**:
1. Either refactor Ollama provider to use direct HTTP calls (like completions)
2. Or remove these invalid tests and mark them as integration tests
3. Or use SDK mocking (requires dependency injection)

---

### 2. ⚠️ **PARTIAL TESTING: SSE Events Split Across Chunks**

**Location**: `sse_scenarios.rs:42-48`

**Issue**: Test explicitly says it can't test events split across HTTP chunks:

```rust
#[tokio::test]
async fn test_sse_event_split_across_chunks() {
    // This test simulates what happens when an SSE event is split across multiple HTTP chunks
    // Note: wiremock sends the entire body at once, so this is a limitation
    // In real scenarios, the streaming implementation should buffer incomplete events
    // This is already handled by the line buffer in Ollama provider
    // OpenAI-compatible provider might need similar buffering
}
```

**Why**: Wiremock limitation - it sends entire response body at once

**Impact**:
- **REAL BUG MAY EXIST**: OpenAI-compatible provider doesn't buffer incomplete lines
- Events split across network chunks could be lost or cause parsing errors
- This is a common real-world scenario

**Evidence of Bug**:
Looking at `openai_compatible.rs:344-398`, the streaming implementation does:
```rust
let stream = response.bytes_stream().flat_map(|result| {
    let chunks: Vec<AppResult<CompletionChunk>> = match result {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            let mut parsed_chunks = Vec::new();

            for line in text.lines() {  // ⚠️ BUG: No buffering of incomplete lines!
                if let Some(json_str) = line.strip_prefix("data: ") {
```

**The Bug**: If an SSE event like `data: {"id":"test"...` is split across two byte chunks:
- Chunk 1: `data: {"id":"te`
- Chunk 2: `st"...}\n\n`

The first chunk will not have a complete line, and `text.lines()` will process the incomplete `data: {"id":"te` as if it's a complete line, causing JSON parsing to fail.

**Fix Required**: Add line buffering like Ollama provider has (lines 333-355)

---

### 3. ❌ **INVALID TESTS: Providers with Hardcoded Base URLs**

**Location**: Multiple test files

**Issue**: Several tests create mock servers but the providers use hardcoded base URLs, so mocks are never called.

#### Anthropic Tests

**Test Code**:
```rust
#[tokio::test]
async fn test_anthropic_completion() {
    let mock = AnthropicMockBuilder::new().await.mock_completion().await;
    // Note: We can't easily test this without modifying AnthropicProvider to accept custom base URL
    // This is a structural test showing how it would work
    // TODO: Refactor AnthropicProvider to accept custom base_url for testing
}
```

**Provider Code** (`anthropic.rs:135`):
```rust
pub const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com";
```

The provider **always** uses the real API URL, never the mock.

**Impact**: Tests don't actually execute any provider code.

#### Similar Issues:
- `groq_tests.rs` - Groq uses hardcoded `https://api.groq.com/openai/v1`
- `openrouter_tests.rs` - OpenRouter uses hardcoded URL
- Several other providers

**Fix Required**: Refactor providers to accept base_url in constructor

---

### 4. ⚠️ **INCOMPLETE VALIDATION: Request Capture Not Verified**

**Location**: `openai_compatible_detailed.rs`

**Issue**: Several tests use request capture but don't verify critical details.

**Example** (`test_request_has_correct_headers`):
```rust
#[tokio::test]
async fn test_request_has_correct_headers() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(...))
        .expect(1)  // ⚠️ Only checks that 1 request was made
        .mount(&mock_server)
        .await;

    // ... make request ...

    // Verification happens via wiremock's expect() - if headers were wrong, test would fail
    // ⚠️ BUT: This doesn't actually verify header values!
}
```

**Issue**: The test relies on wiremock's `.expect(1)` to verify headers, but:
- It only verifies the request count
- It doesn't verify Authorization header value
- It doesn't verify Content-Type value
- The comment is misleading

**Fix**: Actually capture and assert on headers like other tests do.

---

### 5. ⚠️ **FALSE POSITIVE: Empty Choices Test**

**Location**: `http_scenarios.rs:192-224`

**Test Code**:
```rust
#[tokio::test]
async fn test_empty_choices_array() {
    // ... creates response with empty choices array ...

    let result = provider.complete(request).await;

    // This should succeed but have empty choices
    if let Ok(response) = result {
        assert_eq!(response.choices.len(), 0);
    } else {
        // Some providers might error on empty choices
        assert!(result.is_err());
    }
}
```

**Issue**: Test accepts **both success and failure** as valid outcomes. This means:
- Test will always pass
- We don't know the actual behavior
- Different providers might behave differently

**Fix**: Decide on the correct behavior and test for it specifically.

---

### 6. 🐛 **FOUND BUG: OpenAI-Compatible Provider Doesn't Handle Empty Choices**

Let me check the actual provider code:

