import { useState, useEffect, useMemo } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/Button'
import { PanelRight } from 'lucide-react'
import { ResizablePanelGroup, ResizablePanel, ResizableHandle } from '@/components/ui/resizable'
import { cn } from '@/lib/utils'
import { useMonitorEvents } from '@/views/monitor/hooks/useMonitorEvents'
import { showEventTypeColumn } from '@/views/monitor/monitor-events'
import { EventList } from '@/views/monitor/event-list'
import { EventDetail } from '@/views/monitor/event-detail'
import { EventFilters } from '@/views/monitor/event-filters'
import { TryItOutPanel } from '@/views/monitor/try-it-out-panel'
import type { MonitorEventFilter, InterceptRule } from '@/types/tauri-commands'
import { isRequest } from './activity-data'

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
  try {
    const raw = localStorage.getItem(FILTER_STORAGE_KEY)
    if (!raw) return EMPTY_FILTER
    const saved = JSON.parse(raw) as Partial<MonitorEventFilter>
    return {
      ...EMPTY_FILTER,
      event_types: saved.event_types ?? null,
      status: saved.status ?? null,
      search: saved.search ?? null,
    }
  } catch (error) {
    console.error('Failed to load persisted monitor filter:', error)
    return EMPTY_FILTER
  }
}

interface RequestMonitorProps {
  /** Bumped by the dashboard's refresh control to reload the event snapshot. */
  reloadSignal: number
  className?: string
}

export function RequestMonitor({ reloadSignal, className }: RequestMonitorProps) {
  const [filter, setFilter] = useState<MonitorEventFilter>(loadPersistedFilter)
  const [tryItOutOpen, setTryItOutOpen] = useState(false)
  const [interceptRule, setInterceptRule] = useState<InterceptRule | null>(null)

  // Persist the deliberate filter dimensions whenever they change so the
  // dashboard restores the user's last filter on the next launch.
  useEffect(() => {
    try {
      localStorage.setItem(
        FILTER_STORAGE_KEY,
        JSON.stringify({
          event_types: filter.event_types,
          status: filter.status,
          search: filter.search,
        }),
      )
    } catch (error) {
      console.error('Failed to persist monitor filter:', error)
    }
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

  useEffect(() => {
    if (reloadSignal > 0) reload()
  }, [reloadSignal, reload])

  const pending = events.filter(event => isRequest(event) && event.status === 'pending').length

  const filterBar = (
    <div className="flex items-center border-b">
      <div className="flex-1 min-w-0">
        <EventFilters
          filter={filter}
          onFilterChange={setFilter}
          interceptRule={interceptRule}
          onInterceptRuleChange={setInterceptRule}
        />
      </div>
      <div className="pr-2 flex items-center gap-2">
        {pending > 0 && (
          <span className="flex items-center gap-1.5 whitespace-nowrap rounded-full bg-teal-500/10 px-2 py-0.5 text-[10px] font-medium text-teal-700 dark:text-teal-300">
            <span className="h-1.5 w-1.5 rounded-full bg-teal-500" />
            {pending} in progress
          </span>
        )}
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
      <button onClick={reload} className="ml-2 underline">
        Retry
      </button>
    </div>
  )

  const list = isLoading && events.length === 0 ? (
    <p className="py-16 text-center text-sm text-muted-foreground">Loading activity…</p>
  ) : (
    <EventList
      events={events}
      showType={showEventTypeColumn(filter.event_types)}
      selectedId={selectedId}
      onSelect={selectEvent}
    />
  )

  const eventSplit = selectedId ? (
    <ResizablePanelGroup direction="vertical" className="flex-1 min-h-0">
      <ResizablePanel defaultSize="35%" minSize="20%">
        {list}
      </ResizablePanel>
      <ResizableHandle withHandle orientation="vertical" />
      <ResizablePanel defaultSize="65%" minSize="20%">
        <EventDetail key={selectedId} event={selectedEvent} loading={isDetailLoading} error={detailError} onRetry={retryDetail} />
      </ResizablePanel>
    </ResizablePanelGroup>
  ) : (
    <div className="flex-1 min-h-0">{list}</div>
  )

  const monitor = (
    <div className="flex flex-col h-full">
      {filterBar}
      {errorBanner}
      {eventSplit}
    </div>
  )

  return (
    <section
      aria-label="Request monitor"
      className={cn('overflow-hidden rounded-2xl border bg-card', className)}
    >
      {tryItOutOpen ? (
        <ResizablePanelGroup direction="horizontal" className="h-full">
          <ResizablePanel defaultSize={60} minSize={30}>
            {monitor}
          </ResizablePanel>
          <ResizableHandle withHandle />
          <ResizablePanel defaultSize={40} minSize={15}>
            <TryItOutPanel onClose={() => setTryItOutOpen(false)} />
          </ResizablePanel>
        </ResizablePanelGroup>
      ) : (
        monitor
      )}
    </section>
  )
}
