import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Button from '../ui/Button'
import Card from '../ui/Card'
import { Strategy } from '../strategies/StrategyConfigEditor'
import StrategyDetailPage from '../strategies/StrategyDetailPage'

interface Client {
  id: string
  name: string
  strategy_id: string
  enabled: boolean
}

interface RoutingTabProps {
  activeSubTab: string | null
  onTabChange: (tab: string, subTab: string | null) => void
}

export default function RoutingTab({ activeSubTab, onTabChange }: RoutingTabProps) {
  const [strategies, setStrategies] = useState<Strategy[]>([])
  const [clients, setClients] = useState<Client[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    loadData()
  }, [])

  const loadData = async () => {
    setLoading(true)
    try {
      const [strategiesData, clientsData] = await Promise.all([
        invoke<Strategy[]>('list_strategies'),
        invoke<Client[]>('list_clients'),
      ])
      setStrategies(strategiesData)
      setClients(clientsData)
    } catch (error) {
      console.error('Failed to load routing data:', error)
    } finally {
      setLoading(false)
    }
  }

  const createNewStrategy = async () => {
    const name = prompt('Strategy name:')
    if (!name) return

    try {
      await invoke('create_strategy', { name, parent: null })
      await loadData()
    } catch (error) {
      console.error('Failed to create strategy:', error)
      alert(`Failed to create strategy: ${error}`)
    }
  }

  // If viewing detail page, render StrategyDetailPage
  if (activeSubTab && activeSubTab !== 'list') {
    return (
      <StrategyDetailPage
        strategyId={activeSubTab}
        onBack={() => onTabChange('routing', 'list')}
        onUpdate={loadData}
      />
    )
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="text-gray-500 dark:text-gray-400">Loading strategies...</div>
      </div>
    )
  }

  return (
    <div className="p-8">
      <div className="flex justify-between items-center mb-6">
        <div>
          <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100">Routing Strategies</h2>
          <p className="text-sm text-gray-600 dark:text-gray-400 mt-1">
            Manage reusable routing configurations with auto-routing and rate limits
          </p>
        </div>
        <Button onClick={createNewStrategy}>+ Create Strategy</Button>
      </div>

      <Card>
        {strategies.length === 0 ? (
          <div className="text-center py-12 text-gray-500 dark:text-gray-400">
            No strategies found. Create one to get started.
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full">
              <thead>
                <tr className="border-b border-gray-200 dark:border-gray-700 text-left">
                  <th className="p-3 font-semibold text-gray-900 dark:text-gray-100">Name</th>
                  <th className="p-3 font-semibold text-gray-900 dark:text-gray-100">Clients Using</th>
                  <th className="p-3 font-semibold text-gray-900 dark:text-gray-100">Auto Routing</th>
                  <th className="p-3 font-semibold text-gray-900 dark:text-gray-100">Rate Limits</th>
                  <th className="p-3 font-semibold text-gray-900 dark:text-gray-100">Type</th>
                </tr>
              </thead>
              <tbody>
                {strategies.map((strategy) => {
                  const clientsUsing = clients.filter((c) => c.strategy_id === strategy.id)

                  return (
                    <tr
                      key={strategy.id}
                      className="border-b border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800 cursor-pointer transition-colors"
                      onClick={() => onTabChange('routing', strategy.id)}
                    >
                      <td className="p-3">
                        <div className="font-medium text-gray-900 dark:text-gray-100">{strategy.name}</div>
                        <div className="text-xs text-gray-500 dark:text-gray-400 font-mono">{strategy.id}</div>
                      </td>
                      <td className="p-3">
                        <span className="text-sm">{clientsUsing.length}</span>
                      </td>
                      <td className="p-3">
                        {strategy.auto_config?.enabled ? (
                          <span className="inline-flex items-center px-2 py-1 rounded text-xs font-medium bg-green-100 dark:bg-green-900/30 text-green-800 dark:text-green-400">
                            Enabled
                          </span>
                        ) : (
                          <span className="inline-flex items-center px-2 py-1 rounded text-xs font-medium bg-gray-100 dark:bg-gray-800 text-gray-800 dark:text-gray-300">
                            Disabled
                          </span>
                        )}
                      </td>
                      <td className="p-3">
                        <span className="text-sm text-gray-900 dark:text-gray-100">{strategy.rate_limits.length}</span>
                      </td>
                      <td className="p-3">
                        {strategy.parent ? (
                          <span className="inline-flex items-center px-2 py-1 rounded text-xs font-medium bg-blue-100 dark:bg-blue-900/30 text-blue-800 dark:text-blue-400">
                            Owned
                          </span>
                        ) : (
                          <span className="inline-flex items-center px-2 py-1 rounded text-xs font-medium bg-purple-100 dark:bg-purple-900/30 text-purple-800 dark:text-purple-400">
                            Shared
                          </span>
                        )}
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          </div>
        )}
      </Card>
    </div>
  )
}
