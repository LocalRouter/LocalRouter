import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'
import Modal from '../ui/Modal'
import Input from '../ui/Input'
import ClientDetailPage from '../clients/ClientDetailPage'
import ComparisonPanel from '../ComparisonPanel'
import { useMetricsSubscription } from '../../hooks/useMetricsSubscription'

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  allowed_llm_providers: string[]
  mcp_access_mode: "none" | "all" | "specific"
  mcp_servers: string[]
  created_at: string
  last_used: string | null
}

interface ClientsTabProps {
  activeSubTab: string | null
  onTabChange?: (tab: 'clients', subTab: string) => void
}

export default function ClientsTab({ activeSubTab, onTabChange }: ClientsTabProps) {
  const refreshKey = useMetricsSubscription()
  const [clients, setClients] = useState<Client[]>([])
  const [trackedApiKeys, setTrackedApiKeys] = useState<string[]>([])
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
    loadTrackedApiKeys()
  }, [refreshKey])

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

  const loadTrackedApiKeys = async () => {
    try {
      const apiKeys = await invoke<string[]>('list_tracked_api_keys')
      setTrackedApiKeys(apiKeys)
    } catch (error) {
      console.error('Failed to load tracked API keys:', error)
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

  const copyToClipboard = (text: string, label: string) => {
    navigator.clipboard.writeText(text)
    alert(`${label} copied to clipboard!`)
  }

  const maskSecret = (secret: string) => {
    if (secret.length <= 8) return secret
    return `${secret.slice(0, 6)}...${secret.slice(-4)}`
  }

  // If viewing a detail page
  if (activeSubTab && activeSubTab !== 'list') {
    // Parse activeSubTab for format: "clientId|tab|routingMode"
    const parts = activeSubTab.split('|')
    const clientId = parts[0]
    const initialTab = parts[1] || undefined
    const initialRoutingMode = parts[2] as 'forced' | 'multi' | 'prioritized' | undefined

    return (
      <ClientDetailPage
        clientId={clientId}
        initialTab={initialTab}
        initialRoutingMode={initialRoutingMode}
        onBack={() => onTabChange?.('clients', 'list')}
      />
    )
  }

  return (
    <div className="space-y-6">
      {/* Metrics Overview */}
      {!loading && trackedApiKeys.length > 0 && (
        <ComparisonPanel
          title="Client Usage Overview"
          compareType="api_keys"
          ids={trackedApiKeys}
          metricOptions={[
            { id: 'requests', label: 'Requests' },
            { id: 'cost', label: 'Cost' },
            { id: 'tokens', label: 'Tokens' },
          ]}
          defaultMetric="requests"
          defaultTimeRange="day"
          refreshTrigger={refreshKey}
        />
      )}

      <Card>
        <div className="mb-6 flex justify-between items-start">
          <div>
            <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100">Clients</h2>
            <p className="text-sm text-gray-500 dark:text-gray-400 mt-1">
              Manage API clients for LLM and MCP access
            </p>
          </div>
          <Button onClick={() => setShowCreateModal(true)}>
            Create Client
          </Button>
        </div>

        {loading ? (
          <div className="text-center py-12 text-gray-500 dark:text-gray-400">Loading clients...</div>
        ) : clients.length === 0 ? (
          <div className="text-center py-12">
            <p className="text-gray-400 dark:text-gray-500 mb-4">No clients yet</p>
            <Button onClick={() => setShowCreateModal(true)}>
              Create Your First Client
            </Button>
          </div>
        ) : (
          <>
            <div className="space-y-2">
              {clients.map((client) => (
                <div
                  key={client.id}
                  onClick={() => onTabChange?.('clients', client.client_id)}
                  className="bg-gray-50 dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg p-4 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors cursor-pointer"
                >
                  <div className="flex items-center justify-between">
                    <div>
                      <h3 className="text-base font-semibold text-gray-900 dark:text-gray-100">{client.name}</h3>
                      <p className="text-sm text-gray-500 dark:text-gray-400 font-mono mt-0.5">{maskSecret(client.client_id)}</p>
                    </div>
                    <div className="flex items-center gap-2">
                      <Badge variant={client.enabled ? 'success' : 'error'}>
                        {client.enabled ? 'Enabled' : 'Disabled'}
                      </Badge>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          </>
        )}
      </Card>

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
          <div className="bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-700 rounded p-4">
            <p className="text-yellow-800 dark:text-yellow-200 text-sm">
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
                <p className="text-sm text-gray-400 dark:text-gray-500 mt-1">
                  Use this as: <code className="bg-gray-100 dark:bg-gray-800 px-1 text-gray-900 dark:text-gray-100">Authorization: Bearer {maskSecret(newClientSecret)}</code>
                </p>
              </div>
            </>
          )}

          <div className="bg-gray-100 dark:bg-gray-800 rounded p-4">
            <h4 className="font-medium text-gray-900 dark:text-gray-100 mb-2">Next Steps:</h4>
            <ol className="list-decimal list-inside space-y-1 text-sm text-gray-600 dark:text-gray-300">
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
