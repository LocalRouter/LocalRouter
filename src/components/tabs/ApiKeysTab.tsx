import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'
import Modal from '../ui/Modal'
import Input from '../ui/Input'
import Select from '../ui/Select'

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

interface Router {
  name: string
}

export default function ApiKeysTab() {
  const [apiKeys, setApiKeys] = useState<ApiKey[]>([])
  const [loading, setLoading] = useState(true)
  const [showCreateModal, setShowCreateModal] = useState(false)
  const [showKeyModal, setShowKeyModal] = useState(false)
  const [newKeyValue, setNewKeyValue] = useState('')
  const [models, setModels] = useState<Model[]>([])
  const [routers, setRouters] = useState<Router[]>([])
  const [keyCache, setKeyCache] = useState<Map<string, string>>(new Map())
  const [keychainErrors, setKeychainErrors] = useState<Set<string>>(new Set())

  // Form state
  const [keyName, setKeyName] = useState('')
  const [selectedModel, setSelectedModel] = useState('')
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

  const loadRouters = async () => {
    try {
      const routerList = await invoke<Router[]>('list_routers')
      setRouters(routerList)
    } catch (error) {
      console.error('Failed to load routers:', error)
    }
  }

  const handleCreateKey = async (e: React.FormEvent) => {
    e.preventDefault()

    if (!selectedModel) {
      alert('Please select a model')
      return
    }

    setIsCreating(true)

    try {
      let modelSelection: any

      if (selectedModel === 'any') {
        // Use the first available router
        if (routers.length === 0) {
          alert('No routers configured. Please configure a router first.')
          return
        }
        modelSelection = { type: 'router', router_name: routers[0].name }
      } else {
        const [provider, ...modelParts] = selectedModel.split('/')
        const model = modelParts.join('/')
        modelSelection = { type: 'direct_model', provider, model }
      }

      const result = await invoke<[string, ApiKey]>('create_api_key', {
        name: keyName || null,
        modelSelection,
      })

      const [key, keyInfo] = result
      setNewKeyValue(key)
      setKeyCache(new Map(keyCache.set(keyInfo.id, key)))

      setShowCreateModal(false)
      setShowKeyModal(true)
      setKeyName('')
      setSelectedModel('')
      await loadApiKeys()
    } catch (error) {
      console.error('Failed to create API key:', error)
      alert(`Error creating API key: ${error}`)
    } finally {
      setIsCreating(false)
    }
  }

  const handleDeleteKey = async (id: string) => {
    if (!confirm('Delete this API key? This action cannot be undone and will immediately revoke access.')) {
      return
    }

    try {
      await invoke('delete_api_key', { id })
      setKeyCache((cache) => {
        const newCache = new Map(cache)
        newCache.delete(id)
        return newCache
      })
      await loadApiKeys()
      alert('API key deleted successfully')
    } catch (error) {
      console.error('Failed to delete API key:', error)
      alert(`Error deleting API key: ${error}`)
    }
  }

  const handleShowKey = async (id: string) => {
    if (keyCache.has(id) || keychainErrors.has(id)) return

    try {
      const key = await invoke<string>('get_api_key_value', { id })
      setKeyCache(new Map(keyCache.set(id, key)))
    } catch (error) {
      console.error('Failed to load API key:', error)
      setKeychainErrors(new Set(keychainErrors.add(id)))
      alert(`Failed to load API key: ${error}`)
    }
  }

  const handleCopyKey = async (id: string) => {
    let key = keyCache.get(id)

    if (!key) {
      if (keychainErrors.has(id)) {
        alert('This API key cannot be retrieved from the keychain.')
        return
      }

      try {
        key = await invoke<string>('get_api_key_value', { id })
        setKeyCache(new Map(keyCache.set(id, key)))
      } catch (error) {
        console.error('Failed to load API key:', error)
        setKeychainErrors(new Set(keychainErrors.add(id)))
        alert(`Failed to load API key: ${error}`)
        return
      }
    }

    try {
      await navigator.clipboard.writeText(key)
      alert('API key copied to clipboard!')
    } catch (error) {
      console.error('Failed to copy:', error)
      alert('Failed to copy key to clipboard')
    }
  }

  const formatModelSelection = (selection: any) => {
    if (typeof selection === 'string') return selection
    if (selection.type === 'router') return 'All'
    if (selection.type === 'direct_model') return `${selection.provider}/${selection.model}`
    return 'Unknown'
  }

  const handleOpenCreateModal = async () => {
    await loadModels()
    await loadRouters()
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

  // Group models by provider
  const groupedModels: Record<string, Model[]> = models.reduce((acc, model) => {
    if (!acc[model.provider]) acc[model.provider] = []
    acc[model.provider].push(model)
    return acc
  }, {} as Record<string, Model[]>)

  return (
    <div>
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
          <ul className="space-y-3">
            {apiKeys.map((key) => {
              const hasKey = keyCache.has(key.id)
              const keyValue = keyCache.get(key.id) || ''
              const hasError = keychainErrors.has(key.id)

              return (
                <li key={key.id} className="bg-gray-50 border border-gray-200 rounded-lg p-4">
                  <div className="flex justify-between items-start mb-2">
                    <div>
                      <h3 className="text-base font-semibold text-gray-900">{key.name}</h3>
                      <p className="text-sm text-gray-500 mt-1">
                        ID: {key.id.substring(0, 8)}... |{' '}
                        {formatModelSelection(key.model_selection)} |{' '}
                        Created: {new Date(key.created_at).toLocaleDateString()}
                      </p>
                    </div>
                    <div className="flex gap-2 items-center">
                      <Badge variant={key.enabled ? 'success' : 'error'}>
                        {key.enabled ? 'Enabled' : 'Disabled'}
                      </Badge>
                      <Button
                        variant="danger"
                        onClick={() => handleDeleteKey(key.id)}
                        className="px-3 py-1.5 text-xs"
                      >
                        Delete
                      </Button>
                    </div>
                  </div>

                  {hasError ? (
                    <div className="mt-3 px-3 py-2 bg-red-50 border border-red-200 rounded-md text-sm text-red-700">
                      ⚠️ Key not found in keychain. This key may have been created before keychain support was added.
                    </div>
                  ) : (
                    <div className="flex items-center gap-2 mt-3 px-2 py-2 bg-white border border-gray-200 rounded-md">
                      <input
                        type={hasKey ? 'password' : 'password'}
                        value={hasKey ? keyValue : '••••••••••••••••'}
                        readOnly
                        className="flex-1 font-mono text-sm px-2 bg-transparent text-gray-900 border-none outline-none"
                      />
                      {!hasKey ? (
                        <button
                          onClick={() => handleShowKey(key.id)}
                          className="px-2 py-1 hover:bg-gray-100 rounded text-xl"
                          title="Show"
                        >
                          👁️
                        </button>
                      ) : null}
                      <button
                        onClick={() => handleCopyKey(key.id)}
                        className="px-2 py-1 hover:bg-gray-100 rounded text-xl"
                        title="Copy"
                      >
                        📋
                      </button>
                    </div>
                  )}
                </li>
              )
            })}
          </ul>
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

          <Select
            label="Select Model"
            value={selectedModel}
            onChange={(e) => setSelectedModel(e.target.value)}
            required
            helperText="Select 'All' for smart routing, or choose a specific model. Make sure the provider is enabled in the Providers tab."
          >
            <option value="">Select a model...</option>
            <option value="any">All</option>
            <option disabled>──────────</option>
            {Object.keys(groupedModels).sort().map((provider) => (
              <optgroup key={provider} label={provider}>
                {groupedModels[provider].map((model) => (
                  <option key={`${provider}/${model.id}`} value={`${provider}/${model.id}`}>
                    {model.id}
                  </option>
                ))}
              </optgroup>
            ))}
            {models.length === 0 && (
              <option disabled className="text-gray-400 text-sm">
                No specific models available - enable providers in Providers tab
              </option>
            )}
          </Select>

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
