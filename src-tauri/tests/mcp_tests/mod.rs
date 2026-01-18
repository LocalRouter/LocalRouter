//! MCP Integration Test Suite
//!
//! This module provides comprehensive testing for the Model Context Protocol (MCP)
//! implementation in LocalRouter AI.
//!
//! ## Test Organization
//!
//! - `common` - Mock server builders and shared utilities
//! - `request_validation` - JSON-RPC assertion helpers
//! - `stdio_transport_tests` - STDIO process management tests
//! - `sse_transport_tests` - SSE transport tests
//! - `websocket_transport_tests` - WebSocket transport tests
//! - `oauth_client_tests` - OAuth client authentication tests
//! - `oauth_server_tests` - MCP server OAuth discovery/tokens tests
//! - `proxy_integration_tests` - End-to-end proxy flow tests
//! - `manager_lifecycle_tests` - McpServerManager lifecycle tests
//! - `concurrent_requests_tests` - Concurrent request handling
//! - `error_scenarios_tests` - HTTP errors, timeouts, failures
//! - `health_check_tests` - Health monitoring tests
//!
//! ## Running Tests
//!
//! ```bash
//! # Run all MCP integration tests
//! cargo test --test mcp_integration_tests
//!
//! # Run specific test modules
//! cargo test --test mcp_integration_tests stdio_transport
//! cargo test --test mcp_integration_tests oauth_client
//!
//! # Run with output
//! cargo test --test mcp_integration_tests -- --nocapture
//!
//! # Run tests serially to avoid port conflicts
//! cargo test --test mcp_integration_tests -- --test-threads=1
//! ```

pub mod common;
pub mod request_validation;

// Transport tests
pub mod stdio_transport_tests;
pub mod sse_transport_tests;
pub mod websocket_transport_tests;

// OAuth tests
pub mod oauth_client_tests;
pub mod oauth_server_tests;

// Integration tests
pub mod proxy_integration_tests;
pub mod manager_lifecycle_tests;

// Edge case tests
pub mod concurrent_requests_tests;
pub mod error_scenarios_tests;
pub mod health_check_tests;
