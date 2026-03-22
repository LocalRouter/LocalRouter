import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Wrench, ExternalLink } from "lucide-react"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { FEATURES } from "@/constants/features"
import { ExperimentalBadge } from "@/components/shared/ExperimentalBadge"
import { ModelDownloadCard } from "@/components/shared/ModelDownloadCard"
import { useModelDownload } from "@/hooks/useModelDownload"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import { InfoTooltip } from "@/components/ui/info-tooltip"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { cn } from "@/lib/utils"
import { listenSafe } from "@/hooks/useTauriListener"
import { McpToolDisplay } from "@/components/shared/McpToolDisplay"
import type { McpToolDisplayItem } from "@/components/shared/McpToolDisplay"
import { ContentStorePreview } from "@/components/shared/ContentStorePreview"
import { VirtualMcpIndexingTree } from "@/components/permissions/VirtualMcpIndexingTree"
import { GatewayIndexingTree } from "@/components/permissions/GatewayIndexingTree"
import { IndexingStateButton } from "@/components/permissions/IndexingStateButton"
import type {
  ContextManagementConfig,
  IndexingState,
  ToolDefinition,
  ClientFeatureStatus,
  GetFeatureClientsStatusParams,
  EmbeddingStatus,
} from "@/types/tauri-commands"

// Must match defaults in crates/lr-config/src/types.rs
const DEFAULT_RESPONSE_THRESHOLD_BYTES = 200

const SAMPLE_DOCUMENT = `# API Reference - Authentication Service

## Overview

The Authentication Service provides OAuth 2.0 and API key based authentication for all microservices.
It handles user sessions, token refresh, and permission management.

## Endpoints

### POST /auth/login

Authenticates a user and returns an access token.

**Request body:**
\`\`\`json
{
  "email": "user@example.com",
  "password": "secure_password",
  "mfa_code": "123456"
}
\`\`\`

**Response:**
\`\`\`json
{
  "access_token": "eyJhbG...",
  "refresh_token": "dGhpcyB...",
  "expires_in": 3600,
  "token_type": "Bearer"
}
\`\`\`

### POST /auth/refresh

Refreshes an expired access token using a valid refresh token.

**Headers:** \`Authorization: Bearer <refresh_token>\`

### GET /auth/me

Returns the current user's profile and permissions.

**Response:**
\`\`\`json
{
  "id": "usr_abc123",
  "email": "user@example.com",
  "roles": ["admin", "developer"],
  "permissions": ["read:all", "write:projects"]
}
\`\`\`

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| AUTH_JWT_SECRET | (required) | Secret key for JWT signing |
| AUTH_TOKEN_TTL | 3600 | Access token TTL in seconds |
| AUTH_REFRESH_TTL | 604800 | Refresh token TTL in seconds |
| AUTH_MFA_ENABLED | true | Whether MFA is required |
| AUTH_RATE_LIMIT | 100 | Max login attempts per minute |

### Rate Limiting

The service implements token-bucket rate limiting:
- Login endpoint: 5 attempts per minute per IP
- Token refresh: 30 requests per minute per user
- Profile endpoint: 60 requests per minute per user

Exceeding limits returns HTTP 429 with a Retry-After header.

## Error Codes

| Code | HTTP Status | Description |
|------|------------|-------------|
| AUTH_001 | 401 | Invalid credentials |
| AUTH_002 | 401 | Token expired |
| AUTH_003 | 403 | Insufficient permissions |
| AUTH_004 | 429 | Rate limit exceeded |
| AUTH_005 | 400 | Invalid MFA code |
| AUTH_006 | 503 | Auth service unavailable |

## SDK Usage

\`\`\`typescript
import { AuthClient } from '@company/auth-sdk'

const auth = new AuthClient({
  baseUrl: 'https://auth.example.com',
  clientId: 'app_xyz'
})

// Login
const session = await auth.login({
  email: 'user@example.com',
  password: 'secure_password'
})

// Make authenticated requests
const profile = await auth.getProfile()
console.log(profile.permissions)

// Refresh token
await auth.refreshToken()
\`\`\`
`


interface ResponseRagViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function ResponseRagView({ activeSubTab, onTabChange }: ResponseRagViewProps) {
  const [config, setConfig] = useState<ContextManagementConfig | null>(null)
  const [, setSaving] = useState(false)
  const [contextTools, setContextTools] = useState<McpToolDisplayItem[]>([])
  const [clientStatuses, setClientStatuses] = useState<ClientFeatureStatus[]>([])
  const [embeddingStatus, setEmbeddingStatus] = useState<EmbeddingStatus | null>(null)

  const tab = activeSubTab || "info"

  const handleTabChange = (newTab: string) => {
    onTabChange?.("response-rag", newTab)
  }

  const loadEmbeddingStatus = async () => {
    try {
      const status = await invoke<EmbeddingStatus>("get_embedding_status")
      setEmbeddingStatus(status)
    } catch (err) {
      console.error("Failed to load embedding status:", err)
    }
  }

  const embeddingDownload = useModelDownload({
    isDownloaded: embeddingStatus?.downloaded ?? false,
    downloadCommand: "install_embedding_model",
    progressEvent: "embedding-download-progress",
    completeEvent: "embedding-download-complete",
    onComplete: () => {
      toast.success("Embedding model downloaded and loaded")
      loadEmbeddingStatus()
    },
    onFailed: (err: string) => toast.error(`Download failed: ${err}`),
  })

  const loadClientStatuses = useCallback(async () => {
    try {
      const data = await invoke<ClientFeatureStatus[]>("get_feature_clients_status", {
        feature: "context_management",
      } satisfies GetFeatureClientsStatusParams)
      setClientStatuses(data)
    } catch (err) {
      console.error("Failed to load client statuses:", err)
    }
  }, [])

  useEffect(() => {
    let ignore = false

    invoke<ContextManagementConfig>("get_context_management_config")
      .then((cfg) => { if (!ignore) setConfig(cfg) })
      .catch((err) => console.error("Failed to load context management config:", err))

    invoke<ToolDefinition[]>("get_context_mode_tool_definitions")
      .then((defs) => {
        if (!ignore) setContextTools(defs.map((d): McpToolDisplayItem => ({
          name: d.name,
          description: d.description,
          inputSchema: d.input_schema,
        })))
      })
      .catch(() => {})

    loadClientStatuses()
    loadEmbeddingStatus()

    return () => {
      ignore = true
    }
  }, [loadClientStatuses])

  useEffect(() => {
    const listeners = [
      listenSafe("clients-changed", loadClientStatuses),
      listenSafe("config-changed", loadClientStatuses),
    ]
    return () => { listeners.forEach(l => l.cleanup()) }
  }, [loadClientStatuses])

  const updateField = async (field: string, value: unknown) => {
    try {
      setSaving(true)
      await invoke("update_context_management_config", { [field]: value })
      const updated = await invoke<ContextManagementConfig>("get_context_management_config")
      setConfig(updated)
    } catch (error) {
      toast.error(`Failed to update: ${error}`)
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="flex flex-col h-full min-h-0 max-w-5xl">
      <div className="flex-shrink-0 pb-4">
        <div className="flex items-center gap-2">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <FEATURES.responseRag.icon className={`h-6 w-6 ${FEATURES.responseRag.color}`} />
            {FEATURES.responseRag.name}
          </h1>
          <ExperimentalBadge />
        </div>
        <p className="text-sm text-muted-foreground">
          Index and compress tool responses using FTS5 search indexing
        </p>
      </div>

      <Tabs
        value={tab}
        onValueChange={handleTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="info">
            <TAB_ICONS.info className={TAB_ICON_CLASS} />
            Info
          </TabsTrigger>
          <TabsTrigger value="preview">
            <TAB_ICONS.tryItOut className={TAB_ICON_CLASS} />
            Try It Out
          </TabsTrigger>
          <TabsTrigger value="settings">
            <TAB_ICONS.settings className={TAB_ICON_CLASS} />
            Settings
          </TabsTrigger>
        </TabsList>

        {/* Info Tab */}
        <TabsContent value="info" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-2xl">
            {/* Overview */}
            <Card>
              <CardHeader>
                <CardTitle className="text-base">How it works</CardTitle>
              </CardHeader>
              <CardContent className="space-y-3 text-sm text-muted-foreground">
                <p>
                  MCP Response RAG intercepts tool call responses that exceed a configurable size threshold
                  and replaces them with a compressed preview. The full content is indexed into an FTS5
                  full-text search database so the LLM can retrieve specific sections on demand.
                </p>
                <p>
                  This reduces context window consumption while preserving the LLM&apos;s ability to access
                  all information. Two tools are exposed to the LLM:
                </p>
                <ul className="list-disc list-inside space-y-1 ml-1">
                  <li><code className="text-xs bg-muted px-1 py-0.5 rounded">IndexSearch</code> — full-text search across all indexed content</li>
                  <li><code className="text-xs bg-muted px-1 py-0.5 rounded">IndexRead</code> — read the full content of an indexed source with pagination</li>
                </ul>
                <p>
                  The compressed preview includes a table of contents with line references and a search
                  hint so the LLM knows it can retrieve more detail. Tool indexing below controls which
                  tool responses are eligible for compression.
                </p>
              </CardContent>
            </Card>

            {/* Embedding Model Download */}
            <ModelDownloadCard
              title="Semantic Search (Optional)"
              description="Download a small local embedding model (~80MB) to enable hybrid FTS5 + vector search. Keyword search works without it."
              modelName={embeddingStatus?.model_name}
              modelInfo={embeddingStatus?.model_size_mb != null ? `${embeddingStatus.model_size_mb.toFixed(0)} MB` : undefined}
              status={embeddingDownload.status}
              progress={embeddingDownload.progress}
              error={embeddingDownload.error}
              onDownload={embeddingDownload.startDownload}
              onRetry={embeddingDownload.retry}
              downloadLabel="Download all-MiniLM-L6-v2 (~80MB)"
            >
              <p className="text-xs text-muted-foreground">
                Enables semantic search: "SQL database for login" finds "We chose PostgreSQL for authentication."
                Runs locally via Metal/CUDA/CPU — no external API calls.
              </p>
            </ModelDownloadCard>

            {/* MCP Tools exposed to the LLM */}
            {contextTools.length > 0 && (
              <Card>
                <CardHeader className="pb-3">
                  <CardTitle className="text-base flex items-center gap-2">
                    <Wrench className="h-4 w-4" />
                    Tool Definitions
                  </CardTitle>
                  <CardDescription>
                    These tools are exposed to the LLM for searching and reading indexed content.
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <McpToolDisplay tools={contextTools} />
                </CardContent>
              </Card>
            )}

            {/* Tool Indexing */}
            {config && (
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">Tool Indexing</CardTitle>
                  <CardDescription>
                    Control which tool responses get indexed into FTS5 for search.
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  {/* Built-in MCPs */}
                  <div>
                    <p className="text-xs text-muted-foreground mb-1.5">
                      Built-in tools like skills, marketplace, and coding agents.
                    </p>
                    <VirtualMcpIndexingTree
                      permissions={config.virtual_indexing}
                      onUpdate={async () => {
                        const updated = await invoke<ContextManagementConfig>("get_context_management_config")
                        setConfig(updated)
                      }}
                    />
                  </div>

                  {/* MCPs */}
                  <div>
                    <p className="text-xs text-muted-foreground mb-1.5">
                      External MCP servers connected via the gateway.
                    </p>
                    <GatewayIndexingTree
                      permissions={config.gateway_indexing}
                      onUpdate={async () => {
                        const updated = await invoke<ContextManagementConfig>("get_context_management_config")
                        setConfig(updated)
                      }}
                    />
                  </div>

                  {/* Client MCPs */}
                  <div>
                    <p className="text-xs text-muted-foreground mb-1.5">
                      Tools provided directly by connected clients.
                    </p>
                    <div className="border rounded-lg overflow-hidden">
                      <div className="flex items-center gap-2 px-3 py-3 bg-background">
                        <span className="font-semibold text-sm flex-1 min-w-0 truncate">Client Tools</span>
                        <div className="shrink-0">
                          <IndexingStateButton
                            value={config.client_tools_indexing_default}
                            onChange={(state: IndexingState) => updateField("clientToolsIndexingDefault", state)}
                            size="sm"
                          />
                        </div>
                      </div>
                      <div className="flex items-center gap-2 py-2 border-t border-border/50 hover:bg-muted/30 transition-colors text-sm" style={{ paddingLeft: "28px", paddingRight: "12px" }}>
                        <div className="w-5" />
                        <span className="text-xs text-muted-foreground">
                          Per-client overrides are configured in each client&apos;s settings.
                        </span>
                      </div>
                    </div>
                  </div>
                </CardContent>
              </Card>
            )}

            <Card>
              <CardHeader>
                <CardTitle className="text-base">MCP Response RAG (Per-Client)</CardTitle>
                <CardDescription>
                  Response RAG is controlled per-client in each client&apos;s Optimize tab.
                </CardDescription>
              </CardHeader>
              {clientStatuses.length > 0 && (
                <CardContent className="pt-0">
                  <div className="border-t pt-3 space-y-1.5">
                    {clientStatuses.map((s) => (
                      <div
                        key={s.client_id}
                        className="flex items-center justify-between py-1 px-2 rounded-md hover:bg-muted/50 group"
                      >
                        <div className="flex items-center gap-2 min-w-0">
                          {onTabChange ? (
                            <button
                              onClick={() => onTabChange("clients", `${s.client_id}|optimize`)}
                              className="text-sm font-medium truncate hover:underline text-left"
                            >
                              {s.client_name}
                            </button>
                          ) : (
                            <span className="text-sm font-medium truncate">{s.client_name}</span>
                          )}
                          {s.source === "override" && (
                            <Badge variant="outline" className="text-[10px] px-1 py-0 shrink-0">
                              Override
                            </Badge>
                          )}
                        </div>
                        <div className="flex items-center gap-2 shrink-0">
                          <Badge
                            variant="outline"
                            className={cn(
                              "text-[10px] px-1.5 py-0",
                              s.active
                                ? "border-emerald-500/50 text-emerald-600"
                                : "border-red-500/50 text-red-600",
                            )}
                          >
                            {s.active ? "Enabled" : "Disabled"}
                          </Badge>
                          {onTabChange && (
                            <button
                              onClick={() => onTabChange("clients", `${s.client_id}|optimize`)}
                              className="opacity-0 group-hover:opacity-100 text-muted-foreground hover:text-foreground transition-opacity"
                              title="Go to client settings"
                            >
                              <ExternalLink className="h-3.5 w-3.5" />
                            </button>
                          )}
                        </div>
                      </div>
                    ))}
                  </div>
                </CardContent>
              )}
            </Card>
          </div>
        </TabsContent>

        {/* Settings Tab */}
        <TabsContent value="settings" className="flex-1 min-h-0 mt-4">
          {config && (
            <div className="space-y-4 max-w-2xl">
              {/* Response Threshold */}
              <Card>
                <CardHeader>
                  <CardTitle className="text-base flex items-center gap-1">
                    Response Compression Threshold
                    <InfoTooltip content="Minimum response size in bytes before compression is applied. Responses smaller than this are passed through unchanged." />
                  </CardTitle>
                  <CardDescription>
                    Tool responses larger than this threshold are indexed and replaced with a
                    truncated preview and a search hint.
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <div className="flex gap-2 items-center">
                    <Input
                      type="number"
                      defaultValue={config.response_threshold_bytes}
                      key={`response-${config.response_threshold_bytes}`}
                      onBlur={(e) => {
                        const v = parseInt(e.target.value)
                        if (!isNaN(v) && v >= 0 && v !== config.response_threshold_bytes) {
                          updateField("responseThresholdBytes", v)
                        }
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          (e.target as HTMLInputElement).blur()
                        }
                      }}
                      className="w-40"
                      min={0}
                    />
                    <span className="text-sm text-muted-foreground">bytes</span>
                    {config.response_threshold_bytes !== DEFAULT_RESPONSE_THRESHOLD_BYTES && (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => updateField("responseThresholdBytes", DEFAULT_RESPONSE_THRESHOLD_BYTES)}
                      >
                        Reset to default
                      </Button>
                    )}
                  </div>
                  <p className="text-xs text-muted-foreground mt-2">
                    Default: {DEFAULT_RESPONSE_THRESHOLD_BYTES.toLocaleString()} bytes. Set higher to compress fewer responses. Set to 0 to always compress.
                  </p>
                </CardContent>
              </Card>
            </div>
          )}
        </TabsContent>

        {/* Preview / Try it out Tab */}
        <TabsContent value="preview" className="flex-1 min-h-0 mt-4 overflow-y-auto">
          <div className="max-w-2xl">
            <ContentStorePreview
              loadSample={() => Promise.resolve(SAMPLE_DOCUMENT)}
              sourceLabel="tool-response:1"
              responseThresholdBytes={config?.response_threshold_bytes ?? DEFAULT_RESPONSE_THRESHOLD_BYTES}
              searchPlaceholder='e.g. "rate limiting", "login endpoint"'
              defaultMode="index"
              showModeToggle={false}
              alwaysShowCompressed
            />
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}
