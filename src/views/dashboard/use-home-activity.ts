import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listenSafe } from '@/hooks/useTauriListener'
import { mergeMonitorEvents } from '@/views/monitor/monitor-events'
import type {
  GraphData,
  MonitorEventListResponse,
  MonitorEventSummary,
  TimeRange,
} from '@/types/tauri-commands'

const EVENT_LIMIT = 100

export function useHomeActivity(range: TimeRange) {
  const [revision, setRevision] = useState(0)
  const [metrics, setMetrics] = useState<{
    range: TimeRange
    llm: GraphData | null
    mcp: GraphData | null
    cost: GraphData | null
  } | null>(null)
  const [metricsLoading, setMetricsLoading] = useState(true)
  const [events, setEvents] = useState<MonitorEventSummary[]>([])
  const [eventsLoading, setEventsLoading] = useState(true)
  const [eventsError, setEventsError] = useState(false)
  const refreshEvents = useRef<() => void>(() => {})

  useEffect(() => {
    const interval = window.setInterval(() => {
      setRevision((value) => value + 1)
    }, 15_000)
    return () => window.clearInterval(interval)
  }, [])

  useEffect(() => {
    let cancelled = false
    setMetricsLoading(true)
    Promise.allSettled([
      invoke<GraphData>('get_global_metrics', {
        timeRange: range,
        metricType: 'requests',
      }),
      invoke<GraphData>('get_global_mcp_metrics', {
        timeRange: range,
        metricType: 'requests',
      }),
      invoke<GraphData>('get_global_metrics', {
        timeRange: range,
        metricType: 'cost',
      }),
    ]).then(([llm, mcp, cost]) => {
      if (cancelled) return
      setMetrics({
        range,
        llm: llm.status === 'fulfilled' ? llm.value : null,
        mcp: mcp.status === 'fulfilled' ? mcp.value : null,
        cost: cost.status === 'fulfilled' ? cost.value : null,
      })
      setMetricsLoading(false)
    })
    return () => {
      cancelled = true
    }
  }, [range, revision])

  useEffect(() => {
    let cancelled = false
    let loading = false
    let updates = new Map<string, MonitorEventSummary>()
    const load = async () => {
      if (loading || cancelled) return
      loading = true
      updates = new Map()
      try {
        const result = await invoke<MonitorEventListResponse>(
          'get_monitor_events',
          { offset: 0, limit: EVENT_LIMIT, filter: null },
        )
        if (!cancelled) {
          setEvents(
            mergeMonitorEvents(
              result.events,
              updates.values(),
              null,
              EVENT_LIMIT,
            ),
          )
          setEventsError(false)
        }
      } catch {
        if (!cancelled) setEventsError(true)
      } finally {
        loading = false
        if (!cancelled) setEventsLoading(false)
      }
    }
    const receive = ({ payload }: { payload: string }) => {
      if (cancelled) return
      try {
        const event = JSON.parse(payload) as MonitorEventSummary
        if (loading) updates.set(event.id, event)
        setEvents((previous) =>
          mergeMonitorEvents(previous, [event], null, EVENT_LIMIT),
        )
      } catch {
        /* Ignore malformed event payloads. */
      }
    }
    const listeners = [
      listenSafe<string>('monitor-event-created', receive),
      listenSafe<string>('monitor-event-updated', receive),
    ]
    // Subscribe before reading a snapshot so requests arriving during load are retained.
    Promise.all(listeners.map((listener) => listener.promise)).then(() => {
      if (!cancelled) {
        refreshEvents.current = load
        void load()
      }
    })
    return () => {
      cancelled = true
      refreshEvents.current = () => {}
      listeners.forEach((listener) => listener.cleanup())
    }
  }, [])

  useEffect(() => {
    refreshEvents.current()
  }, [revision])

  return {
    metrics: metrics?.range === range ? metrics : null,
    metricsLoading,
    events,
    eventsLoading,
    eventsError,
    refresh: () => {
      setRevision((value) => value + 1)
    },
  }
}
