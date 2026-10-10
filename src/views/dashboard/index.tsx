import { useState } from 'react'
import type { TimeRange } from '@/types/tauri-commands'
import { RequestMonitor } from './request-monitor'
import { RequestTraffic } from './request-traffic'
import { UsageLimits } from './usage-limits'
import { useHomeActivity } from './use-home-activity'
import { useInFlightRequests } from './use-in-flight'

interface DashboardViewProps {
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function DashboardView({ onTabChange }: DashboardViewProps = {}) {
  const [range, setRange] = useState<TimeRange>('ten_minutes')
  const [monitorReload, setMonitorReload] = useState(0)
  const home = useHomeActivity(range)
  const inFlight = useInFlightRequests(home.refreshSoon)

  // The page fills the window: the traffic strip is fixed and the monitor takes
  // the remaining height, splitting it between the list and the selected event.
  // Below the monitor's minimum height the page scrolls instead of overlapping.
  return (
    <div className="flex h-full flex-col gap-3 overflow-y-auto">
      <h1 className="mx-auto w-full max-w-[1440px] shrink-0 text-xl font-semibold leading-tight tracking-tight">
        Dashboard
      </h1>

      <RequestTraffic
        className="mx-auto w-full max-w-[1440px] shrink-0"
        range={range}
        onRangeChange={setRange}
        metrics={home.metrics}
        loading={home.metricsLoading}
        inFlight={inFlight.length}
        onRefresh={() => {
          home.refresh()
          setMonitorReload((value) => value + 1)
        }}
      />

      <UsageLimits
        className="mx-auto w-full max-w-[1440px] shrink-0"
        onOpenSettings={onTabChange ? () => onTabChange('settings', 'usage') : undefined}
      />

      <RequestMonitor
        className="mx-auto min-h-[440px] w-full max-w-[1440px] flex-1"
        reloadSignal={monitorReload}
      />
    </div>
  )
}
