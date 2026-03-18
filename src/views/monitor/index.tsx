import { useState, useMemo } from 'react'
import { Button } from '@/components/ui/Button'
import { PanelRight } from 'lucide-react'
import { ResizablePanelGroup, ResizablePanel, ResizableHandle } from '@/components/ui/resizable'
import { useMonitorEvents } from './hooks/useMonitorEvents'
import { EventList } from './event-list'
import { EventDetail } from './event-detail'
import { EventFilters } from './event-filters'
import { TryItOutPanel } from './try-it-out-panel'
import type { MonitorEventFilter } from '@/types/tauri-commands'

export function MonitorView() {
  const [filter, setFilter] = useState<MonitorEventFilter>({
    event_types: null,
    client_id: null,
    status: null,
    search: null,
  })
  const [tryItOutOpen, setTryItOutOpen] = useState(false)

  // Only pass filter to backend if it has actual values
  const activeFilter = useMemo(() => {
    const hasFilter = filter.event_types || filter.client_id || filter.status || filter.search
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
    <div className="flex items-center">
      <div className="flex-1">
        <EventFilters
          filter={filter}
          onFilterChange={setFilter}
          onClear={clearEvents}
        />
      </div>
      <div className="pr-2 border-b flex items-center">
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

  const eventSplit = (
    <ResizablePanelGroup direction="vertical" className="flex-1">
      <ResizablePanel defaultSize={55} minSize={20}>
        <EventList
          events={events}
          selectedId={selectedId}
          onSelect={selectEvent}
        />
      </ResizablePanel>
      <ResizableHandle withHandle />
      <ResizablePanel defaultSize={45} minSize={15}>
        <EventDetail event={selectedEvent} />
      </ResizablePanel>
    </ResizablePanelGroup>
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
