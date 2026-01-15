import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'
import Modal from '../ui/Modal'
import Input from '../ui/Input'

interface ProviderInstance {
  instance_name: string
  provider_type: string
  enabled: boolean
  created_at: string
}

interface ProviderHealth {
  status: string
  latency_ms?: number
}

interface ServerConfig {
  host: string
  port: number
  enable_cors: boolean
}

export default function ProvidersTab() {
  const [ollamaEnabled, setOllamaEnabled] = useState(false)
  const [ollamaStatus, setOllamaStatus] = useState<'Not Configured' | 'Enabled' | 'Disabled'>('Not Configured')
  const [ollamaHealth, setOllamaHealth] = useState<ProviderHealth | null>(null)
  const [ollamaModelsCount, setOllamaModelsCount] = useState<number>(0)

  const [openaiProviders, setOpenaiProviders] = useState<ProviderInstance[]>([])
  const [providersHealth, setProvidersHealth] = useState<Record<string, ProviderHealth>>({})
  const [loading, setLoading] = useState(true)

  const [serverConfig, setServerConfig] = useState<ServerConfig | null>(null)
  const [serverLoading, setServerLoading] = useState(true)

  const [showAddProviderModal, setShowAddProviderModal] = useState(false)
  const [showEditServerModal, setShowEditServerModal] = useState(false)

  // Add provider form state
  const [providerName, setProviderName] = useState('')
  const [providerUrl, setProviderUrl] = useState('')
  const [providerApiKey, setProviderApiKey] = useState('')

  // Edit server form state
  const [editHost, setEditHost] = useState('')
  const [editPort, setEditPort] = useState(3625)

  useEffect(() => {
    loadProviders()
    loadServerConfig()
  }, [])

  const loadProviders = async () => {
    setLoading(true)
    try {
      const instances = await invoke<ProviderInstance[]>('list_provider_instances')
      const health = await invoke<Record<string, ProviderHealth>>('get_providers_health')
      setProvidersHealth(health)

      // Load Ollama
      const ollamaInstance = instances.find((i: ProviderInstance) => i.provider_type === 'ollama')
      if (ollamaInstance) {
        setOllamaEnabled(ollamaInstance.enabled)
        setOllamaStatus(ollamaInstance.enabled ? 'Enabled' : 'Disabled')

        if (ollamaInstance.enabled) {
          const ollamaHealthData = health[ollamaInstance.instance_name]
          setOllamaHealth(ollamaHealthData || null)

          try {
            const models = await invoke<any[]>('list_provider_models', {
              instanceName: ollamaInstance.instance_name,
            })
            setOllamaModelsCount(models.length)
          } catch (error) {
            console.error('Failed to load Ollama models:', error)
          }
        } else {
          setOllamaHealth(null)
          setOllamaModelsCount(0)
        }
      } else {
        setOllamaEnabled(false)
        setOllamaStatus('Not Configured')
        setOllamaHealth(null)
        setOllamaModelsCount(0)
      }

      // Load OpenAI-compatible providers
      const openaiCompatible = instances.filter((i: ProviderInstance) => i.provider_type === 'openai_compatible')
      setOpenaiProviders(openaiCompatible)
    } catch (error) {
      console.error('Failed to load providers:', error)
    } finally {
      setLoading(false)
    }
  }

  const loadServerConfig = async () => {
    setServerLoading(true)
    try {
      const config = await invoke<ServerConfig>('get_server_config')
      setServerConfig(config)
    } catch (error) {
      console.error('Failed to load server config:', error)
    } finally {
      setServerLoading(false)
    }
  }

  const handleToggleOllama = async (checked: boolean) => {
    try {
      const instances = await invoke<ProviderInstance[]>('list_provider_instances')
      const ollamaInstance = instances.find((i: ProviderInstance) => i.provider_type === 'ollama')

      if (!ollamaInstance && checked) {
        await invoke('create_provider_instance', {
          instanceName: 'ollama',
          providerType: 'ollama',
          config: {},
        })
      } else if (ollamaInstance) {
        await invoke('set_provider_enabled', {
          instanceName: ollamaInstance.instance_name,
          enabled: checked,
        })
      }

      await loadProviders()
    } catch (error) {
      console.error('Failed to toggle Ollama:', error)
      alert(`Error toggling Ollama: ${error}`)
      setOllamaEnabled(!checked) // Revert
    }
  }

  const handleAddProvider = async (e: React.FormEvent) => {
    e.preventDefault()

    if (!providerName || !providerUrl) {
      alert('Instance name and base URL are required')
      return
    }

    try {
      const config: any = { base_url: providerUrl }
      if (providerApiKey) {
        config.api_key = providerApiKey
      }

      await invoke('create_provider_instance', {
        instanceName: providerName,
        providerType: 'openai_compatible',
        config,
      })

      setShowAddProviderModal(false)
      setProviderName('')
      setProviderUrl('')
      setProviderApiKey('')
      await loadProviders()
      alert(`Provider "${providerName}" added successfully!`)
    } catch (error) {
      console.error('Failed to add provider:', error)
      alert(`Error adding provider: ${error}`)
    }
  }

  const handleToggleProviderEnabled = async (instanceName: string, enabled: boolean) => {
    try {
      await invoke('set_provider_enabled', { instanceName, enabled })
      await loadProviders()
    } catch (error) {
      console.error('Failed to toggle provider:', error)
      alert(`Error toggling provider: ${error}`)
    }
  }

  const handleRemoveProvider = async (instanceName: string) => {
    if (!confirm(`Remove provider "${instanceName}"? This action cannot be undone.`)) {
      return
    }

    try {
      await invoke('remove_provider_instance', { instanceName })
      await loadProviders()
      alert(`Provider "${instanceName}" removed successfully!`)
    } catch (error) {
      console.error('Failed to remove provider:', error)
      alert(`Error removing provider: ${error}`)
    }
  }

  const handleSaveServerConfig = async (e: React.FormEvent) => {
    e.preventDefault()

    if (!editHost || !editPort || editPort < 1 || editPort > 65535) {
      alert('Please provide valid host and port values')
      return
    }

    try {
      await invoke('update_server_config', { host: editHost, port: editPort })
      alert('Server configuration saved! Click "Restart Server" for changes to take effect.')
      setShowEditServerModal(false)
      await loadServerConfig()
    } catch (error) {
      console.error('Failed to save server config:', error)
      alert(`Error saving config: ${error}`)
    }
  }

  const handleRestartServer = async () => {
    if (!confirm('Restart the server? Active connections will be interrupted.')) {
      return
    }

    try {
      await invoke('restart_server')
      alert('Server restart requested. The server will restart momentarily.')
    } catch (error) {
      console.error('Failed to restart server:', error)
      alert(`Error restarting server: ${error}`)
    }
  }

  const handleOpenEditServerModal = () => {
    if (serverConfig) {
      setEditHost(serverConfig.host)
      setEditPort(serverConfig.port)
      setShowEditServerModal(true)
    }
  }

  return (
    <div className="space-y-6">
      {/* Ollama Provider */}
      <Card>
        <div className="flex justify-between items-center mb-4">
          <div>
            <h2 className="text-xl font-bold text-gray-900">Ollama (Local)</h2>
            <p className="text-sm text-gray-500 mt-1">
              Local Ollama instance at http://localhost:11434
            </p>
          </div>
          <div className="flex items-center gap-4">
            <Badge variant={ollamaStatus === 'Enabled' ? 'success' : 'warning'}>
              {ollamaStatus}
            </Badge>
            <label className="flex items-center cursor-pointer">
              <input
                type="checkbox"
                checked={ollamaEnabled}
                onChange={(e) => handleToggleOllama(e.target.checked)}
                className="mr-2 w-5 h-5 cursor-pointer"
              />
              <span className="text-sm font-medium">Enabled</span>
            </label>
          </div>
        </div>

        {ollamaHealth && (
          <div className="mt-4 p-4 bg-gray-50 rounded-md">
            <div className="grid grid-cols-3 gap-4">
              <div>
                <label className="text-xs font-medium text-gray-500 uppercase">Status</label>
                <p className="text-sm text-gray-900 mt-1">{ollamaHealth.status}</p>
              </div>
              <div>
                <label className="text-xs font-medium text-gray-500 uppercase">Latency</label>
                <p className="text-sm text-gray-900 mt-1">
                  {ollamaHealth.latency_ms ? `${ollamaHealth.latency_ms}ms` : '-'}
                </p>
              </div>
              <div>
                <label className="text-xs font-medium text-gray-500 uppercase">Models</label>
                <p className="text-sm text-gray-900 mt-1">{ollamaModelsCount}</p>
              </div>
            </div>
          </div>
        )}
      </Card>

      {/* OpenAI-Compatible Providers */}
      <Card>
        <div className="flex justify-between items-center mb-4">
          <div>
            <h2 className="text-xl font-bold text-gray-900">OpenAI-Compatible Providers</h2>
            <p className="text-sm text-gray-500 mt-1">
              Connect to LocalAI, LM Studio, vLLM, or any OpenAI-compatible API
            </p>
          </div>
          <Button onClick={() => setShowAddProviderModal(true)} className="px-3 py-2 text-xs">
            + Add Provider
          </Button>
        </div>

        {loading ? (
          <div className="text-center py-8 text-gray-500">Loading providers...</div>
        ) : openaiProviders.length === 0 ? (
          <div className="text-center py-12 text-gray-500">
            <p>No OpenAI-compatible providers configured. Click "Add Provider" to get started.</p>
          </div>
        ) : (
          <ul className="space-y-3">
            {openaiProviders.map((provider) => {
              const health = providersHealth[provider.instance_name]
              const healthStatus = health?.status || 'Unknown'
              const healthVariant =
                healthStatus === 'Healthy'
                  ? 'success'
                  : healthStatus === 'Degraded'
                  ? 'warning'
                  : 'error'

              return (
                <li
                  key={provider.instance_name}
                  className="bg-gray-50 border border-gray-200 rounded-lg p-4 flex justify-between items-center"
                >
                  <div>
                    <h3 className="text-base font-semibold text-gray-900">
                      {provider.instance_name}
                    </h3>
                    <p className="text-xs text-gray-500 mt-1">
                      Created: {new Date(provider.created_at).toLocaleDateString()}
                    </p>
                  </div>
                  <div className="flex gap-2 items-center">
                    <Badge variant={healthVariant}>{healthStatus}</Badge>
                    <Badge variant={provider.enabled ? 'success' : 'warning'}>
                      {provider.enabled ? 'Enabled' : 'Disabled'}
                    </Badge>
                    <Button
                      variant="secondary"
                      onClick={() =>
                        handleToggleProviderEnabled(provider.instance_name, !provider.enabled)
                      }
                      className="px-3 py-1.5 text-xs"
                    >
                      {provider.enabled ? 'Disable' : 'Enable'}
                    </Button>
                    <Button
                      variant="danger"
                      onClick={() => handleRemoveProvider(provider.instance_name)}
                      className="px-3 py-1.5 text-xs"
                    >
                      Remove
                    </Button>
                  </div>
                </li>
              )
            })}
          </ul>
        )}
      </Card>

      {/* Server Configuration */}
      <Card>
        <div className="flex justify-between items-center mb-4">
          <div>
            <h2 className="text-xl font-bold text-gray-900">Server Configuration</h2>
            <p className="text-sm text-gray-500 mt-1">
              API Gateway listens on this address. Default: localhost:3625
            </p>
          </div>
          <div className="flex gap-2">
            <Button onClick={handleOpenEditServerModal} className="px-3 py-2 text-xs">
              Edit
            </Button>
            <Button
              onClick={handleRestartServer}
              className="px-3 py-2 text-xs bg-yellow-500 hover:bg-yellow-600"
            >
              Restart Server
            </Button>
          </div>
        </div>

        {serverLoading ? (
          <div className="text-center py-8 text-gray-500">Loading...</div>
        ) : serverConfig ? (
          <div className="p-4 bg-gray-50 rounded-md">
            <div className="grid grid-cols-3 gap-4">
              <div>
                <label className="text-xs font-medium text-gray-500 uppercase">
                  Host / Interface
                </label>
                <p className="text-base text-gray-900 mt-1 font-mono">{serverConfig.host}</p>
                <p className="text-xs text-gray-500 mt-1">
                  127.0.0.1 = localhost only, 0.0.0.0 = all interfaces
                </p>
              </div>
              <div>
                <label className="text-xs font-medium text-gray-500 uppercase">Port</label>
                <p className="text-base text-gray-900 mt-1 font-mono">{serverConfig.port}</p>
                <p className="text-xs text-gray-500 mt-1">
                  OpenAI-compatible endpoint: http://{serverConfig.host}:{serverConfig.port}/v1
                </p>
              </div>
              <div>
                <label className="text-xs font-medium text-gray-500 uppercase">CORS</label>
                <p className="text-base text-gray-900 mt-1">
                  {serverConfig.enable_cors ? 'Enabled' : 'Disabled'}
                </p>
                <p className="text-xs text-gray-500 mt-1">Cross-origin requests</p>
              </div>
            </div>
          </div>
        ) : null}
      </Card>

      {/* Add Provider Modal */}
      <Modal
        isOpen={showAddProviderModal}
        onClose={() => setShowAddProviderModal(false)}
        title="Add OpenAI-Compatible Provider"
      >
        <form onSubmit={handleAddProvider}>
          <Input
            label="Instance Name *"
            placeholder="e.g., my-localai, lm-studio"
            value={providerName}
            onChange={(e) => setProviderName(e.target.value)}
            required
            helperText="Unique name to identify this provider instance"
          />

          <Input
            label="Base URL *"
            placeholder="http://localhost:8080/v1"
            value={providerUrl}
            onChange={(e) => setProviderUrl(e.target.value)}
            required
            helperText="API endpoint (must include /v1 suffix for OpenAI compatibility)"
          />

          <Input
            label="API Key (Optional)"
            type="password"
            placeholder="Leave empty if not required"
            value={providerApiKey}
            onChange={(e) => setProviderApiKey(e.target.value)}
            helperText="Some services like LocalAI don't require an API key"
          />

          <div className="flex gap-2 mt-6">
            <Button type="submit">Add Provider</Button>
            <Button
              type="button"
              variant="secondary"
              onClick={() => setShowAddProviderModal(false)}
            >
              Cancel
            </Button>
          </div>
        </form>
      </Modal>

      {/* Edit Server Config Modal */}
      <Modal
        isOpen={showEditServerModal}
        onClose={() => setShowEditServerModal(false)}
        title="Edit Server Configuration"
      >
        <form onSubmit={handleSaveServerConfig}>
          <Input
            label="Host / Interface *"
            placeholder="127.0.0.1"
            value={editHost}
            onChange={(e) => setEditHost(e.target.value)}
            required
            helperText="127.0.0.1 = localhost only (recommended for security) | 0.0.0.0 = all network interfaces (allows external access)"
          />

          <Input
            label="Port *"
            type="number"
            placeholder="3625"
            min={1}
            max={65535}
            value={editPort}
            onChange={(e) => setEditPort(parseInt(e.target.value))}
            required
            helperText="Port number between 1-65535. Default: 3625"
          />

          <div className="bg-yellow-50 border border-yellow-400 rounded-md p-4 my-4">
            <p className="text-sm text-yellow-900">
              ⚠️ <strong>Note:</strong> You must restart the server for these changes to take
              effect. Click "Restart Server" after saving.
            </p>
          </div>

          <div className="flex gap-2 mt-6">
            <Button type="submit">Save Changes</Button>
            <Button
              type="button"
              variant="secondary"
              onClick={() => setShowEditServerModal(false)}
            >
              Cancel
            </Button>
          </div>
        </form>
      </Modal>
    </div>
  )
}
