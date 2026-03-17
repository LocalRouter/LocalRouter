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
        {type === 'llm_request' && <LlmRequestDetail data={data} />}
        {type === 'llm_request_transformed' && <LlmRequestTransformedDetail data={data} />}
        {type === 'llm_response' && <LlmResponseDetail data={data} />}
        {type === 'llm_error' && <LlmErrorDetail data={data} />}
        {type === 'mcp_tool_call' && <McpToolCallDetail data={data} />}
        {type === 'mcp_tool_response' && <McpToolResponseDetail data={data} />}
        {(type === 'mcp_resource_read' || type === 'mcp_prompt_get') && <McpRequestDetail data={data} />}
        {(type === 'mcp_resource_response' || type === 'mcp_prompt_response') && <McpResponseDetail data={data} />}
        {(type === 'guardrail_request' || type === 'guardrail_response') && <GuardrailDetail data={data} />}
        {(type === 'secret_scan_request' || type === 'secret_scan_response') && <SecretScanDetail data={data} />}
        {(type === 'route_llm_request' || type === 'route_llm_response' || type === 'routing_decision') && <RoutingDetail data={data} />}
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

function LlmRequestDetail({ data }: { data: EventData }) {
  const body = data.request_body as Record<string, unknown> | undefined
  const messages = body?.messages as Array<Record<string, unknown>> | undefined
  const tools = body?.tools as Array<Record<string, unknown>> | undefined

  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Endpoint" value={data.endpoint as string} />
        <Field label="Model" value={data.model as string} />
        <Field label="Stream" value={String(data.stream)} />
        <Field label="Messages" value={String(data.message_count)} />
        {(data.has_tools as boolean) && <Field label="Tools" value={String(data.tool_count)} />}
      </div>

      {messages && messages.length > 0 && (
        <MessagesSection messages={messages} />
      )}

      {tools && tools.length > 0 && (
        <ToolsSection tools={tools} />
      )}

      {/* Parameters */}
      {body && (
        <JsonSection title="Parameters" data={{
          temperature: body.temperature,
          max_tokens: body.max_tokens,
          top_p: body.top_p,
          frequency_penalty: body.frequency_penalty,
          presence_penalty: body.presence_penalty,
          seed: body.seed,
        }} defaultOpen={false} />
      )}

      {body && (
        <JsonSection title="Full Request Body" data={body} defaultOpen={false} />
      )}
    </div>
  )
}

function LlmRequestTransformedDetail({ data }: { data: EventData }) {
  const body = data.request_body as Record<string, unknown> | undefined
  const messages = body?.messages as Array<Record<string, unknown>> | undefined
  const tools = body?.tools as Array<Record<string, unknown>> | undefined
  const transformations = data.transformations_applied as string[] | undefined

  return (
    <div className="space-y-2">
      {transformations && transformations.length > 0 && (
        <div className="flex items-center gap-1.5 flex-wrap">
          <span className="text-xs text-muted-foreground font-medium">Applied:</span>
          {transformations.map((t, i) => (
            <Badge key={i} variant="secondary" className="text-[10px]">{t}</Badge>
          ))}
        </div>
      )}

      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Endpoint" value={data.endpoint as string} />
        <Field label="Model" value={data.model as string} />
        <Field label="Stream" value={String(data.stream)} />
        <Field label="Messages" value={String(data.message_count)} />
        {(data.has_tools as boolean) && <Field label="Tools" value={String(data.tool_count)} />}
      </div>

      {messages && messages.length > 0 && (
        <MessagesSection messages={messages} />
      )}

      {tools && tools.length > 0 && (
        <ToolsSection tools={tools} />
      )}

      {body && (
        <JsonSection title="Full Request Body" data={body} defaultOpen={false} />
      )}
    </div>
  )
}

function LlmResponseDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Provider" value={data.provider as string} />
        <Field label="Model" value={data.model as string} />
        <Field label="Status" value={String(data.status_code)} />
        <Field label="Streamed" value={String(data.streamed)} />
      </div>

      {/* Token usage */}
      <div className="grid grid-cols-3 gap-2 text-xs">
        <Field label="Input Tokens" value={String(data.input_tokens)} />
        <Field label="Output Tokens" value={String(data.output_tokens)} />
        <Field label="Total Tokens" value={String(data.total_tokens)} />
      </div>

      {data.cost_usd != null && (
        <div className="grid grid-cols-2 gap-2 text-xs">
          <Field label="Cost" value={`$${(data.cost_usd as number).toFixed(6)}`} />
          <Field label="Latency" value={`${String(data.latency_ms)}ms`} />
        </div>
      )}

      {data.finish_reason && <Field label="Finish Reason" value={data.finish_reason as string} />}

      {/* Content preview */}
      {data.content_preview && (
        <div className="text-xs">
          <span className="text-muted-foreground font-medium">Content:</span>
          <pre className="mt-1 p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[300px] overflow-auto">
            {data.content_preview as string}
          </pre>
        </div>
      )}
    </div>
  )
}

function LlmErrorDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Provider" value={data.provider as string} />
        <Field label="Model" value={data.model as string} />
        <Field label="Status Code" value={String(data.status_code)} />
      </div>
      <div className="text-xs">
        <span className="text-muted-foreground font-medium">Error:</span>
        <pre className="mt-1 p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
          {data.error as string}
        </pre>
      </div>
    </div>
  )
}

function McpToolCallDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Tool" value={data.tool_name as string} />
        <div className="flex items-center gap-1 text-xs">
          <Server className="h-3 w-3 text-muted-foreground" />
          <span className="text-muted-foreground">Server:</span>
          <span>{(data.server_name || data.server_id) as string}</span>
        </div>
      </div>
      {data.firewall_action && (
        <Field label="Firewall" value={data.firewall_action as string} />
      )}
      <JsonSection title="Arguments" data={data.arguments as unknown} defaultOpen={true} />
    </div>
  )
}

function McpToolResponseDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Tool" value={data.tool_name as string} />
        <Field label="Success" value={String(data.success)} />
        <Field label="Latency" value={`${data.latency_ms}ms`} />
      </div>
      {data.error && (
        <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
          {data.error as string}
        </pre>
      )}
      {data.response_preview && (
        <div className="text-xs">
          <span className="text-muted-foreground font-medium">Response:</span>
          <pre className="mt-1 p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[300px] overflow-auto">
            {data.response_preview as string}
          </pre>
        </div>
      )}
    </div>
  )
}

function McpRequestDetail({ data }: { data: EventData }) {
  const name = (data.uri || data.prompt_name || data.tool_name) as string
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Name" value={name} />
        <Field label="Server" value={(data.server_name || data.server_id) as string} />
      </div>
      {data.arguments && <JsonSection title="Arguments" data={data.arguments as unknown} defaultOpen={true} />}
    </div>
  )
}

function McpResponseDetail({ data }: { data: EventData }) {
  const name = (data.uri || data.prompt_name || data.tool_name) as string
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <Field label="Name" value={name} />
        <Field label="Success" value={String(data.success)} />
        <Field label="Latency" value={`${data.latency_ms}ms`} />
      </div>
      {data.error && (
        <pre className="p-2 bg-destructive/10 rounded text-xs whitespace-pre-wrap text-destructive">
          {data.error as string}
        </pre>
      )}
      {data.content_preview && (
        <pre className="p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[300px] overflow-auto">
          {data.content_preview as string}
        </pre>
      )}
    </div>
  )
}

function GuardrailDetail({ data }: { data: EventData }) {
  const categories = data.flagged_categories as Array<Record<string, unknown>> | undefined
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        {data.direction && <Field label="Direction" value={data.direction as string} />}
        {data.result && <Field label="Result" value={data.result as string} />}
        {data.action_taken && <Field label="Action" value={data.action_taken as string} />}
        {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
      </div>
      {data.text_preview && (
        <pre className="p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[100px] overflow-auto">
          {data.text_preview as string}
        </pre>
      )}
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
      {data.models_used && (
        <Field label="Models" value={(data.models_used as string[]).join(', ')} />
      )}
    </div>
  )
}

function SecretScanDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        {data.findings_count != null && <Field label="Findings" value={String(data.findings_count)} />}
        {data.action_taken && <Field label="Action" value={data.action_taken as string} />}
        {data.rules_count != null && <Field label="Rules" value={String(data.rules_count)} />}
        {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
      </div>
      {data.text_preview && (
        <pre className="p-2 bg-muted rounded text-xs whitespace-pre-wrap max-h-[100px] overflow-auto">
          {data.text_preview as string}
        </pre>
      )}
      {data.findings && <JsonSection title="Findings" data={data.findings as unknown} defaultOpen={true} />}
    </div>
  )
}

function RoutingDetail({ data }: { data: EventData }) {
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 gap-2 text-xs">
        {data.routing_type && <Field label="Type" value={data.routing_type as string} />}
        {data.original_model && <Field label="Original Model" value={data.original_model as string} />}
        {data.final_model && <Field label="Final Model" value={data.final_model as string} />}
        {data.routed_model && <Field label="Routed Model" value={data.routed_model as string} />}
        {data.win_rate != null && <Field label="Win Rate" value={((data.win_rate as number) * 100).toFixed(1) + '%'} />}
        {data.threshold != null && <Field label="Threshold" value={String(data.threshold)} />}
        {data.selected_tier && <Field label="Tier" value={data.selected_tier as string} />}
        {data.firewall_action && <Field label="Firewall" value={data.firewall_action as string} />}
        {data.latency_ms != null && <Field label="Latency" value={`${data.latency_ms}ms`} />}
      </div>
      {data.candidate_models && (
        <Field label="Candidates" value={(data.candidate_models as string[]).join(', ')} />
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
