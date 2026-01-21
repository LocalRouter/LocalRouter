import { useState, useEffect, useMemo } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'
import Input from '../ui/Input'
import Select from '../ui/Select'
import DetailPageLayout from '../layouts/DetailPageLayout'
import MetricsPanel from '../MetricsPanel'
import { useMetricsSubscription } from '../../hooks/useMetricsSubscription'
import ModelSelectionTable, { Model, ModelSelectionValue } from '../ModelSelectionTable'
import PrioritizedModelList from '../PrioritizedModelList'
import ForcedModelSelector from '../ForcedModelSelector'
import { ContextualChat } from '../chat/ContextualChat'
import FilteredAccessLogs from '../logs/FilteredAccessLogs'
import StrategyConfigEditor, { Strategy } from '../strategies/StrategyConfigEditor'

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
  strategy_id: string
  allowed_llm_providers: string[]
  allowed_mcp_servers: string[]
  mcp_deferred_loading: boolean
  created_at: string
  last_used: string | null
}

interface McpServer {
  id: string
  name: string
  enabled: boolean
  url?: string // Legacy field (deprecated)
  proxy_url: string // Individual proxy endpoint: /mcp/{server_id}
  gateway_url: string // Unified gateway: /
}

interface ServerTokenStats {
  server_id: string
  tool_count: number
  resource_count: number
  prompt_count: number
  estimated_tokens: number
}

interface McpTokenStats {
  server_stats: ServerTokenStats[]
  total_tokens: number
  deferred_tokens: number
  savings_tokens: number
  savings_percent: number
}

export default function ClientDetailPage({ clientId, initialTab, initialRoutingMode, onBack }: ClientDetailPageProps) {
  const refreshKey = useMetricsSubscription()
  const [client, setClient] = useState<Client | null>(null)
  const [mcpServers, setMcpServers] = useState<McpServer[]>([])
  const [models, setModels] = useState<Model[]>([])
  const [strategies, setStrategies] = useState<Strategy[]>([])
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
  const [tokenStats, setTokenStats] = useState<McpTokenStats | null>(null)
  const [loadingTokenStats, setLoadingTokenStats] = useState(false)
  const [deferredLoading, setDeferredLoading] = useState(false)

  useEffect(() => {
    loadClientData()
    loadMcpServers()
    loadModels()
    loadStrategies()
  }, [clientId])

  // Load token stats when MCP tab is active
  useEffect(() => {
    if (activeTab === 'mcp' && client && client.allowed_mcp_servers.length > 0) {
      loadTokenStats()
    }
  }, [activeTab, client?.allowed_mcp_servers.length])

  // Update active tab when prop changes (e.g., from system tray)
  useEffect(() => {
    if (initialTab && initialTab !== activeTab) {
      setActiveTab(initialTab)
    }
  }, [initialTab])

  // Update routing mode when prop changes (e.g., from system tray)
  useEffect(() => {
    if (initialRoutingMode && initialRoutingMode !== routingMode) {
      setRoutingMode(initialRoutingMode)
    }
  }, [initialRoutingMode])

  // Memoize context object to prevent re-renders
  // IMPORTANT: This must be before any early returns to comply with Rules of Hooks
  const chatContext = useMemo(() => ({
    type: 'api_key' as const,
    apiKeyId: client?.client_id || '',
    apiKeyName: client?.name || '',
    modelSelection: null,
  }), [client?.client_id, client?.name]);

  const loadClientData = async () => {
    setLoading(true)
    try {
      const clients = await invoke<Client[]>('list_clients')
      const clientData = clients.find((c) => c.client_id === clientId)

      if (clientData) {
        setClient(clientData)
        setName(clientData.name)
        setEnabled(clientData.enabled)
        setDeferredLoading(clientData.mcp_deferred_loading || false)
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

  const loadStrategies = async () => {
    try {
      const strategyList = await invoke<Strategy[]>('list_strategies')
      setStrategies(strategyList)
    } catch (error) {
      console.error('Failed to load strategies:', error)
    }
  }

  const loadTokenStats = async () => {
    if (!client) return

    setLoadingTokenStats(true)
    try {
      const stats = await invoke<McpTokenStats>('get_mcp_token_stats', {
        clientId: client.id
      })
      setTokenStats(stats)
    } catch (error) {
      console.error('Failed to load token stats:', error)
    } finally {
      setLoadingTokenStats(false)
    }
  }

  const handleToggleDeferredLoading = async () => {
    if (!client) return

    const newValue = !deferredLoading
    setDeferredLoading(newValue)

    try {
      await invoke('toggle_client_deferred_loading', {
        clientId: client.id,
        enabled: newValue
      })
      console.log('Deferred loading toggled:', newValue)
    } catch (error) {
      console.error('Failed to toggle deferred loading:', error)
      alert(`Error toggling deferred loading: ${error}`)
      // Revert on error
      setDeferredLoading(!newValue)
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
    try {
      await invoke('set_client_routing_strategy', { clientId: client?.client_id, strategy: mode })
      console.log('Routing mode changed to:', mode)
    } catch (error) {
      console.error('Failed to save routing mode:', error)
      alert(`Error saving routing mode: ${error}`)
    }
  }

  const handleForcedModelChange = async (model: [string, string] | null) => {
    setForcedModel(model)
    try {
      await invoke('set_client_forced_model', {
        clientId: client?.client_id,
        provider: model ? model[0] : null,
        model: model ? model[1] : null,
      })
      console.log('Forced model changed:', model)
    } catch (error) {
      console.error('Failed to save forced model:', error)
      alert(`Error saving forced model: ${error}`)
    }
  }

  const handleMultiModelChange = async (selection: ModelSelectionValue) => {
    setModelSelection(selection)
    try {
      await invoke('update_client_available_models', {
        clientId: client?.client_id,
        allProviderModels: selection?.all_provider_models || [],
        individualModels: selection?.individual_models || [],
      })
      console.log('Model selection changed:', selection)
    } catch (error) {
      console.error('Failed to save model selection:', error)
      alert(`Error saving model selection: ${error}`)
    }
  }

  const handlePrioritizedModelsChange = async (models: [string, string][]) => {
    setPrioritizedModels(models)
    try {
      await invoke('update_client_prioritized_models', {
        clientId: client?.client_id,
        prioritizedModels: models,
      })
      console.log('Prioritized models changed:', models)
    } catch (error) {
      console.error('Failed to save prioritized models:', error)
      alert(`Error saving prioritized models: ${error}`)
    }
  }

  const handleStrategyChange = async (strategyId: string) => {
    if (!client) return

    try {
      await invoke('assign_client_strategy', {
        client_id: client.id,
        strategy_id: strategyId,
      })
      await loadClientData()
      console.log('Strategy assigned:', strategyId)
    } catch (error) {
      console.error('Failed to assign strategy:', error)
      alert(`Error assigning strategy: ${error}`)
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
        <div className="text-gray-400 dark:text-gray-500">Loading...</div>
      </div>
    )
  }

  if (!client) {
    return (
      <div className="flex flex-col items-center justify-center h-64">
        <div className="text-gray-400 dark:text-gray-500 mb-4">Client not found</div>
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
          <MetricsPanel
            title="LLM Metrics"
            chartType="llm"
            metricOptions={[
              { id: 'requests', label: 'Requests' },
              { id: 'tokens', label: 'Tokens' },
              { id: 'cost', label: 'Cost' },
              { id: 'latency', label: 'Latency' },
              { id: 'successrate', label: 'Success' },
            ]}
            scope="api_key"
            scopeId={client.client_id}
            defaultMetric="requests"
            defaultTimeRange="day"
            refreshTrigger={refreshKey}
          />

          <MetricsPanel
            title="MCP Metrics"
            chartType="mcp-methods"
            metricOptions={[
              { id: 'requests', label: 'Requests' },
              { id: 'latency', label: 'Latency' },
              { id: 'successrate', label: 'Success' },
            ]}
            scope="client"
            scopeId={client.client_id}
            defaultMetric="requests"
            defaultTimeRange="day"
            refreshTrigger={refreshKey}
            showMethodBreakdown={true}
          />
        </div>
      ),
    },
    {
      id: 'configuration',
      label: 'Configuration',
      content: (
        <div className="space-y-6">
          {/* API Endpoints Summary */}
          <Card>
            <div className="p-6 space-y-4">
              <h3 className="text-lg font-semibold">API Endpoints</h3>

              {/* Bearer Token */}
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
                      className="absolute right-3 top-1/2 transform -translate-y-1/2 text-gray-400 dark:text-gray-500 hover:text-gray-600 dark:hover:text-gray-400"
                    >
                      {showClientSecret ? <EyeOffIcon /> : <EyeIcon />}
                    </button>
                  </div>
                  <Button
                    variant="secondary"
                    onClick={() => copyToClipboard(client.client_id, 'config_bearer')}
                  >
                    {copiedField === 'config_bearer' ? <CheckIcon /> : <CopyIcon />}
                  </Button>
                </div>
                <p className="text-sm text-gray-400 dark:text-gray-500 mt-1">
                  Use this in the Authorization header: <code className="bg-gray-800 dark:bg-gray-800 px-1">Authorization: Bearer {maskSecret(client.client_id)}</code>
                </p>
              </div>

              {/* LLM Endpoints */}
              {client.allowed_llm_providers.length > 0 && (
                <div>
                  <label className="block text-sm font-medium mb-2">LLM API Endpoints (OpenAI-Compatible)</label>
                  <div className="bg-gray-800 dark:bg-gray-800 rounded p-3 space-y-2">
                    <div className="flex items-center justify-between">
                      <code className="text-xs text-gray-300 dark:text-gray-400 font-mono">{getApiUrl()}/v1/chat/completions</code>
                      <Button
                        variant="secondary"
                        onClick={() => copyToClipboard(`${getApiUrl()}/v1/chat/completions`, 'llm_chat')}
                      >
                        {copiedField === 'llm_chat' ? <CheckIcon /> : <CopyIcon />}
                      </Button>
                    </div>
                    <div className="flex items-center justify-between">
                      <code className="text-xs text-gray-300 dark:text-gray-400 font-mono">{getApiUrl()}/v1/completions</code>
                      <Button
                        variant="secondary"
                        onClick={() => copyToClipboard(`${getApiUrl()}/v1/completions`, 'llm_completions')}
                      >
                        {copiedField === 'llm_completions' ? <CheckIcon /> : <CopyIcon />}
                      </Button>
                    </div>
                    <div className="flex items-center justify-between">
                      <code className="text-xs text-gray-300 dark:text-gray-400 font-mono">{getApiUrl()}/v1/models</code>
                      <Button
                        variant="secondary"
                        onClick={() => copyToClipboard(`${getApiUrl()}/v1/models`, 'llm_models')}
                      >
                        {copiedField === 'llm_models' ? <CheckIcon /> : <CopyIcon />}
                      </Button>
                    </div>
                  </div>
                </div>
              )}

              {/* MCP Endpoints */}
              {client.allowed_mcp_servers.length > 0 && mcpServers.length > 0 && (
                <div>
                  <label className="block text-sm font-medium mb-2">MCP Endpoints</label>
                  <div className="space-y-2">
                    {/* Unified Gateway */}
                    {mcpServers[0].gateway_url && (
                      <div className="bg-blue-900/20 dark:bg-blue-900/30 border border-blue-700 dark:border-blue-600 rounded p-3">
                        <div className="flex items-center justify-between mb-1">
                          <span className="text-xs font-medium text-blue-300 dark:text-blue-400">Unified Gateway (All MCP Servers)</span>
                          <Button
                            variant="secondary"
                            onClick={() => copyToClipboard(mcpServers[0].gateway_url, 'config_mcp_gateway')}
                          >
                            {copiedField === 'config_mcp_gateway' ? <CheckIcon /> : <CopyIcon />}
                          </Button>
                        </div>
                        <code className="text-xs text-blue-200 dark:text-blue-300 font-mono">{mcpServers[0].gateway_url}</code>
                      </div>
                    )}

                    {/* Individual Proxies */}
                    <details className="bg-gray-800 dark:bg-gray-800 rounded p-3">
                      <summary className="text-xs font-medium text-gray-400 dark:text-gray-500 cursor-pointer">
                        Individual Server Proxies ({client.allowed_mcp_servers.length} servers)
                      </summary>
                      <div className="mt-2 space-y-2">
                        {mcpServers
                          .filter(server => client.allowed_mcp_servers.includes(server.id))
                          .map(server => (
                            <div key={server.id} className="flex items-center justify-between pl-2">
                              <code className="text-xs text-gray-300 dark:text-gray-400 font-mono">{server.proxy_url}</code>
                              <Button
                                variant="secondary"
                                onClick={() => copyToClipboard(server.proxy_url, `config_mcp_${server.id}`)}
                              >
                                {copiedField === `config_mcp_${server.id}` ? <CheckIcon /> : <CopyIcon />}
                              </Button>
                            </div>
                          ))}
                      </div>
                    </details>
                  </div>
                </div>
              )}

              {client.allowed_llm_providers.length === 0 && client.allowed_mcp_servers.length === 0 && (
                <div className="p-4 bg-yellow-900/20 dark:bg-yellow-900/30 border border-yellow-700 dark:border-yellow-600 rounded">
                  <p className="text-sm text-yellow-300 dark:text-yellow-400">
                    No LLM providers or MCP servers configured for this client. Configure access in the Models or MCP tabs.
                  </p>
                </div>
              )}
            </div>
          </Card>

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
                <p className="text-sm text-gray-400 dark:text-gray-500 mt-1">
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
                  <p className="text-sm text-gray-400 dark:text-gray-500">Client ID</p>
                  <p className="font-mono text-sm">{client.client_id}</p>
                </div>
                <div>
                  <p className="text-sm text-gray-400 dark:text-gray-500">Created</p>
                  <p className="font-medium">{formatDate(client.created_at)}</p>
                </div>
                <div>
                  <p className="text-sm text-gray-400 dark:text-gray-500">Last Used</p>
                  <p className="font-medium">{formatDate(client.last_used)}</p>
                </div>
                <div>
                  <p className="text-sm text-gray-400 dark:text-gray-500">Status</p>
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
                      className="absolute right-3 top-1/2 transform -translate-y-1/2 text-gray-400 dark:text-gray-500 hover:text-gray-600 dark:hover:text-gray-400"
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
                <p className="text-sm text-gray-400 dark:text-gray-500 mt-1">
                  Use this in the Authorization header: <code className="bg-gray-800 dark:bg-gray-800 px-1">Authorization: Bearer {maskSecret(client.client_id)}</code>
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
                  <p className="text-sm text-gray-400 dark:text-gray-500 mt-1">
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
                  <p className="text-sm text-gray-400 dark:text-gray-500 mt-1">
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
      id: 'strategy',
      label: 'Strategy',
      content: (
        <div className="space-y-6">
          <Card>
            <div className="p-6 space-y-4">
              <h3 className="text-lg font-semibold">Routing Strategy</h3>
              <p className="text-sm text-gray-400 dark:text-gray-500">
                Select a routing strategy to control which models this client can access, configure auto-routing with intelligent fallback, and set rate limits.
              </p>

              <div>
                <label className="block text-sm font-medium mb-2">Strategy</label>
                <Select
                  value={client?.strategy_id || 'default'}
                  onChange={(e) => handleStrategyChange(e.target.value)}
                  className="w-full"
                >
                  {strategies.map((strategy) => (
                    <option key={strategy.id} value={strategy.id}>
                      {strategy.name} {strategy.parent ? '(Owned)' : '(Shared)'}
                    </option>
                  ))}
                </Select>
              </div>

              {/* Show warning if using shared strategy */}
              {client && client.strategy_id && (() => {
                const selectedStrategy = strategies.find(s => s.id === client.strategy_id)
                if (selectedStrategy && selectedStrategy.parent !== client.id) {
                  return (
                    <div className="p-3 bg-yellow-900/20 dark:bg-yellow-900/30 border border-yellow-700 dark:border-yellow-600 rounded">
                      <p className="text-sm text-yellow-300 dark:text-yellow-400">
                        <strong>Shared Strategy:</strong> Changes to this strategy will affect all clients using it. Create a new strategy or duplicate this one for client-specific configuration.
                      </p>
                    </div>
                  )
                }
                return null
              })()}
            </div>
          </Card>

          {/* Embedded StrategyConfigEditor */}
          {client && client.strategy_id && (
            <StrategyConfigEditor
              strategyId={client.strategy_id}
              readOnly={false}
              onSave={() => {
                loadClientData()
                loadStrategies()
              }}
            />
          )}
        </div>
      ),
    },
    {
      id: 'mcp',
      label: 'MCP',
      content: (
        <div className="space-y-6">
          {/* Token Statistics - Show First */}
          {client.allowed_mcp_servers.length > 0 && (
            <Card>
              <div className="p-6 space-y-4">
                <div className="flex items-center justify-between">
                  <h3 className="text-lg font-semibold">Token Consumption Statistics</h3>
                  <Button
                    variant="secondary"
                    onClick={loadTokenStats}
                    disabled={loadingTokenStats}
                  >
                    {loadingTokenStats ? 'Loading...' : 'Refresh Stats'}
                  </Button>
                </div>

                {loadingTokenStats && (
                  <div className="text-center py-8 text-gray-400 dark:text-gray-500">
                    Analyzing MCP servers...
                  </div>
                )}

                {!loadingTokenStats && tokenStats && (
                  <>
                    <div className="bg-gray-800 dark:bg-gray-800 rounded-lg overflow-hidden">
                      <table className="w-full">
                        <thead className="bg-gray-700 dark:bg-gray-700">
                          <tr>
                            <th className="px-4 py-3 text-left text-sm font-medium">Server</th>
                            <th className="px-4 py-3 text-right text-sm font-medium">Tools</th>
                            <th className="px-4 py-3 text-right text-sm font-medium">Resources</th>
                            <th className="px-4 py-3 text-right text-sm font-medium">Prompts</th>
                            <th className="px-4 py-3 text-right text-sm font-medium">Est. Tokens</th>
                          </tr>
                        </thead>
                        <tbody className="divide-y divide-gray-700 dark:divide-gray-700">
                          {tokenStats.server_stats.map((stat) => (
                            <tr key={stat.server_id} className="hover:bg-gray-750 dark:hover:bg-gray-750">
                              <td className="px-4 py-3 text-sm font-medium">{stat.server_id}</td>
                              <td className="px-4 py-3 text-sm text-right text-gray-300 dark:text-gray-400">{stat.tool_count}</td>
                              <td className="px-4 py-3 text-sm text-right text-gray-300 dark:text-gray-400">{stat.resource_count}</td>
                              <td className="px-4 py-3 text-sm text-right text-gray-300 dark:text-gray-400">{stat.prompt_count}</td>
                              <td className="px-4 py-3 text-sm text-right font-mono text-gray-300 dark:text-gray-400">
                                {stat.estimated_tokens.toLocaleString()}
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>

                    <div className="bg-blue-900/20 dark:bg-blue-900/30 border border-blue-700 dark:border-blue-600 rounded-lg p-4">
                      <h4 className="font-medium text-blue-300 dark:text-blue-400 mb-3">Token Consumption Summary</h4>
                      <div className="grid grid-cols-2 gap-4 text-sm">
                        <div>
                          <p className="text-gray-400 dark:text-gray-500">Without Deferred Loading</p>
                          <p className="text-2xl font-mono font-bold text-gray-100 dark:text-gray-200">
                            {tokenStats.total_tokens.toLocaleString()}
                          </p>
                          <p className="text-xs text-gray-500 dark:text-gray-600">tokens per request</p>
                        </div>
                        <div>
                          <p className="text-gray-400 dark:text-gray-500">With Deferred Loading</p>
                          <p className="text-2xl font-mono font-bold text-green-400 dark:text-green-400">
                            {tokenStats.deferred_tokens.toLocaleString()}
                          </p>
                          <p className="text-xs text-gray-500 dark:text-gray-600">tokens per request (search tool only)</p>
                        </div>
                        <div className="col-span-2">
                          <p className="text-gray-400 dark:text-gray-500">Potential Savings</p>
                          <p className="text-3xl font-mono font-bold text-green-400 dark:text-green-400">
                            {tokenStats.savings_tokens.toLocaleString()} tokens ({tokenStats.savings_percent.toFixed(1)}%)
                          </p>
                        </div>
                      </div>
                    </div>

                    <div className="bg-gray-800 dark:bg-gray-800 rounded-lg p-4">
                      <div className="flex items-start justify-between">
                        <div className="flex-1">
                          <label className="flex items-center gap-3 cursor-pointer">
                            <input
                              type="checkbox"
                              checked={deferredLoading}
                              onChange={handleToggleDeferredLoading}
                              className="w-5 h-5"
                            />
                            <div>
                              <span className="font-medium text-lg">Enable Deferred Loading</span>
                              <p className="text-sm text-gray-400 dark:text-gray-500 mt-1">
                                Only load tools on demand using a search interface. Reduces initial token consumption by {tokenStats.savings_percent.toFixed(0)}%,
                                saving ~{tokenStats.savings_tokens.toLocaleString()} tokens per request.
                              </p>
                            </div>
                          </label>
                        </div>
                      </div>

                      {deferredLoading && (
                        <div className="mt-3 p-3 bg-green-900/20 dark:bg-green-900/30 border border-green-700 dark:border-green-600 rounded">
                          <p className="text-sm text-green-300 dark:text-green-400">
                            ✓ Deferred loading is enabled. Tools will be loaded on demand via search.
                          </p>
                        </div>
                      )}
                    </div>
                  </>
                )}

                {!loadingTokenStats && !tokenStats && (
                  <div className="text-center py-8 text-gray-400 dark:text-gray-500">
                    <p className="mb-2">No statistics available</p>
                    <p className="text-sm">Click "Refresh Stats" to analyze your MCP servers</p>
                  </div>
                )}
              </div>
            </Card>
          )}

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
                          className="absolute right-3 top-1/2 transform -translate-y-1/2 text-gray-400 dark:text-gray-500 hover:text-gray-600 dark:hover:text-gray-400"
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
                      <p className="text-sm text-gray-400 dark:text-gray-500">No MCP servers granted access yet. Add servers below.</p>
                    ) : (
                      <div className="space-y-3">
                        {/* Unified Gateway URL */}
                        {mcpServers.length > 0 && mcpServers[0].gateway_url && (
                          <div className="bg-blue-900/20 dark:bg-blue-900/30 border border-blue-700 dark:border-blue-600 rounded p-3">
                            <div className="flex items-center justify-between mb-2">
                              <span className="font-medium text-sm text-blue-300 dark:text-blue-400">Unified Gateway (All Servers)</span>
                              <Button
                                variant="secondary"
                                onClick={() => copyToClipboard(mcpServers[0].gateway_url, 'gateway_url')}
                              >
                                {copiedField === 'gateway_url' ? <CheckIcon /> : <CopyIcon />}
                              </Button>
                            </div>
                            <code className="text-xs text-blue-200 dark:text-blue-300 font-mono">{mcpServers[0].gateway_url}</code>
                            <p className="text-xs text-blue-300 dark:text-blue-400 mt-1">
                              Access all granted servers through a single endpoint with automatic routing
                            </p>
                          </div>
                        )}

                        {/* Individual Server URLs */}
                        <div className="space-y-2">
                          <p className="text-xs font-medium text-gray-400 dark:text-gray-500">Individual Server Proxies</p>
                          {mcpServers
                            .filter(server => client.allowed_mcp_servers.includes(server.id))
                            .map(server => (
                              <div key={server.id} className="bg-gray-800 dark:bg-gray-800 rounded p-3">
                                <div className="flex items-center justify-between mb-1">
                                  <span className="font-medium text-sm">{server.name}</span>
                                  <Button
                                    variant="secondary"
                                    onClick={() => copyToClipboard(server.proxy_url, `proxy_url_${server.id}`)}
                                  >
                                    {copiedField === `proxy_url_${server.id}` ? <CheckIcon /> : <CopyIcon />}
                                  </Button>
                                </div>
                                <code className="text-xs text-gray-300 dark:text-gray-400 font-mono">{server.proxy_url}</code>
                              </div>
                            ))}
                        </div>
                      </div>
                    )}
                  </div>
                </div>
              )}

              {/* STDIO Auth (Supergateway) */}
              {selectedMcpAuthType === 'stdio' && (
                <div className="space-y-4">
                  <div className="bg-gray-800 dark:bg-gray-800 border border-gray-600 dark:border-gray-700 rounded p-4">
                    <h4 className="font-medium text-gray-100 dark:text-gray-200 mb-2">Supergateway Configuration</h4>
                    <p className="text-sm text-gray-300 dark:text-gray-400 mb-3">
                      Use the Anthropic Supergateway to connect MCP servers via STDIO transport.
                      Set the bearer token as an environment variable:
                    </p>
                    <div className="bg-gray-900 dark:bg-gray-900 rounded p-3 font-mono text-sm">
                      <div className="flex gap-2 items-center">
                        <code className="flex-1 text-gray-100 dark:text-gray-200">
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
                          className="absolute right-3 top-1/2 transform -translate-y-1/2 text-gray-400 dark:text-gray-500 hover:text-gray-600 dark:hover:text-gray-400"
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
                          className="absolute right-3 top-1/2 transform -translate-y-1/2 text-gray-400 dark:text-gray-500 hover:text-gray-600 dark:hover:text-gray-400"
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
                      <p className="text-sm text-gray-400 dark:text-gray-500">No MCP servers granted access yet. Add servers below.</p>
                    ) : (
                      <div className="space-y-3">
                        {/* Unified Gateway URL */}
                        {mcpServers.length > 0 && mcpServers[0].gateway_url && (
                          <div className="bg-blue-900/20 dark:bg-blue-900/30 border border-blue-700 dark:border-blue-600 rounded p-3">
                            <div className="flex items-center justify-between mb-2">
                              <span className="font-medium text-sm text-blue-300 dark:text-blue-400">Unified Gateway (All Servers)</span>
                              <Button
                                variant="secondary"
                                onClick={() => copyToClipboard(mcpServers[0].gateway_url, 'oauth_gateway_url')}
                              >
                                {copiedField === 'oauth_gateway_url' ? <CheckIcon /> : <CopyIcon />}
                              </Button>
                            </div>
                            <code className="text-xs text-blue-200 dark:text-blue-300 font-mono">{mcpServers[0].gateway_url}</code>
                            <p className="text-xs text-blue-300 dark:text-blue-400 mt-1">
                              Access all granted servers through a single endpoint with automatic routing
                            </p>
                          </div>
                        )}

                        {/* Individual Server URLs */}
                        <div className="space-y-2">
                          <p className="text-xs font-medium text-gray-400 dark:text-gray-500">Individual Server Proxies</p>
                          {mcpServers
                            .filter(server => client.allowed_mcp_servers.includes(server.id))
                            .map(server => (
                              <div key={server.id} className="bg-gray-800 dark:bg-gray-800 rounded p-3">
                                <div className="flex items-center justify-between mb-1">
                                  <span className="font-medium text-sm">{server.name}</span>
                                  <Button
                                    variant="secondary"
                                    onClick={() => copyToClipboard(server.proxy_url, `oauth_proxy_url_${server.id}`)}
                                  >
                                    {copiedField === `oauth_proxy_url_${server.id}` ? <CheckIcon /> : <CopyIcon />}
                                  </Button>
                                </div>
                                <code className="text-xs text-gray-300 dark:text-gray-400 font-mono">{server.proxy_url}</code>
                              </div>
                            ))}
                        </div>
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
              <p className="text-sm text-gray-400 dark:text-gray-500 mb-4">
                Select which MCP servers this client can access.
              </p>

              {mcpServers.length === 0 ? (
                <div className="p-4 bg-gray-800 dark:bg-gray-800 rounded text-center">
                  <p className="text-gray-400 dark:text-gray-500 text-sm">No MCP servers configured</p>
                </div>
              ) : (
                <div className="space-y-2">
                  {mcpServers.map((server) => {
                    const hasAccess = client.allowed_mcp_servers.includes(server.id)
                    return (
                      <div
                        key={server.id}
                        className="flex items-center justify-between p-3 bg-gray-800 dark:bg-gray-800 rounded"
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
    {
      id: 'chat',
      label: 'Chat',
      content: (
        <Card>
          <div className="p-6">
            <h3 className="text-lg font-semibold mb-4">Chat</h3>
            <ContextualChat
              context={chatContext}
              disabled={!client.enabled}
            />
          </div>
        </Card>
      ),
    },
    {
      id: 'logs',
      label: 'Logs',
      content: (
        <>
          <FilteredAccessLogs
            type="llm"
            clientName={client.name}
            active={activeTab === 'logs'}
          />
          <div className="mt-6">
            <FilteredAccessLogs
              type="mcp"
              clientId={client.client_id}
              active={activeTab === 'logs'}
            />
          </div>
        </>
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
