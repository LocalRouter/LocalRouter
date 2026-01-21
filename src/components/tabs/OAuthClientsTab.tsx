import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'
import Modal from '../ui/Modal'
import Input from '../ui/Input'
import OAuthClientDetailPage from '../oauth/OAuthClientDetailPage'

interface OAuthClient {
  id: string
  name: string
  client_id: string
  enabled: boolean
  created_at: string
  last_used: string | null
  linked_server_ids: string[]
}

interface OAuthClientsTabProps {
  activeSubTab: string | null
  onTabChange?: (tab: 'oauth-clients', subTab: string) => void
}

export default function OAuthClientsTab({ activeSubTab, onTabChange }: OAuthClientsTabProps) {
  const [clients, setClients] = useState<OAuthClient[]>([])
  const [loading, setLoading] = useState(true)
  const [showCreateModal, setShowCreateModal] = useState(false)

  // Form state
  const [clientName, setClientName] = useState('')
  const [isCreating, setIsCreating] = useState(false)

  useEffect(() => {
    loadClients()
  }, [])

  const loadClients = async () => {
    setLoading(true)
    try {
      const clientList = await invoke<OAuthClient[]>('list_oauth_clients')
      setClients(clientList)
    } catch (error) {
      console.error('Failed to load OAuth clients:', error)
      alert(`Error loading OAuth clients: ${error}`)
    } finally {
      setLoading(false)
    }
  }

  const handleCreateClient = async (e: React.FormEvent) => {
    e.preventDefault()

    setIsCreating(true)

    try {
      const result = await invoke<[string, string, OAuthClient]>('create_oauth_client', {
        name: clientName || null,
      })

      const [_clientId, _clientSecret, clientInfo] = result

      // Close modal
      setShowCreateModal(false)

      // Reload clients
      await loadClients()

      // Reset form
      setClientName('')

      // Navigate to the newly created client's detail page
      onTabChange?.('oauth-clients', clientInfo.id)
    } catch (error) {
      console.error('Failed to create OAuth client:', error)
      alert(`Error creating OAuth client: ${error}`)
    } finally {
      setIsCreating(false)
    }
  }

  const formatDate = (dateStr: string | null) => {
    if (!dateStr) return 'Never'
    return new Date(dateStr).toLocaleString()
  }

  const maskClientId = (clientId: string) => {
    if (clientId.length <= 8) return clientId
    return `${clientId.slice(0, 4)}...${clientId.slice(-4)}`
  }

  // If viewing a detail page
  if (activeSubTab && activeSubTab !== 'list') {
    return (
      <OAuthClientDetailPage
        clientId={activeSubTab}
        onBack={() => onTabChange?.('oauth-clients', 'list')}
      />
    )
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex justify-between items-center">
        <div>
          <h1 className="text-3xl font-bold text-gray-900 dark:text-gray-100">OAuth Clients</h1>
          <p className="text-gray-600 dark:text-gray-400 mt-1">
            Manage OAuth clients for MCP (Model Context Protocol) authentication
          </p>
        </div>
        <Button onClick={() => setShowCreateModal(true)}>
          Create Client
        </Button>
      </div>

      {/* Clients List */}
      <Card>
        <div className="p-6">
          <h2 className="text-xl font-semibold text-gray-900 dark:text-gray-100 mb-4">OAuth Clients</h2>

          {loading ? (
            <div className="text-center py-8 text-gray-500 dark:text-gray-400">Loading...</div>
          ) : clients.length === 0 ? (
            <div className="text-center py-8">
              <p className="text-gray-500 dark:text-gray-400 mb-4">No OAuth clients yet</p>
              <Button onClick={() => setShowCreateModal(true)}>
                Create Your First Client
              </Button>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr className="border-b border-gray-200 dark:border-gray-700">
                    <th className="text-left p-3 font-medium text-gray-600 dark:text-gray-400">Name</th>
                    <th className="text-left p-3 font-medium text-gray-600 dark:text-gray-400">Client ID</th>
                    <th className="text-left p-3 font-medium text-gray-600 dark:text-gray-400">Status</th>
                    <th className="text-left p-3 font-medium text-gray-600 dark:text-gray-400">Linked Servers</th>
                    <th className="text-left p-3 font-medium text-gray-600 dark:text-gray-400">Created</th>
                    <th className="text-left p-3 font-medium text-gray-600 dark:text-gray-400">Last Used</th>
                    <th className="text-left p-3 font-medium text-gray-600 dark:text-gray-400">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {clients.map((client) => (
                    <tr
                      key={client.id}
                      className="border-b border-gray-200 dark:border-gray-800 hover:bg-gray-50 dark:hover:bg-gray-800/50 cursor-pointer"
                      onClick={() => onTabChange?.('oauth-clients', client.id)}
                    >
                      <td className="p-3 text-gray-900 dark:text-gray-100">{client.name}</td>
                      <td className="p-3 font-mono text-sm text-gray-600 dark:text-gray-400">
                        {maskClientId(client.client_id)}
                      </td>
                      <td className="p-3">
                        <Badge variant={client.enabled ? 'success' : 'error'}>
                          {client.enabled ? 'Enabled' : 'Disabled'}
                        </Badge>
                      </td>
                      <td className="p-3">
                        <Badge variant="info">
                          {client.linked_server_ids.length} servers
                        </Badge>
                      </td>
                      <td className="p-3 text-sm text-gray-600 dark:text-gray-400">
                        {formatDate(client.created_at)}
                      </td>
                      <td className="p-3 text-sm text-gray-600 dark:text-gray-400">
                        {formatDate(client.last_used)}
                      </td>
                      <td className="p-3">
                        <Button
                          variant="secondary"
                          onClick={(e) => {
                            e.stopPropagation()
                            onTabChange?.('oauth-clients', client.id)
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

      {/* Create Client Modal */}
      <Modal
        isOpen={showCreateModal}
        onClose={() => setShowCreateModal(false)}
        title="Create OAuth Client"
      >
        <form onSubmit={handleCreateClient} className="space-y-4">
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
              Client Name (Optional)
            </label>
            <Input
              value={clientName}
              onChange={(e) => setClientName(e.target.value)}
              placeholder="My MCP Client"
            />
            <p className="text-sm text-gray-500 dark:text-gray-400 mt-1">
              A friendly name to identify this client
            </p>
          </div>

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
              {isCreating ? 'Creating...' : 'Create Client'}
            </Button>
          </div>
        </form>
      </Modal>
    </div>
  )
}
