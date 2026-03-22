/**
 * TypeScript types for Tauri commands (request parameters and return values).
 *
 * This file contains:
 * - Response types: Mirror Rust structs returned from #[tauri::command] functions
 * - Request types: Parameters passed to invoke() calls (at end of file)
 *
 * Each type includes a comment linking to its Rust source file.
 *
 * !! SYNC REQUIRED - UPDATE WHEN MODIFYING TAURI COMMANDS !!
 *
 * When you add or modify a Tauri command:
 * 1. Update/add the response type (if command returns data)
 * 2. Update/add the request params type (in "Command Parameters" section)
 * 3. Update website/src/components/demo/TauriMockSetup.ts with mock data
 * 4. Run `npx tsc --noEmit` to verify types compile
 *
 * Usage:
 *   import type { RouteLLMTestResult, RouteLLMTestPredictionParams } from '@/types/tauri-commands'
 *   const result = await invoke<RouteLLMTestResult>('routellm_test_prediction', params satisfies RouteLLMTestPredictionParams)
 *
 * See CLAUDE.md "Adding/Modifying Tauri Commands" for full checklist.
 */

// =============================================================================
// Permission & Access Control Types
// Rust: crates/lr-config/src/types.rs
// =============================================================================

/**
 * Unified permission state for access control.
 * Rust: crates/lr-config/src/types.rs - PermissionState enum
 */
export type PermissionState = 'allow' | 'ask' | 'off'

/**
 * Hierarchical MCP permission system.
 * Rust: crates/lr-config/src/types.rs - McpPermissions struct
 */
export interface McpPermissions {
  global: PermissionState
  servers: Record<string, PermissionState>
  tools: Record<string, PermissionState>
  resources: Record<string, PermissionState>
  prompts: Record<string, PermissionState>
}

/**
 * Hierarchical skills permission system.
 * Rust: crates/lr-config/src/types.rs - SkillsPermissions struct
 */
export interface SkillsPermissions {
  global: PermissionState
  skills: Record<string, PermissionState>
  tools: Record<string, PermissionState>
}

/**
 * Hierarchical coding agents permission system.
 * Rust: crates/lr-config/src/types.rs - CodingAgentsPermissions struct
 */
export interface CodingAgentsPermissions {
  global: PermissionState
  agents: Record<string, PermissionState>
}

/**
 * Hierarchical model permission system.
 * Rust: crates/lr-config/src/types.rs - ModelPermissions struct
 */
export interface ModelPermissions {
  global: PermissionState
  providers: Record<string, PermissionState>
  models: Record<string, PermissionState>
}

// =============================================================================
// Indexing Eligibility Types
// Rust: crates/lr-config/src/types.rs
// =============================================================================

/**
 * Indexing state for gateway and client tools.
 * Rust: crates/lr-config/src/types.rs - IndexingState enum
 */
export type IndexingState = 'enable' | 'disable'

/**
 * Unified MCP Gateway indexing permissions (GLOBAL only).
 * Hierarchy: global → server → tool.
 * Rust: crates/lr-config/src/types.rs - GatewayIndexingPermissions struct
 */
export interface GatewayIndexingPermissions {
  global: IndexingState
  servers: Record<string, IndexingState>
  tools: Record<string, IndexingState>
}

/**
 * Virtual MCP server indexing info for UI display.
 * Rust: src-tauri/src/ui/commands.rs - VirtualMcpIndexingInfo struct
 */
export interface VirtualMcpIndexingInfo {
  id: string
  display_name: string
  tools: VirtualMcpToolIndexingInfo[]
}

/**
 * Single virtual MCP tool indexing info.
 * Rust: src-tauri/src/ui/commands.rs - VirtualMcpToolIndexingInfo struct
 */
export interface VirtualMcpToolIndexingInfo {
  name: string
  indexable: boolean
}

/**
 * Client Tools indexing permissions — global default + per-tool overrides.
 * Rust: crates/lr-config/src/types.rs - ClientToolsIndexingPermissions struct
 */
export interface ClientToolsIndexingPermissions {
  global: IndexingState | null
  tools: Record<string, IndexingState>
}

/**
 * Known client tool entry from the built-in registry.
 * Rust: crates/lr-config/src/known_client_tools.rs - KnownToolEntry struct
 */
export interface KnownToolEntry {
  name: string
  default_state: IndexingState
  indexable: boolean
}

/** Params for set_gateway_indexing_permission */
export interface SetGatewayIndexingPermissionParams {
  level: string
  key?: string | null
  state: string
}

/** Params for set_virtual_indexing_permission */
export interface SetVirtualIndexingPermissionParams {
  level: string
  key?: string | null
  state: string
}

/** Params for get_known_client_tools */
export interface GetKnownClientToolsParams {
  templateId: string
}

/** Params for get_seen_client_tools */
export interface GetSeenClientToolsParams {
  clientId: string
}

/** Params for get_client_tools_indexing */
export interface GetClientToolsIndexingParams {
  clientId: string
}

/** Params for set_client_tools_indexing */
export interface SetClientToolsIndexingParams {
  clientId: string
  level: string
  key?: string | null
  state?: string | null
}

// =============================================================================
// Client Types
// Rust: src-tauri/src/ui/commands_clients.rs
// =============================================================================

/**
 * Client mode determines which features are exposed.
 * Rust: crates/lr-config/src/types.rs - ClientMode enum
 */
export type ClientMode = 'both' | 'llm_only' | 'mcp_only' | 'mcp_via_llm'

/**
 * Client information returned from list_clients and create_client.
 * Rust: src-tauri/src/ui/commands_clients.rs - ClientInfo struct
 */
export interface ClientInfo {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
  /** Per-client context management override (null = inherit global, false = disabled) */
  context_management_enabled: boolean | null
  created_at: string
  last_used: string | null
  mcp_permissions: McpPermissions
  skills_permissions: SkillsPermissions
  coding_agent_permission: PermissionState
  coding_agent_type: CodingAgentType | null
  model_permissions: ModelPermissions
  marketplace_permission: PermissionState
  /** Sampling permission (Allow/Ask/Off) */
  mcp_sampling_permission: PermissionState
  /** Elicitation permission (Allow/Ask/Off) */
  mcp_elicitation_permission: PermissionState
  client_mode: ClientMode
  template_id: string | null
  sync_config: boolean
  guardrails_active: boolean
  json_repair_active: boolean
}

/**
 * Effective (inheritance-resolved) configuration for a client.
 * Rust: src-tauri/src/ui/commands_clients.rs - ClientEffectiveConfig struct
 */
export interface ClientEffectiveConfig {
  strategy_name: string
  context_management_effective: boolean
  context_management_source: 'client' | 'global'
  catalog_compression_effective: boolean
  catalog_compression_source: 'client' | 'global'
  json_repair_effective: boolean
  json_repair_source: 'client' | 'global'
}

/**
 * App capabilities: installation status and supported modes.
 * Rust: src-tauri/src/ui/commands_clients.rs - AppCapabilities struct
 */
export interface AppCapabilities {
  installed: boolean
  binary_path: string | null
  version: string | null
  supports_try_it_out: boolean
  supports_permanent_config: boolean
}

/**
 * Result of a configure or launch operation.
 * Rust: src-tauri/src/ui/commands_clients.rs - LaunchResult struct
 */
export interface LaunchResult {
  success: boolean
  message: string
  modified_files: string[]
  backup_files: string[]
  /** For CLI apps: the command the user should run in their terminal */
  terminal_command?: string | null
}

// =============================================================================
// Strategy Types
// Rust: crates/lr-config/src/types.rs
// =============================================================================

/**
 * Models selection configuration for strategies.
 * Rust: crates/lr-config/src/types.rs - AvailableModelsSelection struct
 */
export interface AvailableModelsSelection {
  mode: 'all' | 'selected'
  models: string[]
}

/**
 * RouteLLM routing configuration within auto model config.
 * Rust: crates/lr-config/src/types.rs - RouteLLMConfig struct
 */
export interface RouteLLMConfig {
  enabled: boolean
  threshold: number
  weak_models: [string, string][]
}

/**
 * Auto routing configuration for localrouter/auto virtual model.
 * Rust: crates/lr-config/src/types.rs - AutoModelConfig struct
 */
export interface AutoModelConfig {
  permission: PermissionState
  model_name: string
  prioritized_models: [string, string][]
  available_models: [string, string][]
  routellm_config?: RouteLLMConfig | null
}

/**
 * Rate limit configuration for strategies.
 * Rust: crates/lr-config/src/types.rs - StrategyRateLimit struct
 */
export interface StrategyRateLimit {
  limit_type: 'requests' | 'tokens' | 'cost'
  value: number
  time_window_seconds: number
  enabled?: boolean
}

/**
 * Strategy configuration.
 * Rust: crates/lr-config/src/types.rs - Strategy struct
 */
/** Rust: crates/lr-config/src/types.rs - FreeTierFallback enum */
export type FreeTierFallback = 'off' | 'ask' | 'allow'

export interface Strategy {
  id: string
  name: string
  parent?: string | null
  allowed_models: AvailableModelsSelection
  /** Always present after config v25 migration (unified model selection) */
  auto_config?: AutoModelConfig | null
  rate_limits: StrategyRateLimit[]
  free_tier_only?: boolean
  free_tier_fallback?: FreeTierFallback
}

// =============================================================================
// Free Tier Types
// Rust: crates/lr-config/src/types.rs, src-tauri/src/ui/commands_free_tier.rs
// =============================================================================

/**
 * Free tier reset period.
 * Rust: crates/lr-config/src/types.rs - FreeTierResetPeriod enum
 */
export type FreeTierResetPeriod = 'daily' | 'monthly' | 'never'

/**
 * Credit detection method.
 * Rust: crates/lr-config/src/types.ts - CreditDetection enum
 */
export type CreditDetection =
  | { type: 'local_only' }
  | { type: 'provider_api' }
  | { type: 'custom_endpoint'; url: string; method: string; headers: [string, string][]; remaining_credits_path: string | null; total_credits_path: string | null; is_free_tier_path: string | null }

/**
 * Free tier kind discriminated union.
 * Rust: crates/lr-config/src/types.rs - FreeTierKind enum
 */
export type FreeTierKind =
  | { kind: 'none' }
  | { kind: 'always_free_local' }
  | { kind: 'subscription' }
  | { kind: 'rate_limited_free'; max_rpm: number; max_rpd: number; max_tpm: number; max_tpd: number; max_monthly_calls: number; max_monthly_tokens: number }
  | { kind: 'credit_based'; budget_usd: number; reset_period: FreeTierResetPeriod; detection: CreditDetection }
  | { kind: 'free_models_only'; free_model_patterns: string[]; max_rpm: number }

/**
 * Provider free tier status for UI display.
 * Rust: src-tauri/src/ui/commands_free_tier.rs - ProviderFreeTierStatus struct
 */
export interface ProviderFreeTierStatus {
  provider_instance: string
  provider_type: string
  display_name: string
  free_tier: FreeTierKind
  is_user_override: boolean
  supports_credit_check: boolean
  rate_rpm_used: number | null
  rate_rpm_limit: number | null
  rate_rpd_used: number | null
  rate_rpd_limit: number | null
  rate_tpm_used: number | null
  rate_tpm_limit: number | null
  rate_tpd_used: number | null
  rate_tpd_limit: number | null
  rate_monthly_calls_used: number | null
  rate_monthly_calls_limit: number | null
  rate_monthly_tokens_used: number | null
  rate_monthly_tokens_limit: number | null
  credit_used_usd: number | null
  credit_budget_usd: number | null
  credit_remaining_usd: number | null
  is_backed_off: boolean
  backoff_retry_after_secs: number | null
  backoff_reason: string | null
  has_capacity: boolean
  status_message: string
}

// =============================================================================
// Provider Types
// Rust: crates/lr-providers/src/registry.rs, src-tauri/src/providers/mod.rs
// =============================================================================

/**
 * Provider instance information.
 * Rust: crates/lr-providers/src/registry.rs - ProviderInstanceInfo struct
 */
export interface ProviderInstanceInfo {
  instance_name: string
  provider_type: string
  provider_name: string
  created_at: string
  enabled: boolean
}

/**
 * Provider type information from the registry.
 * Rust: crates/lr-providers/src/registry.rs - ProviderTypeInfo struct
 */
export interface ProviderTypeInfo {
  provider_type: string
  display_name: string
  category: string
  description: string
  setup_parameters: SetupParameter[]
  default_free_tier: FreeTierKind
  free_tier_short_text: string
  free_tier_long_text: string
}

/**
 * Setup parameter for provider configuration.
 * Rust: crates/lr-providers/src/registry.rs - SetupParameter struct
 */
export interface SetupParameter {
  name: string
  label: string
  param_type: 'string' | 'password' | 'boolean' | 'number'
  required: boolean
  default_value?: string | null
  placeholder?: string | null
  help_text?: string | null
}

/**
 * Provider health status.
 * Rust: src-tauri/src/providers/mod.rs - ProviderHealth struct
 */
export interface ProviderHealth {
  status: 'healthy' | 'degraded' | 'unhealthy' | 'unknown'
  latency_ms?: number | null
  last_checked?: string | null
  error_message?: string | null
}

/**
 * Provider key status for listing.
 * Rust: src-tauri/src/ui/commands_providers.rs - ProviderKeyStatus struct
 */
export interface ProviderKeyStatus {
  provider: string
  has_key: boolean
}

// =============================================================================
// Model Types
// Rust: src-tauri/src/providers/mod.rs
// =============================================================================

/**
 * Model information.
 * Rust: src-tauri/src/providers/mod.rs - ModelInfo struct
 */
export interface ModelInfo {
  id: string
  name: string
  provider: string
  parameter_count?: number | null
  context_window?: number | null
  supports_streaming: boolean
  capabilities: string[]
  detailed_capabilities?: ModelCapabilities | null
}

/**
 * Detailed model capabilities.
 * Rust: src-tauri/src/providers/mod.rs - ModelCapabilities struct
 */
export interface ModelCapabilities {
  chat: boolean
  completion: boolean
  embeddings: boolean
  function_calling: boolean
  vision: boolean
  json_mode: boolean
  structured_outputs: boolean
}

/**
 * Detailed model info with pricing.
 * Rust: src-tauri/src/ui/commands_providers.rs - DetailedModelInfo struct
 */
export interface DetailedModelInfo extends ModelInfo {
  pricing?: ModelPricing | null
}

/**
 * Model pricing information.
 * Rust: crates/lr-catalog/src/types.rs - ModelPricing struct
 */
export interface ModelPricing {
  input_per_million: number
  output_per_million: number
  cache_read_per_million?: number | null
  cache_write_per_million?: number | null
}

// =============================================================================
// MCP Server Types
// Rust: src-tauri/src/ui/commands_mcp.rs, crates/lr-mcp/src/manager.rs
// =============================================================================

/**
 * MCP transport configuration.
 * Rust: crates/lr-config/src/types.rs - McpTransportConfig enum
 */
export type McpTransportConfig =
  | { type: 'stdio'; command: string; args?: string[]; env?: Record<string, string> }
  | { type: 'http_sse'; url: string }
  | { type: 'websocket'; url: string }

/**
 * MCP authentication configuration.
 * Rust: crates/lr-config/src/types.rs - McpAuthConfig enum
 */
export type McpAuthConfig =
  | { type: 'none' }
  | { type: 'bearer_token'; token: string }
  | { type: 'custom_headers'; headers: Record<string, string> }
  | { type: 'oauth'; client_id: string; client_secret: string; token_url: string; scopes?: string[] }
  | { type: 'oauth_browser'; authorization_url: string; token_url: string; client_id: string; scopes?: string[] }
  | { type: 'env_vars'; vars: Record<string, string> }

/**
 * MCP server information.
 * Rust: src-tauri/src/ui/commands_mcp.rs - McpServerInfo struct
 */
export interface McpServerInfo {
  id: string
  name: string
  transport: string
  transport_config: McpTransportConfig
  auth_config?: McpAuthConfig | null
  enabled: boolean
  running: boolean
  created_at: string
  proxy_url: string
  gateway_url: string
  url?: string | null
}

/**
 * MCP server health status.
 * Rust: crates/lr-mcp/src/manager.rs - McpServerHealth struct
 */
export interface McpServerHealth {
  server_id: string
  server_name: string
  status: 'healthy' | 'ready' | 'unhealthy' | 'unknown'
  latency_ms?: number | null
  error?: string | null
  last_check?: string | null
}

/**
 * MCP server capabilities (tools, resources, prompts).
 * Rust: src-tauri/src/ui/commands_mcp.rs - McpServerCapabilities struct
 */
export interface McpServerCapabilities {
  tools: McpTool[]
  resources: McpResource[]
  prompts: McpPrompt[]
  server_name: string
}

/** Rust: src-tauri/src/ui/commands_mcp.rs - McpGatewaySettingsResponse struct */
export interface McpGatewaySettingsResponse {
  sampling: 'passthrough' | 'direct_allow' | 'direct_ask' | 'off'
  elicitation_mode: 'passthrough' | 'direct' | 'off'
}

/** Params for set_mcp_gateway_settings */
export interface SetMcpGatewaySettingsParams {
  sampling?: string | null
  elicitationMode?: string | null
}

/**
 * MCP tool definition.
 */
export interface McpTool {
  name: string
  description?: string | null
  input_schema?: Record<string, unknown> | null
}

/**
 * MCP resource definition.
 */
export interface McpResource {
  uri: string
  name: string
  description?: string | null
  mime_type?: string | null
}

/**
 * MCP prompt definition.
 */
export interface McpPrompt {
  name: string
  description?: string | null
  arguments?: McpPromptArgument[] | null
}

/**
 * MCP prompt argument.
 */
export interface McpPromptArgument {
  name: string
  description?: string | null
  required?: boolean
}

/**
 * MCP token stats for a client.
 * Rust: src-tauri/src/ui/commands_mcp.rs - McpTokenStats struct
 */
export interface McpTokenStats {
  total_input_tokens: number
  total_output_tokens: number
  servers: Record<string, { input_tokens: number; output_tokens: number }>
}

// =============================================================================
// Skills Types
// Rust: crates/lr-skills/src/types.rs
// =============================================================================

/**
 * Skill information.
 * Rust: crates/lr-skills/src/types.rs - SkillInfo struct
 */
export interface SkillInfo {
  name: string
  version?: string | null
  description?: string | null
  author?: string | null
  tags: string[]
  extra: Record<string, unknown>
  source_path: string
  script_count: number
  reference_count: number
  asset_count: number
  enabled: boolean
}

/**
 * Skill definition with full details.
 * Rust: crates/lr-skills/src/types.rs - SkillDefinition struct
 */
export interface SkillDefinition {
  name: string
  version?: string | null
  description?: string | null
  author?: string | null
  tags: string[]
  mcp_servers?: string[] | null
  tools?: SkillToolDefinition[] | null
  extra: Record<string, unknown>
}

/**
 * Skill tool definition.
 */
export interface SkillToolDefinition {
  name: string
  description?: string | null
  input_schema?: Record<string, unknown> | null
}

/**
 * Skill tool info returned from get_skill_tools.
 * Rust: src-tauri/src/ui/commands.rs - SkillToolInfo struct
 */
export interface SkillToolInfo {
  name: string
  description?: string | null
}

/**
 * Skill file info returned from get_skill_files.
 * Rust: src-tauri/src/ui/commands.rs - SkillFileInfo struct
 */
export interface SkillFileInfo {
  name: string
  category: 'script' | 'reference' | 'asset'
  content_preview?: string | null
}

/**
 * Skills configuration.
 * Rust: crates/lr-config/src/types.rs - SkillsConfig struct
 */
export interface SkillsConfig {
  paths: string[]
  disabled_skills: string[]
  async_enabled: boolean
  tool_name: string
}

/**
 * Context management configuration.
 * Rust: crates/lr-config/src/types.rs - ContextManagementConfig struct
 */
export interface ContextManagementConfig {
  catalog_compression: boolean
  catalog_threshold_bytes: number
  response_threshold_bytes: number
  gateway_indexing: GatewayIndexingPermissions
  virtual_indexing: GatewayIndexingPermissions
  client_tools_indexing_default: IndexingState
  search_tool_name: string
  read_tool_name: string
  /** Enable semantic vector search globally (hybrid FTS5 + embeddings) */
  vector_search_enabled: boolean
}

/** Params for update_context_management_config */
export interface UpdateContextManagementConfigParams {
  catalogCompression?: boolean
  catalogThresholdBytes?: number
  responseThresholdBytes?: number
  searchToolName?: string
  readToolName?: string
  clientToolsIndexingDefault?: string
  vectorSearchEnabled?: boolean
}

/** Rust: src-tauri/src/ui/commands.rs - CatalogCompressionPreview */
export interface CatalogCompressionPreview {
  welcome_message: string
  welcome_message_uncompressed: string
  uncompressed_size: number
  compressed_size: number
  welcome_size: number
  tool_definitions_size: number
  compressed_tool_definitions_size: number
  indexed_welcomes_count: number
  deferred_servers_count: number
  welcome_toc_dropped_count: number
  batch_toc_dropped_count: number
  servers: PreviewServerEntry[]
}

/** Rust: src-tauri/src/ui/commands.rs - PreviewServerEntry */
export interface PreviewServerEntry {
  name: string
  is_virtual: boolean
  tool_names: string[]
  resource_names: string[]
  prompt_names: string[]
  description: string | null
  instructions: string | null
  /** "visible" | "compressed" | "deferred" | "truncated" */
  compression_state: string
  tools: PreviewToolDetail[]
  resources: PreviewResourceDetail[]
  prompts: PreviewPromptDetail[]
}

/** Tool detail for compression preview */
export interface PreviewToolDetail {
  name: string
  description: string | null
  input_schema: unknown | null
}

/** Resource detail for compression preview */
export interface PreviewResourceDetail {
  name: string
  uri: string | null
  description: string | null
}

/** Prompt detail for compression preview */
export interface PreviewPromptDetail {
  name: string
  description: string | null
}

/** Params for preview_catalog_compression */
export interface PreviewCatalogCompressionParams {
  catalogThresholdBytes: number
  source?: string | null
}

/** Params for toggle_client_context_management */
export interface ToggleClientContextManagementParams {
  clientId: string
  enabled: boolean | null
}

/** Params for toggle_client_catalog_compression */
export interface ToggleClientCatalogCompressionParams {
  clientId: string
  enabled: boolean | null
}

/**
 * A catalog source entry from a context management session.
 * Rust: crates/lr-mcp/src/gateway/gateway.rs - CatalogSourceEntry struct
 */
export interface CatalogSourceEntry {
  source_label: string
  item_type: string
  activated: boolean
}

/** Params for terminate_session */
export interface TerminateSessionParams {
  sessionId: string
}

/** Params for get_session_context_sources */
export interface GetSessionContextSourcesParams {
  sessionId: string
}

/** Params for get_session_context_stats */
export interface GetSessionContextStatsParams {
  sessionId: string
}

/** Params for query_session_context_index */
export interface QuerySessionContextIndexParams {
  sessionId: string
  query: string
  source?: string | null
}

/**
 * Active gateway session info.
 * Rust: crates/lr-mcp/src/gateway/gateway.rs - ActiveSessionInfo struct
 */
export interface ActiveSessionInfo {
  session_id: string
  client_id: string
  client_name: string
  duration_secs: number
  initialized_servers: number
  failed_servers: number
  total_tools: number
  context_management_enabled: boolean
  cm_indexed_sources: number
  cm_activated_tools: number
  cm_total_tools: number
  cm_catalog_threshold_bytes: number
}

// =============================================================================
// Response RAG Preview Types
// Rust: src-tauri/src/ui/commands.rs, crates/lr-context/src/types.rs
// =============================================================================

/** Content type for search hits */
export type RagContentType = 'prose' | 'code'

/** Match layer for search hits */
export type RagMatchLayer = 'porter' | 'trigram' | 'fuzzy'

/** Rust: crates/lr-context/src/types.rs - ChunkToc */
export interface RagChunkToc {
  title: string
  line_ref: string
  depth: number
}

/** Rust: crates/lr-context/src/types.rs - IndexResult */
export interface RagIndexResult {
  source_id: number
  label: string
  total_chunks: number
  code_chunks: number
  total_lines: number
  content_bytes: number
  chunk_titles: RagChunkToc[]
}

/** Rust: crates/lr-context/src/types.rs - SearchHit */
export interface RagSearchHit {
  title: string
  content: string
  source: string
  rank: number
  content_type: RagContentType
  match_layer: RagMatchLayer
  line_start: number
  line_end: number
}

/** Rust: crates/lr-context/src/types.rs - SearchResult */
export interface RagSearchResult {
  query: string
  hits: RagSearchHit[]
  corrected_query: string | null
}

/** Rust: crates/lr-context/src/types.rs - ReadResult */
export interface RagReadResult {
  label: string
  content: string
  total_lines: number
  showing_start: string
  showing_end: string
}

/** Rust: crates/lr-context/src/types.rs - SourceInfo */
export interface RagSourceInfo {
  label: string
  total_lines: number
  chunk_count: number
  code_chunk_count: number
}

/** Rust: src-tauri/src/ui/commands.rs - RagPreviewIndexResult */
export interface RagPreviewIndexResult {
  compressed_preview: string
  index_result: RagIndexResult
  sources: RagSourceInfo[]
}

/** Params for preview_rag_index */
export interface PreviewRagIndexParams {
  content: string
  label: string
  responseThresholdBytes: number
}

/** Params for preview_rag_search */
export interface PreviewRagSearchParams {
  query?: string | null
  queries?: string[] | null
  limit?: number | null
  source?: string | null
}

/** Params for preview_rag_read */
export interface PreviewRagReadParams {
  label: string
  offset?: string | null
  limit?: number | null
}

// =============================================================================
// Statistics & Metrics Types
// Rust: crates/lr-server/src/state.rs, crates/lr-monitoring/src/graphs.rs
// =============================================================================

/**
 * Aggregate statistics for the dashboard.
 * Rust: crates/lr-server/src/state.rs - AggregateStats struct
 */
export interface AggregateStats {
  total_requests: number
  total_tokens: number
  total_cost: number
  successful_requests: number
}

/**
 * Feature-level statistics for the dashboard.
 * Rust: crates/lr-server/src/state.rs - FeatureStatsSnapshot struct
 */
export interface FeatureStatsSnapshot {
  routellm_strong: number
  routellm_weak: number
  json_repairs: number
  compression_tokens_saved: number
  compression_cost_saved_micros: number
  context_mgmt_tokens_saved: number
}

/**
 * Graph data for charts.
 * Rust: crates/lr-monitoring/src/graphs.rs - GraphData struct
 */
export interface GraphData {
  labels: string[]
  datasets: Dataset[]
  rate_limits?: RateLimitInfo[] | null
}

/**
 * Dataset for graph charts.
 * Rust: crates/lr-monitoring/src/graphs.rs - Dataset struct
 */
export interface Dataset {
  label: string
  data: number[]
  background_color?: string | null
  border_color?: string | null
  fill?: boolean | null
  tension?: number | null
}

/**
 * Rate limit info for graph annotations.
 * Rust: crates/lr-monitoring/src/graphs.rs - RateLimitInfo struct
 */
export interface RateLimitInfo {
  limit_type: string
  value: number
  time_window_seconds: number
}

/**
 * Time range for metrics queries.
 */
export type TimeRange = 'hour' | 'day' | 'week' | 'month'

/**
 * Metric type for LLM metrics.
 */
export type MetricType = 'requests' | 'tokens' | 'cost' | 'latency'

/**
 * Metric type for MCP metrics.
 */
export type McpMetricType = 'requests' | 'latency' | 'errors'

// =============================================================================
// Health & Status Types
// Rust: crates/lr-providers/src/health_cache.rs
// =============================================================================

/**
 * Item health status enum.
 * Rust: crates/lr-providers/src/health_cache.rs - ItemHealthStatus enum
 */
export type ItemHealthStatus = 'healthy' | 'degraded' | 'unhealthy' | 'ready' | 'pending' | 'disabled'

/**
 * Aggregate health status for the system.
 * Rust: crates/lr-providers/src/health_cache.rs - AggregateHealthStatus enum
 */
export type AggregateHealthStatus = 'red' | 'green' | 'yellow'

/**
 * Individual item health information.
 * Rust: crates/lr-providers/src/health_cache.rs - ItemHealth struct
 */
export interface ItemHealth {
  name: string
  status: ItemHealthStatus
  latency_ms?: number | null
  error?: string | null
  last_checked: string
}

/**
 * Complete health cache state.
 * Rust: crates/lr-providers/src/health_cache.rs - HealthCacheState struct
 */
export interface HealthCacheState {
  server_running: boolean
  server_host?: string | null
  server_port?: number | null
  providers: Record<string, ItemHealth>
  mcp_servers: Record<string, ItemHealth>
  last_refresh?: string | null
  aggregate_status: AggregateHealthStatus
}

// =============================================================================
// Server Configuration Types
// Rust: src-tauri/src/ui/commands.rs
// =============================================================================

/**
 * Server configuration information.
 * Rust: src-tauri/src/ui/commands.rs - ServerConfigInfo struct
 */
export interface ServerConfigInfo {
  host: string
  port: number
  actual_port?: number | null
  enable_cors: boolean
}

/**
 * Available runtimes detected on the system.
 * Rust: src-tauri/src/ui/commands.rs - AvailableRuntimes struct
 */
export interface AvailableRuntimes {
  npx: boolean
  uvx: boolean
  docker: boolean
}

/**
 * Network interface information.
 * Rust: src-tauri/src/ui/commands.rs - NetworkInterface struct
 */
export interface NetworkInterface {
  name: string
  ip: string
  is_loopback: boolean
}

// =============================================================================
// Logging Types
// Rust: crates/lr-monitoring/src/logger.rs, src-tauri/src/ui/commands.rs
// =============================================================================

/**
 * LLM access log entry.
 * Rust: crates/lr-monitoring/src/logger.rs - AccessLogEntry struct
 */
export interface AccessLogEntry {
  id: string
  timestamp: string
  client_id?: string | null
  provider: string
  model: string
  request_tokens: number
  response_tokens: number
  latency_ms: number
  status: 'success' | 'error'
  cost?: number | null
  error?: string | null
  routellm_win_rate?: number | null
}

/**
 * MCP access log entry.
 * Rust: crates/lr-monitoring/src/mcp_logger.rs - McpAccessLogEntry struct
 */
export interface McpAccessLogEntry {
  id: string
  timestamp: string
  client_id: string
  server_id: string
  method: string
  tool?: string | null
  status: 'success' | 'error'
  latency_ms: number
  transport: string
  firewall_action?: string | null
  error?: string | null
}

/**
 * Logging configuration response.
 * Rust: src-tauri/src/ui/commands.rs - LoggingConfigResponse struct
 */
export interface LoggingConfigResponse {
  enabled: boolean
  log_dir: string
}

// =============================================================================
// RouteLLM Types
// Rust: crates/lr-routellm/src/status.rs
// =============================================================================

/**
 * RouteLLM operational state.
 * Rust: crates/lr-routellm/src/status.rs - RouteLLMState enum
 */
export type RouteLLMState =
  | 'not_downloaded'
  | 'downloading'
  | 'downloaded_not_running'
  | 'initializing'
  | 'started'

/**
 * RouteLLM status information.
 * Rust: crates/lr-routellm/src/status.rs - RouteLLMStatus struct
 */
export interface RouteLLMStatus {
  state: RouteLLMState
  memory_usage_mb?: number | null
  last_access_secs_ago?: number | null
  /** Display path for the routellm directory (e.g. ~/.localrouter-dev/routellm/) */
  model_dir: string
  /** HuggingFace model identifier (e.g. routellm/bert_gpt4_augmented) */
  model_name: string
}

/**
 * RouteLLM test prediction result.
 * Rust: crates/lr-routellm/src/status.rs - RouteLLMTestResult struct
 */
export interface RouteLLMTestResult {
  is_strong: boolean
  win_rate: number
  latency_ms: number
}

// =============================================================================
// Update Configuration Types
// Rust: crates/lr-config/src/types.rs
// =============================================================================

/**
 * Update mode.
 * Rust: crates/lr-config/src/types.rs - UpdateMode enum
 */
export type UpdateMode = 'manual' | 'automatic'

/**
 * Update configuration.
 * Rust: crates/lr-config/src/types.rs - UpdateConfig struct
 */
export interface UpdateConfig {
  mode: UpdateMode
  check_interval_days: number
  last_check?: string | null
  skipped_version?: string | null
}

// =============================================================================
// Tray & UI Types
// Rust: src-tauri/src/ui/commands.rs
// =============================================================================

/**
 * Tray graph settings.
 * Rust: src-tauri/src/ui/commands.rs - TrayGraphSettings struct
 */
export interface TrayGraphSettings {
  enabled: boolean
  refresh_rate_secs: number
}

// =============================================================================
// OAuth Types
// Rust: src-tauri/src/ui/commands.rs
// =============================================================================

/**
 * OAuth provider information.
 * Rust: src-tauri/src/ui/commands.rs - OAuthProviderInfo struct
 */
export interface OAuthProviderInfo {
  provider_id: string
  provider_name: string
}

/**
 * OAuth client information.
 * Rust: src-tauri/src/ui/commands.rs - OAuthClientInfo struct
 */
export interface OAuthClientInfo {
  id: string
  name: string
  client_id: string
  linked_server_ids: string[]
  enabled: boolean
  created_at: string
}

/**
 * OAuth flow result.
 * Rust: src-tauri/src/ui/commands.rs - OAuthFlowResult struct
 */
export interface OAuthFlowResult {
  flow_id: string
  auth_url?: string | null
}

/**
 * MCP OAuth browser flow result.
 * Rust: src-tauri/src/ui/commands_mcp.rs - OAuthBrowserFlowResult struct
 */
export interface OAuthBrowserFlowResult {
  flow_id: string
  auth_url: string
}

/**
 * MCP OAuth browser flow status.
 * Rust: src-tauri/src/ui/commands_mcp.rs - OAuthBrowserFlowStatus struct
 */
export interface OAuthBrowserFlowStatus {
  status: 'pending' | 'success' | 'error' | 'cancelled'
  error?: string | null
}

// =============================================================================
// Marketplace Types
// Rust: crates/lr-marketplace/src/types.rs, crates/lr-config/src/types.rs
// =============================================================================

/**
 * Marketplace skill source configuration.
 * Rust: crates/lr-config/src/types.rs - MarketplaceSkillSource struct
 */
export interface MarketplaceSkillSource {
  label: string
  repo_url: string
  branch?: string | null
  skills_path?: string | null
}

/**
 * Marketplace configuration.
 * Rust: crates/lr-config/src/types.rs - MarketplaceConfig struct
 */
export interface MarketplaceConfig {
  mcp_enabled: boolean
  skills_enabled: boolean
  registry_url: string
  skill_sources: MarketplaceSkillSource[]
  search_tool_name: string
  install_tool_name: string
}

/**
 * MCP server package info from marketplace.
 * Rust: crates/lr-marketplace/src/types.rs - McpPackage struct
 */
export interface McpPackage {
  registry: string
  name: string
  version?: string | null
  runtime?: string | null
  license?: string | null
}

/**
 * MCP server remote endpoint.
 * Rust: crates/lr-marketplace/src/types.rs - McpRemote struct
 */
export interface McpRemote {
  transport_type: string
  url: string
}

/**
 * MCP server listing from marketplace search.
 * Rust: crates/lr-marketplace/src/types.rs - McpServerListing struct
 */
export interface McpServerListing {
  name: string
  description?: string | null
  source_id: string
  homepage?: string | null
  vendor?: string | null
  packages: McpPackage[]
  remotes: McpRemote[]
  available_transports: string[]
  install_hint?: string | null
}

/**
 * Skill file info for marketplace listings.
 * Rust: crates/lr-marketplace/src/types.rs - SkillFileReference struct
 */
export interface SkillFileReference {
  path: string
  url: string
}

/**
 * Skill listing from marketplace search.
 * Rust: crates/lr-marketplace/src/types.rs - SkillListing struct
 */
export interface SkillListing {
  name: string
  description?: string | null
  source_id: string
  author?: string | null
  version?: string | null
  tags?: string[] | null
  source_label?: string | null
  source_repo?: string | null
  source_path?: string | null
  source_branch?: string | null
  skill_md_url?: string | null
  is_multi_file: boolean
  files: SkillFileReference[]
}

/**
 * Marketplace cache status.
 * Rust: crates/lr-marketplace/src/service.rs - CacheStatus struct
 */
export interface CacheStatus {
  mcp_servers_cached: number
  skills_cached: number
  last_refresh?: string | null
}

/**
 * MCP server install configuration.
 * Rust: crates/lr-marketplace/src/types.rs - McpInstallConfig struct
 */
export interface McpInstallConfig {
  name: string
  transport_type: string
  command?: string | null
  args?: string[] | null
  url?: string | null
  env?: Record<string, string> | null
}

/**
 * Installed server result.
 * Rust: crates/lr-marketplace/src/types.rs - InstalledServer struct
 */
export interface InstalledServer {
  server_id: string
  name: string
}

/**
 * Installed skill result.
 * Rust: crates/lr-marketplace/src/types.rs - InstalledSkill struct
 */
export interface InstalledSkill {
  skill_name: string
  install_path: string
}

// =============================================================================
// Firewall Approval Types
// Rust: src-tauri/src/ui/commands_clients.rs
// =============================================================================

/**
 * Pending firewall approval info.
 * Rust: src-tauri/src/state.rs - PendingApprovalInfo struct
 */
export interface PendingApprovalInfo {
  request_id: string
  client_id: string
  client_name: string
  item_type: 'mcp_tool' | 'mcp_resource' | 'skill_tool'
  item_name: string
  server_id?: string | null
  server_name?: string | null
  skill_name?: string | null
  description?: string | null
  created_at: string
}

/**
 * Firewall approval action.
 * Rust: crates/lr-mcp/src/gateway/firewall.rs - FirewallApprovalAction enum
 */
export type FirewallApprovalAction = 'deny' | 'deny_session' | 'deny_always' | 'block_categories' | 'allow_once' | 'allow_session' | 'allow_1_minute' | 'allow_1_hour' | 'allow_permanent' | 'allow_categories' | 'deny_1_hour'

// =============================================================================
// Active Connections Types
// Rust: src-tauri/src/ui/commands.rs
// =============================================================================

/**
 * Active connection information.
 * Rust: crates/lr-server/src/state.rs - ActiveConnection struct
 */
export interface ActiveConnection {
  client_id: string
  client_name: string
  connected_at: string
  last_activity: string
  requests_count: number
  transport: string
}

// =============================================================================
// Catalog Types
// Rust: crates/lr-catalog/catalog/catalog.rs
// =============================================================================

/**
 * Model catalog metadata.
 * Rust: crates/lr-catalog/catalog/catalog.rs - CatalogMetadata struct
 */
export interface CatalogMetadata {
  version: string
  generated_at: string
  provider_count: number
  model_count: number
}

/**
 * Model catalog statistics.
 * Rust: crates/lr-catalog/catalog/catalog.rs - CatalogStats struct
 */
export interface CatalogStats {
  total_providers: number
  total_models: number
  models_with_pricing: number
  models_with_context_window: number
}

/**
 * Model pricing override.
 * Rust: crates/lr-config/src/types.rs - ModelPricingOverride struct
 */
export interface ModelPricingOverride {
  input_per_million: number
  output_per_million: number
}

// =============================================================================
// Inline OAuth Flow Types (for MCP server creation)
// Rust: src-tauri/src/ui/commands_mcp.rs
// =============================================================================

/**
 * Inline OAuth flow result.
 * Rust: src-tauri/src/ui/commands_mcp.rs - InlineOAuthFlowResult struct
 */
export interface InlineOAuthFlowResult {
  flow_id: string
  auth_url: string
}

/**
 * Inline OAuth flow status.
 * Rust: src-tauri/src/ui/commands_mcp.rs - InlineOAuthFlowStatus struct
 */
export interface InlineOAuthFlowStatus {
  status: 'pending' | 'success' | 'error' | 'cancelled'
  tokens?: OAuthTokens | null
  error?: string | null
}

/**
 * OAuth tokens from successful flow.
 */
export interface OAuthTokens {
  access_token: string
  refresh_token?: string | null
  expires_at?: string | null
}

// =============================================================================
// MCP OAuth Discovery
// Rust: src-tauri/src/ui/commands_mcp.rs
// =============================================================================

/**
 * MCP OAuth endpoint discovery result.
 * Rust: crates/lr-mcp/src/oauth/discovery.rs - McpOAuthDiscovery struct
 */
export interface McpOAuthDiscovery {
  authorization_endpoint: string
  token_endpoint: string
  registration_endpoint?: string | null
  scopes_supported?: string[] | null
}

// =============================================================================
// Pending Install Types (Marketplace)
// Rust: crates/lr-marketplace/src/types.rs
// =============================================================================

/**
 * Pending install information for marketplace items.
 * Rust: crates/lr-marketplace/src/service.rs - PendingInstallInfo struct
 */
export interface PendingInstallInfo {
  request_id: string
  install_type: 'mcp_server' | 'skill'
  name: string
  description?: string | null
  source: string
  created_at: string
}

// =============================================================================
// =============================================================================
//
//                        COMMAND REQUEST PARAMETERS
//
// Parameter types for invoke() calls. Use with `satisfies` for type safety:
//   invoke<ResponseType>('command_name', params satisfies ParamsType)
//
// Naming convention: {CommandName}Params (PascalCase command + "Params")
// Note: Rust uses snake_case params, frontend converts to camelCase
//
// =============================================================================
// =============================================================================

// =============================================================================
// Client Commands
// Rust: src-tauri/src/ui/commands_clients.rs
// =============================================================================

/** Params for create_client */
export interface CreateClientParams {
  name: string
}

/** Params for delete_client */
export interface DeleteClientParams {
  clientId: string
}

/** Params for clone_client */
export interface CloneClientParams {
  clientId: string
}

/** Params for update_client_name */
export interface UpdateClientNameParams {
  clientId: string
  name: string
}

/** Params for toggle_client_enabled */
export interface ToggleClientEnabledParams {
  clientId: string
  enabled: boolean
}

/** Params for rotate_client_secret */
export interface RotateClientSecretParams {
  clientId: string
}

/** Params for get_client */
export interface GetClientParams {
  clientId: string
}

/** Params for get_client_value */
export interface GetClientValueParams {
  id: string
}

/** Params for get_client_effective_config */
export interface GetClientEffectiveConfigParams {
  clientId: string
}

/** Params for set_client_mode */
export interface SetClientModeParams {
  clientId: string
  mode: ClientMode
}

/** Params for set_client_template */
export interface SetClientTemplateParams {
  clientId: string
  templateId: string | null
}

/** Params for get_client_guardrails_config */
export interface GetClientGuardrailsConfigParams {
  clientId: string
}

/** Params for update_client_guardrails_config */
export interface UpdateClientGuardrailsConfigParams {
  clientId: string
  configJson: string
}

/** Params for get_app_capabilities */
export interface GetAppCapabilitiesParams {
  templateId: string
}

/** Params for try_it_out_app */
export interface TryItOutAppParams {
  clientId: string
}

/** Params for configure_app_permanent */
export interface ConfigureAppPermanentParams {
  clientId: string
}

/** Params for toggle_client_sync_config */
export interface ToggleClientSyncConfigParams {
  clientId: string
  enabled: boolean
}

/** Params for sync_client_config */
export interface SyncClientConfigParams {
  clientId: string
}

// =============================================================================
// Strategy Commands
// Rust: src-tauri/src/ui/commands_clients.rs
// =============================================================================

/** Params for create_strategy */
export interface CreateStrategyParams {
  name: string
  parent?: string | null
}

/** Params for get_strategy */
export interface GetStrategyParams {
  strategyId: string
}

/** Params for update_strategy */
export interface UpdateStrategyParams {
  strategyId: string
  name?: string | null
  allowedModels?: AvailableModelsSelection | null
  autoConfig?: AutoModelConfig | null
  rateLimits?: StrategyRateLimit[] | null
  freeTierOnly?: boolean | null
  freeTierFallback?: FreeTierFallback | null
}

/** Params for delete_strategy */
export interface DeleteStrategyParams {
  strategyId: string
}

/** Params for get_clients_using_strategy */
export interface GetClientsUsingStrategyParams {
  strategyId: string
}

/** Status of one client for a specific Optimize feature.
 * Rust: src-tauri/src/ui/commands_clients.rs - ClientFeatureStatus struct */
export interface ClientFeatureStatus {
  client_id: string
  client_name: string
  /** Whether the feature is effectively active for this client */
  active: boolean
  /** "override" if per-client setting exists, "global" if inherited */
  source: 'override' | 'global'
  /** Feature-specific effective value (e.g. "ask", "notify", "off" for secret_scanning) */
  effective_value?: string
}

/** Params for get_feature_clients_status */
export interface GetFeatureClientsStatusParams {
  feature: 'json_repair' | 'prompt_compression' | 'guardrails' | 'secret_scanning' | 'catalog_compression' | 'context_management' | 'memory' | 'strong_weak' | 'coding_agents'
}

// =============================================================================
// Permission Commands
// Rust: src-tauri/src/ui/commands_clients.rs
// =============================================================================

/** Permission level for MCP permissions */
export type McpPermissionLevel = 'global' | 'server' | 'tool' | 'resource' | 'prompt'

/** Permission level for skills permissions */
export type SkillsPermissionLevel = 'global' | 'skill' | 'tool'

/** Permission level for model permissions */
export type ModelPermissionLevel = 'global' | 'provider' | 'model'

/** Params for set_client_mcp_permission */
export interface SetClientMcpPermissionParams {
  clientId: string
  level: McpPermissionLevel
  key?: string | null
  state: PermissionState
  clear?: boolean
}

/** Params for set_client_skills_permission */
export interface SetClientSkillsPermissionParams {
  clientId: string
  level: SkillsPermissionLevel
  key?: string | null
  state: PermissionState
  clear?: boolean
}

/** Params for set_client_model_permission */
export interface SetClientModelPermissionParams {
  clientId: string
  level: ModelPermissionLevel
  key?: string | null
  state: PermissionState
  clear?: boolean
}

/** Params for set_client_marketplace_permission */
export interface SetClientMarketplacePermissionParams {
  clientId: string
  state: PermissionState
}

/** Params for set_client_sampling_permission */
export interface SetClientSamplingPermissionParams {
  clientId: string
  state: PermissionState
}

/** Params for set_client_elicitation_permission */
export interface SetClientElicitationPermissionParams {
  clientId: string
  state: PermissionState
}

/** Params for clear_client_mcp_child_permissions */
export interface ClearClientMcpChildPermissionsParams {
  clientId: string
  /** If provided, only clear children of this server. If null, clear all children. */
  serverId?: string | null
}

/** Params for clear_client_skills_child_permissions */
export interface ClearClientSkillsChildPermissionsParams {
  clientId: string
  /** If provided, only clear children of this skill. If null, clear all children. */
  skillName?: string | null
}

/** Params for clear_client_model_child_permissions */
export interface ClearClientModelChildPermissionsParams {
  clientId: string
  /** If provided, only clear children of this provider. If null, clear all children. */
  provider?: string | null
}

// =============================================================================
// Provider Commands
// Rust: src-tauri/src/ui/commands_providers.rs
// =============================================================================

/** Params for set_provider_api_key */
export interface SetProviderApiKeyParams {
  provider: string
  apiKey: string
}

/** Params for has_provider_api_key */
export interface HasProviderApiKeyParams {
  provider: string
}

/** Params for delete_provider_api_key */
export interface DeleteProviderApiKeyParams {
  provider: string
}

/** Params for create_provider_instance */
export interface CreateProviderInstanceParams {
  instanceName: string
  providerType: string
  config: Record<string, string>
}

/** Params for get_provider_config */
export interface GetProviderConfigParams {
  instanceName: string
}

/** Params for update_provider_instance */
export interface UpdateProviderInstanceParams {
  instanceName: string
  providerType: string
  config: Record<string, string>
}

/** Params for rename_provider_instance */
export interface RenameProviderInstanceParams {
  instanceName: string
  newName: string
}

/** Params for get_provider_api_key */
export interface GetProviderApiKeyParams {
  instanceName: string
}

/** Params for remove_provider_instance */
export interface RemoveProviderInstanceParams {
  instanceName: string
}

/** Params for clone_provider_instance */
export interface CloneProviderInstanceParams {
  instanceName: string
}

/** Params for set_provider_enabled */
export interface SetProviderEnabledParams {
  instanceName: string
  enabled: boolean
}

/** Params for check_single_provider_health */
export interface CheckSingleProviderHealthParams {
  instanceName: string
}

/** Params for list_provider_models */
export interface ListProviderModelsParams {
  instanceName: string
}

/** Event payload for models-provider-loaded */
export interface ProviderModelsPayload {
  provider: string
  models: Array<{ id: string; provider: string }>
}

/** Event payload for models-refresh-started */
export interface ModelsRefreshStartedPayload {
  providers: string[]
}

// =============================================================================
// MCP Server Commands
// Rust: src-tauri/src/ui/commands_mcp.rs
// =============================================================================

/** Params for create_mcp_server */
export interface CreateMcpServerParams {
  name: string
  transport: string
  transportConfig: Record<string, unknown>
  authConfig?: Record<string, unknown> | null
}

/** Params for delete_mcp_server */
export interface DeleteMcpServerParams {
  serverId: string
}

/** Params for clone_mcp_server */
export interface CloneMcpServerParams {
  serverId: string
}

/** Params for start_mcp_server */
export interface StartMcpServerParams {
  serverId: string
}

/** Params for stop_mcp_server */
export interface StopMcpServerParams {
  serverId: string
}

/** Params for get_mcp_server_health */
export interface GetMcpServerHealthParams {
  serverId: string
}

/** Params for check_single_mcp_health */
export interface CheckSingleMcpHealthParams {
  serverId: string
}

/** Params for update_mcp_server_name */
export interface UpdateMcpServerNameParams {
  serverId: string
  name: string
}

/** Params for update_mcp_server_config */
export interface UpdateMcpServerConfigParams {
  serverId: string
  name: string
  transportConfig: Record<string, unknown>
  authConfig?: Record<string, unknown> | null
}

/** Params for update_mcp_server */
export interface UpdateMcpServerParams {
  serverId: string
  updates: Record<string, unknown>
}

/** Params for toggle_mcp_server_enabled */
export interface ToggleMcpServerEnabledParams {
  serverId: string
  enabled: boolean
}

/** Params for list_mcp_tools */
export interface ListMcpToolsParams {
  serverId: string
}

/** Params for call_mcp_tool */
export interface CallMcpToolParams {
  serverId: string
  toolName: string
  arguments: Record<string, unknown>
}

/** Params for get_mcp_token_stats */
export interface GetMcpTokenStatsParams {
  clientId: string
}

/** Params for get_mcp_server_capabilities */
export interface GetMcpServerCapabilitiesParams {
  serverId: string
}

// =============================================================================
// MCP OAuth Commands
// Rust: src-tauri/src/ui/commands_mcp.rs
// =============================================================================

/** Params for start_mcp_oauth_browser_flow */
export interface StartMcpOAuthBrowserFlowParams {
  serverId: string
}

/** Params for poll_mcp_oauth_browser_status */
export interface PollMcpOAuthBrowserStatusParams {
  serverId: string
}

/** Params for cancel_mcp_oauth_browser_flow */
export interface CancelMcpOAuthBrowserFlowParams {
  serverId: string
}

/** Params for discover_mcp_oauth_endpoints */
export interface DiscoverMcpOAuthEndpointsParams {
  baseUrl: string
}

/** Params for test_mcp_oauth_connection */
export interface TestMcpOAuthConnectionParams {
  serverId: string
}

/** Params for revoke_mcp_oauth_tokens */
export interface RevokeMcpOAuthTokensParams {
  serverId: string
}

/** Params for start_inline_oauth_flow */
export interface StartInlineOAuthFlowParams {
  mcpUrl: string
  clientId?: string | null
  clientSecret?: string | null
}

/** Params for poll_inline_oauth_status */
export interface PollInlineOAuthStatusParams {
  flowId: string
}

/** Params for cancel_inline_oauth_flow */
export interface CancelInlineOAuthFlowParams {
  flowId: string
}

// =============================================================================
// RouteLLM Commands
// Rust: src-tauri/src/ui/commands_routellm.rs
// =============================================================================

/** Params for routellm_test_prediction */
export interface RouteLLMTestPredictionParams {
  prompt: string
  threshold: number
}

/** Params for routellm_update_settings */
export interface RouteLLMUpdateSettingsParams {
  idleTimeoutSecs: number
}

// =============================================================================
// Skills Commands
// Rust: src-tauri/src/ui/commands.rs
// =============================================================================

/** Params for get_skill */
export interface GetSkillParams {
  skillName: string
}

/** Params for add_skill_source */
export interface AddSkillSourceParams {
  path: string
}

/** Params for remove_skill_source */
export interface RemoveSkillSourceParams {
  path: string
}

/** Params for create_skill */
export interface CreateSkillParams {
  name: string
  description: string | null
  content: string
}

/** Params for is_user_created_skill */
export interface IsUserCreatedSkillParams {
  skillPath: string
}

/** Params for delete_user_skill */
export interface DeleteUserSkillParams {
  skillName: string
  skillPath: string
}

/** Params for set_skill_enabled */
export interface SetSkillEnabledParams {
  skillName: string
  enabled: boolean
}

/** Params for get_skill_tools */
export interface GetSkillToolsParams {
  skillName: string
}

/** Params for get_skill_files */
export interface GetSkillFilesParams {
  skillName: string
}

// =============================================================================
// Metrics Commands
// Rust: src-tauri/src/ui/commands_metrics.rs
// =============================================================================

/** Params for get_global_metrics */
export interface GetGlobalMetricsParams {
  timeRange: TimeRange
  metricType: MetricType
}

/** Params for get_api_key_metrics */
export interface GetApiKeyMetricsParams {
  apiKeyId: string
  timeRange: TimeRange
  metricType: MetricType
}

/** Params for get_provider_metrics */
export interface GetProviderMetricsParams {
  provider: string
  timeRange: TimeRange
  metricType: MetricType
}

/** Params for get_model_metrics */
export interface GetModelMetricsParams {
  model: string
  timeRange: TimeRange
  metricType: MetricType
}

/** Params for get_strategy_metrics */
export interface GetStrategyMetricsParams {
  strategyId: string
  timeRange: TimeRange
  metricType: MetricType
}

/** Params for compare_api_keys */
export interface CompareApiKeysParams {
  apiKeyIds: string[]
  timeRange: TimeRange
  metricType: MetricType
}

/** Params for compare_providers */
export interface CompareProvidersParams {
  providers: string[]
  timeRange: TimeRange
  metricType: MetricType
}

/** Params for compare_models */
export interface CompareModelsParams {
  models: string[]
  timeRange: TimeRange
  metricType: MetricType
}

/** Params for compare_strategies */
export interface CompareStrategiesParams {
  strategyIds: string[]
  timeRange: TimeRange
  metricType: MetricType
}

// =============================================================================
// MCP Metrics Commands
// Rust: src-tauri/src/ui/commands_mcp_metrics.rs
// =============================================================================

/** Params for get_global_mcp_metrics */
export interface GetGlobalMcpMetricsParams {
  timeRange: TimeRange
  metricType: McpMetricType
}

/** Params for get_client_mcp_metrics */
export interface GetClientMcpMetricsParams {
  clientId: string
  timeRange: TimeRange
  metricType: McpMetricType
}

/** Params for get_mcp_server_metrics */
export interface GetMcpServerMetricsParams {
  serverId: string
  timeRange: TimeRange
  metricType: McpMetricType
}

/** Params for get_mcp_method_breakdown */
export interface GetMcpMethodBreakdownParams {
  scope: string
  timeRange: TimeRange
}

/** Params for compare_mcp_clients */
export interface CompareMcpClientsParams {
  clientIds: string[]
  timeRange: TimeRange
  metricType: McpMetricType
}

/** Params for compare_mcp_servers */
export interface CompareMcpServersParams {
  serverIds: string[]
  timeRange: TimeRange
  metricType: McpMetricType
}

/** Params for get_mcp_latency_percentiles */
export interface GetMcpLatencyPercentilesParams {
  scope: string
  timeRange: TimeRange
}

// =============================================================================
// Logging Commands
// Rust: src-tauri/src/ui/commands.rs
// =============================================================================

/** Params for get_llm_logs */
export interface GetLlmLogsParams {
  limit?: number
  offset?: number
  clientName?: string
  provider?: string
  model?: string
}

/** Params for get_mcp_logs */
export interface GetMcpLogsParams {
  limit?: number
  offset?: number
  clientId?: string
  serverId?: string
}

/** Params for update_logging_config */
export interface UpdateLoggingConfigParams {
  enabled: boolean
}

// =============================================================================
// Server Config Commands
// Rust: src-tauri/src/ui/commands.rs
// =============================================================================

/** Params for update_server_config */
export interface UpdateServerConfigParams {
  host?: string | null
  port?: number | null
  enableCors?: boolean | null
}

// =============================================================================
// Update Config Commands
// Rust: src-tauri/src/ui/commands.rs
// =============================================================================

/** Params for update_update_config */
export interface UpdateUpdateConfigParams {
  mode: UpdateMode
  checkIntervalDays: number
}

/** Params for skip_update_version */
export interface SkipUpdateVersionParams {
  version?: string | null
}

// =============================================================================
// OAuth Commands
// Rust: src-tauri/src/ui/commands.rs
// =============================================================================

/** Params for start_oauth_flow */
export interface StartOAuthFlowParams {
  providerId: string
}

/** Params for poll_oauth_status */
export interface PollOAuthStatusParams {
  providerId: string
}

/** Params for cancel_oauth_flow */
export interface CancelOAuthFlowParams {
  providerId: string
}

/** Params for delete_oauth_credentials */
export interface DeleteOAuthCredentialsParams {
  providerId: string
}

// =============================================================================
// OAuth Client Commands
// Rust: src-tauri/src/ui/commands.rs
// =============================================================================

/** Params for create_oauth_client */
export interface CreateOAuthClientParams {
  name?: string | null
}

/** Params for get_oauth_client_secret */
export interface GetOAuthClientSecretParams {
  id: string
}

/** Params for delete_oauth_client */
export interface DeleteOAuthClientParams {
  id: string
}

/** Params for update_oauth_client_name */
export interface UpdateOAuthClientNameParams {
  id: string
  name: string
}

/** Params for toggle_oauth_client_enabled */
export interface ToggleOAuthClientEnabledParams {
  id: string
  enabled: boolean
}

/** Params for link_mcp_server */
export interface LinkMcpServerParams {
  clientId: string
  serverId: string
}

/** Params for unlink_mcp_server */
export interface UnlinkMcpServerParams {
  clientId: string
  serverId: string
}

/** Params for get_oauth_client_linked_servers */
export interface GetOAuthClientLinkedServersParams {
  clientId: string
}

// =============================================================================
// Firewall Commands
// Rust: src-tauri/src/ui/commands_clients.rs
// =============================================================================

/** Params for submit_firewall_approval */
export interface SubmitFirewallApprovalParams {
  requestId: string
  action: FirewallApprovalAction
  editedArguments?: string | null
}

/** Params for get_firewall_approval_details */
export interface GetFirewallApprovalDetailsParams {
  requestId: string
}

/** Params for get_firewall_full_arguments */
export interface GetFirewallFullArgumentsParams {
  requestId: string
}

/** Params for debug_trigger_firewall_popup */
export interface DebugTriggerFirewallPopupParams {
  popupType?: "mcp_tool" | "llm_model" | "skill" | "marketplace" | "free_tier_fallback" | null
  sendMultiple?: boolean | null
}

/** Params for debug_set_tray_overlay */
export interface DebugSetTrayOverlayParams {
  overlay?: "none" | "warning_yellow" | "warning_red" | "update_available" | "firewall_pending" | null
}

// =============================================================================
// Marketplace Commands
// Rust: src-tauri/src/ui/commands_marketplace.rs
// =============================================================================

/** Params for marketplace_set_enabled */
export interface MarketplaceSetEnabledParams {
  enabled: boolean
}

/** Params for marketplace_set_mcp_enabled */
export interface MarketplaceSetMcpEnabledParams {
  enabled: boolean
}

/** Params for marketplace_set_skills_enabled */
export interface MarketplaceSetSkillsEnabledParams {
  enabled: boolean
}

/** Params for marketplace_set_registry_url */
export interface MarketplaceSetRegistryUrlParams {
  url: string
}

/** Params for marketplace_add_skill_source */
export interface MarketplaceAddSkillSourceParams {
  source: MarketplaceSkillSource
}

/** Params for marketplace_remove_skill_source */
export interface MarketplaceRemoveSkillSourceParams {
  repoUrl: string
}

/** Params for marketplace_search_mcp_servers */
export interface MarketplaceSearchMcpServersParams {
  query: string
  limit?: number
}

/** Params for marketplace_search_skills */
export interface MarketplaceSearchSkillsParams {
  query?: string
  source?: string
}

/** Params for marketplace_install_mcp_server_direct */
export interface MarketplaceInstallMcpServerDirectParams {
  config: McpInstallConfig
}

/** Params for marketplace_install_skill_direct */
export interface MarketplaceInstallSkillDirectParams {
  sourceUrl: string
  skillName: string
}

/** Params for marketplace_delete_skill */
export interface MarketplaceDeleteSkillParams {
  skillName: string
  skillPath: string
}

/** Params for marketplace_is_skill_from_marketplace */
export interface MarketplaceIsSkillFromMarketplaceParams {
  skillPath: string
}

/** Params for marketplace_get_pending_install */
export interface MarketplaceGetPendingInstallParams {
  requestId: string
}

/** Params for marketplace_install_respond */
export interface MarketplaceInstallRespondParams {
  requestId: string
  action: string
  config?: Record<string, unknown> | null
}

/** Params for set_client_marketplace_enabled */
export interface SetClientMarketplaceEnabledParams {
  clientId: string
  enabled: boolean
}

/** Params for get_client_marketplace_enabled */
export interface GetClientMarketplaceEnabledParams {
  clientId: string
}

// =============================================================================
// Tray Graph Commands
// Rust: src-tauri/src/ui/commands.rs
// =============================================================================

/** Params for update_tray_graph_settings */
export interface UpdateTrayGraphSettingsParams {
  enabled: boolean
  refreshRateSecs: number
}

// =============================================================================
// Pricing Override Commands
// Rust: src-tauri/src/ui/commands.rs
// =============================================================================

/** Params for get_pricing_override */
export interface GetPricingOverrideParams {
  provider: string
  model: string
}

/** Params for set_pricing_override */
export interface SetPricingOverrideParams {
  provider: string
  model: string
  inputPerMillion: number
  outputPerMillion: number
}

/** Params for delete_pricing_override */
export interface DeletePricingOverrideParams {
  provider: string
  model: string
}

// =============================================================================
// Utility Commands
// Rust: src-tauri/src/ui/commands.rs
// =============================================================================

/** Params for open_path */
export interface OpenPathParams {
  path: string
}

/** Params for create_test_client_for_strategy */
export interface CreateTestClientForStrategyParams {
  strategyId: string
}

// =============================================================================
// GuardRails Types - LLM-based Safety Models
// Rust: crates/lr-guardrails/src/safety_model.rs, crates/lr-config/src/types.rs
// =============================================================================

/** Global guardrails configuration */
export interface GuardrailsConfig {
  scan_requests: boolean
  safety_models: SafetyModelConfig[]
  category_actions: CategoryActionEntry[]
  default_confidence_threshold: number
  parallel_guardrails: boolean
  moderation_api_enabled: boolean
}

/** Per-client guardrails configuration */
export interface ClientGuardrailsConfig {
  category_actions: CategoryActionEntry[] | null
}

/** Configuration for a safety model */
export interface SafetyModelConfig {
  id: string
  label: string
  model_type: string
  enabled: boolean
  provider_id: string | null
  model_name: string | null
  confidence_threshold: number | null
  enabled_categories: string[] | null
  prompt_template: string | null
  safe_indicator: string | null
  output_regex: string | null
  category_mapping: CategoryMappingEntry[] | null
}

/** Mapping from native model label to normalized safety category */
export interface CategoryMappingEntry {
  native_label: string
  safety_category: string
}

/** Rust: SafetyCategory enum - most variants serialize as snake_case strings,
 * but Custom(String) serializes as { custom: "value" } */
export type SafetyCategory = string | { custom: string }

/** Per-category action configuration (always uses string keys in config) */
export interface CategoryActionEntry {
  category: string
  action: "allow" | "notify" | "ask" | "block"
}

/** A flagged category from a safety model verdict */
export interface FlaggedCategory {
  category: SafetyCategory
  confidence: number | null
  native_label: string
}

/** Verdict from a single safety model check */
export interface SafetyVerdict {
  model_id: string
  model_label?: string
  is_safe: boolean
  flagged_categories: FlaggedCategory[]
  confidence: number | null
  raw_output: string
  check_duration_ms: number
}

/** Action required for a specific category */
export interface CategoryActionRequired {
  category: SafetyCategory
  action: "allow" | "notify" | "ask" | "block"
  model_id: string
  confidence: number | null
}

/** Result from running safety checks across all enabled models */
export interface SafetyCheckResult {
  verdicts: SafetyVerdict[]
  actions_required: CategoryActionRequired[]
  total_duration_ms: number
  scan_direction: "request" | "response"
  errors?: SafetyModelError[]
}

/** Rust: crates/lr-guardrails/src/safety_model.rs - SafetyModelError struct */
export interface SafetyModelError {
  model_id: string
  error: string
}

/** Guardrail approval details sent to popup */
export interface GuardrailApprovalDetails {
  verdicts: SafetyVerdict[]
  actions_required: CategoryActionRequired[]
  total_duration_ms: number
  scan_direction: "request" | "response"
  flagged_text: string
}

/** Safety category info returned by get_all_safety_categories */
export interface SafetyCategoryInfo {
  category: string
  display_name: string
  description: string
  supported_by: string[]
}

/** Params for update_guardrails_config */
export interface UpdateGuardrailsConfigParams {
  configJson: string
}

/** Params for test_safety_check */
export interface TestSafetyCheckParams {
  text: string
  clientId?: string | null
}

/** Params for get_safety_model_status */
export interface GetSafetyModelStatusParams {
  modelId: string
}

/** Params for test_safety_model */
export interface TestSafetyModelParams {
  modelId: string
  text: string
}

/** Params for add_safety_model */
export interface AddSafetyModelParams {
  configJson: string
}

/** Params for remove_safety_model */
export interface RemoveSafetyModelParams {
  modelId: string
}

/** Params for toggle_safety_model */
export interface ToggleSafetyModelParams {
  modelId: string
  enabled: boolean
}

/** Params for pull_provider_model */
export interface PullProviderModelParams {
  providerId: string
  modelName: string
}

// =============================================================================
// Free Tier Commands
// Rust: src-tauri/src/ui/commands_free_tier.rs
// =============================================================================

/** Params for set_provider_free_tier */
export interface SetProviderFreeTierParams {
  providerInstance: string
  freeTier: FreeTierKind | null
}

/** Params for reset_provider_free_tier_usage */
export interface ResetProviderFreeTierUsageParams {
  providerInstance: string
}

/** Params for set_provider_free_tier_usage */
export interface SetProviderFreeTierUsageParams {
  providerInstance: string
  creditUsedUsd: number | null
  creditRemainingUsd: number | null
  dailyRequests: number | null
  monthlyRequests: number | null
  monthlyTokens: number | null
}

/** Params for get_default_free_tier */
export interface GetDefaultFreeTierParams {
  providerType: string
}

/** Progress event from provider model pull */
export interface ProviderModelPullProgress {
  status: string
  total: number | null
  completed: number | null
}

// =============================================================================
// Coding Agents Types
// Rust: src-tauri/src/ui/commands_coding_agents.rs
// =============================================================================

/**
 * Coding agent type enum.
 * Rust: crates/lr-config/src/types.rs - CodingAgentType enum
 */
export type CodingAgentType =
  | 'claude_code'
  | 'gemini_cli'
  | 'codex'
  | 'amp'
  | 'opencode'
  | 'cursor'
  | 'qwen_code'
  | 'copilot'
  | 'droid'
  | 'aider'

/**
 * Coding agent permission mode.
 * Rust: crates/lr-config/src/types.rs - CodingPermissionMode enum
 */
export type CodingPermissionMode = 'auto' | 'supervised' | 'plan'

/**
 * Coding agent info returned from list_coding_agents.
 * Rust: src-tauri/src/ui/commands_coding_agents.rs - CodingAgentInfo struct
 */
export interface CodingAgentInfo {
  agentType: CodingAgentType
  displayName: string
  binaryName: string
  installed: boolean
  binaryPath: string | null
  description: string
  supportsModelSelection: boolean
  supportedPermissionModes: CodingPermissionMode[]
  mcpToolPrefix: string
}

/**
 * Coding session info returned from list_coding_sessions.
 * Rust: src-tauri/src/ui/commands_coding_agents.rs - CodingSessionInfo struct
 */
export interface CodingSessionInfo {
  sessionId: string
  agentType: CodingAgentType
  clientId: string
  workingDirectory: string
  displayText: string
  status: string
  createdAt: string
}

// =============================================================================
// Coding Agents Command Parameters
// Rust: src-tauri/src/ui/commands_coding_agents.rs
// =============================================================================

/** Params for get_coding_agent_version */
export interface GetCodingAgentVersionParams {
  agentType: CodingAgentType
}

/**
 * Tool definition returned from get_coding_agent_tool_definitions.
 * Rust: src-tauri/src/ui/commands_coding_agents.rs - ToolDefinition struct
 */
export interface ToolDefinition {
  name: string
  description: string | null
  input_schema: Record<string, unknown>
}

/** Params for get_coding_agent_tool_definitions */
export interface GetCodingAgentToolDefinitionsParams {
  agentType: CodingAgentType
}

/**
 * Detailed coding session info returned from get_coding_session_detail.
 * Rust: crates/lr-coding-agents/src/types.rs - SessionDetail struct
 */
export interface CodingSessionDetail {
  sessionId: string
  agentType: CodingAgentType
  clientId: string
  workingDirectory: string
  displayText: string
  status: string
  createdAt: string
  recentOutput: string[]
  costUsd: number | null
  turnCount: number | null
  result: string | null
  error: string | null
  exitCode: number | null
}

/** Params for get_coding_session_detail */
export interface GetCodingSessionDetailParams {
  sessionId: string
}

/** Params for end_coding_session */
export interface EndCodingSessionParams {
  sessionId: string
}

/** Params for set_client_coding_agent_permission */
export interface SetClientCodingAgentPermissionParams {
  clientId: string
  permission: PermissionState
}

/** Params for set_client_coding_agent_type */
export interface SetClientCodingAgentTypeParams {
  clientId: string
  agentType: CodingAgentType | null
}

/** Params for set_max_coding_sessions */
export interface SetMaxCodingSessionsParams {
  maxSessions: number
}

/** Params for set_coding_agent_tool_prefix */
export interface SetCodingAgentToolPrefixParams {
  prefix: string
}

/** Params for update_marketplace_tool_names */
export interface UpdateMarketplaceToolNamesParams {
  searchToolName?: string | null
  installToolName?: string | null
}

/** Params for update_skills_tool_names */
export interface UpdateSkillsToolNamesParams {
  toolName?: string | null
  readFileToolName?: string | null
}

/** Params for set_periodic_health_enabled */
export interface SetPeriodicHealthEnabledParams {
  enabled: boolean
}

// =============================================================================
// Sidebar Command Parameters
// Rust: src-tauri/src/ui/commands.rs
// =============================================================================

/** Params for set_sidebar_expanded */
export interface SetSidebarExpandedParams {
  expanded: boolean
}

// =============================================================================
// Prompt Compression Types
// Rust: crates/lr-config/src/types.rs, crates/lr-compression/src/types.rs
// =============================================================================

/** Global prompt compression configuration */
export interface PromptCompressionConfig {
  enabled: boolean
  model_size: string
  default_rate: number
  compress_system_prompt: boolean
  min_messages: number
  preserve_recent: number
  min_message_words: number
  preserve_quoted_text: boolean
  compression_notice: boolean
}

/** Per-client prompt compression configuration */
export interface ClientPromptCompressionConfig {
  enabled: boolean | null
}

/** Compression model status */
export interface CompressionStatus {
  model_downloaded: boolean
  model_loaded: boolean
  model_size_bytes: number | null
  model_repo: string
}

/** Embedding model status (semantic vector search) */
export interface EmbeddingStatus {
  downloaded: boolean
  loaded: boolean
  model_name: string
  model_size_mb: number | null
}

/** Compression test result (from compress_prompt tool) */
export interface CompressionTestResult {
  compressed_text: string
  original_tokens: number
  compressed_tokens: number
  ratio: number
  kept_indices: number[]
  protected_indices: number[]
}

/** Params for test_compression */
export interface TestCompressionParams {
  text: string
  rate: number
  preserveQuoted: boolean
  compressionNotice: boolean
}

/** Params for update_compression_config */
export interface UpdateCompressionConfigParams {
  configJson: string
}

/** Params for get_client_compression_config */
export interface GetClientCompressionConfigParams {
  clientId: string
}

/** Params for update_client_compression_config */
export interface UpdateClientCompressionConfigParams {
  clientId: string
  configJson: string
}

// ============================================================================
// JSON Repair
// ============================================================================

/** Rust: src-tauri/src/ui/commands.rs - JsonRepairConfig (via lr_config) */
export interface JsonRepairConfig {
  enabled: boolean
  syntax_repair: boolean
  schema_coercion: boolean
  strip_extra_fields: boolean
  add_defaults: boolean
  normalize_enums: boolean
}

/** Per-client JSON repair configuration */
export interface ClientJsonRepairConfig {
  enabled: boolean | null
  syntax_repair: boolean | null
  schema_coercion: boolean | null
}

/** Params for get_client_json_repair_config */
export interface GetClientJsonRepairConfigParams {
  clientId: string
}

/** Params for update_client_json_repair_config */
export interface UpdateClientJsonRepairConfigParams {
  clientId: string
  configJson: string
}

/** JSON repair test result */
export interface JsonRepairTestResult {
  original: string
  repaired: string
  was_modified: boolean
  repairs: JsonRepairAction[]
}

/** A single repair action that was performed */
export type JsonRepairAction =
  | 'stripped_markdown_fences'
  | 'stripped_prose'
  | 'syntax_repaired'
  | { type_coerced: { path: string; from: string; to: string } }
  | { extra_field_removed: { path: string } }
  | { default_added: { path: string } }
  | { enum_normalized: { path: string; from: string; to: string } }

/** Params for update_json_repair_config */
export interface UpdateJsonRepairConfigParams {
  configJson: string
}

/** Params for test_json_repair */
export interface TestJsonRepairParams {
  content: string
  schema: string | null
}

// =============================================================================
// Local Provider Discovery Types
// Rust: crates/lr-providers/src/factory.rs, src-tauri/src/ui/commands.rs
// =============================================================================

/** Rust: crates/lr-providers/src/factory.rs - DiscoveredProvider struct */
export interface DiscoveredProvider {
  provider_type: string
  instance_name: string
  base_url: string
}

/** Rust: src-tauri/src/ui/commands.rs - DiscoverProviderResult struct */
export interface DiscoverProviderResult {
  discovered: DiscoveredProvider[]
  added: string[]
  skipped: string[]
}

// =============================================================================
// Secret Scanning Types
// Rust: crates/lr-config/src/types.rs, crates/lr-mcp/src/gateway/firewall.rs
// =============================================================================

/** Secret scan action: what to do when a secret is detected */
export type SecretScanAction = 'ask' | 'notify' | 'off'

/** Rust: crates/lr-config/src/types.rs - SecretScanningConfig */
export interface SecretScanningConfig {
  action: SecretScanAction
  entropy_threshold: number
  scan_system_messages: boolean
  allowlist: string[]
}

/** Per-client secret scanning configuration */
export interface ClientSecretScanningConfig {
  action: SecretScanAction | null
}

/** Summary of a single secret finding (for approval popup) */
export interface SecretFindingSummary {
  rule_id: string
  rule_description: string
  category: string
  matched_text: string
  entropy: number
}

/** Secret scan approval details (sent to popup) */
export interface SecretScanApprovalDetails {
  findings: SecretFindingSummary[]
  scan_duration_ms: number
}

/** Result of a secret scan */
export interface SecretScanResult {
  findings: SecretFinding[]
  scan_duration_ms: number
  rules_evaluated: number
}

/** A single secret finding */
export interface SecretFinding {
  rule_id: string
  rule_description: string
  category: string
  regex_pattern: string
  keywords: string[]
  rule_entropy_threshold: number | null
  message_index: number
  matched_text: string
  entropy: number
}

/** Metadata about a compiled secret scanning rule */
export interface SecretRuleMetadata {
  id: string
  description: string
  regex: string
  category: string
  entropy_threshold: number | null
  keywords: string[]
}

/** Params for get_secret_scanning_config - no params needed */

/** Params for update_secret_scanning_config */
export interface UpdateSecretScanningConfigParams {
  configJson: string
}

/** Params for get_client_secret_scanning_config */
export interface GetClientSecretScanningConfigParams {
  clientId: string
}

/** Params for update_client_secret_scanning_config */
export interface UpdateClientSecretScanningConfigParams {
  clientId: string
  configJson: string
}

/** Params for test_secret_scan */
export interface TestSecretScanParams {
  input: string
}

// =============================================================================
// Feature Support Matrix Types
// Rust: crates/lr-providers/src/lib.rs
// =============================================================================

/** Level of support for a feature or endpoint */
export type SupportLevel = 'supported' | 'partial' | 'translated' | 'not_supported' | 'not_implemented'

/** Support information for a single API endpoint */
export interface EndpointSupport {
  name: string
  endpoint: string
  support: SupportLevel
  notes: string | null
}

/** Support information for a single feature */
export interface FeatureSupport {
  name: string
  support: SupportLevel
  notes: string | null
}

/** Complete feature support information for a provider */
export interface ProviderFeatureSupport {
  provider_type: string
  provider_instance: string
  endpoints: EndpointSupport[]
  model_features: FeatureSupport[]
  optimization_features: FeatureSupport[]
}

/** A cell in the feature-endpoint matrix */
export interface MatrixCell {
  support: SupportLevel
  notes: string | null
}

/** A row in the feature × endpoint matrix */
export interface FeatureEndpointRow {
  feature_name: string
  cells: MatrixCell[]
}

/** A row in the feature/endpoint × client mode matrix */
export interface FeatureModeRow {
  name: string
  cells: MatrixCell[]
}

/** Static matrix of optimization features × endpoints × client modes */
export interface FeatureEndpointMatrix {
  endpoints: string[]
  client_modes: string[]
  feature_rows: FeatureEndpointRow[]
  mode_rows: FeatureModeRow[]
}

/** Params for get_provider_feature_support */
export interface GetProviderFeatureSupportParams {
  instanceName: string
}

/** Params for get_all_provider_feature_support - no params needed */

/** Params for get_feature_endpoint_matrix - no params needed */

// ============================================================================
// Memory Configuration Types
// ============================================================================

/** Rust: crates/lr-config/src/types.rs - MemoryConfig struct */
export interface MemoryConfig {
  /** Whether LLM-based compaction is enabled (default: false) */
  compaction_enabled: boolean
  /** Compaction LLM model routed through LocalRouter (e.g., "anthropic/claude-haiku-4-5-20251001") */
  compaction_model: string | null
  search_top_k: number
  session_inactivity_minutes: number
  max_session_minutes: number
  /** Tool name for search (default: "MemorySearch"). Read tool is derived as "MemoryRead". */
  recall_tool_name: string
}

/** Params for update_memory_config */
export interface UpdateMemoryConfigParams {
  configJson: string
}

/** Rust: src-tauri/src/ui/commands.rs - MemoryClientInfo */
export interface MemoryClientInfo {
  client_id: string
  client_name: string
  source_count: number
  total_lines: number
}

/** Rust: src-tauri/src/ui/commands.rs - CompactionStatsResult */
export interface CompactionStatsResult {
  active_sessions: number
  pending_compaction: number
  archived_sessions: number
  summarized_sessions: number
  indexed_sources: number
  total_lines: number
}

/** Params for get_memory_compaction_stats */
export interface GetMemoryCompactionStatsParams {
  clientId: string
}

/** Params for force_compact_memory */
export interface ForceCompactMemoryParams {
  clientId: string
}

/** Params for recompact_memory */
export interface RecompactMemoryParams {
  clientId: string
}

/** Params for reindex_client_memory */
export interface ReindexClientMemoryParams {
  clientId: string
}

/** Event payload for memory-compact-progress */
export interface MemoryCompactProgress {
  client_id: string
  current: number
  total: number
}

/** Event payload for memory-compact-complete */
export interface MemoryCompactComplete {
  client_id: string
  archived_count: number
  summarized_count: number
}

/** Event payload for memory-recompact-progress */
export interface MemoryRecompactProgress {
  client_id: string
  current: number
  total: number
}

/** Event payload for memory-recompact-complete */
export interface MemoryRecompactComplete {
  client_id: string
  recompacted_count: number
}

/** Event payload for memory-reindex-progress */
export interface MemoryReindexProgress {
  client_id: string
  current: number
  total: number
}

/** Event payload for memory-reindex-complete */
export interface MemoryReindexComplete {
  client_id: string
  indexed_count: number
}

// =============================================================================
// Monitor Types
// Rust: crates/lr-monitor/src/types.rs
// =============================================================================

export type MonitorEventType =
  | 'llm_call'
  | 'mcp_tool_call' | 'mcp_resource_read' | 'mcp_prompt_get'
  | 'mcp_elicitation' | 'mcp_sampling'
  | 'guardrail_scan' | 'guardrail_response_scan' | 'secret_scan'
  | 'route_llm_classify'
  | 'routing_decision' | 'auth_error' | 'access_denied'
  | 'rate_limit_event' | 'validation_error' | 'mcp_server_event'
  | 'oauth_event' | 'internal_error' | 'moderation_event'
  | 'connection_error' | 'prompt_compression' | 'memory_compaction'
  | 'firewall_decision' | 'sse_connection'

export type EventStatus = 'pending' | 'complete' | 'error'

/** Rust: crates/lr-monitor/src/types.rs - MonitorEventSummary */
export interface MonitorEventSummary {
  id: string
  sequence: number
  timestamp: string
  event_type: MonitorEventType
  session_id: string | null
  client_id: string | null
  client_name: string | null
  status: EventStatus
  duration_ms: number | null
  summary: string
}

/** Rust: crates/lr-monitor/src/types.rs - MonitorEvent */
export interface MonitorEvent {
  id: string
  sequence: number
  timestamp: string
  event_type: MonitorEventType
  session_id: string | null
  client_id: string | null
  client_name: string | null
  data: Record<string, unknown>
  status: EventStatus
  duration_ms: number | null
}

/** Rust: crates/lr-monitor/src/types.rs - MonitorEventListResponse */
export interface MonitorEventListResponse {
  events: MonitorEventSummary[]
  total: number
}

/** Rust: crates/lr-monitor/src/types.rs - MonitorStats */
export interface MonitorStats {
  total_events: number
  max_capacity: number
  events_by_type: Record<string, number>
}

/** Rust: crates/lr-monitor/src/types.rs - MonitorEventFilter */
export interface MonitorEventFilter {
  event_types: MonitorEventType[] | null
  session_id: string | null
  client_id: string | null
  status: EventStatus | null
  search: string | null
}

// Monitor Command Parameters

/** Params for get_monitor_events */
export interface GetMonitorEventsParams {
  offset: number
  limit: number
  filter: MonitorEventFilter | null
}

/** Params for get_monitor_event_detail */
export interface GetMonitorEventDetailParams {
  eventId: string
}

/** Params for set_monitor_max_capacity */
export interface SetMonitorMaxCapacityParams {
  capacity: number
}

/** Category of requests to intercept from the monitor page */
/** Rust: crates/lr-mcp/src/gateway/firewall.rs - InterceptCategory enum */
export type InterceptCategory =
  | 'llm'
  | 'mcp'
  | 'skill'
  | 'marketplace'
  | 'coding_agent'
  | 'guardrails'
  | 'secret_scan'
  | 'sampling'
  | 'elicitation'

/** Active intercept rule for the monitor page (transient, not persisted) */
/** Rust: crates/lr-mcp/src/gateway/firewall.rs - InterceptRule struct */
export interface InterceptRule {
  categories: InterceptCategory[]
  client_ids: string[]
}

/** Params for set_monitor_intercept_rule */
export interface SetMonitorInterceptRuleParams {
  rule: InterceptRule | null
}
