import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'
import Input from '../ui/Input'
import Select from '../ui/Select'
import DetailPageLayout from '../layouts/DetailPageLayout'
import { MetricsChart } from '../charts/MetricsChart'
import { McpMetricsChart } from '../charts/McpMetricsChart'
import { McpMethodBreakdown } from '../charts/McpMethodBreakdown'
import { useMetricsSubscription } from '../../hooks/useMetricsSubscription'
import ModelSelectionTable, { Model, ModelSelectionValue } from '../ModelSelectionTable'
import PrioritizedModelList from '../PrioritizedModelList'
import ForcedModelSelector from '../ForcedModelSelector'

// Simple icon components
const EyeIcon = () => (
  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
  </svg>
)

const EyeOffIcon = () => (
  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
  </svg>
)

const CopyIcon = () => (
  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" />
  </svg>
)

const CheckIcon = () => (
  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
  </svg>
)

interface ClientDetailPageProps {
  clientId: string
  initialTab?: string
  initialRoutingMode?: 'forced' | 'multi' | 'prioritized'
  onBack: () => void
}

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  allowed_llm_providers: string[]
  allowed_mcp_servers: string[]
  created_at: string
  last_used: string | null
}

interface McpServer {
  id: string
  name: string
  enabled: boolean
  url?: string
}

export default function ClientDetailPage({ clientId, initialTab, initialRoutingMode, onBack }: ClientDetailPageProps) {
  const refreshKey = useMetricsSubscription()
  const [client, setClient] = useState<Client | null>(null)
  const [mcpServers, setMcpServers] = useState<McpServer[]>([])
  const [models, setModels] = useState<Model[]>([])
  const [loading, setLoading] = useState(true)
  const [activeTab, setActiveTab] = useState<string>(initialTab || 'metrics')
  const [isSaving, setIsSaving] = useState(false)

  // Configuration state
  const [name, setName] = useState('')
  const [enabled, setEnabled] = useState(true)

  // Models tab state
  const [routingMode, setRoutingMode] = useState<'forced' | 'multi' | 'prioritized'>(initialRoutingMode || 'multi')
  const [forcedModel, setForcedModel] = useState<[string, string] | null>(null)
  const [modelSelection, setModelSelection] = useState<ModelSelectionValue | null>(null)
  const [prioritizedModels, setPrioritizedModels] = useState<[string, string][]>([])

  // Secret visibility state
  const [showClientSecret, setShowClientSecret] = useState(false)
  const [copiedField, setCopiedField] = useState<string | null>(null)

  // MCP tab state
  const [selectedMcpAuthType, setSelectedMcpAuthType] = useState<'bearer' | 'stdio' | 'oauth'>('bearer')

  useEffect(() => {
    loadClientData()
    loadMcpServers()
    loadModels()
  }, [clientId])

  const loadClientData = async () => {
    setLoading(true)
    try {
      const clients = await invoke<Client[]>('list_clients')
      const clientData = clients.find((c) => c.client_id === clientId)

      if (clientData) {
        setClient(clientData)
        setName(clientData.name)
        setEnabled(clientData.enabled)
      }
    } catch (error) {
      console.error('Failed to load client data:', error)
    } finally {
      setLoading(false)
    }
  }

  const loadMcpServers = async () => {
    try {
      const serverList = await invoke<McpServer[]>('list_mcp_servers')
      setMcpServers(serverList)
    } catch (error) {
      console.error('Failed to load MCP servers:', error)
    }
  }

  const loadModels = async () => {
    try {
      const modelList = await invoke<Array<{ id: string; provider: string }>>('list_all_models')
      setModels(modelList)
    } catch (error) {
      console.error('Failed to load models:', error)
    }
  }

  const handleSaveConfiguration = async () => {
    if (!client) return

    setIsSaving(true)
    try {
      await invoke('update_client_name', {
        clientId: client.client_id,
        name,
      })

      await invoke('toggle_client_enabled', {
        clientId: client.client_id,
        enabled,
      })

      alert('Configuration saved successfully')
      await loadClientData()
    } catch (error) {
      console.error('Failed to save configuration:', error)
      alert(`Error saving configuration: ${error}`)
    } finally {
      setIsSaving(false)
    }
  }

  const handleToggleMcpServer = async (serverId: string, hasAccess: boolean) => {
    if (!client) return

    try {
      if (hasAccess) {
        await invoke('remove_client_mcp_server', {
          clientId: client.client_id,
          serverId,
        })
      } else {
        await invoke('add_client_mcp_server', {
          clientId: client.client_id,
          serverId,
        })
      }

      await loadClientData()
    } catch (error) {
      console.error('Failed to toggle MCP server:', error)
      alert(`Error updating MCP server access: ${error}`)
    }
  }

  const copyToClipboard = async (text: string, label: string) => {
    try {
      await navigator.clipboard.writeText(text)
      setCopiedField(label)
      setTimeout(() => setCopiedField(null), 2000)
    } catch (error) {
      console.error('Failed to copy to clipboard:', error)
      alert('Failed to copy to clipboard')
    }
  }

  // Auto-save handlers for model routing
  const handleRoutingModeChange = async (mode: 'forced' | 'multi' | 'prioritized') => {
    setRoutingMode(mode)
    // TODO: Save routing mode to backend
    try {
      // await invoke('update_client_routing_mode', { clientId: client?.client_id, mode })
      console.log('Routing mode changed to:', mode)
    } catch (error) {
      console.error('Failed to save routing mode:', error)
    }
  }

  const handleForcedModelChange = async (model: [string, string] | null) => {
    setForcedModel(model)
    // TODO: Save forced model to backend
    try {
      // await invoke('update_client_forced_model', { clientId: client?.client_id, model })
      console.log('Forced model changed:', model)
    } catch (error) {
      console.error('Failed to save forced model:', error)
    }
  }

  const handleMultiModelChange = async (selection: ModelSelectionValue) => {
    setModelSelection(selection)
    // TODO: Save multi-model selection to backend
    try {
      // await invoke('update_client_model_selection', { clientId: client?.client_id, selection })
      console.log('Model selection changed:', selection)
    } catch (error) {
      console.error('Failed to save model selection:', error)
    }
  }

  const handlePrioritizedModelsChange = async (models: [string, string][]) => {
    setPrioritizedModels(models)
    // TODO: Save prioritized models to backend
    try {
      // await invoke('update_client_prioritized_models', { clientId: client?.client_id, models })
      console.log('Prioritized models changed:', models)
    } catch (error) {
      console.error('Failed to save prioritized models:', error)
    }
  }

  const formatDate = (dateStr: string | null) => {
    if (!dateStr) return 'Never'
    return new Date(dateStr).toLocaleString()
  }

  const maskSecret = (secret: string) => {
    if (secret.length <= 8) return secret
    return `${secret.slice(0, 6)}...${secret.slice(-4)}`
  }

  const getApiUrl = () => {
    return 'http://localhost:3625'
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-gray-400">Loading...</div>
      </div>
    )
  }

  if (!client) {
    return (
      <div className="flex flex-col items-center justify-center h-64">
        <div className="text-gray-400 mb-4">Client not found</div>
        <Button onClick={onBack}>Go Back</Button>
      </div>
    )
  }

  // Define tab content
  const tabs = [
    {
      id: 'metrics',
      label: 'Metrics',
      content: (
        <div className="space-y-6">
          <Card>
            <h3 className="text-lg font-semibold mb-4">Request Metrics</h3>
            <div className="grid grid-cols-2 gap-4">
              <MetricsChart
                scope="api_key"
                scopeId={client.client_id}
                timeRange="day"
                metricType="requests"
                title="Requests (Last 24h)"
                refreshTrigger={refreshKey}
              />
              <MetricsChart
                scope="api_key"
                scopeId={client.client_id}
                timeRange="day"
                metricType="tokens"
                title="Tokens (Last 24h)"
                refreshTrigger={refreshKey}
              />
            </div>
          </Card>

          <Card>
            <h3 className="text-lg font-semibold mb-4">Cost & Performance</h3>
            <div className="grid grid-cols-2 gap-4">
              <MetricsChart
                scope="api_key"
                scopeId={client.client_id}
                timeRange="day"
                metricType="cost"
                title="Cost (Last 24h)"
                refreshTrigger={refreshKey}
              />
              <MetricsChart
                scope="api_key"
                scopeId={client.client_id}
                timeRange="day"
                metricType="latency"
                title="Latency (Last 24h)"
                refreshTrigger={refreshKey}
              />
            </div>
          </Card>

          <Card>
            <h3 className="text-lg font-semibold mb-4">Success Rate</h3>
            <MetricsChart
              scope="api_key"
              scopeId={client.client_id}
              timeRange="day"
              metricType="successrate"
              title="Success Rate (Last 24h)"
              refreshTrigger={refreshKey}
            />
          </Card>
        </div>
      ),
    },
    {
      id: 'mcp-metrics',
      label: 'MCP Metrics',
      content: (
        <div className="space-y-6">
          <Card>
            <h3 className="text-lg font-semibold mb-4">MCP Method Breakdown</h3>
            <McpMethodBreakdown
              scope={`client:${client.client_id}`}
              timeRange="day"
              title="MCP Methods Used (Last 24h)"
              refreshTrigger={refreshKey}
            />
          </Card>

          <Card>
            <h3 className="text-lg font-semibold mb-4">MCP Request Metrics</h3>
            <div className="grid grid-cols-2 gap-4">
              <McpMetricsChart
                scope="client"
                scopeId={client.client_id}
                timeRange="day"
                metricType="requests"
                title="MCP Requests (Last 24h)"
                refreshTrigger={refreshKey}
              />
              <McpMetricsChart
                scope="client"
                scopeId={client.client_id}
                timeRange="day"
                metricType="latency"
                title="MCP Latency (Last 24h)"
                refreshTrigger={refreshKey}
              />
            </div>
          </Card>

          <Card>
            <h3 className="text-lg font-semibold mb-4">MCP Success Rate</h3>
            <McpMetricsChart
              scope="client"
              scopeId={client.client_id}
              timeRange="day"
              metricType="successrate"
              title="MCP Success Rate (Last 24h)"
              refreshTrigger={refreshKey}
            />
          </Card>
        </div>
      ),
    },
    {
      id: 'configuration',
      label: 'Configuration',
      content: (
        <div className="space-y-6">
          <Card>
            <div className="p-6 space-y-4">
              <h3 className="text-lg font-semibold">Client Settings</h3>

              <div>
                <label className="block text-sm font-medium mb-2">Client Name</label>
                <Input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="My Application"
                />
              </div>

              <div>
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={enabled}
                    onChange={(e) => setEnabled(e.target.checked)}
                    className="w-4 h-4"
                  />
                  <span className="font-medium">Enabled</span>
                </label>
                <p className="text-sm text-gray-400 mt-1">
                  Disabled clients cannot authenticate or make API requests
                </p>
              </div>

              <div className="flex justify-end">
                <Button onClick={handleSaveConfiguration} disabled={isSaving}>
                  {isSaving ? 'Saving...' : 'Save Settings'}
                </Button>
              </div>
            </div>
          </Card>

          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold mb-4">Client Information</h3>
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <p className="text-sm text-gray-400">Client ID</p>
                  <p className="font-mono text-sm">{client.client_id}</p>
                </div>
                <div>
                  <p className="text-sm text-gray-400">Created</p>
                  <p className="font-medium">{formatDate(client.created_at)}</p>
                </div>
                <div>
                  <p className="text-sm text-gray-400">Last Used</p>
                  <p className="font-medium">{formatDate(client.last_used)}</p>
                </div>
                <div>
                  <p className="text-sm text-gray-400">Status</p>
                  <Badge variant={client.enabled ? 'success' : 'error'}>
                    {client.enabled ? 'Enabled' : 'Disabled'}
                  </Badge>
                </div>
              </div>
            </div>
          </Card>
        </div>
      ),
    },
    {
      id: 'models',
      label: 'Models',
      content: (
        <div className="space-y-6">
          {/* Authentication Section - Show First */}
          <Card>
            <div className="p-6 space-y-4">
              <h3 className="text-lg font-semibold">API Authentication</h3>

              <div>
                <label className="block text-sm font-medium mb-2">API URL</label>
                <div className="flex gap-2">
                  <Input
                    value={getApiUrl()}
                    readOnly
                    className="flex-1 font-mono"
                  />
                  <Button
                    variant="secondary"
                    onClick={() => copyToClipboard(getApiUrl(), 'api_url')}
                  >
                    {copiedField === 'api_url' ? <CheckIcon /> : <CopyIcon />}
                  </Button>
                </div>
              </div>

              <div>
                <label className="block text-sm font-medium mb-2">Bearer Token (Client Secret)</label>
                <div className="flex gap-2">
                  <div className="relative flex-1">
                    <Input
                      type={showClientSecret ? 'text' : 'password'}
                      value={showClientSecret ? client.client_id : maskSecret(client.client_id)}
                      readOnly
                      className="font-mono pr-10"
                    />
                    <button
                      onClick={() => setShowClientSecret(!showClientSecret)}
                      className="absolute right-3 top-1/2 transform -translate-y-1/2 text-gray-400 hover:text-gray-600"
                    >
                      {showClientSecret ? <EyeOffIcon /> : <EyeIcon />}
                    </button>
                  </div>
                  <Button
                    variant="secondary"
                    onClick={() => copyToClipboard(client.client_id, 'bearer_token')}
                  >
                    {copiedField === 'bearer_token' ? <CheckIcon /> : <CopyIcon />}
                  </Button>
                </div>
                <p className="text-sm text-gray-400 mt-1">
                  Use this in the Authorization header: <code className="bg-gray-800 px-1">Authorization: Bearer {maskSecret(client.client_id)}</code>
                </p>
              </div>
            </div>
          </Card>

          {/* Model Routing Section */}
          <Card>
            <div className="p-6 space-y-4">
              <h3 className="text-lg font-semibold">Model Routing</h3>

              <div>
                <label className="block text-sm font-medium mb-2">Routing Mode</label>
                <Select
                  value={routingMode}
                  onChange={(e) => handleRoutingModeChange(e.target.value as 'forced' | 'multi' | 'prioritized')}
                >
                  <option value="forced">Forced Model - Always use a specific model</option>
                  <option value="multi">Multi-Model - Select from available models</option>
                  <option value="prioritized">Prioritized List - Fallback priority order</option>
                </Select>
              </div>

              {routingMode === 'forced' && (
                <div>
                  <label className="block text-sm font-medium mb-2">Select Forced Model</label>
                  <ForcedModelSelector
                    models={models}
                    selectedModel={forcedModel}
                    onChange={handleForcedModelChange}
                  />
                  <p className="text-sm text-gray-400 mt-1">
                    All requests will be routed to the selected model regardless of the requested model. Select only one model.
                  </p>
                </div>
              )}

              {routingMode === 'multi' && (
                <div>
                  <label className="block text-sm font-medium mb-2">Available Models</label>
                  <ModelSelectionTable
                    models={models}
                    value={modelSelection}
                    onChange={handleMultiModelChange}
                  />
                  <p className="text-sm text-gray-400 mt-1">
                    Select which models are available to this client
                  </p>
                </div>
              )}

              {routingMode === 'prioritized' && (
                <div>
                  <label className="block text-sm font-medium mb-2">Model Priority Order</label>
                  <PrioritizedModelList
                    models={models}
                    prioritizedModels={prioritizedModels}
                    onChange={handlePrioritizedModelsChange}
                  />
                </div>
              )}
            </div>
          </Card>
        </div>
      ),
    },
    {
      id: 'mcp',
      label: 'MCP',
      content: (
        <div className="space-y-6">
          {/* Authentication Instructions - Show First */}
          <Card>
            <div className="p-6 space-y-4">
              <h3 className="text-lg font-semibold">MCP Server Authentication</h3>

              <div>
                <label className="block text-sm font-medium mb-2">Authentication Type</label>
                <Select
                  value={selectedMcpAuthType}
                  onChange={(e) => setSelectedMcpAuthType(e.target.value as 'bearer' | 'stdio' | 'oauth')}
                >
                  <option value="bearer">Bearer Key (Direct HTTP)</option>
                  <option value="stdio">STDIO (via Supergateway)</option>
                  <option value="oauth">OAuth (Pre-registered)</option>
                </Select>
              </div>

              {/* Bearer Key Auth */}
              {selectedMcpAuthType === 'bearer' && (
                <div className="space-y-4">
                  <div>
                    <label className="block text-sm font-medium mb-2">Bearer Token</label>
                    <div className="flex gap-2">
                      <div className="relative flex-1">
                        <Input
                          type={showClientSecret ? 'text' : 'password'}
                          value={showClientSecret ? client.client_id : maskSecret(client.client_id)}
                          readOnly
                          className="font-mono pr-10"
                        />
                        <button
                          onClick={() => setShowClientSecret(!showClientSecret)}
                          className="absolute right-3 top-1/2 transform -translate-y-1/2 text-gray-400 hover:text-gray-600"
                        >
                          {showClientSecret ? <EyeOffIcon /> : <EyeIcon />}
                        </button>
                      </div>
                      <Button
                        variant="secondary"
                        onClick={() => copyToClipboard(client.client_id, 'mcp_bearer')}
                      >
                        {copiedField === 'mcp_bearer' ? <CheckIcon /> : <CopyIcon />}
                      </Button>
                    </div>
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2">Accessible MCP Server URLs</label>
                    {client.allowed_mcp_servers.length === 0 ? (
                      <p className="text-sm text-gray-400">No MCP servers granted access yet. Add servers below.</p>
                    ) : (
                      <div className="space-y-2">
                        {mcpServers
                          .filter(server => client.allowed_mcp_servers.includes(server.id))
                          .map(server => (
                            <div key={server.id} className="bg-gray-800 rounded p-3">
                              <div className="flex items-center justify-between mb-1">
                                <span className="font-medium text-sm">{server.name}</span>
                                {server.url && (
                                  <Button
                                    variant="secondary"
                                    onClick={() => copyToClipboard(server.url!, `mcp_url_${server.id}`)}
                                  >
                                    {copiedField === `mcp_url_${server.id}` ? <CheckIcon /> : <CopyIcon />}
                                  </Button>
                                )}
                              </div>
                              {server.url ? (
                                <code className="text-xs text-gray-300 font-mono">{server.url}</code>
                              ) : (
                                <span className="text-xs text-gray-500">No URL configured</span>
                              )}
                            </div>
                          ))}
                      </div>
                    )}
                  </div>
                </div>
              )}

              {/* STDIO Auth (Supergateway) */}
              {selectedMcpAuthType === 'stdio' && (
                <div className="space-y-4">
                  <div className="bg-gray-800 border border-gray-600 rounded p-4">
                    <h4 className="font-medium text-gray-100 mb-2">Supergateway Configuration</h4>
                    <p className="text-sm text-gray-300 mb-3">
                      Use the Anthropic Supergateway to connect MCP servers via STDIO transport.
                      Set the bearer token as an environment variable:
                    </p>
                    <div className="bg-gray-900 rounded p-3 font-mono text-sm">
                      <div className="flex gap-2 items-center">
                        <code className="flex-1 text-gray-100">
                          LOCALROUTER_BEARER_TOKEN={showClientSecret ? client.client_id : maskSecret(client.client_id)}
                        </code>
                        <Button
                          variant="secondary"
                          onClick={() => copyToClipboard(`LOCALROUTER_BEARER_TOKEN=${client.client_id}`, 'env_var')}
                        >
                          {copiedField === 'env_var' ? <CheckIcon /> : <CopyIcon />}
                        </Button>
                      </div>
                    </div>
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2">Bearer Token (for reference)</label>
                    <div className="flex gap-2">
                      <div className="relative flex-1">
                        <Input
                          type={showClientSecret ? 'text' : 'password'}
                          value={showClientSecret ? client.client_id : maskSecret(client.client_id)}
                          readOnly
                          className="font-mono pr-10"
                        />
                        <button
                          onClick={() => setShowClientSecret(!showClientSecret)}
                          className="absolute right-3 top-1/2 transform -translate-y-1/2 text-gray-400 hover:text-gray-600"
                        >
                          {showClientSecret ? <EyeOffIcon /> : <EyeIcon />}
                        </button>
                      </div>
                      <Button
                        variant="secondary"
                        onClick={() => copyToClipboard(client.client_id, 'stdio_bearer')}
                      >
                        {copiedField === 'stdio_bearer' ? <CheckIcon /> : <CopyIcon />}
                      </Button>
                    </div>
                  </div>
                </div>
              )}

              {/* OAuth Auth */}
              {selectedMcpAuthType === 'oauth' && (
                <div className="space-y-4">
                  <div>
                    <label className="block text-sm font-medium mb-2">OAuth Token Endpoint</label>
                    <div className="flex gap-2">
                      <Input
                        value={getApiUrl() + '/oauth/token'}
                        readOnly
                        className="flex-1 font-mono"
                      />
                      <Button
                        variant="secondary"
                        onClick={() => copyToClipboard(getApiUrl() + '/oauth/token', 'oauth_url')}
                      >
                        {copiedField === 'oauth_url' ? <CheckIcon /> : <CopyIcon />}
                      </Button>
                    </div>
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2">Client ID</label>
                    <div className="flex gap-2">
                      <Input
                        value={client.client_id}
                        readOnly
                        className="flex-1 font-mono"
                      />
                      <Button
                        variant="secondary"
                        onClick={() => copyToClipboard(client.client_id, 'oauth_client_id')}
                      >
                        {copiedField === 'oauth_client_id' ? <CheckIcon /> : <CopyIcon />}
                      </Button>
                    </div>
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2">Client Secret</label>
                    <div className="flex gap-2">
                      <div className="relative flex-1">
                        <Input
                          type={showClientSecret ? 'text' : 'password'}
                          value={showClientSecret ? client.client_id : maskSecret(client.client_id)}
                          readOnly
                          className="font-mono pr-10"
                        />
                        <button
                          onClick={() => setShowClientSecret(!showClientSecret)}
                          className="absolute right-3 top-1/2 transform -translate-y-1/2 text-gray-400 hover:text-gray-600"
                        >
                          {showClientSecret ? <EyeOffIcon /> : <EyeIcon />}
                        </button>
                      </div>
                      <Button
                        variant="secondary"
                        onClick={() => copyToClipboard(client.client_id, 'oauth_secret')}
                      >
                        {copiedField === 'oauth_secret' ? <CheckIcon /> : <CopyIcon />}
                      </Button>
                    </div>
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2">Accessible MCP Server URLs</label>
                    {client.allowed_mcp_servers.length === 0 ? (
                      <p className="text-sm text-gray-400">No MCP servers granted access yet. Add servers below.</p>
                    ) : (
                      <div className="space-y-2">
                        {mcpServers
                          .filter(server => client.allowed_mcp_servers.includes(server.id))
                          .map(server => (
                            <div key={server.id} className="bg-gray-800 rounded p-3">
                              <div className="flex items-center justify-between mb-1">
                                <span className="font-medium text-sm">{server.name}</span>
                                {server.url && (
                                  <Button
                                    variant="secondary"
                                    onClick={() => copyToClipboard(server.url!, `oauth_mcp_url_${server.id}`)}
                                  >
                                    {copiedField === `oauth_mcp_url_${server.id}` ? <CheckIcon /> : <CopyIcon />}
                                  </Button>
                                )}
                              </div>
                              {server.url ? (
                                <code className="text-xs text-gray-300 font-mono">{server.url}</code>
                              ) : (
                                <span className="text-xs text-gray-500">No URL configured</span>
                              )}
                            </div>
                          ))}
                      </div>
                    )}
                  </div>
                </div>
              )}
            </div>
          </Card>

          {/* MCP Server Access Control */}
          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold mb-4">MCP Server Access</h3>
              <p className="text-sm text-gray-400 mb-4">
                Select which MCP servers this client can access.
              </p>

              {mcpServers.length === 0 ? (
                <div className="p-4 bg-gray-800 rounded text-center">
                  <p className="text-gray-400 text-sm">No MCP servers configured</p>
                </div>
              ) : (
                <div className="space-y-2">
                  {mcpServers.map((server) => {
                    const hasAccess = client.allowed_mcp_servers.includes(server.id)
                    return (
                      <div
                        key={server.id}
                        className="flex items-center justify-between p-3 bg-gray-800 rounded"
                      >
                        <div className="flex items-center gap-3">
                          <span className="font-medium">{server.name}</span>
                          {!server.enabled && (
                            <Badge variant="secondary">Server Disabled</Badge>
                          )}
                        </div>
                        <Button
                          variant={hasAccess ? 'danger' : 'primary'}
                          onClick={() => handleToggleMcpServer(server.id, hasAccess)}
                          disabled={!server.enabled}
                        >
                          {hasAccess ? 'Remove Access' : 'Grant Access'}
                        </Button>
                      </div>
                    )
                  })}
                </div>
              )}
            </div>
          </Card>
        </div>
      ),
    },
  ]

  return (
    <DetailPageLayout
      title={client.name}
      badges={[
        {
          label: client.enabled ? 'Enabled' : 'Disabled',
          variant: client.enabled ? 'success' : 'error',
        },
      ]}
      actions={
        <Button variant="secondary" onClick={onBack}>
          Back
        </Button>
      }
      tabs={tabs}
      activeTab={activeTab}
      onTabChange={setActiveTab}
      loading={loading}
    />
  )
}
