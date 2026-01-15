import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import StatCard from '../ui/StatCard'

export default function HomeTab() {
  const [stats, setStats] = useState({
    requests: 0,
    tokens: 0,
    cost: 0,
    activeKeys: 0,
  })

  useEffect(() => {
    loadStats()
  }, [])

  const loadStats = async () => {
    try {
      const keys = await invoke<any[]>('list_api_keys')
      setStats({
        ...stats,
        activeKeys: keys.filter((k: any) => k.enabled).length,
      })
    } catch (error) {
      console.error('Failed to load stats:', error)
    }
  }

  return (
    <div>
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4 mb-6">
        <StatCard title="Total Requests" value={stats.requests} />
        <StatCard title="Total Tokens" value={stats.tokens} />
        <StatCard title="Total Cost" value={`$${stats.cost.toFixed(2)}`} />
        <StatCard title="Active Keys" value={stats.activeKeys} />
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
