# MCP OAuth Flow Improvements

## Summary

Redesign the MCP server creation UI to provide two distinct OAuth options inline during server creation, add re-authorization capabilities, and ensure proper cleanup when navigating away.

## Requirements

1. **Reorder form sections**: Headers should come AFTER Authentication for HTTP-SSE
2. **Update example URL**: Change to `https://api.example.com/mcp`
3. **Two OAuth options**:
   - `OAuth (Pre-generated)`: Collect client_id + client_secret immediately
   - `OAuth (External browser)`: Button to complete full OAuth flow inline
4. **Inline OAuth flow**: Discovery → Browser → Callback → Complete (all in creation dialog)
5. **Re-authorize button**: In MCP settings after creation
6. **Cleanup on navigation**: Cancel OAuth flow and shut down temp server
7. **Reusable**: The unified oauth_browser module works for both MCP and LLM providers

## Existing Infrastructure (Already Implemented)

- **`src-tauri/src/oauth_browser/`**: Unified OAuth 2.0 flow with PKCE, callback server management, cleanup
- **`src-tauri/src/mcp/oauth.rs`**: OAuth discovery via `.well-known/oauth-protected-resource`
- **Config enums**: `McpAuthConfig::OAuth` (pre-generated) and `McpAuthConfig::OAuthBrowser` (browser flow)
- **Tauri commands**: `discover_mcp_oauth_endpoints`, `start_mcp_oauth_browser_flow`, etc.

---

## Implementation Steps

### 1. Backend: New Tauri Command for Inline OAuth Flow

**File**: `src-tauri/src/ui/commands.rs`

Add a new command that combines discovery + flow start for inline use:

```rust
#[tauri::command]
pub async fn start_inline_oauth_flow(
    mcp_url: String,
    client_id: Option<String>,
    client_secret: Option<String>,
    // ... state params
) -> Result<InlineOAuthFlowResult, String>
```

**Behavior**:
1. Call `discover_oauth()` on the MCP URL
2. If discovery fails, return descriptive error
3. Start callback server and build authorization URL
4. Return flow_id, auth_url, redirect_uri, and discovered endpoints

Add corresponding poll and cancel commands:
- `poll_inline_oauth_status(flow_id)` - returns status with tokens on success
- `cancel_inline_oauth_flow(flow_id)` - cancels flow and cleans up

Register new commands in `src-tauri/src/main.rs`.

### 2. Frontend: Update mcp-servers-panel.tsx

**File**: `src/views/resources/mcp-servers-panel.tsx`

#### 2.1 Reorder Form Sections (lines ~1033-1105)

Change HTTP-SSE config order from:
```
URL → Headers → Authentication
```
To:
```
URL → Authentication → Headers
```

#### 2.2 Update Example URL (line ~1040)

```tsx
placeholder="https://api.example.com/mcp"
```

#### 2.3 New Auth Method Options (lines ~1066-1074)

```tsx
<LegacySelect value={authMethod} onChange={...}>
  <option value="none">None / Via headers</option>
  <option value="bearer">Bearer Token</option>
  <option value="oauth_pregenerated">OAuth (Pre-generated credentials)</option>
  <option value="oauth_browser">OAuth (External browser)</option>
</LegacySelect>
```

#### 2.4 Add State Variables (around line ~127)

```typescript
// Inline OAuth flow state
const [inlineOAuthFlowId, setInlineOAuthFlowId] = useState<string | null>(null)
const [inlineOAuthStatus, setInlineOAuthStatus] = useState<'idle' | 'discovering' | 'waiting' | 'success' | 'error'>('idle')
const [inlineOAuthError, setInlineOAuthError] = useState<string | null>(null)
const [inlineOAuthDiscovery, setInlineOAuthDiscovery] = useState<{...} | null>(null)

// OAuth credentials
const [oauthClientId, setOauthClientId] = useState('')
const [oauthClientSecret, setOauthClientSecret] = useState('')
```

#### 2.5 OAuth (Pre-generated) UI

When `authMethod === 'oauth_pregenerated'`:
- Client ID input (required)
- Client Secret input (required, password type)
- Note: "Stored securely in system keychain"

#### 2.6 OAuth (External browser) UI

When `authMethod === 'oauth_browser'`:
- Client ID input (optional for public clients)
- Client Secret input (optional)
- **Authorize button** that triggers inline flow
- Status display: Discovering... → Waiting for authorization... → Success/Error
- Cancel button while waiting

#### 2.7 Handler Functions

```typescript
const handleStartInlineOAuth = async () => {
  setInlineOAuthStatus('discovering')
  const result = await invoke('start_inline_oauth_flow', {...})
  setInlineOAuthFlowId(result.flowId)
  setInlineOAuthDiscovery(result.discovery)
  setInlineOAuthStatus('waiting')
  await open(result.authUrl) // Opens browser
  startPolling(result.flowId)
}

const handleCancelInlineOAuth = async () => {
  if (inlineOAuthFlowId) {
    await invoke('cancel_inline_oauth_flow', { flowId: inlineOAuthFlowId })
  }
  setInlineOAuthFlowId(null)
  setInlineOAuthStatus('idle')
}
```

#### 2.8 Cleanup on Navigation

```typescript
// In Dialog onOpenChange
if (!open && inlineOAuthFlowId) {
  invoke('cancel_inline_oauth_flow', { flowId: inlineOAuthFlowId })
}

// useEffect cleanup
useEffect(() => {
  return () => {
    if (inlineOAuthFlowId) {
      invoke('cancel_inline_oauth_flow', { flowId: inlineOAuthFlowId })
    }
  }
}, [inlineOAuthFlowId])
```

#### 2.9 Update handleCreateServer

Include OAuth discovery data in auth_config when creating server:
- For `oauth_pregenerated`: Run discovery, then save with client credentials
- For `oauth_browser`: Save discovered endpoints + credentials after successful flow

#### 2.10 Re-authorize Button in Detail View (lines ~824-897)

Update OAuth Authentication card to show:
- Current auth status (Authenticated / Not authenticated)
- **Re-authorize** or **Authorize** button (opens McpOAuthModal)
- **Revoke** button for authenticated servers

---

## Critical Files

| File | Changes |
|------|---------|
| `src-tauri/src/ui/commands.rs` | Add `start_inline_oauth_flow`, `poll_inline_oauth_status`, `cancel_inline_oauth_flow` |
| `src-tauri/src/main.rs` | Register new commands |
| `src/views/resources/mcp-servers-panel.tsx` | Form reorder, new auth options, inline OAuth UI, cleanup |

---

## Verification

1. **Create MCP server with OAuth (Pre-generated)**:
   - Enter URL, client ID, client secret
   - Server should save and be able to authenticate

2. **Create MCP server with OAuth (External browser)**:
   - Enter URL, optionally client ID/secret
   - Click Authorize → browser opens → complete auth → success shown
   - Server saved with OAuth credentials

3. **Cleanup on navigation**:
   - Start OAuth flow, then close modal
   - Callback server should shut down (check with `lsof -i :8080`)

4. **Re-authorize existing server**:
   - Open MCP server detail view
   - Click Re-authorize → completes flow → tokens refreshed

5. **Error handling**:
   - Invalid URL → "MCP server does not support OAuth"
   - User denies auth → "Authorization denied"
   - Timeout (5 min) → "Authorization timed out"
