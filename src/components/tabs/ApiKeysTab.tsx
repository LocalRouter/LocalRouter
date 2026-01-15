import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'
import Modal from '../ui/Modal'
import Input from '../ui/Input'
import ModelSelectionTable, { ModelSelectionValue } from '../ModelSelectionTable'

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

export default function ApiKeysTab() {
  const [apiKeys, setApiKeys] = useState<ApiKey[]>([])
  const [loading, setLoading] = useState(true)
  const [showCreateModal, setShowCreateModal] = useState(false)
  const [showKeyModal, setShowKeyModal] = useState(false)
  const [showRotateConfirm, setShowRotateConfirm] = useState(false)
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false)
  const [showEditModelsModal, setShowEditModelsModal] = useState(false)
  const [rotateKeyId, setRotateKeyId] = useState<string | null>(null)
  const [deleteKeyId, setDeleteKeyId] = useState<string | null>(null)
  const [editKeyId, setEditKeyId] = useState<string | null>(null)
  const [newKeyValue, setNewKeyValue] = useState('')
  const [models, setModels] = useState<Model[]>([])
  const [keyCache, setKeyCache] = useState<Map<string, string>>(new Map())
  const [keychainErrors, setKeychainErrors] = useState<Set<string>>(new Set())
  const [visibleKeys, setVisibleKeys] = useState<Set<string>>(new Set())
  const [copiedKeyId, setCopiedKeyId] = useState<string | null>(null)

  // Name editing state
  const [editingNameId, setEditingNameId] = useState<string | null>(null)
  const [editingNameValue, setEditingNameValue] = useState('')

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

      const [key, keyInfo] = result
      setNewKeyValue(key)
      setKeyCache(new Map(keyCache.set(keyInfo.id, key)))

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

  const handleDeleteKeyRequest = (id: string) => {
    setDeleteKeyId(id)
    setShowDeleteConfirm(true)
  }

  const handleDeleteKeyConfirm = async () => {
    if (!deleteKeyId) return

    setShowDeleteConfirm(false)

    try {
      await invoke('delete_api_key', { id: deleteKeyId })
      setKeyCache((cache) => {
        const newCache = new Map(cache)
        newCache.delete(deleteKeyId)
        return newCache
      })
      await loadApiKeys()
      setDeleteKeyId(null)
      alert('API key deleted successfully')
    } catch (error) {
      console.error('Failed to delete API key:', error)
      alert(`Error deleting API key: ${error}`)
      setDeleteKeyId(null)
    }
  }

  const handleDeleteKeyCancel = () => {
    setShowDeleteConfirm(false)
    setDeleteKeyId(null)
  }

  const handleRotateKeyRequest = (id: string) => {
    setRotateKeyId(id)
    setShowRotateConfirm(true)
  }

  const handleRotateKeyConfirm = async () => {
    if (!rotateKeyId) return

    setShowRotateConfirm(false)

    try {
      const newKey = await invoke<string>('rotate_api_key', { id: rotateKeyId })
      setNewKeyValue(newKey)
      setKeyCache(new Map(keyCache.set(rotateKeyId, newKey)))
      setVisibleKeys(prev => new Set(prev.add(rotateKeyId)))
      setShowKeyModal(true)
      setRotateKeyId(null)
      alert('API key rotated successfully. Make sure to update your applications with the new key.')
    } catch (error) {
      console.error('Failed to rotate API key:', error)
      alert(`Error rotating API key: ${error}`)
      setRotateKeyId(null)
    }
  }

  const handleRotateKeyCancel = () => {
    setShowRotateConfirm(false)
    setRotateKeyId(null)
  }

  const handleToggleKeyVisibility = async (id: string) => {
    if (keychainErrors.has(id)) return

    // If already visible, just hide it
    if (visibleKeys.has(id)) {
      setVisibleKeys(prev => {
        const newSet = new Set(prev)
        newSet.delete(id)
        return newSet
      })
      return
    }

    // If not in cache, fetch it
    if (!keyCache.has(id)) {
      try {
        const key = await invoke<string>('get_api_key_value', { id })
        setKeyCache(new Map(keyCache.set(id, key)))
      } catch (error) {
        console.error('Failed to load API key:', error)
        setKeychainErrors(new Set(keychainErrors.add(id)))
        alert(`Failed to load API key: ${error}`)
        return
      }
    }

    // Show the key
    setVisibleKeys(prev => new Set(prev.add(id)))
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
      setCopiedKeyId(id)
      setTimeout(() => setCopiedKeyId(null), 2000)
    } catch (error) {
      console.error('Failed to copy:', error)
      alert('Failed to copy key to clipboard')
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

  const handleOpenEditModelsModal = async (keyId: string) => {
    const key = apiKeys.find(k => k.id === keyId)
    if (!key) return

    await loadModels()
    setEditKeyId(keyId)
    setModelSelection(key.model_selection || { type: 'all' })
    setShowEditModelsModal(true)
  }

  const handleSaveModelSelection = async () => {
    if (!editKeyId) return

    setIsCreating(true)

    try {
      await invoke('update_api_key_model', {
        id: editKeyId,
        modelSelection,
      })

      await loadApiKeys()
      setShowEditModelsModal(false)
      setEditKeyId(null)
      alert('Model selection updated successfully')
    } catch (error) {
      console.error('Failed to update model selection:', error)
      alert(`Error updating model selection: ${error}`)
    } finally {
      setIsCreating(false)
    }
  }

  const handleToggleEnabled = async (id: string, currentEnabled: boolean) => {
    try {
      await invoke('toggle_api_key_enabled', {
        id,
        enabled: !currentEnabled,
      })

      await loadApiKeys()
    } catch (error) {
      console.error('Failed to toggle API key:', error)
      alert(`Error toggling API key: ${error}`)
    }
  }

  const handleStartEditName = (id: string, currentName: string) => {
    setEditingNameId(id)
    setEditingNameValue(currentName)
  }

  const handleCancelEditName = () => {
    setEditingNameId(null)
    setEditingNameValue('')
  }

  const handleSaveName = async (id: string) => {
    if (editingNameValue.trim() === '') {
      alert('API key name cannot be empty')
      return
    }

    try {
      await invoke('update_api_key_name', {
        id,
        name: editingNameValue.trim(),
      })

      await loadApiKeys()
      setEditingNameId(null)
      setEditingNameValue('')
    } catch (error) {
      console.error('Failed to update API key name:', error)
      alert(`Error updating API key name: ${error}`)
    }
  }

  const handleNameKeyDown = (e: React.KeyboardEvent, id: string) => {
    if (e.key === 'Enter') {
      handleSaveName(id)
    } else if (e.key === 'Escape') {
      handleCancelEditName()
    }
  }

  const formatModelSelectionDetailed = (selection: any) => {
    if (!selection) return []
    if (selection.type === 'all') return ['All providers and models']

    if (selection.type === 'custom') {
      const providers = selection.all_provider_models || []
      const models = selection.individual_models || []

      const items: string[] = []

      if (providers.length > 0) {
        items.push(...providers.map((p: string) => `${p}/* (all models)`))
      }

      if (models.length > 0) {
        items.push(...models.map(([provider, model]: [string, string]) => `${provider}/${model}`))
      }

      return items.length > 0 ? items : ['No models selected']
    }

    return ['Unknown selection']
  }

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

              const modelDetails = formatModelSelectionDetailed(key.model_selection)

              return (
                <li key={key.id} className="bg-gray-50 border border-gray-200 rounded-lg p-4">
                  <div className="flex justify-between items-start mb-2">
                    <div className="flex-1">
                      {editingNameId === key.id ? (
                        <div className="flex items-center gap-2 mb-1">
                          <input
                            type="text"
                            value={editingNameValue}
                            onChange={(e) => setEditingNameValue(e.target.value)}
                            onKeyDown={(e) => handleNameKeyDown(e, key.id)}
                            onBlur={() => handleSaveName(key.id)}
                            autoFocus
                            className="text-base font-semibold text-gray-900 px-2 py-1 border border-blue-500 rounded focus:outline-none focus:ring-2 focus:ring-blue-500"
                          />
                          <button
                            onClick={() => handleSaveName(key.id)}
                            className="px-2 py-1 bg-blue-600 text-white text-sm rounded hover:bg-blue-700"
                          >
                            Save
                          </button>
                          <button
                            onClick={handleCancelEditName}
                            className="px-2 py-1 bg-gray-300 text-gray-700 text-sm rounded hover:bg-gray-400"
                          >
                            Cancel
                          </button>
                        </div>
                      ) : (
                        <div className="flex items-center gap-2 group mb-1">
                          <h3 className="text-base font-semibold text-gray-900">{key.name}</h3>
                          <button
                            onClick={() => handleStartEditName(key.id, key.name)}
                            className="opacity-0 group-hover:opacity-100 transition-opacity px-1.5 py-0.5 text-xs text-gray-600 hover:text-blue-600"
                            title="Edit name"
                          >
                            ✏️
                          </button>
                        </div>
                      )}
                      <p className="text-sm text-gray-500 mt-1">
                        ID: {key.id.substring(0, 8)}... |{' '}
                        Created: {new Date(key.created_at).toLocaleDateString()}
                      </p>

                      {/* Model Selection - Always Visible */}
                      <div className="mt-2">
                        <p className="text-sm font-medium text-gray-700 mb-1">
                          Model Selection:
                        </p>
                        <div className="pl-2 border-l-2 border-gray-300">
                          <div className="text-sm text-gray-600 space-y-0.5">
                            {modelDetails.map((item, idx) => (
                              <div key={idx} className="font-mono">{item}</div>
                            ))}
                          </div>
                        </div>
                      </div>
                    </div>
                    <div className="flex gap-2 items-center">
                      <Badge variant={key.enabled ? 'success' : 'error'}>
                        {key.enabled ? 'Enabled' : 'Disabled'}
                      </Badge>
                      <Button
                        variant={key.enabled ? 'secondary' : 'primary'}
                        onClick={() => handleToggleEnabled(key.id, key.enabled)}
                        className="px-3 py-1.5 text-xs"
                      >
                        {key.enabled ? 'Disable' : 'Enable'}
                      </Button>
                      <Button
                        variant="secondary"
                        onClick={() => handleOpenEditModelsModal(key.id)}
                        className="px-3 py-1.5 text-xs"
                      >
                        Edit Models
                      </Button>
                      <Button
                        variant="secondary"
                        onClick={() => handleRotateKeyRequest(key.id)}
                        className="px-3 py-1.5 text-xs"
                      >
                        Rotate
                      </Button>
                      <Button
                        variant="danger"
                        onClick={() => handleDeleteKeyRequest(key.id)}
                        className="px-3 py-1.5 text-xs"
                      >
                        Delete
                      </Button>
                    </div>
                  </div>

                  {hasError ? (
                    <div className="mt-3 px-3 py-2 bg-red-50 border border-red-200 rounded-md text-sm text-red-700">
                      ⚠️ Key not found in keychain. This key may have expired.
                    </div>
                  ) : (
                    <div className="flex items-center gap-2 mt-3 px-2 py-2 bg-white border border-gray-200 rounded-md">
                      <input
                        type={visibleKeys.has(key.id) ? 'text' : 'password'}
                        value={hasKey ? keyValue : '••••••••••••••••'}
                        readOnly
                        className="flex-1 font-mono text-sm px-2 bg-transparent text-gray-900 border-none outline-none"
                      />
                      <button
                        onClick={() => handleToggleKeyVisibility(key.id)}
                        className="px-2 py-1 hover:bg-gray-100 rounded text-xl"
                        title={visibleKeys.has(key.id) ? "Hide" : "Show"}
                      >
                        {visibleKeys.has(key.id) ? '🙈' : '👁️'}
                      </button>
                      <button
                        onClick={() => handleCopyKey(key.id)}
                        className="px-2 py-1 hover:bg-gray-100 rounded text-xl"
                        title="Copy"
                      >
                        {copiedKeyId === key.id ? '✅' : '📋'}
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

      {/* Rotate Key Confirmation Modal */}
      <Modal
        isOpen={showRotateConfirm}
        onClose={handleRotateKeyCancel}
        title="Rotate API Key"
      >
        <p className="text-gray-600 mb-4">
          Are you sure you want to rotate this API key?
        </p>
        <p className="text-gray-700 mb-6 font-semibold">
          The old key will be immediately invalidated and a new key will be generated.
          You'll need to update all applications using this key.
        </p>
        <div className="flex gap-2">
          <Button onClick={handleRotateKeyConfirm}>
            Yes, Rotate Key
          </Button>
          <Button variant="secondary" onClick={handleRotateKeyCancel}>
            Cancel
          </Button>
        </div>
      </Modal>

      {/* Delete Key Confirmation Modal */}
      <Modal
        isOpen={showDeleteConfirm}
        onClose={handleDeleteKeyCancel}
        title="Delete API Key"
      >
        <p className="text-gray-600 mb-4">
          Are you sure you want to delete this API key?
        </p>
        <p className="text-red-700 mb-6 font-semibold">
          This action cannot be undone and will immediately revoke access for all applications using this key.
        </p>
        <div className="flex gap-2">
          <Button variant="danger" onClick={handleDeleteKeyConfirm}>
            Yes, Delete Key
          </Button>
          <Button variant="secondary" onClick={handleDeleteKeyCancel}>
            Cancel
          </Button>
        </div>
      </Modal>

      {/* Edit Models Modal */}
      <Modal
        isOpen={showEditModelsModal}
        onClose={() => setShowEditModelsModal(false)}
        title="Edit Model Selection"
      >
        <div className="mb-4">
          <p className="text-sm text-gray-600 mb-3">
            Update which models this API key can access. Check "All" to allow all providers and models,
            check individual providers to allow all their models, or check specific models for fine-grained control.
          </p>
          <ModelSelectionTable
            models={models}
            value={modelSelection}
            onChange={setModelSelection}
          />
        </div>

        <div className="flex gap-2 mt-6">
          <Button onClick={handleSaveModelSelection} disabled={isCreating}>
            {isCreating ? 'Saving...' : 'Save Changes'}
          </Button>
          <Button
            variant="secondary"
            onClick={() => setShowEditModelsModal(false)}
          >
            Cancel
          </Button>
        </div>
      </Modal>
    </div>
  )
}
