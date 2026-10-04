import { useState } from 'react'
import type { TimeRange } from '@/types/tauri-commands'
import { RequestMonitor } from './request-monitor'
import { RequestTraffic } from './request-traffic'
import { useHomeActivity } from './use-home-activity'

export function DashboardView() {
  const [range, setRange] = useState<TimeRange>('ten_minutes')
  const [monitorReload, setMonitorReload] = useState(0)
  const home = useHomeActivity(range)

  // The page is its own scroll container so the monitor can be exactly one
  // viewport tall: scrolling past the traffic chart shows it at full height.
  return (
    <div className="flex h-full flex-col gap-5 overflow-y-auto">
      <h1 className="mx-auto w-full max-w-[1440px] shrink-0 text-[28px] font-semibold leading-tight tracking-tight">
        Dashboard
      </h1>

      <RequestTraffic
        className="mx-auto w-full max-w-[1440px] shrink-0"
        range={range}
        onRangeChange={setRange}
        metrics={home.metrics}
        loading={home.metricsLoading}
        onRefresh={() => {
          home.refresh()
          setMonitorReload((value) => value + 1)
        }}
      />

      <RequestMonitor
        className="mx-auto h-full min-h-[480px] w-full max-w-[1440px] shrink-0"
        reloadSignal={monitorReload}
      />
    </div>
  )
}
