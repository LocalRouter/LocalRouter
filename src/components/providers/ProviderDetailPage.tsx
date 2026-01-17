import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Badge from '../ui/Badge'
import Button from '../ui/Button'
import ProviderIcon from '../ProviderIcon'
import { ContextualChat } from '../chat/ContextualChat'

interface ProviderDetailPageProps {
  instanceName: string
  providerType: string
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

export default function ProviderDetailPage({
  instanceName,
  providerType,
}: ProviderDetailPageProps) {
  const [health, setHealth] = useState<ProviderHealth | null>(null)
  const [models, setModels] = useState<Model[]>([])
  const [config, setConfig] = useState<Record<string, string>>({})
  const [enabled, setEnabled] = useState(true)
  const [loading, setLoading] = useState(true)
  const [isSaving, setIsSaving] = useState(false)
  const [activeTab, setActiveTab] = useState<'details' | 'models' | 'chat'>('details')

  useEffect(() => {
    loadProviderData()
  }, [instanceName])

  const loadProviderData = async () => {
    setLoading(true)
    try {
      const [healthData, basicModels, configData, instances] = await Promise.all([
        invoke<Record<string, ProviderHealth>>('get_providers_health'),
        invoke<Array<{ id: string; provider: string }>>('list_all_models').catch(() => []),
        invoke<Record<string, string>>('get_provider_config', { instanceName }),
        invoke<Array<{ instance_name: string; enabled: boolean }>>('list_provider_instances'),
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

  if (loading) {
    return (
      <div className="bg-white rounded-lg shadow-sm p-6">
        <div className="text-center py-8 text-gray-500">Loading provider details...</div>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <Card>
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-4">
            <ProviderIcon providerId={providerType} size={48} />
            <div>
              <h2 className="text-2xl font-bold text-gray-900">{instanceName}</h2>
              <p className="text-sm text-gray-500">{providerType}</p>
            </div>
          </div>
          <div className="flex items-center gap-3">
            {health && (
              <div className="flex items-center gap-2">
                <Badge variant={healthVariant}>{health.status}</Badge>
                {health.latency_ms && (
                  <span className="text-sm text-gray-500">{health.latency_ms}ms</span>
                )}
              </div>
            )}
            <Badge variant={enabled ? 'success' : 'warning'}>
              {enabled ? 'Enabled' : 'Disabled'}
            </Badge>
            <Button
              variant={enabled ? 'secondary' : 'primary'}
              onClick={handleToggleEnabled}
            >
              {enabled ? 'Disable' : 'Enable'}
            </Button>
          </div>
        </div>
      </Card>

      {/* Tabs */}
      <div className="flex border-b border-gray-200">
        <button
          onClick={() => setActiveTab('details')}
          className={`px-6 py-3 font-medium transition-colors ${
            activeTab === 'details'
              ? 'border-b-2 border-blue-500 text-blue-600'
              : 'text-gray-600 hover:text-gray-900'
          }`}
        >
          Details & Configuration
        </button>
        <button
          onClick={() => setActiveTab('models')}
          className={`px-6 py-3 font-medium transition-colors ${
            activeTab === 'models'
              ? 'border-b-2 border-blue-500 text-blue-600'
              : 'text-gray-600 hover:text-gray-900'
          }`}
        >
          Models ({models.length})
        </button>
        <button
          onClick={() => setActiveTab('chat')}
          className={`px-6 py-3 font-medium transition-colors ${
            activeTab === 'chat'
              ? 'border-b-2 border-blue-500 text-blue-600'
              : 'text-gray-600 hover:text-gray-900'
          }`}
        >
          Chat
        </button>
      </div>

      {/* Tab Content */}
      {activeTab === 'details' && (
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
      )}

      {activeTab === 'models' && (
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
                  className="bg-gray-50 border border-gray-200 rounded-lg p-4 hover:bg-gray-100 transition-colors"
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
      )}

      {activeTab === 'chat' && (
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
      )}
    </div>
  )
}
