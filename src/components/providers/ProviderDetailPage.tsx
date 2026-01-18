import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Badge from '../ui/Badge'
import Button from '../ui/Button'
import ProviderIcon from '../ProviderIcon'
import { ContextualChat } from '../chat/ContextualChat'
import DetailPageLayout from '../layouts/DetailPageLayout'
import { MetricsChart } from '../charts/MetricsChart'
import { useMetricsSubscription } from '../../hooks/useMetricsSubscription'

interface ProviderDetailPageProps {
  instanceName: string
  providerType: string
  onTabChange?: (tab: 'providers' | 'api-keys' | 'models', subTab: string) => void
}

interface ProviderHealth {
  status: 'Healthy' | 'Degraded' | 'Unhealthy'
  latency_ms: number | null
  last_checked: string
  error_message: string | null
}

interface Model {
  model_id: string
  provider_instance: string
  context_window: number
  capabilities: string[]
  supports_streaming: boolean
}

interface ApiKey {
  id: string
  name: string
  enabled: boolean
  model_selection: any
}

export default function ProviderDetailPage({
  instanceName,
  providerType,
  onTabChange,
}: ProviderDetailPageProps) {
  const refreshKey = useMetricsSubscription()
  const [health, setHealth] = useState<ProviderHealth | null>(null)
  const [models, setModels] = useState<Model[]>([])
  const [apiKeys, setApiKeys] = useState<ApiKey[]>([])
  const [config, setConfig] = useState<Record<string, string>>({})
  const [enabled, setEnabled] = useState(true)
  const [loading, setLoading] = useState(true)
  const [isSaving, setIsSaving] = useState(false)
  const [activeTab, setActiveTab] = useState<string>('metrics')

  useEffect(() => {
    loadProviderData()
  }, [instanceName])

  const loadProviderData = async () => {
    setLoading(true)
    try {
      const [healthData, basicModels, configData, instances, keys] = await Promise.all([
        invoke<Record<string, ProviderHealth>>('get_providers_health'),
        invoke<Array<{ id: string; provider: string }>>('list_all_models').catch(() => []),
        invoke<Record<string, string>>('get_provider_config', { instanceName }),
        invoke<Array<{ instance_name: string; enabled: boolean }>>('list_provider_instances'),
        invoke<ApiKey[]>('list_api_keys').catch(() => []),
      ])

      setHealth(healthData[instanceName] || null)

      // Convert basic model list to detailed format and filter by provider
      const providerModels = basicModels
        .filter((m) => m.provider === instanceName)
        .map((m) => ({
          model_id: m.id,
          provider_instance: m.provider,
          context_window: 0,
          capabilities: [],
          supports_streaming: true,
        }))

      setModels(providerModels)
      setConfig(configData)

      const instance = instances.find((i) => i.instance_name === instanceName)
      setEnabled(instance?.enabled ?? true)

      // Filter API keys that use this provider
      const filteredKeys = keys.filter((key) => {
        if (!key.model_selection) return false
        if (key.model_selection.type === 'all') return true
        if (key.model_selection.type === 'custom') {
          const providers = key.model_selection.all_provider_models || []
          const individualModels = key.model_selection.individual_models || []
          // Check if this provider is in the list
          if (providers.includes(instanceName)) return true
          // Check if any individual models from this provider are selected
          return individualModels.some(([provider]: [string, string]) => provider === instanceName)
        }
        return false
      })
      setApiKeys(filteredKeys)
    } catch (error) {
      console.error('Failed to load provider data:', error)
      alert(`Error loading provider data: ${error}`)
    } finally {
      setLoading(false)
    }
  }

  const handleRefreshModels = async () => {
    try {
      // Try to refresh models from the provider (if supported)
      try {
        await invoke('refresh_provider_models', { instanceName })
      } catch (refreshError) {
        console.warn('refresh_provider_models not available, just reloading data')
      }

      await loadProviderData()
      alert('Models refreshed successfully!')
    } catch (error) {
      console.error('Failed to refresh models:', error)
      alert(`Error refreshing models: ${error}`)
    }
  }

  const handleToggleEnabled = async () => {
    try {
      await invoke('set_provider_enabled', { instanceName, enabled: !enabled })
      setEnabled(!enabled)
    } catch (error) {
      console.error('Failed to toggle provider:', error)
      alert(`Error toggling provider: ${error}`)
    }
  }

  const handleSaveConfig = async () => {
    setIsSaving(true)
    try {
      await invoke('update_provider_instance', {
        instanceName,
        providerType,
        config,
      })
      alert('Configuration saved successfully!')
    } catch (error) {
      console.error('Failed to save config:', error)
      alert(`Error saving config: ${error}`)
    } finally {
      setIsSaving(false)
    }
  }

  const handleConfigChange = (key: string, value: string) => {
    setConfig((prev) => ({ ...prev, [key]: value }))
  }

  const formatContextWindow = (tokens: number) => {
    if (tokens >= 1000000) {
      return `${(tokens / 1000000).toFixed(1)}M tokens`
    } else if (tokens >= 1000) {
      return `${(tokens / 1000).toFixed(0)}K tokens`
    }
    return `${tokens} tokens`
  }

  const healthVariant =
    health?.status === 'Healthy' ? 'success' :
    health?.status === 'Degraded' ? 'warning' : 'error'

  const tabs = [
    {
      id: 'metrics',
      label: 'Metrics',
      content: (
        <div className="space-y-6">
          <div className="grid grid-cols-2 gap-6">
            <MetricsChart
              scope="provider"
              scopeId={instanceName}
              timeRange="day"
              metricType="requests"
              title="Requests"
              refreshTrigger={refreshKey}
            />

            <MetricsChart
              scope="provider"
              scopeId={instanceName}
              timeRange="day"
              metricType="tokens"
              title="Tokens"
              refreshTrigger={refreshKey}
            />

            <MetricsChart
              scope="provider"
              scopeId={instanceName}
              timeRange="day"
              metricType="cost"
              title="Cost"
              refreshTrigger={refreshKey}
            />

            <MetricsChart
              scope="provider"
              scopeId={instanceName}
              timeRange="day"
              metricType="latency"
              title="Latency"
              refreshTrigger={refreshKey}
            />
          </div>
        </div>
      ),
    },
    {
      id: 'configuration',
      label: 'Configuration',
      content: (
        <Card>
          <h3 className="text-lg font-semibold text-gray-900 mb-4">Configuration</h3>
          {health?.error_message && (
            <div className="mb-4 p-3 bg-red-50 border border-red-200 rounded-lg text-sm text-red-700">
              <strong>Error:</strong> {health.error_message}
            </div>
          )}
          <div className="space-y-4">
            {Object.entries(config).map(([key, value]) => (
              <div key={key}>
                <label className="block text-sm font-medium text-gray-700 mb-1">
                  {key}
                </label>
                <input
                  type={key.toLowerCase().includes('key') || key.toLowerCase().includes('secret') ? 'password' : 'text'}
                  value={value}
                  onChange={(e) => handleConfigChange(key, e.target.value)}
                  className="w-full px-3 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
                />
              </div>
            ))}
          </div>
          <div className="mt-6 flex justify-end">
            <Button onClick={handleSaveConfig} disabled={isSaving}>
              {isSaving ? 'Saving...' : 'Save Changes'}
            </Button>
          </div>
        </Card>
      ),
    },
    {
      id: 'models',
      label: 'Models',
      count: models.length,
      content: (
        <Card>
          <div className="flex justify-between items-center mb-4">
            <h3 className="text-lg font-semibold text-gray-900">Available Models</h3>
            <Button variant="secondary" onClick={handleRefreshModels}>
              Refresh Models
            </Button>
          </div>
          {models.length === 0 ? (
            <div className="text-center py-8 text-gray-500">
              No models found for this provider.
            </div>
          ) : (
            <div className="space-y-2">
              {models.map((model) => (
                <div
                  key={model.model_id}
                  onClick={() => onTabChange?.('models', `${model.provider_instance}/${model.model_id}`)}
                  className="bg-gray-50 border border-gray-200 rounded-lg p-4 hover:bg-gray-100 transition-colors cursor-pointer"
                >
                  <div className="flex items-start justify-between">
                    <div>
                      <h4 className="text-base font-semibold text-gray-900">{model.model_id}</h4>
                      <div className="flex flex-wrap gap-3 text-sm text-gray-600 mt-2">
                        {model.context_window > 0 && (
                          <span>Context: {formatContextWindow(model.context_window)}</span>
                        )}
                        {model.supports_streaming && <span>Streaming: Yes</span>}
                      </div>
                      {model.capabilities.length > 0 && (
                        <div className="flex flex-wrap gap-1 mt-2">
                          {model.capabilities.map((cap) => (
                            <Badge key={cap} variant="warning">
                              {cap}
                            </Badge>
                          ))}
                        </div>
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
    {
      id: 'api-keys',
      label: 'API Keys',
      count: apiKeys.length,
      content: (
        <Card>
          <h3 className="text-lg font-semibold text-gray-900 mb-4">API Keys Using This Provider</h3>
          {apiKeys.length === 0 ? (
            <div className="text-center py-8 text-gray-500">
              No API keys are configured to use this provider.
            </div>
          ) : (
            <div className="space-y-2">
              {apiKeys.map((key) => (
                <div
                  key={key.id}
                  onClick={() => onTabChange?.('api-keys', key.id)}
                  className="bg-gray-50 border border-gray-200 rounded-lg p-4 hover:bg-gray-100 transition-colors cursor-pointer"
                >
                  <div className="flex justify-between items-center">
                    <div>
                      <h4 className="text-base font-semibold text-gray-900">{key.name}</h4>
                    </div>
                    <Badge variant={key.enabled ? 'success' : 'warning'}>
                      {key.enabled ? 'Enabled' : 'Disabled'}
                    </Badge>
                  </div>
                </div>
              ))}
            </div>
          )}
        </Card>
      ),
    },
    {
      id: 'chat',
      label: 'Chat',
      content: (
        <Card>
          <h3 className="text-lg font-semibold text-gray-900 mb-4">Chat with Provider</h3>
          <ContextualChat
            context={{
              type: 'provider',
              instanceName,
              providerType,
              models,
            }}
            disabled={!enabled}
          />
        </Card>
      ),
    },
  ]

  return (
    <DetailPageLayout
      icon={<ProviderIcon providerId={providerType} size={48} />}
      title={instanceName}
      subtitle={providerType}
      badges={[
        ...(health ? [{
          label: `${health.status}${health.latency_ms ? ` (${health.latency_ms}ms)` : ''}`,
          variant: healthVariant,
        }] : []),
        {
          label: enabled ? 'Enabled' : 'Disabled',
          variant: enabled ? 'success' : 'warning',
        },
      ]}
      actions={
        <Button
          variant={enabled ? 'secondary' : 'primary'}
          onClick={handleToggleEnabled}
        >
          {enabled ? 'Disable' : 'Enable'}
        </Button>
      }
      tabs={tabs}
      activeTab={activeTab}
      onTabChange={setActiveTab}
      loading={loading}
    />
  )
}
