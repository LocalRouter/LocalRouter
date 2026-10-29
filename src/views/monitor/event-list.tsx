import { Fragment, useCallback, useLayoutEffect, useMemo, useRef } from 'react'
import { eventRelationships } from './event-relationships'
import { singleLinePreview } from './monitor-events'
import { EventDuration } from './event-duration'
import { LLM_API_LABELS, LLM_API_SHORT, llmApiFlow } from './llm-api'
import { cn } from '@/lib/utils'
import { Wrench, Shield, GitBranch, Link, AlertTriangle, Loader2, CheckCircle2, XCircle, KeyRound, Gauge, AlertCircle, Server, Ban, Minimize2, Database, Forward } from 'lucide-react'
import { ProvidersIcon } from '@/components/icons/category-icons'
import type { MonitorEventSummary, MonitorEventType, EventStatus } from '@/types/tauri-commands'

interface EventListProps {
  grouping?: 'events' | 'conversations'
  events: MonitorEventSummary[]
  showType: boolean
  selectedId: string | null
  onSelect: (id: string) => void
  onToggleConversationFilter: (id: string) => void
  conversationFiltered?: boolean
  /** A filter narrows the list, so an empty list means "no matches", not "no events". */
  filtered?: boolean
}

const categoryConfig: Record<string, { icon: React.ComponentType<{ className?: string }>; color: string }> = {
  llm: { icon: ProvidersIcon, color: 'text-blue-500' },
  mcp: { icon: Wrench, color: 'text-green-500' },
  mcp_server: { icon: Server, color: 'text-emerald-500' },
  guardrail: { icon: Shield, color: 'text-red-500' },
  secret_scan: { icon: KeyRound, color: 'text-orange-500' },
  routing: { icon: GitBranch, color: 'text-purple-500' },
  optimization: { icon: Minimize2, color: 'text-blue-500' },
  memory: { icon: Database, color: 'text-amber-500' },
  firewall: { icon: Shield, color: 'text-red-500' },
  connection: { icon: Link, color: 'text-gray-500' },
  // Non-LLM traffic that only passed through — muted, and flagged amber
  // because it is usually accidental.
  proxy: { icon: Forward, color: 'text-amber-600' },
  auth: { icon: KeyRound, color: 'text-red-500' },
  rate_limit: { icon: Gauge, color: 'text-amber-500' },
  validation: { icon: AlertCircle, color: 'text-yellow-600' },
  internal: { icon: AlertTriangle, color: 'text-red-600' },
  moderation: { icon: Ban, color: 'text-orange-600' },
}

function getCategory(type: MonitorEventType): string {
  if (type === 'llm_call') return 'llm'
  if (type === 'mcp_server_event') return 'mcp_server'
  if (type.startsWith('mcp_')) return 'mcp'
  if (type.startsWith('guardrail')) return 'guardrail'
  if (type === 'secret_scan') return 'secret_scan'
  if (type === 'route_llm_classify' || type === 'routing_decision') return 'routing'
  if (type === 'prompt_compression' || type === 'json_repair') return 'optimization'
  if (type === 'memory_compaction') return 'memory'
  if (type === 'firewall_decision') return 'firewall'
  if (type === 'sse_connection' || type === 'connection_error') return 'connection'
  if (type === 'proxy_passthrough') return 'proxy'
  if (type === 'auth_error' || type === 'access_denied' || type === 'oauth_event') return 'auth'
  if (type === 'rate_limit_event') return 'rate_limit'
  if (type === 'validation_error') return 'validation'
  if (type === 'internal_error') return 'internal'
  if (type === 'moderation_event') return 'moderation'
  return 'llm'
}

function getTypeLabel(type: MonitorEventType): string {
  return type
    .replace(/_/g, ' ')
    .replace(/\b\w/g, l => l.toUpperCase())
    .replace('Llm', 'LLM')
    .replace('Json', 'JSON')
    .replace('Mcp', 'MCP')
    .replace('Sse', 'SSE')
    .replace('Oauth', 'OAuth')
}

function StatusBadge({ status }: { status: EventStatus }) {
  switch (status) {
    case 'pending':
      return <Loader2 className="h-3 w-3 animate-spin text-yellow-500" />
    case 'complete':
      return <CheckCircle2 className="h-3 w-3 text-green-500" />
    case 'error':
      return <XCircle className="h-3 w-3 text-red-500" />
  }
}

function formatTime(timestamp: string): { short: string; full: string } {
  const date = new Date(timestamp)
  const h = String(date.getHours()).padStart(2, '0')
  const m = String(date.getMinutes()).padStart(2, '0')
  const s = String(date.getSeconds()).padStart(2, '0')
  const ms = String(date.getMilliseconds()).padStart(3, '0')
  return {
    short: `${h}:${m}:${s}.${ms}`,
    full: date.toLocaleString(),
  }
}

/** "Responses → Messages" when translated, otherwise the client's API. */
function ApiCell({ event }: { event: MonitorEventSummary }) {
  if (event.event_type !== 'llm_call') return null
  const flow = llmApiFlow(event)
  if (!flow.client) return null
  const title = flow.translated
    ? `${LLM_API_LABELS[flow.client]} translated to ${LLM_API_LABELS[flow.upstream!]}`
    : LLM_API_LABELS[flow.client]
  return (
    <span className="flex min-w-0 items-center gap-1 truncate text-muted-foreground" title={title}>
      <span className="truncate">{LLM_API_SHORT[flow.client]}</span>
      {flow.translated && (
        <span className="truncate text-amber-600 dark:text-amber-400">→ {LLM_API_SHORT[flow.upstream!]}</span>
      )}
    </span>
  )
}

export function EventList({ grouping = 'events', events, showType, selectedId, onSelect, onToggleConversationFilter, conversationFiltered = false, filtered = false }: EventListProps) {
  const relationships = useMemo(() => eventRelationships(events), [events])
  const selectedGroup = grouping === 'events' ? relationships.find(group => group.members.has(selectedId ?? '')) : undefined
  const rows = grouping === 'conversations' ? relationships.flatMap(group => group.events) : events
  const first = selectedGroup ? rows.findIndex(e => selectedGroup.members.has(e.id)) : -1
  const last = selectedGroup ? rows.reduce((last, e, i) => selectedGroup.members.has(e.id) ? i : last, -1) : -1
  const scrollRef = useRef<HTMLDivElement>(null)
  const selectedRowRef = useRef<HTMLTableRowElement>(null)
  const anchorRef = useRef<{ id: string; top: number; scrollTop: number; height: number } | null>(null)

  const rememberAnchor = useCallback(() => {
    const container = scrollRef.current
    const row = selectedRowRef.current
    anchorRef.current = null
    if (grouping !== 'conversations' || !selectedId || !container || !row) return
    const viewport = container.getBoundingClientRect()
    const bounds = row.getBoundingClientRect()
    // Keep a layout snapshot even offscreen: a user scroll can happen before
    // its scroll event fires, so the next update must account for scrollTop.
    anchorRef.current = { id: selectedId, top: bounds.top - viewport.top, scrollTop: container.scrollTop, height: bounds.height }
  }, [grouping, selectedId])

  useLayoutEffect(() => {
    const container = scrollRef.current
    const row = selectedRowRef.current
    const anchor = anchorRef.current
    if (grouping === 'conversations' && anchor?.id === selectedId && container && row) {
      // React may move the entire group or prepend rows. Compensate before
      // paint so the event being read stays put, within the scroll bounds.
      const viewport = container.getBoundingClientRect()
      const previousTop = anchor.top - (container.scrollTop - anchor.scrollTop)
      const headerHeight = container.querySelector('thead')?.getBoundingClientRect().height ?? 0
      // Only hold a row the reader could see. Scrolling away must never snap
      // them back, including arrivals before the pending scroll event fires.
      if (previousTop + anchor.height > headerHeight && previousTop < viewport.height) {
        const top = row.getBoundingClientRect().top - viewport.top
        container.scrollTop += top - previousTop
      }
    }
    rememberAnchor()
  }, [events, grouping, selectedId, rememberAnchor])

  if (events.length === 0) {
    return (
      <div className="flex h-full items-center justify-center text-muted-foreground text-sm">
        <div className="text-center">
          <AlertTriangle className="mx-auto h-8 w-8 mb-2 opacity-50" />
          {filtered ? (
            <>
              <p>No events match these filters</p>
              <p className="text-xs mt-1">Matching events will appear here as they happen</p>
            </>
          ) : (
            <>
              <p>No events captured yet</p>
              <p className="text-xs mt-1">Events will appear here as requests flow through LocalRouter</p>
            </>
          )}
        </div>
      </div>
    )
  }

  return (
    // Columns drop out as the list narrows (e.g. with the detail docked beside it).
    <div className="flex flex-col h-full">
      <div ref={scrollRef} onScroll={rememberAnchor} style={grouping === 'conversations' ? { overflowAnchor: 'none' } : undefined} className="overflow-auto flex-1 min-h-0 [container-type:inline-size]">
      <table className="w-full table-fixed text-xs">
        <thead className="sticky top-0 bg-background border-b z-10">
          <tr className="text-left text-muted-foreground">
            {grouping === 'events' && <th className="w-[24px]"><span className="sr-only">Related events</span></th>}
            <th className="px-2 py-1.5 w-[24px]"></th>
            <th className="px-2 py-1.5 w-[105px]">Time</th>
            {showType && <th className="px-2 py-1.5 w-[140px]">Type</th>}
            <th className={'px-2 py-1.5 w-[100px] hidden [@container(min-width:480px)]:table-cell'}>Client</th>
            <th className="px-2 py-1.5">Question</th>
            <th className={'px-2 py-1.5 hidden [@container(min-width:760px)]:table-cell'}>Answer</th>
            <th className={'px-2 py-1.5 w-[176px] hidden [@container(min-width:760px)]:table-cell'}>API</th>
            <th className="px-2 py-1.5 w-[76px] text-right">Duration</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((event, index) => {
            const category = getCategory(event.event_type)
            const config = categoryConfig[category] ?? categoryConfig.llm
            const Icon = config.icon
            const isSelected = selectedId === event.id

            const header = grouping === 'conversations' ? relationships.find(group => group.events[0].id === event.id) : undefined
            return (
              <Fragment key={event.id}>
              {header && <tr data-testid="conversation-header" className="border-b bg-muted/50">
                <th colSpan={showType ? 8 : 7} scope="rowgroup" className="px-3 py-2 text-left font-medium" title={header.description}>
                  <div className="truncate">{header.title}</div>
                </th>
              </tr>}
              <tr
                ref={isSelected ? selectedRowRef : undefined}
                data-event-id={event.id}
                onClick={() => onSelect(event.id)}
                tabIndex={0}
                onKeyDown={e => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault()
                    onSelect(event.id)
                  }
                }}
                aria-selected={isSelected}
                className={cn(
                  'cursor-pointer transition-colors border-b border-border/50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary',
                  isSelected ? 'bg-primary/5 hover:bg-primary/10' : 'hover:bg-accent/50'
                )}
              >
                {grouping === 'events' && <td className="relative p-0 text-primary" data-testid="event-links" data-related={selectedGroup?.members.has(event.id) || undefined}>
                  {selectedGroup && selectedGroup.members.size > 1 && index >= first && index <= last && <>
                    <span aria-hidden="true" className="absolute left-1/2 w-[2px] -translate-x-1/2 bg-primary/20" style={{ top: index === first ? '50%' : 0, bottom: index === last ? '50%' : 0 }} />
                    {selectedGroup.members.has(event.id) && <button
                      type="button"
                      aria-label={`Toggle conversation filter: ${selectedGroup.title}`}
                      aria-pressed={conversationFiltered}
                      title={conversationFiltered ? 'Show all conversations' : 'Filter to this conversation'}
                      className="absolute left-1/2 top-1/2 flex h-5 w-5 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary"
                      onClick={e => { e.stopPropagation(); onToggleConversationFilter(event.id) }}
                      onKeyDown={e => e.stopPropagation()}
                    >
                      <span aria-hidden="true" className={cn('h-1.5 w-1.5 rounded-full bg-primary/35 ring-1 ring-background', isSelected && 'h-2 w-2 bg-primary/50')} />
                    </button>}
                    {selectedGroup.members.has(event.id) && <span className="sr-only">Related to selected event</span>}
                  </>}
                </td>}
                <td className="px-2 py-1 border-l-2 border-l-transparent">
                  <span title={event.duplicate_hop != null ? `Duplicate hop ${event.duplicate_hop} — not counted in stats` : event.status} aria-label={event.status}>
                    {event.duplicate_hop != null ? <AlertTriangle className="h-3 w-3 text-amber-500" /> : <StatusBadge status={event.status} />}
                  </span>
                </td>
                <td className="px-2 py-1 font-mono text-muted-foreground whitespace-nowrap" title={formatTime(event.timestamp).full}>
                  {formatTime(event.timestamp).short}
                </td>
                {showType && <td className="px-2 py-1">
                  <div className="flex items-center gap-1">
                    <Icon className={cn('h-3 w-3 shrink-0', config.color)} />
                    <span className="truncate">{getTypeLabel(event.event_type)}</span>
                  </div>
                </td>}
                <td className={'px-2 py-1 truncate text-muted-foreground hidden [@container(min-width:480px)]:table-cell'}>
                  {event.client_name || event.client_id?.slice(0, 8) || '—'}
                </td>
                <td className="px-2 py-1 truncate" title={singleLinePreview(event.question)}>
                  {singleLinePreview(event.question)}
                </td>
                <td className={cn('px-2 py-1 truncate hidden [@container(min-width:760px)]:table-cell', event.status === 'error' ? 'text-red-600 dark:text-red-400' : 'text-muted-foreground')} title={singleLinePreview(event.answer)}>
                  {singleLinePreview(event.answer)}
                </td>
                <td className={'px-2 py-1 hidden [@container(min-width:760px)]:table-cell'}>
                  <ApiCell event={event} />
                </td>
                <td className="px-2 py-1 text-right font-mono text-muted-foreground">
                  <EventDuration event={event} />
                </td>
              </tr>
              </Fragment>
            )
          })}
        </tbody>
      </table>
      </div>
    </div>
  )
}
