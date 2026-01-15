//! Provider integration tests
//!
//! Comprehensive tests for all model providers using mock HTTP servers.
//!
//! ## Test Organization
//!
//! ### Core Test Modules
//! - `common.rs` - Shared utilities, mock server builders, and test helpers
//! - `request_validation.rs` - Request validation utilities (headers, body, etc.)
//!
//! ### Basic Provider Tests
//! - `openai_compatible_tests.rs` - Tests for OpenAI-compatible providers
//! - `ollama_tests.rs` - Tests for Ollama (custom cumulative streaming)
//! - `anthropic_tests.rs` - Tests for Anthropic (Messages API)
//! - `gemini_tests.rs` - Tests for Google Gemini
//! - `cohere_tests.rs` - Tests for Cohere (v2 API)
//!
//! ### Detailed Validation Tests
//! - `openai_compatible_detailed.rs` - Detailed request/response validation for OpenAI format
//!
//! ### Scenario Tests
//! - `http_scenarios.rs` - HTTP error codes, network errors, malformed responses
//! - `sse_scenarios.rs` - SSE edge cases, chunking, formatting
//!
//! ## Test Coverage
//!
//! ### Basic Tests (all providers)
//! 1. Health checks
//! 2. Model listing
//! 3. Non-streaming completions
//! 4. Streaming completions
//! 5. Pricing information
//!
//! ### Detailed Validation (OpenAI-compatible)
//! 1. Request headers (Authorization, Content-Type, etc.)
//! 2. Request body structure and fields
//! 3. Messages array format
//! 4. Optional parameters
//! 5. Response field mapping
//!
//! ### HTTP Scenarios
//! 1. Status codes: 200, 400, 401, 403, 404, 429, 500, 503
//! 2. Network errors (connection refused, timeout)
//! 3. Malformed responses (invalid JSON, missing fields, empty responses)
//! 4. Unicode and special characters
//! 5. Very long content
//!
//! ### SSE Scenarios
//! 1. Single/multiple events per chunk
//! 2. Events split across chunks
//! 3. Empty lines and comments
//! 4. SSE fields (event, id, retry)
//! 5. [DONE] marker handling
//! 6. Invalid JSON in stream
//! 7. Mixed valid/invalid events
//! 8. Unicode in streamed content
//!
//! ## Adding New Tests
//!
//! When adding a new request type that should work across all providers:
//! 1. Add the request builder to `common.rs`
//! 2. Add mock responses for each provider format
//! 3. Add test cases to each provider test file
//!
//! When adding new validation:
//! 1. Add validation helpers to `request_validation.rs`
//! 2. Use in detailed test files
//!
//! ## Limitations
//!
//! Some providers use hardcoded base URLs and need refactoring to support
//! custom URLs for testing. These are marked with TODO comments.

mod common;
mod request_validation;
mod openai_compatible_tests;
mod openai_compatible_detailed;
mod ollama_tests;
mod anthropic_tests;
mod gemini_tests;
mod cohere_tests;
mod http_scenarios;
mod sse_scenarios;
