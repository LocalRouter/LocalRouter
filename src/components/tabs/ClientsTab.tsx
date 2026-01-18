import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'
import Modal from '../ui/Modal'
import Input from '../ui/Input'
import ClientDetailPage from '../clients/ClientDetailPage'

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

interface ClientsTabProps {
  activeSubTab: string | null
  onTabChange?: (tab: 'clients', subTab: string) => void
}

export default function ClientsTab({ activeSubTab, onTabChange }: ClientsTabProps) {
  const [clients, setClients] = useState<Client[]>([])
  const [loading, setLoading] = useState(true)
  const [showCreateModal, setShowCreateModal] = useState(false)
  const [showCredentialsModal, setShowCredentialsModal] = useState(false)
  const [newClientSecret, setNewClientSecret] = useState('')
  const [newClientInfo, setNewClientInfo] = useState<Client | null>(null)

  // Form state
  const [clientName, setClientName] = useState('')
  const [isCreating, setIsCreating] = useState(false)

  useEffect(() => {
    loadClients()
  }, [])

  const loadClients = async () => {
    setLoading(true)
    try {
      const clientList = await invoke<Client[]>('list_clients')
      setClients(clientList)
    } catch (error) {
      console.error('Failed to load clients:', error)
      alert(`Error loading clients: ${error}`)
    } finally {
      setLoading(false)
    }
  }

  const handleCreateClient = async (e: React.FormEvent) => {
    e.preventDefault()

    setIsCreating(true)

    try {
      const result = await invoke<[string, Client]>('create_client', {
        name: clientName || null,
      })

      const [secret, clientInfo] = result
      setNewClientSecret(secret)
      setNewClientInfo(clientInfo)

      // Show credentials modal
      setShowCreateModal(false)
      setShowCredentialsModal(true)

      // Reload clients
      await loadClients()

      // Reset form
      setClientName('')
    } catch (error) {
      console.error('Failed to create client:', error)
      alert(`Error creating client: ${error}`)
    } finally {
      setIsCreating(false)
    }
  }

  const handleDeleteClient = async (clientId: string) => {
    if (!confirm('Are you sure you want to delete this client? This will invalidate all API keys and tokens for this client.')) {
      return
    }

    try {
      await invoke('delete_client', { clientId })
      await loadClients()
    } catch (error) {
      console.error('Failed to delete client:', error)
      alert(`Error deleting client: ${error}`)
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

  const maskSecret = (secret: string) => {
    if (secret.length <= 8) return secret
    return `${secret.slice(0, 6)}...${secret.slice(-4)}`
  }

  const getAuthMethodBadge = () => {
    // All clients use Bearer Token authentication
    return <Badge variant="info">Bearer Token</Badge>
  }

  // If viewing a detail page
  if (activeSubTab && activeSubTab !== 'list') {
    return (
      <ClientDetailPage
        clientId={activeSubTab}
        onBack={() => onTabChange?.('clients', 'list')}
      />
    )
  }

  return (
    <div className="p-6">
      <div className="flex justify-between items-center mb-6">
        <div>
          <h2 className="text-2xl font-bold">Clients</h2>
          <p className="text-gray-400 mt-1">
            Manage API clients for LLM and MCP access
          </p>
        </div>
        <Button onClick={() => setShowCreateModal(true)}>
          Create Client
        </Button>
      </div>

      {loading ? (
        <div className="flex items-center justify-center h-64">
          <div className="text-gray-400">Loading...</div>
        </div>
      ) : clients.length === 0 ? (
        <Card>
          <div className="p-12 text-center">
            <p className="text-gray-400 mb-4">No clients yet</p>
            <Button onClick={() => setShowCreateModal(true)}>
              Create Your First Client
            </Button>
          </div>
        </Card>
      ) : (
        <div className="grid gap-4">
          {clients.map((client) => (
            <Card key={client.id} className="hover:border-blue-500 transition-colors">
              <div className="p-6">
                <div className="flex justify-between items-start">
                  <div className="flex-1">
                    <div className="flex items-center gap-3 mb-2">
                      <h3
                        className="text-lg font-semibold cursor-pointer hover:text-blue-400"
                        onClick={() => onTabChange?.('clients', client.client_id)}
                      >
                        {client.name}
                      </h3>
                      {getAuthMethodBadge()}
                      <Badge variant={client.enabled ? 'success' : 'error'}>
                        {client.enabled ? 'Enabled' : 'Disabled'}
                      </Badge>
                    </div>

                    <div className="grid grid-cols-2 gap-4 mt-4">
                      <div>
                        <p className="text-sm text-gray-400">Client ID</p>
                        <p className="font-mono text-sm">{maskSecret(client.client_id)}</p>
                      </div>
                      <div>
                        <p className="text-sm text-gray-400">Created</p>
                        <p className="font-medium">{formatDate(client.created_at)}</p>
                      </div>
                      <div>
                        <p className="text-sm text-gray-400">LLM Providers</p>
                        <p className="font-medium">
                          {client.allowed_llm_providers.length === 0
                            ? 'No access'
                            : `${client.allowed_llm_providers.length} provider${client.allowed_llm_providers.length !== 1 ? 's' : ''}`}
                        </p>
                      </div>
                      <div>
                        <p className="text-sm text-gray-400">MCP Servers</p>
                        <p className="font-medium">
                          {client.allowed_mcp_servers.length === 0
                            ? 'No access'
                            : `${client.allowed_mcp_servers.length} server${client.allowed_mcp_servers.length !== 1 ? 's' : ''}`}
                        </p>
                      </div>
                      <div>
                        <p className="text-sm text-gray-400">Last Used</p>
                        <p className="font-medium">{formatDate(client.last_used)}</p>
                      </div>
                    </div>
                  </div>

                  <div className="flex gap-2">
                    <Button
                      variant="secondary"
                      onClick={() => onTabChange?.('clients', client.client_id)}
                    >
                      View Details
                    </Button>
                    <Button
                      variant="error"
                      onClick={() => handleDeleteClient(client.client_id)}
                    >
                      Delete
                    </Button>
                  </div>
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}

      {/* Create Client Modal */}
      <Modal
        isOpen={showCreateModal}
        onClose={() => setShowCreateModal(false)}
        title="Create New Client"
      >
        <form onSubmit={handleCreateClient} className="space-y-4">
          <div>
            <label className="block text-sm font-medium mb-2">
              Client Name
            </label>
            <Input
              value={clientName}
              onChange={(e) => setClientName(e.target.value)}
              placeholder="My Application"
              required
            />
            <p className="text-sm text-gray-400 mt-1">
              A descriptive name for this client
            </p>
          </div>

          <div className="bg-blue-900/20 border border-blue-700 rounded p-4">
            <h4 className="font-medium text-blue-200 mb-2">Authentication Method</h4>
            <p className="text-sm text-gray-300">
              All clients use <strong>Bearer Token</strong> authentication.
              You'll receive a secret key that must be included in the
              <code className="bg-gray-800 px-1 mx-1">Authorization: Bearer</code>
              header.
            </p>
          </div>

          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="secondary"
              onClick={() => setShowCreateModal(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={isCreating}>
              {isCreating ? 'Creating...' : 'Create Client'}
            </Button>
          </div>
        </form>
      </Modal>

      {/* Show Credentials Modal */}
      <Modal
        isOpen={showCredentialsModal}
        onClose={() => setShowCredentialsModal(false)}
        title="Client Created Successfully"
      >
        <div className="space-y-4">
          <div className="bg-yellow-900/20 border border-yellow-700 rounded p-4">
            <p className="text-yellow-200 text-sm">
              <strong>Important:</strong> Save these credentials now. The secret will not be shown again.
            </p>
          </div>

          {newClientInfo && (
            <>
              <div>
                <label className="block text-sm font-medium mb-2">Client ID</label>
                <div className="flex gap-2">
                  <Input
                    value={newClientInfo.client_id}
                    readOnly
                    className="flex-1 font-mono"
                  />
                  <Button
                    variant="secondary"
                    onClick={() => copyToClipboard(newClientInfo.client_id, 'Client ID')}
                  >
                    Copy
                  </Button>
                </div>
              </div>

              <div>
                <label className="block text-sm font-medium mb-2">Client Secret</label>
                <div className="flex gap-2">
                  <Input
                    value={newClientSecret}
                    readOnly
                    className="flex-1 font-mono"
                  />
                  <Button
                    variant="secondary"
                    onClick={() => copyToClipboard(newClientSecret, 'Client Secret')}
                  >
                    Copy
                  </Button>
                </div>
                <p className="text-sm text-gray-400 mt-1">
                  Use this as: <code className="bg-gray-800 px-1">Authorization: Bearer {maskSecret(newClientSecret)}</code>
                </p>
              </div>
            </>
          )}

          <div className="bg-gray-800 rounded p-4">
            <h4 className="font-medium mb-2">Next Steps:</h4>
            <ol className="list-decimal list-inside space-y-1 text-sm text-gray-300">
              <li>Save the credentials in a secure location</li>
              <li>Configure LLM provider access in the client details</li>
              <li>Optionally configure MCP server access</li>
              <li>Use the bearer token in your API requests</li>
            </ol>
          </div>

          <div className="flex justify-end">
            <Button onClick={() => setShowCredentialsModal(false)}>
              Done
            </Button>
          </div>
        </div>
      </Modal>
    </div>
  )
}
