# OpenAI API Comparison and Bug Report

**Date**: 2026-01-20
**Status**: Analysis Complete
**Purpose**: Compare LocalRouter AI's OpenAI-compatible endpoints with the official OpenAI API specification and identify bugs

---

## Executive Summary

LocalRouter AI implements **4 of 40+ OpenAI API endpoints**, focusing on core chat/completion functionality. The implementation is functionally correct for the supported endpoints but has several bugs and missing parameters that affect OpenAI compatibility.

**Compatibility Level**: ~15% endpoint coverage, ~70% parameter coverage for implemented endpoints

## Fixes Applied (2026-01-20)

✅ **Bug #1**: Completions streaming now implemented
- Added `handle_streaming` function to `completions.rs`
- Converts chat completion chunks to legacy completion chunks
- Streaming response uses SSE (Server-Sent Events)
- OpenAPI documentation updated to reflect streaming support

✅ **Bug #7**: Response format now passed to provider adapters
- Added `ResponseFormat` enum to provider types
- Added `response_format` field to `CompletionRequest`
- Conversion logic in `chat.rs` to pass format from server to provider
- Providers can now enforce JSON mode and JSON schema

✅ **Bug #8**: `max_completion_tokens` support added
- Prefer `max_completion_tokens` over `max_tokens` when both present
- Supports o-series models properly

✅ **Bug #9**: Extended parameter validation ranges documented
- Added documentation clarifying LocalRouter extensions vs OpenAI standard
- `top_k` marked as LocalRouter extension
- `repetition_penalty` documented with range 0.0-2.0 (LocalRouter constraint)
- `seed` noted as supported by some OpenAI models
- Schema maximum added for `repetition_penalty`

---

## 1. Endpoint Coverage Comparison

### ✅ Implemented Endpoints

| Endpoint | Status | Notes |
|----------|--------|-------|
| `POST /v1/chat/completions` | ✅ Implemented | Streaming + non-streaming support |
| `POST /v1/completions` | ✅ Implemented | Streaming + non-streaming support (Bug #1 FIXED) |
| `POST /v1/embeddings` | ⚠️ Stub | Returns 501 Not Implemented (bug #2) |
| `GET /v1/models` | ✅ Implemented | With strategy-based filtering |
| `GET /v1/models/{id}` | ✅ Implemented | Detailed model info |
| `GET /v1/models/{provider}/{model}/pricing` | ✅ Extension | LocalRouter-specific |

### ❌ Missing Core OpenAI Endpoints

#### High Priority (Common Use Cases)
- `POST /v1/audio/speech` - Text-to-speech
- `POST /v1/audio/transcriptions` - Speech-to-text (Whisper)
- `POST /v1/audio/translations` - Audio translation
- `POST /v1/images/generations` - DALL-E image generation
- `POST /v1/moderations` - Content moderation
- `POST /v1/embeddings` - **CRITICAL**: Needs full implementation

#### Medium Priority (Advanced Features)
- `POST /v1/batch` - Batch processing
- `POST /v1/files` - File upload/management
- `GET /v1/files/{file_id}` - File retrieval
- `DELETE /v1/files/{file_id}` - File deletion

#### Low Priority (Specialized/Deprecated)
- Assistants API endpoints (being deprecated in Aug 2026)
- Fine-tuning endpoints
- Realtime API (WebRTC/WebSocket)
- Responses API (new, for agents)
- Vector stores endpoints
- ChatKit endpoints (beta)

---

## 2. Parameter Coverage Analysis

### Chat Completions (`POST /v1/chat/completions`)

#### ✅ Supported Parameters (18/29)

**Core Parameters:**
- ✅ `model` - Model identifier
- ✅ `messages` - Conversation history
- ✅ `temperature` - Sampling temperature (0-2)
- ✅ `max_tokens` - Max output tokens
- ✅ `stream` - Enable streaming
- ✅ `stop` - Stop sequences
- ✅ `top_p` - Nucleus sampling
- ✅ `frequency_penalty` - Penalize frequent tokens (-2 to 2)
- ✅ `presence_penalty` - Penalize present tokens (-2 to 2)
- ✅ `user` - User tracking identifier

**Extended Parameters:**
- ✅ `top_k` - Top-K sampling (extended)
- ✅ `seed` - Deterministic sampling
- ✅ `repetition_penalty` - Repetition penalty (extended)
- ✅ `response_format` - JSON mode / JSON schema
- ✅ `tools` - Tool definitions (defined but not used - **bug #4**)
- ✅ `tool_choice` - Tool selection (defined but not used - **bug #4**)
- ✅ `extensions` - Provider-specific extensions

#### ❌ Missing Parameters (11/29)

**High Priority:**
- ❌ `n` - Number of completions to generate (default: 1) - **Bug #5**
- ❌ `logprobs` - Return log probabilities - **Bug #6**
- ❌ `top_logprobs` - Number of logprobs to return (0-20)
- ❌ `logit_bias` - Modify token likelihoods
- ❌ `max_completion_tokens` - Replaces `max_tokens` (o-series models) - **Bug #8**

**Medium Priority:**
- ❌ `parallel_tool_calls` - Enable parallel function calling
- ❌ `service_tier` - Latency tier (auto/default)
- ❌ `store` - Store for distillation/evals
- ❌ `metadata` - Developer-defined tags
- ❌ `modalities` - Output modalities (text/audio)
- ❌ `audio` - Audio output configuration

**Low Priority:**
- ❌ `prediction` - Predicted output for latency optimization

### Completions (`POST /v1/completions`)

#### ✅ Supported (9/13)
- ✅ `model`, `prompt`, `temperature`, `max_tokens`, `top_p`
- ✅ `frequency_penalty`, `presence_penalty`, `stop`
- ✅ `stream` - Full streaming support (**Bug #1 FIXED**)

#### ❌ Missing (5/13)
- ❌ `n` - Number of completions
- ❌ `logprobs` - Log probabilities
- ❌ `echo` - Echo prompt in response
- ❌ `logit_bias` - Token likelihood modification
- ❌ `best_of` - Generate best_of completions

### Embeddings (`POST /v1/embeddings`)

#### ✅ Defined Types (4/5)
- ✅ `model`, `input`, `encoding_format`, `dimensions`

#### ❌ Status
- ⚠️ **Endpoint returns 501 Not Implemented** (**Bug #2**)
- Validation logic present but no provider integration

### Models (`GET /v1/models`)

#### ✅ Fully Compatible
- Exceeds OpenAI spec with LocalRouter extensions:
  - `provider` - Provider name
  - `context_window` - Token limit
  - `pricing` - Cost information
  - `capabilities` - Model features
  - `detailed_capabilities` - Advanced capability tracking

---

## 3. Identified Bugs

### 🔴 Critical Bugs

#### Bug #2: Embeddings Endpoint Not Implemented
**File**: `src-tauri/src/server/routes/embeddings.rs:48-52`
**Issue**: Endpoint returns 501 Not Implemented
**Impact**: Complete lack of embeddings support
**Fix Required**:
1. Add `embed()` method to `ModelProvider` trait
2. Implement embeddings for providers (OpenAI, Cohere, etc.)
3. Add router support for embedding models
4. Implement request handling in `embeddings.rs`

**Code Location**:
```rust
// Line 48-52
Err(ApiErrorResponse::new(
    axum::http::StatusCode::NOT_IMPLEMENTED,
    "not_implemented",
    "Embeddings endpoint not yet implemented. This is planned for a future release.",
))
```

### 🟡 High Priority Bugs

#### ✅ Bug #1: Completions Streaming Not Supported (FIXED)
**File**: `src-tauri/src/server/routes/completions.rs`
**Status**: ✅ FIXED (2026-01-20)
**Issue**: Previously returned error for `stream: true`
**Fix Applied**:
1. ✅ Added `handle_streaming` function that converts chat completion chunks to legacy completion chunks
2. ✅ Implemented SSE streaming response handling
3. ✅ Added proper metrics tracking and token estimation for streaming
4. ✅ Updated OpenAPI documentation to reflect streaming support

**Implementation Details**:
- Converts `ChatCompletionChunk` to `CompletionChunk` format
- Uses `object: "text_completion"` for legacy compatibility
- Maps `delta.content` to `text` field in chunk choices
- Tracks tokens and records metrics asynchronously after stream completes

#### Bug #3: Multimodal Content Rejected
**File**: `src-tauri/src/server/routes/chat.rs:340-347`
**Issue**: Rejects `content` with image parts
**Impact**: Cannot use vision models (GPT-4 Vision, Claude 3, etc.)
**Fix Required**:
1. Extract and pass multimodal content to providers
2. Update provider trait to support multimodal messages
3. Implement image_url handling

**Code Location**:
```rust
// Line 340-347
Some(MessageContent::Parts(_)) => {
    // For now, extract text from parts
    // Full multimodal support would require more complex handling
    return Err(ApiErrorResponse::bad_request(
        "Multimodal content not yet fully supported",
    ));
}
```

#### Bug #4: Tool Calling Not Implemented
**File**: `src-tauri/src/server/types.rs:79-83`, `chat.rs`
**Issue**: Tool definitions accepted but not passed to providers
**Impact**: Function calling doesn't work
**Fix Required**:
1. Add `tools` and `tool_choice` to `ProviderCompletionRequest`
2. Implement tool calling in provider adapters
3. Handle tool call responses

**Observation**: Types are fully defined (Tool, ToolChoice, FunctionDefinition) but never used in routing logic.

### 🟢 Medium Priority Bugs

#### Bug #5: Missing `n` Parameter
**Issue**: Cannot generate multiple completion choices
**Impact**: No multi-generation support
**OpenAI Spec**: `n` (integer, default: 1, max varies by model)
**Fix Required**:
1. Add `n: Option<u32>` to `ChatCompletionRequest`
2. Modify router to generate multiple completions
3. Return multiple choices in response

#### Bug #6: Missing Logprobs Support
**Issue**: Cannot return token probabilities
**Impact**: No probability analysis for outputs
**OpenAI Spec**: `logprobs` (boolean), `top_logprobs` (integer 0-20)
**Fix Required**:
1. Add `logprobs` and `top_logprobs` to request types
2. Implement in provider adapters (OpenAI, Anthropic if supported)
3. Return `logprobs` field in response choices

#### Bug #7: Response Format Not Fully Implemented
**File**: `src-tauri/src/server/routes/chat.rs:134-160`
**Issue**: Validates `response_format` but doesn't enforce it
**Impact**: Providers may ignore JSON mode/schema
**Fix Required**:
1. Pass `response_format` to provider adapters
2. Use feature adapters to transform to provider-specific format
3. Validate response conforms to schema

### 🔵 Low Priority Issues

#### Bug #8: `max_completion_tokens` Not Supported
**Issue**: New parameter for o-series models not supported
**Impact**: Cannot use o1/o3 models optimally
**OpenAI Spec**: Replaces `max_tokens` for reasoning models
**Fix Required**:
1. Add `max_completion_tokens: Option<u32>` to request
2. Prefer `max_completion_tokens` over `max_tokens` when both present
3. Return error if used with incompatible models

#### Bug #9: Validation Range Issues
**File**: `src-tauri/src/server/routes/chat.rs:96-131`
**Issue**: Some validation ranges don't match OpenAI spec
**Examples**:
- `repetition_penalty` validated as 0.0-2.0 (LocalRouter extension)
- Should document extended parameters separately

---

## 4. Missing Response Fields

### Chat Completion Response

#### ❌ Missing Fields:
- `system_fingerprint` - Model version identifier
- `service_tier` - Tier used for request
- `choices[].logprobs` - Log probability information
- `choices[].message.tool_calls` - Tool call information (**related to Bug #4**)
- `choices[].message.refusal` - Model refusal (new safety feature)

### Streaming Response

#### ❌ Missing Features:
- Final chunk with `usage` field (token counts)
- `service_tier` in chunks
- Tool call streaming support

---

## 5. Recommendations

### Immediate Actions (Critical)

1. **Implement Embeddings** (Bug #2)
   - Priority: **CRITICAL**
   - Timeline: 1-2 weeks
   - Providers: Start with OpenAI, Cohere

2. **Fix Completions Streaming** (Bug #1)
   - Priority: **HIGH**
   - Timeline: 2-3 days
   - Impact: Improved legacy endpoint compatibility

3. **Add Multimodal Support** (Bug #3)
   - Priority: **HIGH**
   - Timeline: 1 week
   - Impact: Enable vision models

### Short-term Improvements (1-2 months)

4. **Implement Tool Calling** (Bug #4)
   - Priority: **HIGH**
   - Timeline: 2-3 weeks
   - Impact: Function calling support

5. **Add `n` Parameter** (Bug #5)
   - Priority: **MEDIUM**
   - Timeline: 1 week
   - Impact: Multi-generation support

6. **Add Logprobs Support** (Bug #6)
   - Priority: **MEDIUM**
   - Timeline: 1-2 weeks
   - Impact: Token probability analysis

### Long-term Enhancements (3-6 months)

7. **Audio Endpoints**
   - Speech-to-text (Whisper)
   - Text-to-speech

8. **Image Generation**
   - DALL-E support

9. **Moderation Endpoint**
   - Content safety

10. **Batch Processing**
    - Async job processing

---

## 6. Test Coverage Gaps

Based on the analysis:

### Missing Tests:
1. **Streaming validation** - No tests for SSE format compliance
2. **Tool calling** - No tests (feature not implemented)
3. **Multimodal content** - No tests for image_url parts
4. **Multiple completions** (`n` parameter) - No tests
5. **Logprobs** - No tests
6. **Response format enforcement** - Tests validate input but not output
7. **Embeddings** - Basic validation tests but no integration tests

### Existing Test Gaps:
- No tests for edge cases (empty strings, Unicode, very long inputs)
- No tests for concurrent requests
- No tests for rate limiting integration
- No tests for model access control (partially covered)

---

## 7. Documentation Gaps

### OpenAPI Specification Issues:

1. **Missing deprecation notices**:
   - Should note `max_tokens` → `max_completion_tokens` transition
   - Should document legacy completions endpoint status

2. **Missing parameter descriptions**:
   - Extended parameters (top_k, repetition_penalty) not marked as non-standard
   - Provider-specific behavior not documented

3. **Missing examples**:
   - No streaming examples in OpenAPI spec
   - No tool calling examples
   - No multimodal examples

---

## 8. Compatibility Matrix

| Feature | OpenAI | LocalRouter | Notes |
|---------|--------|-------------|-------|
| Chat Completions | ✅ | ✅ | Missing: n, logprobs, tools |
| Completions | ✅ | ✅ | Full support (Bug #1 FIXED) |
| Embeddings | ✅ | ❌ | Not implemented |
| Audio | ✅ | ❌ | Not planned |
| Images | ✅ | ❌ | Not planned |
| Files | ✅ | ❌ | Not planned |
| Fine-tuning | ✅ | ❌ | Not planned |
| Batch | ✅ | ❌ | Future consideration |
| Moderations | ✅ | ❌ | Future consideration |
| Streaming | ✅ | ✅ | Chat + Completions (Bug #1 FIXED) |
| Tool Calling | ✅ | ❌ | Defined but not implemented |
| Multimodal | ✅ | ❌ | Vision not supported |
| JSON Mode | ✅ | ⚠️ | Validated but not enforced |
| Structured Outputs | ✅ | ⚠️ | Schema defined but not enforced |

---

## 9. Priority Ranking for Bug Fixes

### P0 - Blocking Issues (Fix Immediately)
1. **Bug #2**: Embeddings not implemented
   - **Rationale**: Core use case for many applications
   - **User Impact**: HIGH - Cannot use for RAG, semantic search

### P1 - Major Issues (Fix Within 1 Week)
2. ~~**Bug #1**: Completions streaming not supported~~ ✅ FIXED (2026-01-20)

3. **Bug #3**: Multimodal content rejected
   - **Rationale**: Cannot use vision models
   - **User Impact**: MEDIUM - Blocks GPT-4V, Claude 3 Opus usage

### P2 - Important Issues (Fix Within 1 Month)
4. **Bug #4**: Tool calling not implemented
   - **Rationale**: Growing use case for agents
   - **User Impact**: MEDIUM - Alternative: manual function calling

5. **Bug #5**: Missing `n` parameter
   - **Rationale**: Standard OpenAI parameter
   - **User Impact**: LOW - Workaround: multiple requests

6. **Bug #6**: Missing logprobs
   - **Rationale**: Useful for debugging and analysis
   - **User Impact**: LOW - Niche use case

### P3 - Nice to Have (Fix When Possible)
7. **Bug #7**: Response format not enforced ✅ FIXED
8. **Bug #8**: `max_completion_tokens` not supported ✅ FIXED
9. **Bug #9**: Validation range documentation ✅ FIXED

---

## 10. Sources

This analysis is based on:
- [OpenAI API Reference](https://platform.openai.com/docs/api-reference/introduction)
- [Chat Completions API](https://platform.openai.com/docs/api-reference/chat)
- [Completions API](https://platform.openai.com/docs/api-reference/completions)
- [Embeddings API](https://platform.openai.com/docs/api-reference/embeddings)
- [Models API](https://platform.openai.com/docs/api-reference/models/list)
- [Structured Outputs Guide](https://platform.openai.com/docs/guides/structured-outputs)
- LocalRouter AI source code (src-tauri/src/server/)

---

## Appendix: Full Parameter Comparison Table

### Chat Completions Request Parameters

| Parameter | OpenAI | LocalRouter | Type | Notes |
|-----------|--------|-------------|------|-------|
| model | ✅ | ✅ | string | ✅ Fully supported |
| messages | ✅ | ✅ | array | ⚠️ No multimodal (Bug #3) |
| max_tokens | ✅ | ✅ | integer | ✅ Supported |
| max_completion_tokens | ✅ | ❌ | integer | ❌ Bug #8 |
| temperature | ✅ | ✅ | float | ✅ 0-2 range |
| top_p | ✅ | ✅ | float | ✅ 0-1 range |
| n | ✅ | ❌ | integer | ❌ Bug #5 |
| stream | ✅ | ✅ | boolean | ✅ Supported |
| stop | ✅ | ✅ | string/array | ✅ Supported |
| presence_penalty | ✅ | ✅ | float | ✅ -2 to 2 |
| frequency_penalty | ✅ | ✅ | float | ✅ -2 to 2 |
| logit_bias | ✅ | ❌ | map | ❌ Not supported |
| logprobs | ✅ | ❌ | boolean | ❌ Bug #6 |
| top_logprobs | ✅ | ❌ | integer | ❌ Bug #6 |
| user | ✅ | ✅ | string | ✅ Supported |
| seed | ✅ | ✅ | integer | ✅ Supported |
| tools | ✅ | ⚠️ | array | ⚠️ Bug #4 (defined, not used) |
| tool_choice | ✅ | ⚠️ | string/object | ⚠️ Bug #4 (defined, not used) |
| parallel_tool_calls | ✅ | ❌ | boolean | ❌ Not supported |
| response_format | ✅ | ⚠️ | object | ⚠️ Bug #7 (validated, not enforced) |
| service_tier | ✅ | ❌ | string | ❌ Not supported |
| store | ✅ | ❌ | boolean | ❌ Not supported |
| metadata | ✅ | ❌ | object | ❌ Not supported |
| modalities | ✅ | ❌ | array | ❌ Not supported |
| audio | ✅ | ❌ | object | ❌ Not supported |
| prediction | ✅ | ❌ | object | ❌ Not supported |
| top_k | ❌ | ✅ | integer | ✅ Extension parameter |
| repetition_penalty | ❌ | ✅ | float | ✅ Extension parameter |
| extensions | ❌ | ✅ | object | ✅ LocalRouter extension |

---

**End of Report**
