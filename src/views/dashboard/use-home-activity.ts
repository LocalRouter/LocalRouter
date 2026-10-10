import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import type {
  GetTrafficTimelineParams,
  TimeRange,
  TrafficTimeline,
} from '@/types/tauri-commands'
import { RANGES } from './activity-data'

/** Coalesces bursts of completed requests into one traffic read. */
const REFRESH_DEBOUNCE_MS = 400

export function useHomeActivity(range: TimeRange) {
  const [revision, setRevision] = useState(0)
  const [traffic, setTraffic] = useState<{
    range: TimeRange
    /** `null` when the read failed. */
    timeline: TrafficTimeline | null
  } | null>(null)
  const [trafficLoading, setTrafficLoading] = useState(true)
  const debounceRef = useRef<ReturnType<typeof setTimeout>>()

  useEffect(() => {
    const interval = window.setInterval(() => {
      setRevision((value) => value + 1)
    }, RANGES[range].pollMs)
    return () => window.clearInterval(interval)
  }, [range])

  useEffect(() => () => clearTimeout(debounceRef.current), [])

  useEffect(() => {
    let cancelled = false
    setTrafficLoading(true)
    invoke<TrafficTimeline>('get_traffic_timeline', {
      timeRange: range,
    } satisfies GetTrafficTimelineParams)
      .then(
        (timeline) => timeline,
        (error) => {
          console.error('Failed to load traffic:', error)
          return null
        },
      )
      .then((timeline) => {
        if (cancelled) return
        setTraffic({ range, timeline })
        setTrafficLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [range, revision])

  const refresh = useCallback(() => {
    setRevision((value) => value + 1)
  }, [])

  const refreshSoon = useCallback(() => {
    clearTimeout(debounceRef.current)
    debounceRef.current = setTimeout(refresh, REFRESH_DEBOUNCE_MS)
  }, [refresh])

  const current = traffic?.range === range ? traffic : null
  return {
    /** `undefined` until the first read of this range; `null` if it failed. */
    timeline: current ? current.timeline : undefined,
    trafficLoading,
    refresh,
    /** Debounced refresh for live events. */
    refreshSoon,
  }
}
