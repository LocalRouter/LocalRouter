import { useState, useEffect, useMemo } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import DetailPageLayout from '../layouts/DetailPageLayout'
import { ContextualChat } from '../chat/ContextualChat'
import ModelSelectionTable, { ModelSelectionValue } from '../ModelSelectionTable'
import { MetricsChart } from '../charts/MetricsChart'
import { useMetricsSubscription } from '../../hooks/useMetricsSubscription'

interface ApiKeyDetailPageProps {
  keyId: string
}

interface ApiKey {
  id: string
  name: string
  enabled: boolean
  created_at: string
  model_selection: any
}

interface Model {
  id: string
  provider: string
}

export default function ApiKeyDetailPage({ keyId }: ApiKeyDetailPageProps) {
  const refreshKey = useMetricsSubscription()
  const [apiKey, setApiKey] = useState<ApiKey | null>(null)
  const [keyValue, setKeyValue] = useState<string>('')
  const [showKey, setShowKey] = useState(false)
  const [keyLoaded, setKeyLoaded] = useState(false)
  const [keyLoadError, setKeyLoadError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [activeTab, setActiveTab] = useState<string>('metrics')
  const [isSaving, setIsSaving] = useState(false)

  // Form state
  const [name, setName] = useState('')
  const [enabled, setEnabled] = useState(true)

  // Model selection state
  const [models, setModels] = useState<Model[]>([])
  const [modelSelection, setModelSelection] = useState<ModelSelectionValue>({ type: 'all' })

  useEffect(() => {
    loadApiKeyData()
  }, [keyId])

  const loadApiKeyData = async () => {
    setLoading(true)
    try {
      const [keys, modelList] = await Promise.all([
        invoke<ApiKey[]>('list_api_keys'),
        invoke<Model[]>('list_all_models').catch(() => []),
      ])

      const key = keys.find((k) => k.id === keyId)
      if (key) {
        setApiKey(key)
        setName(key.name)
        setEnabled(key.enabled)
        setModelSelection(key.model_selection || { type: 'all' })
      }

      setModels(modelList)
    } catch (error) {
      console.error('Failed to load API key data:', error)
    } finally {
      setLoading(false)
    }
  }

  const loadKeyValue = async () => {
    if (keyLoaded) {
      setShowKey(!showKey)
      return
    }

    try {
      setKeyLoadError(null)
      const value = await invoke<string>('get_api_key_value', { id: keyId })
      setKeyValue(value)
      setKeyLoaded(true)
      setShowKey(true)
    } catch (error: any) {
      console.error('Failed to load API key value:', error)
      const errorMsg = error?.toString() || 'Unknown error'
      if (errorMsg.includes('passphrase') || errorMsg.includes('keychain')) {
        setKeyLoadError('Keychain access denied. Please approve keychain access or enter your password to view the API key.')
      } else {
        setKeyLoadError(`Failed to load key: ${errorMsg}`)
      }
    }
  }

  const handleSaveSettings = async () => {
    setIsSaving(true)
    try {
      if (name !== apiKey?.name) {
        await invoke('update_api_key_name', { id: keyId, name })
      }

      if (enabled !== apiKey?.enabled) {
        await invoke('toggle_api_key_enabled', { id: keyId, enabled })
      }

      await loadApiKeyData()
      alert('Settings saved successfully!')
    } catch (error) {
      console.error('Failed to save settings:', error)
      alert(`Error saving settings: ${error}`)
    } finally {
      setIsSaving(false)
    }
  }

  const handleSaveModelSelection = async () => {
    setIsSaving(true)
    try {
      await invoke('update_api_key_model', {
        id: keyId,
        modelSelection,
      })

      await loadApiKeyData()
      alert('Model selection updated successfully!')
    } catch (error) {
      console.error('Failed to update model selection:', error)
      alert(`Error updating model selection: ${error}`)
    } finally {
      setIsSaving(false)
    }
  }

  const handleCopyKey = async () => {
    // Load key if not already loaded
    if (!keyLoaded) {
      await loadKeyValue()
      if (!keyLoaded) return // Failed to load
    }

    try {
      await navigator.clipboard.writeText(keyValue)
      alert('API key copied to clipboard!')
    } catch (error) {
      console.error('Failed to copy:', error)
      alert('Failed to copy key to clipboard')
    }
  }

  if (loading || !apiKey) {
    return (
      <div className="bg-white rounded-lg shadow-sm p-6">
        <div className="text-center py-8 text-gray-500">Loading API key details...</div>
      </div>
    )
  }

  // Memoize context object to prevent re-renders
  const chatContext = useMemo(() => ({
    type: 'api_key' as const,
    apiKeyId: keyId,
    apiKeyName: apiKey.name,
    modelSelection: apiKey.model_selection,
  }), [keyId, apiKey.name, apiKey.model_selection]);

  const tabs = [
    {
      id: 'metrics',
      label: 'Metrics',
      content: (
        <div className="space-y-6">
          <div className="grid grid-cols-2 gap-6">
            <MetricsChart
              scope="api_key"
              scopeId={keyId}
              timeRange="day"
              metricType="requests"
              title="Requests"
              refreshTrigger={refreshKey}
            />

            <MetricsChart
              scope="api_key"
              scopeId={keyId}
              timeRange="day"
              metricType="tokens"
              title="Tokens"
              refreshTrigger={refreshKey}
            />

            <MetricsChart
              scope="api_key"
              scopeId={keyId}
              timeRange="day"
              metricType="cost"
              title="Cost"
              refreshTrigger={refreshKey}
            />

            <MetricsChart
              scope="api_key"
              scopeId={keyId}
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
      id: 'settings',
      label: 'Settings',
      content: (
        <div className="space-y-6">
          <Card>
            <h3 className="text-lg font-semibold text-gray-900 mb-4">API Key Value</h3>
            <div className="space-y-3">
              {keyLoadError && (
                <div className="p-3 bg-red-50 border border-red-200 rounded-lg text-sm text-red-700">
                  {keyLoadError}
                </div>
              )}
              {!keyLoaded ? (
                <div className="text-center py-4">
                  <Button onClick={loadKeyValue}>
                    Load API Key from Keychain
                  </Button>
                  <p className="text-xs text-gray-500 mt-2">
                    This will prompt for your system password to access the secure keychain.
                  </p>
                </div>
              ) : (
                <>
                  <div className="flex items-center gap-2 px-3 py-2 bg-gray-50 border border-gray-200 rounded-lg">
                    <input
                      type={showKey ? 'text' : 'password'}
                      value={keyValue}
                      readOnly
                      className="flex-1 font-mono text-sm bg-transparent outline-none"
                    />
                    <button
                      onClick={() => setShowKey(!showKey)}
                      className="px-2 py-1 hover:bg-gray-200 rounded text-xl"
                      title={showKey ? 'Hide' : 'Show'}
                    >
                      {showKey ? '🙈' : '👁️'}
                    </button>
                    <button
                      onClick={handleCopyKey}
                      className="px-2 py-1 hover:bg-gray-200 rounded text-xl"
                      title="Copy"
                    >
                      📋
                    </button>
                  </div>
                  <p className="text-xs text-gray-500">
                    Keep this key secret and secure. Anyone with access to this key can use your API access.
                  </p>
                </>
              )}
            </div>
          </Card>

          <Card>
            <h3 className="text-lg font-semibold text-gray-900 mb-4">Configuration</h3>
            <div className="space-y-4">
              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">
                  Name
                </label>
                <input
                  type="text"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  className="w-full px-3 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
                />
              </div>

              <div>
                <label className="flex items-center gap-2">
                  <input
                    type="checkbox"
                    checked={enabled}
                    onChange={(e) => setEnabled(e.target.checked)}
                    className="w-4 h-4 text-blue-600 rounded focus:ring-blue-500"
                  />
                  <span className="text-sm font-medium text-gray-700">Enabled</span>
                </label>
                <p className="text-xs text-gray-500 mt-1 ml-6">
                  Disabled API keys cannot be used to authenticate requests.
                </p>
              </div>
            </div>

            <div className="mt-6 flex justify-end">
              <Button onClick={handleSaveSettings} disabled={isSaving}>
                {isSaving ? 'Saving...' : 'Save Changes'}
              </Button>
            </div>
          </Card>
        </div>
      ),
    },
    {
      id: 'models',
      label: 'Model Selection',
      content: (
        <Card>
          <h3 className="text-lg font-semibold text-gray-900 mb-4">Model Selection</h3>
          <p className="text-sm text-gray-500 mb-4">
            Configure which models this API key can access. Select "All" to allow all providers and models,
            select individual providers to allow all their models, or select specific models for fine-grained control.
          </p>
          <ModelSelectionTable
            models={models}
            value={modelSelection}
            onChange={setModelSelection}
          />
          <div className="mt-6 flex justify-end">
            <Button onClick={handleSaveModelSelection} disabled={isSaving}>
              {isSaving ? 'Saving...' : 'Save Changes'}
            </Button>
          </div>
        </Card>
      ),
    },
    {
      id: 'chat',
      label: 'Chat',
      content: (
        <Card>
          <h3 className="text-lg font-semibold text-gray-900 mb-4">Chat</h3>
          <ContextualChat
            context={chatContext}
            disabled={!apiKey.enabled}
          />
        </Card>
      ),
    },
  ]

  return (
    <DetailPageLayout
      title={apiKey.name}
      subtitle={`ID: ${apiKey.id.substring(0, 16)}... | Created: ${new Date(apiKey.created_at).toLocaleDateString()}`}
      badges={[
        {
          label: apiKey.enabled ? 'Enabled' : 'Disabled',
          variant: apiKey.enabled ? 'success' : 'warning',
        },
      ]}
      tabs={tabs}
      activeTab={activeTab}
      onTabChange={setActiveTab}
      loading={loading}
    />
  )
}
