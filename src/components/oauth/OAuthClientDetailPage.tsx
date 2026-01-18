import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Input from '../ui/Input'
import Badge from '../ui/Badge'
import DetailPageLayout from '../layouts/DetailPageLayout'

interface OAuthClientDetailPageProps {
  clientId: string
  onBack: () => void
}

interface OAuthClient {
  id: string
  name: string
  client_id: string
  enabled: boolean
  created_at: string
  last_used: string | null
  linked_server_ids: string[]
}

interface McpServer {
  id: string
  name: string
  enabled: boolean
}

export default function OAuthClientDetailPage({ clientId, onBack }: OAuthClientDetailPageProps) {
  const [client, setClient] = useState<OAuthClient | null>(null)
  const [clientSecret, setClientSecret] = useState<string>('')
  const [showSecret, setShowSecret] = useState(false)
  const [secretLoaded, setSecretLoaded] = useState(false)
  const [secretLoadError, setSecretLoadError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [activeTab, setActiveTab] = useState<string>('credentials')
  const [isSaving, setIsSaving] = useState(false)

  // Form state
  const [name, setName] = useState('')
  const [enabled, setEnabled] = useState(true)

  // MCP servers state
  const [mcpServers, setMcpServers] = useState<McpServer[]>([])
  const [linkedServerIds, setLinkedServerIds] = useState<string[]>([])

  useEffect(() => {
    loadClientData()
  }, [clientId])

  const loadClientData = async () => {
    setLoading(true)
    try {
      const [clients, servers] = await Promise.all([
        invoke<OAuthClient[]>('list_oauth_clients'),
        invoke<McpServer[]>('list_mcp_servers').catch(() => []),
      ])

      const clientData = clients.find((c) => c.id === clientId)
      if (clientData) {
        setClient(clientData)
        setName(clientData.name)
        setEnabled(clientData.enabled)
        setLinkedServerIds(clientData.linked_server_ids || [])
      }

      setMcpServers(servers)
    } catch (error) {
      console.error('Failed to load OAuth client data:', error)
    } finally {
      setLoading(false)
    }
  }

  const loadClientSecret = async () => {
    if (secretLoaded) {
      setShowSecret(!showSecret)
      return
    }

    try {
      setSecretLoadError(null)
      const secret = await invoke<string>('get_oauth_client_secret', { id: clientId })
      setClientSecret(secret)
      setSecretLoaded(true)
      setShowSecret(true)
    } catch (error: any) {
      console.error('Failed to load client secret:', error)
      const errorMsg = error?.toString() || 'Unknown error'
      if (errorMsg.includes('passphrase') || errorMsg.includes('keychain')) {
        setSecretLoadError('Keychain access denied. Please approve keychain access to view the secret.')
      } else {
        setSecretLoadError(`Failed to load secret: ${errorMsg}`)
      }
    }
  }

  const handleSaveSettings = async () => {
    setIsSaving(true)
    try {
      // Update name
      if (name !== client?.name) {
        await invoke('update_oauth_client_name', { id: clientId, name })
      }

      // Update enabled status
      if (enabled !== client?.enabled) {
        await invoke('toggle_oauth_client_enabled', { id: clientId })
      }

      alert('Settings saved successfully')
      await loadClientData()
    } catch (error) {
      console.error('Failed to save settings:', error)
      alert(`Error saving settings: ${error}`)
    } finally {
      setIsSaving(false)
    }
  }

  const handleSaveLinkedServers = async () => {
    setIsSaving(true)
    try {
      // Get current linked servers
      const currentLinked = client?.linked_server_ids || []

      // Determine which to link/unlink
      const toLink = linkedServerIds.filter(id => !currentLinked.includes(id))
      const toUnlink = currentLinked.filter(id => !linkedServerIds.includes(id))

      // Execute link/unlink operations
      for (const serverId of toLink) {
        await invoke('link_mcp_server', { clientId, serverId })
      }

      for (const serverId of toUnlink) {
        await invoke('unlink_mcp_server', { clientId, serverId })
      }

      alert('Linked servers updated successfully')
      await loadClientData()
    } catch (error) {
      console.error('Failed to save linked servers:', error)
      alert(`Error saving linked servers: ${error}`)
    } finally {
      setIsSaving(false)
    }
  }

  const handleDelete = async () => {
    if (!confirm('Are you sure you want to delete this OAuth client? This action cannot be undone.')) {
      return
    }

    try {
      await invoke('delete_oauth_client', { id: clientId })
      alert('OAuth client deleted successfully')
      onBack()
    } catch (error) {
      console.error('Failed to delete OAuth client:', error)
      alert(`Error deleting OAuth client: ${error}`)
    }
  }

  const copyToClipboard = (text: string, label: string) => {
    navigator.clipboard.writeText(text)
    alert(`${label} copied to clipboard!`)
  }

  const formatDate = (dateStr: string | null) => {
    if (!dateStr) return 'Never'
    return new Date(dateStr).toLocaleString()
  }

  const tabs = [
    { id: 'credentials', label: 'Credentials' },
    { id: 'linked-servers', label: 'Linked Servers' },
    { id: 'settings', label: 'Settings' },
  ]

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
        <div className="text-gray-400 dark:text-gray-500 mb-4">OAuth client not found</div>
        <Button onClick={onBack}>Go Back</Button>
      </div>
    )
  }

  return (
    <DetailPageLayout
      title={client.name || 'Unnamed Client'}
      tabs={tabs}
      activeTab={activeTab}
      onTabChange={setActiveTab}
      actions={
        <Badge variant={client.enabled ? 'success' : 'error'}>
          {client.enabled ? 'Enabled' : 'Disabled'}
        </Badge>
      }
    >
      {/* Credentials Tab */}
      {activeTab === 'credentials' && (
        <div className="space-y-6">
          <Card>
            <div className="p-6 space-y-4">
              <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">OAuth Credentials</h3>

              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">Client ID</label>
                <div className="flex gap-2">
                  <Input
                    value={client.client_id}
                    readOnly
                    className="flex-1 font-mono text-sm bg-gray-100 dark:bg-gray-800"
                  />
                  <Button
                    variant="secondary"
                    onClick={() => copyToClipboard(client.client_id, 'Client ID')}
                  >
                    Copy
                  </Button>
                </div>
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">Client Secret</label>
                <div className="flex gap-2">
                  <Input
                    type={showSecret ? 'text' : 'password'}
                    value={showSecret ? clientSecret : '••••••••••••••••'}
                    readOnly
                    className="flex-1 font-mono text-sm bg-gray-100 dark:bg-gray-800"
                  />
                  <Button
                    variant="secondary"
                    onClick={loadClientSecret}
                  >
                    {secretLoaded ? (showSecret ? 'Hide' : 'Show') : 'Reveal'}
                  </Button>
                  {showSecret && (
                    <Button
                      variant="secondary"
                      onClick={() => copyToClipboard(clientSecret, 'Client Secret')}
                    >
                      Copy
                    </Button>
                  )}
                </div>
                {secretLoadError && (
                  <p className="text-red-400 dark:text-red-500 text-sm mt-2">{secretLoadError}</p>
                )}
              </div>

              <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-700 rounded p-4">
                <p className="text-blue-900 dark:text-blue-200 text-sm">
                  <strong>Authentication:</strong> Use these credentials with OAuth 2.0 Client Credentials flow.
                  Include them as <code className="bg-blue-100 dark:bg-gray-800 px-1 rounded">Authorization: Basic base64(client_id:client_secret)</code>
                </p>
              </div>
            </div>
          </Card>

          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-4">Client Information</h3>
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <p className="text-sm text-gray-500 dark:text-gray-400">Name</p>
                  <p className="font-medium text-gray-900 dark:text-gray-100">{client.name}</p>
                </div>
                <div>
                  <p className="text-sm text-gray-500 dark:text-gray-400">Status</p>
                  <Badge variant={client.enabled ? 'success' : 'error'}>
                    {client.enabled ? 'Enabled' : 'Disabled'}
                  </Badge>
                </div>
                <div>
                  <p className="text-sm text-gray-500 dark:text-gray-400">Created</p>
                  <p className="font-medium text-gray-900 dark:text-gray-100">{formatDate(client.created_at)}</p>
                </div>
                <div>
                  <p className="text-sm text-gray-500 dark:text-gray-400">Last Used</p>
                  <p className="font-medium text-gray-900 dark:text-gray-100">{formatDate(client.last_used)}</p>
                </div>
              </div>
            </div>
          </Card>
        </div>
      )}

      {/* Linked Servers Tab */}
      {activeTab === 'linked-servers' && (
        <div className="space-y-6">
          <Card>
            <div className="p-6">
              <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-4">Link MCP Servers</h3>
              <p className="text-gray-600 dark:text-gray-400 text-sm mb-4">
                Select which MCP servers this OAuth client can access. Only linked servers will be available to this client.
              </p>

              {mcpServers.length === 0 ? (
                <div className="text-center py-8 text-gray-500 dark:text-gray-400">
                  No MCP servers configured yet. Create an MCP server first.
                </div>
              ) : (
                <div className="space-y-2">
                  {mcpServers.map((server) => (
                    <label
                      key={server.id}
                      className="flex items-center gap-3 p-3 rounded border border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800/50 cursor-pointer"
                    >
                      <input
                        type="checkbox"
                        checked={linkedServerIds.includes(server.id)}
                        onChange={(e) => {
                          if (e.target.checked) {
                            setLinkedServerIds([...linkedServerIds, server.id])
                          } else {
                            setLinkedServerIds(linkedServerIds.filter(id => id !== server.id))
                          }
                        }}
                        className="w-4 h-4"
                      />
                      <div className="flex-1">
                        <div className="font-medium text-gray-900 dark:text-gray-100">{server.name}</div>
                        <div className="text-sm text-gray-500 dark:text-gray-400">ID: {server.id}</div>
                      </div>
                      <Badge variant={server.enabled ? 'success' : 'error'}>
                        {server.enabled ? 'Enabled' : 'Disabled'}
                      </Badge>
                    </label>
                  ))}
                </div>
              )}

              <div className="mt-6 flex justify-end">
                <Button onClick={handleSaveLinkedServers} disabled={isSaving}>
                  {isSaving ? 'Saving...' : 'Save Changes'}
                </Button>
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
              <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Client Settings</h3>

              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">Client Name</label>
                <Input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="My MCP Client"
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
                  <span className="font-medium text-gray-900 dark:text-gray-100">Enabled</span>
                </label>
                <p className="text-sm text-gray-500 dark:text-gray-400 mt-1">
                  Disabled clients cannot authenticate
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
              <h3 className="text-lg font-semibold text-red-600 dark:text-red-400 mb-2">Danger Zone</h3>
              <p className="text-gray-600 dark:text-gray-400 text-sm mb-4">
                Deleting this OAuth client will revoke access for all applications using these credentials.
              </p>
              <Button variant="danger" onClick={handleDelete}>
                Delete OAuth Client
              </Button>
            </div>
          </Card>
        </div>
      )}
    </DetailPageLayout>
  )
}
