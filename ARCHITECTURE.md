# LocalRouter AI - Architecture & Design Document

## Overview

LocalRouter AI is a cross-platform desktop application built with Rust and Tauri that provides a local OpenAI-compatible API gateway with intelligent routing, API key management, and multi-provider support.

## System Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Desktop UI (Tauri)                      │
│  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐       │
│  │  Home Tab    │ │ API Keys Tab │ │ Routers Tab  │       │
│  │  (Graphs)    │ │ (Management) │ │ (Config)     │       │
│  └──────────────┘ └──────────────┘ └──────────────┘       │
│                                                              │
│  ┌────────────────────────────────────────────────────┐   │
│  │           System Menu (Tray Icon)                  │   │
│  │  - Quick API Key Access                            │   │
│  │  - Model Selection                                 │   │
│  │  - Settings & Quit                                 │   │
│  └────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                    Core Backend (Rust)                      │
│                                                              │
│  ┌────────────────────────────────────────────────────┐   │
│  │         OpenAI Compatible Web Server               │   │
│  │  - HTTP Server (Axum/Actix)                        │   │
│  │  - OpenAI API Compatibility Layer                  │   │
│  │  - Request Validation & Authentication             │   │
│  └────────────────────────────────────────────────────┘   │
│                            │                                │
│                            ▼                                │
│  ┌────────────────────────────────────────────────────┐   │
│  │            Request Router & Manager                │   │
│  │  - API Key Resolver                                │   │
│  │  - Model Selection Logic                           │   │
│  │  - Smart Routing Engine                            │   │
│  │  - Fallback Handler                                │   │
│  │  - Rate Limiter                                    │   │
│  └────────────────────────────────────────────────────┘   │
│                            │                                │
│                            ▼                                │
│  ┌────────────────────────────────────────────────────┐   │
│  │         Model Provider Abstraction Layer           │   │
│  │  - Common Provider Trait                           │   │
│  │  - Health Check System                             │   │
│  │  - Model Catalog Manager                           │   │
│  │  - Pricing Information Cache                       │   │
│  └────────────────────────────────────────────────────┘   │
│                            │                                │
│         ┌──────────────────┼──────────────────┐           │
│         ▼                  ▼                  ▼             │
│  ┌───────────┐     ┌───────────┐     ┌───────────┐       │
│  │  Ollama   │     │ OpenRouter│     │  OpenAI   │       │
│  │ Provider  │     │ Provider  │     │ Provider  │       │
│  └───────────┘     └───────────┘     └───────────┘       │
│         ▼                  ▼                  ▼             │
│  ┌───────────┐     ┌───────────┐     ┌───────────┐       │
│  │ Anthropic │     │  Gemini   │     │OAuth-based│       │
│  │ Provider  │     │ Provider  │     │ Providers │       │
│  └───────────┘     └───────────┘     └───────────┘       │
│                                                              │
│  ┌────────────────────────────────────────────────────┐   │
│  │         Configuration & Storage Module             │   │
│  │  - Settings Manager                                │   │
│  │  - API Key Storage (Encrypted)                     │   │
│  │  - Router Configuration                            │   │
│  │  - Provider Settings                               │   │
│  └────────────────────────────────────────────────────┘   │
│                                                              │
│  ┌────────────────────────────────────────────────────┐   │
│  │       Monitoring & Logging System                  │   │
│  │  - Access Log Writer                               │   │
│  │  - Metrics Collection (In-Memory)                  │   │
│  │  - Historical Log Parser                           │   │
│  │  - Graph Data Generator                            │   │
│  └────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                  Persistent Storage                          │
│  - ~/.localrouter/settings.yaml                             │
│  - ~/.localrouter/api_keys.json (encrypted)                │
│  - ~/.localrouter/routers.yaml                              │
│  - /var/log/localrouter/ (Linux) or OS equivalent          │
└─────────────────────────────────────────────────────────────┘
```

## Component Breakdown

### 1. Web Server Component

**Responsibility**: Expose OpenAI-compatible HTTP API endpoint

**Key Features**:
- HTTP server using Axum or Actix-web
- OpenAI API compatibility layer (`/v1/chat/completions`, `/v1/completions`, `/v1/models`)
- Request parsing and validation
- API key authentication
- Request/response streaming support
- CORS handling for local development

**Interfaces**:
```rust
trait WebServer {
    async fn start(&self, host: String, port: u16) -> Result<()>;
    async fn stop(&self) -> Result<()>;
    fn is_running(&self) -> bool;
}
```

### 2. Configuration Module

**Responsibility**: Persist and manage all application settings

**Key Features**:
- YAML-based configuration
- Encrypted API key storage
- Hot-reload capability
- Schema versioning and migration
- Thread-safe access

**Storage Locations**:
- `~/.localrouter/settings.yaml` - General settings
- `~/.localrouter/api_keys.json` - Encrypted API keys
- `~/.localrouter/routers.yaml` - Router configurations
- `~/.localrouter/providers.yaml` - Provider configurations

**Data Structures**:
```rust
struct AppConfig {
    server: ServerConfig,
    api_keys: Vec<ApiKeyConfig>,
    routers: Vec<RouterConfig>,
    providers: Vec<ProviderConfig>,
    logging: LoggingConfig,
}

struct ServerConfig {
    host: String,
    port: u16,
    enable_cors: bool,
}

struct ApiKeyConfig {
    id: String,
    name: String,
    key_hash: String,
    model_selection: ModelSelection,
    created_at: DateTime<Utc>,
}

enum ModelSelection {
    DirectModel { provider: String, model: String },
    Router { router_name: String },
}
```

### 3. Model Provider System

**Responsibility**: Abstract and manage different AI model providers

**Common Provider Interface**:
```rust
#[async_trait]
trait ModelProvider: Send + Sync {
    fn name(&self) -> &str;

    async fn health_check(&self) -> ProviderHealth;

    async fn list_models(&self) -> Result<Vec<ModelInfo>>;

    async fn get_pricing(&self, model: &str) -> Result<PricingInfo>;

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse>;

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<CompletionChunk>>>>>;
}

struct ModelInfo {
    id: String,
    name: String,
    provider: String,
    parameter_count: Option<u64>,
    context_window: u32,
    supports_streaming: bool,
    capabilities: Vec<Capability>,
}

struct PricingInfo {
    input_cost_per_1k: f64,
    output_cost_per_1k: f64,
    currency: String,
}

struct ProviderHealth {
    status: HealthStatus,
    latency_ms: Option<u64>,
    last_checked: DateTime<Utc>,
    error_message: Option<String>,
}

enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}
```

**Provider Implementations**:

1. **Ollama Provider**
   - Local HTTP API
   - Model listing via `/api/tags`
   - No cost tracking
   - Always available when service is running

2. **OpenRouter Provider**
   - Proxies to multiple providers
   - Model catalog from API
   - Real-time pricing
   - API key authentication

3. **OpenAI Provider**
   - Direct OpenAI API
   - Model catalog (GPT-4, GPT-3.5, etc.)
   - Pricing from documentation
   - API key authentication

4. **Anthropic Provider**
   - Claude models
   - API key authentication
   - Pricing tracking

5. **Google Gemini Provider**
   - Gemini models
   - API key authentication

6. **OAuth Subscription Providers**
   - OpenAI subscription (ChatGPT Plus/Pro)
   - Anthropic subscription (Claude Pro)
   - OAuth 2.0 flow for authentication
   - Session management

### 4. Smart Router System

**Responsibility**: Intelligent model selection and request routing

**Router Configuration**:
```rust
struct RouterConfig {
    name: String,
    model_selection: ModelSelectionStrategy,
    strategies: Vec<RoutingStrategy>,
    fallback_enabled: bool,
    rate_limiters: Vec<RateLimiter>,
}

enum ModelSelectionStrategy {
    Automatic {
        providers: Vec<ProviderFilter>,
        min_parameters: Option<u64>,
        max_parameters: Option<u64>,
    },
    Manual {
        models: Vec<(String, String)>, // (provider, model) in priority order
    },
}

struct ProviderFilter {
    provider_name: String,
    include_models: Option<Vec<String>>, // None = all models
    exclude_models: Vec<String>,
}

enum RoutingStrategy {
    LowestCost,
    HighestPerformance,
    LocalFirst,
    RemoteFirst,
    SubscriptionFirst,
    ApiFirst,
}

struct RateLimiter {
    limit_type: RateLimitType,
    value: f64,
    time_window: Duration,
}

enum RateLimitType {
    Requests,
    InputTokens,
    OutputTokens,
    TotalTokens,
    Cost, // in USD
}
```

**Routing Algorithm**:
1. Filter models based on `ModelSelectionStrategy`
2. Check health status of available providers
3. Apply `RoutingStrategy` to order candidates
4. Check rate limiters for each candidate
5. Select first available model
6. If request fails, attempt fallback to next candidate

**Default Routers**:
- **Minimum Cost**: Lowest cost per token, with health check
- **Maximum Performance**: Highest parameter count, best latency

### 5. API Key Management

**Responsibility**: Manage user API keys and their associations

**Features**:
- Generate unique API keys (cryptographically secure)
- Store hashed versions (bcrypt or similar)
- Associate keys with names and model selections
- Track usage per key
- Enable/disable keys

**API Key Structure**:
```rust
struct ApiKey {
    id: Uuid,
    name: String,
    key_hash: String, // Hashed value
    model_selection: ModelSelection,
    enabled: bool,
    created_at: DateTime<Utc>,
    last_used: Option<DateTime<Utc>>,
}
```

**Key Format**: `lr-` + base64url(random 32 bytes) = `lr-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx`

### 6. Monitoring & Logging System

**Responsibility**: Track usage, generate metrics, and maintain logs

**Access Log Format** (Structured JSON Lines):
```json
{
  "timestamp": "2026-01-14T10:30:00Z",
  "api_key_name": "my-app-123",
  "provider": "openai",
  "model": "gpt-4",
  "status": "success",
  "status_code": 200,
  "input_tokens": 150,
  "output_tokens": 500,
  "total_tokens": 650,
  "cost_usd": 0.0325,
  "latency_ms": 1234,
  "request_id": "req_abc123"
}
```

**Metrics Collection**:
- In-memory time-series data (last 24 hours at 1-minute granularity)
- Aggregated metrics: tokens/hour, cost/hour, requests/hour
- Per-API-key metrics
- Per-provider metrics
- Global metrics

**Graph Data**:
- Tokens over time
- Cost over time
- Requests over time
- Success rate
- Latency percentiles

**Log Rotation**:
- Daily log files: `localrouter-2026-01-14.log`
- Automatic rotation (keep last 30 days)
- Compressed archives for older logs

### 7. Desktop UI (Tauri)

**Responsibility**: Provide user interface for configuration and monitoring

**Technology Stack**:
- **Backend**: Rust (Tauri commands)
- **Frontend**: HTML/CSS/JavaScript (React, Vue, or Svelte)
- **Charts**: Chart.js or similar
- **Styling**: Tailwind CSS or similar

**Main App Tabs**:

1. **Home Tab**:
   - Global metrics dashboard
   - Combined graph (all API keys + all providers)
   - Toggle between: Tokens/Time, Cost/Time, Requests/Time
   - Time range selector (Last hour, 24 hours, 7 days, 30 days)

2. **API Keys Tab**:
   - List view of all API keys (table)
   - Columns: Name, Model/Router, Created, Last Used, Status
   - Actions: Create, Edit, Delete, Enable/Disable, Regenerate
   - Detail view when selected:
     - Key name (editable)
     - API key value (hidden, with copy button)
     - Model selection (dropdown: Direct Model or Router)
     - Usage graph for this key
     - Usage statistics

3. **Routers Tab**:
   - List view of routers
   - Actions: Create, Edit, Delete
   - Router editor:
     - Name
     - Model selection strategy (radio buttons):
       - **Automatic**: Nested checkbox tree (All → Providers → Models)
       - **Manual**: Draggable priority list
     - Strategies (checkboxes): Lowest cost, Local first, etc.
     - Rate limiters (configurable list)
     - Live preview: Shows ordered list of models that would be selected
     - Model table: Shows all models with columns (Provider, Model, Parameters, Cost, Health)

4. **Providers Tab** (bonus):
   - List of configured providers
   - Health status indicators
   - API key configuration
   - OAuth login buttons for subscription providers

5. **Settings Tab**:
   - Server configuration (host, port)
   - Log settings
   - Theme (light/dark)
   - About & version info

### 8. System Menu (Tray Icon)

**Responsibility**: Quick access to key features from system tray

**Menu Structure**:
```
[App Icon] LocalRouter AI
├─ [Key Icon] my-app-123 >
│  ├─ 📊 View Metrics
│  ├─ Router: Minimum Cost ✓
│  ├─ Router: Maximum Performance
│  ├─ ─────────
│  ├─ Ollama/llama3.3
│  ├─ OpenRouter/OpenAI/gpt-4
│  ├─ OpenAI/gpt-3.5-turbo
│  └─ ...
├─ [Key Icon] another-key >
│  └─ ...
├─ ➕ Generate New Key...
├─ ─────────
├─ 📊 Open Dashboard
├─ ⚙️  Preferences
├─ ❌ Quit
```

**Features**:
- List all API keys with quick access
- Nested menu for each key showing routers and models
- Checkmark indicates current selection
- Mini graph preview on hover (optional)
- Quick key generation

### 9. Settings & Storage

**Storage Locations** (OS-specific):

**Linux**:
- Config: `~/.config/localrouter/` or `~/.localrouter/`
- Logs: `/var/log/localrouter/` (if permissions) or `~/.localrouter/logs/`
- Cache: `~/.cache/localrouter/`

**macOS**:
- Config: `~/Library/Application Support/LocalRouter/`
- Logs: `~/Library/Logs/LocalRouter/`
- Cache: `~/Library/Caches/LocalRouter/`

**Windows**:
- Config: `%APPDATA%\LocalRouter\`
- Logs: `%APPDATA%\LocalRouter\logs\`
- Cache: `%LOCALAPPDATA%\LocalRouter\`

**Security**:
- API keys encrypted at rest using system keyring (keyring-rs crate)
- Provider API keys encrypted
- Settings file permissions: 0600 (user read/write only)

## Technology Stack

### Backend (Rust)
- **Web Server**: `axum` or `actix-web`
- **Async Runtime**: `tokio`
- **HTTP Client**: `reqwest`
- **Serialization**: `serde`, `serde_json`, `serde_yaml`
- **Cryptography**: `ring`, `bcrypt`
- **Logging**: `tracing`, `tracing-subscriber`
- **Configuration**: `config` crate
- **Keyring**: `keyring-rs`
- **OAuth**: `oauth2` crate

### Desktop Framework
- **Framework**: Tauri 2.x
- **IPC**: Tauri commands
- **System Tray**: Tauri tray API

### Frontend
- **Framework**: React, Vue, or Svelte (TBD)
- **Styling**: Tailwind CSS
- **Charts**: Chart.js or Apache ECharts
- **State Management**: Zustand (React) or Pinia (Vue)

## Security Considerations

1. **API Key Storage**: Encrypted using system keyring
2. **Provider Keys**: Never logged or exposed in UI
3. **Access Logs**: Do not contain request/response bodies
4. **Rate Limiting**: Prevent abuse of configured keys
5. **HTTPS**: For remote providers (enforced)
6. **Input Validation**: All user inputs sanitized
7. **File Permissions**: Restrictive permissions on config files

## Performance Considerations

1. **Async/Await**: All I/O operations are async
2. **Connection Pooling**: Reuse HTTP connections to providers
3. **Caching**:
   - Model catalogs (refresh every 5 minutes)
   - Pricing information (refresh every hour)
   - Health checks (refresh every 30 seconds)
4. **In-Memory Metrics**: Recent data kept in memory for fast access
5. **Streaming**: Support for streaming responses to reduce latency

## Error Handling & Fallback

1. **Provider Failures**:
   - Automatic fallback to next provider in router
   - Circuit breaker pattern to avoid repeatedly calling failed providers
   - Exponential backoff for retries

2. **Rate Limiting**:
   - Return 429 status code when limit exceeded
   - Include `Retry-After` header

3. **Configuration Errors**:
   - Validate on load
   - Fall back to defaults if config is corrupted
   - Backup config before writing

## Testing Strategy

1. **Unit Tests**: Individual components
2. **Integration Tests**: Provider implementations, router logic
3. **E2E Tests**: Full request flow through web server
4. **UI Tests**: Tauri frontend tests
5. **Load Tests**: Performance under concurrent requests

## Deployment & Distribution

1. **Packaging**:
   - macOS: `.dmg` and `.app` bundle
   - Windows: `.msi` installer
   - Linux: `.deb`, `.rpm`, AppImage

2. **Auto-Update**: Tauri updater for automatic updates

3. **Version Management**: Semantic versioning (semver)

## Future Enhancements

1. **Plugins**: Provider plugins for community extensions
2. **Custom Prompts**: Pre-configured prompt templates
3. **Request Caching**: Cache identical requests
4. **Load Balancing**: Distribute load across multiple provider instances
5. **Cost Alerts**: Notifications when cost thresholds are exceeded
6. **A/B Testing**: Route requests to different models for comparison
7. **Web Dashboard**: Optional web-based dashboard (in addition to desktop)
8. **Multi-User**: Support for multiple user profiles

## Module Structure

```
src/
├── main.rs                    # Application entry point
├── server/                    # Web server module
│   ├── mod.rs
│   ├── routes.rs              # API route handlers
│   ├── middleware.rs          # Auth, logging middleware
│   └── openai_compat.rs       # OpenAI compatibility layer
├── config/                    # Configuration management
│   ├── mod.rs
│   ├── settings.rs            # Settings struct and loading
│   ├── storage.rs             # File system operations
│   └── migration.rs           # Config schema migration
├── providers/                 # Model provider implementations
│   ├── mod.rs                 # Provider trait definition
│   ├── ollama.rs
│   ├── openrouter.rs
│   ├── openai.rs
│   ├── anthropic.rs
│   ├── gemini.rs
│   ├── subscription.rs        # OAuth-based providers
│   └── health.rs              # Health check system
├── router/                    # Smart routing system
│   ├── mod.rs
│   ├── engine.rs              # Routing algorithm
│   ├── strategy.rs            # Routing strategies
│   ├── rate_limit.rs          # Rate limiter
│   └── fallback.rs            # Fallback handler
├── api_keys/                  # API key management
│   ├── mod.rs
│   ├── manager.rs             # CRUD operations
│   └── auth.rs                # Authentication
├── monitoring/                # Monitoring & logging
│   ├── mod.rs
│   ├── metrics.rs             # In-memory metrics
│   ├── logger.rs              # Access log writer
│   ├── parser.rs              # Log parser for historical data
│   └── graphs.rs              # Graph data generation
├── ui/                        # Tauri commands
│   ├── mod.rs
│   ├── commands.rs            # Tauri command handlers
│   └── tray.rs                # System tray management
└── utils/                     # Utility functions
    ├── mod.rs
    ├── crypto.rs              # Encryption utilities
    └── errors.rs              # Error types
```

## Development Phases

See PROGRESS.md for detailed feature breakdown and implementation tracking.

1. **Phase 1: Core Infrastructure** (Weeks 1-2)
   - Project setup
   - Configuration system
   - Basic web server
   - Provider trait definition

2. **Phase 2: Provider Implementations** (Weeks 3-4)
   - Implement 2-3 core providers (Ollama, OpenAI, OpenRouter)
   - Health checking system
   - Model catalog

3. **Phase 3: Routing System** (Week 5)
   - Basic routing engine
   - Rate limiting
   - Fallback mechanism

4. **Phase 4: API Key Management** (Week 6)
   - Key generation and storage
   - Authentication middleware
   - Key-to-model mapping

5. **Phase 5: Monitoring** (Week 7)
   - Access logging
   - Metrics collection
   - Log parsing

6. **Phase 6: Desktop UI** (Weeks 8-10)
   - Tauri setup
   - Main app tabs
   - System tray menu
   - Graphs and visualizations

7. **Phase 7: Polish & Testing** (Weeks 11-12)
   - Comprehensive testing
   - Bug fixes
   - Documentation
   - Packaging

---

**Document Version**: 1.0
**Last Updated**: 2026-01-14
**Author**: LocalRouter AI Team
