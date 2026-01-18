import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Input from '../ui/Input'
import Select from '../ui/Select'
import Badge from '../ui/Badge'
import KeyValueInput from '../ui/KeyValueInput'
import DetailPageLayout from '../layouts/DetailPageLayout'

interface McpServerDetailPageProps {
  serverId: string
  onBack: () => void
}

interface McpServer {
  id: string
  name: string
  transport: 'Stdio' | 'Sse'
  transport_config: TransportConfig
  auth_config: AuthConfig | null
  oauth_config: OAuthConfig | null
  enabled: boolean
  created_at: string
}

type TransportConfig =
  | { Stdio: { command: string; args: string[]; env: Record<string, string> } }
  | { Sse: { url: string; headers: Record<string, string> } }

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

interface McpServerHealth {
  server_id: string
  server_name: string
  status: 'healthy' | 'unhealthy' | 'unknown'
  error: string | null
  last_check: string
}

export default function McpServerDetailPage({ serverId, onBack }: McpServerDetailPageProps) {
  const [server, setServer] = useState<McpServer | null>(null)
  const [health, setHealth] = useState<McpServerHealth | null>(null)
  const [loading, setLoading] = useState(true)
  const [activeTab, setActiveTab] = useState<string>('configuration')
  const [isSaving, setIsSaving] = useState(false)
  const [isStarting, setIsStarting] = useState(false)
  const [isStopping, setIsStopping] = useState(false)
  const [isTesting, setIsTesting] = useState(false)
  const [testResult, setTestResult] = useState<{ success: boolean; message: string } | null>(null)

  // Form state
  const [name, setName] = useState('')
  const [enabled, setEnabled] = useState(true)
  const [command, setCommand] = useState('')
  const [args, setArgs] = useState('')
  const [envVars, setEnvVars] = useState<Record<string, string>>({})
  const [url, setUrl] = useState('')
  const [headers, setHeaders] = useState<Record<string, string>>({})

  useEffect(() => {
    loadServerData()
    loadHealth()

    // Refresh health periodically
    const interval = setInterval(loadHealth, 10000)
    return () => clearInterval(interval)
  }, [serverId])

  const loadServerData = async () => {
    setLoading(true)
    try {
      const servers = await invoke<McpServer[]>('list_mcp_servers')
      const serverData = servers.find((s) => s.id === serverId)

      if (serverData) {
        setServer(serverData)
        setName(serverData.name)
        setEnabled(serverData.enabled)

        // Populate form fields based on transport type
        if ('Stdio' in serverData.transport_config) {
          const config = serverData.transport_config.Stdio
          setCommand(config.command)
          setArgs(config.args.join('\n'))
          setEnvVars(config.env)
        } else if ('Sse' in serverData.transport_config) {
          const config = serverData.transport_config.Sse
          setUrl(config.url)
          setHeaders(config.headers)
        }
      }
    } catch (error) {
      console.error('Failed to load MCP server data:', error)
    } finally {
      setLoading(false)
    }
  }

  const loadHealth = async () => {
    try {
      const healthList = await invoke<McpServerHealth[]>('get_all_mcp_server_health')
      const serverHealth = healthList.find((h) => h.server_id === serverId)
      if (serverHealth) {
        setHealth(serverHealth)
      }
    } catch (error) {
      console.error('Failed to load health:', error)
    }
  }

  const handleSaveConfiguration = async () => {
    if (!server) return

    setIsSaving(true)
    try {
      // Build transport config based on type
      let transportConfig
      if (server.transport === 'Stdio') {
        const argsList = args.trim() ? args.split('\n').map(a => a.trim()).filter(a => a) : []
        transportConfig = { Stdio: { command, args: argsList, env: envVars } }
      } else {
        transportConfig = { Sse: { url, headers: headers } }
      }

      await invoke('update_mcp_server', {
        id: serverId,
        name,
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

  const handleToggleEnabled = async () => {
    setIsSaving(true)
    try {
      await invoke('toggle_mcp_server_enabled', { id: serverId })
      await loadServerData()
    } catch (error) {
      console.error('Failed to toggle enabled:', error)
      alert(`Error toggling enabled: ${error}`)
    } finally {
      setIsSaving(false)
    }
  }

  const handleStartServer = async () => {
    setIsStarting(true)
    try {
      await invoke('start_mcp_server', { serverId })
      await loadHealth()
      alert('MCP server started successfully')
    } catch (error) {
      console.error('Failed to start server:', error)
      alert(`Error starting server: ${error}`)
    } finally {
      setIsStarting(false)
    }
  }

  const handleStopServer = async () => {
    setIsStopping(true)
    try {
      await invoke('stop_mcp_server', { serverId })
      await loadHealth()
      alert('MCP server stopped successfully')
    } catch (error) {
      console.error('Failed to stop server:', error)
      alert(`Error stopping server: ${error}`)
    } finally {
      setIsStopping(false)
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

  const handleTestConnection = async () => {
    setIsTesting(true)
    setTestResult(null)

    try {
      // Start the server if it's not already running
      await invoke('start_mcp_server', { serverId })

      // Wait a moment for the server to initialize
      await new Promise(resolve => setTimeout(resolve, 2000))

      // Check health
      const healthList = await invoke<McpServerHealth[]>('get_all_mcp_server_health')
      const serverHealth = healthList.find((h) => h.server_id === serverId)

      if (serverHealth && serverHealth.status === 'healthy') {
        setTestResult({
          success: true,
          message: 'Successfully connected to MCP server with authentication'
        })
      } else {
        setTestResult({
          success: false,
          message: serverHealth?.error || 'Connection failed - server is unhealthy'
        })
      }

      // Refresh health display
      await loadHealth()
    } catch (error) {
      console.error('Test connection failed:', error)
      setTestResult({
        success: false,
        message: `Connection test failed: ${error}`
      })
    } finally {
      setIsTesting(false)
    }
  }

  const formatDate = (dateStr: string) => {
    return new Date(dateStr).toLocaleString()
  }

  const getHealthBadge = () => {
    if (!health) {
      return <Badge variant="secondary">Unknown</Badge>
    }

    switch (health.status) {
      case 'healthy':
        return <Badge variant="success">Healthy</Badge>
      case 'unhealthy':
        return <Badge variant="error">Unhealthy</Badge>
      default:
        return <Badge variant="secondary">Unknown</Badge>
    }
  }

  const getTransportBadge = (transport: string) => {
    const colors = {
      Stdio: 'info',
      Sse: 'warning'
    } as const

    const displayName = transport === 'Sse' ? 'HTTP-SSE' : transport
    return <Badge variant={colors[transport as keyof typeof colors] || 'secondary'}>{displayName}</Badge>
  }

  const maskSecret = (secret: string) => {
    if (secret.length <= 8) return '***'
    return `${secret.slice(0, 4)}...${secret.slice(-4)}`
  }

  const getAuthMethodName = (authConfig: AuthConfig | null): string => {
    if (!authConfig) return 'None'
    if ('None' in authConfig) return 'None'
    if ('BearerToken' in authConfig) return 'Bearer Token'
    if ('CustomHeaders' in authConfig) return 'Custom Headers'
    if ('OAuth' in authConfig) return 'OAuth'
    if ('EnvVars' in authConfig) return 'Environment Variables'
    return 'Unknown'
  }

  const getAuthMethodBadge = (authConfig: AuthConfig | null) => {
    const name = getAuthMethodName(authConfig)
    const variant = name === 'None' ? 'secondary' : 'info'
    return <Badge variant={variant}>{name}</Badge>
  }

  const tabs = [
    { id: 'configuration', label: 'Configuration' },
    { id: 'authentication', label: 'Authentication' },
    { id: 'health', label: 'Health' },
    { id: 'oauth', label: 'OAuth' },
    { id: 'examples', label: 'Connection Examples' },
    { id: 'settings', label: 'Settings' },
  ]

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
    >
      {/* Configuration Tab */}
      {activeTab === 'configuration' && (
        <div className="space-y-6">
          <Card>
            <div className="p-6 space-y-4">
              <h3 className="text-lg font-semibold">Transport Configuration</h3>

              <div>
                <label className="block text-sm font-medium mb-2">Server Name</label>
                <Input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="My MCP Server"
                />
              </div>

              <div>
                <label className="block text-sm font-medium mb-2">Transport Type</label>
                <div className="flex items-center gap-2">
                  {getTransportBadge(server.transport)}
                  <span className="text-sm text-gray-400">(Read-only)</span>
                </div>
              </div>

              {/* STDIO Config */}
              {server.transport === 'Stdio' && (
                <>
                  <div>
                    <label className="block text-sm font-medium mb-2">Command</label>
                    <Input
                      value={command}
                      onChange={(e) => setCommand(e.target.value)}
                      placeholder="npx -y @modelcontextprotocol/server-everything"
                    />
                    <p className="text-xs text-gray-500 mt-1">
                      Example: npx -y &lt;command&gt;
                    </p>
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2">Arguments (one per line)</label>
                    <textarea
                      value={args}
                      onChange={(e) => setArgs(e.target.value)}
                      placeholder="-y&#10;@modelcontextprotocol/server-everything"
                      className="w-full px-3 py-2 bg-gray-800 border border-gray-700 rounded-md text-gray-100"
                      rows={3}
                    />
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2">
                      Environment Variables
                    </label>
                    <KeyValueInput
                      value={envVars}
                      onChange={setEnvVars}
                      keyPlaceholder="KEY"
                      valuePlaceholder="VALUE"
                    />
                  </div>
                </>
              )}

              {/* HTTP-SSE Config */}
              {server.transport === 'Sse' && (
                <>
                  <div>
                    <label className="block text-sm font-medium mb-2">URL</label>
                    <Input
                      value={url}
                      onChange={(e) => setUrl(e.target.value)}
                      placeholder="https://mcp.example.com/sse"
                    />
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2">
                      Headers
                    </label>
                    <KeyValueInput
                      value={headers}
                      onChange={setHeaders}
                      keyPlaceholder="Header Name"
                      valuePlaceholder="Header Value"
                    />
                  </div>
                </>
              )}

              <div className="flex justify-end">
                <Button onClick={handleSaveConfiguration} disabled={isSaving}>
                  {isSaving ? 'Saving...' : 'Save Configuration'}
                </Button>
              </div>
            </div>
          </Card>

          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold mb-4">Server Information</h3>
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <p className="text-sm text-gray-400">ID</p>
                  <p className="font-mono text-sm">{server.id}</p>
                </div>
                <div>
                  <p className="text-sm text-gray-400">Created</p>
                  <p className="font-medium">{formatDate(server.created_at)}</p>
                </div>
              </div>
            </div>
          </Card>
        </div>
      )}

      {/* Authentication Tab */}
      {activeTab === 'authentication' && (
        <div className="space-y-6">
          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold mb-4">Authentication Configuration</h3>

              <div className="space-y-4">
                <div className="flex items-center justify-between">
                  <span className="text-sm text-gray-400">Authentication Method</span>
                  {getAuthMethodBadge(server.auth_config)}
                </div>

                {server.auth_config && 'BearerToken' in server.auth_config && (
                  <>
                    <div className="border-t border-gray-700 pt-4">
                      <div className="bg-blue-900/20 border border-blue-700 rounded p-4 mb-4">
                        <p className="text-blue-200 text-sm">
                          <strong>Bearer Token Authentication:</strong> The token is securely stored in the system keychain and automatically included in requests.
                        </p>
                      </div>

                      <div>
                        <label className="block text-sm font-medium mb-2">Token Reference</label>
                        <Input
                          value={maskSecret(server.auth_config.BearerToken.token_ref)}
                          readOnly
                          className="bg-gray-800 font-mono"
                        />
                        <p className="text-sm text-gray-400 mt-1">
                          Stored in system keychain as: <code className="bg-gray-800 px-1">LocalRouter-McpServers/{server.id}</code>
                        </p>
                      </div>
                    </div>
                  </>
                )}

                {server.auth_config && 'CustomHeaders' in server.auth_config && (
                  <>
                    <div className="border-t border-gray-700 pt-4">
                      <div className="bg-blue-900/20 border border-blue-700 rounded p-4 mb-4">
                        <p className="text-blue-200 text-sm">
                          <strong>Custom Headers:</strong> Additional HTTP headers are automatically included in all requests to this MCP server.
                        </p>
                      </div>

                      <div>
                        <label className="block text-sm font-medium mb-2">Headers</label>
                        <div className="bg-gray-800 rounded-md p-4 space-y-2">
                          {Object.entries(server.auth_config.CustomHeaders.headers).map(([key, value]) => (
                            <div key={key} className="flex justify-between items-center">
                              <span className="font-mono text-sm text-gray-300">{key}:</span>
                              <span className="font-mono text-sm text-gray-400">{maskSecret(value)}</span>
                            </div>
                          ))}
                          {Object.keys(server.auth_config.CustomHeaders.headers).length === 0 && (
                            <p className="text-sm text-gray-400">No custom headers configured</p>
                          )}
                        </div>
                      </div>
                    </div>
                  </>
                )}

                {server.auth_config && 'OAuth' in server.auth_config && (
                  <>
                    <div className="border-t border-gray-700 pt-4">
                      <div className="bg-blue-900/20 border border-blue-700 rounded p-4 mb-4">
                        <p className="text-blue-200 text-sm">
                          <strong>OAuth Authentication:</strong> OAuth tokens are automatically managed and refreshed. The client secret is securely stored in the system keychain.
                        </p>
                      </div>

                      <div className="space-y-3">
                        <div>
                          <label className="block text-sm font-medium mb-2">Client ID</label>
                          <Input
                            value={server.auth_config.OAuth.client_id}
                            readOnly
                            className="bg-gray-800 font-mono"
                          />
                        </div>

                        <div>
                          <label className="block text-sm font-medium mb-2">Client Secret Reference</label>
                          <Input
                            value={maskSecret(server.auth_config.OAuth.client_secret_ref)}
                            readOnly
                            className="bg-gray-800 font-mono"
                          />
                          <p className="text-sm text-gray-400 mt-1">
                            Stored in system keychain
                          </p>
                        </div>

                        <div>
                          <label className="block text-sm font-medium mb-2">Authorization URL</label>
                          <Input
                            value={server.auth_config.OAuth.auth_url}
                            readOnly
                            className="bg-gray-800"
                          />
                        </div>

                        <div>
                          <label className="block text-sm font-medium mb-2">Token URL</label>
                          <Input
                            value={server.auth_config.OAuth.token_url}
                            readOnly
                            className="bg-gray-800"
                          />
                        </div>

                        <div>
                          <label className="block text-sm font-medium mb-2">Scopes</label>
                          <div className="flex flex-wrap gap-2">
                            {server.auth_config.OAuth.scopes.map((scope) => (
                              <Badge key={scope} variant="info">{scope}</Badge>
                            ))}
                            {server.auth_config.OAuth.scopes.length === 0 && (
                              <span className="text-sm text-gray-400">No scopes configured</span>
                            )}
                          </div>
                        </div>
                      </div>
                    </div>
                  </>
                )}

                {server.auth_config && 'EnvVars' in server.auth_config && (
                  <>
                    <div className="border-t border-gray-700 pt-4">
                      <div className="bg-blue-900/20 border border-blue-700 rounded p-4 mb-4">
                        <p className="text-blue-200 text-sm">
                          <strong>Environment Variables:</strong> These environment variables are passed to the STDIO subprocess when starting the MCP server.
                        </p>
                      </div>

                      <div>
                        <label className="block text-sm font-medium mb-2">Environment Variables</label>
                        <div className="bg-gray-800 rounded-md p-4 space-y-2">
                          {Object.entries(server.auth_config.EnvVars.env).map(([key, value]) => (
                            <div key={key} className="flex justify-between items-center">
                              <span className="font-mono text-sm text-gray-300">{key}=</span>
                              <span className="font-mono text-sm text-gray-400">{maskSecret(value)}</span>
                            </div>
                          ))}
                          {Object.keys(server.auth_config.EnvVars.env).length === 0 && (
                            <p className="text-sm text-gray-400">No environment variables configured</p>
                          )}
                        </div>
                      </div>
                    </div>
                  </>
                )}

                {(!server.auth_config || 'None' in server.auth_config) && (
                  <div className="text-center py-8">
                    <p className="text-gray-400 mb-4">
                      No authentication configured. This MCP server will be accessed without authentication.
                    </p>
                    <p className="text-sm text-gray-500">
                      Authentication can be configured when creating a new MCP server or by editing the configuration.
                    </p>
                  </div>
                )}

                {/* Test Connection Button */}
                <div className="border-t border-gray-700 pt-4">
                  <div className="flex items-center gap-3">
                    <Button
                      onClick={handleTestConnection}
                      disabled={isTesting || !server.enabled}
                      variant="secondary"
                    >
                      {isTesting ? 'Testing Connection...' : 'Test Connection'}
                    </Button>
                    {!server.enabled && (
                      <span className="text-sm text-gray-400">
                        Server must be enabled to test
                      </span>
                    )}
                  </div>

                  {testResult && (
                    <div className={`mt-3 p-4 rounded border ${
                      testResult.success
                        ? 'bg-green-900/20 border-green-700'
                        : 'bg-red-900/20 border-red-700'
                    }`}>
                      <p className={`text-sm ${
                        testResult.success ? 'text-green-200' : 'text-red-200'
                      }`}>
                        <strong>{testResult.success ? 'Success:' : 'Error:'}</strong> {testResult.message}
                      </p>
                    </div>
                  )}

                  <p className="text-sm text-gray-500 mt-2">
                    This will start the MCP server (if not running) and verify that the connection and authentication are working correctly.
                  </p>
                </div>
              </div>
            </div>
          </Card>

          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold mb-4">Security Notes</h3>
              <ul className="list-disc list-inside space-y-2 text-sm text-gray-400">
                <li>Secrets (tokens, API keys, client secrets) are stored securely in the system keychain</li>
                <li>Credentials are never logged or displayed in full</li>
                <li>Authentication is applied automatically when connecting to the MCP server</li>
                <li>For STDIO transports, environment variables are passed to the subprocess</li>
                <li>For HTTP-based transports (SSE), credentials are included in HTTP headers</li>
                <li>OAuth tokens are automatically refreshed when they expire</li>
              </ul>
            </div>
          </Card>
        </div>
      )}

      {/* Health Tab */}
      {activeTab === 'health' && (
        <div className="space-y-6">
          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold mb-4">Health Status</h3>

              <div className="space-y-4">
                <div className="flex items-center justify-between">
                  <span className="text-sm text-gray-400">Current Status</span>
                  {getHealthBadge()}
                </div>

                {health && (
                  <>
                    <div className="flex items-center justify-between">
                      <span className="text-sm text-gray-400">Last Check</span>
                      <span className="font-medium">{formatDate(health.last_check)}</span>
                    </div>

                    {health.error && (
                      <div className="bg-red-900/20 border border-red-700 rounded p-4">
                        <p className="text-red-200 text-sm">
                          <strong>Error:</strong> {health.error}
                        </p>
                      </div>
                    )}
                  </>
                )}

                <div className="flex gap-2 pt-4">
                  <Button
                    onClick={handleStartServer}
                    disabled={isStarting || health?.status === 'healthy'}
                  >
                    {isStarting ? 'Starting...' : 'Start Server'}
                  </Button>
                  <Button
                    variant="secondary"
                    onClick={handleStopServer}
                    disabled={isStopping || health?.status !== 'healthy'}
                  >
                    {isStopping ? 'Stopping...' : 'Stop Server'}
                  </Button>
                  <Button
                    variant="secondary"
                    onClick={loadHealth}
                  >
                    Refresh
                  </Button>
                </div>
              </div>
            </div>
          </Card>
        </div>
      )}

      {/* OAuth Tab */}
      {activeTab === 'oauth' && (
        <div className="space-y-6">
          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold mb-4">OAuth Configuration</h3>

              {server.oauth_config ? (
                <div className="space-y-4">
                  <div className="bg-blue-900/20 border border-blue-700 rounded p-4 mb-4">
                    <p className="text-blue-200 text-sm">
                      <strong>OAuth Discovered:</strong> This MCP server requires OAuth authentication.
                      Tokens are managed automatically.
                    </p>
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2">Authorization URL</label>
                    <Input value={server.oauth_config.auth_url} readOnly className="bg-gray-800" />
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2">Token URL</label>
                    <Input value={server.oauth_config.token_url} readOnly className="bg-gray-800" />
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2">Client ID</label>
                    <Input value={server.oauth_config.client_id} readOnly className="bg-gray-800" />
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2">Scopes</label>
                    <div className="flex flex-wrap gap-2">
                      {server.oauth_config.scopes.map((scope) => (
                        <Badge key={scope} variant="info">{scope}</Badge>
                      ))}
                    </div>
                  </div>
                </div>
              ) : (
                <div className="text-center py-8">
                  <p className="text-gray-400 mb-4">
                    No OAuth configuration detected. This server does not require OAuth authentication.
                  </p>
                  <p className="text-sm text-gray-500">
                    OAuth is auto-discovered via the MCP protocol's <code className="bg-gray-800 px-1">/.well-known/oauth-protected-resource</code> endpoint.
                  </p>
                </div>
              )}
            </div>
          </Card>
        </div>
      )}

      {/* Connection Examples Tab */}
      {activeTab === 'examples' && (
        <div className="space-y-6">
          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold mb-4">Connection Examples</h3>
              <p className="text-sm text-gray-400 mb-6">
                Examples of different ways to connect to this MCP server, including using Supergateway for bridging.
              </p>

              <div className="space-y-8">
                {/* STDIO Transport Examples */}
                {server.transport === 'Stdio' && (
                  <>
                    <div>
                      <h4 className="text-md font-semibold mb-3">Direct STDIO Connection</h4>
                      <p className="text-sm text-gray-400 mb-3">
                        Run the MCP server as a local subprocess:
                      </p>
                      <div className="bg-gray-800 rounded-md p-4 overflow-x-auto">
                        <pre className="text-sm text-gray-300">
{`{
  "transport": "Stdio",
  "config": {
    "type": "stdio",
    "command": "${command || 'npx'}",
    "args": ${JSON.stringify(args.split('\n').filter(a => a.trim()), null, 2)},
    "env": ${JSON.stringify(envVars, null, 2)}
  }
}`}
                        </pre>
                      </div>
                    </div>

                    <div className="border-t border-gray-700 pt-6">
                      <h4 className="text-md font-semibold mb-3">Supergateway Bridge to Remote SSE</h4>
                      <p className="text-sm text-gray-400 mb-3">
                        Use <a href="https://github.com/supercorp-ai/supergateway" target="_blank" rel="noopener noreferrer" className="text-blue-400 hover:underline">Supergateway</a> to bridge STDIO to a remote SSE endpoint:
                      </p>
                      <div className="bg-gray-800 rounded-md p-4 overflow-x-auto">
                        <pre className="text-sm text-gray-300">
{`{
  "transport": "Stdio",
  "config": {
    "type": "stdio",
    "command": "npx",
    "args": [
      "-y",
      "@uptech/supergateway",
      "https://your-mcp-server.example.com/sse"
    ],
    "env": {
      "MCP_API_KEY": "your-remote-server-api-key"
    }
  }
}`}
                        </pre>
                      </div>
                      <p className="text-sm text-gray-500 mt-3">
                        <strong>How it works:</strong> Supergateway spawns as a subprocess and bridges JSON-RPC messages
                        between stdin/stdout and the remote SSE server. The API key is passed via environment variables.
                      </p>
                    </div>

                    <div className="border-t border-gray-700 pt-6">
                      <h4 className="text-md font-semibold mb-3">Supergateway with Authorization Header</h4>
                      <p className="text-sm text-gray-400 mb-3">
                        Use custom headers for authentication:
                      </p>
                      <div className="bg-gray-800 rounded-md p-4 overflow-x-auto">
                        <pre className="text-sm text-gray-300">
{`{
  "transport": "Stdio",
  "config": {
    "type": "stdio",
    "command": "npx",
    "args": [
      "-y",
      "@uptech/supergateway",
      "--header",
      "Authorization: Bearer \${MCP_API_KEY}",
      "https://api.example.com/mcp/sse"
    ],
    "env": {
      "MCP_API_KEY": "your-api-key-here"
    }
  }
}`}
                        </pre>
                      </div>
                      <div className="bg-blue-900/20 border border-blue-700 rounded p-3 mt-3">
                        <p className="text-blue-200 text-sm">
                          <strong>Benefits:</strong> Use STDIO-based tooling with remote SSE servers, centralized API key management,
                          easy to test locally before deploying.
                        </p>
                      </div>
                    </div>
                  </>
                )}

                {/* HTTP-SSE Transport Examples */}
                {server.transport === 'Sse' && (
                  <>
                    <div>
                      <h4 className="text-md font-semibold mb-3">Direct HTTP-SSE Connection</h4>
                      <p className="text-sm text-gray-400 mb-3">
                        Connect directly to the remote HTTP-SSE endpoint:
                      </p>
                      <div className="bg-gray-800 rounded-md p-4 overflow-x-auto">
                        <pre className="text-sm text-gray-300">
{`{
  "transport": "Sse",
  "config": {
    "type": "http_sse",
    "url": "${url || 'https://mcp.example.com/sse'}",
    "headers": ${JSON.stringify(headers, null, 2)}
  }
}`}
                        </pre>
                      </div>
                    </div>

                    <div className="border-t border-gray-700 pt-6">
                      <h4 className="text-md font-semibold mb-3">Via Supergateway (STDIO Bridge)</h4>
                      <p className="text-sm text-gray-400 mb-3">
                        If you prefer STDIO-based tools, use Supergateway to connect to this HTTP-SSE server:
                      </p>
                      <div className="bg-gray-800 rounded-md p-4 overflow-x-auto">
                        <pre className="text-sm text-gray-300">
{`{
  "transport": "Stdio",
  "config": {
    "type": "stdio",
    "command": "npx",
    "args": [
      "-y",
      "@uptech/supergateway",
      "${url || 'https://mcp.example.com/sse'}"
    ],
    "env": {
      "MCP_API_KEY": "your-api-key"
    }
  }
}`}
                        </pre>
                      </div>
                      <p className="text-sm text-gray-500 mt-3">
                        This allows STDIO-based MCP clients (like Claude Desktop) to connect to this HTTP-SSE server.
                      </p>
                    </div>
                  </>
                )}

                {/* General Tips */}
                <div className="border-t border-gray-700 pt-6">
                  <h4 className="text-md font-semibold mb-3">Best Practices</h4>
                  <ul className="list-disc list-inside space-y-2 text-sm text-gray-400">
                    <li>Use descriptive environment variable names (e.g., OPENAI_API_KEY, ANTHROPIC_API_KEY)</li>
                    <li>Never commit secrets to version control - use environment variables or keychain storage</li>
                    <li>Use different API keys for development vs production environments</li>
                    <li>Always use HTTPS/TLS for remote connections</li>
                    <li>Rotate API keys regularly for security</li>
                  </ul>
                </div>
              </div>
            </div>
          </Card>
        </div>
      )}

      {/* Settings Tab */}
      {activeTab === 'settings' && (
        <div className="space-y-6">
          <Card>
            <div className="p-6 space-y-4">
              <h3 className="text-lg font-semibold">Server Settings</h3>

              <div>
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={enabled}
                    onChange={handleToggleEnabled}
                    className="w-4 h-4"
                    disabled={isSaving}
                  />
                  <span className="font-medium">Enabled</span>
                </label>
                <p className="text-sm text-gray-400 mt-1">
                  Disabled servers cannot be started
                </p>
              </div>
            </div>
          </Card>

          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold text-red-400 mb-2">Danger Zone</h3>
              <p className="text-gray-400 text-sm mb-4">
                Deleting this MCP server will remove all configuration and stop the server if running.
              </p>
              <Button variant="error" onClick={handleDelete}>
                Delete MCP Server
              </Button>
            </div>
          </Card>
        </div>
      )}
    </DetailPageLayout>
  )
}
