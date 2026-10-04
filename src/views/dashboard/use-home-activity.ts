import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import type { GraphData, TimeRange } from '@/types/tauri-commands'

export function useHomeActivity(range: TimeRange) {
  const [revision, setRevision] = useState(0)
  const [metrics, setMetrics] = useState<{
    range: TimeRange
    llm: GraphData | null
    mcp: GraphData | null
    cost: GraphData | null
  } | null>(null)
  const [metricsLoading, setMetricsLoading] = useState(true)

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

  return {
    metrics: metrics?.range === range ? metrics : null,
    metricsLoading,
    refresh: () => {
      setRevision((value) => value + 1)
    },
  }
}
