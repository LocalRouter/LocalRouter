# MCP Gateway Integration Test Review

**Date**: 2026-01-19
**Status**: Needs Comprehensive Mock-Based Tests

## Current Test Status

### Existing Tests (`mcp_gateway_integration_tests.rs`)

Currently we have **11 tests** that are mostly unit-level tests of gateway components:

1. ✅ `test_gateway_session_creation` - Session creation with empty servers
2. ✅ `test_gateway_namespace_parsing` - Namespace application/parsing
3. ✅ `test_gateway_empty_allowed_servers` - Empty server list handling
4. ✅ `test_gateway_config_defaults` - Configuration defaults
5. ✅ `test_gateway_session_expiration` - Session TTL expiration
6. ✅ `test_gateway_concurrent_requests` - Concurrent client handling
7. ✅ `test_search_tool_creation` - Deferred loading search tool
8. ✅ `test_gateway_method_routing` - Broadcast vs direct routing logic
9. ✅ `test_cached_list_validity` - Cache TTL validation
10. ✅ `test_gateway_cleanup_expired_sessions` - Session cleanup
11. ✅ `test_deferred_loading_search_relevance` - Search relevance scoring

**Problem**: These tests don't use mock MCP servers. They test individual components but not the full integration flow.

## Required Integration Tests

### Test Infrastructure Needed

**Mock MCP Server Setup**:
- Spin up 2 mock MCP servers (SSE transport for simplicity)
- Configure MCP ServerManager to use these mocks
- Create Gateway instance pointing to these servers
- Each test should verify:
  1. Requests sent to individual mock servers
  2. Merged/transformed responses from gateway

### Critical Test Scenarios

#### 1. Initialize Endpoint Tests

| Test | Description | Verification |
|------|-------------|--------------|
| `test_initialize_merges_capabilities` | Two servers with different capabilities | ✓ Merged capabilities (union of features)<br>✓ Protocol version (minimum)<br>✓ Server description lists both servers |
| `test_initialize_partial_failure` | One server succeeds, one fails | ✓ Returns successful server's data<br>✓ Notes failure in description<br>✓ No error thrown |
| `test_initialize_all_servers_fail` | Both servers timeout/error | ✓ Returns appropriate error<br>✓ Clear failure message |
| `test_initialize_protocol_version_negotiation` | Servers with different protocol versions | ✓ Uses minimum version<br>✓ Both servers receive same version |

#### 2. Tools/List Endpoint Tests

| Test | Description | Verification |
|------|-------------|--------------|
| `test_tools_list_merges_and_namespaces` | Server1 has 2 tools, Server2 has 2 tools | ✓ Returns 4 tools total<br>✓ All tools have `server__tool` format<br>✓ Descriptions unchanged<br>✓ Input schemas preserved |
| `test_tools_list_handles_duplicates` | Both servers have tool named "read" | ✓ Both returned as `server1__read` and `server2__read`<br>✓ No collision |
| `test_tools_list_empty_server` | Server1 has tools, Server2 returns empty | ✓ Returns Server1 tools only<br>✓ No error |
| `test_tools_list_caching` | Second request without cache invalidation | ✓ Server called once<br>✓ Second request uses cache<br>✓ Same results |
| `test_tools_list_cache_invalidation` | Cache invalidated via notification | ✓ Notification triggers refresh<br>✓ Next request fetches fresh data |
| `test_tools_list_partial_failure` | Server1 succeeds, Server2 fails | ✓ Returns Server1 tools<br>✓ Server2 failure noted<br>✓ No error |

#### 3. Resources/List Endpoint Tests

| Test | Description | Verification |
|------|-------------|--------------|
| `test_resources_list_merges_and_namespaces` | Server1 and Server2 each have resources | ✓ All resources namespaced<br>✓ URIs unchanged<br>✓ MIME types preserved |
| `test_resources_list_with_templates` | Resources with URI templates | ✓ Templates preserved<br>✓ Namespace applied to name only |
| `test_resources_list_partial_failure` | One server fails | ✓ Returns working server's resources |

#### 4. Prompts/List Endpoint Tests

| Test | Description | Verification |
|------|-------------|--------------|
| `test_prompts_list_merges_and_namespaces` | Server1 and Server2 each have prompts | ✓ All prompts namespaced<br>✓ Arguments preserved<br>✓ Descriptions unchanged |
| `test_prompts_list_with_arguments` | Prompts with different argument sets | ✓ Argument schemas preserved<br>✓ Required/optional markers intact |

#### 5. Tools/Call Endpoint Tests (Direct Routing)

| Test | Description | Verification |
|------|-------------|--------------|
| `test_tools_call_routes_to_correct_server` | Call `server1__read_file` and `server2__write_file` | ✓ Requests routed to correct servers<br>✓ Namespace stripped before forwarding<br>✓ Original tool name sent to backend<br>✓ Response returned unchanged |
| `test_tools_call_strips_namespace` | Request with `server1__tool`, backend receives `tool` | ✓ Request params show original name<br>✓ No namespace in backend request |
| `test_tools_call_unknown_tool` | Call `nonexistent__tool` | ✓ Error returned<br>✓ Clear error message |
| `test_tools_call_wrong_server` | Call tool from server client doesn't have access to | ✓ Access denied error |
| `test_tools_call_passes_arguments` | Call with complex arguments | ✓ All arguments passed through<br>✓ No transformation |
| `test_tools_call_handles_error_response` | Backend returns error | ✓ Error passed through<br>✓ Error format preserved |

#### 6. Resources/Read Endpoint Tests (Direct Routing)

| Test | Description | Verification |
|------|-------------|--------------|
| `test_resources_read_routes_by_uri` | Read resource by URI | ✓ Correct server identified<br>✓ Request forwarded<br>✓ Response returned |
| `test_resources_read_by_name` | Read resource by namespaced name | ✓ Namespace parsed<br>✓ Routed to correct server |
| `test_resources_read_binary_content` | Resource with binary content | ✓ Binary data preserved<br>✓ MIME type correct |
| `test_resources_read_not_found` | Non-existent resource | ✓ Clear error message |

#### 7. Prompts/Get Endpoint Tests (Direct Routing)

| Test | Description | Verification |
|------|-------------|--------------|
| `test_prompts_get_routes_by_namespace` | Get `server1__review` prompt | ✓ Routed to server1<br>✓ Namespace stripped<br>✓ Response with messages returned |
| `test_prompts_get_with_arguments` | Prompt requiring arguments | ✓ Arguments passed through<br>✓ Response interpolated |
| `test_prompts_get_not_found` | Non-existent prompt | ✓ Error returned |

#### 8. Deferred Loading Tests

| Test | Description | Verification |
|------|-------------|--------------|
| `test_deferred_loading_initial_state` | Enable deferred loading on session create | ✓ tools/list returns only search tool<br>✓ Full catalog stored in session |
| `test_deferred_loading_search_activates_tools` | Call search tool with query "read" | ✓ Relevant tools activated<br>✓ Activation persists for session<br>✓ Next tools/list includes activated tools |
| `test_deferred_loading_activation_threshold` | Search with varying relevance | ✓ High-relevance tools activated (>0.7)<br>✓ Minimum 3 tools if >0.3 relevance |
| `test_deferred_loading_resources_and_prompts` | Search for resources and prompts | ✓ Can activate resources<br>✓ Can activate prompts |
| `test_deferred_loading_token_savings` | Calculate token reduction | ✓ Without: ~10,000+ tokens<br>✓ With: ~300 tokens (search tool only) |

#### 9. Session Management Tests

| Test | Description | Verification |
|------|-------------|--------------|
| `test_session_reuse` | Multiple requests from same client | ✓ Session reused<br>✓ Mappings cached<br>✓ Last activity updated |
| `test_session_expiration_cleanup` | Session expires after TTL | ✓ Old session removed<br>✓ New session created on next request |
| `test_concurrent_clients` | Multiple clients, separate sessions | ✓ Sessions isolated<br>✓ No data leakage<br>✓ Correct server access per client |

#### 10. Notification Handling Tests

| Test | Description | Verification |
|------|-------------|--------------|
| `test_notification_invalidates_tools_cache` | Server sends `tools/list_changed` | ✓ Cache invalidated<br>✓ Next request fetches fresh data |
| `test_notification_invalidates_resources_cache` | Server sends `resources/list_changed` | ✓ Cache invalidated |
| `test_notification_invalidates_prompts_cache` | Server sends `prompts/list_changed` | ✓ Cache invalidated |
| `test_notification_forwarded_to_client` | Server notification forwarded | ✓ Client receives notification |

#### 11. Error Handling Tests

| Test | Description | Verification |
|------|-------------|--------------|
| `test_all_servers_timeout` | Both servers timeout | ✓ Error after retry<br>✓ Clear timeout message |
| `test_malformed_json_response` | Server returns invalid JSON | ✓ Error handled gracefully<br>✓ Doesn't crash gateway |
| `test_http_500_error` | Server returns HTTP 500 | ✓ Error logged<br>✓ Partial results if other servers work |
| `test_connection_refused` | Server not reachable | ✓ Connection error handled<br>✓ Retry attempted |
| `test_invalid_namespace_format` | Tool name with single underscore | ✓ Error: invalid namespace<br>✓ Clear error message |
| `test_json_rpc_error_response` | Server returns JSON-RPC error | ✓ Error passed through<br>✓ Error code/message preserved |

#### 12. Performance Tests

| Test | Description | Target | Verification |
|------|-------------|--------|--------------|
| `test_initialize_latency` | Initialize with 3 servers | <500ms | ✓ Under target |
| `test_tools_list_cached_latency` | tools/list (cached) | <200ms | ✓ Under target |
| `test_tools_list_uncached_latency` | tools/list (fresh) | <1s | ✓ Under target |
| `test_tools_call_overhead` | tools/call routing overhead | +50ms | ✓ Under target |
| `test_concurrent_sessions_memory` | 100 sessions | <1GB | ✓ Under target |

## Test Implementation Approach

### Mock Server Pattern

```rust
// Create mock MCP servers
let server1_mock = MockMcpServer::new().await;
let server2_mock = MockMcpServer::new().await;

// Configure responses
server1_mock.mock_method("tools/list", json!({
    "tools": [{"name": "read_file", ...}]
})).await;

server2_mock.mock_method("tools/list", json!({
    "tools": [{"name": "write_file", ...}]
})).await;

// Create gateway with these servers
let gateway = setup_gateway(vec![
    ("server1", server1_mock.base_url()),
    ("server2", server2_mock.base_url()),
]).await;

// Test request
let request = JsonRpcRequest::new(...);
let response = gateway.handle_request(
    "test-client",
    vec!["server1", "server2"],
    false,
    request
).await;

// Verify merged response
assert_eq!(response.result["tools"].len(), 2);
assert_eq!(response.result["tools"][0]["name"], "server1__read_file");
assert_eq!(response.result["tools"][1]["name"], "server2__write_file");
```

### Request Verification Pattern

```rust
// Capture requests sent to mock server
let server1_requests = server1_mock.received_requests().await;

// Verify request was sent
assert_eq!(server1_requests.len(), 1);
let req = &server1_requests[0];

// Parse JSON-RPC request body
let body: JsonRpcRequest = serde_json::from_slice(&req.body).unwrap();

// Verify method
assert_eq!(body.method, "tools/call");

// Verify namespace was stripped
assert_eq!(body.params["name"], "read_file"); // NOT "server1__read_file"
assert_eq!(body.params["arguments"]["path"], "/test.txt");
```

## Summary

### Coverage Analysis

**Current Tests**: 11 unit-level tests
**Required Tests**: 60+ integration tests covering:
- 6 major endpoints (initialize, tools/list, tools/call, resources/list, resources/read, prompts/list, prompts/get)
- 12 test categories
- Both success and failure scenarios
- Request verification AND response verification

### Priority

**P0 (Critical)**:
- Tools/list merging and namespacing
- Tools/call routing and namespace stripping
- Initialize merging
- Partial failure handling

**P1 (High)**:
- Resources/prompts endpoints
- Deferred loading core functionality
- Session management
- Error handling

**P2 (Medium)**:
- Notification handling
- Performance tests
- Edge cases (malformed data, etc.)

### Implementation Status

❌ **NOT IMPLEMENTED**: Mock-based integration tests
✅ **IMPLEMENTED**: Component unit tests
⚠️ **PARTIAL**: Created test file skeleton but needs completion

### Next Steps

1. Complete `mcp_gateway_mock_integration_tests.rs` implementation
2. Fix all mock mounting code to use proper wiremock API
3. Add request capture and verification
4. Run full test suite
5. Address any failures
6. Add performance benchmarks

---

**Recommendation**: Implement P0 tests first (15-20 tests), then P1, then P2.

**Estimated Effort**:
- P0: 4-6 hours
- P1: 4-6 hours
- P2: 2-4 hours
- **Total**: 10-16 hours of test development

