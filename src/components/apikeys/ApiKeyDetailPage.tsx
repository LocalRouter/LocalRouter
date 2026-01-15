import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Badge from '../ui/Badge'
import Button from '../ui/Button'
import { ChatInterface } from '../visualization/ChatInterface'
import OpenAI from 'openai'

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
  model_id: string
  provider_instance: string
}

export default function ApiKeyDetailPage({ keyId }: ApiKeyDetailPageProps) {
  const [apiKey, setApiKey] = useState<ApiKey | null>(null)
  const [keyValue, setKeyValue] = useState<string>('')
  const [showKey, setShowKey] = useState(false)
  const [loading, setLoading] = useState(true)
  const [activeTab, setActiveTab] = useState<'settings' | 'chat'>('chat')
  const [isSaving, setIsSaving] = useState(false)

  // Form state
  const [name, setName] = useState('')
  const [enabled, setEnabled] = useState(true)

  // Chat state
  const [chatClient, setChatClient] = useState<OpenAI | null>(null)
  const [_availableModels, setAvailableModels] = useState<Model[]>([])

  useEffect(() => {
    loadApiKeyData()
    loadServerConfig()
  }, [keyId])

  const loadApiKeyData = async () => {
    setLoading(true)
    try {
      const [keys, value, models] = await Promise.all([
        invoke<ApiKey[]>('list_api_keys'),
        invoke<string>('get_api_key_value', { id: keyId }),
        invoke<Model[]>('list_all_models').catch(() => []),
      ])

      const key = keys.find((k) => k.id === keyId)
      if (key) {
        setApiKey(key)
        setName(key.name)
        setEnabled(key.enabled)
      }

      setKeyValue(value)
      setAvailableModels(models)
    } catch (error) {
      console.error('Failed to load API key data:', error)
    } finally {
      setLoading(false)
    }
  }

  const loadServerConfig = async () => {
    try {
      const serverConfig = await invoke<{ host: string; port: number }>('get_server_config')
      const value = await invoke<string>('get_api_key_value', { id: keyId })

      const newClient = new OpenAI({
        apiKey: value,
        baseURL: `http://${serverConfig.host}:${serverConfig.port}/v1`,
        dangerouslyAllowBrowser: true,
      })
      setChatClient(newClient)
    } catch (err) {
      console.error('Failed to load server config:', err)
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

  const handleCopyKey = async () => {
    try {
      await navigator.clipboard.writeText(keyValue)
      alert('API key copied to clipboard!')
    } catch (error) {
      console.error('Failed to copy:', error)
      alert('Failed to copy key to clipboard')
    }
  }

  const handleSendMessage = async (
    messages: Array<{ role: 'user' | 'assistant'; content: string }>,
    userMessage: string
  ) => {
    if (!chatClient) {
      throw new Error('Chat client not initialized')
    }

    // Use 'gpt-4' as a generic model identifier - routing will handle the actual model
    const stream = await chatClient.chat.completions.create({
      model: 'gpt-4',
      messages: [
        ...messages,
        {
          role: 'user',
          content: userMessage,
        },
      ],
      stream: true,
    })

    async function* generateChunks() {
      for await (const chunk of stream) {
        const content = chunk.choices[0]?.delta?.content || ''
        if (content) {
          yield content
        }
      }
    }

    return generateChunks()
  }

  const formatModelSelection = (selection: any): string => {
    if (!selection) return 'Not configured'
    if (selection.type === 'all') return 'All providers and models'

    if (selection.type === 'custom') {
      const providers = selection.all_provider_models || []
      const models = selection.individual_models || []

      const parts: string[] = []
      if (providers.length > 0) {
        parts.push(`${providers.length} provider(s)`)
      }
      if (models.length > 0) {
        parts.push(`${models.length} specific model(s)`)
      }

      return parts.length > 0 ? parts.join(', ') : 'No models selected'
    }

    return 'Unknown configuration'
  }

  if (loading || !apiKey) {
    return (
      <div className="bg-white rounded-lg shadow-sm p-6">
        <div className="text-center py-8 text-gray-500">Loading API key details...</div>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <Card>
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-2xl font-bold text-gray-900">{apiKey.name}</h2>
            <p className="text-sm text-gray-500 mt-1">
              ID: {apiKey.id.substring(0, 16)}... | Created: {new Date(apiKey.created_at).toLocaleDateString()}
            </p>
          </div>
          <div className="flex items-center gap-3">
            <Badge variant={apiKey.enabled ? 'success' : 'warning'}>
              {apiKey.enabled ? 'Enabled' : 'Disabled'}
            </Badge>
          </div>
        </div>
      </Card>

      {/* Tabs */}
      <div className="flex border-b border-gray-200">
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
        <button
          onClick={() => setActiveTab('settings')}
          className={`px-6 py-3 font-medium transition-colors ${
            activeTab === 'settings'
              ? 'border-b-2 border-blue-500 text-blue-600'
              : 'text-gray-600 hover:text-gray-900'
          }`}
        >
          Settings
        </button>
      </div>

      {/* Tab Content */}
      {activeTab === 'chat' && (
        <Card>
          <h3 className="text-lg font-semibold text-gray-900 mb-4">Chat</h3>
          {!apiKey.enabled && (
            <div className="mb-4 p-3 bg-yellow-50 border border-yellow-200 rounded-lg text-sm text-yellow-700">
              This API key is disabled. Enable it in Settings to use chat.
            </div>
          )}
          <div className="mb-4 p-3 bg-blue-50 border border-blue-200 rounded-lg text-sm text-blue-700">
            <p>
              <strong>Routing:</strong> {formatModelSelection(apiKey.model_selection)}
            </p>
            <p className="mt-1 text-xs">
              This API key will route requests according to its model selection configuration.
            </p>
          </div>
          {chatClient ? (
            <ChatInterface
              onSendMessage={handleSendMessage}
              placeholder={`Chat using ${apiKey.name}...`}
              disabled={!apiKey.enabled}
            />
          ) : (
            <div className="bg-yellow-50 border border-yellow-200 rounded-lg p-4">
              <p className="text-yellow-900 text-sm">
                <strong>Note:</strong> To use chat, make sure the server is running.
              </p>
            </div>
          )}
        </Card>
      )}

      {activeTab === 'settings' && (
        <div className="space-y-6">
          <Card>
            <h3 className="text-lg font-semibold text-gray-900 mb-4">API Key Value</h3>
            <div className="space-y-3">
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

          <Card>
            <h3 className="text-lg font-semibold text-gray-900 mb-4">Model Selection</h3>
            <div className="p-3 bg-gray-50 border border-gray-200 rounded-lg">
              <p className="text-sm text-gray-700 font-medium mb-2">Current Configuration:</p>
              <p className="text-sm text-gray-600">{formatModelSelection(apiKey.model_selection)}</p>
            </div>
            <p className="text-xs text-gray-500 mt-3">
              To modify model selection, use the main API Keys page.
            </p>
          </Card>
        </div>
      )}
    </div>
  )
}
