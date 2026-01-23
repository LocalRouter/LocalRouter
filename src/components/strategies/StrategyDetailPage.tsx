import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import DetailPageLayout from '../layouts/DetailPageLayout'
import MetricsPanel from '../MetricsPanel'
import { useMetricsSubscription } from '../../hooks/useMetricsSubscription'
import StrategyConfigEditor, { Strategy } from './StrategyConfigEditor'
import { RouteLLMConfigEditor, RouteLLMConfig } from '../routellm'

interface Client {
  id: string
  name: string
  strategy_id: string
  enabled: boolean
  created_at: string
}

interface StrategyDetailPageProps {
  strategyId: string
  onBack: () => void
  onUpdate: () => void
}

export default function StrategyDetailPage({
  strategyId,
  onBack,
  onUpdate,
}: StrategyDetailPageProps) {
  const refreshKey = useMetricsSubscription()
  const [strategy, setStrategy] = useState<Strategy | null>(null)
  const [clients, setClients] = useState<Client[]>([])
  const [loading, setLoading] = useState(true)
  const [activeTab, setActiveTab] = useState<string>('metrics')
  const [deleting, setDeleting] = useState(false)

  useEffect(() => {
    loadStrategyData()
  }, [strategyId])

  const loadStrategyData = async () => {
    setLoading(true)
    try {
      const [strategyData, clientsData] = await Promise.all([
        invoke<Strategy>('get_strategy', { strategyId }),
        invoke<Client[]>('get_clients_using_strategy', { strategyId }),
      ])

      setStrategy(strategyData)
      setClients(clientsData)
    } catch (error) {
      console.error('Failed to load strategy data:', error)
    } finally {
      setLoading(false)
    }
  }

  const handleDelete = async () => {
    if (clients.length > 0) {
      alert(`Cannot delete strategy: ${clients.length} client(s) are using it`)
      return
    }

    if (!confirm(`Delete strategy "${strategy?.name}"? This action cannot be undone.`)) {
      return
    }

    setDeleting(true)
    try {
      await invoke('delete_strategy', { strategyId })
      onUpdate()
      onBack()
    } catch (error) {
      console.error('Failed to delete strategy:', error)
      alert(`Failed to delete strategy: ${error}`)
    } finally {
      setDeleting(false)
    }
  }

  const handleStrategyUpdated = async () => {
    await loadStrategyData()
    onUpdate()
  }

  if (loading || !strategy) {
    return (
      <div className="bg-white dark:bg-gray-800 rounded-lg shadow-sm p-6">
        <div className="text-center py-8 text-gray-500 dark:text-gray-400">Loading strategy details...</div>
      </div>
    )
  }

  const tabs = [
    {
      id: 'metrics',
      label: 'Metrics',
      content: (
        <MetricsPanel
          title="Strategy Metrics"
          chartType="llm"
          metricOptions={[
            { id: 'requests', label: 'Requests' },
            { id: 'tokens', label: 'Tokens' },
            { id: 'cost', label: 'Cost' },
            { id: 'latency', label: 'Latency' },
            { id: 'successrate', label: 'Success' },
          ]}
          scope="strategy"
          scopeId={strategyId}
          defaultMetric="requests"
          defaultTimeRange="day"
          refreshTrigger={refreshKey}
        />
      ),
    },
    {
      id: 'configuration',
      label: 'Configuration',
      content: (
        <div>
          <StrategyConfigEditor
            strategyId={strategyId}
            readOnly={false}
            onSave={handleStrategyUpdated}
          />
        </div>
      ),
    },
    {
      id: 'routing',
      label: 'Intelligent Routing',
      content: (
        <Card>
          <RouteLLMConfigEditor
            config={strategy?.auto_config?.routellm_config || {
              enabled: false,
              threshold: 0.3,
              weak_models: [],
            }}
            onChange={async (routellmConfig: RouteLLMConfig) => {
              try {
                const updatedStrategy = {
                  ...strategy,
                  auto_config: {
                    ...strategy.auto_config,
                    enabled: strategy.auto_config?.enabled || false,
                    prioritized_models: strategy.auto_config?.prioritized_models || [],
                    available_models: strategy.auto_config?.available_models || [],
                    routellm_config: routellmConfig,
                  },
                }
                await invoke('update_strategy', {
                  strategyId,
                  strategy: updatedStrategy,
                })
                await handleStrategyUpdated()
              } catch (error) {
                console.error('Failed to update RouteLLM config:', error)
                alert(`Failed to update: ${error}`)
              }
            }}
            availableModels={[]}
          />
        </Card>
      ),
    },
    {
      id: 'clients',
      label: 'Clients',
      count: clients.length,
      content: (
        <Card>
          <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-4">Clients Using This Strategy</h3>
          {clients.length === 0 ? (
            <div className="text-center py-12 text-gray-500 dark:text-gray-400">
              No clients are currently using this strategy.
            </div>
          ) : (
            <div className="space-y-3">
              {clients.map((client) => (
                <div
                  key={client.id}
                  className="p-4 border border-gray-200 dark:border-gray-700 rounded-lg hover:bg-gray-50 dark:hover:bg-gray-800 transition-colors"
                >
                  <div className="flex items-center justify-between">
                    <div>
                      <div className="font-medium text-gray-900 dark:text-gray-100">{client.name}</div>
                      <div className="text-sm text-gray-500 dark:text-gray-400 font-mono mt-1">{client.id}</div>
                      <div className="text-xs text-gray-400 dark:text-gray-500 mt-1">
                        Created: {new Date(client.created_at).toLocaleDateString()}
                      </div>
                    </div>
                    <div>
                      {client.enabled ? (
                        <span className="inline-flex items-center px-2 py-1 rounded text-xs font-medium bg-green-100 dark:bg-green-900/30 text-green-800 dark:text-green-400">
                          Enabled
                        </span>
                      ) : (
                        <span className="inline-flex items-center px-2 py-1 rounded text-xs font-medium bg-gray-100 dark:bg-gray-800 text-gray-800 dark:text-gray-300">
                          Disabled
                        </span>
                      )}
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )}
        </Card>
      ),
    },
  ]

  const badges = []
  if (strategy.auto_config?.enabled) {
    badges.push({
      label: 'Auto Routing Enabled',
      variant: 'success' as const,
    })
  }
  if (strategy.auto_config?.routellm_config?.enabled) {
    badges.push({
      label: 'RouteLLM Active',
      variant: 'info' as const,
    })
  }
  if (strategy.parent) {
    badges.push({
      label: 'Owned',
      variant: 'secondary' as const,
    })
  } else {
    badges.push({
      label: 'Shared',
      variant: 'info' as const,
    })
  }
  if (strategy.rate_limits.length > 0) {
    badges.push({
      label: `${strategy.rate_limits.length} Rate Limit${strategy.rate_limits.length > 1 ? 's' : ''}`,
      variant: 'warning' as const,
    })
  }

  return (
    <DetailPageLayout
      title={strategy.name}
      subtitle={`Strategy ID: ${strategy.id}`}
      badges={badges}
      actions={
        <div className="flex gap-2">
          <Button onClick={onBack} variant="secondary">
            Back
          </Button>
          {!strategy.parent && clients.length === 0 && (
            <Button onClick={handleDelete} variant="danger" disabled={deleting}>
              {deleting ? 'Deleting...' : 'Delete'}
            </Button>
          )}
        </div>
      }
      tabs={tabs}
      activeTab={activeTab}
      onTabChange={setActiveTab}
      loading={loading}
    />
  )
}
