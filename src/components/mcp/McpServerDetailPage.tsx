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
  transport: 'Stdio' | 'HttpSse' | 'Sse'
  transport_config: TransportConfig
  auth_config: AuthConfig | null
  oauth_config: OAuthConfig | null
  enabled: boolean
  created_at: string
}

type TransportConfig =
  | { type: 'stdio'; command: string; args: string[]; env: Record<string, string> }
  | { type: 'http_sse' | 'sse'; url: string; headers: Record<string, string> }

type AuthConfig =
  | { type: 'none' }
  | { type: 'bearer_token'; token_ref: string }
  | { type: 'custom_headers'; headers: Record<string, string> }
  | { type: 'oauth'; client_id: string; client_secret_ref: string; auth_url: string; token_url: string; scopes: string[] }
  | { type: 'env_vars'; env: Record<string, string> }

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
  const [toolsError, setToolsError] = useState<string | null>(null)
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
    // Normalize transport type for form (HttpSse -> Sse)
    let normalizedTransport: 'Stdio' | 'Sse' = 'Stdio'
    if (serverData.transport === 'Stdio') {
      normalizedTransport = 'Stdio'
    } else if (serverData.transport === 'HttpSse' || serverData.transport === 'Sse') {
      normalizedTransport = 'Sse'
    }

    const newFormData: McpConfigFormData = {
      serverName: serverData.name,
      transportType: normalizedTransport,
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

    // Populate transport config (tagged union format)
    if (serverData.transport_config && typeof serverData.transport_config === 'object') {
      const config = serverData.transport_config as TransportConfig

      if (config.type === 'stdio') {
        newFormData.command = config.command
        newFormData.args = config.args.join('\n')
        newFormData.envVars = Object.entries(config.env || {})
          .map(([k, v]) => `${k}=${v}`)
          .join('\n')
      } else if (config.type === 'http_sse' || config.type === 'sse') {
        newFormData.url = config.url
        newFormData.headers = Object.entries(config.headers || {})
          .map(([k, v]) => `${k}: ${v}`)
          .join('\n')
      }
    }

    // Populate auth config
    if (serverData.auth_config && typeof serverData.auth_config === 'object') {
      const authConfig = serverData.auth_config as AuthConfig

      if (authConfig.type === 'bearer_token') {
        newFormData.authMethod = 'bearer'
        // Use placeholder to indicate token exists in keychain
        newFormData.bearerToken = '********-STORED-IN-KEYCHAIN-********'
      } else if (authConfig.type === 'custom_headers') {
        newFormData.authMethod = 'custom_headers'
        newFormData.authHeaders = Object.entries(authConfig.headers)
          .map(([k, v]) => `${k}: ${v}`)
          .join('\n')
      } else if (authConfig.type === 'oauth') {
        newFormData.authMethod = 'oauth'
        newFormData.oauthClientId = authConfig.client_id
        newFormData.oauthAuthUrl = authConfig.auth_url
        newFormData.oauthTokenUrl = authConfig.token_url
        newFormData.oauthScopes = authConfig.scopes.join('\n')
      } else if (authConfig.type === 'env_vars') {
        newFormData.authMethod = 'env_vars'
        newFormData.authEnvVars = Object.entries(authConfig.env)
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
      // Build transport config based on type (tagged union format)
      let transportConfig: any
      if (formData.transportType === 'Stdio') {
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
        transportConfig = { type: 'stdio', command: formData.command, args: argsList, env: envMap }
      } else if (formData.transportType === 'Sse') {
        const headersMap: Record<string, string> = {}
        if (formData.headers.trim()) {
          formData.headers.split('\n').forEach(line => {
            const [key, ...valueParts] = line.split(':')
            if (key && valueParts.length > 0) {
              headersMap[key.trim()] = valueParts.join(':').trim()
            }
          })
        }
        transportConfig = { type: 'http_sse', url: formData.url, headers: headersMap }
      } else {
        throw new Error(`Unsupported transport type: ${formData.transportType}`)
      }

      // Build auth config based on auth method (tagged union format)
      let authConfig: any = null
      if (formData.authMethod === 'bearer') {
        // Only send token if it's not the placeholder (which indicates existing token in keychain)
        if (formData.bearerToken && !formData.bearerToken.includes('STORED-IN-KEYCHAIN')) {
          authConfig = {
            type: 'bearer_token',
            token: formData.bearerToken
          }
        } else if (formData.bearerToken.includes('STORED-IN-KEYCHAIN')) {
          // Token exists in keychain, don't update it
          console.log('Keeping existing bearer token (stored in keychain)')
          // Don't send auth_config to avoid overwriting
          authConfig = undefined
        }
      } else if (formData.authMethod === 'custom_headers') {
        const authHeadersMap: Record<string, string> = {}
        if (formData.authHeaders.trim()) {
          formData.authHeaders.split('\n').forEach(line => {
            const [key, ...valueParts] = line.split(':')
            if (key && valueParts.length > 0) {
              authHeadersMap[key.trim()] = valueParts.join(':').trim()
            }
          })
        }
        if (Object.keys(authHeadersMap).length > 0) {
          authConfig = {
            type: 'custom_headers',
            headers: authHeadersMap
          }
        }
      } else if (formData.authMethod === 'oauth') {
        if (formData.oauthClientId && formData.oauthClientSecret) {
          const scopesList = formData.oauthScopes.trim()
            ? formData.oauthScopes.split(/[\n,]/).map(s => s.trim()).filter(s => s)
            : []
          authConfig = {
            type: 'oauth',
            client_id: formData.oauthClientId,
            client_secret: formData.oauthClientSecret,
            auth_url: formData.oauthAuthUrl || '',
            token_url: formData.oauthTokenUrl || '',
            scopes: scopesList
          }
        }
      } else if (formData.authMethod === 'env_vars') {
        const envVarsMap: Record<string, string> = {}
        if (formData.authEnvVars.trim()) {
          formData.authEnvVars.split('\n').forEach(line => {
            const [key, ...valueParts] = line.split('=')
            if (key && valueParts.length > 0) {
              envVarsMap[key.trim()] = valueParts.join('=').trim()
            }
          })
        }
        if (Object.keys(envVarsMap).length > 0) {
          authConfig = {
            type: 'env_vars',
            env: envVarsMap
          }
        }
      }

      console.log('Saving config with transport:', transportConfig)
      console.log('Saving config with auth:', authConfig)

      // Build the command parameters
      const params: any = {
        serverId: serverId,
        name: formData.serverName,
        transportConfig: transportConfig,
      }

      // Only include authConfig if it's not undefined (which means "keep existing")
      if (authConfig !== undefined) {
        params.authConfig = authConfig
      }

      const result = await invoke('update_mcp_server_config', params)

      console.log('Save result:', result)
      alert('Configuration saved successfully')

      // Reload server data to reflect changes
      await loadServerData()
    } catch (error) {
      console.error('Failed to save configuration:', error)
      alert(`Error saving configuration: ${error}`)
      // Don't reload on error to preserve user's changes
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
    setToolsError(null)
    try {
      const result = await invoke<any>('list_mcp_tools', { serverId })

      // Extract tools from result
      if (result && result.tools && Array.isArray(result.tools)) {
        setTools(result.tools)
        setToolsError(null)
      } else {
        console.warn('Unexpected tools response format:', result)
        setTools([])
        setToolsError('Unexpected response format from server')
      }
    } catch (error) {
      console.error('Failed to load tools:', error)
      const errorMessage = error instanceof Error ? error.message : String(error)
      setToolsError(errorMessage)
      setTools([])
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

  const getTransportBadge = (transport: string) => {
    const colors = {
      Stdio: 'info',
      Sse: 'warning',
      HttpSse: 'warning'
    } as const

    return <Badge variant={colors[transport as keyof typeof colors] || 'secondary'}>{transport}</Badge>
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
                showTransportType={true}
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

              {toolsError && (
                <div className="bg-red-900/20 border border-red-500/50 rounded p-4 mb-4">
                  <div className="flex items-start gap-3">
                    <svg className="w-5 h-5 text-red-500 flex-shrink-0 mt-0.5" fill="currentColor" viewBox="0 0 20 20">
                      <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z" clipRule="evenodd" />
                    </svg>
                    <div className="flex-1">
                      <h4 className="text-sm font-semibold text-red-400 mb-1">Failed to load tools</h4>
                      <p className="text-sm text-red-300">{toolsError}</p>
                    </div>
                  </div>
                </div>
              )}

              {!toolsError && (
                <>
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
                </>
              )}
            </div>
          </Card>
        </div>
      ),
    },
  ]

  return (
    <div>
      <Button onClick={onBack} variant="secondary" className="mb-4">
        ← Back to MCP Servers
      </Button>
      <DetailPageLayout
        title={server.name || 'Unnamed Server'}
        tabs={tabs}
        activeTab={activeTab}
        onTabChange={setActiveTab}
        actions={
          <div className="flex items-center gap-2">
            {getTransportBadge(server.transport)}
            <Badge variant={server.enabled ? 'success' : 'error'}>
              {server.enabled ? 'Enabled' : 'Disabled'}
            </Badge>
            <Button variant="danger" onClick={handleDelete}>
              Delete
            </Button>
          </div>
        }
      />
    </div>
  )
}
