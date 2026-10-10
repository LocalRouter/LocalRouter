import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import type { GraphData, TimeRange } from '@/types/tauri-commands'
import { RANGES } from './activity-data'

/** Coalesces bursts of completed requests into one metrics read. */
const REFRESH_DEBOUNCE_MS = 400

export function useHomeActivity(range: TimeRange) {
  const [revision, setRevision] = useState(0)
  const [metrics, setMetrics] = useState<{
    range: TimeRange
    llm: GraphData | null
    mcp: GraphData | null
    cost: GraphData | null
  } | null>(null)
  const [metricsLoading, setMetricsLoading] = useState(true)
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

  const refresh = useCallback(() => {
    setRevision((value) => value + 1)
  }, [])

  const refreshSoon = useCallback(() => {
    clearTimeout(debounceRef.current)
    debounceRef.current = setTimeout(refresh, REFRESH_DEBOUNCE_MS)
  }, [refresh])

  return {
    metrics: metrics?.range === range ? metrics : null,
    metricsLoading,
    refresh,
    /** Debounced refresh for live events. */
    refreshSoon,
  }
}
