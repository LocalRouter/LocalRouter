import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'
import Modal from '../ui/Modal'
import Input from '../ui/Input'
import Select from '../ui/Select'
import McpServerDetailPage from '../mcp/McpServerDetailPage'

interface McpServer {
  id: string
  name: string
  transport: 'Stdio' | 'Sse' | 'WebSocket'
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

  // Form state
  const [serverName, setServerName] = useState('')
  const [transportType, setTransportType] = useState<'Stdio' | 'Sse' | 'WebSocket'>('Stdio')
  const [command, setCommand] = useState('')
  const [args, setArgs] = useState('')
  const [envVars, setEnvVars] = useState('')
  const [url, setUrl] = useState('')
  const [headers, setHeaders] = useState('')
  const [isCreating, setIsCreating] = useState(false)

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

        // Parse env vars (KEY=VALUE format, one per line)
        const envMap: Record<string, string> = {}
        if (envVars.trim()) {
          envVars.split('\n').forEach(line => {
            const [key, ...valueParts] = line.split('=')
            if (key && valueParts.length > 0) {
              envMap[key.trim()] = valueParts.join('=').trim()
            }
          })
        }

        transportConfig = {
          type: 'stdio',
          command,
          args: argsList,
          env: envMap
        }
      } else if (transportType === 'Sse') {
        // Parse headers (KEY: VALUE format, one per line)
        const headersMap: Record<string, string> = {}
        if (headers.trim()) {
          headers.split('\n').forEach(line => {
            const [key, ...valueParts] = line.split(':')
            if (key && valueParts.length > 0) {
              headersMap[key.trim()] = valueParts.join(':').trim()
            }
          })
        }

        transportConfig = {
          type: 'sse',
          url,
          headers: headersMap
        }
      } else { // WebSocket
        const headersMap: Record<string, string> = {}
        if (headers.trim()) {
          headers.split('\n').forEach(line => {
            const [key, ...valueParts] = line.split(':')
            if (key && valueParts.length > 0) {
              headersMap[key.trim()] = valueParts.join(':').trim()
            }
          })
        }

        transportConfig = {
          type: 'web_socket',
          url,
          headers: headersMap
        }
      }

      await invoke('create_mcp_server', {
        name: serverName || null,
        transport: transportType,
        transportConfig,
      })

      // Reload servers
      await loadServers()

      // Reset form and close modal
      setShowCreateModal(false)
      setServerName('')
      setCommand('')
      setArgs('')
      setEnvVars('')
      setUrl('')
      setHeaders('')
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
      Sse: 'warning',
      WebSocket: 'success'
    } as const

    return <Badge variant={colors[transport as keyof typeof colors] || 'secondary'}>{transport}</Badge>
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
          <h1 className="text-3xl font-bold">MCP Servers</h1>
          <p className="text-gray-400 mt-1">
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
          <h2 className="text-xl font-semibold mb-4">MCP Servers</h2>

          {loading ? (
            <div className="text-center py-8 text-gray-400">Loading...</div>
          ) : servers.length === 0 ? (
            <div className="text-center py-8">
              <p className="text-gray-400 mb-4">No MCP servers configured yet</p>
              <Button onClick={() => setShowCreateModal(true)}>
                Create Your First Server
              </Button>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr className="border-b border-gray-700">
                    <th className="text-left p-3 font-medium text-gray-400">Name</th>
                    <th className="text-left p-3 font-medium text-gray-400">Transport</th>
                    <th className="text-left p-3 font-medium text-gray-400">Health</th>
                    <th className="text-left p-3 font-medium text-gray-400">Status</th>
                    <th className="text-left p-3 font-medium text-gray-400">Created</th>
                    <th className="text-left p-3 font-medium text-gray-400">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {servers.map((server) => (
                    <tr
                      key={server.id}
                      className="border-b border-gray-800 hover:bg-gray-800/50 cursor-pointer"
                      onClick={() => onTabChange?.('mcp-servers', server.id)}
                    >
                      <td className="p-3 font-medium">{server.name}</td>
                      <td className="p-3">{getTransportBadge(server.transport)}</td>
                      <td className="p-3">{getHealthBadge(server.id)}</td>
                      <td className="p-3">
                        <Badge variant={server.enabled ? 'success' : 'error'}>
                          {server.enabled ? 'Enabled' : 'Disabled'}
                        </Badge>
                      </td>
                      <td className="p-3 text-sm text-gray-400">
                        {formatDate(server.created_at)}
                      </td>
                      <td className="p-3">
                        <Button
                          size="sm"
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
        onClose={() => setShowCreateModal(false)}
        title="Create MCP Server"
      >
        <form onSubmit={handleCreateServer} className="space-y-4">
          <div>
            <label className="block text-sm font-medium mb-2">
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
            <label className="block text-sm font-medium mb-2">
              Transport Type
            </label>
            <Select
              value={transportType}
              onChange={(e) => setTransportType(e.target.value as any)}
            >
              <option value="Stdio">STDIO (Subprocess)</option>
              <option value="Sse">SSE (Server-Sent Events)</option>
              <option value="WebSocket">WebSocket</option>
            </Select>
          </div>

          {/* STDIO Config */}
          {transportType === 'Stdio' && (
            <>
              <div>
                <label className="block text-sm font-medium mb-2">
                  Command
                </label>
                <Input
                  value={command}
                  onChange={(e) => setCommand(e.target.value)}
                  placeholder="npx"
                  required
                />
              </div>

              <div>
                <label className="block text-sm font-medium mb-2">
                  Arguments (one per line)
                </label>
                <textarea
                  value={args}
                  onChange={(e) => setArgs(e.target.value)}
                  placeholder="-y&#10;@modelcontextprotocol/server-everything"
                  className="w-full px-3 py-2 bg-gray-800 border border-gray-700 rounded-md"
                  rows={3}
                />
              </div>

              <div>
                <label className="block text-sm font-medium mb-2">
                  Environment Variables (KEY=VALUE, one per line)
                </label>
                <textarea
                  value={envVars}
                  onChange={(e) => setEnvVars(e.target.value)}
                  placeholder="API_KEY=your_key&#10;DEBUG=true"
                  className="w-full px-3 py-2 bg-gray-800 border border-gray-700 rounded-md"
                  rows={3}
                />
              </div>
            </>
          )}

          {/* SSE/WebSocket Config */}
          {(transportType === 'Sse' || transportType === 'WebSocket') && (
            <>
              <div>
                <label className="block text-sm font-medium mb-2">
                  URL
                </label>
                <Input
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  placeholder={transportType === 'WebSocket' ? 'ws://localhost:3000' : 'http://localhost:3000'}
                  required
                />
              </div>

              <div>
                <label className="block text-sm font-medium mb-2">
                  Headers (KEY: VALUE, one per line)
                </label>
                <textarea
                  value={headers}
                  onChange={(e) => setHeaders(e.target.value)}
                  placeholder="Authorization: Bearer token&#10;X-Custom-Header: value"
                  className="w-full px-3 py-2 bg-gray-800 border border-gray-700 rounded-md"
                  rows={3}
                />
              </div>
            </>
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
