import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { ArrowRight, Clock3 } from 'lucide-react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { EventDetail } from '@/views/monitor/event-detail'
import { EventList } from '@/views/monitor/event-list'
import { cn } from '@/lib/utils'
import type {
  MonitorEvent,
  MonitorEventSummary,
  TimeRange,
} from '@/types/tauri-commands'
import { isRequest } from './activity-data'
import { RequestTraffic } from './request-traffic'
import { useHomeActivity } from './use-home-activity'

interface DashboardViewProps {
  onViewChange?: (view: string, subTab?: string | null) => void
}

export function DashboardView({ onViewChange }: DashboardViewProps) {
  const [range, setRange] = useState<TimeRange>('day')
  const [selected, setSelected] = useState<MonitorEventSummary | null>(null)
  const [detail, setDetail] = useState<MonitorEvent | null>(null)
  const [detailState, setDetailState] = useState<
    'loading' | 'ready' | 'missing' | 'error'
  >('loading')
  const home = useHomeActivity(range)
  const pending = home.events.filter(
    (event) => isRequest(event) && event.status === 'pending',
  ).length
  const snapshot = home.events.slice(0, 8)
  const snapshotProps = {
    events: snapshot,
    showType: true,
    selectedId: selected?.id ?? null,
    onSelect: (id: string) =>
      setSelected(home.events.find((event) => event.id === id) ?? null),
  }
  const selectedLive =
    home.events.find((event) => event.id === selected?.id) ?? selected

  useEffect(() => {
    if (!selectedLive) return
    let cancelled = false
    setDetail(null)
    setDetailState('loading')
    invoke<MonitorEvent | null>('get_monitor_event_detail', {
      eventId: selectedLive.id,
    })
      .then((value) => {
        if (!cancelled) {
          setDetail(value)
          setDetailState(value ? 'ready' : 'missing')
        }
      })
      .catch(() => {
        if (!cancelled) setDetailState('error')
      })
    return () => {
      cancelled = true
    }
  }, [selectedLive?.id, selectedLive?.status, selectedLive?.duration_ms])

  return (
    <div className="mx-auto max-w-[1440px] space-y-5 p-1 pb-5 sm:p-3">
      <div className="flex flex-wrap items-center justify-between gap-4 pb-1">
        <h1 className="text-[28px] font-semibold leading-tight tracking-tight">
          Dashboard
        </h1>
      </div>

      <div className="min-w-0 space-y-5">
        <RequestTraffic
          range={range}
          onRangeChange={setRange}
          metrics={home.metrics}
          loading={home.metricsLoading}
          onRefresh={home.refresh}
        />

        <section
          className="overflow-hidden rounded-2xl border bg-card"
          aria-label="Monitor snapshot"
        >
          <div className="flex flex-wrap items-center justify-between gap-3 px-5 pb-4 pt-5">
            <div className="flex items-center gap-2.5">
              <h2 className="text-sm font-semibold">Recent activity</h2>
              {pending > 0 && (
                <span className="flex items-center gap-1.5 rounded-full bg-teal-500/10 px-2 py-0.5 text-[10px] font-medium text-teal-700 dark:text-teal-300">
                  <span className="h-1.5 w-1.5 rounded-full bg-teal-500" />
                  {pending} in progress
                </span>
              )}
            </div>
            <span className="text-xs text-muted-foreground">
              Monitor snapshot
            </span>
          </div>
          {home.eventsError && (
            <div
              role="status"
              className="border-t bg-amber-500/5 px-5 py-3 text-xs text-amber-700 dark:text-amber-300"
            >
              Activity could not be refreshed.{' '}
              {home.events.length > 0
                ? 'Showing the last loaded events.'
                : 'Try refreshing the page data.'}
              <button onClick={home.refresh} className="ml-2 underline">
                Retry
              </button>
            </div>
          )}
          <div className="border-t">
            {home.eventsLoading ? (
              <p className="py-16 text-center text-sm text-muted-foreground">
                Loading recent activity…
              </p>
            ) : !snapshot.length && home.eventsError ? (
              <p className="py-16 text-center text-sm text-muted-foreground">
                Activity is unavailable
              </p>
            ) : (
              <div className="h-[260px] overflow-x-auto">
                <div
                  className={cn(
                    'h-full',
                    snapshot.length > 0 && 'min-w-[640px]',
                  )}
                >
                  <EventList {...snapshotProps} />
                </div>
              </div>
            )}
          </div>
          <div className="flex items-center justify-between border-t bg-muted/20 px-5 py-3 text-[10px] text-muted-foreground">
            <span>Latest {snapshot.length} events · updates live</span>
            <button
              className="flex items-center gap-1 text-xs hover:text-foreground"
              onClick={() => onViewChange?.('monitor')}
            >
              View all
              <ArrowRight className="h-3 w-3" />
            </button>
          </div>
        </section>
      </div>
      <div className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
        <Clock3 className="h-3 w-3" />
        Traffic refreshes every 15 seconds. Activity stays on this device.
      </div>

      <Dialog
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
      >
        <DialogContent className="flex max-h-[85vh] w-[calc(100%-2rem)] max-w-4xl flex-col overflow-hidden p-0">
          <DialogHeader className="shrink-0 border-b px-6 pb-4 pt-6">
            <DialogTitle>Activity detail</DialogTitle>
            <DialogDescription className="break-words pr-4">
              {selected?.summary}
            </DialogDescription>
          </DialogHeader>
          <div className="min-h-0 overflow-auto p-4">
            {detailState === 'ready' && detail && detail.id === selected?.id ? (
              <EventDetail key={detail.id} event={detail} />
            ) : (
              <p className="py-12 text-center text-sm text-muted-foreground">
                {detailState === 'loading'
                  ? 'Loading event details…'
                  : detailState === 'missing'
                    ? 'This event is no longer available in the local history.'
                    : 'Could not load this event. Close and reopen to retry.'}
              </p>
            )}
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}
