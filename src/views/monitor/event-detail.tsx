import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { McpToolDisplay, type McpToolDisplayItem } from '@/components/shared/McpToolDisplay'
import { JsonTree } from '@/components/shared/JsonTree'
import { cn } from '@/lib/utils'
import { User, Server, Copy, Check, FileText, AlertTriangle, ChevronRight, ArrowRight, ArrowUpRight, ArrowDownLeft, Loader2, Shuffle } from 'lucide-react'
import { EventDuration } from './event-duration'
import { useState, useCallback, useRef, type ReactNode, type KeyboardEvent } from 'react'
import { invoke } from '@tauri-apps/api/core'
import ReactMarkdown from 'react-markdown'
import { markdownLinkComponents } from '@/components/shared/MarkdownLink'
import remarkGfm from 'remark-gfm'
import { capturedExcerpt, capturedRequestBody, capturedResponseMessages, contentText, requestMessages } from './message-content'
import { LLM_API_LABELS, llmApiFlow, type ApiFlow } from './llm-api'
import type { EventStatus, LlmProtocol, MonitorEvent, ReadMemoryArchiveFileParams } from '@/types/tauri-commands'
import type { SystemOneAnswer, SystemOneQuestion } from '@/types/systemone'
import { SystemOneAnswerView, SystemOneQuestionView } from '@/components/shared/SystemOneAnswers'

const MARKDOWN_STYLES =
  'text-xs leading-relaxed break-words [&_p]:my-1 [&_ul]:list-disc [&_ul]:ml-4 [&_ol]:list-decimal [&_ol]:ml-4 [&_li]:my-0.5 ' +
  '[&_code]:bg-muted [&_code]:px-1 [&_code]:py-0.5 [&_code]:rounded [&_code]:text-[11px] ' +
  '[&_pre]:bg-muted [&_pre]:p-2 [&_pre]:rounded [&_pre]:overflow-auto [&_pre>code]:bg-transparent [&_pre>code]:p-0 ' +
  '[&_h1]:font-bold [&_h1]:text-sm [&_h2]:font-bold [&_h3]:font-semibold [&_h1]:mt-2 [&_h2]:mt-2 [&_h3]:mt-1.5 ' +
  '[&_a]:text-primary [&_a]:underline [&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-2 [&_blockquote]:text-muted-foreground ' +
  '[&_table]:border-collapse [&_th]:border [&_th]:border-border [&_th]:px-1.5 [&_td]:border [&_td]:border-border [&_td]:px-1.5'

/** Show a raw text payload (e.g. the exact wire bytes) with a copy button. */
function RawBlock({ text, label }: { text: string | undefined; label?: string }) {
  const [copied, setCopied] = useState(false)
  if (!text) {
    return <p className="text-xs text-muted-foreground italic">Not captured for this event.</p>
  }
  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => {
          navigator.clipboard.writeText(text).then(() => {
            setCopied(true)
            setTimeout(() => setCopied(false), 1500)
          })
        }}
        title={label ? `Copy ${label}` : 'Copy raw'}
        className="absolute right-1 top-1 z-10 flex items-center gap-1 rounded border border-border/60 bg-background/80 px-1 text-[10px] text-muted-foreground hover:text-foreground"
      >
        {copied ? <Check className="h-3 w-3 text-green-500" /> : <Copy className="h-3 w-3" />}
        {copied ? 'Copied' : label ? `Copy ${label}` : 'Copy'}
      </button>
      <pre className="p-2 bg-muted rounded text-[11px] font-mono whitespace-pre-wrap break-all max-h-[480px] overflow-auto">
        {text}
      </pre>
    </div>
  )
}

/** Extract reasoning ("thinking") text from an Anthropic response body. */
function extractReasoning(responseBody: unknown): string | null {
  if (!responseBody || typeof responseBody !== 'object') return null
  const content = (responseBody as Record<string, unknown>).content
  if (!Array.isArray(content)) return null
  const parts = (content as Array<Record<string, unknown>>)
    .filter((b) => b.type === 'thinking' || b.type === 'redacted_thinking')
    .map((b) => (b.thinking as string) || (b.text as string) || '')
    .filter(Boolean)
  return parts.length > 0 ? parts.join('\n') : null
}

/** Auto-detect the format of a text blob so we can render it appropriately. */
function detectFormat(text: string): 'json' | 'markdown' | 'text' {
  const t = text.trim()
  if ((t.startsWith('{') && t.endsWith('}')) || (t.startsWith('[') && t.endsWith(']'))) {
    try {
      JSON.parse(t)
      return 'json'
    } catch { /* not json */ }
  }
  // Markdown markers: headings, fences, lists, links, blockquotes, emphasis, inline code, tables.
  if (/(^|\n)#{1,6}\s|```|(^|\n)\s*[-*+]\s|(^|\n)\s*\d+\.\s|\[[^\]]+\]\([^)]+\)|(^|\n)>\s|\*\*[^*]+\*\*|`[^`]+`|(^|\n)\|.+\|/.test(text)) {
    return 'markdown'
  }
  return 'text'
}

/**
 * Renders a text blob with format auto-detection (JSON / Markdown / plain) and,
 * for JSON and Markdown, a toggle to view the raw source.
 */
function SmartText({ text }: { text: string }) {
  const format = detectFormat(text)
  const [raw, setRaw] = useState(false)

  if (format === 'text') {
    return <div className="whitespace-pre-wrap break-words leading-relaxed">{text}</div>
  }

  return (
    <div className="space-y-1">
      <div className="flex items-center justify-end gap-1">
        <span className="text-[9px] uppercase tracking-wide text-muted-foreground/60">{format}</span>
        <button
          type="button"
          onClick={() => setRaw((r) => !r)}
          className="text-[10px] text-muted-foreground hover:text-foreground rounded border border-border/50 px-1"
        >
          {raw ? 'Rendered' : 'Raw'}
        </button>
      </div>
      {raw ? (
        <pre className="whitespace-pre-wrap font-mono text-[11px]">
          {format === 'json' ? formatJsonString(text) : text}
        </pre>
      ) : format === 'json' ? (
        <JsonTree data={JSON.parse(text.trim())} />
      ) : (
        <div className={MARKDOWN_STYLES}>
          <ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownLinkComponents}>{text}</ReactMarkdown>
        </div>
      )}
    </div>
  )
}

interface EventDetailProps {
  event: MonitorEvent | null
  loading?: boolean
  error?: string | null
  onRetry?: () => void
  /** Controls rendered at the right of the header (dock, close). */
  toolbar?: ReactNode
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type EventData = Record<string, any>

interface DetailTab {
  id: string
  label: string
  badge?: string | number
  content: ReactNode
}

/** The last tab the user picked; reopened for the next event when it has it. */
let preferredTab = 'exchange'

const STATUS_STYLES: Record<EventStatus, string> = {
  pending: 'bg-amber-500/15 text-amber-700 dark:text-amber-300',
  complete: 'bg-emerald-500/15 text-emerald-700 dark:text-emerald-300',
  error: 'bg-destructive/15 text-destructive',
}

function StatusPill({ status }: { status: EventStatus }) {
  return (
    <span className={cn('inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-medium', STATUS_STYLES[status])}>
      {status === 'pending' && <Loader2 className="h-3 w-3 animate-spin" />}
      {status === 'pending' ? 'In progress' : status === 'complete' ? 'Complete' : 'Error'}
    </span>
  )
}

/** "Responses → Anthropic Messages · translated" */
export function ApiFlowBadge({ flow }: { flow: ApiFlow }) {
  if (!flow.client) return null
  const title = flow.translated
    ? `The client called the ${LLM_API_LABELS[flow.client]} API; LocalRouter translated it to ${LLM_API_LABELS[flow.upstream!]} for the provider.`
    : flow.upstream
      ? `Sent upstream as ${LLM_API_LABELS[flow.upstream]} without translation.`
      : `The client called the ${LLM_API_LABELS[flow.client]} API.`
  return (
    <span data-testid="api-flow" title={title} className="inline-flex items-center gap-1 rounded-md border bg-background px-1.5 py-0.5 text-[11px]">
      {LLM_API_LABELS[flow.client]}
      {flow.upstream && flow.translated && (
        <>
          <ArrowRight className="h-3 w-3 text-muted-foreground" />
          {LLM_API_LABELS[flow.upstream]}
          <span className="ml-0.5 rounded bg-amber-500/15 px-1 text-[10px] font-medium text-amber-700 dark:text-amber-300">translated</span>
        </>
      )}
      {flow.upstream && !flow.translated && <span className="ml-0.5 text-[10px] text-muted-foreground">native</span>}
    </span>
  )
}

function DetailTabs({ tabs, active, onChange }: { tabs: DetailTab[]; active: string; onChange: (id: string) => void }) {
  const refs = useRef<Record<string, HTMLButtonElement | null>>({})
  const onKeyDown = (event: KeyboardEvent) => {
    const index = tabs.findIndex(tab => tab.id === active)
    const next = event.key === 'ArrowRight' ? index + 1 : event.key === 'ArrowLeft' ? index - 1 : event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : null
    if (next === null) return
    event.preventDefault()
    const tab = tabs[(next + tabs.length) % tabs.length]
    onChange(tab.id)
    refs.current[tab.id]?.focus()
  }
  return (
    <div role="tablist" aria-label="Event details" className="flex gap-1 overflow-x-auto px-2" onKeyDown={onKeyDown}>
      {tabs.map(tab => (
        <button
          key={tab.id}
          ref={el => { refs.current[tab.id] = el }}
          type="button"
          role="tab"
          id={`event-tab-${tab.id}`}
          aria-selected={tab.id === active}
          aria-controls="event-tab-panel"
          tabIndex={tab.id === active ? 0 : -1}
          onClick={() => onChange(tab.id)}
          className={cn(
            '-mb-px flex shrink-0 items-center gap-1.5 border-b-2 px-2.5 py-1.5 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
            tab.id === active ? 'border-primary text-foreground' : 'border-transparent text-muted-foreground hover:text-foreground',
          )}
        >
          {tab.label}
          {tab.badge != null && <span className="rounded-full bg-muted px-1.5 text-[10px] tabular-nums text-muted-foreground">{tab.badge}</span>}
        </button>
      ))}
    </div>
  )
}

export function EventDetail({ event, loading, error, onRetry, toolbar }: EventDetailProps) {
  const [copied, setCopied] = useState(false)
  const [tab, setTab] = useState(preferredTab)
  const handleCopyEvent = useCallback(() => {
    navigator.clipboard.writeText(JSON.stringify(event, null, 2)).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    })
  }, [event])

  if (!event) {
    return (
      <div className="flex h-full flex-col">
        {toolbar && <div className="flex shrink-0 justify-end border-b px-2 py-1">{toolbar}</div>}
        <div className="flex flex-1 flex-col gap-3 items-center justify-center text-muted-foreground text-sm" role="status">
          {loading ? <><Loader2 className="h-4 w-4 animate-spin" />Loading event details…</> : error ?? 'Select an event to view details'}
          {!loading && error && <Button variant="outline" size="sm" onClick={onRetry}>Retry</Button>}
        </div>
      </div>
    )
  }

  const data = event.data as EventData
  const type = data.type as string
  const tabs = eventTabs(event)
  const active = tabs.find(t => t.id === tab) ?? tabs[0]
  const title = String(data.model || data.tool_name || data.prompt_name || data.uri || type.replace(/_/g, ' '))

  return (
    <div className="flex h-full flex-col min-h-0">
      {error && <div className="flex items-center gap-3 px-4 py-2 text-sm text-destructive" role="status">{error}<Button variant="outline" size="sm" onClick={onRetry}>Retry</Button></div>}
      <div className="shrink-0 border-b bg-muted/20">
        <div className="flex flex-wrap items-center gap-2 px-3 pt-2 pb-1.5">
          <StatusPill status={event.status} />
          <span className="text-sm font-semibold truncate max-w-[280px]" title={title}>{title}</span>
          {type === 'llm_call' && <ApiFlowBadge flow={llmApiFlow(data)} />}
          {(event.client_name || event.client_id) && (
            <span className="text-xs text-muted-foreground flex items-center gap-1">
              <User className="h-3 w-3" />{event.client_name || event.client_id}
            </span>
          )}
          <EventDuration event={event} showClock />
          <div className="ml-auto flex items-center gap-1">
            <Button
              variant="ghost"
              size="sm"
              className="h-7 gap-1.5 px-2 text-xs text-muted-foreground"
              onClick={handleCopyEvent}
              title="Copy the entire event (request + response) as JSON"
            >
              {copied ? <Check className="h-3 w-3 text-green-500" /> : <Copy className="h-3 w-3" />}
              {copied ? 'Copied' : 'Copy event'}
            </Button>
            {toolbar}
          </div>
        </div>

        {type === 'llm_call' && data.duplicate_hop != null && (
          <div className="mx-3 mb-2 flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 px-2.5 py-2 text-xs text-amber-700 dark:text-amber-300">
            <AlertTriangle className="h-3.5 w-3.5 mt-0.5 shrink-0" />
            <div className="space-y-0.5">
              <p className="font-medium">
                Duplicate hop {data.duplicate_hop} — this request already passed through LocalRouter
              </p>
              <p className="text-amber-700/80 dark:text-amber-300/80">
                It was forwarded unmodified (no guardrails, compression, JSON repair or prompts) and is
                <strong> not counted</strong> in usage stats or rate limits. Search the Monitor for trace{' '}
                <code className="font-mono break-all">{String(data.trace_id ?? '')}</code> to see every hop.
              </p>
            </div>
          </div>
        )}

        <DetailTabs tabs={tabs} active={active.id} onChange={id => { preferredTab = id; setTab(id) }} />
      </div>

      <div
        id="event-tab-panel"
        role="tabpanel"
        aria-labelledby={`event-tab-${active.id}`}
        className="flex-1 min-h-0 overflow-auto p-4 space-y-4 [container-type:inline-size]"
        data-testid="event-detail-scroll"
      >
        {active.content}
      </div>
    </div>
  )
}

// ---- Tabs per event type ----

const EXCHANGE_TYPES = new Set([
  'mcp_tool_call', 'mcp_resource_read', 'mcp_prompt_get', 'mcp_elicitation', 'mcp_sampling',
  'guardrail_scan', 'guardrail_response_scan', 'secret_scan', 'route_llm_classify', 'routing_decision',
  'memory_compaction',
])

function eventTabs(event: MonitorEvent): DetailTab[] {
  const data = event.data as EventData
  const type = data.type as string
  if (type === 'llm_call') return llmTabs(event, data)
  const primary = EXCHANGE_TYPES.has(type)
    ? { id: 'exchange', label: 'Request & response' }
    : { id: 'overview', label: 'Overview' }
  return [
    { ...primary, content: <TypeDetail data={data} status={event.status} /> },
    { id: 'details', label: 'Details', content: <EventMetadata event={event} /> },
    { id: 'raw', label: 'Raw', content: <RawSection title="Full event"><JsonTree data={event} label="event" /></RawSection> },
  ]
}

function TypeDetail({ data, status }: { data: EventData; status: EventStatus }) {
  switch (data.type as string) {
    case 'mcp_tool_call': return <McpToolCallDetail data={data} />
    case 'mcp_resource_read': return <McpResourceReadDetail data={data} />
    case 'mcp_prompt_get': return <McpPromptGetDetail data={data} />
    case 'mcp_elicitation': return <McpElicitationDetail data={data} />
    case 'mcp_sampling': return <McpSamplingDetail data={data} />
    case 'guardrail_scan':
    case 'guardrail_response_scan': return <GuardrailDetail data={data} />
    case 'secret_scan': return <SecretScanDetail data={data} />
    case 'route_llm_classify':
    case 'routing_decision': return <RoutingDetail data={data} />
    case 'auth_error':
    case 'access_denied': return <AuthErrorDetail data={data} />
    case 'rate_limit_event': return <RateLimitDetail data={data} />
    case 'validation_error': return <ValidationErrorDetail data={data} />
    case 'mcp_server_event': return <McpServerEventDetail data={data} />
    case 'oauth_event': return <OAuthEventDetail data={data} />
    case 'internal_error': return <InternalErrorDetail data={data} />
    case 'moderation_event': return <ModerationEventDetail data={data} />
    case 'connection_error': return <ConnectionErrorDetail data={data} />
    case 'prompt_compression': return <PromptCompressionDetail data={data} />
    case 'json_repair': return <JsonRepairDetail data={data} />
    case 'memory_compaction': return <MemoryCompactionDetail data={data} status={status} />
    case 'firewall_decision': return <FirewallDecisionDetail data={data} />
    case 'sse_connection': return <SseConnectionDetail data={data} />
    case 'proxy_passthrough': return <ProxyPassthroughDetail data={data} />
    default: return <JsonTree data={data} />
  }
}

const TYPE_NAMES: Record<string, string> = { llm_call: 'LLM call', mcp_tool_call: 'MCP tool call', mcp_resource_read: 'MCP resource read', mcp_prompt_get: 'MCP prompt', mcp_elicitation: 'MCP elicitation', mcp_sampling: 'MCP sampling' }

function formatTimestamp(timestamp: string | number): string {
  const date = new Date(timestamp)
  if (Number.isNaN(date.getTime())) return String(timestamp)
  return `${date.toLocaleDateString()} ${date.toLocaleTimeString()}.${String(date.getMilliseconds()).padStart(3, '0')}`
}

function EventMetadata({ event }: { event: MonitorEvent }) {
  const type = event.event_type
  return (
    <Section title="Event">
      <Properties items={[
        ['Type', TYPE_NAMES[type] ?? type.replace(/_/g, ' ')],
        ['Status', <StatusPill key="s" status={event.status} />],
        ['Started', formatTimestamp(event.timestamp)],
        ['Duration', event.duration_ms != null || event.status === 'pending' ? <EventDuration key="d" event={event} /> : null],
        ['Client', event.client_name],
        ['Client ID', event.client_id && <Mono key="c">{event.client_id}</Mono>],
        ['Session', event.session_id && <Mono key="se">{event.session_id}</Mono>],
        ['Event ID', <Mono key="i">{event.id}</Mono>],
      ]} />
    </Section>
  )
}

// ---- Layout primitives for readable property sheets ----

function Section({ title, description, children }: { title: string; description?: ReactNode; children: ReactNode }) {
  return (
    <section aria-label={title} className="space-y-2">
      <div>
        <h3 className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">{title}</h3>
        {description && <p className="mt-0.5 text-xs text-muted-foreground">{description}</p>}
      </div>
      {children}
    </section>
  )
}

function Properties({ items }: { items: [string, ReactNode][] }) {
  const visible = items.filter(([, value]) => value !== null && value !== undefined && value !== '' && value !== false)
  if (visible.length === 0) return <p className="text-xs italic text-muted-foreground">Nothing recorded.</p>
  return (
    <dl className="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-x-6 gap-y-3 rounded-lg border bg-muted/10 p-3">
      {visible.map(([label, value]) => (
        <div key={label} className="min-w-0">
          <dt className="text-[11px] text-muted-foreground">{label}</dt>
          <dd className="mt-0.5 text-xs font-medium break-words">{value}</dd>
        </div>
      ))}
    </dl>
  )
}

function Mono({ children }: { children: ReactNode }) {
  return <code className="font-mono text-[11px] break-all">{children}</code>
}

function RawSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section aria-label={title} className="space-y-1.5">
      <h3 className="text-xs font-semibold">{title}</h3>
      {children}
    </section>
  )
}

const tokens = (value: unknown) => (typeof value === 'number' ? value.toLocaleString() : null)

// ---- Utility Functions ----

const ROLE_COLORS: Record<string, string> = {
  system: 'bg-purple-500/10 text-purple-700 dark:text-purple-400 border-purple-500/20',
  user: 'bg-blue-500/10 text-blue-700 dark:text-blue-400 border-blue-500/20',
  assistant: 'bg-green-500/10 text-green-700 dark:text-green-400 border-green-500/20',
  tool: 'bg-orange-500/10 text-orange-700 dark:text-orange-400 border-orange-500/20',
  developer: 'bg-purple-500/10 text-purple-700 dark:text-purple-400 border-purple-500/20',
}

function extractTextContent(content: unknown): string | null {
  return contentText(content) || null
}

function hasImageContent(content: unknown): boolean {
  return Array.isArray(content) && (content as Array<Record<string, unknown>>).some(
    p => ['image_url', 'image', 'input_image'].includes(p.type as string)
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
    <div className="rounded-md border text-xs overflow-hidden min-w-0">
      <div className={cn('flex items-center gap-2 px-2 py-1 border-b', ROLE_COLORS[role] || 'bg-muted')}>
        <span className="font-medium text-[11px]">{role}</span>
        {name && <span className="text-[10px] opacity-70 font-mono">{name}</span>}
        {toolCallId && <span className="text-[10px] opacity-70 font-mono truncate">← {toolCallId}</span>}
      </div>
      {contentText && (
        <div className="px-3 py-2.5">
          <SmartText text={contentText} />
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

/** A labelled value; renders nothing when the value was not recorded. */
function Field({ label, value }: { label: string; value: string | undefined }) {
  if (!value || value === 'undefined' || value === 'null') return null
  return (
    <div className="min-w-0 text-xs">
      <div className="text-[11px] text-muted-foreground">{label}</div>
      <div className="font-medium break-words">{value}</div>
    </div>
  )
}

function ServerField({ data }: { data: EventData }) {
  return (
    <div className="min-w-0 text-xs">
      <div className="flex items-center gap-1 text-[11px] text-muted-foreground"><Server className="h-3 w-3" />Server</div>
      <div className="font-medium break-words">{(data.server_name || data.server_id) as string}</div>
    </div>
  )
}

function JsonBlock({ data, label }: { data: unknown; label?: string }) {
  if (data === null || data === undefined) return null
  return <JsonTree data={data} label={label} className="max-h-[400px]" />
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
    <dl className="divide-y divide-border/40 rounded-md border text-xs">
      {entries.map(([key, value]) => (
        <div key={key} className="grid grid-cols-[minmax(90px,160px)_1fr] gap-3 px-2.5 py-1.5">
          <dt className="text-muted-foreground font-mono text-[11px] break-all">{key}</dt>
          <dd className="min-w-0 font-mono text-[11px] whitespace-pre-wrap break-words">
            {value !== null && typeof value === 'object' ? <JsonTree data={value} /> : String(value)}
          </dd>
        </div>
      ))}
    </dl>
  )
}

function McpResponseContent({ data }: { data: EventData }) {
  const raw = (data.response_preview || data.content_preview) as string | undefined
  return <div className="space-y-3">
    <ResponseError error={data.error} />
    {raw && <SmartText text={extractMcpContent(raw)} />}
    <div className="flex flex-wrap gap-x-6 gap-y-2">
      {data.success != null && <Field label="Success" value={data.success ? 'Yes' : 'No'} />}
      {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
    </div>
    {raw && <Disclosure title="Full response"><RawBlock text={formatJsonString(raw)} label="response" /></Disclosure>}
  </div>
}


// ---- System One (POST /v1/systemone) ----

/** Whether an llm_call event is a System One decision request rather than chat. */
function isSystemOneEvent(data: EventData): boolean {
  if ((data.protocol as LlmProtocol | undefined) === 'system_one') return true
  if (data.endpoint === '/v1/systemone' || data.endpoint === '/systemone') return true
  const body = data.request_body as Record<string, unknown> | undefined
  return body != null && typeof body.questions === 'object' && body.questions !== null && !('messages' in body)
}

function systemOneQuestions(body: Record<string, unknown> | undefined): Record<string, SystemOneQuestion> {
  const q = body?.questions
  return q && typeof q === 'object' && !Array.isArray(q) ? (q as Record<string, SystemOneQuestion>) : {}
}

/** Request view: the state being judged and every question asked about it. */
function SystemOneRequestView({ body }: { body: Record<string, unknown> }) {
  const state = body.state
  const questions = Object.entries(systemOneQuestions(body))
  return (
    <div className="space-y-2">
      <div className="space-y-1">
        <div className="text-[11px] font-medium text-muted-foreground">State</div>
        <div className="p-2 bg-muted rounded text-xs">
          {typeof state === 'string'
            ? <SmartText text={state} />
            : <JsonTree data={state} />}
        </div>
      </div>
      <div className="space-y-1">
        <div className="text-[11px] font-medium text-muted-foreground">Questions ({questions.length})</div>
        <div className="space-y-1.5">
          {questions.map(([id, q]) => <SystemOneQuestionView key={id} id={id} question={q} />)}
        </div>
      </div>
    </div>
  )
}

/** Response view: each answer with its probability bars. */
function SystemOneAnswersList({ data }: { data: EventData }) {
  const answers = (data.response_body as Record<string, unknown> | undefined)?.answers as
    Record<string, SystemOneAnswer> | undefined
  const questions = systemOneQuestions(data.request_body as Record<string, unknown> | undefined)
  if (!answers || Object.keys(answers).length === 0) {
    return <p className="text-xs text-muted-foreground italic">No answers in the response.</p>
  }
  return (
    <div className="space-y-2">
      {Object.entries(answers).map(([id, answer]) => (
        <SystemOneAnswerView key={id} id={id} answer={answer} question={questions[id]} />
      ))}
    </div>
  )
}

// ---- Exchange building blocks ----

function Disclosure({ title, description, children }: { title: string; description?: string; children: ReactNode }) {
  const [open, setOpen] = useState(false)
  return (
    <details onToggle={event => setOpen(event.currentTarget.open)} className="group/disclosure rounded-lg border bg-background text-xs">
      <summary className="flex min-h-9 cursor-pointer list-none items-center gap-2 px-3 py-1.5 hover:bg-muted/40 focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring [&::-webkit-details-marker]:hidden">
        <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform group-open/disclosure:rotate-90" />
        <span className="font-medium">{title}</span>
        {description && <span className="ml-auto truncate text-muted-foreground">{description}</span>}
      </summary>
      {open && <div className="border-t p-3 min-w-0 space-y-3">{children}</div>}
    </details>
  )
}

function CopyPayload({ value, label }: { value: unknown; label: string }) {
  const [copied, setCopied] = useState(false)
  if (value == null) return null
  return (
    <Button variant="ghost" size="sm" className="h-7 gap-1.5 px-2 text-xs text-muted-foreground" aria-label={`Copy ${label}`} onClick={() => {
      navigator.clipboard.writeText(typeof value === 'string' ? value : JSON.stringify(value, null, 2)).then(() => {
        setCopied(true)
        setTimeout(() => setCopied(false), 1500)
      })
    }}>
      {copied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
      {copied ? 'Copied' : 'Copy'}
    </Button>
  )
}

function ExchangeGrid({ children }: { children: ReactNode }) {
  return <div className="grid grid-cols-1 gap-3 [@container(min-width:640px)]:grid-cols-2" data-testid="exchange-grid">{children}</div>
}

function ExchangeCard({ title, description, payload, children, error = false }: { title: 'Request' | 'Response'; description?: ReactNode; payload?: unknown; children: ReactNode; error?: boolean }) {
  const isRequest = title === 'Request'
  const Icon = isRequest ? ArrowUpRight : ArrowDownLeft
  return (
    <section aria-label={title} className={cn('min-w-0 rounded-lg border overflow-hidden self-start', error && 'border-destructive/40')}>
      <div className={cn('flex items-center gap-2 border-b px-3 py-1', isRequest ? 'bg-blue-500/5' : error ? 'bg-destructive/5' : 'bg-emerald-500/5')}>
        <Icon className={cn('h-4 w-4 shrink-0', isRequest ? 'text-blue-500' : error ? 'text-destructive' : 'text-emerald-500')} />
        <h3 className="text-sm font-semibold">{title}</h3>
        {description && <span className="min-w-0 text-[11px] text-muted-foreground truncate">{description}</span>}
        <div className="ml-auto"><CopyPayload value={payload} label={title.toLowerCase()} /></div>
      </div>
      <div className="p-3 space-y-3 text-xs break-words">{children}</div>
    </section>
  )
}

function ResponseState({ status }: { status: EventStatus }) {
  return <p className="flex items-center gap-2 py-3 text-muted-foreground">
    {status === 'pending' && <Loader2 className="h-4 w-4 animate-spin" />}
    {status === 'pending' ? 'Waiting for response…' : status === 'error' ? 'The request failed. No response body was captured.' : 'No response content was captured.'}
  </p>
}

function ResponseError({ error }: { error: unknown }) {
  if (error == null) return null
  return <div className="rounded-md border border-destructive/20 bg-destructive/5 p-3 text-red-600 dark:text-red-400 whitespace-pre-wrap break-words" role="note">
    <div className="font-medium flex items-center gap-1.5 mb-1"><AlertTriangle className="h-3.5 w-3.5" />Request failed</div>
    {typeof error === 'string' ? error : JSON.stringify(error, null, 2)}
  </div>
}

function LlmResponseContent({ data, status }: { data: EventData; status: EventStatus }) {
  const body = data.response_body as Record<string, unknown> | undefined
  const messages = capturedResponseMessages(body, data.raw_response)
  const visibleMessages = messages.filter(message => contentText(message.content) || (message.tool_calls as unknown[] | undefined)?.length)
  const reasoning = messages.map(message => message.reasoning_content).filter(Boolean).join('\n') || extractReasoning(body)
  const systemOne = isSystemOneEvent(data) && body?.answers != null
  const excerpt = capturedExcerpt(body, 'answer')
  const hasContent = systemOne || visibleMessages.length > 0 || data.content_preview || excerpt

  return (
    <>
      <ResponseError error={data.error ?? body?.error} />
      {systemOne ? <SystemOneAnswersList data={data} /> : visibleMessages.length > 0 ? (
        <div className="space-y-2">
          {visibleMessages.map((message, index) => <MessageItem key={index} message={message} />)}
        </div>
      ) : excerpt ? <div><p className="text-muted-foreground mb-2">Captured excerpt</p><SmartText text={excerpt} /></div>
      : data.content_preview ? <SmartText text={data.content_preview} /> : !data.error && !body?.error ? (
        body && !reasoning ? <JsonBlock data={body} label="response" /> : !reasoning && !data.raw_response && <ResponseState status={status} />
      ) : null}
      {reasoning && <Disclosure title="Reasoning"><SmartText text={reasoning} /></Disclosure>}
      {!hasContent && !body && data.raw_response && <RawBlock text={data.raw_response} label="raw response" />}
    </>
  )
}

function RequestContent({ body: capturedBody, raw }: { body: Record<string, unknown> | undefined; raw?: string }) {
  const body = capturedRequestBody(capturedBody, raw)
  const excerpt = capturedExcerpt(body, 'question')
  const messages = requestMessages(body)
  const latestUser = messages.map(m => m.role).lastIndexOf('user')
  const primary = latestUser >= 0 ? latestUser : messages.length - 1
  const context = messages.filter((_, index) => index !== primary)
  return (
    <>
      {primary >= 0 ? <MessageItem message={messages[primary]} /> : excerpt ? <div><p className="text-muted-foreground mb-2">Captured excerpt</p><SmartText text={excerpt} /></div> : body ? <JsonBlock data={body} label="request" /> : <p className="text-muted-foreground">No request body was captured.</p>}
      {(context.length > 0 || body?.system || body?.instructions) && (
        <Disclosure title="Conversation context" description={`${messages.length} message${messages.length === 1 ? '' : 's'}`}>
          {body?.system != null && <MessageItem message={{ role: 'system', content: body.system }} />}
          {body?.instructions != null && <MessageItem message={{ role: 'developer', content: body.instructions }} />}
          {messages.map((message, index) => <MessageItem key={index} message={message} />)}
        </Disclosure>
      )}
    </>
  )
}

// ---- LLM call ----

const BODY_CONTENT_KEYS = ['messages', 'input', 'prompt', 'system', 'instructions', 'tools', 'state', 'questions']

function requestParameters(body: Record<string, unknown> | undefined): Record<string, unknown> {
  if (!body) return {}
  return Object.fromEntries(Object.entries(body).filter(([key, value]) => !BODY_CONTENT_KEYS.includes(key) && !key.startsWith('_') && value != null))
}

function requestTools(body: Record<string, unknown> | undefined): McpToolDisplayItem[] {
  const tools = body?.tools as Array<Record<string, unknown>> | undefined
  return (tools || []).map(tool => {
    const fn = tool.function as Record<string, unknown> | undefined
    return {
      name: String(fn?.name || tool.name || tool.type || 'unknown'),
      description: (fn?.description || tool.description || null) as string | null,
      inputSchema: (fn?.parameters || tool.input_schema || tool.parameters || null) as Record<string, unknown> | null,
    }
  })
}

function llmTabs(event: MonitorEvent, data: EventData): DetailTab[] {
  const body = data.request_body as Record<string, unknown> | undefined
  const params = requestParameters(body)
  const tools = requestTools(body)
  const info = data.routing_info as EventData | undefined
  const transformations = (data.transformations_applied as string[] | undefined) ?? []
  const tabs: DetailTab[] = [
    { id: 'exchange', label: 'Request & response', content: <LlmExchange data={data} status={event.status} /> },
    { id: 'details', label: 'Details', content: <LlmOverview event={event} data={data} /> },
  ]
  if (Object.keys(params).length > 0 || tools.length > 0) {
    tabs.push({ id: 'settings', label: 'Parameters & tools', badge: tools.length || undefined, content: <LlmSettings params={params} tools={tools} /> })
  }
  if (info) {
    tabs.push({ id: 'routing', label: 'Routing', badge: info.total_attempts ?? info.attempts?.length, content: <RoutingInfoView info={info} /> })
  }
  if (data.transformed_body || transformations.length > 0) {
    tabs.push({ id: 'transformations', label: 'Transformations', badge: transformations.length || undefined, content: <LlmTransformations data={data} /> })
  }
  tabs.push({ id: 'raw', label: 'Raw', content: <LlmRaw event={event} data={data} /> })
  return tabs
}

function LlmExchange({ data, status }: { data: EventData; status: EventStatus }) {
  const body = data.request_body as Record<string, unknown> | undefined
  const flow = llmApiFlow(data)
  const responseCopy = data.response_body ?? data.raw_response ?? data.content_preview ?? data.error
  return (
    <>
      <ExchangeGrid>
        <ExchangeCard
          title="Request"
          description={flow.client ? `${LLM_API_LABELS[flow.client]} · ${data.endpoint}` : data.endpoint}
          payload={body ?? data.raw_request}
        >
          {isSystemOneEvent(data) && body?.questions ? <SystemOneRequestView body={body} /> : <RequestContent body={body} raw={data.raw_request} />}
        </ExchangeCard>
        <ExchangeCard
          title="Response"
          description={data.status_code != null ? `HTTP ${data.status_code}${data.streamed ? ' · streamed' : ''}` : status === 'pending' ? 'In progress' : undefined}
          payload={responseCopy}
          error={status === 'error'}
        >
          <LlmResponseContent data={data} status={status} />
        </ExchangeCard>
      </ExchangeGrid>
      <div className="flex flex-wrap items-center gap-x-6 gap-y-2 rounded-lg border bg-muted/10 px-3 py-2" aria-label="Usage">
        <Field label="Provider" value={data.provider} />
        <Field label="Model" value={data.model} />
        <Field label="Input" value={data.input_tokens != null ? `${tokens(data.input_tokens)} tokens` : undefined} />
        <Field label="Output" value={data.output_tokens != null ? `${tokens(data.output_tokens)} tokens` : undefined} />
        <Field label="Cost" value={data.cost_usd != null ? `$${Number(data.cost_usd).toFixed(6)}` : undefined} />
        <Field label="Finish" value={data.finish_reason} />
      </div>
    </>
  )
}

const SOURCE_LABELS: Record<string, string> = {
  api: 'LocalRouter API',
  proxy: 'HTTPS inspection proxy',
  reverse_proxy: 'Reverse proxy',
}

/** Client → LocalRouter → provider, labelled with the API used on each hop. */
function ApiPath({ event, data, flow }: { event: MonitorEvent; data: EventData; flow: ApiFlow }) {
  const node = (label: string, value: ReactNode) => (
    <div className="min-w-0 rounded-md border bg-background px-2.5 py-1.5">
      <div className="text-[10px] uppercase tracking-wide text-muted-foreground">{label}</div>
      <div className="truncate text-xs font-medium">{value}</div>
    </div>
  )
  const hop = (api: ReturnType<typeof llmApiFlow>['client'], pending: string) => (
    <div className="flex min-w-[110px] flex-1 flex-col items-center px-1 text-center">
      <span className={cn('text-[11px] font-medium', !api && 'text-muted-foreground italic font-normal')}>{api ? LLM_API_LABELS[api] : pending}</span>
      <div className="mt-0.5 flex w-full items-center">
        <div className={cn('h-px flex-1', flow.translated ? 'bg-amber-500/60' : 'bg-border')} />
        <ArrowRight className={cn('h-3 w-3 shrink-0', flow.translated ? 'text-amber-500' : 'text-muted-foreground')} />
      </div>
    </div>
  )
  return (
    <div className="space-y-2">
      <div className="flex items-center rounded-lg border bg-muted/10 p-3" data-testid="api-path">
        {node('Client', event.client_name || event.client_id || 'Unknown')}
        {hop(flow.client, 'unknown')}
        {node('LocalRouter', SOURCE_LABELS[data.source as string] ?? 'LocalRouter API')}
        {hop(flow.upstream, event.status === 'pending' ? 'pending…' : 'not recorded')}
        {node('Provider', data.provider || '—')}
      </div>
      {flow.translated ? (
        <p className="flex items-start gap-1.5 rounded-md bg-amber-500/10 px-2.5 py-1.5 text-xs text-amber-800 dark:text-amber-300">
          <Shuffle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          LocalRouter translated this request from {LLM_API_LABELS[flow.client!]} to {LLM_API_LABELS[flow.upstream!]} for the provider, and the response back.
        </p>
      ) : data.source === 'proxy' || data.source === 'reverse_proxy' ? (
        <p className="text-xs text-muted-foreground">Observed in transit and forwarded unchanged.</p>
      ) : null}
    </div>
  )
}

function LlmOverview({ event, data }: { event: MonitorEvent; data: EventData }) {
  const flow = llmApiFlow(data)
  const input = data.input_tokens as number | undefined
  const cached = data.cached_input_tokens as number | undefined
  const requested = data.requested_model as string | undefined
  const served = data.model as string | undefined
  const started = Date.parse(event.timestamp)
  const finished = event.duration_ms != null && Number.isFinite(started) ? started + event.duration_ms : null
  return (
    <>
      <Section title="API">
        <ApiPath event={event} data={data} flow={flow} />
        <Properties items={[
          ['Client API', flow.client && LLM_API_LABELS[flow.client]],
          ['Upstream API', flow.upstream && LLM_API_LABELS[flow.upstream]],
          ['Endpoint', data.endpoint && <Mono key="e">{data.endpoint}</Mono>],
          ['Observed by', SOURCE_LABELS[data.source as string] ?? SOURCE_LABELS.api],
          ['Streaming', data.stream ? 'Yes' : 'No'],
          ['HTTP status', data.status_code != null ? String(data.status_code) : null],
        ]} />
      </Section>
      <Section title="Model">
        <Properties items={[
          ['Requested', requested && <Mono key="r">{requested}</Mono>],
          ['Answered by', served && <Mono key="s">{served}</Mono>],
          ['Provider', data.provider],
          ['Finish reason', data.finish_reason],
          ['Messages', data.message_count ? String(data.message_count) : null],
          ['Tools offered', data.tool_count ? String(data.tool_count) : null],
        ]} />
      </Section>
      <Section title="Usage">
        <Properties items={[
          ['Input tokens', tokens(input)],
          ['Cached input', cached != null ? `${tokens(cached)}${input ? ` (${Math.round((cached / input) * 100)}%)` : ''}` : null],
          ['Output tokens', tokens(data.output_tokens)],
          ['Reasoning tokens', data.reasoning_tokens ? tokens(data.reasoning_tokens) : null],
          ['Total tokens', tokens(data.total_tokens)],
          ['Cost', data.cost_usd != null ? `$${Number(data.cost_usd).toFixed(6)}` : null],
        ]} />
      </Section>
      <Section title="Timing">
        <Properties items={[
          ['Started', formatTimestamp(event.timestamp)],
          ['Finished', finished != null ? formatTimestamp(finished) : null],
          ['Duration', <EventDuration key="d" event={event} />],
        ]} />
      </Section>
      <Section title="Identifiers">
        <Properties items={[
          ['Client', event.client_name],
          ['Client ID', event.client_id && <Mono key="c">{event.client_id}</Mono>],
          ['Session', event.session_id && <Mono key="s">{event.session_id}</Mono>],
          ['Trace', data.trace_id && <Mono key="t">{data.trace_id}</Mono>],
          ['Duplicate hop', data.duplicate_hop != null ? String(data.duplicate_hop) : null],
          ['Event ID', <Mono key="i">{event.id}</Mono>],
        ]} />
      </Section>
    </>
  )
}

function ParameterValue({ value }: { value: unknown }) {
  if (value !== null && typeof value === 'object') return <JsonTree data={value} />
  if (typeof value === 'boolean') return <>{value ? 'true' : 'false'}</>
  return <Mono>{String(value)}</Mono>
}

function LlmSettings({ params, tools }: { params: Record<string, unknown>; tools: McpToolDisplayItem[] }) {
  const scalars = Object.entries(params).filter(([, value]) => value === null || typeof value !== 'object')
  const objects = Object.entries(params).filter(([, value]) => value !== null && typeof value === 'object')
  return (
    <>
      {(scalars.length > 0 || objects.length > 0) && (
        <Section title="Parameters">
          {scalars.length > 0 && <Properties items={scalars.map(([key, value]) => [key, <ParameterValue key={key} value={value} />])} />}
          {objects.map(([key, value]) => (
            <div key={key} className="space-y-1">
              <div className="text-[11px] text-muted-foreground font-mono">{key}</div>
              <ParameterValue value={value} />
            </div>
          ))}
        </Section>
      )}
      {tools.length > 0 && (
        <Section title={`Tools (${tools.length})`}>
          <McpToolDisplay tools={tools} compact collapsible />
        </Section>
      )}
    </>
  )
}

function RoutingInfoView({ info }: { info: EventData }) {
  const decision = info.decision_routing as EventData | undefined
  const attempts = (info.attempts as EventData[] | undefined) ?? []
  const candidates = (info.candidate_models as string[] | undefined) ?? []
  return (
    <>
      {decision && (
        <Section title="Decision">
          <Properties items={[
            ['Route', decision.route],
            ['Decided by', decision.source],
            ['Reason', decision.reason],
            ['Decision time', decision.latency_ms != null ? `${decision.latency_ms} ms` : null],
            ['Omitted messages', decision.context_omitted ? String(decision.context_omitted) : null],
          ]} />
        </Section>
      )}
      {(info.routellm_tier || info.routellm_win_rate != null) && (
        <Section title="Legacy routing">
          <Properties items={[
            ['Tier', info.routellm_tier],
            ['Win rate', info.routellm_win_rate != null ? Number(info.routellm_win_rate).toFixed(3) : null],
          ]} />
        </Section>
      )}
      {candidates.length > 0 && (
        <Section title="Candidates">
          <div className="flex flex-wrap gap-1.5">
            {candidates.map(model => <Badge key={model} variant="outline" className="font-mono text-[11px] font-normal">{model}</Badge>)}
          </div>
        </Section>
      )}
      <Section title={`Attempts (${attempts.length})`}>
        {attempts.length === 0 ? <p className="text-xs italic text-muted-foreground">No attempts recorded.</p> : (
          <ol className="space-y-1.5">
            {attempts.map((attempt, index) => (
              <li key={index} className="flex flex-wrap items-center gap-2 rounded-md border px-2.5 py-1.5 text-xs">
                <span className="w-4 text-muted-foreground tabular-nums">{index + 1}</span>
                <Mono>{attempt.provider}/{attempt.model}</Mono>
                <span className={cn('rounded-full px-1.5 text-[10px] font-medium', attempt.outcome === 'success' ? 'bg-emerald-500/15 text-emerald-700 dark:text-emerald-300' : 'bg-destructive/15 text-destructive')}>{attempt.outcome}</span>
                {attempt.duration_ms != null && <span className="ml-auto text-muted-foreground tabular-nums">{attempt.duration_ms} ms</span>}
                {attempt.error && <p className="basis-full text-destructive break-words">{attempt.error}</p>}
              </li>
            ))}
          </ol>
        )}
      </Section>
    </>
  )
}

function LlmTransformations({ data }: { data: EventData }) {
  const transformations = (data.transformations_applied as string[] | undefined) ?? []
  const original = data.request_body as Record<string, unknown> | undefined
  const transformed = data.transformed_body as Record<string, unknown> | undefined
  return (
    <>
      <Section title="Applied">
        {transformations.length === 0 ? <p className="text-xs italic text-muted-foreground">No transformations were listed.</p> : (
          <ul className="space-y-1">
            {transformations.map(value => <li key={value} className="flex items-center gap-2 text-xs"><ChevronRight className="h-3 w-3 text-muted-foreground" />{value}</li>)}
          </ul>
        )}
      </Section>
      {transformed && (
        <ExchangeGrid>
          <RawSection title="Received from the client"><RequestContent body={original} /></RawSection>
          <RawSection title="Sent upstream"><RequestContent body={transformed} /></RawSection>
          <JsonBlock data={original} label="original" />
          <JsonBlock data={transformed} label="transformed" />
        </ExchangeGrid>
      )}
    </>
  )
}

function LlmRaw({ event, data }: { event: MonitorEvent; data: EventData }) {
  return (
    <>
      <ExchangeGrid>
        <RawSection title="Request body">{data.request_body != null ? <JsonBlock data={data.request_body} label="request body" /> : <p className="text-xs italic text-muted-foreground">Not captured.</p>}</RawSection>
        <RawSection title="Response body">{data.response_body != null ? <JsonBlock data={data.response_body} label="response body" /> : <p className="text-xs italic text-muted-foreground">Not captured.</p>}</RawSection>
      </ExchangeGrid>
      {data.raw_request && <Disclosure title="Raw request (wire bytes)"><RawBlock text={data.raw_request} label="raw request" /></Disclosure>}
      {data.raw_response && <Disclosure title="Raw response (wire bytes)"><RawBlock text={data.raw_response} label="raw response" /></Disclosure>}
      <RawSection title="Full event"><JsonTree data={event} label="event" className="max-h-[480px]" /></RawSection>
    </>
  )
}

// ---- MCP Tool Call Detail ----

function McpToolCallDetail({ data }: { data: EventData }) {
  const hasResponse = data.success != null || data.error != null || data.response_preview != null || data.content_preview != null

  return (
    <ExchangeGrid>
      <ExchangeCard title="Request">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Tool" value={data.tool_name as string} />
          <ServerField data={data} />
          {data.firewall_action && <Field label="Firewall" value={data.firewall_action as string} />}
        </div>
        {data.arguments != null && <ArgumentsBlock args={data.arguments as unknown} />}
      </ExchangeCard>

      {hasResponse && (
        <ExchangeCard title="Response">
          <McpResponseContent data={data} />
        </ExchangeCard>
      )}
      {!hasResponse && <ExchangeCard title="Response"><p className="text-muted-foreground">No response captured yet.</p></ExchangeCard>}
    </ExchangeGrid>
  )
}

// ---- MCP Resource Read Detail ----

function McpResourceReadDetail({ data }: { data: EventData }) {
  const hasResponse = data.success != null || data.error != null || data.response_preview != null || data.content_preview != null

  return (
    <ExchangeGrid>
      <ExchangeCard title="Request">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="URI" value={data.uri as string} />
          <ServerField data={data} />
        </div>
      </ExchangeCard>

      {hasResponse && (
        <ExchangeCard title="Response">
          <McpResponseContent data={data} />
        </ExchangeCard>
      )}
      {!hasResponse && <ExchangeCard title="Response"><p className="text-muted-foreground">No response captured yet.</p></ExchangeCard>}
    </ExchangeGrid>
  )
}

// ---- MCP Prompt Get Detail ----

function McpPromptGetDetail({ data }: { data: EventData }) {
  const hasResponse = data.success != null || data.error != null || data.response_preview != null || data.content_preview != null

  return (
    <ExchangeGrid>
      <ExchangeCard title="Request">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Prompt" value={data.prompt_name as string} />
          <ServerField data={data} />
        </div>
        {data.arguments && <JsonBlock data={data.arguments as unknown} />}
      </ExchangeCard>

      {hasResponse && (
        <ExchangeCard title="Response">
          <McpResponseContent data={data} />
        </ExchangeCard>
      )}
      {!hasResponse && <ExchangeCard title="Response"><p className="text-muted-foreground">No response captured yet.</p></ExchangeCard>}
    </ExchangeGrid>
  )
}

// ---- MCP Elicitation Detail ----

function McpElicitationDetail({ data }: { data: EventData }) {
  const hasResponse = data.action != null

  return (
    <ExchangeGrid>
      <ExchangeCard title="Request">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <ServerField data={data} />
        </div>
        {data.message && (
          <pre className="p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[200px] overflow-auto">
            {data.message as string}
          </pre>
        )}
        {data.schema && <JsonBlock data={data.schema as unknown} />}
      </ExchangeCard>

      {hasResponse && (
        <ExchangeCard title="Response">
          <div className="grid grid-cols-2 gap-2 text-xs">
            <Field label="Action" value={data.action as string} />
            {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
          </div>
          {data.content && <JsonBlock data={data.content as unknown} />}
        </ExchangeCard>
      )}
      {!hasResponse && <ExchangeCard title="Response"><p className="text-muted-foreground">No response captured yet.</p></ExchangeCard>}
    </ExchangeGrid>
  )
}

// ---- MCP Sampling Detail ----

function McpSamplingDetail({ data }: { data: EventData }) {
  const hasResponse = data.action != null

  return (
    <ExchangeGrid>
      <ExchangeCard title="Request">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <ServerField data={data} />
          {data.message_count != null && <Field label="Messages" value={String(data.message_count)} />}
          {data.model_hint && <Field label="Model Hint" value={data.model_hint as string} />}
          {data.max_tokens != null && <Field label="Max Tokens" value={String(data.max_tokens)} />}
        </div>
      </ExchangeCard>

      {hasResponse && (
        <ExchangeCard title="Response">
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
        </ExchangeCard>
      )}
      {!hasResponse && <ExchangeCard title="Response"><p className="text-muted-foreground">No response captured yet.</p></ExchangeCard>}
    </ExchangeGrid>
  )
}

// ---- Guardrail Detail ----

function GuardrailDetail({ data }: { data: EventData }) {
  const categories = data.flagged_categories as Array<Record<string, unknown>> | undefined
  const hasResult = data.result != null

  return (
    <ExchangeGrid>
      <ExchangeCard title="Request">
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
      </ExchangeCard>

      {hasResult && (
        <ExchangeCard title="Response">
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
        </ExchangeCard>
      )}
    </ExchangeGrid>
  )
}

// ---- Secret Scan Detail ----

function SecretScanDetail({ data }: { data: EventData }) {
  const hasResult = data.findings_count != null

  return (
    <ExchangeGrid>
      <ExchangeCard title="Request">
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
      </ExchangeCard>

      {hasResult && (
        <ExchangeCard title="Response">
          <div className="grid grid-cols-2 gap-2 text-xs">
            <Field label="Findings" value={String(data.findings_count)} />
            {data.action_taken && <Field label="Action" value={data.action_taken as string} />}
            {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
          </div>
          {data.findings && <JsonBlock data={data.findings as unknown} />}
        </ExchangeCard>
      )}
    </ExchangeGrid>
  )
}

// ---- Routing Detail ----

function RoutingDetail({ data }: { data: EventData }) {
  const hasResult = data.selected_tier != null || data.win_rate != null || data.final_model != null

  return (
    <ExchangeGrid>
      <ExchangeCard title="Request">
        <div className="grid grid-cols-2 gap-2 text-xs">
          {data.routing_type && <Field label="Type" value={data.routing_type as string} />}
          {data.original_model && <Field label="Original Model" value={data.original_model as string} />}
          {data.threshold != null && <Field label="Threshold" value={String(data.threshold)} />}
        </div>
      </ExchangeCard>

      {hasResult && (
        <ExchangeCard title="Response">
          <div className="grid grid-cols-2 gap-2 text-xs">
            {data.selected_tier && <Field label="Tier" value={data.selected_tier as string} />}
            {data.win_rate != null && <Field label="Win Rate" value={((data.win_rate as number) * 100).toFixed(1) + '%'} />}
            {data.routed_model && <Field label="Routed Model" value={data.routed_model as string} />}
            {data.final_model && <Field label="Final Model" value={data.final_model as string} />}
            {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
            {data.firewall_action && <Field label="Firewall" value={data.firewall_action as string} />}
            {data.candidate_models && <Field label="Candidates" value={(data.candidate_models as string[]).join(', ')} />}
          </div>
        </ExchangeCard>
      )}
    </ExchangeGrid>
  )
}

// ---- Error/Message Event Details ----

function AuthErrorDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-3">
      <div className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Error Type" value={data.error_type as string} />
          <Field label="Status Code" value={String(data.status_code)} />
          <Field label="Endpoint" value={data.endpoint as string} />
          {data.reason && <Field label="Reason" value={data.reason as string} />}
        </div>
      </div>
      {data.message && (
        <section aria-label="Response">
          <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
            {data.message as string}
          </pre>
        </section>
      )}
    </div>
  )
}

function RateLimitDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-3">
      <div className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Reason" value={data.reason as string} />
          <Field label="Status Code" value={String(data.status_code)} />
          <Field label="Endpoint" value={data.endpoint as string} />
          {data.retry_after_secs != null && <Field label="Retry After" value={`${data.retry_after_secs}s`} />}
        </div>
      </div>
      {data.message && (
        <section aria-label="Response">
          <pre className="p-2 bg-amber-500/10 rounded text-xs whitespace-pre-wrap text-amber-700 dark:text-amber-400">
            {data.message as string}
          </pre>
        </section>
      )}
    </div>
  )
}

function ValidationErrorDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-3">
      <div className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Endpoint" value={data.endpoint as string} />
          <Field label="Status Code" value={String(data.status_code)} />
          {data.field && <Field label="Field" value={data.field as string} />}
        </div>
      </div>
      {data.message && (
        <section aria-label="Response">
          <pre className="p-2 bg-yellow-500/10 rounded text-xs whitespace-pre-wrap text-yellow-700 dark:text-yellow-400">
            {data.message as string}
          </pre>
        </section>
      )}
    </div>
  )
}

function McpServerEventDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-3">
      <div className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <div className="flex items-center gap-1 text-xs">
            <Server className="h-3 w-3 text-muted-foreground" />
            <span className="text-muted-foreground">Server:</span>
            <span>{(data.server_name || data.server_id) as string}</span>
          </div>
          <Field label="Action" value={data.action as string} />
        </div>
      </div>
      {data.message && (
        <section aria-label="Response">
          <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
            {data.message as string}
          </pre>
        </section>
      )}
    </div>
  )
}

function OAuthEventDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-3">
      <div className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Action" value={data.action as string} />
          <Field label="Status Code" value={String(data.status_code)} />
          {data.client_id_hint && <Field label="Client" value={data.client_id_hint as string} />}
        </div>
      </div>
      {data.message && (
        <section aria-label="Response">
          <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
            {data.message as string}
          </pre>
        </section>
      )}
    </div>
  )
}

function InternalErrorDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-3">
      <div className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Error Type" value={data.error_type as string} />
          <Field label="Status Code" value={String(data.status_code)} />
        </div>
      </div>
      {data.message && (
        <section aria-label="Response">
          <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
            {data.message as string}
          </pre>
        </section>
      )}
    </div>
  )
}

function ModerationEventDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-3">
      <div className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Reason" value={data.reason as string} />
          <Field label="Status Code" value={String(data.status_code)} />
        </div>
      </div>
      {data.message && (
        <section aria-label="Response">
          <pre className="p-2 bg-orange-500/10 rounded text-xs whitespace-pre-wrap text-orange-700 dark:text-orange-400">
            {data.message as string}
          </pre>
        </section>
      )}
    </div>
  )
}

function ConnectionErrorDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-3">
      <div className="space-y-2">
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Transport" value={data.transport as string} />
          <Field label="Action" value={data.action as string} />
        </div>
      </div>
      {data.message && (
        <section aria-label="Response">
          <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
            {data.message as string}
          </pre>
        </section>
      )}
    </div>
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

function JsonRepairDetail({ data }: { data: EventData }) {
  const repairs = (data.repairs as string[] | undefined) ?? []
  return (
    <div className="space-y-3 text-xs">
      <div className="grid grid-cols-2 gap-2">
        <Field label="Model" value={data.model as string} />
        <Field label="Response" value={data.streamed ? 'Streamed' : 'Complete'} />
      </div>
      <div>
        <span className="text-muted-foreground font-medium">Repairs:</span>
        <ul className="mt-1 list-disc pl-5 space-y-0.5">
          {repairs.map(repair => <li key={repair}>{repair}</li>)}
        </ul>
      </div>
      {data.original != null && (
        <div>
          <span className="text-muted-foreground font-medium">Before:</span>
          <div className="mt-1"><RawBlock label="before" text={data.original as string} /></div>
        </div>
      )}
      {data.repaired != null && (
        <div>
          <span className="text-muted-foreground font-medium">After:</span>
          <div className="mt-1"><RawBlock label="after" text={data.repaired as string} /></div>
        </div>
      )}
    </div>
  )
}

function MemoryCompactionDetail({ data, status }: { data: EventData; status: EventStatus }) {
  const body = data.request_body as Record<string, unknown> | undefined
  return <div className="space-y-3">
    <ExchangeGrid>
      <ExchangeCard title="Request" payload={body}>
        <RequestContent body={body} />
        {data.transcript_path && <ArchiveFileField label="Transcript" path={data.transcript_path} />}
      </ExchangeCard>
      <ExchangeCard title="Response" payload={data.response_body ?? data.content_preview ?? data.error} error={status === 'error'}>
        <ResponseError error={data.error} />
        {data.response_body || data.content_preview || data.summary_bytes != null ? <CompactionResponseContent data={data} /> : !data.error && <ResponseState status={status} />}
      </ExchangeCard>
    </ExchangeGrid>
    <Disclosure title="Compaction metadata & payloads">
      <Field label="Session" value={data.session_id} />
      <Field label="Model" value={data.model} />
      <Field label="Transcript size" value={`${data.transcript_bytes} bytes`} />
      <JsonBlock data={body} label="request body" />
      <JsonBlock data={data.response_body} label="response body" />
    </Disclosure>
  </div>
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

      <LlmResponseContent data={{ ...data, error: undefined }} status="complete" />
      {responseBody && <Disclosure title="Full response body"><JsonBlock data={responseBody} /></Disclosure>}
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

const PASSTHROUGH_MODES: Record<string, string> = {
  tunnel: 'Tunneled (encrypted, never decrypted)',
  http: 'Plain HTTP, forwarded unchanged',
  inspected: 'Inspected host, but not an LLM endpoint',
  websocket: 'WebSocket, relayed unchanged',
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  return `${(n / (1024 * 1024)).toFixed(1)} MB`
}

/** Non-LLM traffic that passed through the proxy untouched. Shows where it went
 *  and how much moved — deliberately never its content. */
function ProxyPassthroughDetail({ data }: { data: EventData }) {
  const host = data.host as string
  const port = data.port as number
  const destination = port === 443 || port === 80 ? host : `${host}:${port}`
  const method = data.method as string | undefined
  const path = data.path as string | undefined
  const sent = data.bytes_sent as number | undefined
  const received = data.bytes_received as number | undefined

  return (
    <div className="space-y-3">
      <div className="flex items-start gap-2 rounded border border-amber-500/30 bg-amber-500/10 p-2">
        <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-500" />
        <p className="whitespace-pre-line text-xs leading-relaxed">{data.note as string}</p>
      </div>

      <div className="grid grid-cols-2 gap-2">
        <Field label="Destination" value={destination} />
        <Field label="Forwarded as" value={PASSTHROUGH_MODES[data.mode as string] ?? (data.mode as string)} />
        {method && path && <Field label="Request" value={`${method} ${path}`} />}
        <Field label="Status" value={data.status_code ? String(data.status_code) : undefined} />
        <Field label="Sent" value={sent !== undefined && sent !== null ? formatBytes(sent) : undefined} />
        <Field label="Received" value={received !== undefined && received !== null ? formatBytes(received) : undefined} />
      </div>

      {data.error != null && (
        <div className="rounded border border-destructive/30 bg-destructive/10 p-2 text-xs text-destructive">
          {data.error as string}
        </div>
      )}

      <p className="text-[11px] italic text-muted-foreground">
        No request or response content was captured for this event — LocalRouter only inspects LLM calls.
      </p>
    </div>
  )
}
