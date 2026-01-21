import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'
import Modal from '../ui/Modal'
import Input from '../ui/Input'
import Select from '../ui/Select'
import KeyValueInput from '../ui/KeyValueInput'
import McpServerDetailPage from '../mcp/McpServerDetailPage'
import { McpServerTemplates, McpServerTemplate } from '../mcp/McpServerTemplates'

interface McpServer {
  id: string
  name: string
  transport: 'Stdio' | 'Sse'
  enabled: boolean
  created_at: string
}

interface McpServerHealth {
  server_id: string
  server_name: string
  status: 'healthy' | 'unhealthy' | 'unknown'
  error: string | null
  last_check: string
}

interface McpServersTabProps {
  activeSubTab: string | null
  onTabChange?: (tab: 'mcp-servers', subTab: string) => void
}

export default function McpServersTab({ activeSubTab, onTabChange }: McpServersTabProps) {
  const [servers, setServers] = useState<McpServer[]>([])
  const [healthStatus, setHealthStatus] = useState<Record<string, McpServerHealth>>({})
  const [loading, setLoading] = useState(true)
  const [showCreateModal, setShowCreateModal] = useState(false)
  const [selectedTemplate, setSelectedTemplate] = useState<McpServerTemplate | null>(null)

  // Form state
  const [serverName, setServerName] = useState('')
  const [transportType, setTransportType] = useState<'Stdio' | 'Sse'>('Stdio')
  const [command, setCommand] = useState('')
  const [args, setArgs] = useState('')
  const [envVars, setEnvVars] = useState<Record<string, string>>({})
  const [url, setUrl] = useState('')
  const [headers, setHeaders] = useState<Record<string, string>>({})
  const [isCreating, setIsCreating] = useState(false)

  // Auth config state
  const [authMethod, setAuthMethod] = useState<'none' | 'bearer' | 'oauth_preregistered' | 'oauth_browser'>('none')
  const [bearerToken, setBearerToken] = useState('')
  const [oauthClientId, setOauthClientId] = useState('')
  const [oauthClientSecret, setOauthClientSecret] = useState('')
  const [oauthScopes, setOauthScopes] = useState('')

  useEffect(() => {
    loadServers()
    loadHealth()

    // Refresh health status periodically
    const interval = setInterval(loadHealth, 10000)
    return () => clearInterval(interval)
  }, [])

  const loadServers = async () => {
    setLoading(true)
    try {
      const serverList = await invoke<McpServer[]>('list_mcp_servers')
      setServers(serverList)
    } catch (error) {
      console.error('Failed to load MCP servers:', error)
      alert(`Error loading MCP servers: ${error}`)
    } finally {
      setLoading(false)
    }
  }

  const loadHealth = async () => {
    try {
      const healthList = await invoke<McpServerHealth[]>('get_all_mcp_server_health')
      const healthMap: Record<string, McpServerHealth> = {}
      healthList.forEach(h => {
        healthMap[h.server_id] = h
      })
      setHealthStatus(healthMap)
    } catch (error) {
      console.error('Failed to load health status:', error)
    }
  }

  const handleSelectTemplate = (template: McpServerTemplate) => {
    setSelectedTemplate(template)

    // Pre-populate form fields from template
    setServerName(template.name)
    setTransportType(template.transport)

    if (template.transport === 'Stdio' && template.command) {
      setCommand(template.command)
      if (template.args) {
        setArgs(template.args.join('\n'))
      }
    } else if (template.transport === 'Sse' && template.url) {
      setUrl(template.url)
    }

    // Set auth method
    if (template.authMethod === 'oauth_browser') {
      setAuthMethod('oauth_browser')
      if (template.defaultScopes) {
        setOauthScopes(template.defaultScopes.join(' '))
      }
    } else {
      setAuthMethod(template.authMethod)
    }
  }

  const handleCreateServer = async (e: React.FormEvent) => {
    e.preventDefault()

    setIsCreating(true)

    try {
      // Parse transport config based on type
      // Note: Backend expects tagged enum with "type" field and snake_case variant names
      let transportConfig
      if (transportType === 'Stdio') {
        // Parse args (newline or comma separated)
        const argsList = args.trim() ? args.split(/[\n,]/).map(a => a.trim()).filter(a => a) : []

        transportConfig = {
          type: 'stdio',
          command,
          args: argsList,
          env: envVars
        }
      } else { // Sse (HTTP-SSE)
        transportConfig = {
          type: 'http_sse',
          url,
          headers: headers
        }
      }

      // Build auth config based on auth method
      let authConfig = null
      if (authMethod === 'bearer') {
        authConfig = {
          type: 'bearer_token',
          token: bearerToken // Token will be stored in keychain by backend
        }
      } else if (authMethod === 'oauth_preregistered') {
        const scopesList = oauthScopes.trim() ? oauthScopes.split(/[\s,]+/).map(s => s.trim()).filter(s => s) : []
        authConfig = {
          type: 'oauth_preregistered',
          client_id: oauthClientId,
          client_secret: oauthClientSecret, // Will be stored in keychain
          scopes: scopesList
        }
      } else if (authMethod === 'oauth_browser') {
        authConfig = {
          type: 'oauth_browser'
        }
      }

      await invoke('create_mcp_server', {
        name: serverName || null,
        transport: transportType,
        transportConfig,
        authConfig,
      })

      // Reload servers
      await loadServers()

      // Reset form and close modal
      setShowCreateModal(false)
      setServerName('')
      setCommand('')
      setArgs('')
      setEnvVars({})
      setUrl('')
      setHeaders({})
      setAuthMethod('none')
      setBearerToken('')
      setOauthClientId('')
      setOauthClientSecret('')
      setOauthScopes('')
    } catch (error) {
      console.error('Failed to create MCP server:', error)
      alert(`Error creating MCP server: ${error}`)
    } finally {
      setIsCreating(false)
    }
  }

  const formatDate = (dateStr: string) => {
    return new Date(dateStr).toLocaleString()
  }

  const getHealthBadge = (serverId: string) => {
    const health = healthStatus[serverId]
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

  // If viewing a detail page
  if (activeSubTab && activeSubTab !== 'list') {
    return (
      <McpServerDetailPage
        serverId={activeSubTab}
        onBack={() => onTabChange?.('mcp-servers', 'list')}
      />
    )
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex justify-between items-center">
        <div>
          <h1 className="text-3xl font-bold text-gray-900 dark:text-gray-100">MCP Servers</h1>
          <p className="text-gray-600 dark:text-gray-400 mt-1">
            Manage Model Context Protocol servers for tool integration
          </p>
        </div>
        <Button onClick={() => setShowCreateModal(true)}>
          Create Server
        </Button>
      </div>

      {/* Servers List */}
      <Card>
        <div className="p-6">
          <h2 className="text-xl font-semibold mb-4 text-gray-900 dark:text-gray-100">MCP Servers</h2>

          {loading ? (
            <div className="text-center py-8 text-gray-600 dark:text-gray-400">Loading...</div>
          ) : servers.length === 0 ? (
            <div className="text-center py-8">
              <p className="text-gray-600 dark:text-gray-400 mb-4">No MCP servers configured yet</p>
              <Button onClick={() => setShowCreateModal(true)}>
                Create Your First Server
              </Button>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr className="border-b border-gray-200 dark:border-gray-700">
                    <th className="text-left p-3 font-medium text-gray-700 dark:text-gray-400">Name</th>
                    <th className="text-left p-3 font-medium text-gray-700 dark:text-gray-400">Transport</th>
                    <th className="text-left p-3 font-medium text-gray-700 dark:text-gray-400">Health</th>
                    <th className="text-left p-3 font-medium text-gray-700 dark:text-gray-400">Status</th>
                    <th className="text-left p-3 font-medium text-gray-700 dark:text-gray-400">Created</th>
                    <th className="text-left p-3 font-medium text-gray-700 dark:text-gray-400">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {servers.map((server) => (
                    <tr
                      key={server.id}
                      className="border-b border-gray-200 dark:border-gray-800 hover:bg-gray-50 dark:hover:bg-gray-800/50 cursor-pointer"
                      onClick={() => onTabChange?.('mcp-servers', server.id)}
                    >
                      <td className="p-3 font-medium text-gray-900 dark:text-gray-100">{server.name}</td>
                      <td className="p-3">{getTransportBadge(server.transport)}</td>
                      <td className="p-3">{getHealthBadge(server.id)}</td>
                      <td className="p-3">
                        <Badge variant={server.enabled ? 'success' : 'error'}>
                          {server.enabled ? 'Enabled' : 'Disabled'}
                        </Badge>
                      </td>
                      <td className="p-3 text-sm text-gray-600 dark:text-gray-400">
                        {formatDate(server.created_at)}
                      </td>
                      <td className="p-3">
                        <Button
                          variant="secondary"
                          onClick={(e) => {
                            e.stopPropagation()
                            onTabChange?.('mcp-servers', server.id)
                          }}
                        >
                          View
                        </Button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      </Card>

      {/* Create Server Modal */}
      <Modal
        isOpen={showCreateModal}
        onClose={() => {
          setShowCreateModal(false)
          setSelectedTemplate(null)
          // Reset form fields
          setServerName('')
          setCommand('')
          setArgs('')
          setUrl('')
          setAuthMethod('none')
          setBearerToken('')
          setOauthClientId('')
          setOauthClientSecret('')
          setOauthScopes('')
        }}
        title="Create MCP Server"
      >
        <form onSubmit={handleCreateServer} className="space-y-4">
          {/* Templates Section */}
          {!selectedTemplate && (
            <McpServerTemplates onSelectTemplate={handleSelectTemplate} />
          )}

          {/* Show selected template info if one was chosen */}
          {selectedTemplate && (
            <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded p-3 mb-4">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <span className="text-2xl">{selectedTemplate.icon}</span>
                  <div>
                    <p className="text-sm font-medium text-blue-900 dark:text-blue-100">
                      Using template: {selectedTemplate.name}
                    </p>
                    <p className="text-xs text-blue-700 dark:text-blue-300">
                      You can customize the settings below
                    </p>
                  </div>
                </div>
                <Button
                  variant="secondary"
                  onClick={() => {
                    setSelectedTemplate(null)
                    setServerName('')
                    setCommand('')
                    setArgs('')
                    setUrl('')
                    setAuthMethod('none')
                  }}
                >
                  Clear Template
                </Button>
              </div>
              {selectedTemplate.setupInstructions && (
                <p className="text-xs text-blue-700 dark:text-blue-300 mt-2">
                  ℹ️ {selectedTemplate.setupInstructions}
                </p>
              )}
            </div>
          )}

          <div>
            <label className="block text-sm font-medium mb-2 text-gray-900 dark:text-gray-100">
              Server Name
            </label>
            <Input
              value={serverName}
              onChange={(e) => setServerName(e.target.value)}
              placeholder="My MCP Server"
              required
            />
          </div>

          <div>
            <label className="block text-sm font-medium mb-2 text-gray-900 dark:text-gray-100">
              Transport Type
            </label>
            <Select
              value={transportType}
              onChange={(e) => setTransportType(e.target.value as any)}
            >
              <option value="Stdio">STDIO (Subprocess)</option>
              <option value="Sse">HTTP-SSE (Server-Sent Events)</option>
            </Select>
          </div>

          {/* STDIO Config */}
          {transportType === 'Stdio' && (
            <>
              <div>
                <label className="block text-sm font-medium mb-2 text-gray-900 dark:text-gray-100">
                  Command
                </label>
                <Input
                  value={command}
                  onChange={(e) => setCommand(e.target.value)}
                  placeholder="npx -y @modelcontextprotocol/server-everything"
                  required
                />
                <p className="text-xs text-gray-600 dark:text-gray-500 mt-1">
                  Example: npx -y &lt;command&gt;
                </p>
              </div>

              <div>
                <label className="block text-sm font-medium mb-2 text-gray-900 dark:text-gray-100">
                  Arguments (one per line)
                </label>
                <textarea
                  value={args}
                  onChange={(e) => setArgs(e.target.value)}
                  placeholder="-y&#10;@modelcontextprotocol/server-everything"
                  className="w-full px-3 py-2 bg-white dark:bg-gray-800 border border-gray-300 dark:border-gray-700 rounded-md text-gray-900 dark:text-gray-100"
                  rows={3}
                />
              </div>

              <div>
                <label className="block text-sm font-medium mb-2 text-gray-900 dark:text-gray-100">
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
          {transportType === 'Sse' && (
            <>
              <div>
                <label className="block text-sm font-medium mb-2 text-gray-900 dark:text-gray-100">
                  URL
                </label>
                <Input
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  placeholder="https://mcp.example.com/sse"
                  required
                />
              </div>

              <div>
                <label className="block text-sm font-medium mb-2 text-gray-900 dark:text-gray-100">
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

          {/* Authentication Configuration */}
          {transportType === 'Sse' && (
            <div className="border-t border-gray-200 dark:border-gray-700 pt-4 mt-4">
              <h3 className="text-md font-semibold mb-3">Authentication (Optional)</h3>
              <p className="text-sm text-gray-500 dark:text-gray-400 mb-3">
                Configure how LocalRouter authenticates to this MCP server
              </p>

              <div>
                <label className="block text-sm font-medium mb-2 text-gray-900 dark:text-gray-100">
                  Authentication
                </label>
                <Select
                  value={authMethod}
                  onChange={(e) => setAuthMethod(e.target.value as any)}
                >
                  <option value="none">None / Via headers</option>
                  <option value="bearer">Bearer Token</option>
                  <option value="oauth_preregistered">OAuth Pre-registered</option>
                  <option value="oauth_browser">OAuth</option>
                </Select>
              </div>

              {/* Bearer Token Auth */}
              {authMethod === 'bearer' && (
                <div className="mt-3">
                  <label className="block text-sm font-medium mb-2 text-gray-900 dark:text-gray-100">
                    Bearer Token
                  </label>
                  <Input
                    type="password"
                    value={bearerToken}
                    onChange={(e) => setBearerToken(e.target.value)}
                    placeholder="your-bearer-token"
                    required
                  />
                  <p className="text-xs text-gray-600 dark:text-gray-500 mt-1">
                    Token will be stored securely in system keychain
                  </p>
                </div>
              )}

              {/* OAuth Pre-registered */}
              {authMethod === 'oauth_preregistered' && (
                <div className="mt-3 space-y-3">
                  <div>
                    <label className="block text-sm font-medium mb-2 text-gray-900 dark:text-gray-100">
                      Client ID
                    </label>
                    <Input
                      value={oauthClientId}
                      onChange={(e) => setOauthClientId(e.target.value)}
                      placeholder="your-client-id"
                      required
                    />
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2 text-gray-900 dark:text-gray-100">
                      Client Secret
                    </label>
                    <Input
                      value={oauthClientSecret}
                      onChange={(e) => setOauthClientSecret(e.target.value)}
                      placeholder="your-client-secret"
                      required
                    />
                    <p className="text-xs text-gray-600 dark:text-gray-500 mt-1">
                      Secret will be stored securely in system keychain
                    </p>
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2 text-gray-900 dark:text-gray-100">
                      Scope
                    </label>
                    <Input
                      value={oauthScopes}
                      onChange={(e) => setOauthScopes(e.target.value)}
                      placeholder="tools:read tools:execute"
                    />
                    <p className="text-xs text-gray-600 dark:text-gray-500 mt-1">
                      Space or comma separated. The remaining OAuth details will be discovered from the MCP server.
                    </p>
                  </div>
                </div>
              )}

              {/* OAuth Browser Flow */}
              {authMethod === 'oauth_browser' && (
                <div className="mt-3">
                  <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-700 rounded p-3">
                    <p className="text-blue-800 dark:text-blue-200 text-sm font-medium mb-1">
                      Browser-based OAuth Authentication
                    </p>
                    <p className="text-blue-700 dark:text-blue-300 text-xs">
                      After creating the server, go to the Configuration tab to set up OAuth credentials (Client ID & Secret) and complete browser authentication.
                    </p>
                  </div>
                </div>
              )}
            </div>
          )}

          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="secondary"
              onClick={() => setShowCreateModal(false)}
              disabled={isCreating}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={isCreating}>
              {isCreating ? 'Creating...' : 'Create Server'}
            </Button>
          </div>
        </form>
      </Modal>
    </div>
  )
}
