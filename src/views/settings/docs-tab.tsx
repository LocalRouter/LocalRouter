
import { useState, useEffect, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { RefreshCw } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Badge } from "@/components/ui/Badge"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import "rapidoc"

// Declare RapiDoc web component for TypeScript
declare global {
  namespace JSX {
    interface IntrinsicElements {
      "rapi-doc": any
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

export function DocsTab() {
  const [spec, setSpec] = useState<string>("")
  const [config, setConfig] = useState<ServerConfig | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [clients, setClients] = useState<Client[]>([])
  const [selectedClientId, setSelectedClientId] = useState<string>("")
  const [accessToken, setAccessToken] = useState<string>("")
  const rapiDocRef = useRef<any>(null)

  useEffect(() => {
    loadServerConfig()
    loadOpenAPISpec()
    loadClients()
  }, [])

  useEffect(() => {
    if (rapiDocRef.current && spec) {
      rapiDocRef.current.loadSpec(JSON.parse(spec))
    }
  }, [spec])

  useEffect(() => {
    if (rapiDocRef.current && config) {
      const port = config.actual_port ?? config.port ?? 3625
      const baseUrl = `http://${config.host ?? "127.0.0.1"}:${port}`
      rapiDocRef.current.setAttribute("server-url", baseUrl)
    }
  }, [config])

  const loadServerConfig = async () => {
    try {
      const serverConfig = await invoke<ServerConfig>("get_server_config")
      setConfig(serverConfig)
    } catch (err: any) {
      console.error("Failed to load server config:", err)
      setError(`Failed to load server config: ${err.message || err}`)
    }
  }

  const loadClients = async () => {
    try {
      const clientList = await invoke<Client[]>("list_clients")
      const enabledClients = clientList.filter((c) => c.enabled)
      setClients(enabledClients)

      if (enabledClients.length > 0 && !selectedClientId) {
        const firstClient = enabledClients[0]
        setSelectedClientId(firstClient.id)
        await getTokenForClient(firstClient.id, enabledClients)
      }
    } catch (err: any) {
      console.error("Failed to load clients:", err)
    }
  }

  const getTokenForClient = async (clientId: string, clientList?: Client[]) => {
    try {
      const clientsToSearch = clientList || clients
      const client = clientsToSearch.find((c) => c.id === clientId)
      if (!client) {
        console.error("Client not found:", clientId)
        return
      }

      const clientSecret = await invoke<string>("get_client_value", {
        id: client.client_id,
      })
      setAccessToken(clientSecret)
    } catch (err: any) {
      console.error("Failed to get client secret:", err)
      setError(`Failed to get client token: ${err.message || err}`)
    }
  }

  const handleClientChange = async (clientId: string) => {
    setSelectedClientId(clientId)
    await getTokenForClient(clientId)
  }

  const loadOpenAPISpec = async () => {
    try {
      setIsLoading(true)
      setError(null)
      const openApiSpec = await invoke<string>("get_openapi_spec")
      setSpec(openApiSpec)
    } catch (err: any) {
      console.error("Failed to load OpenAPI spec:", err)
      setError(`Failed to load OpenAPI spec: ${err.message || err}`)
    } finally {
      setIsLoading(false)
    }
  }

  const handleRefresh = () => {
    loadServerConfig()
    loadClients()
    loadOpenAPISpec()
  }

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-96">
        <div className="text-center space-y-4">
          <div className="text-6xl animate-spin">⚙️</div>
          <p className="text-muted-foreground">Loading OpenAPI specification...</p>
        </div>
      </div>
    )
  }

  if (error || !spec) {
    return (
      <div className="flex items-center justify-center h-96">
        <div className="text-center space-y-4 max-w-md">
          <div className="text-6xl">⚠️</div>
          <p className="text-red-500 font-medium">
            {error || "Failed to load OpenAPI specification"}
          </p>
          <Button variant="outline" onClick={handleRefresh}>
            <RefreshCw className="h-4 w-4 mr-2" />
            Retry
          </Button>
        </div>
      </div>
    )
  }

  const port = config?.actual_port ?? config?.port ?? 3625
  const baseUrl = `http://${config?.host ?? "127.0.0.1"}:${port}`

  return (
    <div className="h-[calc(100vh-12rem)] flex flex-col rounded-lg border overflow-hidden">
      {/* Header */}
      <div className="p-3 border-b bg-muted/50 flex items-center justify-between flex-shrink-0">
        <div className="flex items-center gap-3">
          <div className="text-sm text-muted-foreground">
            Server:{" "}
            <span className="font-mono font-medium text-blue-600">{baseUrl}</span>
          </div>
          {accessToken && (
            <div className="flex items-center gap-1">
              <div className="w-2 h-2 bg-green-500 rounded-full" />
              <span className="text-xs text-green-600">Authenticated</span>
            </div>
          )}
        </div>

        <div className="flex items-center gap-2">
          {clients.length > 0 ? (
            <Select value={selectedClientId} onValueChange={handleClientChange}>
              <SelectTrigger className="w-48 h-8 text-xs">
                <SelectValue placeholder="Select client" />
              </SelectTrigger>
              <SelectContent>
                {clients.map((client) => (
                  <SelectItem key={client.id} value={client.id}>
                    {client.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          ) : (
            <span className="text-xs text-muted-foreground">
              No clients available
            </span>
          )}
          <Button variant="ghost" size="sm" onClick={handleRefresh}>
            <RefreshCw className="h-3 w-3" />
          </Button>
        </div>
      </div>

      {/* RapiDoc API Reference */}
      <div className="flex-1 overflow-auto bg-white dark:bg-gray-900">
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
          api-key-value={accessToken ? `Bearer ${accessToken}` : ""}
          api-key-location="header"
          server-url={baseUrl}
          style={{ width: "100%", height: "100%" }}
        />
      </div>

      {/* Dev-only notice */}
      <div className="p-2 border-t bg-yellow-500/10 text-center flex-shrink-0">
        <Badge variant="outline" className="text-yellow-600 border-yellow-500/50">
          Development Only
        </Badge>
        <span className="text-xs text-muted-foreground ml-2">
          This tab is only visible in development builds
        </span>
      </div>
    </div>
  )
}
