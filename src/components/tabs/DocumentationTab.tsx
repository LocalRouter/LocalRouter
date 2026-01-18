import { useState, useEffect, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import 'rapidoc'

// Declare RapiDoc web component for TypeScript
declare global {
  namespace JSX {
    interface IntrinsicElements {
      'rapi-doc': any
    }
  }
}

interface ServerConfig {
  host: string
  port: number
  actual_port?: number
  enable_cors: boolean
}

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
}

export default function DocumentationTab() {
  const [spec, setSpec] = useState<string>('')
  const [config, setConfig] = useState<ServerConfig | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [clients, setClients] = useState<Client[]>([])
  const [selectedClientId, setSelectedClientId] = useState<string>('')
  const [accessToken, setAccessToken] = useState<string>('')
  const rapiDocRef = useRef<any>(null)

  useEffect(() => {
    loadServerConfig()
    loadOpenAPISpec()
    loadClients()
  }, [])

  // Update RapiDoc spec when it changes
  useEffect(() => {
    if (rapiDocRef.current && spec) {
      rapiDocRef.current.loadSpec(JSON.parse(spec))
    }
  }, [spec])

  // Update RapiDoc server URL when config changes
  useEffect(() => {
    if (rapiDocRef.current && config) {
      const port = config.actual_port ?? config.port ?? 3625
      const baseUrl = `http://${config.host ?? '127.0.0.1'}:${port}`
      rapiDocRef.current.setAttribute('server-url', baseUrl)
      console.log('Updated RapiDoc server-url to:', baseUrl)
    }
  }, [config])

  const loadServerConfig = async () => {
    try {
      const serverConfig = await invoke<ServerConfig>('get_server_config')
      setConfig(serverConfig)
    } catch (err: any) {
      console.error('Failed to load server config:', err)
      setError(`Failed to load server config: ${err.message || err}`)
    }
  }

  const loadClients = async () => {
    try {
      const clientList = await invoke<Client[]>('list_clients')
      const enabledClients = clientList.filter((c) => c.enabled)
      setClients(enabledClients)

      // Auto-select first client and get token
      if (enabledClients.length > 0 && !selectedClientId) {
        const firstClient = enabledClients[0]
        setSelectedClientId(firstClient.id)
        await getTokenForClient(firstClient.id, enabledClients)
      }
    } catch (err: any) {
      console.error('Failed to load clients:', err)
    }
  }

  const getTokenForClient = async (clientId: string, clientList?: Client[]) => {
    try {
      const clientsToSearch = clientList || clients
      const client = clientsToSearch.find(c => c.id === clientId)
      if (!client) {
        console.error('Client not found:', clientId)
        return
      }

      // Get the client secret from keychain (this IS the bearer token for clients)
      // Note: get_client_value expects the client_id (public identifier), not the internal id
      const clientSecret = await invoke<string>('get_client_value', { id: client.client_id })

      console.log('Client secret obtained successfully')
      setAccessToken(clientSecret)
    } catch (err: any) {
      console.error('Failed to get client secret:', err)
      setError(`Failed to get client token: ${err.message || err}`)
    }
  }

  const handleClientChange = async (event: React.ChangeEvent<HTMLSelectElement>) => {
    const clientId = event.target.value
    setSelectedClientId(clientId)
    await getTokenForClient(clientId)
  }

  const loadOpenAPISpec = async () => {
    try {
      setIsLoading(true)
      setError(null)
      const openApiSpec = await invoke<string>('get_openapi_spec')
      setSpec(openApiSpec)
    } catch (err: any) {
      console.error('Failed to load OpenAPI spec:', err)
      setError(`Failed to load OpenAPI spec: ${err.message || err}`)
    } finally {
      setIsLoading(false)
    }
  }


  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="text-center space-y-4">
          <div className="text-6xl animate-spin">⚙️</div>
          <p className="text-gray-600">Loading OpenAPI specification...</p>
        </div>
      </div>
    )
  }

  if (error || !spec) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="text-center space-y-4 max-w-md">
          <div className="text-6xl">⚠️</div>
          <p className="text-red-600 font-semibold">{error || 'Failed to load OpenAPI specification'}</p>
        </div>
      </div>
    )
  }

  const port = config?.actual_port ?? config?.port ?? 3625
  const baseUrl = `http://${config?.host ?? '127.0.0.1'}:${port}`

  console.log('Server config:', config)
  console.log('Using baseUrl:', baseUrl)

  return (
    <div className="h-full flex flex-col relative">
      {/* Header */}
      <div className="p-4 border-b bg-white flex-shrink-0 space-y-3">
        <div className="flex items-center justify-between">
          <div className="text-sm text-gray-600">
            Server: <span className="font-mono font-semibold text-blue-600">{baseUrl}</span>
          </div>
          <button
            onClick={() => { loadServerConfig(); loadClients(); }}
            className="px-3 py-1 text-sm bg-gray-100 hover:bg-gray-200 rounded-md"
          >
            Refresh
          </button>
        </div>
        {clients.length > 0 ? (
          <div className="flex items-center gap-4">
            <div className="flex items-center gap-2">
              <label className="text-sm font-medium text-gray-700">Client:</label>
              <select
                value={selectedClientId}
                onChange={handleClientChange}
                className="px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
              >
                {clients.map((client) => (
                  <option key={client.id} value={client.id}>
                    {client.name}
                  </option>
                ))}
              </select>
            </div>
            {accessToken && (
              <div className="flex items-center gap-2 text-sm text-green-600">
                <span className="w-2 h-2 bg-green-500 rounded-full"></span>
                <span>Authenticated</span>
              </div>
            )}
          </div>
        ) : (
          <div className="text-sm text-gray-500">No clients available. Create a client to test the API.</div>
        )}
      </div>

      {/* RapiDoc API Reference */}
      <div className="flex-1 overflow-auto">
        <rapi-doc
          ref={rapiDocRef}
          theme="light"
          bg-color="#ffffff"
          text-color="#1f2937"
          primary-color="#3b82f6"
          render-style="view"
          layout="row"
          show-header="false"
          show-info="true"
          allow-authentication="false"
          allow-server-selection="false"
          allow-api-list-style-selection="false"
          api-key-name="Authorization"
          api-key-value={accessToken ? `Bearer ${accessToken}` : ''}
          api-key-location="header"
          server-url={baseUrl}
          style={{ width: '100%', height: '100%' }}
        />
      </div>
    </div>
  )
}
