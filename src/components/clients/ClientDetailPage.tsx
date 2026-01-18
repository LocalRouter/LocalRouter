import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'
import Input from '../ui/Input'
import DetailPageLayout from '../layouts/DetailPageLayout'

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

interface Provider {
  name: string
  enabled: boolean
}

interface McpServer {
  id: string
  name: string
  enabled: boolean
}

export default function ClientDetailPage({ clientId, onBack }: ClientDetailPageProps) {
  const [client, setClient] = useState<Client | null>(null)
  const [providers, setProviders] = useState<Provider[]>([])
  const [mcpServers, setMcpServers] = useState<McpServer[]>([])
  const [loading, setLoading] = useState(true)
  const [activeTab, setActiveTab] = useState<string>('access')
  const [isSaving, setIsSaving] = useState(false)

  // Form state
  const [name, setName] = useState('')
  const [enabled, setEnabled] = useState(true)

  useEffect(() => {
    loadClientData()
    loadProviders()
    loadMcpServers()
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

  const loadProviders = async () => {
    try {
      const providerList = await invoke<Provider[]>('list_providers')
      setProviders(providerList)
    } catch (error) {
      console.error('Failed to load providers:', error)
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

  const handleSaveSettings = async () => {
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

      alert('Settings saved successfully')
      await loadClientData()
    } catch (error) {
      console.error('Failed to save settings:', error)
      alert(`Error saving settings: ${error}`)
    } finally {
      setIsSaving(false)
    }
  }

  const handleToggleLlmProvider = async (providerName: string, hasAccess: boolean) => {
    if (!client) return

    try {
      if (hasAccess) {
        await invoke('remove_client_llm_provider', {
          clientId: client.client_id,
          provider: providerName,
        })
      } else {
        await invoke('add_client_llm_provider', {
          clientId: client.client_id,
          provider: providerName,
        })
      }

      await loadClientData()
    } catch (error) {
      console.error('Failed to toggle LLM provider:', error)
      alert(`Error updating provider access: ${error}`)
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

  const formatDate = (dateStr: string | null) => {
    if (!dateStr) return 'Never'
    return new Date(dateStr).toLocaleString()
  }

  const maskSecret = (secret: string) => {
    if (secret.length <= 8) return secret
    return `${secret.slice(0, 6)}...${secret.slice(-4)}`
  }

  const tabs = [
    { id: 'access', label: 'Access Control' },
    { id: 'settings', label: 'Settings' },
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
          <Badge variant="info">Bearer Token</Badge>
          <Badge variant={client.enabled ? 'success' : 'error'}>
            {client.enabled ? 'Enabled' : 'Disabled'}
          </Badge>
        </div>
      }
    >
      {/* Access Control Tab */}
      {activeTab === 'access' && (
        <div className="space-y-6">
          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold mb-4">Client Information</h3>
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <p className="text-sm text-gray-400">Client ID</p>
                  <p className="font-mono text-sm">{maskSecret(client.client_id)}</p>
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

          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold mb-4">LLM Provider Access</h3>
              <p className="text-sm text-gray-400 mb-4">
                Select which LLM providers this client can access. If none are selected, the client has no LLM access.
              </p>

              <div className="space-y-2">
                {providers.map((provider) => {
                  const hasAccess = client.allowed_llm_providers.includes(provider.name)
                  return (
                    <div
                      key={provider.name}
                      className="flex items-center justify-between p-3 bg-gray-800 rounded"
                    >
                      <div className="flex items-center gap-3">
                        <span className="font-medium">{provider.name}</span>
                        {!provider.enabled && (
                          <Badge variant="secondary">Provider Disabled</Badge>
                        )}
                      </div>
                      <Button
                        variant={hasAccess ? 'error' : 'primary'}
                        onClick={() => handleToggleLlmProvider(provider.name, hasAccess)}
                        disabled={!provider.enabled}
                      >
                        {hasAccess ? 'Remove Access' : 'Grant Access'}
                      </Button>
                    </div>
                  )
                })}
              </div>

              {client.allowed_llm_providers.length === 0 && (
                <div className="mt-4 p-4 bg-yellow-900/20 border border-yellow-700 rounded">
                  <p className="text-yellow-200 text-sm">
                    This client has no LLM provider access. Grant access to at least one provider to use LLM features.
                  </p>
                </div>
              )}
            </div>
          </Card>

          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold mb-4">MCP Server Access</h3>
              <p className="text-sm text-gray-400 mb-4">
                Select which MCP servers this client can access. If none are selected, the client has no MCP access.
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
                          variant={hasAccess ? 'error' : 'primary'}
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

      {/* Settings Tab */}
      {activeTab === 'settings' && (
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
                <Button onClick={handleSaveSettings} disabled={isSaving}>
                  {isSaving ? 'Saving...' : 'Save Settings'}
                </Button>
              </div>
            </div>
          </Card>

          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold text-red-400 mb-2">Danger Zone</h3>
              <p className="text-gray-400 text-sm mb-4">
                The client secret cannot be regenerated. If compromised, you must delete this client and create a new one.
              </p>
              <div className="bg-red-900/20 border border-red-700 rounded p-4">
                <p className="text-red-200 text-sm">
                  <strong>Security Note:</strong> Client secrets are stored securely in the system keychain and cannot be retrieved after creation.
                  Make sure to save the secret when first creating the client.
                </p>
              </div>
            </div>
          </Card>
        </div>
      )}
    </DetailPageLayout>
  )
}
