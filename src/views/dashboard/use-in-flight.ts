import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useTauriListener } from '@/hooks/useTauriListener'
import type {
  GetMonitorEventsParams,
  MonitorEventListResponse,
  MonitorEventSummary,
} from '@/types/tauri-commands'
import { applyInFlight } from './activity-data'

const RESYNC_MS = 30_000

/**
 * Requests (LLM and MCP) that are still running, independent of the monitor's
 * list filter. `onSettled` fires whenever a request finishes, so traffic
 * metrics can be refreshed the moment a count changes.
 */
export function useInFlightRequests(onSettled: () => void) {
  const inFlightRef = useRef(new Map<string, MonitorEventSummary>())
  const [inFlight, setInFlight] = useState<MonitorEventSummary[]>([])
  const onSettledRef = useRef(onSettled)
  onSettledRef.current = onSettled

  const publish = useCallback(() => {
    setInFlight(
      [...inFlightRef.current.values()].sort(
        (a, b) => Date.parse(a.timestamp) - Date.parse(b.timestamp),
      ),
    )
  }, [])

  // Live events seen while a snapshot is loading; replayed over it so a
  // request that finished meanwhile does not come back as running.
  const liveDuringSyncRef = useRef<Map<string, MonitorEventSummary> | null>(null)
  const syncRef = useRef(0)

  const sync = useCallback(() => {
    const request = ++syncRef.current
    const live = new Map<string, MonitorEventSummary>()
    liveDuringSyncRef.current = live
    invoke<MonitorEventListResponse>('get_monitor_events', {
      offset: 0,
      limit: 500,
      filter: {
        event_types: null,
        session_id: null,
        client_id: null,
        status: 'pending',
        search: null,
      },
    } satisfies GetMonitorEventsParams)
      .then((response) => {
        if (request !== syncRef.current) return
        const next = new Map<string, MonitorEventSummary>()
        for (const event of response.events) applyInFlight(next, event)
        for (const event of live.values()) applyInFlight(next, event)
        inFlightRef.current = next
        publish()
      })
      .catch((error) => console.error('Failed to load in-flight requests:', error))
      .finally(() => {
        if (request === syncRef.current) liveDuringSyncRef.current = null
      })
  }, [publish])

  // Re-sync now and then, so a missed completion cannot leave a request
  // marked as running.
  useEffect(() => {
    sync()
    const timer = window.setInterval(sync, RESYNC_MS)
    return () => {
      window.clearInterval(timer)
      ++syncRef.current
    }
  }, [sync])

  const handle = useCallback(
    (payload: string) => {
      try {
        const event: MonitorEventSummary = JSON.parse(payload)
        liveDuringSyncRef.current?.set(event.id, event)
        const settled = applyInFlight(inFlightRef.current, event)
        publish()
        if (settled) onSettledRef.current()
      } catch {
        // Ignore malformed payloads.
      }
    },
    [publish],
  )

  useTauriListener<string>('monitor-event-created', (event) => handle(event.payload), [handle])
  useTauriListener<string>('monitor-event-updated', (event) => handle(event.payload), [handle])

  return inFlight
}
