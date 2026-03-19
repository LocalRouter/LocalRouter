import { Badge } from '@/components/ui/Badge'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import { McpToolDisplay, type McpToolDisplayItem } from '@/components/shared/McpToolDisplay'
import { cn } from '@/lib/utils'
import { ChevronRight, Clock, User, Server } from 'lucide-react'
import { useState } from 'react'
import type { MonitorEvent } from '@/types/tauri-commands'

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
        {type === 'firewall_decision' && <FirewallDecisionDetail data={data} />}
        {type === 'sse_connection' && <SseConnectionDetail data={data} />}
      </div>
    </ScrollArea>
  )
}

// ---- Pretty-printed Messages ----

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

function MessagesSection({ messages }: { messages: Array<Record<string, unknown>> }) {
  const [isOpen, setIsOpen] = useState(messages.length <= 5)

  return (
    <Collapsible open={isOpen} onOpenChange={setIsOpen}>
      <CollapsibleTrigger className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors">
        <ChevronRight className={cn('h-3 w-3 transition-transform', isOpen && 'rotate-90')} />
        <span className="font-medium">Messages ({messages.length})</span>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div className="mt-1 space-y-1.5">
          {messages.map((msg, i) => (
            <MessageItem key={i} message={msg} />
          ))}
        </div>
      </CollapsibleContent>
    </Collapsible>
  )
}

// ---- Pretty-printed Tools using McpToolDisplay ----

function ToolsSection({ tools }: { tools: Array<Record<string, unknown>> }) {
  const [isOpen, setIsOpen] = useState(false)

  const displayItems: McpToolDisplayItem[] = tools.map(t => {
    const fn = t.function as Record<string, unknown> | undefined
    return {
      name: (fn?.name as string) || (t.name as string) || 'unknown',
      description: (fn?.description as string) || null,
      inputSchema: (fn?.parameters as Record<string, unknown>) || null,
    }
  })

  return (
    <Collapsible open={isOpen} onOpenChange={setIsOpen}>
      <CollapsibleTrigger className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors">
        <ChevronRight className={cn('h-3 w-3 transition-transform', isOpen && 'rotate-90')} />
        <span className="font-medium">Tools ({tools.length})</span>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div className="mt-1">
          <McpToolDisplay tools={displayItems} compact />
        </div>
      </CollapsibleContent>
    </Collapsible>
  )
}

// ---- Type-specific detail components ----

function SectionHeader({ title }: { title: string }) {
  return (
    <div className="text-[10px] uppercase tracking-wider text-muted-foreground font-semibold border-b border-border/50 pb-0.5 pt-1">
      {title}
    </div>
  )
}

function LlmCallDetail({ data }: { data: EventData }) {
  const body = data.request_body as Record<string, unknown> | undefined
  const transformedBody = data.transformed_body as Record<string, unknown> | undefined
  const transformations = data.transformations_applied as string[] | undefined
  const hasTransformed = transformedBody != null
  const hasResponse = data.provider != null
  const hasError = data.error != null

  // Toggle between original and transformed request view
  const [showTransformed, setShowTransformed] = useState(hasTransformed)

  // The active body to display (original or transformed)
  const activeBody = (showTransformed && transformedBody) ? transformedBody : body
  const messages = activeBody?.messages as Array<Record<string, unknown>> | undefined
  const tools = activeBody?.tools as Array<Record<string, unknown>> | undefined

  return (
    <div className="space-y-2">
      {/* Request section header with original/transformed toggle */}
      {(hasResponse || hasError) && (
        <div className="flex items-center justify-between">
          <SectionHeader title="Request" />
        </div>
      )}

      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Endpoint" value={data.endpoint as string} />
        <Field label="Model" value={data.model as string} />
        <Field label="Stream" value={data.stream != null ? String(data.stream) : undefined} />
        <Field label="Messages" value={data.message_count != null ? String(data.message_count) : undefined} />
        {(data.has_tools as boolean) && <Field label="Tools" value={String(data.tool_count)} />}
      </div>

      {/* Original / Transformed toggle */}
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

      {messages && messages.length > 0 && (
        <MessagesSection messages={messages} />
      )}

      {tools && tools.length > 0 && (
        <ToolsSection tools={tools} />
      )}

      {activeBody && (
        <JsonSection title="Parameters" data={{
          temperature: activeBody.temperature,
          max_tokens: activeBody.max_tokens,
          top_p: activeBody.top_p,
          frequency_penalty: activeBody.frequency_penalty,
          presence_penalty: activeBody.presence_penalty,
          seed: activeBody.seed,
        }} defaultOpen={false} />
      )}

      {activeBody && (
        <JsonSection title="Full Request Body" data={activeBody} defaultOpen={false} />
      )}

      {/* Response section (present when status=complete) */}
      {hasResponse && (
        <>
          <SectionHeader title="Response" />

          <div className="grid grid-cols-2 gap-2 text-xs">
            <Field label="Provider" value={data.provider as string} />
            <Field label="Status" value={data.status_code != null ? String(data.status_code) : undefined} />
            <Field label="Streamed" value={data.streamed != null ? String(data.streamed) : undefined} />
          </div>

          {data.total_tokens != null && (
            <div className="grid grid-cols-3 gap-2 text-xs">
              <Field label="Input Tokens" value={String(data.input_tokens)} />
              <Field label="Output Tokens" value={String(data.output_tokens)} />
              <Field label="Total Tokens" value={String(data.total_tokens)} />
            </div>
          )}

          {(data.cost_usd != null || data.latency_ms != null) && (
            <div className="grid grid-cols-2 gap-2 text-xs">
              {data.cost_usd != null && <Field label="Cost" value={`$${(data.cost_usd as number).toFixed(6)}`} />}
              {data.latency_ms != null && <Field label="Latency" value={`${String(data.latency_ms)}ms`} />}
            </div>
          )}

          {data.finish_reason && <Field label="Finish Reason" value={data.finish_reason as string} />}

          {data.content_preview && (
            <div className="text-xs">
              <span className="text-muted-foreground font-medium">Content:</span>
              <pre className="mt-1 p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[300px] overflow-auto">
                {data.content_preview as string}
              </pre>
            </div>
          )}
        </>
      )}

      {/* Error section (present when status=error) */}
      {hasError && (
        <>
          <SectionHeader title="Error" />
          <div className="grid grid-cols-2 gap-2 text-xs">
            {data.provider && <Field label="Provider" value={data.provider as string} />}
            {data.status_code != null && <Field label="Status Code" value={String(data.status_code)} />}
          </div>
          <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
            {data.error as string}
          </pre>
        </>
      )}
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

function McpResponseSection({ data }: { data: EventData }) {
  if (data.success == null && !data.error && !data.response_preview && !data.content_preview) return null
  return (
    <>
      <SectionHeader title="Response" />
      {data.success != null && (
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Success" value={String(data.success)} />
          {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
        </div>
      )}
      {data.error && (
        <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
          {data.error as string}
        </pre>
      )}
      {(data.response_preview || data.content_preview) && (
        <div className="text-xs">
          <span className="text-muted-foreground font-medium">Content:</span>
          <pre className="mt-1 p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[300px] overflow-auto">
            {(data.response_preview || data.content_preview) as string}
          </pre>
        </div>
      )}
    </>
  )
}

function McpToolCallDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Tool" value={data.tool_name as string} />
        <ServerField data={data} />
      </div>
      {data.firewall_action && (
        <Field label="Firewall" value={data.firewall_action as string} />
      )}
      <JsonSection title="Arguments" data={data.arguments as unknown} defaultOpen={true} />
      <McpResponseSection data={data} />
    </div>
  )
}

function McpResourceReadDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="URI" value={data.uri as string} />
        <ServerField data={data} />
      </div>
      <McpResponseSection data={data} />
    </div>
  )
}

function McpPromptGetDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Prompt" value={data.prompt_name as string} />
        <ServerField data={data} />
      </div>
      {data.arguments && <JsonSection title="Arguments" data={data.arguments as unknown} defaultOpen={true} />}
      <McpResponseSection data={data} />
    </div>
  )
}

function GuardrailDetail({ data }: { data: EventData }) {
  const categories = data.flagged_categories as Array<Record<string, unknown>> | undefined
  const hasResponse = data.result != null
  return (
    <div className="space-y-2">
      {hasResponse && <SectionHeader title="Scan" />}
      <div className="grid grid-cols-2 gap-2 text-xs">
        {data.direction && <Field label="Direction" value={data.direction as string} />}
        {data.models_used && (
          <Field label="Models" value={(data.models_used as string[]).join(', ')} />
        )}
      </div>
      {data.text_preview && (
        <div className="text-xs">
          <span className="text-muted-foreground font-medium">Input:</span>
          <pre className="mt-1 p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[100px] overflow-auto">
            {data.text_preview as string}
          </pre>
        </div>
      )}

      {hasResponse && (
        <>
          <SectionHeader title="Result" />
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
        </>
      )}
    </div>
  )
}

function SecretScanDetail({ data }: { data: EventData }) {
  const hasResponse = data.findings_count != null
  return (
    <div className="space-y-2">
      {hasResponse && <SectionHeader title="Scan" />}
      <div className="grid grid-cols-2 gap-2 text-xs">
        {data.rules_count != null && <Field label="Rules" value={String(data.rules_count)} />}
      </div>
      {data.text_preview && (
        <div className="text-xs">
          <span className="text-muted-foreground font-medium">Input:</span>
          <pre className="mt-1 p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[100px] overflow-auto">
            {data.text_preview as string}
          </pre>
        </div>
      )}

      {hasResponse && (
        <>
          <SectionHeader title="Result" />
          <div className="grid grid-cols-2 gap-2 text-xs">
            <Field label="Findings" value={String(data.findings_count)} />
            {data.action_taken && <Field label="Action" value={data.action_taken as string} />}
            {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
          </div>
          {data.findings && <JsonSection title="Findings" data={data.findings as unknown} defaultOpen={true} />}
        </>
      )}
    </div>
  )
}

function RoutingDetail({ data }: { data: EventData }) {
  // RouteLlmClassify has request+response; RoutingDecision is standalone
  const hasClassifyResponse = data.selected_tier != null || data.win_rate != null
  return (
    <div className="space-y-2">
      {hasClassifyResponse && <SectionHeader title="Classification Request" />}
      <div className="grid grid-cols-2 gap-2 text-xs">
        {data.routing_type && <Field label="Type" value={data.routing_type as string} />}
        {data.original_model && <Field label="Original Model" value={data.original_model as string} />}
        {data.threshold != null && <Field label="Threshold" value={String(data.threshold)} />}
      </div>

      {hasClassifyResponse && (
        <>
          <SectionHeader title="Classification Result" />
          <div className="grid grid-cols-2 gap-2 text-xs">
            {data.selected_tier && <Field label="Tier" value={data.selected_tier as string} />}
            {data.win_rate != null && <Field label="Win Rate" value={((data.win_rate as number) * 100).toFixed(1) + '%'} />}
            {data.routed_model && <Field label="Routed Model" value={data.routed_model as string} />}
            {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
          </div>
        </>
      )}

      {/* RoutingDecision standalone fields */}
      {data.final_model && <Field label="Final Model" value={data.final_model as string} />}
      {data.firewall_action && <Field label="Firewall" value={data.firewall_action as string} />}
      {data.candidate_models && (
        <Field label="Candidates" value={(data.candidate_models as string[]).join(', ')} />
      )}
    </div>
  )
}

// ---- New event detail components ----

function AuthErrorDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Error Type" value={data.error_type as string} />
        <Field label="Status Code" value={String(data.status_code)} />
        <Field label="Endpoint" value={data.endpoint as string} />
        {data.reason && <Field label="Reason" value={data.reason as string} />}
      </div>
      <div className="text-xs">
        <span className="text-muted-foreground font-medium">Message:</span>
        <pre className="mt-1 p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
          {data.message as string}
        </pre>
      </div>
    </div>
  )
}

function RateLimitDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Reason" value={data.reason as string} />
        <Field label="Status Code" value={String(data.status_code)} />
        <Field label="Endpoint" value={data.endpoint as string} />
        {data.retry_after_secs != null && <Field label="Retry After" value={`${data.retry_after_secs}s`} />}
      </div>
      <div className="text-xs">
        <span className="text-muted-foreground font-medium">Message:</span>
        <pre className="mt-1 p-2 bg-amber-500/10 rounded text-xs whitespace-pre-wrap text-amber-700 dark:text-amber-400">
          {data.message as string}
        </pre>
      </div>
    </div>
  )
}

function ValidationErrorDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Endpoint" value={data.endpoint as string} />
        <Field label="Status Code" value={String(data.status_code)} />
        {data.field && <Field label="Field" value={data.field as string} />}
      </div>
      <div className="text-xs">
        <span className="text-muted-foreground font-medium">Message:</span>
        <pre className="mt-1 p-2 bg-yellow-500/10 rounded text-xs whitespace-pre-wrap text-yellow-700 dark:text-yellow-400">
          {data.message as string}
        </pre>
      </div>
    </div>
  )
}

function McpServerEventDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <div className="flex items-center gap-1 text-xs">
          <Server className="h-3 w-3 text-muted-foreground" />
          <span className="text-muted-foreground">Server:</span>
          <span>{(data.server_name || data.server_id) as string}</span>
        </div>
        <Field label="Action" value={data.action as string} />
      </div>
      <div className="text-xs">
        <span className="text-muted-foreground font-medium">Message:</span>
        <pre className="mt-1 p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
          {data.message as string}
        </pre>
      </div>
    </div>
  )
}

function OAuthEventDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Action" value={data.action as string} />
        <Field label="Status Code" value={String(data.status_code)} />
        {data.client_id_hint && <Field label="Client" value={data.client_id_hint as string} />}
      </div>
      <div className="text-xs">
        <span className="text-muted-foreground font-medium">Message:</span>
        <pre className="mt-1 p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
          {data.message as string}
        </pre>
      </div>
    </div>
  )
}

function InternalErrorDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Error Type" value={data.error_type as string} />
        <Field label="Status Code" value={String(data.status_code)} />
      </div>
      <div className="text-xs">
        <span className="text-muted-foreground font-medium">Message:</span>
        <pre className="mt-1 p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
          {data.message as string}
        </pre>
      </div>
    </div>
  )
}

function ModerationEventDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Reason" value={data.reason as string} />
        <Field label="Status Code" value={String(data.status_code)} />
      </div>
      <div className="text-xs">
        <span className="text-muted-foreground font-medium">Message:</span>
        <pre className="mt-1 p-2 bg-orange-500/10 rounded text-xs whitespace-pre-wrap text-orange-700 dark:text-orange-400">
          {data.message as string}
        </pre>
      </div>
    </div>
  )
}

function ConnectionErrorDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Transport" value={data.transport as string} />
        <Field label="Action" value={data.action as string} />
      </div>
      <div className="text-xs">
        <span className="text-muted-foreground font-medium">Message:</span>
        <pre className="mt-1 p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
          {data.message as string}
        </pre>
      </div>
    </div>
  )
}

function McpElicitationDetail({ data }: { data: EventData }) {
  const hasResponse = data.action != null
  return (
    <div className="space-y-2">
      {hasResponse && <SectionHeader title="Request" />}
      <div className="grid grid-cols-2 gap-2 text-xs">
        <ServerField data={data} />
      </div>
      {data.message && (
        <div className="text-xs">
          <span className="text-muted-foreground font-medium">Message:</span>
          <pre className="mt-1 p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[200px] overflow-auto">
            {data.message as string}
          </pre>
        </div>
      )}
      {data.schema && <JsonSection title="Schema" data={data.schema as unknown} defaultOpen={false} />}

      {hasResponse && (
        <>
          <SectionHeader title="Response" />
          <div className="grid grid-cols-2 gap-2 text-xs">
            <Field label="Action" value={data.action as string} />
            {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
          </div>
          {data.content && <JsonSection title="Content" data={data.content as unknown} defaultOpen={true} />}
        </>
      )}
    </div>
  )
}

function McpSamplingDetail({ data }: { data: EventData }) {
  const hasResponse = data.action != null
  return (
    <div className="space-y-2">
      {hasResponse && <SectionHeader title="Request" />}
      <div className="grid grid-cols-2 gap-2 text-xs">
        <ServerField data={data} />
        {data.message_count != null && <Field label="Messages" value={String(data.message_count)} />}
        {data.model_hint && <Field label="Model Hint" value={data.model_hint as string} />}
        {data.max_tokens != null && <Field label="Max Tokens" value={String(data.max_tokens)} />}
      </div>

      {hasResponse && (
        <>
          <SectionHeader title="Response" />
          <div className="grid grid-cols-2 gap-2 text-xs">
            <Field label="Action" value={data.action as string} />
            {data.model_used && <Field label="Model Used" value={data.model_used as string} />}
            {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
          </div>
          {data.content_preview && (
            <div className="text-xs">
              <span className="text-muted-foreground font-medium">Content:</span>
              <pre className="mt-1 p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[200px] overflow-auto">
                {data.content_preview as string}
              </pre>
            </div>
          )}
        </>
      )}
    </div>
  )
}

function PromptCompressionDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Method" value={data.method as string} />
        <Field label="Reduction" value={`${((data.reduction_percent as number) ?? 0).toFixed(1)}%`} />
        <Field label="Original Tokens" value={String(data.original_tokens)} />
        <Field label="Compressed Tokens" value={String(data.compressed_tokens)} />
        <Field label="Duration" value={`${data.duration_ms}ms`} />
      </div>
    </div>
  )
}

function FirewallDecisionDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Type" value={data.firewall_type as string} />
        <Field label="Item" value={data.item_name as string} />
        <Field label="Action" value={data.action as string} />
        {data.duration && <Field label="Duration" value={data.duration as string} />}
      </div>
    </div>
  )
}

function SseConnectionDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Session" value={data.session_id as string} />
        <Field label="Action" value={data.action as string} />
      </div>
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

function JsonSection({ title, data, defaultOpen }: { title: string; data: unknown; defaultOpen: boolean }) {
  const [isOpen, setIsOpen] = useState(defaultOpen)

  if (data === null || data === undefined) return null

  // Filter out null/undefined values from objects
  const cleanData = typeof data === 'object' && !Array.isArray(data)
    ? Object.fromEntries(Object.entries(data as Record<string, unknown>).filter(([, v]) => v != null))
    : data

  if (typeof cleanData === 'object' && !Array.isArray(cleanData) && Object.keys(cleanData as Record<string, unknown>).length === 0) return null

  return (
    <Collapsible open={isOpen} onOpenChange={setIsOpen}>
      <CollapsibleTrigger className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors">
        <ChevronRight className={cn('h-3 w-3 transition-transform', isOpen && 'rotate-90')} />
        <span className="font-medium">{title}</span>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <pre className="mt-1 p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[400px] overflow-auto">
          {JSON.stringify(cleanData, null, 2)}
        </pre>
      </CollapsibleContent>
    </Collapsible>
  )
}
