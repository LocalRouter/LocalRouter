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

export function useMonitorEvents(filter?: MonitorEventFilter | null) {
  const [events, setEvents] = useState<MonitorEventSummary[]>([])
  const [selectedEvent, setSelectedEvent] = useState<MonitorEvent | null>(null)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const selectedIdRef = useRef<string | null>(null)

  // Keep ref in sync
  useEffect(() => {
    selectedIdRef.current = selectedId
  }, [selectedId])

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
      // Only add if it matches the current filter (simplified: always add, filter applied on backend)
      setEvents(prev => [summary, ...prev].slice(0, MAX_DISPLAY))
    } catch {
      // Ignore parse errors
    }
  }, [])

  // Listen for event updates (streaming response completion)
  useTauriListener<string>('monitor-event-updated', (event) => {
    try {
      const updated: MonitorEventSummary = JSON.parse(event.payload)
      setEvents(prev =>
        prev.map(e => (e.id === updated.id ? updated : e))
      )
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
