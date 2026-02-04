// Mock data for website demo
// Comprehensive dummy data for all app features
// Types imported from main app for type safety

// Permission types - must match @/components/permissions/types.ts
type PermissionState = "allow" | "ask" | "off"

interface McpPermissions {
  global: PermissionState
  servers: Record<string, PermissionState>
  tools: Record<string, PermissionState>
  resources: Record<string, PermissionState>
  prompts: Record<string, PermissionState>
}

interface SkillsPermissions {
  global: PermissionState
  skills: Record<string, PermissionState>
  tools: Record<string, PermissionState>
}

interface ModelPermissions {
  global: PermissionState
  providers: Record<string, PermissionState>
  models: Record<string, PermissionState>
}

// Client interface - must match src/views/clients/index.tsx
interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
  mcp_deferred_loading: boolean
  mcp_permissions: McpPermissions
  skills_permissions: SkillsPermissions
  model_permissions: ModelPermissions
  marketplace_permission: PermissionState
  created_at: string
  last_used: string | null
}

// Skill interface - must match src/views/skills/index.tsx SkillInfo
interface SkillInfo {
  name: string
  description: string | null
  version: string | null
  author: string | null
  tags: string[]
  extra: Record<string, unknown>
  source_path: string
  script_count: number
  reference_count: number
  asset_count: number
  enabled: boolean
}

// MCP Server interface - must match src/views/resources/mcp-servers-panel.tsx
interface McpServer {
  id: string
  name: string | null
  enabled: boolean
  transport: "Stdio" | "Sse"
  transport_config: {
    command?: string
    args?: string[]
    env?: Record<string, string>
    url?: string
    headers?: Record<string, string>
  }
  auth_config: { type: string } | null
  description?: string
  tools_count?: number
}

export const mockData = {
  clients: [
    {
      id: "client-1",
      client_id: "cursor-ide",
      name: "Cursor",
      enabled: true,
      strategy_id: "strategy-default",
      mcp_deferred_loading: false,
      created_at: "2025-01-15T10:00:00Z",
      last_used: "2025-02-03T14:30:00Z",
      mcp_permissions: { global: "allow", servers: {}, tools: {}, resources: {}, prompts: {} },
      skills_permissions: { global: "allow", skills: {}, tools: {} },
      model_permissions: { global: "allow", providers: {}, models: {} },
      marketplace_permission: "allow",
    },
    {
      id: "client-2",
      client_id: "claude-code",
      name: "Claude Code",
      enabled: true,
      strategy_id: "strategy-default",
      mcp_deferred_loading: true,
      created_at: "2025-01-20T08:00:00Z",
      last_used: "2025-02-03T15:45:00Z",
      mcp_permissions: { global: "ask", servers: { "mcp-github": "allow", "mcp-filesystem": "allow" }, tools: {}, resources: {}, prompts: {} },
      skills_permissions: { global: "allow", skills: {}, tools: {} },
      model_permissions: { global: "allow", providers: {}, models: { "gpt-4o": "allow", "claude-3-5-sonnet-20241022": "allow" } },
      marketplace_permission: "ask",
    },
    {
      id: "client-3",
      client_id: "open-webui",
      name: "Open WebUI",
      enabled: true,
      strategy_id: "strategy-fast",
      mcp_deferred_loading: false,
      created_at: "2025-01-25T12:00:00Z",
      last_used: "2025-02-02T09:15:00Z",
      mcp_permissions: { global: "off", servers: {}, tools: {}, resources: {}, prompts: {} },
      skills_permissions: { global: "off", skills: {}, tools: {} },
      model_permissions: { global: "allow", providers: {}, models: {} },
      marketplace_permission: "off",
    },
    {
      id: "client-4",
      client_id: "cline-vscode",
      name: "Cline",
      enabled: false,
      strategy_id: "strategy-quality",
      mcp_deferred_loading: false,
      created_at: "2025-01-28T14:00:00Z",
      last_used: null,
      mcp_permissions: { global: "ask", servers: {}, tools: {}, resources: {}, prompts: {} },
      skills_permissions: { global: "ask", skills: {}, tools: {} },
      model_permissions: { global: "ask", providers: {}, models: {} },
      marketplace_permission: "ask",
    },
  ] as Client[],

  providers: [
    {
      instance_name: "openai-primary",
      provider_type: "openai",
      enabled: true,
      display_name: "OpenAI (Primary)",
      api_key_set: true,
    },
    {
      instance_name: "anthropic-main",
      provider_type: "anthropic",
      enabled: true,
      display_name: "Anthropic",
      api_key_set: true,
    },
    {
      instance_name: "ollama-local",
      provider_type: "ollama",
      enabled: true,
      display_name: "Ollama (Local)",
      api_key_set: false,
      base_url: "http://localhost:11434",
    },
    {
      instance_name: "gemini-google",
      provider_type: "gemini",
      enabled: true,
      display_name: "Google Gemini",
      api_key_set: true,
    },
    {
      instance_name: "groq-fast",
      provider_type: "groq",
      enabled: true,
      display_name: "Groq (Fast)",
      api_key_set: true,
    },
    {
      instance_name: "openrouter-backup",
      provider_type: "openrouter",
      enabled: false,
      display_name: "OpenRouter (Backup)",
      api_key_set: true,
    },
  ],

  providerTypes: [
    {
      provider_type: "openai",
      display_name: "OpenAI",
      category: "first_party",
      description: "GPT-4, GPT-4o, and other OpenAI models",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "OpenAI API key", sensitive: true },
        { key: "base_url", param_type: "string", required: false, description: "Custom API base URL", default_value: "https://api.openai.com/v1", sensitive: false },
        { key: "organization", param_type: "string", required: false, description: "OpenAI organization ID", sensitive: false },
      ],
    },
    {
      provider_type: "anthropic",
      display_name: "Anthropic",
      category: "first_party",
      description: "Claude 3.5 Sonnet, Claude 3 Opus, and other Claude models",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Anthropic API key", sensitive: true },
        { key: "base_url", param_type: "string", required: false, description: "Custom API base URL", default_value: "https://api.anthropic.com", sensitive: false },
      ],
    },
    {
      provider_type: "ollama",
      display_name: "Ollama",
      category: "local",
      description: "Run open-source models locally",
      setup_parameters: [
        { key: "base_url", param_type: "string", required: false, description: "Ollama server URL", default_value: "http://localhost:11434", sensitive: false },
      ],
    },
    {
      provider_type: "gemini",
      display_name: "Google Gemini",
      category: "first_party",
      description: "Gemini Pro, Gemini Ultra, and other Google models",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Google AI API key", sensitive: true },
      ],
    },
    {
      provider_type: "groq",
      display_name: "Groq",
      category: "third_party",
      description: "Ultra-fast inference for Llama and Mixtral",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Groq API key", sensitive: true },
      ],
    },
    {
      provider_type: "mistral",
      display_name: "Mistral AI",
      category: "first_party",
      description: "Mistral Large, Mistral Medium, and Codestral",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Mistral API key", sensitive: true },
      ],
    },
    {
      provider_type: "openrouter",
      display_name: "OpenRouter",
      category: "third_party",
      description: "Access multiple providers through one API",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "OpenRouter API key", sensitive: true },
      ],
    },
    {
      provider_type: "together",
      display_name: "Together AI",
      category: "third_party",
      description: "Open-source models with fast inference",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Together AI API key", sensitive: true },
      ],
    },
    {
      provider_type: "deepinfra",
      display_name: "DeepInfra",
      category: "third_party",
      description: "Serverless inference for open models",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "DeepInfra API key", sensitive: true },
      ],
    },
    {
      provider_type: "perplexity",
      display_name: "Perplexity",
      category: "third_party",
      description: "Search-augmented AI models",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Perplexity API key", sensitive: true },
      ],
    },
    {
      provider_type: "xai",
      display_name: "xAI",
      category: "first_party",
      description: "Grok models from xAI",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "xAI API key", sensitive: true },
      ],
    },
    {
      provider_type: "cerebras",
      display_name: "Cerebras",
      category: "third_party",
      description: "Fast inference on Cerebras hardware",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Cerebras API key", sensitive: true },
      ],
    },
    {
      provider_type: "cohere",
      display_name: "Cohere",
      category: "first_party",
      description: "Command and Embed models",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Cohere API key", sensitive: true },
      ],
    },
    {
      provider_type: "lmstudio",
      display_name: "LM Studio",
      category: "local",
      description: "Run local models with LM Studio",
      setup_parameters: [
        { key: "base_url", param_type: "string", required: false, description: "LM Studio server URL", default_value: "http://localhost:1234/v1", sensitive: false },
      ],
    },
  ],

  mcpServers: [
    {
      id: "mcp-github",
      name: "GitHub",
      enabled: true,
      transport: "Sse",
      transport_config: {
        url: "https://mcp.github.com/sse",
        headers: {},
      },
      auth_config: { type: "oauth_browser" },
      description: "Create issues, PRs, search repos, manage workflows",
      tools_count: 12,
    },
    {
      id: "mcp-filesystem",
      name: "Filesystem",
      enabled: true,
      transport: "Stdio",
      transport_config: {
        command: "npx",
        args: ["-y", "@anthropic/mcp-filesystem"],
        env: {},
      },
      auth_config: null,
      description: "Read, write, and manage local files",
      tools_count: 8,
    },
    {
      id: "mcp-slack",
      name: "Slack",
      enabled: true,
      transport: "Sse",
      transport_config: {
        url: "https://mcp.slack.com/sse",
        headers: { "Authorization": "Bearer ..." },
      },
      auth_config: { type: "oauth_browser" },
      description: "Send messages, manage channels, search history",
      tools_count: 15,
    },
    {
      id: "mcp-postgres",
      name: "PostgreSQL",
      enabled: false,
      transport: "Stdio",
      transport_config: {
        command: "npx",
        args: ["-y", "@anthropic/mcp-postgres"],
        env: { "DATABASE_URL": "postgres://..." },
      },
      auth_config: null,
      description: "Query and manage PostgreSQL databases",
      tools_count: 6,
    },
    {
      id: "mcp-browser",
      name: "Browser Control",
      enabled: true,
      transport: "Stdio",
      transport_config: {
        command: "npx",
        args: ["-y", "@anthropic/mcp-browser"],
        env: {},
      },
      auth_config: null,
      description: "Automate browser interactions and web scraping",
      tools_count: 10,
    },
  ] as McpServer[],

  strategies: [
    {
      id: "strategy-default",
      name: "Default Strategy",
      parent: null,
      allowed_models: {
        mode: "all" as const,
        models: [],
      },
      auto_config: null,
      rate_limits: [],
    },
    {
      id: "strategy-fast",
      name: "Fast & Cheap",
      parent: null,
      allowed_models: {
        mode: "selected" as const,
        models: [
          ["openai-primary", "gpt-4o-mini"],
          ["anthropic-main", "claude-3-5-haiku-20241022"],
          ["groq-fast", "llama-3.2-3b-instruct"],
        ] as [string, string][],
      },
      auto_config: null,
      rate_limits: [],
    },
    {
      id: "strategy-quality",
      name: "High Quality",
      parent: null,
      allowed_models: {
        mode: "selected" as const,
        models: [
          ["openai-primary", "gpt-4o"],
          ["anthropic-main", "claude-3-5-sonnet-20241022"],
          ["gemini-google", "gemini-1.5-pro"],
        ] as [string, string][],
      },
      auto_config: {
        enabled: true,
        model_name: "auto",
        prioritized_models: [
          ["openai-primary", "gpt-4o"],
          ["anthropic-main", "claude-3-5-sonnet-20241022"],
          ["gemini-google", "gemini-1.5-pro"],
        ] as [string, string][],
        available_models: [] as [string, string][],
        routellm_config: {
          enabled: false,
          threshold: 0.5,
          weak_models: [] as [string, string][],
        },
      },
      rate_limits: [],
    },
    {
      id: "strategy-local",
      name: "Local Only",
      parent: null,
      allowed_models: {
        mode: "selected" as const,
        models: [
          ["ollama-local", "llama3.2:latest"],
          ["ollama-local", "codellama:latest"],
          ["ollama-local", "mistral:latest"],
        ] as [string, string][],
      },
      auto_config: null,
      rate_limits: [],
    },
  ],

  models: [
    // OpenAI models
    { id: "gpt-4o", provider: "openai-primary", display_name: "GPT-4o", context_length: 128000 },
    { id: "gpt-4o-mini", provider: "openai-primary", display_name: "GPT-4o Mini", context_length: 128000 },
    { id: "gpt-4-turbo", provider: "openai-primary", display_name: "GPT-4 Turbo", context_length: 128000 },
    { id: "o1-preview", provider: "openai-primary", display_name: "o1 Preview", context_length: 128000 },
    { id: "o1-mini", provider: "openai-primary", display_name: "o1 Mini", context_length: 128000 },
    // Anthropic models
    { id: "claude-3-5-sonnet-20241022", provider: "anthropic-main", display_name: "Claude 3.5 Sonnet", context_length: 200000 },
    { id: "claude-3-5-haiku-20241022", provider: "anthropic-main", display_name: "Claude 3.5 Haiku", context_length: 200000 },
    { id: "claude-3-opus-20240229", provider: "anthropic-main", display_name: "Claude 3 Opus", context_length: 200000 },
    // Gemini models
    { id: "gemini-1.5-pro", provider: "gemini-google", display_name: "Gemini 1.5 Pro", context_length: 1000000 },
    { id: "gemini-1.5-flash", provider: "gemini-google", display_name: "Gemini 1.5 Flash", context_length: 1000000 },
    // Groq models
    { id: "llama-3.2-70b-instruct", provider: "groq-fast", display_name: "Llama 3.2 70B", context_length: 8192 },
    { id: "llama-3.2-3b-instruct", provider: "groq-fast", display_name: "Llama 3.2 3B", context_length: 8192 },
    { id: "mixtral-8x7b-instruct", provider: "groq-fast", display_name: "Mixtral 8x7B", context_length: 32768 },
    // Ollama models
    { id: "llama3.2:latest", provider: "ollama-local", display_name: "Llama 3.2 (Local)", context_length: 8192 },
    { id: "codellama:latest", provider: "ollama-local", display_name: "Code Llama (Local)", context_length: 16384 },
    { id: "mistral:latest", provider: "ollama-local", display_name: "Mistral (Local)", context_length: 8192 },
  ],

  stats: {
    total_requests: 24853,
    total_tokens: 4582917,
    total_cost: 287.45,
    successful_requests: 24712,
    failed_requests: 141,
    avg_latency_ms: 1245,
    requests_today: 1523,
    tokens_today: 285621,
    cost_today: 18.32,
    requests_by_provider: {
      "openai-primary": 12456,
      "anthropic-main": 8234,
      "groq-fast": 2891,
      "ollama-local": 1134,
      "gemini-google": 138,
    },
    requests_by_model: {
      "gpt-4o": 6521,
      "gpt-4o-mini": 5935,
      "claude-3-5-sonnet-20241022": 5890,
      "claude-3-5-haiku-20241022": 2344,
      "llama-3.2-70b-instruct": 1823,
      "llama3.2:latest": 1134,
      "mixtral-8x7b-instruct": 1068,
      "gemini-1.5-flash": 138,
    },
    requests_by_client: {
      "cursor-ide": 15234,
      "claude-code": 7891,
      "open-webui": 1728,
    },
  },

  healthCache: {
    aggregate_status: "green" as const,
    providers: {
      "openai-primary": { status: "healthy" as const, name: "OpenAI (Primary)", latency_ms: 245, last_check: new Date().toISOString() },
      "anthropic-main": { status: "healthy" as const, name: "Anthropic", latency_ms: 312, last_check: new Date().toISOString() },
      "ollama-local": { status: "healthy" as const, name: "Ollama (Local)", latency_ms: 45, last_check: new Date().toISOString() },
      "gemini-google": { status: "healthy" as const, name: "Google Gemini", latency_ms: 198, last_check: new Date().toISOString() },
      "groq-fast": { status: "healthy" as const, name: "Groq (Fast)", latency_ms: 89, last_check: new Date().toISOString() },
      "openrouter-backup": { status: "unknown" as const, name: "OpenRouter (Backup)", latency_ms: null, last_check: null },
    },
    mcp_servers: {
      "mcp-github": { status: "healthy" as const, name: "GitHub", latency_ms: 156, last_check: new Date().toISOString() },
      "mcp-filesystem": { status: "healthy" as const, name: "Filesystem", latency_ms: 12, last_check: new Date().toISOString() },
      "mcp-slack": { status: "healthy" as const, name: "Slack", latency_ms: 178, last_check: new Date().toISOString() },
      "mcp-postgres": { status: "unknown" as const, name: "PostgreSQL", latency_ms: null, last_check: null },
      "mcp-browser": { status: "healthy" as const, name: "Browser Control", latency_ms: 23, last_check: new Date().toISOString() },
    },
  },

  serverConfig: {
    host: "127.0.0.1",
    port: 3625,
    cors_origins: ["*"],
    max_connections: 100,
  },

  networkInterfaces: [
    { name: "lo0", ip: "127.0.0.1", is_loopback: true },
    { name: "en0", ip: "192.168.1.105", is_loopback: false },
    { name: "en1", ip: "10.0.0.50", is_loopback: false },
  ],

  skills: [
    {
      name: "web-search",
      description: "Search the web and return summarized results",
      version: "1.2.0",
      author: "LocalRouter",
      tags: ["search", "web", "utility"],
      extra: { license: "MIT", homepage: "https://localrouter.ai/skills/web-search" },
      source_path: "~/.localrouter/skills/web-search",
      script_count: 3,
      reference_count: 0,
      asset_count: 1,
      enabled: true,
    },
    {
      name: "code-review",
      description: "Analyze code for bugs, security issues, and improvements",
      version: "2.0.1",
      author: "LocalRouter",
      tags: ["code", "review", "security"],
      extra: { license: "MIT" },
      source_path: "~/.localrouter/skills/code-review",
      script_count: 5,
      reference_count: 2,
      asset_count: 0,
      enabled: true,
    },
    {
      name: "doc-writer",
      description: "Generate documentation from code and comments",
      version: "1.0.0",
      author: "Community",
      tags: ["documentation", "markdown"],
      extra: {},
      source_path: "~/.localrouter/skills/doc-writer",
      script_count: 2,
      reference_count: 1,
      asset_count: 0,
      enabled: true,
    },
    {
      name: "test-generator",
      description: "Generate unit tests and integration tests",
      version: "0.9.0",
      author: "Community",
      tags: ["testing", "automation"],
      extra: { beta: true },
      source_path: "~/.localrouter/skills/test-generator",
      script_count: 4,
      reference_count: 0,
      asset_count: 0,
      enabled: false,
    },
  ] as SkillInfo[],

  loggingConfig: {
    log_dir: "~/.localrouter/logs",
    max_log_files: 10,
    max_file_size_mb: 50,
    llm_logging_enabled: true,
    mcp_logging_enabled: true,
    log_level: "info",
  },

  llmLogs: [
    {
      id: "log-1",
      timestamp: new Date(Date.now() - 60000).toISOString(),
      client_id: "cursor-ide",
      model: "gpt-4o",
      provider: "openai-primary",
      request_tokens: 1250,
      response_tokens: 856,
      latency_ms: 2341,
      status: "success",
      cost: 0.042,
    },
    {
      id: "log-2",
      timestamp: new Date(Date.now() - 120000).toISOString(),
      client_id: "claude-code",
      model: "claude-3-5-sonnet-20241022",
      provider: "anthropic-main",
      request_tokens: 3420,
      response_tokens: 1523,
      latency_ms: 3156,
      status: "success",
      cost: 0.089,
    },
    {
      id: "log-3",
      timestamp: new Date(Date.now() - 180000).toISOString(),
      client_id: "cursor-ide",
      model: "gpt-4o-mini",
      provider: "openai-primary",
      request_tokens: 856,
      response_tokens: 234,
      latency_ms: 876,
      status: "success",
      cost: 0.003,
    },
  ],

  mcpLogs: [
    {
      id: "mcp-log-1",
      timestamp: new Date(Date.now() - 30000).toISOString(),
      client_id: "claude-code",
      server_id: "mcp-github",
      tool: "create_issue",
      latency_ms: 456,
      status: "success",
    },
    {
      id: "mcp-log-2",
      timestamp: new Date(Date.now() - 90000).toISOString(),
      client_id: "claude-code",
      server_id: "mcp-filesystem",
      tool: "read_file",
      latency_ms: 12,
      status: "success",
    },
  ],

  routellmStatus: {
    enabled: false,
    model_loaded: false,
    model_path: null,
    threshold: 0.5,
    cache_size: 1000,
  },

  updateConfig: {
    auto_check: true,
    check_interval_hours: 24,
    last_check: new Date(Date.now() - 3600000).toISOString(),
    skipped_versions: [] as string[],
  },

  marketplaceConfig: {
    enabled: true,
    registry_url: "https://registry.localrouter.ai",
    skill_sources: [
      {
        repo_url: "https://github.com/localrouter/skills",
        branch: "main",
        path: "skills",
        label: "Official Skills",
      },
      {
        repo_url: "https://github.com/community/localrouter-skills",
        branch: "main",
        path: ".",
        label: "Community Skills",
      },
    ],
  },

  trayGraphSettings: {
    enabled: true,
    refresh_rate_secs: 10,
    show_requests: true,
    show_tokens: true,
  },

  activeConnections: [
    {
      id: "conn-1",
      client_id: "cursor-ide",
      client_name: "Cursor",
      connected_at: new Date(Date.now() - 3600000).toISOString(),
      last_activity: new Date(Date.now() - 30000).toISOString(),
      requests_count: 142,
    },
    {
      id: "conn-2",
      client_id: "claude-code",
      client_name: "Claude Code",
      connected_at: new Date(Date.now() - 1800000).toISOString(),
      last_activity: new Date(Date.now() - 60000).toISOString(),
      requests_count: 67,
    },
  ],

  oauthClients: [],

  oauthCredentials: [
    {
      id: "oauth-github",
      provider: "github",
      name: "GitHub OAuth",
      created_at: "2025-01-15T10:00:00Z",
      expires_at: "2025-07-15T10:00:00Z",
    },
    {
      id: "oauth-slack",
      provider: "slack",
      name: "Slack OAuth",
      created_at: "2025-01-20T14:00:00Z",
      expires_at: "2025-07-20T14:00:00Z",
    },
  ],

  homeDir: "/Users/demo",
}
