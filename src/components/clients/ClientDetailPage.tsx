import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'
import Input from '../ui/Input'
import Select from '../ui/Select'
import DetailPageLayout from '../layouts/DetailPageLayout'
import { MetricsChart } from '../charts/MetricsChart'
import { useMetricsSubscription } from '../../hooks/useMetricsSubscription'
import ModelSelectionTable, { Model, ModelSelectionValue } from '../ModelSelectionTable'

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

export default function ClientDetailPage({ clientId, onBack }: ClientDetailPageProps) {
  const refreshKey = useMetricsSubscription()
  const [client, setClient] = useState<Client | null>(null)
  const [mcpServers, setMcpServers] = useState<McpServer[]>([])
  const [models, setModels] = useState<Model[]>([])
  const [loading, setLoading] = useState(true)
  const [activeTab, setActiveTab] = useState<string>('metrics')
  const [isSaving, setIsSaving] = useState(false)

  // Configuration state
  const [name, setName] = useState('')
  const [enabled, setEnabled] = useState(true)

  // Models tab state
  const [routingMode, setRoutingMode] = useState<'forced' | 'multi' | 'prioritized'>('multi')
  const [forcedModel, setForcedModel] = useState<string>('')
  const [modelSelection, setModelSelection] = useState<ModelSelectionValue | null>(null)

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

  const tabs = [
    { id: 'metrics', label: 'Metrics' },
    { id: 'configuration', label: 'Configuration' },
    { id: 'models', label: 'Models' },
    { id: 'mcp', label: 'MCP' },
    { id: 'auth', label: 'Auth' },
  ]

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

  return (
    <DetailPageLayout
      title={client.name}
      onBack={onBack}
      tabs={tabs}
      activeTab={activeTab}
      onTabChange={setActiveTab}
      headerActions={
        <div className="flex items-center gap-2">
          <Badge variant={client.enabled ? 'success' : 'error'}>
            {client.enabled ? 'Enabled' : 'Disabled'}
          </Badge>
        </div>
      }
    >
      {/* Metrics Tab */}
      {activeTab === 'metrics' && (
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
      )}

      {/* Configuration Tab */}
      {activeTab === 'configuration' && (
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
      )}

      {/* Models Tab */}
      {activeTab === 'models' && (
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
                  onChange={(e) => setRoutingMode(e.target.value as 'forced' | 'multi' | 'prioritized')}
                >
                  <option value="forced">Forced Model - Always use a specific model</option>
                  <option value="multi">Multi-Model - Select from available models</option>
                  <option value="prioritized">Prioritized List - Fallback priority order</option>
                </Select>
              </div>

              {routingMode === 'forced' && (
                <div>
                  <label className="block text-sm font-medium mb-2">Forced Model</label>
                  <Select
                    value={forcedModel}
                    onChange={(e) => setForcedModel(e.target.value)}
                  >
                    <option value="">Select a model...</option>
                    {models.map((model) => (
                      <option key={`${model.provider}/${model.id}`} value={`${model.provider}/${model.id}`}>
                        {model.provider} / {model.id}
                      </option>
                    ))}
                  </Select>
                  <p className="text-sm text-gray-400 mt-1">
                    All requests will be routed to this specific model regardless of the requested model
                  </p>
                </div>
              )}

              {(routingMode === 'multi' || routingMode === 'prioritized') && (
                <div>
                  <label className="block text-sm font-medium mb-2">
                    {routingMode === 'multi' ? 'Available Models' : 'Model Priority Order'}
                  </label>
                  <ModelSelectionTable
                    models={models}
                    value={modelSelection}
                    onChange={setModelSelection}
                  />
                  <p className="text-sm text-gray-400 mt-1">
                    {routingMode === 'multi'
                      ? 'Select which models are available to this client'
                      : 'Models will be tried in order from top to bottom'}
                  </p>
                </div>
              )}

              <div className="flex justify-end">
                <Button onClick={() => alert('Model routing saved!')} disabled={isSaving}>
                  Save Model Routing
                </Button>
              </div>
            </div>
          </Card>
        </div>
      )}

      {/* MCP Tab */}
      {activeTab === 'mcp' && (
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
                    <label className="block text-sm font-medium mb-2">MCP Server URL</label>
                    <div className="flex gap-2">
                      <Input
                        value={getApiUrl() + '/mcp'}
                        readOnly
                        className="flex-1 font-mono"
                      />
                      <Button
                        variant="secondary"
                        onClick={() => copyToClipboard(getApiUrl() + '/mcp', 'mcp_url')}
                      >
                        {copiedField === 'mcp_url' ? <CheckIcon /> : <CopyIcon />}
                      </Button>
                    </div>
                  </div>

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
                </div>
              )}

              {/* STDIO Auth (Supergateway) */}
              {selectedMcpAuthType === 'stdio' && (
                <div className="space-y-4">
                  <div className="bg-blue-900/20 border border-blue-700 rounded p-4">
                    <h4 className="font-medium text-blue-200 mb-2">Supergateway Configuration</h4>
                    <p className="text-sm text-gray-300 mb-3">
                      Use the Anthropic Supergateway to connect MCP servers via STDIO transport.
                      Set the bearer token as an environment variable:
                    </p>
                    <div className="bg-gray-800 rounded p-3 font-mono text-sm">
                      <div className="flex gap-2 items-center">
                        <code className="flex-1">
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
                    <label className="block text-sm font-medium mb-2">MCP Server URL</label>
                    <div className="flex gap-2">
                      <Input
                        value={getApiUrl() + '/mcp'}
                        readOnly
                        className="flex-1 font-mono"
                      />
                      <Button
                        variant="secondary"
                        onClick={() => copyToClipboard(getApiUrl() + '/mcp', 'oauth_mcp_url')}
                      >
                        {copiedField === 'oauth_mcp_url' ? <CheckIcon /> : <CopyIcon />}
                      </Button>
                    </div>
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
      )}

      {/* Auth Tab */}
      {activeTab === 'auth' && (
        <div className="space-y-6">
          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold mb-4">Authentication Overview</h3>
              <p className="text-gray-400 mb-4">
                This client uses a single bearer token for all authentication. The same token is used for:
              </p>
              <ul className="list-disc list-inside space-y-2 text-gray-300">
                <li>LLM API access (Authorization: Bearer header)</li>
                <li>Direct MCP server access (Authorization: Bearer header)</li>
                <li>OAuth client credentials flow (client_secret parameter)</li>
              </ul>
            </div>
          </Card>

          <Card>
            <div className="p-6 space-y-4">
              <h3 className="text-lg font-semibold">Credentials</h3>

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
                    onClick={() => copyToClipboard(client.client_id, 'auth_client_id')}
                  >
                    {copiedField === 'auth_client_id' ? <CheckIcon /> : <CopyIcon />}
                  </Button>
                </div>
              </div>

              <div>
                <label className="block text-sm font-medium mb-2">Client Secret (Bearer Token)</label>
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
                    onClick={() => copyToClipboard(client.client_id, 'auth_secret')}
                  >
                    {copiedField === 'auth_secret' ? <CheckIcon /> : <CopyIcon />}
                  </Button>
                </div>
              </div>

              <div className="bg-red-900/20 border border-red-700 rounded p-4">
                <h4 className="font-medium text-red-200 mb-2">Security Warning</h4>
                <p className="text-red-200 text-sm">
                  Keep your client secret secure. Anyone with access to this secret can make API requests on behalf of this client.
                  The secret cannot be regenerated - if compromised, you must delete this client and create a new one.
                </p>
              </div>
            </div>
          </Card>
        </div>
      )}
    </DetailPageLayout>
  )
}
