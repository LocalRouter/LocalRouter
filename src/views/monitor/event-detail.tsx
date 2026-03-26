import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { McpToolDisplay, type McpToolDisplayItem } from '@/components/shared/McpToolDisplay'
import { cn } from '@/lib/utils'
import { Clock, User, Server, Copy, Check, FileText } from 'lucide-react'
import { useState, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import type { MonitorEvent, ReadMemoryArchiveFileParams } from '@/types/tauri-commands'

interface EventDetailProps {
  event: MonitorEvent | null
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type EventData = Record<string, any>

export function EventDetail({ event }: EventDetailProps) {
  if (!event) {
    return (
      <div className="flex h-full items-center justify-center text-muted-foreground text-sm">
        Select an event to view details
      </div>
    )
  }

  const data = event.data as EventData
  const type = data.type as string
  const [copied, setCopied] = useState(false)

  const handleCopyEvent = useCallback(() => {
    navigator.clipboard.writeText(JSON.stringify(event, null, 2)).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    })
  }, [event])

  return (
    <ScrollArea className="h-full">
      <div className="p-3 space-y-3">
        {/* Header */}
        <div className="flex items-center gap-2 flex-wrap">
          <Badge variant={event.status === 'error' ? 'destructive' : event.status === 'pending' ? 'secondary' : 'default'}>
            {event.status}
          </Badge>
          <span className="text-xs text-muted-foreground font-mono">
            {new Date(event.timestamp).toLocaleString()}
          </span>
          {event.duration_ms != null && (
            <span className="text-xs text-muted-foreground flex items-center gap-1">
              <Clock className="h-3 w-3" />
              {event.duration_ms}ms
            </span>
          )}
          <Button
            variant="ghost"
            size="icon"
            className="ml-auto h-6 w-6"
            onClick={handleCopyEvent}
            title="Copy event JSON to clipboard"
          >
            {copied ? <Check className="h-3.5 w-3.5 text-green-500" /> : <Copy className="h-3.5 w-3.5" />}
          </Button>
        </div>

        {/* Client info */}
        {(event.client_id || event.client_name) && (
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <User className="h-3 w-3" />
            <span>{event.client_name || event.client_id}</span>
          </div>
        )}

        {/* Type-specific rendering */}
        {type === 'llm_call' && <LlmCallDetail data={data} />}
        {type === 'mcp_tool_call' && <McpToolCallDetail data={data} />}
        {type === 'mcp_resource_read' && <McpResourceReadDetail data={data} />}
        {type === 'mcp_prompt_get' && <McpPromptGetDetail data={data} />}
        {type === 'mcp_elicitation' && <McpElicitationDetail data={data} />}
        {type === 'mcp_sampling' && <McpSamplingDetail data={data} />}
        {type === 'guardrail_scan' && <GuardrailDetail data={data} />}
        {type === 'guardrail_response_scan' && <GuardrailDetail data={data} />}
        {type === 'secret_scan' && <SecretScanDetail data={data} />}
        {type === 'route_llm_classify' && <RoutingDetail data={data} />}
        {type === 'routing_decision' && <RoutingDetail data={data} />}
        {(type === 'auth_error' || type === 'access_denied') && <AuthErrorDetail data={data} />}
        {type === 'rate_limit_event' && <RateLimitDetail data={data} />}
        {type === 'validation_error' && <ValidationErrorDetail data={data} />}
        {type === 'mcp_server_event' && <McpServerEventDetail data={data} />}
        {type === 'oauth_event' && <OAuthEventDetail data={data} />}
        {type === 'internal_error' && <InternalErrorDetail data={data} />}
        {type === 'moderation_event' && <ModerationEventDetail data={data} />}
        {type === 'connection_error' && <ConnectionErrorDetail data={data} />}
        {type === 'prompt_compression' && <PromptCompressionDetail data={data} />}
        {type === 'memory_compaction' && <MemoryCompactionDetail data={data} />}
        {type === 'firewall_decision' && <FirewallDecisionDetail data={data} />}
        {type === 'sse_connection' && <SseConnectionDetail data={data} />}
      </div>
    </ScrollArea>
  )
}

// ---- Utility Functions ----

const ROLE_COLORS: Record<string, string> = {
  system: 'bg-purple-500/10 text-purple-700 dark:text-purple-400 border-purple-500/20',
  user: 'bg-blue-500/10 text-blue-700 dark:text-blue-400 border-blue-500/20',
  assistant: 'bg-green-500/10 text-green-700 dark:text-green-400 border-green-500/20',
  tool: 'bg-orange-500/10 text-orange-700 dark:text-orange-400 border-orange-500/20',
  developer: 'bg-purple-500/10 text-purple-700 dark:text-purple-400 border-purple-500/20',
}

function extractTextContent(content: unknown): string | null {
  if (typeof content === 'string') return content
  if (Array.isArray(content)) {
    const textParts = (content as Array<Record<string, unknown>>)
      .filter(p => (p.type as string) === 'text')
      .map(p => p.text as string)
    return textParts.length > 0 ? textParts.join('\n') : null
  }
  if (content && typeof content === 'object') return JSON.stringify(content)
  return null
}

function hasImageContent(content: unknown): boolean {
  return Array.isArray(content) && (content as Array<Record<string, unknown>>).some(
    p => (p.type as string) === 'image_url'
  )
}

function formatToolArgs(args: unknown): string {
  if (typeof args === 'string') {
    try { return JSON.stringify(JSON.parse(args), null, 2) } catch { return args }
  }
  return JSON.stringify(args, null, 2)
}

/** Pretty-print a JSON string. Returns the original string if parsing fails. */
function formatJsonString(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2)
  } catch {
    return raw
  }
}

function extractMcpContent(raw: string): string {
  try {
    const parsed = JSON.parse(raw)
    if (parsed?.content && Array.isArray(parsed.content)) {
      const textParts = (parsed.content as Array<Record<string, unknown>>)
        .filter(p => p.type === 'text' && typeof p.text === 'string')
        .map(p => p.text as string)
      if (textParts.length > 0) return textParts.join('\n')
    }
    return JSON.stringify(parsed, null, 2)
  } catch {
    return raw
  }
}

// ---- Reusable Display Components ----

function MessageItem({ message }: { message: Record<string, unknown> }) {
  const role = (message.role as string) || 'unknown'
  const contentText = extractTextContent(message.content)
  const toolCalls = message.tool_calls as Array<Record<string, unknown>> | undefined
  const toolCallId = message.tool_call_id as string | undefined
  const name = message.name as string | undefined

  return (
    <div className="rounded-md border text-xs overflow-hidden">
      <div className={cn('flex items-center gap-2 px-2 py-1 border-b', ROLE_COLORS[role] || 'bg-muted')}>
        <span className="font-medium text-[11px]">{role}</span>
        {name && <span className="text-[10px] opacity-70 font-mono">{name}</span>}
        {toolCallId && <span className="text-[10px] opacity-70 font-mono truncate">← {toolCallId}</span>}
      </div>
      {contentText && (
        <div className="px-2 py-1.5 whitespace-pre-wrap max-h-[200px] overflow-auto">
          {contentText}
        </div>
      )}
      {hasImageContent(message.content) && (
        <div className="px-2 py-1 text-[10px] text-muted-foreground italic">[image content]</div>
      )}
      {toolCalls && toolCalls.length > 0 && (
        <div className={cn('px-2 py-1.5 space-y-1', contentText && 'border-t')}>
          {toolCalls.map((tc, i) => {
            const fn = tc.function as Record<string, unknown> | undefined
            return (
              <div key={i} className="border-l-2 border-blue-500/40 pl-2">
                <code className="font-mono font-medium text-[11px]">{String(fn?.name ?? '')}</code>
                {fn?.arguments != null && (
                  <pre className="bg-muted rounded p-1 mt-0.5 text-[10px] whitespace-pre-wrap max-h-[150px] overflow-auto">
                    {formatToolArgs(fn.arguments)}
                  </pre>
                )}
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}

function Field({ label, value }: { label: string; value: string | undefined }) {
  if (!value || value === 'undefined' || value === 'null') return null
  return (
    <div className="text-xs">
      <span className="text-muted-foreground">{label}: </span>
      <span className="font-medium">{value}</span>
    </div>
  )
}

function ServerField({ data }: { data: EventData }) {
  return (
    <div className="flex items-center gap-1 text-xs">
      <Server className="h-3 w-3 text-muted-foreground" />
      <span className="text-muted-foreground">Server:</span>
      <span className="font-medium">{(data.server_name || data.server_id) as string}</span>
    </div>
  )
}

function JsonBlock({ data }: { data: unknown }) {
  if (data === null || data === undefined) return null

  const cleanData = typeof data === 'object' && !Array.isArray(data)
    ? Object.fromEntries(Object.entries(data as Record<string, unknown>).filter(([, v]) => v != null))
    : data

  if (typeof cleanData === 'object' && !Array.isArray(cleanData) && Object.keys(cleanData as Record<string, unknown>).length === 0) return null

  return (
    <pre className="p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[400px] overflow-auto">
      {JSON.stringify(cleanData, null, 2)}
    </pre>
  )
}

function ArgumentsBlock({ args }: { args: unknown }) {
  if (args == null || (typeof args === 'object' && Object.keys(args as Record<string, unknown>).length === 0)) return null

  const entries = typeof args === 'object' && !Array.isArray(args)
    ? Object.entries(args as Record<string, unknown>).filter(([, v]) => v != null)
    : []

  if (entries.length === 0) {
    return <JsonBlock data={args} />
  }

  return (
    <table className="text-xs w-full">
      <tbody>
        {entries.map(([key, value]) => (
          <tr key={key} className="border-b border-border/20">
            <td className="text-muted-foreground py-0.5 pr-4 whitespace-nowrap align-top">{key}</td>
            <td className="py-0.5 font-mono whitespace-pre-wrap break-all">
              {typeof value === 'string' ? value : JSON.stringify(value, null, 2)}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

function McpResponseTab({ data }: { data: EventData }) {
  const rawContent = (data.response_preview || data.content_preview) as string | undefined
  const extractedContent = rawContent ? extractMcpContent(rawContent) : null
  const hasRawBody = rawContent != null && rawContent.length > 0

  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        {data.success != null && <Field label="Success" value={String(data.success)} />}
        {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
      </div>
      {data.error && (
        <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
          {data.error as string}
        </pre>
      )}
      {(extractedContent || hasRawBody) && (
        <Tabs defaultValue={extractedContent ? 'content' : 'full_body'}>
          <TabsList className={SUB_TABS_LIST}>
            {extractedContent && (
              <TabsTrigger value="content" className={SUB_TAB}>Content</TabsTrigger>
            )}
            {hasRawBody && (
              <TabsTrigger value="full_body" className={SUB_TAB}>Full Body</TabsTrigger>
            )}
          </TabsList>
          {extractedContent && (
            <TabsContent value="content">
              <pre className="p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[400px] overflow-auto">
                {extractedContent}
              </pre>
            </TabsContent>
          )}
          {hasRawBody && (
            <TabsContent value="full_body">
              <pre className="p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[400px] overflow-auto">
                {formatJsonString(rawContent)}
              </pre>
            </TabsContent>
          )}
        </Tabs>
      )}
    </div>
  )
}

// Sub-tab styling
const SUB_TABS_LIST = "h-7 w-full bg-muted/50 p-0.5"
const SUB_TAB = "text-[11px] h-6 px-2.5"

// ---- LLM Response Content (sub-tabs: Overview | Content | Tool Calls | Full Body) ----

function LlmResponseContent({ data }: { data: EventData }) {
  const responseBody = data.response_body as Record<string, unknown> | undefined
  const choices = responseBody?.choices as Array<Record<string, unknown>> | undefined
  const firstChoice = choices?.[0] as Record<string, unknown> | undefined
  const message = firstChoice?.message as Record<string, unknown> | undefined
  const toolCalls = message?.tool_calls as Array<Record<string, unknown>> | undefined
  const hasToolCalls = toolCalls != null && toolCalls.length > 0
  const reasoningContent = message?.reasoning_content as string | undefined
  const hasReasoning = reasoningContent != null && reasoningContent.length > 0

  const hasEmptyResponse = !data.content_preview && !hasToolCalls && !hasReasoning && responseBody != null
  const defaultSubTab = hasEmptyResponse ? 'empty'
    : hasReasoning ? 'reasoning'
    : hasToolCalls && !data.content_preview ? 'tool_calls'
    : data.content_preview ? 'content' : 'overview'

  return (
    <div className="space-y-2">
      <table className="w-full text-xs">
        <tbody>
          <tr className="border-b border-border/30">
            <td className="text-muted-foreground py-0.5 pr-2 whitespace-nowrap">Provider</td>
            <td className="py-0.5 font-medium">{data.provider as string}</td>
            <td className="text-muted-foreground py-0.5 pr-2 pl-4 whitespace-nowrap">Status</td>
            <td className="py-0.5 font-medium">{data.status_code != null ? String(data.status_code) : '—'}</td>
            {data.streamed != null && (
              <>
                <td className="text-muted-foreground py-0.5 pr-2 pl-4 whitespace-nowrap">Streamed</td>
                <td className="py-0.5 font-medium">{String(data.streamed)}</td>
              </>
            )}
          </tr>
          {data.total_tokens != null && (
            <tr className="border-b border-border/30">
              <td className="text-muted-foreground py-0.5 pr-2 whitespace-nowrap">Input</td>
              <td className="py-0.5 font-medium">{String(data.input_tokens)}</td>
              <td className="text-muted-foreground py-0.5 pr-2 pl-4 whitespace-nowrap">Output</td>
              <td className="py-0.5 font-medium">{String(data.output_tokens)}</td>
              <td className="text-muted-foreground py-0.5 pr-2 pl-4 whitespace-nowrap">Total</td>
              <td className="py-0.5 font-medium">{String(data.total_tokens)}</td>
            </tr>
          )}
          {(data.reasoning_tokens != null && (data.reasoning_tokens as number) > 0) && (
            <tr className="border-b border-border/30">
              <td className="text-muted-foreground py-0.5 pr-2 whitespace-nowrap">Reasoning</td>
              <td className="py-0.5 font-medium" colSpan={5}>{String(data.reasoning_tokens)}</td>
            </tr>
          )}
          {(data.cost_usd != null || data.latency_ms != null || data.finish_reason) && (
            <tr>
              {data.latency_ms != null && (
                <>
                  <td className="text-muted-foreground py-0.5 pr-2 whitespace-nowrap">Latency</td>
                  <td className="py-0.5 font-medium">{String(data.latency_ms)}ms</td>
                </>
              )}
              {data.cost_usd != null && (
                <>
                  <td className="text-muted-foreground py-0.5 pr-2 pl-4 whitespace-nowrap">Cost</td>
                  <td className="py-0.5 font-medium">${(data.cost_usd as number).toFixed(6)}</td>
                </>
              )}
              {data.finish_reason && (
                <>
                  <td className="text-muted-foreground py-0.5 pr-2 pl-4 whitespace-nowrap">Finish</td>
                  <td className="py-0.5 font-medium">{data.finish_reason as string}</td>
                </>
              )}
            </tr>
          )}
        </tbody>
      </table>

      <Tabs defaultValue={defaultSubTab}>
        <TabsList className={SUB_TABS_LIST}>
          {hasEmptyResponse && (
            <TabsTrigger value="empty" className={SUB_TAB}>Response</TabsTrigger>
          )}
          {data.content_preview && (
            <TabsTrigger value="content" className={SUB_TAB}>Content</TabsTrigger>
          )}
          {hasReasoning && (
            <TabsTrigger value="reasoning" className={SUB_TAB}>Reasoning</TabsTrigger>
          )}
          {hasToolCalls && (
            <TabsTrigger value="tool_calls" className={SUB_TAB}>
              Tool Calls ({toolCalls.length})
            </TabsTrigger>
          )}
          {responseBody && (
            <TabsTrigger value="full_body" className={SUB_TAB}>Full Body</TabsTrigger>
          )}
        </TabsList>

        {hasEmptyResponse && (
          <TabsContent value="empty">
            <div className="p-3 rounded border border-yellow-500/30 bg-yellow-500/5 text-xs text-yellow-700 dark:text-yellow-400">
              The LLM returned no text content.
            </div>
          </TabsContent>
        )}

        {data.content_preview && (
          <TabsContent value="content">
            <pre className="p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[300px] overflow-auto">
              {data.content_preview as string}
            </pre>
          </TabsContent>
        )}

        {hasReasoning && (
          <TabsContent value="reasoning">
            <pre className="p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[300px] overflow-auto">
              {reasoningContent}
            </pre>
          </TabsContent>
        )}

        {hasToolCalls && (
          <TabsContent value="tool_calls">
            <div className="space-y-1.5">
              {toolCalls.map((tc, i) => {
                const fn = tc.function as Record<string, unknown> | undefined
                return (
                  <div key={i} className="rounded-md border text-xs overflow-hidden">
                    <div className="flex items-center gap-2 px-2 py-1 border-b bg-blue-500/10 text-blue-700 dark:text-blue-400">
                      <span className="font-mono font-medium text-[11px]">{String(fn?.name ?? tc.type ?? 'unknown')}</span>
                      {tc.id != null && <span className="text-[10px] opacity-70 font-mono">{String(tc.id)}</span>}
                    </div>
                    {fn?.arguments != null && (
                      <pre className="px-2 py-1.5 bg-muted/50 whitespace-pre-wrap max-h-[200px] overflow-auto">
                        {formatToolArgs(fn.arguments)}
                      </pre>
                    )}
                  </div>
                )
              })}
            </div>
          </TabsContent>
        )}

        {responseBody && (
          <TabsContent value="full_body">
            <JsonBlock data={responseBody} />
          </TabsContent>
        )}
      </Tabs>

      {!data.content_preview && !hasToolCalls && !responseBody && (
        <p className="p-2 bg-muted rounded text-xs text-muted-foreground italic">
          No text content{data.finish_reason === 'tool_calls' ? ' — response contained tool calls only' : ''}
        </p>
      )}
    </div>
  )
}

// ---- LLM Call Detail ----

function LlmCallDetail({ data }: { data: EventData }) {
  const body = data.request_body as Record<string, unknown> | undefined
  const transformedBody = data.transformed_body as Record<string, unknown> | undefined
  const transformations = data.transformations_applied as string[] | undefined
  const hasTransformed = transformedBody != null
  const hasResponse = data.provider != null || data.response_body != null
  const hasError = data.error != null
  const routingInfo = data.routing_info as { routellm_tier?: string | null; routellm_win_rate?: number | null; candidate_models?: string[]; attempts?: Array<{ provider: string; model: string; outcome: string; error?: string | null; duration_ms?: number | null }>; total_attempts?: number; successful_attempt?: number | null } | undefined
  const hasRouting = routingInfo != null && routingInfo.attempts != null

  const [showTransformed, setShowTransformed] = useState(hasTransformed)

  const activeBody = (showTransformed && transformedBody) ? transformedBody : body
  const messages = activeBody?.messages as Array<Record<string, unknown>> | undefined
  const tools = activeBody?.tools as Array<Record<string, unknown>> | undefined

  const params = activeBody ? [
    ['temperature', activeBody.temperature],
    ['max_tokens', activeBody.max_tokens],
    ['top_p', activeBody.top_p],
    ['frequency_penalty', activeBody.frequency_penalty],
    ['presence_penalty', activeBody.presence_penalty],
    ['seed', activeBody.seed],
    ['top_k', activeBody.top_k],
    ['repetition_penalty', activeBody.repetition_penalty],
  ].filter(([, v]) => v != null) as [string, unknown][] : []

  const toolDisplayItems: McpToolDisplayItem[] = (tools || []).map(t => {
    const fn = t.function as Record<string, unknown> | undefined
    return {
      name: (fn?.name as string) || (t.name as string) || 'unknown',
      description: (fn?.description as string) || null,
      inputSchema: (fn?.parameters as Record<string, unknown>) || null,
    }
  })

  const defaultSubTab = messages && messages.length > 0 ? 'messages'
    : tools && tools.length > 0 ? 'tools'
    : params.length > 0 ? 'parameters' : 'body'

  return (
    <Tabs defaultValue="request">
      <TabsList className="w-full">
        <TabsTrigger value="request">Request</TabsTrigger>
        <TabsTrigger value="response" disabled={!hasResponse}>Response</TabsTrigger>
        {hasRouting && <TabsTrigger value="routing">Routing</TabsTrigger>}
        <TabsTrigger value="error" disabled={!hasError}>Error</TabsTrigger>
      </TabsList>

      <TabsContent value="request" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Endpoint" value={data.endpoint as string} />
          <Field label="Model" value={data.model as string} />
          <Field label="Stream" value={data.stream != null ? String(data.stream) : undefined} />
        </div>

        {hasTransformed && (
          <div className="flex items-center gap-2">
            <div className="inline-flex rounded-md border border-border text-[11px]">
              <button
                onClick={() => setShowTransformed(false)}
                className={cn(
                  'px-2 py-0.5 rounded-l-md transition-colors',
                  !showTransformed ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:text-foreground'
                )}
              >
                Original
              </button>
              <button
                onClick={() => setShowTransformed(true)}
                className={cn(
                  'px-2 py-0.5 rounded-r-md border-l transition-colors',
                  showTransformed ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:text-foreground'
                )}
              >
                Transformed
              </button>
            </div>
            {showTransformed && transformations && transformations.length > 0 && (
              <div className="flex items-center gap-1 flex-wrap">
                {transformations.map((t, i) => (
                  <Badge key={i} variant="secondary" className="text-[10px]">{t}</Badge>
                ))}
              </div>
            )}
          </div>
        )}

        {activeBody && (
          <Tabs defaultValue={defaultSubTab} key={showTransformed ? 'transformed' : 'original'}>
            <TabsList className={SUB_TABS_LIST}>
              {messages && messages.length > 0 && (
                <TabsTrigger value="messages" className={SUB_TAB}>
                  Messages ({messages.length})
                </TabsTrigger>
              )}
              {tools && tools.length > 0 && (
                <TabsTrigger value="tools" className={SUB_TAB}>
                  Tools ({tools.length})
                </TabsTrigger>
              )}
              {params.length > 0 && (
                <TabsTrigger value="parameters" className={SUB_TAB}>
                  Parameters
                </TabsTrigger>
              )}
              <TabsTrigger value="body" className={SUB_TAB}>
                Full Body
              </TabsTrigger>
            </TabsList>

            {messages && messages.length > 0 && (
              <TabsContent value="messages">
                <div className="space-y-1.5">
                  {messages.map((msg, i) => (
                    <MessageItem key={i} message={msg} />
                  ))}
                </div>
              </TabsContent>
            )}

            {tools && tools.length > 0 && (
              <TabsContent value="tools">
                <McpToolDisplay tools={toolDisplayItems} compact />
              </TabsContent>
            )}

            {params.length > 0 && (
              <TabsContent value="parameters">
                <table className="text-xs w-full">
                  <tbody>
                    {params.map(([key, value]) => (
                      <tr key={key} className="border-b border-border/20">
                        <td className="text-muted-foreground py-0.5 pr-4 whitespace-nowrap">{key}</td>
                        <td className="py-0.5 font-mono">{String(value)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </TabsContent>
            )}

            <TabsContent value="body">
              <JsonBlock data={activeBody} />
            </TabsContent>
          </Tabs>
        )}
      </TabsContent>

      {hasResponse && (
        <TabsContent value="response">
          <LlmResponseContent data={data} />
        </TabsContent>
      )}

      {hasRouting && routingInfo && (
        <TabsContent value="routing" className="space-y-2">
          {routingInfo.routellm_win_rate != null && (
            <div className="grid grid-cols-2 gap-2 text-xs">
              <Field label="RouteLLM Tier" value={routingInfo.routellm_tier || '?'} />
              <Field label="Win Rate" value={routingInfo.routellm_win_rate.toFixed(3)} />
            </div>
          )}
          <div className="text-xs text-muted-foreground">
            {routingInfo.total_attempts} attempt{routingInfo.total_attempts !== 1 ? 's' : ''} across {routingInfo.candidate_models?.length || 0} candidate model{(routingInfo.candidate_models?.length || 0) !== 1 ? 's' : ''}
          </div>
          <div className="space-y-1">
            {(routingInfo.attempts || []).map((attempt, i) => {
              const isSuccess = attempt.outcome === 'success'
              const isSkip = ['backoff', 'not_free', 'cost_backoff', 'rate_limited', 'provider_not_found'].includes(attempt.outcome)
              return (
                <div
                  key={i}
                  className={cn(
                    'flex items-center gap-2 text-xs px-2 py-1 rounded border',
                    isSuccess ? 'bg-green-500/10 border-green-500/30' :
                    isSkip ? 'bg-yellow-500/10 border-yellow-500/30' :
                    'bg-destructive/10 border-destructive/30'
                  )}
                >
                  <span className="font-mono font-medium min-w-0 truncate">
                    {attempt.provider}/{attempt.model}
                  </span>
                  <Badge
                    variant={isSuccess ? 'default' : isSkip ? 'secondary' : 'destructive'}
                    className="text-[10px] shrink-0"
                  >
                    {attempt.outcome}
                  </Badge>
                  {attempt.duration_ms != null && (
                    <span className="text-muted-foreground shrink-0">{attempt.duration_ms}ms</span>
                  )}
                  {routingInfo.successful_attempt === i && (
                    <span className="text-green-500 shrink-0">&#10003;</span>
                  )}
                </div>
              )
            })}
          </div>
          {(routingInfo.attempts || []).some(a => a.error) && (
            <details className="text-xs">
              <summary className="text-muted-foreground cursor-pointer">Error details</summary>
              <div className="mt-1 space-y-1">
                {(routingInfo.attempts || []).filter(a => a.error).map((a, i) => (
                  <div key={i} className="text-destructive/80 font-mono text-[11px] break-all">
                    <span className="text-muted-foreground">{a.provider}/{a.model}:</span> {a.error}
                  </div>
                ))}
              </div>
            </details>
          )}
        </TabsContent>
      )}

      {hasError && (
        <TabsContent value="error" className="space-y-2">
          <div className="grid grid-cols-2 gap-2 text-xs">
            {data.provider && <Field label="Provider" value={data.provider as string} />}
            {data.status_code != null && <Field label="Status Code" value={String(data.status_code)} />}
          </div>
          <Tabs defaultValue="message">
            <TabsList className={SUB_TABS_LIST}>
              <TabsTrigger value="message" className={SUB_TAB}>Message</TabsTrigger>
              {data.response_body && (
                <TabsTrigger value="full_body" className={SUB_TAB}>Full Body</TabsTrigger>
              )}
            </TabsList>
            <TabsContent value="message">
              <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
                {data.error as string}
              </pre>
            </TabsContent>
            {data.response_body && (
              <TabsContent value="full_body">
                <JsonBlock data={data.response_body as unknown} />
              </TabsContent>
            )}
          </Tabs>
        </TabsContent>
      )}
    </Tabs>
  )
}

// ---- MCP Tool Call Detail ----

function McpToolCallDetail({ data }: { data: EventData }) {
  const hasResponse = data.success != null || data.error != null || data.response_preview != null || data.content_preview != null

  return (
    <Tabs defaultValue="request">
      <TabsList className="w-full">
        <TabsTrigger value="request">Request</TabsTrigger>
        <TabsTrigger value="response" disabled={!hasResponse}>Response</TabsTrigger>
      </TabsList>

      <TabsContent value="request" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Tool" value={data.tool_name as string} />
          <ServerField data={data} />
          {data.firewall_action && <Field label="Firewall" value={data.firewall_action as string} />}
        </div>
        {data.arguments != null && <ArgumentsBlock args={data.arguments as unknown} />}
      </TabsContent>

      {hasResponse && (
        <TabsContent value="response">
          <McpResponseTab data={data} />
        </TabsContent>
      )}
    </Tabs>
  )
}

// ---- MCP Resource Read Detail ----

function McpResourceReadDetail({ data }: { data: EventData }) {
  const hasResponse = data.success != null || data.error != null || data.response_preview != null || data.content_preview != null

  return (
    <Tabs defaultValue="request">
      <TabsList className="w-full">
        <TabsTrigger value="request">Request</TabsTrigger>
        <TabsTrigger value="response" disabled={!hasResponse}>Response</TabsTrigger>
      </TabsList>

      <TabsContent value="request" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="URI" value={data.uri as string} />
          <ServerField data={data} />
        </div>
      </TabsContent>

      {hasResponse && (
        <TabsContent value="response">
          <McpResponseTab data={data} />
        </TabsContent>
      )}
    </Tabs>
  )
}

// ---- MCP Prompt Get Detail ----

function McpPromptGetDetail({ data }: { data: EventData }) {
  const hasResponse = data.success != null || data.error != null || data.response_preview != null || data.content_preview != null

  return (
    <Tabs defaultValue="request">
      <TabsList className="w-full">
        <TabsTrigger value="request">Request</TabsTrigger>
        <TabsTrigger value="response" disabled={!hasResponse}>Response</TabsTrigger>
      </TabsList>

      <TabsContent value="request" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Prompt" value={data.prompt_name as string} />
          <ServerField data={data} />
        </div>
        {data.arguments && <JsonBlock data={data.arguments as unknown} />}
      </TabsContent>

      {hasResponse && (
        <TabsContent value="response">
          <McpResponseTab data={data} />
        </TabsContent>
      )}
    </Tabs>
  )
}

// ---- MCP Elicitation Detail ----

function McpElicitationDetail({ data }: { data: EventData }) {
  const hasResponse = data.action != null

  return (
    <Tabs defaultValue="request">
      <TabsList className="w-full">
        <TabsTrigger value="request">Request</TabsTrigger>
        <TabsTrigger value="response" disabled={!hasResponse}>Response</TabsTrigger>
      </TabsList>

      <TabsContent value="request" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <ServerField data={data} />
        </div>
        {data.message && (
          <pre className="p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[200px] overflow-auto">
            {data.message as string}
          </pre>
        )}
        {data.schema && <JsonBlock data={data.schema as unknown} />}
      </TabsContent>

      {hasResponse && (
        <TabsContent value="response" className="space-y-2">
          <div className="grid grid-cols-2 gap-2 text-xs">
            <Field label="Action" value={data.action as string} />
            {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
          </div>
          {data.content && <JsonBlock data={data.content as unknown} />}
        </TabsContent>
      )}
    </Tabs>
  )
}

// ---- MCP Sampling Detail ----

function McpSamplingDetail({ data }: { data: EventData }) {
  const hasResponse = data.action != null

  return (
    <Tabs defaultValue="request">
      <TabsList className="w-full">
        <TabsTrigger value="request">Request</TabsTrigger>
        <TabsTrigger value="response" disabled={!hasResponse}>Response</TabsTrigger>
      </TabsList>

      <TabsContent value="request" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <ServerField data={data} />
          {data.message_count != null && <Field label="Messages" value={String(data.message_count)} />}
          {data.model_hint && <Field label="Model Hint" value={data.model_hint as string} />}
          {data.max_tokens != null && <Field label="Max Tokens" value={String(data.max_tokens)} />}
        </div>
      </TabsContent>

      {hasResponse && (
        <TabsContent value="response" className="space-y-2">
          <div className="grid grid-cols-2 gap-2 text-xs">
            <Field label="Action" value={data.action as string} />
            {data.model_used && <Field label="Model Used" value={data.model_used as string} />}
            {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
          </div>
          {data.content_preview && (
            <pre className="p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[200px] overflow-auto">
              {data.content_preview as string}
            </pre>
          )}
        </TabsContent>
      )}
    </Tabs>
  )
}

// ---- Guardrail Detail ----

function GuardrailDetail({ data }: { data: EventData }) {
  const categories = data.flagged_categories as Array<Record<string, unknown>> | undefined
  const hasResult = data.result != null

  return (
    <Tabs defaultValue="scan">
      <TabsList className="w-full">
        <TabsTrigger value="scan">Scan</TabsTrigger>
        <TabsTrigger value="result" disabled={!hasResult}>Result</TabsTrigger>
      </TabsList>

      <TabsContent value="scan" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          {data.direction && <Field label="Direction" value={data.direction as string} />}
          {data.models_used && <Field label="Models" value={(data.models_used as string[]).join(', ')} />}
        </div>
        {data.text_preview && (
          <div className="text-xs">
            <span className="text-muted-foreground font-medium">Input:</span>
            <pre className="mt-1 p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[200px] overflow-auto">
              {data.text_preview as string}
            </pre>
          </div>
        )}
      </TabsContent>

      {hasResult && (
        <TabsContent value="result" className="space-y-2">
          <div className="grid grid-cols-2 gap-2 text-xs">
            <Field label="Result" value={data.result as string} />
            {data.action_taken && <Field label="Action" value={data.action_taken as string} />}
            {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
          </div>
          {categories && categories.length > 0 && (
            <div className="text-xs space-y-1">
              <span className="text-muted-foreground font-medium">Flagged Categories:</span>
              {categories.map((cat, i) => (
                <div key={i} className="flex items-center gap-2 pl-2">
                  <Badge variant="outline" className="text-[10px]">{cat.category as string}</Badge>
                  <span className="text-muted-foreground">confidence: {((cat.confidence as number) * 100).toFixed(1)}%</span>
                  <span className="text-muted-foreground">action: {cat.action as string}</span>
                </div>
              ))}
            </div>
          )}
        </TabsContent>
      )}
    </Tabs>
  )
}

// ---- Secret Scan Detail ----

function SecretScanDetail({ data }: { data: EventData }) {
  const hasResult = data.findings_count != null

  return (
    <Tabs defaultValue="scan">
      <TabsList className="w-full">
        <TabsTrigger value="scan">Scan</TabsTrigger>
        <TabsTrigger value="result" disabled={!hasResult}>Result</TabsTrigger>
      </TabsList>

      <TabsContent value="scan" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          {data.rules_count != null && <Field label="Rules" value={String(data.rules_count)} />}
        </div>
        {data.text_preview && (
          <div className="text-xs">
            <span className="text-muted-foreground font-medium">Input:</span>
            <pre className="mt-1 p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[200px] overflow-auto">
              {data.text_preview as string}
            </pre>
          </div>
        )}
      </TabsContent>

      {hasResult && (
        <TabsContent value="result" className="space-y-2">
          <div className="grid grid-cols-2 gap-2 text-xs">
            <Field label="Findings" value={String(data.findings_count)} />
            {data.action_taken && <Field label="Action" value={data.action_taken as string} />}
            {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
          </div>
          {data.findings && <JsonBlock data={data.findings as unknown} />}
        </TabsContent>
      )}
    </Tabs>
  )
}

// ---- Routing Detail ----

function RoutingDetail({ data }: { data: EventData }) {
  const hasResult = data.selected_tier != null || data.win_rate != null || data.final_model != null

  return (
    <Tabs defaultValue="request">
      <TabsList className="w-full">
        <TabsTrigger value="request">Request</TabsTrigger>
        <TabsTrigger value="result" disabled={!hasResult}>Result</TabsTrigger>
      </TabsList>

      <TabsContent value="request" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          {data.routing_type && <Field label="Type" value={data.routing_type as string} />}
          {data.original_model && <Field label="Original Model" value={data.original_model as string} />}
          {data.threshold != null && <Field label="Threshold" value={String(data.threshold)} />}
        </div>
      </TabsContent>

      {hasResult && (
        <TabsContent value="result" className="space-y-2">
          <div className="grid grid-cols-2 gap-2 text-xs">
            {data.selected_tier && <Field label="Tier" value={data.selected_tier as string} />}
            {data.win_rate != null && <Field label="Win Rate" value={((data.win_rate as number) * 100).toFixed(1) + '%'} />}
            {data.routed_model && <Field label="Routed Model" value={data.routed_model as string} />}
            {data.final_model && <Field label="Final Model" value={data.final_model as string} />}
            {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
            {data.firewall_action && <Field label="Firewall" value={data.firewall_action as string} />}
            {data.candidate_models && <Field label="Candidates" value={(data.candidate_models as string[]).join(', ')} />}
          </div>
        </TabsContent>
      )}
    </Tabs>
  )
}

// ---- Error/Message Event Details ----

function AuthErrorDetail({ data }: { data: EventData }) {
  return (
    <Tabs defaultValue="overview">
      <TabsList className="w-full">
        <TabsTrigger value="overview">Overview</TabsTrigger>
        <TabsTrigger value="message" disabled={!data.message}>Message</TabsTrigger>
      </TabsList>
      <TabsContent value="overview" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Error Type" value={data.error_type as string} />
          <Field label="Status Code" value={String(data.status_code)} />
          <Field label="Endpoint" value={data.endpoint as string} />
          {data.reason && <Field label="Reason" value={data.reason as string} />}
        </div>
      </TabsContent>
      {data.message && (
        <TabsContent value="message">
          <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
            {data.message as string}
          </pre>
        </TabsContent>
      )}
    </Tabs>
  )
}

function RateLimitDetail({ data }: { data: EventData }) {
  return (
    <Tabs defaultValue="overview">
      <TabsList className="w-full">
        <TabsTrigger value="overview">Overview</TabsTrigger>
        <TabsTrigger value="message" disabled={!data.message}>Message</TabsTrigger>
      </TabsList>
      <TabsContent value="overview" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Reason" value={data.reason as string} />
          <Field label="Status Code" value={String(data.status_code)} />
          <Field label="Endpoint" value={data.endpoint as string} />
          {data.retry_after_secs != null && <Field label="Retry After" value={`${data.retry_after_secs}s`} />}
        </div>
      </TabsContent>
      {data.message && (
        <TabsContent value="message">
          <pre className="p-2 bg-amber-500/10 rounded text-xs whitespace-pre-wrap text-amber-700 dark:text-amber-400">
            {data.message as string}
          </pre>
        </TabsContent>
      )}
    </Tabs>
  )
}

function ValidationErrorDetail({ data }: { data: EventData }) {
  return (
    <Tabs defaultValue="overview">
      <TabsList className="w-full">
        <TabsTrigger value="overview">Overview</TabsTrigger>
        <TabsTrigger value="message" disabled={!data.message}>Message</TabsTrigger>
      </TabsList>
      <TabsContent value="overview" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Endpoint" value={data.endpoint as string} />
          <Field label="Status Code" value={String(data.status_code)} />
          {data.field && <Field label="Field" value={data.field as string} />}
        </div>
      </TabsContent>
      {data.message && (
        <TabsContent value="message">
          <pre className="p-2 bg-yellow-500/10 rounded text-xs whitespace-pre-wrap text-yellow-700 dark:text-yellow-400">
            {data.message as string}
          </pre>
        </TabsContent>
      )}
    </Tabs>
  )
}

function McpServerEventDetail({ data }: { data: EventData }) {
  return (
    <Tabs defaultValue="overview">
      <TabsList className="w-full">
        <TabsTrigger value="overview">Overview</TabsTrigger>
        <TabsTrigger value="message" disabled={!data.message}>Message</TabsTrigger>
      </TabsList>
      <TabsContent value="overview" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <div className="flex items-center gap-1 text-xs">
            <Server className="h-3 w-3 text-muted-foreground" />
            <span className="text-muted-foreground">Server:</span>
            <span>{(data.server_name || data.server_id) as string}</span>
          </div>
          <Field label="Action" value={data.action as string} />
        </div>
      </TabsContent>
      {data.message && (
        <TabsContent value="message">
          <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
            {data.message as string}
          </pre>
        </TabsContent>
      )}
    </Tabs>
  )
}

function OAuthEventDetail({ data }: { data: EventData }) {
  return (
    <Tabs defaultValue="overview">
      <TabsList className="w-full">
        <TabsTrigger value="overview">Overview</TabsTrigger>
        <TabsTrigger value="message" disabled={!data.message}>Message</TabsTrigger>
      </TabsList>
      <TabsContent value="overview" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Action" value={data.action as string} />
          <Field label="Status Code" value={String(data.status_code)} />
          {data.client_id_hint && <Field label="Client" value={data.client_id_hint as string} />}
        </div>
      </TabsContent>
      {data.message && (
        <TabsContent value="message">
          <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
            {data.message as string}
          </pre>
        </TabsContent>
      )}
    </Tabs>
  )
}

function InternalErrorDetail({ data }: { data: EventData }) {
  return (
    <Tabs defaultValue="overview">
      <TabsList className="w-full">
        <TabsTrigger value="overview">Overview</TabsTrigger>
        <TabsTrigger value="message" disabled={!data.message}>Message</TabsTrigger>
      </TabsList>
      <TabsContent value="overview" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Error Type" value={data.error_type as string} />
          <Field label="Status Code" value={String(data.status_code)} />
        </div>
      </TabsContent>
      {data.message && (
        <TabsContent value="message">
          <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
            {data.message as string}
          </pre>
        </TabsContent>
      )}
    </Tabs>
  )
}

function ModerationEventDetail({ data }: { data: EventData }) {
  return (
    <Tabs defaultValue="overview">
      <TabsList className="w-full">
        <TabsTrigger value="overview">Overview</TabsTrigger>
        <TabsTrigger value="message" disabled={!data.message}>Message</TabsTrigger>
      </TabsList>
      <TabsContent value="overview" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Reason" value={data.reason as string} />
          <Field label="Status Code" value={String(data.status_code)} />
        </div>
      </TabsContent>
      {data.message && (
        <TabsContent value="message">
          <pre className="p-2 bg-orange-500/10 rounded text-xs whitespace-pre-wrap text-orange-700 dark:text-orange-400">
            {data.message as string}
          </pre>
        </TabsContent>
      )}
    </Tabs>
  )
}

function ConnectionErrorDetail({ data }: { data: EventData }) {
  return (
    <Tabs defaultValue="overview">
      <TabsList className="w-full">
        <TabsTrigger value="overview">Overview</TabsTrigger>
        <TabsTrigger value="message" disabled={!data.message}>Message</TabsTrigger>
      </TabsList>
      <TabsContent value="overview" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Transport" value={data.transport as string} />
          <Field label="Action" value={data.action as string} />
        </div>
      </TabsContent>
      {data.message && (
        <TabsContent value="message">
          <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
            {data.message as string}
          </pre>
        </TabsContent>
      )}
    </Tabs>
  )
}

// ---- Simple field-only events ----

function PromptCompressionDetail({ data }: { data: EventData }) {
  return (
    <div className="grid grid-cols-2 gap-2 text-xs">
      <Field label="Method" value={data.method as string} />
      <Field label="Reduction" value={`${((data.reduction_percent as number) ?? 0).toFixed(1)}%`} />
      <Field label="Original Tokens" value={String(data.original_tokens)} />
      <Field label="Compressed Tokens" value={String(data.compressed_tokens)} />
      <Field label="Duration" value={`${data.duration_ms}ms`} />
    </div>
  )
}

function MemoryCompactionDetail({ data }: { data: EventData }) {
  const hasResponse = data.summary_bytes != null || data.response_body != null || data.content_preview != null
  const hasError = data.error != null
  const requestBody = data.request_body as Record<string, unknown> | undefined
  const messages = requestBody?.messages as Array<Record<string, unknown>> | undefined

  return (
    <Tabs defaultValue="request">
      <TabsList className="w-full">
        <TabsTrigger value="request">Request</TabsTrigger>
        <TabsTrigger value="response" disabled={!hasResponse}>Response</TabsTrigger>
        <TabsTrigger value="error" disabled={!hasError}>Error</TabsTrigger>
      </TabsList>

      <TabsContent value="request" className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Session" value={data.session_id as string} />
          <Field label="Model" value={data.model as string} />
          <Field label="Transcript Size" value={`${data.transcript_bytes} bytes`} />
        </div>

        {data.transcript_path && (
          <ArchiveFileField
            label="Transcript"
            path={data.transcript_path as string}
          />
        )}

        <Tabs defaultValue={messages && messages.length > 0 ? 'messages' : 'body'}>
          <TabsList className={SUB_TABS_LIST}>
            {messages && messages.length > 0 && (
              <TabsTrigger value="messages" className={SUB_TAB}>
                Messages ({messages.length})
              </TabsTrigger>
            )}
            <TabsTrigger value="body" className={SUB_TAB}>Full Body</TabsTrigger>
          </TabsList>

          {messages && messages.length > 0 && (
            <TabsContent value="messages">
              <div className="space-y-1.5">
                {messages.map((msg, i) => (
                  <MessageItem key={i} message={msg} />
                ))}
              </div>
            </TabsContent>
          )}

          <TabsContent value="body">
            <JsonBlock data={requestBody} />
          </TabsContent>
        </Tabs>
      </TabsContent>

      {hasResponse && (
        <TabsContent value="response" className="space-y-2">
          <CompactionResponseContent data={data} />
        </TabsContent>
      )}

      {hasError && (
        <TabsContent value="error">
          <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
            {data.error as string}
          </pre>
        </TabsContent>
      )}
    </Tabs>
  )
}

function CompactionResponseContent({ data }: { data: EventData }) {
  const summaryBytes = data.summary_bytes as number | undefined
  const ratio = data.compression_ratio as number | undefined
  const responseBody = data.response_body as Record<string, unknown> | undefined

  return (
    <div className="space-y-2">
      <table className="w-full text-xs">
        <tbody>
          {(data.input_tokens != null || data.output_tokens != null) && (
            <tr className="border-b border-border/30">
              <td className="text-muted-foreground py-0.5 pr-2 whitespace-nowrap">Input</td>
              <td className="py-0.5 font-medium">{String(data.input_tokens ?? 0)}</td>
              <td className="text-muted-foreground py-0.5 pr-2 pl-4 whitespace-nowrap">Output</td>
              <td className="py-0.5 font-medium">{String(data.output_tokens ?? 0)}</td>
              {summaryBytes != null && (
                <>
                  <td className="text-muted-foreground py-0.5 pr-2 pl-4 whitespace-nowrap">Summary</td>
                  <td className="py-0.5 font-medium">{summaryBytes} bytes</td>
                </>
              )}
            </tr>
          )}
          {(data.reasoning_tokens != null && (data.reasoning_tokens as number) > 0) && (
            <tr className="border-b border-border/30">
              <td className="text-muted-foreground py-0.5 pr-2 whitespace-nowrap">Reasoning</td>
              <td className="py-0.5 font-medium" colSpan={5}>{String(data.reasoning_tokens)}</td>
            </tr>
          )}
          {(ratio != null || data.finish_reason) && (
            <tr>
              {ratio != null && (
                <>
                  <td className="text-muted-foreground py-0.5 pr-2 whitespace-nowrap">Compression</td>
                  <td className="py-0.5 font-medium">{ratio.toFixed(1)}%</td>
                </>
              )}
              {data.finish_reason && (
                <>
                  <td className="text-muted-foreground py-0.5 pr-2 pl-4 whitespace-nowrap">Finish</td>
                  <td className="py-0.5 font-medium">{data.finish_reason as string}</td>
                </>
              )}
            </tr>
          )}
        </tbody>
      </table>

      {data.summary_path && (
        <div className="text-xs">
          <span className="text-muted-foreground">Summary: </span>
          <code className="font-mono text-[11px]">{data.summary_path as string}</code>
        </div>
      )}

      <Tabs defaultValue={data.content_preview ? 'content' : 'body'}>
        <TabsList className={SUB_TABS_LIST}>
          {data.content_preview && (
            <TabsTrigger value="content" className={SUB_TAB}>Content</TabsTrigger>
          )}
          {responseBody && (
            <TabsTrigger value="body" className={SUB_TAB}>Full Body</TabsTrigger>
          )}
        </TabsList>

        {data.content_preview && (
          <TabsContent value="content">
            <pre className="text-xs whitespace-pre-wrap font-mono bg-muted/50 p-2 rounded max-h-64 overflow-y-auto">
              {data.content_preview as string}
            </pre>
          </TabsContent>
        )}

        {responseBody && (
          <TabsContent value="body">
            <JsonBlock data={responseBody} />
          </TabsContent>
        )}
      </Tabs>
    </div>
  )
}

/** Displays an archive file path with an inline "Read" button that fetches and shows content. */
function ArchiveFileField({ label, path }: { label: string; path: string }) {
  const [content, setContent] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [expanded, setExpanded] = useState(false)

  // Extract client_id and filename from relative path like "{client_id}/archive/{filename}"
  const parts = path.split('/')
  const clientId = parts[0] || ''
  const filename = parts[parts.length - 1] || ''

  const handleRead = useCallback(async () => {
    if (content !== null) {
      setExpanded(!expanded)
      return
    }
    setLoading(true)
    setError(null)
    try {
      const result = await invoke<string>('read_memory_archive_file', {
        clientId,
        filename,
      } satisfies ReadMemoryArchiveFileParams)
      setContent(result)
      setExpanded(true)
    } catch (e) {
      setError(String(e))
    } finally {
      setLoading(false)
    }
  }, [clientId, filename, content, expanded])

  return (
    <div className="space-y-1">
      <div className="flex items-center gap-2 text-xs">
        <span className="text-muted-foreground">{label}:</span>
        <code className="font-mono text-[11px] truncate flex-1">{path}</code>
        <Button
          variant="ghost"
          size="sm"
          className="h-5 px-1.5 text-[10px]"
          onClick={handleRead}
          disabled={loading}
        >
          <FileText className="h-3 w-3 mr-1" />
          {loading ? '...' : expanded ? 'Hide' : 'Read'}
        </Button>
      </div>
      {error && (
        <div className="text-[10px] text-destructive">{error}</div>
      )}
      {expanded && content !== null && (
        <pre className="text-xs whitespace-pre-wrap font-mono bg-muted/50 p-2 rounded max-h-64 overflow-y-auto">
          {content}
        </pre>
      )}
    </div>
  )
}

function FirewallDecisionDetail({ data }: { data: EventData }) {
  return (
    <div className="grid grid-cols-2 gap-2 text-xs">
      <Field label="Type" value={data.firewall_type as string} />
      <Field label="Item" value={data.item_name as string} />
      <Field label="Action" value={data.action as string} />
      {data.duration && <Field label="Duration" value={data.duration as string} />}
    </div>
  )
}

function SseConnectionDetail({ data }: { data: EventData }) {
  return (
    <div className="grid grid-cols-2 gap-2 text-xs">
      <Field label="Session" value={data.session_id as string} />
      <Field label="Action" value={data.action as string} />
    </div>
  )
}
