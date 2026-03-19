import { cn } from '@/lib/utils'
import { Activity, Wrench, Shield, GitBranch, Zap, Link, AlertTriangle, Loader2, CheckCircle2, XCircle, KeyRound, Gauge, AlertCircle, Server, Ban, ChevronRight, ChevronDown } from 'lucide-react'
import { useState, useMemo } from 'react'
import type { MonitorEventSummary, MonitorEventType, EventStatus } from '@/types/tauri-commands'

interface EventListProps {
  events: MonitorEventSummary[]
  selectedId: string | null
  onSelect: (id: string) => void
}

const categoryConfig: Record<string, { icon: typeof Activity; color: string }> = {
  llm: { icon: Activity, color: 'text-blue-500' },
  mcp: { icon: Wrench, color: 'text-green-500' },
  mcp_server: { icon: Server, color: 'text-emerald-500' },
  security: { icon: Shield, color: 'text-orange-500' },
  routing: { icon: GitBranch, color: 'text-purple-500' },
  optimization: { icon: Zap, color: 'text-yellow-500' },
  firewall: { icon: Shield, color: 'text-red-500' },
  connection: { icon: Link, color: 'text-gray-500' },
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
  if (type.startsWith('guardrail') || type === 'secret_scan') return 'security'
  if (type === 'route_llm_classify' || type === 'routing_decision') return 'routing'
  if (type === 'prompt_compression') return 'optimization'
  if (type === 'firewall_decision') return 'firewall'
  if (type === 'sse_connection' || type === 'connection_error') return 'connection'
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

type SessionGroup = {
  kind: 'group'
  sessionId: string
  header: MonitorEventSummary
  children: MonitorEventSummary[]
} | {
  kind: 'standalone'
  event: MonitorEventSummary
}

function EventRow({
  event,
  selectedId,
  onSelect,
  indent,
}: {
  event: MonitorEventSummary
  selectedId: string | null
  onSelect: (id: string) => void
  indent?: boolean
}) {
  const category = getCategory(event.event_type)
  const config = categoryConfig[category] ?? categoryConfig.llm
  const Icon = config.icon

  return (
    <tr
      key={event.id}
      onClick={() => onSelect(event.id)}
      className={cn(
        'cursor-pointer hover:bg-accent/50 transition-colors border-b border-border/50',
        selectedId === event.id && 'bg-accent'
      )}
    >
      <td className="px-2 py-1 font-mono text-muted-foreground whitespace-nowrap" title={formatTime(event.timestamp).full}>
        {indent && <span className="inline-block w-4" />}
        {formatTime(event.timestamp).short}
      </td>
      <td className="px-2 py-1">
        <div className="flex items-center gap-1">
          <Icon className={cn('h-3 w-3 shrink-0', config.color)} />
          <span className="truncate">{getTypeLabel(event.event_type)}</span>
        </div>
      </td>
      <td className="px-2 py-1 truncate text-muted-foreground">
        {event.client_name || event.client_id?.slice(0, 8) || '—'}
      </td>
      <td className="px-2 py-1 truncate" title={event.summary}>
        {event.summary}
      </td>
      <td className="px-2 py-1">
        <StatusBadge status={event.status} />
      </td>
      <td className="px-2 py-1 text-right font-mono text-muted-foreground">
        {event.duration_ms != null ? `${event.duration_ms}ms` : '—'}
      </td>
    </tr>
  )
}

function SessionGroupRows({
  group,
  selectedId,
  onSelect,
}: {
  group: Extract<SessionGroup, { kind: 'group' }>
  selectedId: string | null
  onSelect: (id: string) => void
}) {
  const [expanded, setExpanded] = useState(false)
  const header = group.header
  const category = getCategory(header.event_type)
  const config = categoryConfig[category] ?? categoryConfig.llm
  const Icon = config.icon

  return (
    <>
      <tr
        onClick={() => onSelect(header.id)}
        className={cn(
          'cursor-pointer hover:bg-accent/50 transition-colors border-b border-border/50',
          selectedId === header.id && 'bg-accent'
        )}
      >
        <td className="px-2 py-1 font-mono text-muted-foreground whitespace-nowrap" title={formatTime(header.timestamp).full}>
          <button
            type="button"
            onClick={(e) => { e.stopPropagation(); setExpanded(!expanded) }}
            className="inline-flex items-center justify-center w-4 h-4 mr-0.5 hover:bg-accent rounded"
          >
            {expanded
              ? <ChevronDown className="h-3 w-3" />
              : <ChevronRight className="h-3 w-3" />
            }
          </button>
          {formatTime(header.timestamp).short}
        </td>
        <td className="px-2 py-1">
          <div className="flex items-center gap-1">
            <Icon className={cn('h-3 w-3 shrink-0', config.color)} />
            <span className="truncate">{getTypeLabel(header.event_type)}</span>
            <span className="ml-1 inline-flex items-center justify-center rounded-full bg-muted px-1.5 py-0 text-[10px] font-medium text-muted-foreground">
              +{group.children.length}
            </span>
          </div>
        </td>
        <td className="px-2 py-1 truncate text-muted-foreground">
          {header.client_name || header.client_id?.slice(0, 8) || '—'}
        </td>
        <td className="px-2 py-1 truncate" title={header.summary}>
          {header.summary}
        </td>
        <td className="px-2 py-1">
          <StatusBadge status={header.status} />
        </td>
        <td className="px-2 py-1 text-right font-mono text-muted-foreground">
          {header.duration_ms != null ? `${header.duration_ms}ms` : '—'}
        </td>
      </tr>
      {expanded && group.children.map(child => (
        <EventRow
          key={child.id}
          event={child}
          selectedId={selectedId}
          onSelect={onSelect}
          indent
        />
      ))}
    </>
  )
}

export function EventList({ events, selectedId, onSelect }: EventListProps) {
  const groups = useMemo<SessionGroup[]>(() => {
    const sessionMap = new Map<string, MonitorEventSummary[]>()
    const result: SessionGroup[] = []

    // Collect session groups maintaining order of first appearance
    for (const event of events) {
      if (!event.session_id) {
        result.push({ kind: 'standalone', event })
      } else {
        const existing = sessionMap.get(event.session_id)
        if (existing) {
          existing.push(event)
        } else {
          const group: MonitorEventSummary[] = [event]
          sessionMap.set(event.session_id, group)
          // Push placeholder — we'll resolve it below
          result.push({ kind: 'group', sessionId: event.session_id, header: event, children: [] })
        }
      }
    }

    // Finalize groups: single-event sessions become standalone
    return result.map(item => {
      if (item.kind === 'standalone') return item
      const all = sessionMap.get(item.sessionId)!
      if (all.length === 1) {
        return { kind: 'standalone' as const, event: all[0] }
      }
      return { kind: 'group' as const, sessionId: item.sessionId, header: all[0], children: all.slice(1) }
    })
  }, [events])

  if (events.length === 0) {
    return (
      <div className="flex h-full items-center justify-center text-muted-foreground text-sm">
        <div className="text-center">
          <AlertTriangle className="mx-auto h-8 w-8 mb-2 opacity-50" />
          <p>No events captured yet</p>
          <p className="text-xs mt-1">Events will appear here as requests flow through LocalRouter</p>
        </div>
      </div>
    )
  }

  return (
    <div className="overflow-auto h-full">
      <table className="w-full text-xs">
        <thead className="sticky top-0 bg-background border-b z-10">
          <tr className="text-left text-muted-foreground">
            <th className="px-2 py-1.5 w-[105px]">Time</th>
            <th className="px-2 py-1.5 w-[140px]">Type</th>
            <th className="px-2 py-1.5 w-[100px]">Client</th>
            <th className="px-2 py-1.5">Summary</th>
            <th className="px-2 py-1.5 w-[24px]"></th>
            <th className="px-2 py-1.5 w-[60px] text-right">Duration</th>
          </tr>
        </thead>
        <tbody>
          {groups.map(group => {
            if (group.kind === 'standalone') {
              return (
                <EventRow
                  key={group.event.id}
                  event={group.event}
                  selectedId={selectedId}
                  onSelect={onSelect}
                />
              )
            }
            return (
              <SessionGroupRows
                key={group.sessionId}
                group={group}
                selectedId={selectedId}
                onSelect={onSelect}
              />
            )
          })}
        </tbody>
      </table>
    </div>
  )
}
