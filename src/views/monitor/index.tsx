import { useState, useEffect, useMemo } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/Button'
import { PanelRight } from 'lucide-react'
import { ResizablePanelGroup, ResizablePanel, ResizableHandle } from '@/components/ui/resizable'
import { useMonitorEvents } from './hooks/useMonitorEvents'
import { EventList } from './event-list'
import { EventDetail } from './event-detail'
import { EventFilters } from './event-filters'
import { TryItOutPanel } from './try-it-out-panel'
import type { MonitorEventFilter, InterceptRule } from '@/types/tauri-commands'

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

export function MonitorView() {
  const [filter, setFilter] = useState<MonitorEventFilter>(loadPersistedFilter)
  const [tryItOutOpen, setTryItOutOpen] = useState(false)
  const [interceptRule, setInterceptRule] = useState<InterceptRule | null>(null)

  // Persist the deliberate filter dimensions whenever they change so the tab
  // restores the user's last filter on the next launch.
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

  // Clear intercept rule on unmount (navigating away from monitor page)
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
    selectEvent,
    clearEvents,
  } = useMonitorEvents(activeFilter)

  const filterBar = (
    <div className="flex items-center border-b">
      <div className="flex-1">
        <EventFilters
          filter={filter}
          onFilterChange={setFilter}
          onClear={clearEvents}
          interceptRule={interceptRule}
          onInterceptRuleChange={setInterceptRule}
        />
      </div>
      <div className="pr-2 flex items-center">
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

  const eventSplit = selectedEvent ? (
    <ResizablePanelGroup direction="vertical" className="flex-1">
      <ResizablePanel defaultSize={55} minSize={20}>
        <EventList
          events={events}
          selectedId={selectedId}
          onSelect={selectEvent}
        />
      </ResizablePanel>
      <ResizableHandle withHandle orientation="vertical" />
      <ResizablePanel defaultSize={45} minSize={15}>
        <EventDetail event={selectedEvent} />
      </ResizablePanel>
    </ResizablePanelGroup>
  ) : (
    <div className="flex-1">
      <EventList
        events={events}
        selectedId={selectedId}
        onSelect={selectEvent}
      />
    </div>
  )

  if (!tryItOutOpen) {
    return (
      <div className="flex flex-col h-full">
        {filterBar}
        {eventSplit}
      </div>
    )
  }

  return (
    <ResizablePanelGroup direction="horizontal" className="h-full">
      <ResizablePanel defaultSize={60} minSize={30}>
        <div className="flex flex-col h-full">
          {filterBar}
          {eventSplit}
        </div>
      </ResizablePanel>
      <ResizableHandle withHandle />
      <ResizablePanel defaultSize={40} minSize={15}>
        <TryItOutPanel onClose={() => setTryItOutOpen(false)} />
      </ResizablePanel>
    </ResizablePanelGroup>
  )
}
