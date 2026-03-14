import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Database, Info, Settings, Search, Loader2, BookOpen, PlayCircle, Wrench } from "lucide-react"
import { OPTIMIZE_COLORS } from "@/views/optimize-overview/constants"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { McpToolDisplay } from "@/components/shared/McpToolDisplay"
import type { McpToolDisplayItem } from "@/components/shared/McpToolDisplay"
import { VirtualMcpIndexingTree } from "@/components/permissions/VirtualMcpIndexingTree"
import { GatewayIndexingTree } from "@/components/permissions/GatewayIndexingTree"
import { IndexingStateButton } from "@/components/permissions/IndexingStateButton"
import type {
  ContextManagementConfig,
  IndexingState,
  RagPreviewIndexResult,
  RagSearchResult,
  RagReadResult,
  PreviewRagIndexParams,
  PreviewRagSearchParams,
  PreviewRagReadParams,
  ToolDefinition,
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

  const tab = activeSubTab || "info"

  const handleTabChange = (newTab: string) => {
    onTabChange?.("response-rag", newTab)
  }

  useEffect(() => {
    let ignore = false

    invoke<ContextManagementConfig>("get_context_management_config")
      .then((cfg) => { if (!ignore) setConfig(cfg) })
      .catch((err) => console.error("Failed to load context management config:", err))

    return () => {
      ignore = true
    }
  }, [])

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
            <Database className={`h-6 w-6 ${OPTIMIZE_COLORS.responseRag}`} />
            MCP Response RAG
          </h1>
          <Badge variant="outline" className="bg-purple-500/10 text-purple-900 dark:text-purple-400">EXPERIMENTAL</Badge>
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
            <Info className="h-3.5 w-3.5 mr-1" />
            Info
          </TabsTrigger>
          <TabsTrigger value="preview">
            <PlayCircle className="h-3.5 w-3.5 mr-1" />
            Try it out
          </TabsTrigger>
          <TabsTrigger value="settings">
            <Settings className="h-3.5 w-3.5 mr-1" />
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
          </div>
        </TabsContent>

        {/* Settings Tab */}
        <TabsContent value="settings" className="flex-1 min-h-0 mt-4">
          {config && (
            <div className="space-y-4 max-w-2xl">
              {/* Response Threshold */}
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">Response Compression Threshold</CardTitle>
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
          <RagPreview initialThreshold={config?.response_threshold_bytes ?? DEFAULT_RESPONSE_THRESHOLD_BYTES} />
        </TabsContent>
      </Tabs>
    </div>
  )
}


// ─────────────────────────────────────────────────────────
// RagPreview component
// ─────────────────────────────────────────────────────────

interface RagPreviewProps {
  initialThreshold: number
}

function RagPreview({ initialThreshold }: RagPreviewProps) {
  const [content, setContent] = useState(SAMPLE_DOCUMENT)
  const [indexResult, setIndexResult] = useState<RagPreviewIndexResult | null>(null)
  const [indexing, setIndexing] = useState(false)

  const sourceLabel = "tool-response:1"

  // Search state
  const [searchQuery, setSearchQuery] = useState("")
  const [searchResults, setSearchResults] = useState<RagSearchResult[] | null>(null)
  const [searching, setSearching] = useState(false)

  // Read state
  const [readOffset, setReadOffset] = useState("")
  const [readLimit, setReadLimit] = useState("")
  const [readResult, setReadResult] = useState<RagReadResult | null>(null)
  const [reading, setReading] = useState(false)

  // Tool definitions
  const [contextTools, setContextTools] = useState<McpToolDisplayItem[]>([])

  useEffect(() => {
    invoke<ToolDefinition[]>("get_context_mode_tool_definitions")
      .then((defs) =>
        setContextTools(
          defs.map((d): McpToolDisplayItem => ({
            name: d.name,
            description: d.description,
            inputSchema: d.input_schema,
          }))
        )
      )
      .catch(() => setContextTools([]))
  }, [])

  const doIndex = useCallback(async (text: string) => {
    if (!text.trim()) return
    setIndexing(true)
    try {
      const result = await invoke<RagPreviewIndexResult>("preview_rag_index", {
        content: text,
        label: sourceLabel,
        responseThresholdBytes: initialThreshold,
      } satisfies PreviewRagIndexParams)
      setIndexResult(result)
    } catch (e) {
      toast.error(`Failed to index: ${e}`)
    } finally {
      setIndexing(false)
    }
  }, [initialThreshold])

  // Auto-index with debounce on content change
  useEffect(() => {
    if (!content.trim()) return
    const timer = setTimeout(() => {
      doIndex(content)
    }, 500)
    return () => clearTimeout(timer)
  }, [content, doIndex])

  const doSearch = useCallback(async () => {
    if (!searchQuery.trim()) return
    setSearching(true)
    try {
      const results = await invoke<RagSearchResult[]>("preview_rag_search", {
        query: searchQuery,
        limit: 5,
      } satisfies PreviewRagSearchParams)
      setSearchResults(results)
    } catch (e) {
      toast.error(`Search failed: ${e}`)
    } finally {
      setSearching(false)
    }
  }, [searchQuery])

  const doRead = useCallback(async () => {
    if (!indexResult) return
    setReading(true)
    try {
      const result = await invoke<RagReadResult>("preview_rag_read", {
        label: indexResult.sources[0]?.label ?? sourceLabel,
        offset: readOffset || null,
        limit: readLimit ? parseInt(readLimit) : null,
      } satisfies PreviewRagReadParams)
      setReadResult(result)
    } catch (e) {
      toast.error(`Read failed: ${e}`)
    } finally {
      setReading(false)
    }
  }, [indexResult, readOffset, readLimit])

  return (
    <div className="space-y-4">
      {/* Input Card */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Document Input</CardTitle>
          <CardDescription>
            Paste or edit a document to see how Response RAG indexes and compresses it.
            Changes are automatically re-indexed.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <textarea
            value={content}
            onChange={(e) => setContent(e.target.value)}
            className="w-full h-48 rounded-md border border-input bg-background px-3 py-2 text-xs font-mono ring-offset-background focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2 resize-y"
            placeholder="Paste document content here..."
          />
          <div className="flex items-center gap-3 flex-wrap text-xs text-muted-foreground">
            <span>{content.length.toLocaleString()} bytes</span>
            {indexing && (
              <span className="flex items-center gap-1">
                <Loader2 className="h-3 w-3 animate-spin" />
                Indexing...
              </span>
            )}
            {indexResult && !indexing && (
              <>
                <span>{indexResult.index_result.total_lines} lines</span>
                <span>{indexResult.index_result.total_chunks} chunks ({indexResult.index_result.code_chunks} code)</span>
                <span>{formatBytes(indexResult.index_result.content_bytes)}</span>
              </>
            )}
          </div>
        </CardContent>
      </Card>

      {/* Compressed Preview */}
      {indexResult && (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-base flex items-center justify-between">
              <span>Compressed (what LLM sees)</span>
              <span className="text-xs font-mono font-normal text-muted-foreground">{formatBytes(indexResult.compressed_preview.length)}</span>
            </CardTitle>
            <CardDescription>
              This is the compressed version that replaces the original {formatBytes(content.length)} response in the LLM&apos;s context window.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="bg-muted/50 rounded-md p-3 max-h-[400px] overflow-y-auto">
              <pre className="text-xs whitespace-pre-wrap font-mono leading-relaxed">{indexResult.compressed_preview}</pre>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Chunk Table of Contents */}
      {indexResult && indexResult.index_result.chunk_titles.length > 0 && (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-base">Indexed Chunks</CardTitle>
            <CardDescription>
              The document was split into {indexResult.index_result.total_chunks} searchable chunks.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="space-y-0.5">
              {indexResult.index_result.chunk_titles.map((chunk, i) => (
                <div
                  key={i}
                  className="flex items-center gap-2 py-1 px-2 rounded text-sm hover:bg-muted/50"
                  style={{ paddingLeft: `${8 + chunk.depth * 16}px` }}
                >
                  <code className="text-[10px] text-muted-foreground font-mono w-8 text-right shrink-0">
                    L{chunk.line_ref}
                  </code>
                  <span className="text-xs truncate">{chunk.title}</span>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      )}

      {/* MCP Tools exposed to the LLM */}
      {indexResult && contextTools.length > 0 && (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-base flex items-center gap-2">
              <Wrench className="h-4 w-4" />
              MCP Tools
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

      {/* IndexSearch */}
      {indexResult && (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-base flex items-center gap-2">
              <Search className="h-4 w-4" />
              IndexSearch
            </CardTitle>
            <CardDescription>
              Search the indexed content using FTS5 full-text search. This is what the LLM calls to find relevant sections.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex gap-2">
              <Input
                placeholder="Search query... (e.g. &quot;rate limiting&quot;, &quot;login endpoint&quot;)"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") doSearch()
                }}
                className="flex-1"
              />
              <Button
                size="sm"
                onClick={doSearch}
                disabled={searching || !searchQuery.trim()}
              >
                {searching ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin mr-1" />
                ) : (
                  <Search className="h-3.5 w-3.5 mr-1" />
                )}
                Search
              </Button>
            </div>

            {searchResults !== null && (
              <div className="bg-muted/50 rounded-md p-3 max-h-[400px] overflow-y-auto">
                {searchResults.length === 0 || searchResults.every(r => r.hits.length === 0) ? (
                  <p className="text-sm text-muted-foreground">No results found.</p>
                ) : (
                  <div className="space-y-3">
                    {searchResults.map((r, ri) => (
                      <div key={ri}>
                        {r.corrected_query && (
                          <p className="text-xs text-muted-foreground italic mb-2">
                            Corrected to: &quot;{r.corrected_query}&quot;
                          </p>
                        )}
                        {r.hits.map((hit, hi) => (
                          <div key={hi} className="mb-3">
                            <p className="text-xs font-semibold mb-1">
                              [{hi + 1}] {hit.source} — {hit.title} (lines {hit.line_start}-{hit.line_end})
                            </p>
                            <pre className="text-xs whitespace-pre-wrap font-mono leading-relaxed pl-2 border-l-2 border-border">{hit.content}</pre>
                          </div>
                        ))}
                        <p className="text-[10px] text-muted-foreground mt-2">
                          Use IndexRead(source=&quot;{r.hits[0]?.source ?? sourceLabel}&quot;, offset, limit) for full context.
                        </p>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}
          </CardContent>
        </Card>
      )}

      {/* IndexRead */}
      {indexResult && (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-base flex items-center gap-2">
              <BookOpen className="h-4 w-4" />
              IndexRead
            </CardTitle>
            <CardDescription>
              Read the full indexed content with pagination. The LLM uses this to retrieve specific sections after searching.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex gap-2 items-center flex-wrap">
              <div className="flex items-center gap-1.5">
                <label className="text-sm text-muted-foreground whitespace-nowrap">Source:</label>
                <code className="text-xs bg-muted px-1.5 py-0.5 rounded">
                  {indexResult.sources[0]?.label ?? sourceLabel}
                </code>
              </div>
              <div className="flex items-center gap-1.5">
                <label className="text-sm text-muted-foreground">Offset:</label>
                <Input
                  placeholder="e.g. 10"
                  value={readOffset}
                  onChange={(e) => setReadOffset(e.target.value)}
                  className="w-20 h-8 text-sm"
                />
              </div>
              <div className="flex items-center gap-1.5">
                <label className="text-sm text-muted-foreground">Limit:</label>
                <Input
                  placeholder="e.g. 50"
                  value={readLimit}
                  onChange={(e) => setReadLimit(e.target.value)}
                  className="w-20 h-8 text-sm"
                />
              </div>
              <Button
                size="sm"
                onClick={doRead}
                disabled={reading}
              >
                {reading ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin mr-1" />
                ) : (
                  <BookOpen className="h-3.5 w-3.5 mr-1" />
                )}
                Read
              </Button>
            </div>

            {readResult && (
              <div className="space-y-2">
                <div className="flex gap-3 text-xs text-muted-foreground">
                  <span>Lines {readResult.showing_start}-{readResult.showing_end} of {readResult.total_lines}</span>
                </div>
                <div className="bg-muted/50 rounded-md p-3 max-h-[400px] overflow-y-auto">
                  <pre className="text-xs whitespace-pre-wrap font-mono leading-relaxed">{readResult.content}</pre>
                </div>
              </div>
            )}
          </CardContent>
        </Card>
      )}

    </div>
  )
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B"
  if (bytes < 1024) return `${bytes} B`
  const kb = bytes / 1024
  if (kb < 1024) return `${kb.toFixed(kb < 10 ? 1 : 0)} KB`
  const mb = kb / 1024
  return `${mb.toFixed(1)} MB`
}
