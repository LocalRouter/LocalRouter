# Manual Testing Guide - MCP Authentication Redesign
**Date**: 2026-01-18
**Status**: Ready for Manual Testing

## Overview

This guide provides step-by-step instructions for manually testing the MCP authentication redesign implementation. All automated tests pass (29 integration tests), but manual testing with real MCP servers is recommended to verify the complete user experience.

## Test Summary

### Automated Tests (✅ All Passing)

**Client Authentication Tests** (12 tests)
- ✅ Client creation and verification
- ✅ Secret-based authentication
- ✅ Credentials verification
- ✅ Token generation and verification
- ✅ Token revocation
- ✅ LLM provider access control
- ✅ MCP server access control
- ✅ Client updates and deletion
- ✅ Disabled client authentication
- ✅ Multiple clients management

**MCP Auth Config Tests** (8 tests)
- ✅ Server with no auth
- ✅ Server with EnvVars auth
- ✅ Server with BearerToken auth
- ✅ Server with CustomHeaders auth
- ✅ Server with OAuth auth
- ✅ Auth config updates
- ✅ Config serialization/deserialization
- ✅ Multiple servers with different auth

**Access Control Tests** (9 tests)
- ✅ LLM provider access control
- ✅ MCP server access control
- ✅ Multiple clients independent access
- ✅ Disabled client loses access
- ✅ Access persists across updates
- ✅ Duplicate grants are idempotent
- ✅ Removing nonexistent access is safe
- ✅ Client deletion removes all access
- ✅ Case sensitivity in provider names

## Manual Testing Checklist

### 1. Client Management UI

#### 1.1 Create a New Client
```
Steps:
1. Open LocalRouter AI application
2. Navigate to "Clients" tab
3. Click "Add Client" button
4. Enter client name: "Test Client 1"
5. Click "Create"

Expected Results:
- Client appears in list with unique ID
- Client secret is displayed (starts with "lr-")
- Client is enabled by default
- No LLM providers or MCP servers granted by default
- "Copy Secret" button works
```

#### 1.2 Grant LLM Provider Access
```
Steps:
1. Select "Test Client 1" from clients list
2. Click "Grant Provider Access"
3. Select "openai" from dropdown
4. Click "Grant"
5. Repeat for "anthropic"

Expected Results:
- Both providers appear in "Allowed LLM Providers" section
- Providers are listed alphabetically
- Can remove individual providers
```

#### 1.3 Grant MCP Server Access
```
Steps:
1. In "Test Client 1" detail view
2. Click "Grant MCP Server Access"
3. Select an MCP server from dropdown
4. Click "Grant"

Expected Results:
- Server appears in "Allowed MCP Servers" section
- Can remove server access individually
```

#### 1.4 Disable/Enable Client
```
Steps:
1. In "Test Client 1" detail view
2. Click "Disable" button
3. Try to authenticate with the client secret
4. Re-enable the client
5. Try to authenticate again

Expected Results:
- When disabled, authentication fails
- Client retains all permissions while disabled
- When re-enabled, authentication works again
```

#### 1.5 Delete Client
```
Steps:
1. Select "Test Client 1"
2. Click "Delete" button
3. Confirm deletion
4. Try to authenticate with old secret

Expected Results:
- Client removed from list
- Authentication with old secret fails
- Cannot access any resources
```

### 2. MCP Server Authentication Configuration

#### 2.1 Create MCP Server with No Auth
```
Steps:
1. Navigate to "MCP Servers" tab
2. Click "Add Server"
3. Name: "Test STDIO Server"
4. Transport: "STDIO"
5. Command: "npx"
6. Args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
7. Auth Method: "None"
8. Click "Create"

Expected Results:
- Server appears in list
- Server can be started without errors
- No authentication configured
```

#### 2.2 Create MCP Server with EnvVars Auth
```
Steps:
1. Click "Add Server"
2. Name: "Test EnvVars Server"
3. Transport: "STDIO"
4. Command: "node"
5. Args: ["server.js"]
6. Auth Method: "Environment Variables"
7. Add variable: API_KEY = "test-key-123"
8. Add variable: SECRET = "test-secret-456"
9. Click "Create"

Expected Results:
- Server created with auth config
- Environment variables shown in detail view (masked)
- When server starts, env vars are passed to process
```

#### 2.3 Create MCP Server with BearerToken Auth
```
Steps:
1. Click "Add Server"
2. Name: "Test Bearer Server"
3. Transport: "HTTP/SSE"
4. URL: "http://localhost:8080/sse"
5. Auth Method: "Bearer Token"
6. Enter token: "secret-bearer-token-xyz"
7. Click "Create"

Expected Results:
- Server created with bearer token
- Token stored securely in keychain
- Token shown masked in UI
- When connecting, Authorization header added
```

#### 2.4 Create MCP Server with CustomHeaders Auth
```
Steps:
1. Click "Add Server"
2. Name: "Test Headers Server"
3. Transport: "HTTP/SSE"
4. URL: "http://localhost:8080/sse"
5. Auth Method: "Custom Headers"
6. Add header: X-API-Key = "api-key-123"
7. Add header: X-Custom = "custom-value"
8. Click "Create"

Expected Results:
- Server created with custom headers
- Headers shown in detail view (values masked)
- When connecting, headers included in request
```

#### 2.5 Create MCP Server with OAuth Auth
```
Steps:
1. Click "Add Server"
2. Name: "Test OAuth Server"
3. Transport: "HTTP/SSE"
4. URL: "http://localhost:8080/sse"
5. Auth Method: "OAuth 2.0"
6. Client ID: "oauth-client-123"
7. Client Secret: "oauth-secret-456"
8. Auth URL: "https://auth.example.com/authorize"
9. Token URL: "https://auth.example.com/token"
10. Scopes: ["read", "write"]
11. Click "Create"

Expected Results:
- Server created with OAuth config
- OAuth details shown in detail view (secret masked)
- When connecting, OAuth flow initiated (currently shows warning)
```

### 3. Client API Authentication

#### 3.1 Test Client Secret Authentication
```
Steps:
1. Create a client and copy the secret
2. Use curl or API client to make request:

   curl -X POST http://localhost:3625/v1/chat/completions \
     -H "Authorization: Bearer lr-xxxxx..." \
     -H "Content-Type: application/json" \
     -d '{
       "model": "openai/gpt-4",
       "messages": [{"role": "user", "content": "Hello"}]
     }'

Expected Results:
- Request succeeds if client has openai provider access
- Returns 403 if client lacks provider access
- Returns 401 if secret is invalid
```

#### 3.2 Test OAuth Access Token Authentication
```
Steps:
1. Obtain OAuth access token (via OAuth flow)
2. Use token in request:

   curl -X POST http://localhost:3625/v1/chat/completions \
     -H "Authorization: Bearer lr-xxxxx..." \
     -H "Content-Type: application/json" \
     -d '{
       "model": "anthropic/claude-3-opus",
       "messages": [{"role": "user", "content": "Hello"}]
     }'

Expected Results:
- Token verified against TokenStore
- Request succeeds if client has provider access
- Token expires after 1 hour
```

#### 3.3 Test MCP Proxy Access Control
```
Steps:
1. Create client with MCP server access
2. Make JSON-RPC request via proxy:

   curl -X POST http://localhost:3625/mcp/{client_id}/{server_id} \
     -H "Authorization: Bearer lr-xxxxx..." \
     -H "Content-Type: application/json" \
     -d '{
       "jsonrpc": "2.0",
       "id": 1,
       "method": "tools/list",
       "params": {}
     }'

Expected Results:
- Request succeeds if client has access to server
- Returns 403 if client lacks server access
- Returns 401 if authentication fails
```

### 4. MCP Server Health Checks

#### 4.1 Test STDIO Server Health
```
Steps:
1. Create STDIO MCP server
2. Start the server
3. Navigate to server detail page
4. Click "Test Connection" button

Expected Results:
- Health check shows "Healthy" status
- Last check timestamp updates
- Process is alive and responding
```

#### 4.2 Test SSE Server Health
```
Steps:
1. Create HTTP/SSE MCP server with auth
2. Start the server
3. View health status in detail page

Expected Results:
- Health check shows "Healthy" if connected
- Shows "Unhealthy" if connection lost
- Shows "Not Started" if not running
```

### 5. Access Control Enforcement

#### 5.1 Test LLM Provider Restrictions
```
Steps:
1. Create client with only "openai" access
2. Try to use OpenAI model (should succeed)
3. Try to use Anthropic model (should fail)
4. Grant "anthropic" access
5. Try Anthropic model again (should succeed)

Expected Results:
- 403 error when accessing unauthorized provider
- Clear error message indicating which provider is denied
- Immediate access after granting permission
```

#### 5.2 Test MCP Server Restrictions
```
Steps:
1. Create client with access to server-1 only
2. Try to access server-1 via proxy (should succeed)
3. Try to access server-2 via proxy (should fail)
4. Grant access to server-2
5. Try server-2 again (should succeed)

Expected Results:
- 403 error when accessing unauthorized server
- Error message specifies which server is denied
- Immediate access after granting permission
```

#### 5.3 Test Disabled Client Behavior
```
Steps:
1. Create client with full access
2. Make successful requests
3. Disable the client
4. Try to make requests (should fail)
5. Re-enable client
6. Verify requests work again

Expected Results:
- All requests fail when client is disabled
- 403 or 401 error returned
- Permissions restored when re-enabled
```

### 6. Error Handling

#### 6.1 Test Invalid Credentials
```
Test Cases:
- Wrong client secret
- Expired OAuth token
- Invalid bearer token format
- Missing Authorization header

Expected Results:
- 401 Unauthorized for all cases
- Clear error messages
- No server-side crashes
```

#### 6.2 Test Server Connection Errors
```
Test Cases:
- STDIO server fails to start
- SSE server unreachable
- Bearer token not found in keychain
- OAuth flow timeout

Expected Results:
- Health status shows "Unhealthy"
- Error messages displayed in UI
- Detailed error in logs
- No crashes
```

## Performance Testing

### Load Testing
```
1. Create 10+ clients
2. Grant various permissions to each
3. Make concurrent requests from multiple clients
4. Monitor:
   - Response times
   - Memory usage
   - Error rates
   - Authentication overhead
```

### Token Store Testing
```
1. Generate 100+ OAuth access tokens
2. Verify all tokens work
3. Wait for expiration (1 hour)
4. Verify expired tokens rejected
5. Monitor cleanup of expired tokens
```

## Security Testing

### Authentication Security
```
Test Cases:
- Try requests without Authorization header
- Try with malformed bearer tokens
- Try with tokens from deleted clients
- Try accessing resources without permission

Expected: All should fail with 401 or 403
```

### Secret Storage Security
```
Verify:
- Client secrets stored hashed in keychain
- Bearer tokens stored securely
- OAuth client secrets in keychain
- No secrets in logs or error messages
```

## Regression Testing

### Backward Compatibility
```
1. Test that legacy OAuth clients still work
2. Test that API key authentication still works
3. Test that old MCP servers without auth work
4. Verify config migration from old format
```

## Known Issues

1. **OAuth Flow Not Yet Implemented**: OAuth authentication for MCP servers shows a warning but doesn't perform actual OAuth flow. This is noted in the code and should be implemented in a future update.

2. **WebSocket Transport Removed**: WebSocket transport has been deprecated and removed. Only STDIO and HTTP/SSE transports are supported.

## Test Report Template

```markdown
## Test Report

**Date**: _____
**Tester**: _____
**Build**: _____

### Test Results

#### Client Management UI
- [ ] Create Client: ___
- [ ] Grant Provider Access: ___
- [ ] Grant MCP Access: ___
- [ ] Disable/Enable Client: ___
- [ ] Delete Client: ___

#### MCP Auth Configuration
- [ ] No Auth: ___
- [ ] EnvVars Auth: ___
- [ ] BearerToken Auth: ___
- [ ] CustomHeaders Auth: ___
- [ ] OAuth Auth: ___

#### API Authentication
- [ ] Client Secret Auth: ___
- [ ] OAuth Token Auth: ___
- [ ] MCP Proxy Auth: ___

#### Access Control
- [ ] LLM Provider Restrictions: ___
- [ ] MCP Server Restrictions: ___
- [ ] Disabled Client Behavior: ___

#### Error Handling
- [ ] Invalid Credentials: ___
- [ ] Connection Errors: ___

### Issues Found
1.
2.
3.

### Notes
```

## Next Steps

After manual testing completes:

1. **Document any bugs found** in GitHub issues
2. **Update implementation** based on feedback
3. **Add more automated tests** for edge cases discovered
4. **Update user documentation** with auth setup guides
5. **Create migration guide** for existing deployments

## Questions or Issues?

If you encounter any issues during manual testing:

1. Check the application logs for detailed error messages
2. Verify the test environment matches requirements
3. Review the implementation in relevant source files:
   - `src-tauri/src/clients/mod.rs` - Client manager
   - `src-tauri/src/mcp/manager.rs` - MCP server manager
   - `src-tauri/src/server/middleware/client_auth.rs` - Authentication middleware
   - `src-tauri/src/server/routes/mcp.rs` - MCP proxy routes
   - `src-tauri/src/server/routes/chat.rs` - LLM routes with access control
4. File a detailed bug report with steps to reproduce

---

**Status**: ✅ Automated tests passing, ready for manual verification
