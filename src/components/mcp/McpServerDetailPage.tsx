import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'
import DetailPageLayout from '../layouts/DetailPageLayout'
import McpConfigForm, { McpConfigFormData } from './McpConfigForm'
import { McpMetricsChart } from '../charts/McpMetricsChart'
import { McpMethodBreakdown } from '../charts/McpMethodBreakdown'
import { useMetricsSubscription } from '../../hooks/useMetricsSubscription'

interface McpServerDetailPageProps {
  serverId: string
  onBack: () => void
}

interface McpServer {
  id: string
  name: string
  transport: 'Stdio' | 'Sse' | 'WebSocket'
  transport_config: TransportConfig
  auth_config: AuthConfig | null
  oauth_config: OAuthConfig | null
  enabled: boolean
  created_at: string
}

type TransportConfig =
  | { Stdio: { command: string; args: string[]; env: Record<string, string> } }
  | { Sse: { url: string; headers: Record<string, string> } }
  | { WebSocket: { url: string; headers: Record<string, string> } }

type AuthConfig =
  | { None: {} }
  | { BearerToken: { token_ref: string } }
  | { CustomHeaders: { headers: Record<string, string> } }
  | { OAuth: { client_id: string; client_secret_ref: string; auth_url: string; token_url: string; scopes: string[] } }
  | { EnvVars: { env: Record<string, string> } }

interface OAuthConfig {
  auth_url: string
  token_url: string
  scopes: string[]
  client_id: string
}

interface Tool {
  name: string
  description?: string
  inputSchema: {
    type: string
    properties?: Record<string, any>
    required?: string[]
  }
}

interface ToolCallResult {
  content?: Array<{ type: string; text?: string; [key: string]: any }>
  isError?: boolean
  [key: string]: any
}

export default function McpServerDetailPage({ serverId, onBack }: McpServerDetailPageProps) {
  const refreshKey = useMetricsSubscription()
  const [server, setServer] = useState<McpServer | null>(null)
  const [loading, setLoading] = useState(true)
  const [activeTab, setActiveTab] = useState<string>('metrics')
  const [isSaving, setIsSaving] = useState(false)

  // Form state for configuration
  const [formData, setFormData] = useState<McpConfigFormData>({
    serverName: '',
    transportType: 'Stdio',
    command: '',
    args: '',
    envVars: '',
    url: '',
    headers: '',
    authMethod: 'none',
    bearerToken: '',
    authHeaders: '',
    authEnvVars: '',
    oauthClientId: '',
    oauthClientSecret: '',
    oauthAuthUrl: '',
    oauthTokenUrl: '',
    oauthScopes: '',
  })

  // Try tab state
  const [tools, setTools] = useState<Tool[]>([])
  const [loadingTools, setLoadingTools] = useState(false)
  const [selectedTool, setSelectedTool] = useState<Tool | null>(null)
  const [toolArguments, setToolArguments] = useState<string>('{}')
  const [toolResult, setToolResult] = useState<ToolCallResult | null>(null)
  const [executingTool, setExecutingTool] = useState(false)

  useEffect(() => {
    loadServerData()
  }, [serverId])

  const loadServerData = async () => {
    setLoading(true)
    try {
      const servers = await invoke<McpServer[]>('list_mcp_servers')
      const serverData = servers.find((s) => s.id === serverId)

      if (serverData) {
        setServer(serverData)
        populateFormData(serverData)
      }
    } catch (error) {
      console.error('Failed to load MCP server data:', error)
    } finally {
      setLoading(false)
    }
  }

  const populateFormData = (serverData: McpServer) => {
    const newFormData: McpConfigFormData = {
      serverName: serverData.name,
      transportType: serverData.transport,
      command: '',
      args: '',
      envVars: '',
      url: '',
      headers: '',
      authMethod: 'none',
      bearerToken: '',
      authHeaders: '',
      authEnvVars: '',
      oauthClientId: '',
      oauthClientSecret: '',
      oauthAuthUrl: '',
      oauthTokenUrl: '',
      oauthScopes: '',
    }

    // Populate transport config
    // Add type guard to ensure transport_config is an object
    if (serverData.transport_config && typeof serverData.transport_config === 'object' && 'Stdio' in serverData.transport_config) {
      const config = serverData.transport_config.Stdio
      newFormData.command = config.command
      newFormData.args = config.args.join('\n')
      newFormData.envVars = Object.entries(config.env)
        .map(([k, v]) => `${k}=${v}`)
        .join('\n')
    } else if (serverData.transport_config && typeof serverData.transport_config === 'object' && 'Sse' in serverData.transport_config) {
      const config = serverData.transport_config.Sse
      newFormData.url = config.url
      newFormData.headers = Object.entries(config.headers)
        .map(([k, v]) => `${k}: ${v}`)
        .join('\n')
    } else if (serverData.transport_config && typeof serverData.transport_config === 'object' && 'WebSocket' in serverData.transport_config) {
      const config = serverData.transport_config.WebSocket
      newFormData.url = config.url
      newFormData.headers = Object.entries(config.headers)
        .map(([k, v]) => `${k}: ${v}`)
        .join('\n')
    }

    // Populate auth config
    if (serverData.auth_config && typeof serverData.auth_config === 'object') {
      if ('BearerToken' in serverData.auth_config) {
        newFormData.authMethod = 'bearer'
      } else if ('CustomHeaders' in serverData.auth_config) {
        newFormData.authMethod = 'custom_headers'
        newFormData.authHeaders = Object.entries(serverData.auth_config.CustomHeaders.headers)
          .map(([k, v]) => `${k}: ${v}`)
          .join('\n')
      } else if ('OAuth' in serverData.auth_config) {
        newFormData.authMethod = 'oauth'
        newFormData.oauthClientId = serverData.auth_config.OAuth.client_id
        newFormData.oauthAuthUrl = serverData.auth_config.OAuth.auth_url
        newFormData.oauthTokenUrl = serverData.auth_config.OAuth.token_url
        newFormData.oauthScopes = serverData.auth_config.OAuth.scopes.join('\n')
      } else if ('EnvVars' in serverData.auth_config) {
        newFormData.authMethod = 'env_vars'
        newFormData.authEnvVars = Object.entries(serverData.auth_config.EnvVars.env)
          .map(([k, v]) => `${k}=${v}`)
          .join('\n')
      }
    }

    setFormData(newFormData)
  }

  const handleFormChange = (field: keyof McpConfigFormData, value: string) => {
    setFormData((prev) => ({ ...prev, [field]: value }))
  }

  const handleSaveConfiguration = async () => {
    if (!server) return

    setIsSaving(true)
    try {
      // Build transport config based on type
      let transportConfig
      if (server.transport === 'Stdio') {
        const argsList = formData.args.trim() ? formData.args.split('\n').map(a => a.trim()).filter(a => a) : []
        const envMap: Record<string, string> = {}
        if (formData.envVars.trim()) {
          formData.envVars.split('\n').forEach(line => {
            const [key, ...valueParts] = line.split('=')
            if (key && valueParts.length > 0) {
              envMap[key.trim()] = valueParts.join('=').trim()
            }
          })
        }
        transportConfig = { Stdio: { command: formData.command, args: argsList, env: envMap } }
      } else if (server.transport === 'Sse') {
        const headersMap: Record<string, string> = {}
        if (formData.headers.trim()) {
          formData.headers.split('\n').forEach(line => {
            const [key, ...valueParts] = line.split(':')
            if (key && valueParts.length > 0) {
              headersMap[key.trim()] = valueParts.join(':').trim()
            }
          })
        }
        transportConfig = { Sse: { url: formData.url, headers: headersMap } }
      } else {
        const headersMap: Record<string, string> = {}
        if (formData.headers.trim()) {
          formData.headers.split('\n').forEach(line => {
            const [key, ...valueParts] = line.split(':')
            if (key && valueParts.length > 0) {
              headersMap[key.trim()] = valueParts.join(':').trim()
            }
          })
        }
        transportConfig = { WebSocket: { url: formData.url, headers: headersMap } }
      }

      await invoke('update_mcp_server', {
        id: serverId,
        name: formData.serverName,
        transportConfig,
      })

      alert('Configuration saved successfully')
      await loadServerData()
    } catch (error) {
      console.error('Failed to save configuration:', error)
      alert(`Error saving configuration: ${error}`)
    } finally {
      setIsSaving(false)
    }
  }

  const handleDelete = async () => {
    if (!confirm('Are you sure you want to delete this MCP server? This action cannot be undone.')) {
      return
    }

    try {
      await invoke('delete_mcp_server', { id: serverId })
      alert('MCP server deleted successfully')
      onBack()
    } catch (error) {
      console.error('Failed to delete MCP server:', error)
      alert(`Error deleting MCP server: ${error}`)
    }
  }

  const loadTools = async () => {
    setLoadingTools(true)
    setTools([])
    try {
      const result = await invoke<any>('list_mcp_tools', { serverId })

      // Extract tools from result
      if (result && result.tools && Array.isArray(result.tools)) {
        setTools(result.tools)
      } else {
        console.warn('Unexpected tools response format:', result)
        setTools([])
      }
    } catch (error) {
      console.error('Failed to load tools:', error)
      alert(`Error loading tools: ${error}`)
    } finally {
      setLoadingTools(false)
    }
  }

  const handleExecuteTool = async () => {
    if (!selectedTool) return

    setExecutingTool(true)
    setToolResult(null)
    try {
      const args = JSON.parse(toolArguments)
      const result = await invoke<ToolCallResult>('call_mcp_tool', {
        serverId,
        toolName: selectedTool.name,
        arguments: args,
      })
      setToolResult(result)
    } catch (error) {
      console.error('Failed to execute tool:', error)
      setToolResult({
        isError: true,
        content: [{ type: 'text', text: `Error: ${error}` }],
      })
    } finally {
      setExecutingTool(false)
    }
  }

  const formatDate = (dateStr: string) => {
    return new Date(dateStr).toLocaleString()
  }

  const getTransportBadge = (transport: string) => {
    const colors = {
      Stdio: 'info',
      Sse: 'warning',
      WebSocket: 'success'
    } as const

    return <Badge variant={colors[transport as keyof typeof colors] || 'secondary'}>{transport}</Badge>
  }

  const maskSecret = (secret: string) => {
    if (!secret) return '***'
    if (secret.length <= 8) return '***'
    return `${secret.slice(0, 4)}...${secret.slice(-4)}`
  }

  const getAuthMethodLabel = (authConfig: AuthConfig | null) => {
    if (!authConfig) return 'None'
    if ('None' in authConfig) return 'None'
    if ('BearerToken' in authConfig) return 'Bearer Token'
    if ('CustomHeaders' in authConfig) return 'Custom Headers'
    if ('OAuth' in authConfig) return 'OAuth 2.0'
    if ('EnvVars' in authConfig) return 'Environment Variables'
    return 'Unknown'
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-gray-400">Loading...</div>
      </div>
    )
  }

  if (!server) {
    return (
      <div className="flex flex-col items-center justify-center h-64">
        <div className="text-gray-400 mb-4">MCP server not found</div>
        <Button onClick={onBack}>Go Back</Button>
      </div>
    )
  }

  const tabs = [
    {
      id: 'metrics',
      label: 'Metrics',
      content: (
        <div className="space-y-6">
          <Card>
            <h3 className="text-lg font-semibold mb-4">MCP Method Breakdown</h3>
            <McpMethodBreakdown
              scope={`server:${serverId}`}
              timeRange="day"
              title="Methods Requested (Last 24h)"
              refreshTrigger={refreshKey}
            />
          </Card>

          <Card>
            <h3 className="text-lg font-semibold mb-4">MCP Request Metrics</h3>
            <div className="grid grid-cols-2 gap-4">
              <McpMetricsChart
                scope="server"
                scopeId={serverId}
                timeRange="day"
                metricType="requests"
                title="Request Volume (Last 24h)"
                refreshTrigger={refreshKey}
              />
              <McpMetricsChart
                scope="server"
                scopeId={serverId}
                timeRange="day"
                metricType="latency"
                title="Average Latency (Last 24h)"
                refreshTrigger={refreshKey}
              />
            </div>
          </Card>

          <Card>
            <h3 className="text-lg font-semibold mb-4">MCP Success Rate</h3>
            <McpMetricsChart
              scope="server"
              scopeId={serverId}
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
      id: 'authentication',
      label: 'Authentication',
      content: (
        <div className="space-y-6">
          <Card>
            <div className="p-6 space-y-4">
              <h3 className="text-lg font-semibold">Authentication Configuration</h3>

              <div className="space-y-3">
                <div>
                  <label className="block text-sm font-medium text-gray-400 mb-1">Auth Method</label>
                  <Badge variant="info">{getAuthMethodLabel(server.auth_config)}</Badge>
                </div>

                {server.auth_config && 'BearerToken' in server.auth_config && (
                  <div>
                    <label className="block text-sm font-medium text-gray-400 mb-1">Bearer Token</label>
                    <div className="bg-gray-800 rounded px-3 py-2 font-mono text-sm">
                      {maskSecret(server.auth_config.BearerToken.token_ref)}
                    </div>
                    <p className="text-xs text-gray-500 mt-1">Token is stored securely in keychain</p>
                  </div>
                )}

                {server.auth_config && 'CustomHeaders' in server.auth_config && (
                  <div>
                    <label className="block text-sm font-medium text-gray-400 mb-1">Custom Headers</label>
                    <div className="bg-gray-800 rounded px-3 py-2 space-y-1">
                      {Object.entries(server.auth_config.CustomHeaders.headers).map(([key, value]) => (
                        <div key={key} className="flex justify-between font-mono text-sm">
                          <span className="text-blue-400">{key}:</span>
                          <span className="text-gray-300">{maskSecret(value)}</span>
                        </div>
                      ))}
                    </div>
                  </div>
                )}

                {server.auth_config && 'OAuth' in server.auth_config && (
                  <div className="space-y-3">
                    <div>
                      <label className="block text-sm font-medium text-gray-400 mb-1">Client ID</label>
                      <div className="bg-gray-800 rounded px-3 py-2 font-mono text-sm">
                        {server.auth_config.OAuth.client_id}
                      </div>
                    </div>
                    <div>
                      <label className="block text-sm font-medium text-gray-400 mb-1">Client Secret</label>
                      <div className="bg-gray-800 rounded px-3 py-2 font-mono text-sm">
                        {maskSecret(server.auth_config.OAuth.client_secret_ref)}
                      </div>
                      <p className="text-xs text-gray-500 mt-1">Secret is stored securely in keychain</p>
                    </div>
                    <div>
                      <label className="block text-sm font-medium text-gray-400 mb-1">Authorization URL</label>
                      <div className="bg-gray-800 rounded px-3 py-2 font-mono text-sm break-all">
                        {server.auth_config.OAuth.auth_url}
                      </div>
                    </div>
                    <div>
                      <label className="block text-sm font-medium text-gray-400 mb-1">Token URL</label>
                      <div className="bg-gray-800 rounded px-3 py-2 font-mono text-sm break-all">
                        {server.auth_config.OAuth.token_url}
                      </div>
                    </div>
                    <div>
                      <label className="block text-sm font-medium text-gray-400 mb-1">Scopes</label>
                      <div className="flex flex-wrap gap-2">
                        {server.auth_config.OAuth.scopes.map((scope) => (
                          <Badge key={scope} variant="secondary">{scope}</Badge>
                        ))}
                      </div>
                    </div>
                  </div>
                )}

                {server.auth_config && 'EnvVars' in server.auth_config && (
                  <div>
                    <label className="block text-sm font-medium text-gray-400 mb-1">Environment Variables</label>
                    <div className="bg-gray-800 rounded px-3 py-2 space-y-1">
                      {Object.entries(server.auth_config.EnvVars.env).map(([key, value]) => (
                        <div key={key} className="flex justify-between font-mono text-sm">
                          <span className="text-green-400">{key}=</span>
                          <span className="text-gray-300">{maskSecret(value)}</span>
                        </div>
                      ))}
                    </div>
                    <p className="text-xs text-gray-500 mt-1">Variables are passed to the server process</p>
                  </div>
                )}

                {(!server.auth_config || 'None' in server.auth_config) && (
                  <div className="bg-gray-800 rounded px-4 py-3">
                    <p className="text-gray-400 text-sm">No authentication configured for this server.</p>
                  </div>
                )}
              </div>
            </div>
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
              <h3 className="text-lg font-semibold">Server Configuration</h3>

              <McpConfigForm
                formData={formData}
                onChange={handleFormChange}
                showTransportType={false}
              />

              <div className="flex justify-end gap-2">
                <Button variant="secondary" onClick={loadServerData}>
                  Reset
                </Button>
                <Button onClick={handleSaveConfiguration} disabled={isSaving}>
                  {isSaving ? 'Saving...' : 'Save Changes'}
                </Button>
              </div>
            </div>
          </Card>

          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold text-red-500 mb-4">Danger Zone</h3>
              <p className="text-sm text-gray-400 mb-4">
                Deleting this server will remove all configuration and cannot be undone.
              </p>
              <Button variant="danger" onClick={handleDelete}>
                Delete Server
              </Button>
            </div>
          </Card>
        </div>
      ),
    },
    {
      id: 'try',
      label: 'Try',
      content: (
        <div className="space-y-6">
          <Card>
            <div className="p-6">
              <div className="flex items-center justify-between mb-4">
                <h3 className="text-lg font-semibold">Test MCP Tools</h3>
                <Button onClick={loadTools} disabled={loadingTools || !server.enabled}>
                  {loadingTools ? 'Loading...' : 'Refresh Tools'}
                </Button>
              </div>

              {!server.enabled && (
                <div className="bg-yellow-50 border border-yellow-200 rounded p-4 mb-4">
                  <p className="text-sm text-yellow-800">
                    This server is disabled. Enable it to test tools.
                  </p>
                </div>
              )}

              {tools.length === 0 && !loadingTools ? (
                <div className="text-center py-8">
                  <p className="text-gray-400">No tools available. Click "Refresh Tools" to load them.</p>
                </div>
              ) : (
                <div className="space-y-4">
                  <div>
                    <label className="block text-sm font-medium text-gray-400 mb-2">Select Tool</label>
                    <select
                      className="w-full bg-gray-800 border border-gray-700 rounded px-3 py-2"
                      value={selectedTool?.name || ''}
                      onChange={(e) => {
                        const tool = tools.find((t) => t.name === e.target.value)
                        setSelectedTool(tool || null)
                        setToolResult(null)
                      }}
                    >
                      <option value="">Select a tool...</option>
                      {tools.map((tool) => (
                        <option key={tool.name} value={tool.name}>
                          {tool.name}
                        </option>
                      ))}
                    </select>
                  </div>

                  {selectedTool && (
                    <>
                      {selectedTool.description && (
                        <div className="bg-gray-800 rounded p-4">
                          <p className="text-sm text-gray-300">{selectedTool.description}</p>
                        </div>
                      )}

                      <div>
                        <label className="block text-sm font-medium text-gray-400 mb-2">Arguments (JSON)</label>
                        <textarea
                          className="w-full bg-gray-800 border border-gray-700 rounded px-3 py-2 font-mono text-sm"
                          rows={6}
                          value={toolArguments}
                          onChange={(e) => setToolArguments(e.target.value)}
                          placeholder='{"key": "value"}'
                        />
                      </div>

                      <Button onClick={handleExecuteTool} disabled={executingTool}>
                        {executingTool ? 'Executing...' : 'Execute Tool'}
                      </Button>

                      {toolResult && (
                        <div className="bg-gray-800 rounded p-4">
                          <h4 className="text-sm font-semibold mb-2">Result:</h4>
                          <pre className="text-xs overflow-auto">{JSON.stringify(toolResult, null, 2)}</pre>
                        </div>
                      )}
                    </>
                  )}
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
      title={server.name || 'Unnamed Server'}
      onBack={onBack}
      tabs={tabs}
      activeTab={activeTab}
      onTabChange={setActiveTab}
      headerActions={
        <div className="flex items-center gap-2">
          {getTransportBadge(server.transport)}
          <Badge variant={server.enabled ? 'success' : 'error'}>
            {server.enabled ? 'Enabled' : 'Disabled'}
          </Badge>
        </div>
      }
    />
  )
}
