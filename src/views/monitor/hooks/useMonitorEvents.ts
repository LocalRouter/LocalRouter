import { useState, useEffect, useCallback, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useTauriListener } from '@/hooks/useTauriListener'
import { mergeMonitorEvents } from '../monitor-events'
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
  const [isDetailLoading, setIsDetailLoading] = useState(false)
  const [detailError, setDetailError] = useState<string | null>(null)
  const selectedIdRef = useRef<string | null>(null)
  const listRequestRef = useRef(0)
  const detailRequestRef = useRef(0)
  const pendingUpdatesRef = useRef<Map<string, MonitorEventSummary> | null>(null)
  // Latest filter, read by the live-event listeners (which subscribe once and
  // must not go stale when the filter changes).
  const filterRef = useRef<MonitorEventFilter | null | undefined>(filter)

  filterRef.current = filter

  // Initial load
  useEffect(() => {
    const request = ++listRequestRef.current
    pendingUpdatesRef.current = new Map()
    setIsLoading(true)
    invoke<MonitorEventListResponse>('get_monitor_events', {
      offset: 0,
      limit: MAX_DISPLAY,
      filter: filter ?? null,
    })
      .then(res => {
        if (request !== listRequestRef.current) return
        setEvents(mergeMonitorEvents(res.events, pendingUpdatesRef.current?.values() ?? [], filter, MAX_DISPLAY))
      })
      .catch(() => {})
      .finally(() => {
        if (request !== listRequestRef.current) return
        pendingUpdatesRef.current = null
        setIsLoading(false)
      })
    return () => { ++listRequestRef.current }
  }, [filter])

  useEffect(() => () => { ++detailRequestRef.current }, [])

  const loadDetail = useCallback((id: string) => {
    const request = ++detailRequestRef.current
    setIsDetailLoading(true)
    setDetailError(null)
    invoke<MonitorEvent | null>('get_monitor_event_detail', { eventId: id })
      .then(detail => {
        if (request === detailRequestRef.current && selectedIdRef.current === id) {
          setSelectedEvent(detail)
          if (!detail) setDetailError('This event is no longer available.')
        }
      })
      .catch(() => {
        if (request === detailRequestRef.current && selectedIdRef.current === id) {
          setDetailError('Unable to load event details.')
        }
      })
      .finally(() => {
        if (request === detailRequestRef.current && selectedIdRef.current === id) setIsDetailLoading(false)
      })
  }, [])

  // Listen for new events
  useTauriListener<string>('monitor-event-created', (event) => {
    try {
      const summary: MonitorEventSummary = JSON.parse(event.payload)
      pendingUpdatesRef.current?.set(summary.id, summary)
      setEvents(prev => mergeMonitorEvents(prev, [summary], filterRef.current, MAX_DISPLAY))
    } catch {
      // Ignore parse errors
    }
  }, [])

  // Listen for event updates (streaming response completion)
  useTauriListener<string>('monitor-event-updated', (event) => {
    try {
      const updated: MonitorEventSummary = JSON.parse(event.payload)
      pendingUpdatesRef.current?.set(updated.id, updated)
      setEvents(prev => mergeMonitorEvents(prev, [updated], filterRef.current, MAX_DISPLAY))
      // If this is the currently selected event, refresh detail
      if (selectedIdRef.current === updated.id) {
        loadDetail(updated.id)
      }
    } catch {
      // Ignore parse errors
    }
  }, [])

  const selectEvent = useCallback((id: string | null) => {
    if (selectedIdRef.current === id) return
    selectedIdRef.current = id
    ++detailRequestRef.current
    setSelectedId(id)
    setSelectedEvent(null)
    setDetailError(null)
    if (!id) {
      setIsDetailLoading(false)
      return
    }
    loadDetail(id)
  }, [loadDetail])

  const clearEvents = useCallback(() => {
    invoke('clear_monitor_events').then(() => {
      ++listRequestRef.current
      ++detailRequestRef.current
      pendingUpdatesRef.current = null
      selectedIdRef.current = null
      setIsLoading(false)
      setEvents([])
      setSelectedEvent(null)
      setSelectedId(null)
      setIsDetailLoading(false)
      setDetailError(null)
    }).catch(() => {})
  }, [])

  const retryDetail = useCallback(() => {
    if (selectedIdRef.current) loadDetail(selectedIdRef.current)
  }, [loadDetail])

  return {
    events,
    isDetailLoading,
    detailError,
    retryDetail,
    selectedEvent,
    selectedId,
    isLoading,
    selectEvent,
    clearEvents,
  }
}
