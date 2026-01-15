import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import StatCard from '../ui/StatCard'

interface AggregateStats {
  total_requests: number
  total_tokens: number
  total_cost: number
}

export default function HomeTab() {
  const [stats, setStats] = useState({
    requests: 0,
    tokens: 0,
    cost: 0,
    activeKeys: 0,
  })
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    loadStats()
    // Refresh stats every 5 seconds
    const interval = setInterval(loadStats, 5000)
    return () => clearInterval(interval)
  }, [])

  const loadStats = async () => {
    try {
      const keys = await invoke<any[]>('list_api_keys')

      // Try to get aggregate stats, but if server is not running, use zeros
      let aggregateStats: AggregateStats
      try {
        aggregateStats = await invoke<AggregateStats>('get_aggregate_stats')
      } catch (error) {
        // Server likely not running, use default values
        aggregateStats = {
          total_requests: 0,
          total_tokens: 0,
          total_cost: 0,
        }
      }

      setStats({
        requests: aggregateStats.total_requests,
        tokens: aggregateStats.total_tokens,
        cost: aggregateStats.total_cost,
        activeKeys: keys.filter((k: any) => k.enabled).length,
      })
    } catch (error) {
      console.error('Failed to load stats:', error)
    } finally {
      setLoading(false)
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-gray-500">Loading statistics...</div>
      </div>
    )
  }

  return (
    <div>
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4 mb-6">
        <StatCard title="Total Requests" value={stats.requests} />
        <StatCard title="Total Tokens" value={stats.tokens.toLocaleString()} />
        <StatCard title="Total Cost" value={`$${stats.cost.toFixed(4)}`} />
        <StatCard title="Active Keys" value={stats.activeKeys} />
      </div>

      <div className="bg-blue-50 border border-blue-200 rounded-lg p-4 mb-6">
        <p className="text-sm text-blue-900">
          <strong>Note:</strong> Statistics are tracked in-memory for the last 7 days. The counter resets when the application restarts.
        </p>
      </div>

      <Card className="mb-6">
        <h2 className="text-xl font-bold mb-4 text-gray-900">Request Volume</h2>
        <div className="bg-gray-50 border-2 border-dashed border-gray-300 rounded-lg h-[300px] flex items-center justify-center text-gray-500 font-medium">
          Chart coming soon - Requests over time
        </div>
      </Card>

      <Card>
        <h2 className="text-xl font-bold mb-4 text-gray-900">Cost Analysis</h2>
        <div className="bg-gray-50 border-2 border-dashed border-gray-300 rounded-lg h-[300px] flex items-center justify-center text-gray-500 font-medium">
          Chart coming soon - Cost breakdown by provider
        </div>
      </Card>
    </div>
  )
}
