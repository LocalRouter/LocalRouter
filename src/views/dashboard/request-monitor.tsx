import { memo, useState, useEffect, useMemo, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/Button'
import { Popover, PopoverTrigger, PopoverContent } from '@/components/ui/popover'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { ArrowDownWideNarrow, PanelRight, X } from 'lucide-react'
import { ResizablePanelGroup, ResizablePanel, ResizableHandle } from '@/components/ui/resizable'
import { cn } from '@/lib/utils'
import { useMonitorEvents } from '@/views/monitor/hooks/useMonitorEvents'
import { showEventTypeColumn } from '@/views/monitor/monitor-events'
import { eventRelationships } from '@/views/monitor/event-relationships'
import { EventList } from '@/views/monitor/event-list'
import { EventDetail } from '@/views/monitor/event-detail'
import { EventFilters } from '@/views/monitor/event-filters'
import { TryItOutPanel } from '@/views/monitor/try-it-out-panel'
import type { ReplayInput, ReplayDataset, ReplayImportParams, ReplayEndpoint, MonitorEventFilter, InterceptRule } from '@/types/tauri-commands'
import { readUiState, writeUiState } from '@/hooks/usePersistentUiState'
import { DetailDockControls, DockDropPreview, useDetailDock } from './detail-dock'

// Persists the deliberate filter dimensions across app restarts. We do NOT
// persist `session_id`/`client_id` — those are transient drill-downs (set by
// clicking into a session/client) and a stale value would silently hide all
// events after a restart, since the in-memory event store starts empty.
const FILTER_STORAGE_KEY = 'monitor.filter'

const EMPTY_FILTER: MonitorEventFilter = {
  event_types: null,
  session_id: null,
  client_id: null,
  status: null,
  search: null,
}

function loadPersistedFilter(): MonitorEventFilter {
  const saved = readUiState<Partial<MonitorEventFilter> | null>(FILTER_STORAGE_KEY, null, (value) =>
    value && typeof value === 'object' && !Array.isArray(value) ? (value as Partial<MonitorEventFilter>) : undefined,
  )
  if (!saved) return EMPTY_FILTER
  return {
    ...EMPTY_FILTER,
    event_types: Array.isArray(saved.event_types) ? saved.event_types : null,
    status: typeof saved.status === 'string' ? saved.status : null,
    search: typeof saved.search === 'string' ? saved.search : null,
  }
}

interface RequestMonitorProps {
  className?: string
}

// Memoized: the traffic chart's live updates re-render the dashboard often.
export const RequestMonitor = memo(function RequestMonitor({ className }: RequestMonitorProps) {
  const [grouping, setGrouping] = useState<'events' | 'conversations'>('events')
  const [orderingOpen, setOrderingOpen] = useState(false)
  const [conversationFilter, setConversationFilter] = useState<{ keys: string[]; title: string } | null>(null)
  const [filter, setFilter] = useState<MonitorEventFilter>(loadPersistedFilter)
  const [replayDraft, setReplayDraft] = useState<{ input: ReplayInput; clientId?: string } | null>(null)
  const [replayError, setReplayError] = useState('')
  const [tryItOutOpen, setTryItOutOpen] = useState(false)
  const [interceptRule, setInterceptRule] = useState<InterceptRule | null>(null)

  // Persist the deliberate filter dimensions whenever they change so the
  // dashboard restores the user's last filter on the next launch.
  useEffect(() => {
    writeUiState(FILTER_STORAGE_KEY, {
      event_types: filter.event_types,
      status: filter.status,
      search: filter.search,
    })
  }, [filter.event_types, filter.status, filter.search])

  // Sync intercept rule to backend
  useEffect(() => {
    invoke('set_monitor_intercept_rule', { rule: interceptRule }).catch(console.error)
  }, [interceptRule])

  // Clear intercept rule on unmount (navigating away from the dashboard)
  useEffect(() => {
    return () => {
      invoke('set_monitor_intercept_rule', { rule: null }).catch(console.error)
    }
  }, [])

  // Only pass filter to backend if it has actual values
  const activeFilter = useMemo(() => {
    const hasFilter = filter.event_types || filter.session_id || filter.client_id || filter.status || filter.search
    return hasFilter ? filter : null
  }, [filter])

  const {
    events,
    selectedEvent,
    selectedId,
    isLoading,
    loadError,
    reload,
    isDetailLoading,
    detailError,
    retryDetail,
    selectEvent,
  } = useMonitorEvents(activeFilter)

  const relationships = useMemo(() => eventRelationships(events), [events])
  const selectedGroup = relationships.find(group => group.members.has(selectedId ?? ''))
  const visibleEvents = useMemo(() => {
    if (!conversationFilter || grouping === 'conversations') return events
    const matching = new Set(relationships.filter(group => group.keys.some(key => conversationFilter.keys.includes(key))).flatMap(group => [...group.members]))
    return events.filter(event => matching.has(event.id))
  }, [events, relationships, conversationFilter, grouping])

  const bodyRef = useRef<HTMLDivElement>(null)
  const { dock, setDock, drag, startDrag, detailSize, saveLayout } = useDetailDock(bodyRef)

  const filterBar = (
    <div className="flex items-center border-b">
      <div className="flex-1 min-w-0">
        <EventFilters
          groupingControl={<Popover open={orderingOpen} onOpenChange={setOrderingOpen}>
            <PopoverTrigger asChild>
              <Button variant="outline" size="sm" className="h-7 text-xs gap-1" aria-label={`Order: ${grouping === 'events' ? 'By timestamp' : 'By conversation'}`}>
                <ArrowDownWideNarrow className="h-3 w-3" />
                {grouping === 'events' ? 'By timestamp' : 'By conversation'}
              </Button>
            </PopoverTrigger>
            <PopoverContent align="start" className="w-48 p-3">
              <p className="mb-3 text-xs font-medium">Order events</p>
              <RadioGroup aria-label="Order events" value={grouping} onValueChange={value => {
                const mode = value === 'conversations' ? 'conversations' : 'events'
                setGrouping(mode)
                if (mode === 'conversations') {
                  setConversationFilter(null)
                  setFilter(current => current.session_id == null ? current : { ...current, session_id: null })
                }
                setOrderingOpen(false)
              }}>
                <label className="flex items-center gap-2 text-xs cursor-pointer">
                  <RadioGroupItem value="events" />By timestamp
                </label>
                <label className="flex items-center gap-2 text-xs cursor-pointer">
                  <RadioGroupItem value="conversations" />By conversation
                </label>
              </RadioGroup>
            </PopoverContent>
          </Popover>}
          conversationControl={grouping === 'events' && (conversationFilter ? <Button size="sm" variant="secondary" className="h-7 max-w-[220px] gap-1 text-xs" title={conversationFilter.title} aria-label={`Dismiss conversation filter: ${conversationFilter.title}`} onClick={() => setConversationFilter(null)}>
            <span className="truncate">{conversationFilter.title}</span><X className="h-3 w-3 shrink-0" />
          </Button> : <Button size="sm" variant="ghost" className="h-7 text-xs" disabled={!selectedGroup} onClick={() => selectedGroup && setConversationFilter({ keys: selectedGroup.keys, title: selectedGroup.title })}>
            Filter to this conversation
          </Button>)}
          filter={filter}
          onFilterChange={setFilter}
          interceptRule={interceptRule}
          onInterceptRuleChange={setInterceptRule}
        />
      </div>
      <div className="pr-2 flex items-center gap-2">
        <Button
          variant={tryItOutOpen ? 'secondary' : 'ghost'}
          size="sm"
          className="h-7 text-xs gap-1"
          onClick={() => setTryItOutOpen(!tryItOutOpen)}
        >
          <PanelRight className="h-3 w-3" />
          Try It Out
        </Button>
      </div>
    </div>
  )

  const errorBanner = loadError && (
    <div
      role="status"
      className="border-b bg-amber-500/5 px-4 py-2 text-xs text-amber-700 dark:text-amber-300"
    >
      Activity could not be refreshed.{' '}
      {events.length > 0 ? 'Showing the last loaded events.' : 'Activity is unavailable.'}
      <span className="ml-1 opacity-80">({loadError})</span>
      <button onClick={reload} className="ml-2 underline">
        Retry
      </button>
    </div>
  )

  const list = isLoading && events.length === 0 ? (
    <p className="py-16 text-center text-sm text-muted-foreground">Loading activity…</p>
  ) : (
    <EventList
      events={visibleEvents}
      grouping={grouping}
      showType={showEventTypeColumn(filter.event_types)}
      selectedId={selectedId}
      onSelect={selectEvent}
      conversationFiltered={conversationFilter != null}
      onToggleConversationFilter={id => {
        const group = relationships.find(group => group.members.has(id))
        if (!group) return
        setConversationFilter(current => current && group.keys.some(key => current.keys.includes(key))
          ? null : { keys: group.keys, title: group.title })
      }}
      filtered={conversationFilter != null || filter.event_types != null || filter.status != null || Boolean(filter.search)}
    />
  )

  const detail = selectedId && (
    <EventDetail
      key={selectedId}
      event={selectedEvent}
      onReplay={() => {
        if (!selectedEvent) return
        const data = selectedEvent.data as unknown as Record<string, unknown>
        const paths: Record<string, ReplayEndpoint> = { '/v1/chat/completions': 'chat_completions', '/v1/completions': 'completions', '/v1/responses': 'responses', '/v1/decisions': 'decisions', '/v1/systemone': 'system_one' }
        const endpoint = paths[String(data.endpoint)]
        if (!endpoint || !data.request_body) { setReplayError('A complete supported request body is unavailable. Import or write a standalone request in Replay / Compare.'); return }
        void invoke<ReplayDataset>('replay_import', { json: JSON.stringify({ name: 'Captured request', cases: [{ id: crypto.randomUUID(), name: 'Captured request', input: { endpoint, body: data.request_body, source_event_id: selectedEvent.id }, checks: [] }] }) } satisfies ReplayImportParams).then(dataset => {
          setReplayError(''); setReplayDraft({ input: dataset.cases[0].input, clientId: selectedEvent.client_id ?? undefined }); setTryItOutOpen(true)
        }).catch(e => setReplayError(String(e)))
      }}
      loading={isDetailLoading}
      error={replayError || detailError}
      onRetry={retryDetail}
      toolbar={
        <DetailDockControls
          dock={dock}
          onDockChange={setDock}
          onDragStart={startDrag}
          onClose={() => selectEvent(null)}
        />
      }
    />
  )

  // The list and the selected event share the monitor's height in a split with
  // a drag handle; the detail can be dragged (or toggled) to dock below or beside.
  const body = (
    <div ref={bodyRef} className="relative flex-1 min-h-0">
      <ResizablePanelGroup
        key={dock}
        direction={dock === 'bottom' ? 'vertical' : 'horizontal'}
        onLayoutChanged={saveLayout}
      >
        <ResizablePanel id="list" minSize="15%">
          <div className="h-full" data-testid="monitor-event-list">{list}</div>
        </ResizablePanel>
        {detail && (
          <>
            <ResizableHandle withHandle orientation={dock === 'bottom' ? 'vertical' : 'horizontal'} />
            <ResizablePanel id="detail" defaultSize={`${detailSize}%`} minSize="20%">
              <div className="h-full" data-testid="monitor-event-detail">{detail}</div>
            </ResizablePanel>
          </>
        )}
      </ResizablePanelGroup>
      {drag && <DockDropPreview target={drag.target} />}
    </div>
  )

  const monitor = (
    <div className="flex h-full flex-col">
      {filterBar}
      {errorBanner}
      {body}
    </div>
  )

  return (
    <section
      aria-label="Request monitor"
      className={cn('flex flex-col overflow-hidden rounded-xl border bg-card', className)}
    >
      {tryItOutOpen ? (
        <ResizablePanelGroup direction="horizontal" className="flex-1 min-h-0">
          <ResizablePanel defaultSize="60%" minSize="30%">
            {monitor}
          </ResizablePanel>
          <ResizableHandle withHandle />
          <ResizablePanel defaultSize="40%" minSize="15%">
            {replayError && <p role="alert" className="text-destructive text-xs p-2">{replayError}</p>}
            <TryItOutPanel key={replayDraft?.input.source_event_id ?? 'manual'} initialReplay={replayDraft?.input} initialClientId={replayDraft?.clientId} onClose={() => { setTryItOutOpen(false); setReplayDraft(null) }} />
          </ResizablePanel>
        </ResizablePanelGroup>
      ) : (
        monitor
      )}
    </section>
  )
})
