import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Copy, RotateCcw, Server } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import { Alert, AlertDescription } from "@/components/ui/alert"

interface ServerConfig {
  host: string
  port: number
  actual_port?: number
  enable_cors: boolean
}

interface NetworkInterface {
  name: string
  ip: string
  is_loopback: boolean
}

export function ServerTab() {
  const [config, setConfig] = useState<ServerConfig>({
    host: "127.0.0.1",
    port: 3625,
    enable_cors: true,
  })
  const [editConfig, setEditConfig] = useState<ServerConfig>(config)
  const [isUpdating, setIsUpdating] = useState(false)
  const [isRestarting, setIsRestarting] = useState(false)
  const [networkInterfaces, setNetworkInterfaces] = useState<NetworkInterface[]>([])
  const [hasUnsavedChanges, setHasUnsavedChanges] = useState(false)
  const [executablePath, setExecutablePath] = useState<string>("")

  useEffect(() => {
    loadConfig()
    loadNetworkInterfaces()
    loadExecutablePath()
  }, [])

  useEffect(() => {
    const changed =
      editConfig.host !== config.host || editConfig.port !== config.port
    setHasUnsavedChanges(changed)
  }, [editConfig, config])

  const loadConfig = async () => {
    try {
      const serverConfig = await invoke<ServerConfig>("get_server_config")
      setConfig(serverConfig)
      setEditConfig(serverConfig)
    } catch (error) {
      console.error("Failed to load server config:", error)
    }
  }

  const loadNetworkInterfaces = async () => {
    try {
      const interfaces = await invoke<NetworkInterface[]>("get_network_interfaces")
      setNetworkInterfaces(interfaces)
    } catch (error) {
      console.error("Failed to load network interfaces:", error)
    }
  }

  const loadExecutablePath = async () => {
    try {
      const path = await invoke<string>("get_executable_path")
      setExecutablePath(path)
    } catch (error) {
      console.error("Failed to load executable path:", error)
    }
  }

  const updateConfig = async (e: React.FormEvent) => {
    e.preventDefault()
    setIsUpdating(true)

    try {
      await invoke("update_server_config", {
        host: editConfig.host,
        port: editConfig.port,
        enableCors: true,
      })

      await invoke("restart_server")
      await new Promise((resolve) => setTimeout(resolve, 1500))
      await loadConfig()

      toast.success("Server configuration updated and restarted")
    } catch (error: any) {
      console.error("Failed to update server config:", error)
      toast.error(`Failed to update: ${error.message || error}`)
    } finally {
      setIsUpdating(false)
    }
  }

  const restartServer = async () => {
    setIsRestarting(true)

    try {
      await invoke("restart_server")
      await new Promise((resolve) => setTimeout(resolve, 1500))
      await loadConfig()
      toast.success("Server restarted successfully")
    } catch (error: any) {
      console.error("Failed to restart server:", error)
      toast.error(`Failed to restart: ${error.message || error}`)
    } finally {
      setIsRestarting(false)
    }
  }

  const copyToClipboard = (text: string) => {
    navigator.clipboard.writeText(text)
    toast.success("Copied to clipboard")
  }

  const serverUrl = `http://${config.host}:${config.actual_port ?? config.port}`

  return (
    <div className="space-y-6">
      {/* Server Status */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-2">
            <Server className="h-4 w-4" />
            Server Status
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center justify-between">
            <div className="space-y-1">
              <p className="text-xs text-muted-foreground">OpenAI & MCP Endpoint</p>
              <div className="flex items-center gap-2">
                <code className="text-sm font-mono bg-muted px-2 py-1 rounded">
                  {serverUrl}
                </code>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => copyToClipboard(serverUrl)}
                >
                  <Copy className="h-3 w-3" />
                </Button>
              </div>
            </div>
            <Button
              variant="outline"
              size="sm"
              onClick={restartServer}
              disabled={isRestarting}
            >
              <RotateCcw className="h-3 w-3 mr-1" />
              {isRestarting ? "Restarting..." : "Restart"}
            </Button>
          </div>

          {config.actual_port && config.actual_port !== config.port && (
            <Alert>
              <AlertDescription className="text-xs">
                Port {config.port} was already in use. Server is running on port{" "}
                {config.actual_port} instead.
              </AlertDescription>
            </Alert>
          )}
        </CardContent>
      </Card>

      {/* MCP Client Connection Instructions */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">External MCP Client Connection</CardTitle>
          <CardDescription>
            Connect external MCP clients (Claude Desktop, Cursor, VS Code) to LocalRouter
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {/* HTTP/SSE Connection */}
          <div className="p-3 bg-muted rounded-lg space-y-2">
            <p className="text-xs font-medium">HTTP/SSE Endpoint</p>
            <code className="text-xs font-mono block">
              http://{config.host}:{config.actual_port ?? config.port}
            </code>
            <p className="text-xs text-muted-foreground">
              Include <code className="bg-background px-1 rounded">Authorization: Bearer &lt;client_secret&gt;</code> header
            </p>
          </div>

          {/* STDIO Connection */}
          <div className="p-3 bg-blue-500/10 border border-blue-500/20 rounded-lg space-y-2">
            <p className="text-xs font-medium">STDIO Bridge (Recommended)</p>
            <p className="text-xs text-muted-foreground">
              Most MCP clients use STDIO transport. Configure with:
            </p>
            <pre className="text-xs font-mono bg-background p-2 rounded overflow-x-auto">
{`{
  "mcpServers": {
    "localrouter": {
      "command": "${executablePath || "/path/to/localrouter"}",
      "args": ["--mcp-bridge", "--client-id", "your_client_id"],
      "env": {
        "LOCALROUTER_CLIENT_SECRET": "your_secret_here"
      }
    }
  }
}`}
            </pre>
          </div>
        </CardContent>
      </Card>

      {/* Server Configuration */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">Network Settings</CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={updateConfig} className="space-y-4">
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="network-interface">Network Interface</Label>
                <Select
                  value={editConfig.host}
                  onValueChange={(value) =>
                    setEditConfig({ ...editConfig, host: value })
                  }
                >
                  <SelectTrigger id="network-interface">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {networkInterfaces.map((iface) => (
                      <SelectItem key={iface.ip} value={iface.ip}>
                        {iface.name} ({iface.ip})
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-2">
                <Label htmlFor="port">Port</Label>
                <Input
                  id="port"
                  type="number"
                  value={editConfig.port}
                  onChange={(e) =>
                    setEditConfig({ ...editConfig, port: parseInt(e.target.value) })
                  }
                />
              </div>
            </div>

            <div className="p-3 bg-blue-500/10 border border-blue-500/20 rounded-lg">
              <p className="text-xs text-blue-600 dark:text-blue-400">
                <strong>CORS:</strong> Cross-Origin Resource Sharing is enabled by default.
              </p>
            </div>

            {hasUnsavedChanges && (
              <Alert>
                <AlertDescription className="text-xs">
                  You have unsaved changes. Click "Save & Restart" to apply.
                </AlertDescription>
              </Alert>
            )}

            <div className="flex gap-2">
              <Button type="submit" size="sm" disabled={isUpdating || !hasUnsavedChanges}>
                {isUpdating ? "Updating..." : "Save & Restart"}
              </Button>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => setEditConfig(config)}
                disabled={!hasUnsavedChanges}
              >
                Reset
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>

    </div>
  )
}
