# MCP Authentication & Unified Client Architecture - Implementation Plan

**Date**: 2026-01-17
**Status**: ⚠️ PARTIAL IMPLEMENTATION - Phase 1 mostly complete, Phases 2-5 not started
**Last Updated**: 2026-01-17 (status review)
**Goal**: Redesign authentication architecture to properly handle MCP server authentication and unify API keys with OAuth clients into a single "Client" concept.

## Implementation Status Summary

**✅ COMPLETED:**
- Phase 1.1: Unified Client System (8/8 items)
- Phase 1.3: Config Migration (4/5 items - testing remains)
- OAuth token endpoint for client authentication

**⚠️ PARTIAL:**
- Phase 1.4: Tauri Commands (1/5 items complete)
- Access control methods exist but not enforced in middleware

**❌ NOT STARTED:**
- Phase 1.2: MCP Server Config updates (0/5 items)
- Phase 2: MCP Server Authentication (0/20 items)
- Phase 3: UI Redesign (0/15 items)
- Phase 4: Authentication Middleware (1/11 items)
- Phase 5: Testing & Documentation (1/15 items)

**Key Missing Components:**
1. `McpAuthConfig` enum not defined
2. MCP server authentication configuration
3. WebSocket transport still exists (should be removed)
4. UI not updated (separate tabs still exist, no unified Clients tab)
5. Supergateway connection examples not shown in UI
6. Access control not enforced

---

## Problem Statement

### Current Issues

1. **MCP Server Auth is Auto-Discovery Only**
   - Only supports auto-discovered OAuth via `.well-known/oauth-protected-resource`
   - No way to manually configure bearer tokens, custom headers, or pre-registered OAuth
   - Doesn't match real-world MCP server authentication patterns

2. **Separate API Keys and OAuth Clients**
   - API Keys are for LLM routing only
   - OAuth Clients are for MCP proxy authentication only
   - Fragmented user experience, duplicated concepts

3. **WebSocket Transport Unnecessary**
   - Only need STDIO and HTTP/SSE based on real MCP implementations
   - WebSocket adds complexity without benefit

4. **Client Auth to LocalRouter Not Configurable**
   - No way to configure how external clients authenticate TO LocalRouter
   - Should support: Bearer token, OAuth, or None (localhost only)

---

## Architecture Overview

### Unified Flow

```
External Client → [Auth to LocalRouter] → LocalRouter → [Auth to MCP Server] → External MCP Server
                        ↓                                        ↓
                   Client entity                         McpAuthConfig
                   (Bearer/OAuth)                        (Bearer/OAuth/Headers)
```

**Key Principle**: LocalRouter is a man-in-the-middle proxy that:
1. Authenticates incoming clients (inbound auth)
2. Re-packages requests with its own credentials (outbound auth)
3. Proxies to external MCP servers

---

## New Architecture

### 1. Unified Client Entity (SIMPLIFIED)

Replace `ApiKeyConfig` and `OAuthClientConfig` with a single `Client` concept:

```rust
/// Unified client entity for accessing LocalRouter
/// Can access both LLM routing and MCP servers
/// Uses ONE secret for all authentication methods
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Client {
    /// Unique identifier (internal, UUID)
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// OAuth Client ID (visible, stored in config file)
    /// Generated automatically, format: "lr-..." (32 chars)
    /// Used for OAuth client credentials flow
    pub client_id: String,

    /// Reference to client secret in keychain
    /// Actual secret stored in keyring: service="LocalRouter-Clients", account=client.id
    /// This ONE secret is used for:
    /// - LLM access via Bearer token: Authorization: Bearer {secret}
    /// - MCP access via Bearer token: Authorization: Bearer {secret}
    /// - MCP access via OAuth: client_secret={secret} (to get temporary token)
    pub secret_ref: String,

    /// Whether this client is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    // Access Control

    /// LLM providers this client can access
    /// Empty = no LLM access
    #[serde(default)]
    pub allowed_llm_providers: Vec<String>,

    /// MCP servers this client can access (by server ID)
    /// Empty = no MCP access
    #[serde(default)]
    pub allowed_mcp_servers: Vec<String>,

    // Metadata

    /// When this client was created
    pub created_at: DateTime<Utc>,

    /// Last time this client was used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<DateTime<Utc>>,
}
```

**Key Simplifications**:
- **One Secret**: Each client has ONE secret (not separate API key and OAuth secret)
- **Client ID in Config**: Stored in config file, not keychain (visible to user)
- **Two Connection Methods**: Client chooses how to connect (direct bearer OR OAuth flow)
- **No Auth Method Enum**: All clients authenticate the same way (secret required)

**Client Access Patterns**:

1. **LLM Access** (Direct Bearer Token):
   ```
   POST /v1/chat/completions
   Authorization: Bearer {client_secret}
   ```

2. **MCP Access - Method 1** (Direct Bearer Token):
   ```
   POST /mcp/{server_id}
   Authorization: Bearer {client_secret}
   ```

3. **MCP Access - Method 2** (OAuth Client Credentials Flow):
   ```
   Step 1: Get temporary token
   POST /oauth/token
   Content-Type: application/x-www-form-urlencoded

   grant_type=client_credentials
   &client_id={client_id}
   &client_secret={client_secret}

   Response: {"access_token": "temp-xyz", "expires_in": 3600}

   Step 2: Use temporary token
   POST /mcp/{server_id}
   Authorization: Bearer temp-xyz
   ```

**Migration Strategy**:
- Existing `ApiKeyConfig` → `Client`:
  - Generate new `client_id`
  - Keep existing key hash as `secret` (or regenerate)
  - `allowed_llm_providers` from `model_selection`
  - `allowed_mcp_servers = []`
- Existing `OAuthClientConfig` → `Client`:
  - Keep existing `client_id`
  - Keep existing secret
  - `allowed_llm_providers = []`
  - `allowed_mcp_servers` from `linked_server_ids`

---

### 2. MCP Server Outbound Authentication

Update `McpServerConfig` to support manual authentication configuration:

```rust
/// MCP server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,

    /// Transport type (STDIO or HTTP/SSE only, WebSocket removed)
    pub transport: McpTransportType,
    pub transport_config: McpTransportConfig,

    /// Manual authentication configuration
    /// How LocalRouter authenticates TO this MCP server (outbound)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_config: Option<McpAuthConfig>,

    /// Auto-discovered OAuth configuration (legacy, for auto-detection)
    /// Populated automatically if server has .well-known/oauth-protected-resource
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovered_oauth: Option<McpOAuthDiscovery>,

    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

/// Transport type (WebSocket REMOVED)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpTransportType {
    /// STDIO transport (spawn subprocess)
    Stdio,

    /// HTTP with Server-Sent Events
    HttpSse,
}

/// Authentication configuration for MCP servers (outbound)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpAuthConfig {
    /// No authentication required
    None,

    /// Bearer token authentication (Authorization: Bearer {token})
    BearerToken {
        /// Reference to token in keychain
        /// Stored in keyring: service="LocalRouter-McpServers", account=server.id
        token_ref: String,
    },

    /// Custom headers (can include auth headers)
    CustomHeaders {
        /// Headers to send with every request
        /// Can include: Authorization, X-API-Key, etc.
        headers: HashMap<String, String>,
    },

    /// Pre-registered OAuth credentials
    OAuth {
        /// OAuth client ID
        client_id: String,

        /// Reference to client secret in keychain
        client_secret_ref: String,

        /// Authorization endpoint URL
        auth_url: String,

        /// Token endpoint URL
        token_url: String,

        /// OAuth scopes to request
        scopes: Vec<String>,
    },

    /// Environment variables (for STDIO only)
    /// Can include API keys, tokens, etc.
    EnvVars {
        /// Environment variables to pass to subprocess
        /// Merged with transport_config.env
        env: HashMap<String, String>,
    },
}

/// Auto-discovered OAuth info (from .well-known endpoint)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpOAuthDiscovery {
    pub auth_url: String,
    pub token_url: String,
    pub scopes_supported: Vec<String>,
    pub discovered_at: DateTime<Utc>,
}
```

**Key Changes**:
- `WebSocket` transport removed
- New `auth_config` field for manual authentication
- Separate `discovered_oauth` for auto-detection (kept for compatibility)
- `McpAuthConfig` enum covers all authentication methods

---

### 3. OAuth Flow Implementation (for MCP Server Auth)

When user clicks "Authenticate with OAuth" for an MCP server:

**Challenge**: Tauri desktop app needs to handle OAuth callback

**Solution**: Temporary local web server for OAuth callback

```rust
/// OAuth flow handler for desktop app
pub struct OAuthFlowHandler {
    /// HTTP client for OAuth requests
    client: Client,

    /// Port range for temporary callback server (e.g., 8000-8100)
    callback_port_range: (u16, u16),
}

impl OAuthFlowHandler {
    /// Initiate OAuth flow for an MCP server
    /// Returns the acquired access token and refresh token (if available)
    pub async fn initiate_flow(
        &self,
        auth_url: String,
        token_url: String,
        client_id: String,
        client_secret: String,
        scopes: Vec<String>,
    ) -> AppResult<OAuthTokens> {
        // 1. Start temporary callback server on available port
        let (callback_url, receiver) = self.start_callback_server().await?;

        // 2. Generate PKCE challenge and verifier
        let (pkce_challenge, pkce_verifier) = generate_pkce();

        // 3. Build authorization URL
        let auth_redirect = format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256",
            auth_url, client_id, callback_url, scopes.join(" "), pkce_challenge
        );

        // 4. Open browser to authorization URL
        open::that(&auth_redirect)?;

        // 5. Wait for callback with authorization code (timeout 5 min)
        let auth_code = tokio::time::timeout(
            Duration::from_secs(300),
            receiver
        ).await??;

        // 6. Exchange authorization code for access token
        let tokens = self.exchange_code_for_token(
            token_url,
            client_id,
            client_secret,
            auth_code,
            callback_url,
            pkce_verifier,
        ).await?;

        Ok(tokens)
    }

    /// Start temporary HTTP server to receive OAuth callback
    async fn start_callback_server(&self) -> AppResult<(String, oneshot::Receiver<String>)> {
        // Try ports in range until one is available
        for port in self.callback_port_range.0..=self.callback_port_range.1 {
            if let Ok((callback_url, receiver)) = self.try_start_server(port).await {
                return Ok((callback_url, receiver));
            }
        }

        Err(AppError::Mcp("No available ports for OAuth callback".into()))
    }

    async fn try_start_server(&self, port: u16) -> AppResult<(String, oneshot::Receiver<String>)> {
        let (tx, rx) = oneshot::channel();

        let listener = TcpListener::bind(("127.0.0.1", port)).await?;
        let callback_url = format!("http://127.0.0.1:{}/oauth/callback", port);

        tokio::spawn(async move {
            // Accept ONE connection, extract code, send success page, shutdown
            if let Ok((stream, _)) = listener.accept().await {
                // Parse HTTP request, extract ?code=... parameter
                // Send HTML success page
                // Send code through channel
                let _ = tx.send(code);
            }
        });

        Ok((callback_url, rx))
    }
}
```

**Libraries Needed**:
- `oauth2` crate - OAuth 2.0 flow implementation
- `tokio::net::TcpListener` - Temporary callback server (already have)
- `open` crate - Open browser (Tauri has this via shell plugin)
- PKCE implementation (can use `oauth2` crate)

**UI Flow**:
1. User configures MCP server with OAuth
2. Enters auth_url, token_url, client_id, client_secret, scopes
3. Clicks "Authenticate Now" button
4. Browser opens to authorization page
5. User approves
6. Callback captured, token acquired
7. Token stored in keychain
8. UI shows "Authenticated ✓"

---

### 4. OAuth Token Endpoint & In-Memory Token Store

**Purpose**: Allow clients to use OAuth client credentials flow for temporary access tokens

**OAuth Token Endpoint** (`POST /oauth/token`):

```rust
/// OAuth token endpoint for client credentials flow
#[derive(Deserialize)]
struct TokenRequest {
    grant_type: String,
    client_id: String,
    client_secret: String,
}

#[derive(Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: i64,
}

pub async fn oauth_token(
    form: web::Form<TokenRequest>,
    client_manager: web::Data<Arc<ClientManager>>,
    token_store: web::Data<Arc<TokenStore>>,
) -> Result<HttpResponse, actix_web::Error> {
    // 1. Validate grant_type
    if form.grant_type != "client_credentials" {
        return Ok(HttpResponse::BadRequest().json(json!({
            "error": "unsupported_grant_type"
        })));
    }

    // 2. Verify client credentials
    let client = client_manager
        .verify_credentials(&form.client_id, &form.client_secret)
        .ok_or_else(|| actix_web::error::ErrorUnauthorized("invalid_client"))?;

    // 3. Check if enabled
    if !client.enabled {
        return Err(actix_web::error::ErrorUnauthorized("client_disabled"));
    }

    // 4. Generate temporary token (1 hour)
    let access_token = generate_secure_token();
    let expires_in = 3600;

    // 5. Store in memory
    token_store.store(access_token.clone(), client.id.clone(), expires_in);

    // 6. Return token
    Ok(HttpResponse::Ok().json(TokenResponse {
        access_token,
        token_type: "Bearer".to_string(),
        expires_in,
    }))
}
```

**In-Memory Token Store**:

```rust
/// Temporary OAuth token storage (in-memory only)
pub struct TokenStore {
    /// Map: access_token -> (client_id, expiration_time)
    tokens: Arc<RwLock<HashMap<String, (String, DateTime<Utc>)>>>,
}

impl TokenStore {
    pub fn new() -> Self {
        let store = Self {
            tokens: Arc::new(RwLock::new(HashMap::new())),
        };
        // Start background cleanup task
        store.start_cleanup_task();
        store
    }

    /// Store a temporary token
    pub fn store(&self, token: String, client_id: String, expires_in: i64) {
        let expiration = Utc::now() + Duration::seconds(expires_in);
        self.tokens.write().insert(token, (client_id, expiration));
    }

    /// Verify token and return client_id
    pub fn verify(&self, token: &str) -> Option<String> {
        let tokens = self.tokens.read();
        if let Some((client_id, expiration)) = tokens.get(token) {
            if *expiration > Utc::now() {
                return Some(client_id.clone());
            }
        }
        None
    }

    /// Background cleanup of expired tokens (runs every 5 minutes)
    fn start_cleanup_task(&self) {
        let tokens = self.tokens.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            loop {
                interval.tick().await;
                let now = Utc::now();
                tokens.write().retain(|_, (_, exp)| *exp > now);
            }
        });
    }
}
```

**Auth Middleware Update**:

```rust
/// Authenticate client from request
/// Supports both direct secret and temporary OAuth tokens
pub async fn authenticate_client(
    req: &HttpRequest,
    client_manager: &ClientManager,
    token_store: &TokenStore,
) -> Result<Client, AuthError> {
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .ok_or(AuthError::MissingAuth)?;

    if !auth_header.starts_with("Bearer ") {
        return Err(AuthError::InvalidFormat);
    }

    let token = &auth_header[7..];

    // Try temporary OAuth token first
    if let Some(client_id) = token_store.verify(token) {
        if let Some(client) = client_manager.get_client(&client_id) {
            if client.enabled {
                return Ok(client);
            }
        }
        return Err(AuthError::ClientDisabled);
    }

    // Otherwise, verify as direct client secret
    if let Some(client) = client_manager.verify_secret(token) {
        if client.enabled {
            return Ok(client);
        }
        return Err(AuthError::ClientDisabled);
    }

    Err(AuthError::InvalidToken)
}
```

**Key Points**:
- Tokens are stored **in-memory only** (lost on restart - by design)
- 1 hour expiration (3600 seconds)
- Background cleanup every 5 minutes
- Auth middleware checks temporary tokens first, then direct secret
- Standard OAuth 2.0 error responses

---

### 5. Transport Configuration Updates

**Remove WebSocket**:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransportConfig {
    /// STDIO process configuration
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        /// Base environment variables (auth env vars go in McpAuthConfig::EnvVars)
        #[serde(default)]
        env: HashMap<String, String>,
    },

    /// HTTP with Server-Sent Events
    HttpSse {
        url: String,
        /// Base headers (auth headers go in McpAuthConfig::CustomHeaders or BearerToken)
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}
```

**Key Point**: Separate base config from auth config
- `transport_config.env` = non-auth environment variables
- `auth_config.EnvVars.env` = authentication environment variables (merged at runtime)
- Same for headers

---

## Implementation Phases

### Phase 1: Backend Foundation (Week 1)

**1.1 Create Unified Client System**
- [x] Create `Client` struct in `src-tauri/src/config/mod.rs` ✅
- [x] Create `ClientManager` in `src-tauri/src/clients/mod.rs` ✅
- [x] Implement client creation (auto-generate client_id and secret) ✅
- [x] Implement client secret verification (bearer token) ✅
- [x] Implement OAuth token store (in-memory, short-lived tokens) ✅
- [x] Implement OAuth token endpoint (`POST /oauth/token`) ✅
- [ ] Implement access control checks (`can_access_llm`, `can_access_mcp_server`) ⚠️ PARTIAL - Methods exist but not enforced in middleware
- [x] Store client secrets in keychain (service="LocalRouter-Clients") ✅

**1.2 Update MCP Server Config**
- [ ] Add `McpAuthConfig` enum ❌ NOT DONE
- [ ] Add `auth_config` field to `McpServerConfig` ❌ NOT DONE
- [ ] Rename `oauth_config` → `discovered_oauth` ❌ NOT DONE
- [ ] Remove `WebSocket` from `McpTransportType` ❌ NOT DONE - WebSocket still exists
- [ ] Rename `Sse` → `HttpSse` in transport types ❌ NOT DONE

**1.3 Config Migration**
- [x] Create migration function to convert old config to new format ✅
- [x] Convert `ApiKeyConfig` → `Client` (inbound_auth = BearerToken) ✅
- [x] Convert `OAuthClientConfig` → `Client` (inbound_auth = OAuth) ✅
- [x] Update config version number ✅ (version = 2)
- [ ] Test migration with sample configs ⚠️ NEEDS TESTING

**1.4 Update Tauri Commands**
- [x] Replace `create_api_key` + `create_oauth_client` with `create_client` ✅
- [ ] Add `update_client`, `delete_client`, `list_clients` ⚠️ PARTIAL - delete exists, others TBD
- [ ] Add `add_client_llm_access`, `remove_client_llm_access` ❌ NOT DONE
- [ ] Add `add_client_mcp_access`, `remove_client_mcp_access` ❌ NOT DONE
- [ ] Update all existing commands to use new Client system ⚠️ IN PROGRESS

### Phase 2: MCP Server Authentication (Week 2) ❌ NOT STARTED

**2.1 Implement Auth Config Handling**
- [ ] Update `McpServerManager` to apply auth_config when connecting ❌ NOT DONE
- [ ] Implement `BearerToken` injection for HTTP/SSE ❌ NOT DONE
- [ ] Implement `CustomHeaders` application ❌ NOT DONE
- [ ] Implement `EnvVars` merging for STDIO ❌ NOT DONE
- [ ] Implement pre-registered `OAuth` token acquisition ❌ NOT DONE

**2.2 OAuth Flow Implementation**
- [ ] Add `oauth2` crate dependency ❌ NOT DONE
- [ ] Create `OAuthFlowHandler` in `src-tauri/src/mcp/oauth_flow.rs` ❌ NOT DONE
- [ ] Implement PKCE generation ❌ NOT DONE
- [ ] Implement temporary callback server ❌ NOT DONE
- [ ] Implement code-to-token exchange ❌ NOT DONE
- [ ] Implement token storage in keychain ❌ NOT DONE
- [ ] Implement token refresh logic ❌ NOT DONE

**2.3 OAuth Discovery (Legacy)**
- [ ] Keep existing auto-discovery for `.well-known/oauth-protected-resource` ⚠️ EXISTS - may need updates
- [ ] Store discovered config in `discovered_oauth` field ❌ NOT DONE - still called `oauth_config`
- [ ] Show in UI as "Auto-detected OAuth" (read-only) ❌ NOT DONE
- [ ] Allow user to override with manual OAuth config ❌ NOT DONE

**2.4 Test All Auth Methods**
- [ ] Test STDIO with env vars ❌ NOT DONE
- [ ] Test HTTP/SSE with bearer token ❌ NOT DONE
- [ ] Test HTTP/SSE with custom headers ❌ NOT DONE
- [ ] Test HTTP/SSE with pre-registered OAuth ❌ NOT DONE
- [ ] Test HTTP/SSE with OAuth flow ❌ NOT DONE
- [ ] Test OAuth token refresh ❌ NOT DONE

### Phase 3: UI Redesign (Week 3) ❌ NOT STARTED

**3.1 Unified Clients Tab**
- [ ] Create `ClientsTab.tsx` (replaces ApiKeysTab + OAuthClientsTab) ❌ NOT DONE - Both tabs still exist separately
- [ ] Client creation modal: ❌ NOT DONE
  - Name input
  - Auth method selector (Bearer Token, OAuth, None)
  - Generate credentials button
  - Show credentials modal (copy api_key, client_id/secret)
- [ ] Client list view: ❌ NOT DONE
  - Show name, auth method, LLM access count, MCP access count
  - Enable/disable toggle
  - Delete button
- [ ] Client detail page: ❌ NOT DONE
  - Show credentials (masked, with copy buttons)
  - Rotate credentials button
  - LLM access section (add/remove providers)
  - MCP access section (add/remove servers)
  - Last used timestamp

**3.2 MCP Servers Tab Update**
- [ ] Update creation modal with auth section: ❌ NOT DONE
  - Transport type: STDIO or HTTP/SSE (remove WebSocket) ⚠️ WebSocket still shown in UI
  - STDIO: command, args, base env vars
  - HTTP/SSE: URL, base headers
  - Auth method selector:
    - None
    - Bearer Token (input + store in keychain)
    - Custom Headers (key-value pairs)
    - Pre-registered OAuth (client_id, client_secret, urls, scopes)
    - OAuth Flow (button to trigger, shows status)
  - Test connection button
- [ ] Update detail page: ❌ NOT DONE
  - Show transport config
  - Show auth config (with masked secrets)
  - Show discovered OAuth (if any) - read-only
  - Re-authenticate button (for OAuth flow)
  - Test connection button
- [ ] **ADD Supergateway connection examples in UI** ❌ NEW REQUIREMENT

**3.3 Navigation Updates**
- [ ] Update Sidebar: ❌ NOT DONE
  - Rename "API Keys" → "Clients"
  - Remove "OAuth Clients" (merged)
  - Keep "MCP Servers"
- [ ] Update routing to handle new tab names

**3.4 Auth Flow UI**
- [ ] Create OAuth flow modal: ❌ NOT DONE
  - "Authenticating..." spinner
  - "Opening browser..." message
  - "Waiting for authorization..." status
  - Success/error messages
  - Cancel button (stops callback server)
- [ ] Handle OAuth callback success ❌ NOT DONE
- [ ] Handle OAuth callback failure/timeout ❌ NOT DONE

### Phase 4: Authentication Middleware (Week 4) ❌ NOT STARTED

**4.1 Inbound Auth (Client → LocalRouter)**
- [ ] Update middleware to check Client auth: ⚠️ PARTIAL - Basic implementation exists
  - Extract bearer token or OAuth credentials from request
  - Verify against ClientManager
  - Check if client is enabled
  - Store client_id in request context
- [x] Implement OAuth token endpoint for clients ✅ EXISTS in routes/oauth.rs
- [ ] Test all three auth methods (Bearer, OAuth, None) ❌ NOT DONE

**4.2 Access Control**
- [ ] LLM routing: Check `client.allowed_llm_providers` ❌ NOT ENFORCED
- [ ] MCP proxy: Check `client.allowed_mcp_servers` ❌ NOT ENFORCED
- [ ] Return 403 Forbidden if access denied ❌ NOT DONE
- [ ] Log access attempts ❌ NOT DONE

**4.3 Outbound Auth (LocalRouter → MCP Server)**
- [ ] Apply `auth_config` when making MCP requests ❌ NOT DONE - auth_config doesn't exist yet
- [ ] For BearerToken: Add `Authorization: Bearer {token}` header ❌ NOT DONE
- [ ] For CustomHeaders: Merge with base headers ❌ NOT DONE
- [ ] For OAuth: Get token from cache/refresh if expired, add `Authorization: Bearer {token}` ❌ NOT DONE
- [ ] For EnvVars (STDIO): Merge with base env vars ❌ NOT DONE
- [ ] Handle auth failures (401/403) and log ❌ NOT DONE

### Phase 5: Testing & Documentation (Week 5) ❌ NOT STARTED

**5.1 Integration Tests**
- [ ] Test client creation and management ❌ NOT DONE
- [ ] Test client auth verification (all methods) ❌ NOT DONE
- [ ] Test access control (LLM + MCP) ❌ NOT DONE
- [ ] Test MCP server auth (all methods) ❌ NOT DONE
- [ ] Test OAuth flow end-to-end ❌ NOT DONE
- [ ] Test config migration ❌ NOT DONE

**5.2 Manual Testing**
- [ ] Test with real MCP servers (STDIO and HTTP/SSE) ❌ NOT DONE
- [ ] Test OAuth flow with real OAuth provider (if possible, or mock) ❌ NOT DONE
- [ ] Test client access from external app ❌ NOT DONE
- [ ] Test all UI flows ❌ NOT DONE

**5.3 Documentation**
- [ ] Update README with new architecture ❌ NOT DONE
- [ ] Document client creation and management ❌ NOT DONE
- [ ] Document MCP server authentication options ❌ NOT DONE
- [ ] Document OAuth flow setup ❌ NOT DONE
- [ ] Create migration guide for existing users ❌ NOT DONE
- [x] Document Supergateway connection examples ✅ EXISTS in MCP_CONNECTION_EXAMPLES.md

---

## UI Mockups

### Client Creation Flow

```
┌─────────────────────────────────────────┐
│ Create Client                     [X]   │
├─────────────────────────────────────────┤
│                                         │
│ Client Name                             │
│ ┌─────────────────────────────────────┐ │
│ │ My App                              │ │
│ └─────────────────────────────────────┘ │
│                                         │
│ Authentication Method                   │
│ ○ Bearer Token (API Key)                │
│ ○ OAuth (Client Credentials)            │
│ ○ None (Localhost only) ⚠️               │
│                                         │
│         [Cancel]  [Create Client]       │
└─────────────────────────────────────────┘

After creation:

┌─────────────────────────────────────────┐
│ Client Created Successfully!            │
├─────────────────────────────────────────┤
│                                         │
│ ⚠️ Save these credentials securely.     │
│ They will not be shown again!           │
│                                         │
│ API Key (Bearer Token):                 │
│ ┌─────────────────────────────────────┐ │
│ │ lr-AbCd...XyZ                [Copy] │ │
│ └─────────────────────────────────────┘ │
│                                         │
│ OAuth Client ID:                        │
│ ┌─────────────────────────────────────┐ │
│ │ lr-EfGh...WxY                [Copy] │ │
│ └─────────────────────────────────────┘ │
│                                         │
│ OAuth Client Secret:                    │
│ ┌─────────────────────────────────────┐ │
│ │ lr-IjKl...VwX                [Copy] │ │
│ └─────────────────────────────────────┘ │
│                                         │
│                 [Done]                  │
└─────────────────────────────────────────┘
```

### MCP Server Auth Configuration

```
┌─────────────────────────────────────────┐
│ Create MCP Server                  [X]  │
├─────────────────────────────────────────┤
│                                         │
│ Server Name                             │
│ ┌─────────────────────────────────────┐ │
│ │ OpenAI MCP Server                   │ │
│ └─────────────────────────────────────┘ │
│                                         │
│ Transport Type                          │
│ ○ STDIO (Local subprocess)              │
│ ● HTTP/SSE (Remote server)              │
│                                         │
│ [HTTP/SSE Configuration]                │
│                                         │
│ Server URL                              │
│ ┌─────────────────────────────────────┐ │
│ │ https://api.example.com/mcp         │ │
│ └─────────────────────────────────────┘ │
│                                         │
│ Base Headers (optional)                 │
│ ┌─────────────────────────────────────┐ │
│ │ Content-Type: application/json      │ │
│ └─────────────────────────────────────┘ │
│                                         │
│ [Authentication]                        │
│                                         │
│ Auth Method                             │
│ ○ None                                  │
│ ○ Bearer Token                          │
│ ○ Custom Headers                        │
│ ● Pre-registered OAuth                  │
│ ○ OAuth Flow (Browser)                  │
│                                         │
│ [Pre-registered OAuth]                  │
│                                         │
│ Client ID                               │
│ ┌─────────────────────────────────────┐ │
│ │ my-client-id                        │ │
│ └─────────────────────────────────────┘ │
│                                         │
│ Client Secret                           │
│ ┌─────────────────────────────────────┐ │
│ │ ••••••••••••••••••           [Show] │ │
│ └─────────────────────────────────────┘ │
│                                         │
│ Authorization URL                       │
│ ┌─────────────────────────────────────┐ │
│ │ https://auth.example.com/authorize  │ │
│ └─────────────────────────────────────┘ │
│                                         │
│ Token URL                               │
│ ┌─────────────────────────────────────┐ │
│ │ https://auth.example.com/token      │ │
│ └─────────────────────────────────────┘ │
│                                         │
│ Scopes (space-separated)                │
│ ┌─────────────────────────────────────┐ │
│ │ read write                          │ │
│ └─────────────────────────────────────┘ │
│                                         │
│         [Test Connection]               │
│         [Cancel]  [Create Server]       │
└─────────────────────────────────────────┘
```

---

## Migration Checklist

### Config File Migration

**Before** (`~/.config/localrouter/config.yaml`):
```yaml
api_keys:
  - id: "key1"
    name: "My App"
    key_hash: "..."
    model_selection: {...}

oauth_clients:
  - id: "oauth1"
    name: "MCP Client"
    client_id: "lr-..."
    linked_server_ids: ["server1"]

mcp_servers:
  - id: "server1"
    name: "My MCP"
    transport: "sse"
    transport_config:
      type: "sse"
      url: "..."
      headers: {...}
    oauth_config:  # auto-discovered
      auth_url: "..."
      token_url: "..."
```

**After**:
```yaml
version: 2  # Increment config version

clients:
  - id: "key1"
    name: "My App"
    api_key: "lr-..."  # Re-generate or keep hash
    oauth_client_id: null
    oauth_client_secret_ref: null
    inbound_auth_method: "bearer_token"
    enabled: true
    allowed_llm_providers: [...]  # From model_selection
    allowed_mcp_servers: []
    created_at: "..."

  - id: "oauth1"
    name: "MCP Client"
    api_key: "lr-..."  # Generate new
    oauth_client_id: "lr-..."
    oauth_client_secret_ref: "oauth1"
    inbound_auth_method: "oauth"
    enabled: true
    allowed_llm_providers: []
    allowed_mcp_servers: ["server1"]
    created_at: "..."

mcp_servers:
  - id: "server1"
    name: "My MCP"
    transport: "http_sse"  # Renamed
    transport_config:
      type: "http_sse"
      url: "..."
      headers: {}  # Move auth headers to auth_config
    auth_config:  # NEW - extract from headers if present
      type: "custom_headers"
      headers: {...}
    discovered_oauth:  # Renamed from oauth_config
      auth_url: "..."
      token_url: "..."
      discovered_at: "..."
    enabled: true
    created_at: "..."
```

### Keychain Migration

**Before**:
- Service: `LocalRouter-ApiKeys`, Account: `{key_id}` → API key value
- Service: `LocalRouter-OAuthClients`, Account: `{client_id}` → OAuth secret
- Service: `LocalRouter-McpServerTokens`, Account: `{server_id}_client_secret` → OAuth secret

**After**:
- Service: `LocalRouter-Clients`, Account: `{client_id}` → OAuth client secret (if OAuth auth)
- Service: `LocalRouter-McpServers`, Account: `{server_id}` → Auth credentials (bearer token, OAuth secret, etc.)
- Service: `LocalRouter-McpServers`, Account: `{server_id}_oauth_token` → OAuth access token (if using OAuth)

### Database Migration Function

```rust
async fn migrate_config_v1_to_v2(old_config: ConfigV1) -> AppResult<Config> {
    let mut clients = Vec::new();
    let mut mcp_servers = old_config.mcp_servers.clone();

    // Migrate API Keys → Clients (Bearer Token)
    for api_key in old_config.api_keys {
        let client = Client {
            id: api_key.id.clone(),
            name: api_key.name,
            api_key: generate_api_key(),  // Generate new or use hash
            oauth_client_id: None,
            oauth_client_secret_ref: None,
            inbound_auth_method: ClientAuthMethod::BearerToken,
            enabled: true,
            allowed_llm_providers: extract_allowed_providers(&api_key.model_selection),
            allowed_mcp_servers: vec![],
            created_at: Utc::now(),
            last_used: None,
        };
        clients.push(client);
    }

    // Migrate OAuth Clients → Clients (OAuth)
    for oauth_client in old_config.oauth_clients {
        let client = Client {
            id: oauth_client.id.clone(),
            name: oauth_client.name,
            api_key: generate_api_key(),
            oauth_client_id: Some(oauth_client.client_id.clone()),
            oauth_client_secret_ref: Some(oauth_client.id.clone()),
            inbound_auth_method: ClientAuthMethod::OAuth,
            enabled: oauth_client.enabled,
            allowed_llm_providers: vec![],
            allowed_mcp_servers: oauth_client.linked_server_ids,
            created_at: oauth_client.created_at,
            last_used: oauth_client.last_used,
        };

        // Migrate secret from old keychain location to new
        if let Some(secret) = get_from_keychain("LocalRouter-OAuthClients", &oauth_client.id)? {
            store_in_keychain("LocalRouter-Clients", &client.id, &secret)?;
        }

        clients.push(client);
    }

    // Migrate MCP Servers
    for server in &mut mcp_servers {
        // Rename transport type
        if server.transport == McpTransportType::Sse {
            server.transport = McpTransportType::HttpSse;
        }

        // Move oauth_config → discovered_oauth
        if let Some(oauth) = server.oauth_config.take() {
            server.discovered_oauth = Some(McpOAuthDiscovery {
                auth_url: oauth.auth_url,
                token_url: oauth.token_url,
                scopes_supported: oauth.scopes,
                discovered_at: Utc::now(),
            });
        }

        // Extract auth from headers/env and create auth_config
        // This is a heuristic - may need manual review
        server.auth_config = extract_auth_config(&server.transport_config);
    }

    Ok(Config {
        version: 2,
        clients,
        mcp_servers,
        ..old_config
    })
}
```

---

## Open Questions & Decisions Needed

1. **OAuth Client Registration**
   - Should we support Dynamic Client Registration (RFC 7591)?
   - Or assume users always have pre-registered credentials?
   - **Recommendation**: Start with pre-registered only, add DCR later if needed

2. **OAuth Token Refresh**
   - Automatic refresh on expiration?
   - Background task to refresh before expiry?
   - **Recommendation**: Automatic refresh on next request, with background refresh 5 min before expiry

3. **OAuth Scope Management**
   - Free-form text input?
   - Pre-defined scope suggestions based on common MCP servers?
   - **Recommendation**: Free-form with common suggestions (read, write, admin)

4. **Client Auth - "None" Method**
   - Allow for localhost only?
   - Show warning in UI?
   - Require explicit acknowledgement?
   - **Recommendation**: Show big warning, require checkbox "I understand this is insecure"

5. **Backward Compatibility**
   - Support old config format alongside new for transition period?
   - Auto-migrate on first run?
   - **Recommendation**: Auto-migrate with backup of old config

6. **OAuth Token Storage**
   - Store access token in keychain or in-memory cache?
   - Store refresh token in keychain?
   - **Recommendation**: Both in keychain, cache in-memory for performance

---

## Success Criteria

### Functional Requirements
- ✅ Single "Client" concept replaces API Keys + OAuth Clients
- ✅ Clients can access both LLMs and MCP servers
- ✅ Clients support Bearer Token, OAuth, or None auth methods
- ✅ MCP servers support 5 auth methods: None, Bearer, Custom Headers, Pre-registered OAuth, OAuth Flow
- ✅ WebSocket transport removed, only STDIO and HTTP/SSE
- ✅ OAuth flow works end-to-end with callback handling
- ✅ Config migration preserves all existing functionality
- ✅ UI clearly separates inbound auth (to LocalRouter) from outbound auth (to MCP servers)

### Non-Functional Requirements
- ✅ Migration completes without data loss
- ✅ OAuth tokens stored securely in keychain
- ✅ UI is intuitive and clearly explains auth options
- ✅ All existing API endpoints continue to work (or have migration path)
- ✅ Performance does not degrade (token caching)

---

## Next Steps

**Immediate**:
1. Review this plan with user - confirm approach and priorities
2. Clarify open questions
3. Set timeline and milestones

**Short-term** (This Week):
1. Create feature branch: `feature/unified-auth-architecture`
2. Start Phase 1: Backend foundation
3. Implement Client struct and manager
4. Create migration function

**Medium-term** (Next 2 Weeks):
1. Complete Phases 1-2 (Backend)
2. Begin Phase 3 (UI)
3. Test OAuth flow implementation

**Long-term** (Next Month):
1. Complete all phases
2. Comprehensive testing
3. Documentation
4. Release v2.0 with unified architecture

---

**Plan Status**: 📝 Draft - Awaiting Review
**Next Review**: 2026-01-17
**Owner**: To be assigned
