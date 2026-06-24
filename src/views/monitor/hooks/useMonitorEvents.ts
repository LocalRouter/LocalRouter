import { useState, useEffect, useCallback, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useTauriListener } from '@/hooks/useTauriListener'
import type {
  MonitorEventSummary,
  MonitorEvent,
  MonitorEventFilter,
  MonitorEventListResponse,
} from '@/types/tauri-commands'

const MAX_DISPLAY = 500

/**
 * Client-side mirror of the backend `match_filter` (crates/lr-monitor/src/store.rs).
 * Live events arrive via Tauri events that bypass the backend query, so we must
 * re-apply the same predicate here or new events would ignore the active filter.
 * Kept in sync with the backend semantics (empty `event_types` = no type filter).
 */
function matchesFilter(summary: MonitorEventSummary, filter: MonitorEventFilter | null | undefined): boolean {
  if (!filter) return true

  if (filter.event_types && filter.event_types.length > 0) {
    if (!filter.event_types.includes(summary.event_type)) return false
  }
  if (filter.client_id && summary.client_id !== filter.client_id) return false
  if (filter.status && summary.status !== filter.status) return false
  if (filter.session_id && summary.session_id !== filter.session_id) return false
  if (filter.search) {
    if (!summary.summary.toLowerCase().includes(filter.search.toLowerCase())) return false
  }
  return true
}

export function useMonitorEvents(filter?: MonitorEventFilter | null) {
  const [events, setEvents] = useState<MonitorEventSummary[]>([])
  const [selectedEvent, setSelectedEvent] = useState<MonitorEvent | null>(null)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const selectedIdRef = useRef<string | null>(null)
  // Latest filter, read by the live-event listeners (which subscribe once and
  // must not go stale when the filter changes).
  const filterRef = useRef<MonitorEventFilter | null | undefined>(filter)

  // Keep refs in sync
  useEffect(() => {
    selectedIdRef.current = selectedId
  }, [selectedId])

  useEffect(() => {
    filterRef.current = filter
  }, [filter])

  // Initial load
  useEffect(() => {
    setIsLoading(true)
    invoke<MonitorEventListResponse>('get_monitor_events', {
      offset: 0,
      limit: MAX_DISPLAY,
      filter: filter ?? null,
    })
      .then(res => {
        setEvents(res.events)
        setIsLoading(false)
      })
      .catch(() => setIsLoading(false))
  }, [filter])

  // Listen for new events
  useTauriListener<string>('monitor-event-created', (event) => {
    try {
      const summary: MonitorEventSummary = JSON.parse(event.payload)
      // Live events bypass the backend query, so apply the active filter here
      // too — otherwise new events would show regardless of the filter.
      if (!matchesFilter(summary, filterRef.current)) return
      setEvents(prev => [summary, ...prev].slice(0, MAX_DISPLAY))
    } catch {
      // Ignore parse errors
    }
  }, [])

  // Listen for event updates (streaming response completion)
  useTauriListener<string>('monitor-event-updated', (event) => {
    try {
      const updated: MonitorEventSummary = JSON.parse(event.payload)
      // An update can flip whether an event matches (e.g. status pending →
      // complete under a status filter). Drop it from the list if it no longer
      // matches; otherwise replace the existing entry in place.
      if (!matchesFilter(updated, filterRef.current)) {
        setEvents(prev => prev.filter(e => e.id !== updated.id))
      } else {
        setEvents(prev =>
          prev.map(e => (e.id === updated.id ? updated : e))
        )
      }
      // If this is the currently selected event, refresh detail
      if (selectedIdRef.current === updated.id) {
        invoke<MonitorEvent | null>('get_monitor_event_detail', {
          eventId: updated.id,
        }).then(detail => {
          if (detail) setSelectedEvent(detail)
        }).catch(() => {})
      }
    } catch {
      // Ignore parse errors
    }
  }, [])

  const selectEvent = useCallback((id: string | null) => {
    setSelectedId(id)
    if (!id) {
      setSelectedEvent(null)
      return
    }
    invoke<MonitorEvent | null>('get_monitor_event_detail', { eventId: id })
      .then(detail => {
        if (detail) setSelectedEvent(detail)
      })
      .catch(() => {})
  }, [])

  const clearEvents = useCallback(() => {
    invoke('clear_monitor_events').then(() => {
      setEvents([])
      setSelectedEvent(null)
      setSelectedId(null)
    }).catch(() => {})
  }, [])

  return {
    events,
    selectedEvent,
    selectedId,
    isLoading,
    selectEvent,
    clearEvents,
  }
}
