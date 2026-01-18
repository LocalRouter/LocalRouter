import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'
import Modal from '../ui/Modal'
import Input from '../ui/Input'
import ModelSelectionTable, { ModelSelectionValue } from '../ModelSelectionTable'
import ApiKeyDetailPage from '../apikeys/ApiKeyDetailPage'
import { MetricsChart } from '../charts/MetricsChart'
import { useMetricsSubscription } from '../../hooks/useMetricsSubscription'

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

interface ApiKeysTabProps {
  activeSubTab: string | null
  onTabChange?: (tab: 'api-keys', subTab: string) => void
}

export default function ApiKeysTab({ activeSubTab, onTabChange }: ApiKeysTabProps) {
  const refreshKey = useMetricsSubscription()
  const [apiKeys, setApiKeys] = useState<ApiKey[]>([])
  const [loading, setLoading] = useState(true)
  const [showCreateModal, setShowCreateModal] = useState(false)
  const [showKeyModal, setShowKeyModal] = useState(false)
  const [newKeyValue, setNewKeyValue] = useState('')
  const [models, setModels] = useState<Model[]>([])

  // Form state
  const [keyName, setKeyName] = useState('')
  const [modelSelection, setModelSelection] = useState<ModelSelectionValue>({ type: 'all' })
  const [isCreating, setIsCreating] = useState(false)

  useEffect(() => {
    loadApiKeys()
  }, [])

  const loadApiKeys = async () => {
    setLoading(true)
    try {
      const keys = await invoke<ApiKey[]>('list_api_keys')
      setApiKeys(keys)
    } catch (error) {
      console.error('Failed to load API keys:', error)
      alert(`Error loading API keys: ${error}`)
    } finally {
      setLoading(false)
    }
  }

  const loadModels = async () => {
    try {
      const modelList = await invoke<Model[]>('list_all_models')
      setModels(modelList)
    } catch (error) {
      console.error('Failed to load models:', error)
    }
  }

  const handleCreateKey = async (e: React.FormEvent) => {
    e.preventDefault()

    setIsCreating(true)

    try {
      const result = await invoke<[string, ApiKey]>('create_api_key', {
        name: keyName || null,
        modelSelection,
      })

      const [key, _keyInfo] = result
      setNewKeyValue(key)

      setShowCreateModal(false)
      setShowKeyModal(true)
      setKeyName('')
      setModelSelection({ type: 'all' })
      await loadApiKeys()
    } catch (error) {
      console.error('Failed to create API key:', error)
      alert(`Error creating API key: ${error}`)
    } finally {
      setIsCreating(false)
    }
  }

  const handleOpenCreateModal = async () => {
    await loadModels()
    setModelSelection({ type: 'all' }) // Reset to "All" when opening
    setShowCreateModal(true)
  }

  const copyNewKey = async () => {
    try {
      await navigator.clipboard.writeText(newKeyValue)
      alert('API key copied to clipboard!')
    } catch (error) {
      console.error('Failed to copy:', error)
    }
  }

  // If a sub-tab is selected, show detail page for that specific API key
  if (activeSubTab) {
    const key = apiKeys.find(k => k.id === activeSubTab)

    if (!key && !loading) {
      return (
        <div className="bg-white rounded-lg shadow-sm p-6">
          <h2 className="text-2xl font-bold text-gray-800 mb-4">API Key Not Found</h2>
          <p className="text-gray-600">The requested API key could not be found.</p>
        </div>
      )
    }

    if (loading || !key) {
      return (
        <div className="bg-white rounded-lg shadow-sm p-6">
          <div className="text-center py-8 text-gray-500">Loading API key details...</div>
        </div>
      )
    }

    return <ApiKeyDetailPage keyId={key.id} />
  }

  return (
    <div className="space-y-6">
      {/* Metrics Overview */}
      {!loading && apiKeys.length > 0 && (
        <Card>
          <h3 className="text-lg font-semibold text-gray-900 mb-4">API Key Usage Overview</h3>
          <div className="grid grid-cols-2 gap-4">
            <MetricsChart
              scope="global"
              timeRange="day"
              metricType="requests"
              title="Total Requests by API Key"
              refreshTrigger={refreshKey}
            />
            <MetricsChart
              scope="global"
              timeRange="day"
              metricType="cost"
              title="Total Cost by API Key"
              refreshTrigger={refreshKey}
            />
          </div>
        </Card>
      )}

      <Card>
        <div className="flex justify-between items-center mb-6">
          <h2 className="text-xl font-bold text-gray-900">API Keys</h2>
          <Button onClick={handleOpenCreateModal}>+ Create New Key</Button>
        </div>

        {loading ? (
          <div className="text-center py-8 text-gray-500">Loading API keys...</div>
        ) : apiKeys.length === 0 ? (
          <div className="text-center py-12 text-gray-500">
            <p>No API keys found. Create your first key to get started.</p>
          </div>
        ) : (
          <div className="grid gap-3">
            {apiKeys.map((key) => (
              <div
                key={key.id}
                onClick={() => onTabChange?.('api-keys', key.id)}
                className="bg-gray-50 border border-gray-200 rounded-lg p-4 hover:bg-gray-100 transition-colors cursor-pointer"
              >
                <div className="flex justify-between items-start">
                  <div>
                    <h3 className="text-base font-semibold text-gray-900">{key.name}</h3>
                    <p className="text-sm text-gray-500 mt-1">
                      Created: {new Date(key.created_at).toLocaleDateString()}
                    </p>
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

      {/* Create Key Modal */}
      <Modal
        isOpen={showCreateModal}
        onClose={() => setShowCreateModal(false)}
        title="Create API Key"
      >
        <form onSubmit={handleCreateKey}>
          <Input
            label="Key Name (optional)"
            placeholder="My Application Key"
            value={keyName}
            onChange={(e) => setKeyName(e.target.value)}
          />

          <div className="mt-4">
            <label className="block text-sm font-medium text-gray-700 mb-2">
              Model Selection
            </label>
            <p className="text-sm text-gray-500 mb-3">
              Select which models this API key can access. Check "All" to allow all providers and models (including future ones),
              check individual providers to allow all their models, or check specific models for fine-grained control.
            </p>
            <ModelSelectionTable
              models={models}
              value={modelSelection}
              onChange={setModelSelection}
            />
          </div>

          <div className="flex gap-2 mt-6">
            <Button type="submit" disabled={isCreating}>
              {isCreating ? 'Creating...' : 'Create Key'}
            </Button>
            <Button
              type="button"
              variant="secondary"
              onClick={() => setShowCreateModal(false)}
            >
              Cancel
            </Button>
          </div>
        </form>
      </Modal>

      {/* New Key Display Modal */}
      <Modal
        isOpen={showKeyModal}
        onClose={() => setShowKeyModal(false)}
        title="API Key Created"
      >
        <p className="text-gray-600 mb-4">
          Save this key securely. You won't be able to see it again.
        </p>
        <div className="bg-gray-900 text-gray-200 p-4 rounded-md font-mono text-sm break-all mb-6">
          {newKeyValue}
        </div>
        <div className="flex gap-2">
          <Button onClick={copyNewKey}>Copy to Clipboard</Button>
          <Button variant="secondary" onClick={() => setShowKeyModal(false)}>
            Close
          </Button>
        </div>
      </Modal>
    </div>
  )
}
