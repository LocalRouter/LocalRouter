/**
 * Mock data for website demo
 *
 * Types are imported from src/types/tauri-commands.ts to ensure
 * mock data matches the expected Tauri command return types.
 */

// Types for mock data validation - see src/types/tauri-commands.ts for full definitions
import type { ClientInfo, RouteLLMStatus, SkillInfo } from '@app/types/tauri-commands'

// MCP Server interface - simplified for mock data
// Full type: McpServerInfo from @app/types/tauri-commands
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

      context_management_enabled: null,
      catalog_compression_enabled: null,
      created_at: "2025-01-15T10:00:00Z",
      last_used: "2025-02-03T14:30:00Z",
      mcp_permissions: { global: "allow", servers: {}, tools: {}, resources: {}, prompts: {} },
      skills_permissions: { global: "allow", skills: {}, tools: {} },
      coding_agent_permission: "off",
      coding_agent_type: null,
      model_permissions: { global: "allow", providers: {}, models: {} },
      marketplace_permission: "allow",
      mcp_sampling_permission: "ask",
      mcp_elicitation_permission: "ask",
      client_mode: "llm_only",
      template_id: "cursor",
      sync_config: false,
      guardrails_active: true,
      json_repair_active: true,
    },
    {
      id: "client-2",
      client_id: "claude-code",
      name: "Claude Code",
      enabled: true,
      strategy_id: "strategy-default",

      context_management_enabled: true,
      catalog_compression_enabled: null,
      created_at: "2025-01-20T08:00:00Z",
      last_used: "2025-02-03T15:45:00Z",
      mcp_permissions: { global: "ask", servers: { "mcp-github": "allow", "mcp-filesystem": "allow" }, tools: {}, resources: {}, prompts: {} },
      skills_permissions: { global: "allow", skills: {}, tools: {} },
      coding_agent_permission: "allow",
      coding_agent_type: "claude_code",
      model_permissions: { global: "allow", providers: {}, models: { "gpt-4o": "allow", "claude-3-5-sonnet-20241022": "allow" } },
      marketplace_permission: "ask",
      mcp_sampling_permission: "allow",
      mcp_elicitation_permission: "ask",
      client_mode: "both",
      template_id: "claude-code",
      sync_config: true,
      guardrails_active: false,
      json_repair_active: true,
    },
    {
      id: "client-4",
      client_id: "cline-vscode",
      name: "Cline",
      enabled: false,
      strategy_id: "strategy-quality",

      context_management_enabled: null,
      catalog_compression_enabled: null,
      created_at: "2025-01-28T14:00:00Z",
      last_used: null,
      mcp_permissions: { global: "ask", servers: {}, tools: {}, resources: {}, prompts: {} },
      skills_permissions: { global: "ask", skills: {}, tools: {} },
      coding_agent_permission: "ask",
      coding_agent_type: null,
      model_permissions: { global: "ask", providers: {}, models: {} },
      marketplace_permission: "ask",
      mcp_sampling_permission: "ask",
      mcp_elicitation_permission: "ask",
      client_mode: "mcp_via_llm",
      template_id: "cline",
      sync_config: false,
      guardrails_active: false,
      json_repair_active: true,
    },
  ] as unknown as ClientInfo[],

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
      display_name: "Gemini",
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
      default_free_tier: { kind: "none" },
      free_tier_short_text: "",
      free_tier_long_text: "No free tier available. All API usage is billed.",
      free_tier_notes: null,
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
      default_free_tier: { kind: "none" },
      free_tier_short_text: "",
      free_tier_long_text: "No free tier available. All API usage is billed.",
      free_tier_notes: null,
    },
    {
      provider_type: "ollama",
      display_name: "Ollama",
      category: "local",
      description: "Run open-source models locally",
      setup_parameters: [
        { key: "base_url", param_type: "string", required: false, description: "Ollama server URL", default_value: "http://localhost:11434", sensitive: false },
      ],
      default_free_tier: { kind: "always_free_local" },
      free_tier_short_text: "Free — runs locally",
      free_tier_long_text: "Runs entirely on your machine. No API costs, no rate limits.",
      free_tier_notes: null,
    },
    {
      provider_type: "gemini",
      display_name: "Gemini",
      category: "first_party",
      description: "Gemini Pro, Gemini Ultra, and other Google models",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Google AI API key", sensitive: true },
      ],
      default_free_tier: { kind: "rate_limited_free", max_rpm: 10, max_rpd: 20, max_tpm: 250000, max_tpd: 0, max_monthly_calls: 0, max_monthly_tokens: 0 },
      free_tier_short_text: "Free tier: 10 req/min, 20 req/day",
      free_tier_long_text: "Free access within rate limits: 10 req/min, 20 req/day, 250K tokens/min. Router auto-skips when exhausted.\n\nRate limits vary significantly by model: Flash models allow up to 250 RPD while Pro models are limited to 20 RPD. Limits may also vary by region.",
      free_tier_notes: "Rate limits vary significantly by model: Flash models allow up to 250 RPD while Pro models are limited to 20 RPD. Limits may also vary by region.",
    },
    {
      provider_type: "groq",
      display_name: "Groq",
      category: "third_party",
      description: "Ultra-fast inference for Llama and Mixtral",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Groq API key", sensitive: true },
      ],
      default_free_tier: { kind: "rate_limited_free", max_rpm: 30, max_rpd: 14400, max_tpm: 6000, max_tpd: 500000, max_monthly_calls: 0, max_monthly_tokens: 0 },
      free_tier_short_text: "Free tier: 30 req/min, 14.4K req/day",
      free_tier_long_text: "Free access within rate limits: 30 req/min, 14.4K req/day, 6K tokens/min, 500K tokens/day. Router auto-skips when exhausted.\n\nRate limits vary by model. Some models (e.g. Llama 3.3 70B) have lower daily limits (1K RPD). Token limits also vary per model.",
      free_tier_notes: "Rate limits vary by model. Some models (e.g. Llama 3.3 70B) have lower daily limits (1K RPD). Token limits also vary per model.",
    },
    {
      provider_type: "mistral",
      display_name: "Mistral AI",
      category: "first_party",
      description: "Mistral Large, Mistral Medium, and Codestral",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Mistral API key", sensitive: true },
      ],
      default_free_tier: { kind: "rate_limited_free", max_rpm: 60, max_rpd: 0, max_tpm: 500000, max_tpd: 0, max_monthly_calls: 0, max_monthly_tokens: 1000000000 },
      free_tier_short_text: "Free tier: 60 req/min, 500K tokens/min",
      free_tier_long_text: "Free access within rate limits: 60 req/min, 500K tokens/min, 1B tokens/mo. Router auto-skips when exhausted.\n\nFree tier (experiment plan) allows 1 request/second and 1 billion tokens/month. All models are accessible.",
      free_tier_notes: "Free tier (experiment plan) allows 1 request/second and 1 billion tokens/month. All models are accessible.",
    },
    {
      provider_type: "openrouter",
      display_name: "OpenRouter",
      category: "third_party",
      description: "Access multiple providers through one API",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "OpenRouter API key", sensitive: true },
      ],
      default_free_tier: { kind: "free_models_only", free_model_patterns: [":free"], max_rpm: 20 },
      free_tier_short_text: "Free models: 20 req/min",
      free_tier_long_text: "1 free model. Rate-limited to 20 req/min.\n\nFree tier provides access to 25+ free models (model IDs ending in ':free') at 20 RPM / 50 RPD. Purchasing $10+ in credits unlocks 1,000 RPD on free models. BYOK gives 1M free requests/month.",
      free_tier_notes: "Free tier provides access to 25+ free models (model IDs ending in ':free') at 20 RPM / 50 RPD. Purchasing $10+ in credits unlocks 1,000 RPD on free models. BYOK gives 1M free requests/month.",
    },
    {
      provider_type: "together",
      display_name: "Together AI",
      category: "third_party",
      description: "Open-source models with fast inference",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Together AI API key", sensitive: true },
      ],
      default_free_tier: { kind: "free_models_only", free_model_patterns: ["meta-llama/Llama-3.3-70B-Instruct-Turbo-Free"], max_rpm: 3 },
      free_tier_short_text: "Free models: 3 req/min",
      free_tier_long_text: "1 free model. Rate-limited to 3 req/min.\n\nOnly specific models are free (currently Llama 3.3 70B Instruct Turbo Free). Rate limited to 3 RPM on free models.",
      free_tier_notes: "Only specific models are free (currently Llama 3.3 70B Instruct Turbo Free). Rate limited to 3 RPM on free models.",
    },
    {
      provider_type: "deepinfra",
      display_name: "DeepInfra",
      category: "third_party",
      description: "Serverless inference for open models",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "DeepInfra API key", sensitive: true },
      ],
      default_free_tier: { kind: "credit_based", budget_usd: 5, reset_period: "monthly", detection: { type: "local_only" } },
      free_tier_short_text: "$5/mo free credits",
      free_tier_long_text: "$5 in monthly free credits. Router auto-skips when exhausted.\n\n$5 monthly free credits for inference. Credits reset monthly.",
      free_tier_notes: "$5 monthly free credits for inference. Credits reset monthly.",
    },
    {
      provider_type: "perplexity",
      display_name: "Perplexity",
      category: "third_party",
      description: "Search-augmented AI models",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Perplexity API key", sensitive: true },
      ],
      default_free_tier: { kind: "none" },
      free_tier_short_text: "",
      free_tier_long_text: "No free tier available. All API usage is billed.\n\nNo free API tier. All API usage requires payment.",
      free_tier_notes: "No free API tier. All API usage requires payment.",
    },
    {
      provider_type: "xai",
      display_name: "xAI",
      category: "first_party",
      description: "Grok models from xAI",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "xAI API key", sensitive: true },
      ],
      default_free_tier: { kind: "credit_based", budget_usd: 25, reset_period: "never", detection: { type: "local_only" } },
      free_tier_short_text: "$25 free credits",
      free_tier_long_text: "$25 in one-time free credits. Router auto-skips when exhausted.\n\n$25 one-time signup credits. No recurring free tier.",
      free_tier_notes: "$25 one-time signup credits. No recurring free tier.",
    },
    {
      provider_type: "cerebras",
      display_name: "Cerebras",
      category: "third_party",
      description: "Fast inference on Cerebras hardware",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Cerebras API key", sensitive: true },
      ],
      default_free_tier: { kind: "rate_limited_free", max_rpm: 30, max_rpd: 14400, max_tpm: 60000, max_tpd: 1000000, max_monthly_calls: 0, max_monthly_tokens: 0 },
      free_tier_short_text: "Free tier: 30 req/min, 14.4K req/day",
      free_tier_long_text: "Free access within rate limits: 30 req/min, 14.4K req/day, 60K tokens/min, 1M tokens/day. Router auto-skips when exhausted.\n\nDeveloper tier offers 10x higher limits. Exact free tier limits are not publicly documented and may change.",
      free_tier_notes: "Developer tier offers 10x higher limits. Exact free tier limits are not publicly documented and may change.",
    },
    {
      provider_type: "cohere",
      display_name: "Cohere",
      category: "first_party",
      description: "Command and Embed models",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Cohere API key", sensitive: true },
      ],
      default_free_tier: { kind: "rate_limited_free", max_rpm: 20, max_rpd: 0, max_tpm: 100000, max_tpd: 0, max_monthly_calls: 1000, max_monthly_tokens: 0 },
      free_tier_short_text: "Free tier: 20 req/min, 100K tokens/min",
      free_tier_long_text: "Free access within rate limits: 20 req/min, 100K tokens/min, 1K calls/mo. Router auto-skips when exhausted.\n\nTrial API keys are limited to 1,000 API calls/month and 20 RPM. Contact support for production increases.",
      free_tier_notes: "Trial API keys are limited to 1,000 API calls/month and 20 RPM. Contact support for production increases.",
    },
    {
      provider_type: "lmstudio",
      display_name: "LM Studio",
      category: "local",
      description: "Run local models with LM Studio",
      setup_parameters: [
        { key: "base_url", param_type: "string", required: false, description: "LM Studio server URL", default_value: "http://localhost:1234/v1", sensitive: false },
      ],
      default_free_tier: { kind: "always_free_local" },
      free_tier_short_text: "Free — runs locally",
      free_tier_long_text: "Runs entirely on your machine. No API costs, no rate limits.",
      free_tier_notes: null,
    },
    {
      provider_type: "jan",
      display_name: "Jan",
      category: "local",
      description: "Run local models with Jan.ai",
      setup_parameters: [
        { key: "base_url", param_type: "string", required: false, description: "Jan server URL", default_value: "http://localhost:1337/v1", sensitive: false },
      ],
      default_free_tier: { kind: "always_free_local" },
      free_tier_short_text: "Free — runs locally",
      free_tier_long_text: "Runs entirely on your machine. No API costs, no rate limits.",
      free_tier_notes: null,
    },
    {
      provider_type: "gpt4all",
      display_name: "GPT4All",
      category: "local",
      description: "Run local models with GPT4All",
      setup_parameters: [
        { key: "base_url", param_type: "string", required: false, description: "GPT4All server URL", default_value: "http://localhost:4891/v1", sensitive: false },
      ],
      default_free_tier: { kind: "always_free_local" },
      free_tier_short_text: "Free — runs locally",
      free_tier_long_text: "Runs entirely on your machine. No API costs, no rate limits.",
      free_tier_notes: null,
    },
    {
      provider_type: "localai",
      display_name: "LocalAI",
      category: "local",
      description: "Run local models with LocalAI",
      setup_parameters: [
        { key: "base_url", param_type: "string", required: false, description: "LocalAI server URL", default_value: "http://localhost:8080/v1", sensitive: false },
      ],
      default_free_tier: { kind: "always_free_local" },
      free_tier_short_text: "Free — runs locally",
      free_tier_long_text: "Runs entirely on your machine. No API costs, no rate limits.",
      free_tier_notes: null,
    },
    {
      provider_type: "llamacpp",
      display_name: "llama.cpp",
      category: "local",
      description: "Run local models with llama.cpp server",
      setup_parameters: [
        { key: "base_url", param_type: "string", required: false, description: "llama.cpp server URL", default_value: "http://localhost:8080/v1", sensitive: false },
      ],
      default_free_tier: { kind: "always_free_local" },
      free_tier_short_text: "Free — runs locally",
      free_tier_long_text: "Runs entirely on your machine. No API costs, no rate limits.",
      free_tier_notes: null,
    },
    {
      provider_type: "github_models",
      display_name: "GitHub Models",
      category: "third_party",
      description: "GitHub Models free inference API",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "GitHub Personal Access Token", sensitive: true },
      ],
      default_free_tier: { kind: "rate_limited_free", max_rpm: 10, max_rpd: 50, max_tpm: 0, max_tpd: 0, max_monthly_calls: 0, max_monthly_tokens: 0 },
      free_tier_short_text: "Free tier: 10 req/min, 50 req/day",
      free_tier_long_text: "Free access within rate limits: 10 req/min, 50 req/day. Router auto-skips when exhausted.\n\nLimits vary by model tier: Low models get 15 RPM / 150 RPD, High models get 10 RPM / 50 RPD. Uses GitHub Personal Access Token for auth.",
      free_tier_notes: "Limits vary by model tier: Low models get 15 RPM / 150 RPD, High models get 10 RPM / 50 RPD. Uses GitHub Personal Access Token for auth.",
    },
    {
      provider_type: "nvidia_nim",
      display_name: "NVIDIA NIM",
      category: "third_party",
      description: "NVIDIA NIM inference API for 100+ models",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "NVIDIA API key", sensitive: true },
      ],
      default_free_tier: { kind: "rate_limited_free", max_rpm: 40, max_rpd: 0, max_tpm: 0, max_tpd: 0, max_monthly_calls: 0, max_monthly_tokens: 0 },
      free_tier_short_text: "Free tier: 40 req/min",
      free_tier_long_text: "Free access within rate limits: 40 req/min. Router auto-skips when exhausted.\n\n40 RPM on free tier. Access to 100+ models including Llama, Mistral, Qwen. Daily limits undocumented.",
      free_tier_notes: "40 RPM on free tier. Access to 100+ models including Llama, Mistral, Qwen. Daily limits undocumented.",
    },
    {
      provider_type: "cloudflare_ai",
      display_name: "Cloudflare Workers AI",
      category: "third_party",
      description: "Cloudflare Workers AI inference platform",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Cloudflare API token", sensitive: true },
        { key: "base_url", param_type: "string", required: true, description: "Cloudflare AI Gateway URL (find it on the AI Gateway page)", sensitive: false },
      ],
      default_free_tier: { kind: "rate_limited_free", max_rpm: 0, max_rpd: 0, max_tpm: 0, max_tpd: 0, max_monthly_calls: 0, max_monthly_tokens: 0 },
      free_tier_short_text: "Free tier available",
      free_tier_long_text: "Free access within rate limits. Router auto-skips when exhausted.\n\n10,000 neurons/day free allowance. Neuron cost varies by model and input size. Requires Cloudflare account ID in base URL.",
      free_tier_notes: "10,000 neurons/day free allowance. Neuron cost varies by model and input size. Requires Cloudflare account ID in base URL.",
    },
    {
      provider_type: "llm7",
      display_name: "LLM7.io",
      category: "third_party",
      description: "LLM7.io free inference API for open-source models",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: false, description: "LLM7 API token (optional, increases rate limits)", sensitive: true },
      ],
      default_free_tier: { kind: "rate_limited_free", max_rpm: 30, max_rpd: 0, max_tpm: 0, max_tpd: 0, max_monthly_calls: 0, max_monthly_tokens: 0 },
      free_tier_short_text: "Free tier: 30 req/min",
      free_tier_long_text: "Free access within rate limits: 30 req/min. Router auto-skips when exhausted.\n\n30 RPM without token, 120 RPM with token. Access to DeepSeek R1, Qwen2.5 Coder, and 27+ more models.",
      free_tier_notes: "30 RPM without token, 120 RPM with token. Access to DeepSeek R1, Qwen2.5 Coder, and 27+ more models.",
    },
    {
      provider_type: "kluster_ai",
      display_name: "Kluster AI",
      category: "third_party",
      description: "Kluster AI inference for DeepSeek, Llama, and Qwen models",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Kluster AI API key", sensitive: true },
      ],
      default_free_tier: { kind: "rate_limited_free", max_rpm: 30, max_rpd: 0, max_tpm: 0, max_tpd: 0, max_monthly_calls: 0, max_monthly_tokens: 0 },
      free_tier_short_text: "Free tier: 30 req/min",
      free_tier_long_text: "Free access within rate limits: 30 req/min. Router auto-skips when exhausted.\n\nFree tier limits are undocumented. Supports DeepSeek-R1, Llama 4 Maverick, Qwen3-235B.",
      free_tier_notes: "Free tier limits are undocumented. Supports DeepSeek-R1, Llama 4 Maverick, Qwen3-235B.",
    },
    {
      provider_type: "huggingface",
      display_name: "Hugging Face",
      category: "third_party",
      description: "Hugging Face Inference API for thousands of models",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Hugging Face User Access Token", sensitive: true },
      ],
      default_free_tier: { kind: "credit_based", budget_usd: 0.10, reset_period: "monthly", detection: { type: "local_only" } },
      free_tier_short_text: "$0.10/mo free credits",
      free_tier_long_text: "$0.10 in monthly free credits. Router auto-skips when exhausted.\n\n$0.10/month free credits for all users. PRO users get $2/month. No markup — provider costs passed through directly. Uses HF User Access Token.",
      free_tier_notes: "$0.10/month free credits for all users. PRO users get $2/month. No markup — provider costs passed through directly. Uses HF User Access Token.",
    },
    {
      provider_type: "zhipu",
      display_name: "Zhipu AI",
      category: "first_party",
      description: "Zhipu AI GLM models for Chinese-language focused inference",
      setup_parameters: [
        { key: "api_key", param_type: "string", required: true, description: "Zhipu API key", sensitive: true },
      ],
      default_free_tier: { kind: "rate_limited_free", max_rpm: 0, max_rpd: 0, max_tpm: 0, max_tpd: 0, max_monthly_calls: 0, max_monthly_tokens: 0 },
      free_tier_short_text: "Free tier available",
      free_tier_long_text: "Free access within rate limits. Router auto-skips when exhausted.\n\nFree tier limits are undocumented. Supports GLM-4.7-Flash, GLM-4.5-Flash, GLM-4.6V-Flash. Chinese-language focused provider.",
      free_tier_notes: "Free tier limits are undocumented. Supports GLM-4.7-Flash, GLM-4.5-Flash, GLM-4.6V-Flash. Chinese-language focused provider.",
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
      auto_config: {
        permission: "allow" as const,
        model_name: "localrouter/auto",
        prioritized_models: [] as [string, string][],
        available_models: [] as [string, string][],
        routellm_config: null,
      },
      rate_limits: [
        { limit_type: 'requests' as const, value: 100, time_window_seconds: 3600, enabled: true },
        { limit_type: 'cost' as const, value: 5.00, time_window_seconds: 86400, enabled: false },
      ],
      free_tier_only: false,
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
      auto_config: {
        permission: "allow" as const,
        model_name: "localrouter/auto",
        prioritized_models: [
          ["openai-primary", "gpt-4o-mini"],
          ["anthropic-main", "claude-3-5-haiku-20241022"],
          ["groq-fast", "llama-3.2-3b-instruct"],
        ] as [string, string][],
        available_models: [] as [string, string][],
        routellm_config: null,
      },
      rate_limits: [],
      free_tier_only: true,
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
        permission: "allow" as const,
        model_name: "auto",
        prioritized_models: [
          ["openai-primary", "gpt-4o"],
          ["anthropic-main", "claude-3-5-sonnet-20241022"],
          ["gemini-google", "gemini-1.5-pro"],
        ] as [string, string][],
        available_models: [] as [string, string][],
        routellm_config: {
          enabled: true,
          threshold: 0.5,
          weak_models: [
            ["openai-primary", "gpt-4o-mini"],
            ["groq-fast", "llama-3.2-3b-instruct"],
          ] as [string, string][],
        },
      },
      rate_limits: [],
      free_tier_only: false,
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
      auto_config: {
        permission: "allow" as const,
        model_name: "localrouter/auto",
        prioritized_models: [
          ["ollama-local", "llama3.2:latest"],
          ["ollama-local", "codellama:latest"],
          ["ollama-local", "mistral:latest"],
        ] as [string, string][],
        available_models: [] as [string, string][],
        routellm_config: null,
      },
      rate_limits: [],
      free_tier_only: false,
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
    },
  },

  healthCache: {
    aggregate_status: "green" as const,
    server_running: true,
    server_host: "127.0.0.1",
    server_port: 3625,
    providers: {
      "openai-primary": { status: "healthy" as const, name: "OpenAI (Primary)", latency_ms: 245, last_check: new Date().toISOString() },
      "anthropic-main": { status: "healthy" as const, name: "Anthropic", latency_ms: 312, last_check: new Date().toISOString() },
      "ollama-local": { status: "healthy" as const, name: "Ollama (Local)", latency_ms: 45, last_check: new Date().toISOString() },
      "gemini-google": { status: "healthy" as const, name: "Gemini", latency_ms: 198, last_check: new Date().toISOString() },
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

  // RouteLLMStatus - matches src/types/tauri-commands.ts
  routellmStatus: {
    state: 'downloaded_not_running',
    memory_usage_mb: null,
    last_access_secs_ago: null,
    model_dir: '~/.localrouter/routellm',
    model_name: 'routellm/bert_gpt4_augmented',
  } satisfies RouteLLMStatus,

  // UpdateConfig - matches src/types/tauri-commands.ts
  updateConfig: {
    mode: 'automatic' as const,
    check_interval_days: 1,
    last_check: new Date(Date.now() - 3600000).toISOString(),
    skipped_version: null,
  },

  // MarketplaceConfig - matches src/types/tauri-commands.ts
  marketplaceConfig: {
    mcp_enabled: true,
    skills_enabled: true,
    registry_url: "https://registry.localrouter.ai",
    skill_sources: [
      {
        label: "Official Skills",
        repo_url: "https://github.com/localrouter/skills",
        branch: "main",
        skills_path: "skills",
      },
      {
        label: "Community Skills",
        repo_url: "https://github.com/community/localrouter-skills",
        branch: "main",
        skills_path: null,
      },
    ],
    search_tool_name: "MarketplaceSearch",
    install_tool_name: "MarketplaceInstall",
  },

  // TrayGraphSettings - matches src/types/tauri-commands.ts
  trayGraphSettings: {
    enabled: false,
    refresh_rate_secs: 10,
  },

  // Sidebar expanded state - matches UiConfig.sidebar_expanded
  sidebarExpanded: true,

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

  codingAgents: [
    { agentType: "claude_code", displayName: "Claude Code", binaryName: "claude", installed: true },
    { agentType: "gemini_cli", displayName: "Gemini CLI", binaryName: "gemini", installed: true },
    { agentType: "codex", displayName: "Codex", binaryName: "codex", installed: false },
    { agentType: "aider", displayName: "Aider", binaryName: "aider", installed: true },
  ],

  homeDir: "/Users/demo",
  configDir: "/Users/demo/Library/Application Support",

  monitorEvents: [
    {
      id: "mon-001", sequence: 6, timestamp: new Date(Date.now() - 1000).toISOString(),
      event_type: "llm_call", session_id: "sess-001", client_id: "client-1", client_name: "Claude Code",
      status: "complete", duration_ms: 1250,
      summary: "openai/gpt-4o — 1,523 tokens",
      data: { type: "llm_call", endpoint: "/v1/chat/completions", model: "gpt-4o", stream: true, message_count: 12, has_tools: true, tool_count: 5, request_body: { model: "gpt-4o", messages: [{ role: "system", content: "You are a helpful assistant." }, { role: "user", content: "Help me refactor this function." }] }, provider: "openai", status_code: 200, input_tokens: 1200, output_tokens: 323, total_tokens: 1523, cost_usd: 0.0089, latency_ms: 1250, finish_reason: "stop", content_preview: "Here's the implementation you requested...", streamed: true },
    },
    {
      id: "mon-003", sequence: 4, timestamp: new Date(Date.now() - 5000).toISOString(),
      event_type: "mcp_tool_call", session_id: "sess-002", client_id: "client-2", client_name: "Cursor",
      status: "complete", duration_ms: 340,
      summary: "tools/call → filesystem__read_file (OK)",
      data: { type: "mcp_tool_call", tool_name: "filesystem__read_file", server_id: "srv-1", server_name: "Filesystem", arguments: { path: "/Users/demo/project/src/main.rs" }, firewall_action: null, success: true, latency_ms: 340, response_preview: '{"content": [{"type": "text", "text": "file contents here..."}]}', error: null },
    },
    {
      id: "mon-005", sequence: 2, timestamp: new Date(Date.now() - 10000).toISOString(),
      event_type: "llm_call", session_id: null, client_id: "client-3", client_name: "GPT4All",
      status: "error", duration_ms: null,
      summary: "unknown/llama-3.3 — HTTP 502",
      data: { type: "llm_call", endpoint: "/v1/chat/completions", model: "llama-3.3", stream: false, message_count: 1, has_tools: false, tool_count: 0, request_body: { model: "llama-3.3", messages: [{ role: "user", content: "Hello" }] }, provider: "ollama", status_code: 502, error: "Connection refused: could not connect to Ollama at localhost:11434" },
    },
    {
      id: "mon-006", sequence: 1, timestamp: new Date(Date.now() - 30000).toISOString(),
      event_type: "llm_call", session_id: null, client_id: "client-2", client_name: "Cursor",
      status: "complete", duration_ms: null,
      summary: "/v1/embeddings → text-embedding-3-small (1 msgs)",
      data: { type: "llm_call", endpoint: "/v1/embeddings", model: "text-embedding-3-small", stream: false, message_count: 1, has_tools: false, tool_count: 0, request_body: { model: "text-embedding-3-small", input: "search query" } },
    },
    {
      id: "mon-007", sequence: 0, timestamp: new Date(Date.now() - 35000).toISOString(),
      event_type: "auth_error", session_id: null, client_id: null, client_name: null,
      status: "error", duration_ms: null,
      summary: "HTTP 401 /v1/chat/completions — invalid_key",
      data: { type: "auth_error", error_type: "invalid_key", endpoint: "/v1/chat/completions", message: "Invalid API key", status_code: 401 },
    },
    {
      id: "mon-008", sequence: 0, timestamp: new Date(Date.now() - 40000).toISOString(),
      event_type: "access_denied", session_id: null, client_id: "client-3", client_name: "GPT4All",
      status: "error", duration_ms: null,
      summary: "HTTP 403 /mcp — mcp_via_llm_direct_mcp",
      data: { type: "access_denied", reason: "mcp_via_llm_direct_mcp", endpoint: "/mcp", message: "Client is in MCP-via-LLM mode. MCP tools are available through LLM chat completions, not direct MCP access", status_code: 403 },
    },
    {
      id: "mon-009", sequence: 0, timestamp: new Date(Date.now() - 45000).toISOString(),
      event_type: "rate_limit_event", session_id: null, client_id: "client-1", client_name: "Claude Code",
      status: "error", duration_ms: null,
      summary: "HTTP 429 — rate_limit_exceeded",
      data: { type: "rate_limit_event", reason: "rate_limit_exceeded", endpoint: "/v1/chat/completions", message: "Rate limit exceeded: 100/100 used", status_code: 429, retry_after_secs: 60 },
    },
    {
      id: "mon-010", sequence: 0, timestamp: new Date(Date.now() - 50000).toISOString(),
      event_type: "validation_error", session_id: null, client_id: "client-2", client_name: "Cursor",
      status: "error", duration_ms: null,
      summary: "/v1/embeddings — model is required (model)",
      data: { type: "validation_error", endpoint: "/v1/embeddings", field: "model", message: "model is required", status_code: 400 },
    },
    {
      id: "mon-011", sequence: 0, timestamp: new Date(Date.now() - 55000).toISOString(),
      event_type: "mcp_server_event", session_id: null, client_id: null, client_name: null,
      status: "error", duration_ms: null,
      summary: "filesystem: connection_failed — Failed to spawn process",
      data: { type: "mcp_server_event", server_id: "filesystem", server_name: "Filesystem MCP", action: "connection_failed", message: "Failed to spawn process: No such file or directory (os error 2)" },
    },
  ] as Array<Record<string, unknown>>,
}
