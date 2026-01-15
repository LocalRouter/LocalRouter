# OpenCode LLM Provider Implementation - Research Report

**Date**: 2026-01-15
**Source**: OpenCode repository analysis (../opencode)
**Purpose**: Understanding provider architecture for LocalRouter AI implementation

---

## Executive Summary

OpenCode supports **75+ LLM providers** through a flexible plugin-based architecture combining:
- AI SDK provider adapters (@ai-sdk/* packages)
- OAuth authentication flows for subscription services
- API key management with secure storage
- Dynamic provider loading from models.dev

---

## 1. Supported Providers

### Bundled Providers (Direct Integration)

| Provider | Package | Auth Method | Notes |
|----------|---------|-------------|-------|
| **Anthropic** | `@ai-sdk/anthropic` | API Key + OAuth (Pro/Max) | Claude models, subscription support |
| **OpenAI** | `@ai-sdk/openai` | API Key + OAuth (Codex) | ChatGPT models, Codex for Plus/Pro |
| **GitHub Copilot** | `@ai-sdk/github-copilot` | OAuth | Device code flow, enterprise support |
| **Google Gemini** | `@ai-sdk/google` | API Key | Standard Google AI |
| **Google Vertex** | `@ai-sdk/google-vertex` | OAuth | GCP authentication |
| **Vertex Anthropic** | `@ai-sdk/google-vertex/anthropic` | OAuth | Claude via Vertex |
| **Azure OpenAI** | `@ai-sdk/azure` | API Key | Azure-hosted models |
| **AWS Bedrock** | `@ai-sdk/amazon-bedrock` | AWS Credentials | AWS authentication |
| **OpenRouter** | `@openrouter/ai-sdk-provider` | API Key | Multi-provider gateway |
| **xAI** | `@ai-sdk/xai` | API Key | Grok models |
| **Mistral** | `@ai-sdk/mistral` | API Key | Mistral models |
| **Groq** | `@ai-sdk/groq` | API Key | Fast inference |
| **DeepInfra** | `@ai-sdk/deepinfra` | API Key | Open model hosting |
| **Cerebras** | `@ai-sdk/cerebras` | API Key | High-performance inference |
| **Cohere** | `@ai-sdk/cohere` | API Key | Cohere models |
| **Together AI** | `@ai-sdk/togetherai` | API Key | Open model hosting |
| **Perplexity** | `@ai-sdk/perplexity` | API Key | Search-augmented models |
| **Vercel AI** | `@ai-sdk/vercel` | API Key | Vercel AI Gateway |
| **GitLab Duo** | `@gitlab/gitlab-ai-provider` | OAuth/PAT | GitLab AI features |
| **OpenAI-Compatible** | `@ai-sdk/openai-compatible` | API Key | Generic OpenAI-compatible servers |
| **Gateway** | `@ai-sdk/gateway` | Various | Generic gateway support |

### Additional 50+ Providers via Models.dev API
- DeepSeek, Fireworks AI, Hugging Face, Helicone, IO.NET, Moonshot AI, MiniMax, Nebius, Ollama, Scaleway, Venice AI, Z.AI, ZenMux, and many more

---

## 2. Authentication Architecture

### 2.1 Authentication Types

#### API Key Authentication
```typescript
type ApiAuth = {
  type: "api"
  key: string
}
```

**Storage**: `~/.local/share/opencode/auth.json` (permissions: 0600)
**Use Case**: Traditional API providers (OpenAI, Anthropic, etc.)
**Example**:
```json
{
  "anthropic": {
    "type": "api",
    "key": "sk-ant-..."
  }
}
```

#### OAuth Authentication
```typescript
type OAuthAuth = {
  type: "oauth"
  refresh: string       // Refresh token
  access: string        // Access token
  expires: number       // Expiration timestamp
  accountId?: string    // Organization/account ID
  enterpriseUrl?: string // Enterprise instance URL
}
```

**Storage**: `~/.local/share/opencode/auth.json` (permissions: 0600)
**Use Case**: Subscription services (Claude Pro, ChatGPT Plus, GitHub Copilot)
**Example**:
```json
{
  "openai": {
    "type": "oauth",
    "refresh": "...",
    "access": "...",
    "expires": 1234567890,
    "accountId": "org-..."
  }
}
```

#### Well-Known Configuration
```typescript
type WellKnownAuth = {
  type: "wellknown"
  key: string    // Environment variable name
  token: string  // Actual token value
}
```

**Use Case**: Organization-provided remote configuration
**Endpoint**: `${provider}/.well-known/opencode`

---

## 3. OAuth Implementation Details

### 3.1 GitHub Copilot OAuth

**File**: `packages/opencode/src/plugin/copilot.ts`
**Flow**: OAuth 2.0 Device Code Flow
**Client ID**: `Ov23li8tweQw6odWQebz`

**Implementation**:
```typescript
async authorize() {
  // 1. Request device code
  const deviceResponse = await fetch(
    `https://github.com/login/device/code`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        client_id: CLIENT_ID,
        scope: "read:user",
      }),
    }
  )

  const { verification_uri, user_code, device_code, interval } = await deviceResponse.json()

  // 2. Show user the verification URL and code
  return {
    url: verification_uri,
    instructions: `Enter code: ${user_code}`,
    method: "auto",
    async callback() {
      // 3. Poll for authorization completion
      while (true) {
        await sleep(interval * 1000)

        const tokenResponse = await fetch(
          `https://github.com/login/oauth/access_token`,
          {
            method: "POST",
            body: JSON.stringify({
              client_id: CLIENT_ID,
              device_code: device_code,
              grant_type: "urn:ietf:params:oauth:grant-type:device_code",
            }),
          }
        )

        const data = await tokenResponse.json()

        if (data.access_token) {
          return {
            type: "success",
            refresh: data.access_token,
            access: data.access_token,
            expires: 0, // GitHub tokens don't expire
          }
        }

        if (data.error === "authorization_pending") {
          continue // Keep polling
        }

        throw new Error(data.error_description)
      }
    }
  }
}
```

**Key Features**:
- No redirect server needed - uses device code flow
- Supports GitHub.com and GitHub Enterprise
- Sets all model costs to $0 for subscription users
- Custom headers: `User-Agent`, `Authorization: Bearer`, `Openai-Intent`, `X-Initiator`

### 3.2 OpenAI Codex OAuth

**File**: `packages/opencode/src/plugin/codex.ts`
**Flow**: OAuth 2.0 with PKCE (Proof Key for Code Exchange)
**Client ID**: `app_EMoamEEZ73f0CkXaXp7hrann`
**Issuer**: `https://auth.openai.com`

**Implementation**:
```typescript
async authorize() {
  // 1. Generate PKCE parameters
  const codeVerifier = generateRandomString(128)
  const codeChallenge = await sha256(codeVerifier)
  const state = generateRandomString(32)

  // 2. Build authorization URL
  const redirectUri = `http://127.0.0.1:1455/callback`
  const authUrl = new URL(`${ISSUER}/authorize`)
  authUrl.searchParams.set("client_id", CLIENT_ID)
  authUrl.searchParams.set("response_type", "code")
  authUrl.searchParams.set("redirect_uri", redirectUri)
  authUrl.searchParams.set("scope", "openid profile email")
  authUrl.searchParams.set("code_challenge", codeChallenge)
  authUrl.searchParams.set("code_challenge_method", "S256")
  authUrl.searchParams.set("state", state)

  // 3. Start local callback server
  const server = Bun.serve({
    port: 1455,
    async fetch(req) {
      const url = new URL(req.url)

      if (url.searchParams.get("state") !== state) {
        return new Response("Invalid state", { status: 400 })
      }

      const code = url.searchParams.get("code")

      // 4. Exchange code for tokens
      const tokenResponse = await fetch(`${ISSUER}/oauth/token`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          client_id: CLIENT_ID,
          grant_type: "authorization_code",
          code: code,
          redirect_uri: redirectUri,
          code_verifier: codeVerifier,
        }),
      })

      const tokens = await tokenResponse.json()

      // 5. Decode JWT to extract account ID
      const [, payload] = tokens.access_token.split(".")
      const decoded = JSON.parse(atob(payload))

      return {
        type: "success",
        refresh: tokens.refresh_token,
        access: tokens.access_token,
        expires: Date.now() + tokens.expires_in * 1000,
        accountId: decoded["https://api.openai.com/auth"].user_id,
      }
    },
  })

  return {
    url: authUrl.toString(),
    method: "auto",
    async callback() {
      // Waits for OAuth callback
      return server.waitForResult()
    },
  }
}
```

**Token Refresh**:
```typescript
async refresh(auth: OAuthAuth): Promise<OAuthAuth> {
  const response = await fetch(`${ISSUER}/oauth/token`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      client_id: CLIENT_ID,
      grant_type: "refresh_token",
      refresh_token: auth.refresh,
    }),
  })

  const tokens = await response.json()

  return {
    ...auth,
    access: tokens.access_token,
    expires: Date.now() + tokens.expires_in * 1000,
  }
}
```

**Key Features**:
- PKCE for security (prevents authorization code interception)
- JWT token parsing for account ID extraction
- Local callback server on port 1455
- Automatic token refresh when expired
- Filters to only Codex models (gpt-5.1-codex-max, gpt-5.1-codex-mini, gpt-5.2-codex)
- Sets costs to $0 for ChatGPT subscription

### 3.3 Anthropic Claude Pro/Max OAuth

**Plugin**: `opencode-anthropic-auth@0.0.9` (external npm package)
**Library**: `@openauthjs/openauth`

**Key Features**:
- Similar OAuth flow to OpenAI Codex
- Provides authenticated access to Claude models for Pro/Max subscribers
- Separate npm package for maintainability
- Uses OpenAuth library for OAuth protocol handling

### 3.4 MCP (Model Context Protocol) OAuth

**File**: `packages/opencode/src/mcp/oauth-provider.ts`
**Callback Server**: `packages/opencode/src/mcp/oauth-callback.ts`
**Port**: 19876

**OAuth Provider Implementation**:
```typescript
export class McpOAuthProvider implements OAuthClientProvider {
  get redirectUrl(): string {
    return `http://127.0.0.1:19876/mcp/oauth/callback`
  }

  async clientInformation(): Promise<OAuthClientInformation | undefined> {
    // Check if pre-registered client exists
    if (this.config.clientId) {
      return {
        client_id: this.config.clientId,
        client_secret: this.config.clientSecret,
      }
    }

    // Fall back to dynamic client registration (RFC 7591)
    const entry = await McpAuth.getForUrl(this.mcpName, this.serverUrl)
    return entry?.clientInfo
  }

  async tokens(): Promise<OAuthTokens | undefined> {
    const entry = await McpAuth.getForUrl(this.mcpName, this.serverUrl)
    return entry?.tokens
  }

  async storeTokens(tokens: OAuthTokens): Promise<void> {
    await McpAuth.store(this.mcpName, this.serverUrl, tokens, this.clientInfo)
  }
}
```

**Shared Callback Server**:
```typescript
const OAUTH_CALLBACK_PORT = 19876

Bun.serve({
  port: OAUTH_CALLBACK_PORT,
  fetch(req) {
    const url = new URL(req.url)
    const code = url.searchParams.get("code")
    const state = url.searchParams.get("state")

    // Validate state parameter (CSRF protection)
    if (!state || !pendingAuths.has(state)) {
      return new Response(renderErrorPage("Invalid state - potential CSRF attack"), {
        status: 400,
        headers: { "Content-Type": "text/html" },
      })
    }

    // Resolve pending authorization
    const pending = pendingAuths.get(state)
    pending.resolve(code)

    return new Response(renderSuccessPage(), {
      headers: { "Content-Type": "text/html" },
    })
  }
})
```

**Key Features**:
- Shared callback server on port 19876 for all MCP OAuth flows
- State validation for CSRF protection
- 5-minute timeout for authorization
- Dynamic client registration support (RFC 7591)
- Per-URL credential validation

---

## 4. Provider Configuration System

### 4.1 Configuration Priority (Highest to Lowest)

1. **Inline Config**: `OPENCODE_CONFIG_CONTENT` environment variable
2. **Project Config**: `opencode.json[c]` in project directory
3. **Custom Config Path**: `OPENCODE_CONFIG` CLI flag
4. **Global User Config**: `~/.config/opencode/opencode.json`
5. **Remote/Well-Known Config**: Organization defaults from `/.well-known/opencode`

### 4.2 Provider Schema

```typescript
type Provider = {
  npm?: string                    // AI SDK package name
  name: string                    // Display name
  api?: string                    // Base URL
  env: string[]                   // Environment variable names for API keys
  models: Record<string, Model>   // Model definitions
  options?: {
    apiKey?: string
    baseURL?: string
    timeout?: number | false
    headers?: Record<string, string>
    [key: string]: any            // Additional provider-specific options
  }
}

type Model = {
  name: string           // Display name
  limit: {
    context: number      // Context window size
    output: number       // Max output tokens
  }
  cost?: {
    input: number        // Cost per 1M input tokens
    output: number       // Cost per 1M output tokens
  }
  vision?: boolean       // Supports images
  tools?: boolean        // Supports function calling
  cache?: boolean        // Supports prompt caching
}
```

### 4.3 Example Configuration

```json
{
  "provider": {
    "anthropic": {
      "npm": "@ai-sdk/anthropic",
      "name": "Anthropic",
      "env": ["ANTHROPIC_API_KEY"],
      "options": {
        "baseURL": "https://api.anthropic.com/v1",
        "headers": {
          "anthropic-beta": "claude-code-20250219"
        }
      },
      "models": {
        "claude-3-5-sonnet-20250219": {
          "name": "Claude 3.5 Sonnet",
          "limit": {
            "context": 200000,
            "output": 8192
          },
          "cost": {
            "input": 3.00,
            "output": 15.00
          },
          "vision": true,
          "tools": true,
          "cache": true
        }
      }
    },
    "my-custom-provider": {
      "npm": "@ai-sdk/openai-compatible",
      "name": "My Custom Provider",
      "options": {
        "baseURL": "https://api.myprovider.com/v1",
        "apiKey": "{env:MY_API_KEY}",
        "headers": {
          "X-Custom-Header": "value"
        }
      },
      "models": {
        "my-model": {
          "name": "My Model",
          "limit": {
            "context": 128000,
            "output": 4096
          }
        }
      }
    }
  }
}
```

---

## 5. Provider Loading Mechanism

**File**: `packages/opencode/src/provider/provider.ts`

### Loading Order

```typescript
// 1. Load from environment variables
for (const [providerID, provider] of Object.entries(database)) {
  const apiKey = provider.env.map((item) => env[item]).find(Boolean)
  if (apiKey) {
    mergeProvider(providerID, {
      source: "env",
      key: apiKey,
    })
  }
}

// 2. Load from stored auth credentials
for (const [providerID, provider] of await Auth.all()) {
  if (provider.type === "api") {
    mergeProvider(providerID, {
      source: "api",
      key: provider.key,
    })
  }
}

// 3. Load from plugins (OAuth providers)
for (const plugin of await Plugin.list()) {
  if (plugin.auth) {
    const options = await plugin.auth.loader(
      () => Auth.get(providerID),
      database[providerID]
    )
    mergeProvider(providerID, {
      source: "custom",
      options: options,
    })
  }
}

// 4. Load custom loaders
for (const [providerID, fn] of Object.entries(CUSTOM_LOADERS)) {
  const result = await fn(data)
  if (result.autoload) {
    mergeProvider(providerID, {
      source: "custom",
      options: result.options,
    })
  }
}
```

### Custom Loaders (OAuth Token Injection)

```typescript
const CUSTOM_LOADERS = {
  "github-copilot": async (data) => {
    const auth = await Auth.get("github-copilot")
    if (!auth || auth.type !== "oauth") {
      return { autoload: false }
    }

    // Check if token is expired and refresh if needed
    if (auth.expires > 0 && auth.expires < Date.now()) {
      const refreshed = await refreshToken(auth)
      await Auth.set("github-copilot", refreshed)
      auth = refreshed
    }

    return {
      autoload: true,
      options: {
        headers: {
          "Authorization": `Bearer ${auth.access}`,
          "User-Agent": "OpenCode/1.0",
          "Openai-Intent": "code-completion",
        },
        // Mark all models as free for subscription users
        models: Object.fromEntries(
          Object.entries(data.models).map(([id, model]) => [
            id,
            { ...model, cost: { input: 0, output: 0 } }
          ])
        ),
      },
    }
  },
}
```

---

## 6. Models.dev Integration

**Endpoint**: `https://models.dev/api.json`
**File**: `packages/opencode/src/provider/models.ts`

OpenCode fetches a centralized database of LLM providers and models from models.dev, which includes:
- Provider metadata (name, base URL, npm package)
- Model specifications (context limits, costs, capabilities)
- Environment variable names for API keys
- Default configuration

**Benefits**:
- Centralized provider database
- Automatic updates to model specs
- Community-maintained provider list
- Reduces maintenance burden

**Local Override**:
User configuration always takes precedence over models.dev defaults.

---

## 7. Token Refresh Strategy

### Automatic Refresh
```typescript
async function ensureValidToken(providerID: string): Promise<string> {
  const auth = await Auth.get(providerID)

  if (!auth || auth.type !== "oauth") {
    throw new Error("No OAuth credentials found")
  }

  // Check if token is expired (with 5-minute buffer)
  if (auth.expires > 0 && auth.expires < Date.now() + 5 * 60 * 1000) {
    const plugin = await Plugin.get(providerID)

    if (!plugin?.refresh) {
      throw new Error("No refresh method available")
    }

    const refreshed = await plugin.refresh(auth)
    await Auth.set(providerID, refreshed)

    return refreshed.access
  }

  return auth.access
}
```

### Background Refresh Task
OpenCode checks token expiration before each request and refreshes proactively if the token expires within 5 minutes.

---

## 8. Security Considerations

### File Permissions
- Auth file: `~/.local/share/opencode/auth.json` (mode: 0600)
- MCP auth file: `~/.local/share/opencode/mcp-auth.json` (mode: 0600)
- Config file: `~/.config/opencode/opencode.json` (mode: 0644)

### PKCE (Proof Key for Code Exchange)
- Used in OpenAI Codex OAuth flow
- Prevents authorization code interception attacks
- Generates cryptographic code verifier and challenge

### State Parameter Validation
- All OAuth flows validate state parameter
- Prevents CSRF attacks
- Generated using cryptographic random strings

### Token Storage
- Refresh tokens stored encrypted at rest (future enhancement)
- Access tokens kept in memory when possible
- No tokens logged or exposed in error messages

### OAuth Client Registration
- Some providers support dynamic client registration (RFC 7591)
- Reduces need for hardcoded client IDs
- Per-application client isolation

---

## 9. Implementation Recommendations for LocalRouter AI

### Phase 1: API Key Providers (Immediate)
Implement these providers with API key authentication:
1. **Anthropic** - `@ai-sdk/anthropic`
2. **OpenAI** - `@ai-sdk/openai`
3. **Google Gemini** - `@ai-sdk/google`
4. **Groq** - `@ai-sdk/groq`
5. **Mistral** - `@ai-sdk/mistral`
6. **OpenRouter** - `@openrouter/ai-sdk-provider`
7. **Ollama** - Already implemented, but integrate with AI SDK
8. **OpenAI-Compatible** - `@ai-sdk/openai-compatible` (for custom providers)

### Phase 2: OAuth Providers (Future)
Implement as plugins/extensions:
1. **GitHub Copilot** - Device code flow
2. **OpenAI Codex** - PKCE OAuth flow
3. **Anthropic Claude Pro/Max** - Custom OAuth (consider using `opencode-anthropic-auth`)

### Phase 3: Cloud Providers (Optional)
1. **AWS Bedrock** - `@ai-sdk/amazon-bedrock`
2. **Google Vertex** - `@ai-sdk/google-vertex`
3. **Azure OpenAI** - `@ai-sdk/azure`

### Architecture Recommendations

#### 1. Provider Trait Extension
```rust
pub enum ProviderAuth {
    ApiKey(String),
    OAuth {
        access_token: String,
        refresh_token: String,
        expires_at: i64,
        account_id: Option<String>,
    },
    Custom(Box<dyn CustomAuth>),
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    async fn authenticate(&mut self, auth: ProviderAuth) -> Result<()>;
    async fn refresh_token(&mut self) -> Result<ProviderAuth>;
    // ... existing methods
}
```

#### 2. Separate Auth Storage
```rust
// ~/.local/share/localrouter/auth.json
{
  "providers": {
    "anthropic": {
      "type": "api",
      "key": "sk-ant-..."
    },
    "openai-codex": {
      "type": "oauth",
      "access_token": "...",
      "refresh_token": "...",
      "expires_at": 1234567890,
      "account_id": "org-..."
    }
  }
}
```

#### 3. OAuth Plugin System
```rust
pub trait OAuthPlugin: Send + Sync {
    async fn authorize(&self) -> Result<OAuthFlow>;
    async fn refresh(&self, auth: &OAuthAuth) -> Result<OAuthAuth>;
    fn client_id(&self) -> &str;
    fn provider_id(&self) -> &str;
}

pub struct OAuthFlow {
    pub url: String,
    pub method: FlowMethod,
}

pub enum FlowMethod {
    DeviceCode {
        user_code: String,
        verification_uri: String,
        device_code: String,
        interval: u64,
    },
    PKCE {
        code_verifier: String,
        redirect_uri: String,
    },
}
```

#### 4. Token Refresh Background Task
```rust
async fn token_refresh_worker(state: Arc<AppState>) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;

        for (provider_id, auth) in state.auth_manager.all_oauth().await {
            if auth.expires_at < Utc::now().timestamp() + 300 {
                // Refresh if expiring within 5 minutes
                if let Some(plugin) = state.oauth_plugins.get(&provider_id) {
                    match plugin.refresh(&auth).await {
                        Ok(new_auth) => {
                            state.auth_manager.update(provider_id, new_auth).await;
                        }
                        Err(e) => {
                            warn!("Failed to refresh token for {}: {}", provider_id, e);
                        }
                    }
                }
            }
        }
    }
}
```

#### 5. UI Integration
- **API Keys Tab**: Already exists, extend for multiple providers
- **OAuth Tab**: New tab for subscription services
  - List available OAuth providers
  - "Connect" button that opens browser for authorization
  - Display connection status and account info
  - "Disconnect" button to revoke access

---

## 10. Key Files Reference

| Component | OpenCode File Path |
|-----------|-------------------|
| Auth storage | `packages/opencode/src/auth/index.ts` |
| Provider loading | `packages/opencode/src/provider/provider.ts` |
| Provider auth | `packages/opencode/src/provider/auth.ts` |
| Copilot OAuth | `packages/opencode/src/plugin/copilot.ts` |
| Codex OAuth | `packages/opencode/src/plugin/codex.ts` |
| MCP OAuth provider | `packages/opencode/src/mcp/oauth-provider.ts` |
| MCP OAuth callback | `packages/opencode/src/mcp/oauth-callback.ts` |
| MCP auth storage | `packages/opencode/src/mcp/auth.ts` |
| Config system | `packages/opencode/src/config/config.ts` |
| Models database | `packages/opencode/src/provider/models.ts` |
| Plugin system | `packages/opencode/src/plugin/index.ts` |

---

## 11. Summary

**What OpenCode Does Well**:
1. **Plugin-based architecture** - OAuth providers as separate plugins
2. **Unified credential storage** - Single file with proper permissions
3. **Automatic token refresh** - Background task with expiration tracking
4. **Dynamic provider loading** - Auto-load based on available credentials
5. **Account ID tracking** - Multi-tenant support
6. **Zero-cost subscription models** - Automatically marks subscription models as free
7. **Security best practices** - PKCE, state validation, secure storage

**Key Takeaways for LocalRouter AI**:
- Start with API key providers (simpler, immediate value)
- Design OAuth as optional plugin system (not core)
- Use Vercel AI SDK for unified provider interface
- Implement token refresh background worker
- Store OAuth credentials separately from API keys
- Support account ID field for organization routing
- Consider integrating models.dev for centralized model database

**Providers to Prioritize**:
1. **High Priority**: Anthropic, OpenAI, Google Gemini, Groq, Mistral, OpenRouter
2. **Medium Priority**: GitHub Copilot (OAuth), Cohere, Together AI, Perplexity
3. **Low Priority**: Cloud providers (AWS, Azure, GCP), niche providers

---

**Next Steps**: Implement Phase 1 providers with API key authentication, then design OAuth plugin architecture for Phase 2.
