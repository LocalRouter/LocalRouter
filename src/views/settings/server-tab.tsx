import { gatewayPortExplanation } from '@/utils/gateway-connection'
import { listenSafe } from '@/hooks/useTauriListener'
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
import type {
  ServerConfigInfo,
  SetMaxCodingSessionsParams,
  UpdateServerConfigParams,
} from "@/types/tauri-commands"

type ServerConfig = ServerConfigInfo

const MIB = 1024 * 1024
const MIN_BODY_LIMIT_MIB = 1
const MAX_BODY_LIMIT_MIB = 256
// Bounds of server.large_body_* (validated again by the backend).
const MIN_LARGE_THRESHOLD_MIB = 1
const MAX_LARGE_THRESHOLD_MIB = 256
const MIN_LARGE_BUDGET_MIB = 16
const MAX_LARGE_BUDGET_MIB = 8192
const MAX_LARGE_WAIT_SECS = 60

interface LargeBodyEdit {
  thresholdMib: number
  budgetMib: number
  waitSecs: number
}

function largeBodyEdit(config: ServerConfigInfo): LargeBodyEdit {
  return {
    thresholdMib: Math.round(config.large_body_threshold_bytes / MIB),
    budgetMib: Math.round(config.large_body_budget_bytes / MIB),
    waitSecs: config.large_body_wait_ms / 1000,
  }
}

interface NetworkInterface {
  name: string
  ip: string
  is_loopback: boolean
}

interface ServerTabProps {
  hideResourceLimits?: boolean
}

export function ServerTab({ hideResourceLimits }: ServerTabProps = {}) {
  const [config, setConfig] = useState<ServerConfig>({
    host: "127.0.0.1",
    port: 3625,
    enable_cors: true,
    max_request_body_bytes: 16 * MIB,
    large_body_threshold_bytes: 8 * MIB,
    large_body_budget_bytes: 512 * MIB,
    large_body_wait_ms: 3000,
  })
  const [editConfig, setEditConfig] = useState<ServerConfig>(config)
  const [isUpdating, setIsUpdating] = useState(false)
  const [isRestarting, setIsRestarting] = useState(false)
  const [networkInterfaces, setNetworkInterfaces] = useState<NetworkInterface[]>([])
  const [hasUnsavedChanges, setHasUnsavedChanges] = useState(false)
  const [executablePath, setExecutablePath] = useState<string>("")
  const [maxCodingSessions, setMaxCodingSessions] = useState<number>(10)
  const [editMaxCodingSessions, setEditMaxCodingSessions] = useState<number>(10)
  const [editBodyLimitMib, setEditBodyLimitMib] = useState<number>(16)
  const [editLargeBody, setEditLargeBody] = useState<LargeBodyEdit>({
    thresholdMib: 8,
    budgetMib: 512,
    waitSecs: 3,
  })
  useEffect(() => {
    loadConfig()
    loadNetworkInterfaces()
    loadExecutablePath()
    loadMaxCodingSessions()
    const listeners = ['server-status-changed', 'config-changed'].map(event => listenSafe(event, () => { void invoke<ServerConfig>('get_server_config').then(setConfig).catch(console.error) }))
    return () => listeners.forEach(l => l.cleanup())
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
      setEditBodyLimitMib(Math.round(serverConfig.max_request_body_bytes / MIB))
      setEditLargeBody(largeBodyEdit(serverConfig))
    } catch (error) {
      console.error("Failed to load server config:", error)
    }
  }

  // Reload after saving a request limit without discarding unsaved host or
  // port edits in the form above.
  const reloadLimits = async () => {
    const serverConfig = await invoke<ServerConfig>("get_server_config")
    setConfig(serverConfig)
    setEditBodyLimitMib(Math.round(serverConfig.max_request_body_bytes / MIB))
    setEditLargeBody(largeBodyEdit(serverConfig))
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

  const loadMaxCodingSessions = async () => {
    try {
      const max = await invoke<number>("get_max_coding_sessions")
      setMaxCodingSessions(max)
      setEditMaxCodingSessions(max)
    } catch (error) {
      console.error("Failed to load max coding sessions:", error)
    }
  }

  const saveMaxCodingSessions = async () => {
    try {
      await invoke("set_max_coding_sessions", { maxSessions: editMaxCodingSessions } satisfies SetMaxCodingSessionsParams)
      setMaxCodingSessions(editMaxCodingSessions)
      toast.success("Max coding sessions updated")
    } catch (error: any) {
      toast.error(`Failed to update: ${error.message || error}`)
    }
  }

  const bodyLimitMib = Math.round(config.max_request_body_bytes / MIB)
  const bodyLimitValid =
    Number.isInteger(editBodyLimitMib) &&
    editBodyLimitMib >= MIN_BODY_LIMIT_MIB &&
    editBodyLimitMib <= MAX_BODY_LIMIT_MIB

  const saveBodyLimit = async () => {
    try {
      await invoke("update_server_config", {
        maxRequestBodyBytes: editBodyLimitMib * MIB,
      } satisfies UpdateServerConfigParams)
      await reloadLimits()
      toast.success("Request body limit updated")
    } catch (error: any) {
      toast.error(`Failed to update: ${error.message || error}`)
    }
  }

  const largeBody = largeBodyEdit(config)
  const largeBodyChanged =
    editLargeBody.thresholdMib !== largeBody.thresholdMib ||
    editLargeBody.budgetMib !== largeBody.budgetMib ||
    editLargeBody.waitSecs !== largeBody.waitSecs
  const largeBodyValid =
    Number.isInteger(editLargeBody.thresholdMib) &&
    editLargeBody.thresholdMib >= MIN_LARGE_THRESHOLD_MIB &&
    editLargeBody.thresholdMib <= MAX_LARGE_THRESHOLD_MIB &&
    Number.isInteger(editLargeBody.budgetMib) &&
    editLargeBody.budgetMib >= MIN_LARGE_BUDGET_MIB &&
    editLargeBody.budgetMib <= MAX_LARGE_BUDGET_MIB &&
    editLargeBody.budgetMib >= editLargeBody.thresholdMib &&
    Number.isFinite(editLargeBody.waitSecs) &&
    editLargeBody.waitSecs >= 0 &&
    editLargeBody.waitSecs <= MAX_LARGE_WAIT_SECS

  const saveLargeBody = async () => {
    try {
      await invoke("update_server_config", {
        largeBodyThresholdBytes: editLargeBody.thresholdMib * MIB,
        largeBodyBudgetBytes: editLargeBody.budgetMib * MIB,
        largeBodyWaitMs: Math.round(editLargeBody.waitSecs * 1000),
      } satisfies UpdateServerConfigParams)
      await reloadLimits()
      toast.success("Large request settings updated")
    } catch (error: any) {
      toast.error(`Failed to update: ${error.message || error}`)
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
      {gatewayPortExplanation(config) && <p className="text-sm text-muted-foreground">{gatewayPortExplanation(config)}</p>}
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

      {/* Request Limits */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">Request Limits</CardTitle>
          <CardDescription>
            Largest request the API accepts on its JSON endpoints (chat, completions, responses, messages, embeddings)
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="max-request-body">Max Request Body (MiB)</Label>
              <Input
                id="max-request-body"
                type="number"
                min={MIN_BODY_LIMIT_MIB}
                max={MAX_BODY_LIMIT_MIB}
                value={editBodyLimitMib}
                onChange={(e) => setEditBodyLimitMib(parseInt(e.target.value) || 0)}
              />
              <p className="text-xs text-muted-foreground">
                Between {MIN_BODY_LIMIT_MIB} and {MAX_BODY_LIMIT_MIB} MiB; applies to the next request without a restart.
                Larger requests are refused with HTTP 413. Requests are held in memory while they are processed.
                Audio, image edit and file uploads keep their own limits.
              </p>
            </div>
          </div>

          {editBodyLimitMib !== bodyLimitMib && (
            <div className="flex gap-2">
              <Button size="sm" onClick={saveBodyLimit} disabled={!bodyLimitValid}>
                Save
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => setEditBodyLimitMib(bodyLimitMib)}
              >
                Reset
              </Button>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Large Request Memory */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">Large Request Memory</CardTitle>
          <CardDescription>
            How much memory large requests may hold at once; requests above the threshold wait for room
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-4 sm:grid-cols-3">
            <div className="space-y-2">
              <Label htmlFor="large-body-threshold">Large Request Threshold (MiB)</Label>
              <Input
                id="large-body-threshold"
                type="number"
                min={MIN_LARGE_THRESHOLD_MIB}
                max={MAX_LARGE_THRESHOLD_MIB}
                value={editLargeBody.thresholdMib}
                onChange={(e) =>
                  setEditLargeBody({ ...editLargeBody, thresholdMib: parseInt(e.target.value) || 0 })
                }
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="large-body-budget">Memory Budget (MiB)</Label>
              <Input
                id="large-body-budget"
                type="number"
                min={MIN_LARGE_BUDGET_MIB}
                max={MAX_LARGE_BUDGET_MIB}
                value={editLargeBody.budgetMib}
                onChange={(e) =>
                  setEditLargeBody({ ...editLargeBody, budgetMib: parseInt(e.target.value) || 0 })
                }
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="large-body-wait">Wait for Room (seconds)</Label>
              <Input
                id="large-body-wait"
                type="number"
                min={0}
                max={MAX_LARGE_WAIT_SECS}
                step={0.1}
                value={editLargeBody.waitSecs}
                onChange={(e) =>
                  setEditLargeBody({ ...editLargeBody, waitSecs: parseFloat(e.target.value) || 0 })
                }
              />
            </div>
          </div>
          <p className="text-xs text-muted-foreground">
            Requests larger than the threshold ({MIN_LARGE_THRESHOLD_MIB}–{MAX_LARGE_THRESHOLD_MIB} MiB) reserve
            their size in the memory budget ({MIN_LARGE_BUDGET_MIB}–{MAX_LARGE_BUDGET_MIB} MiB, at least the
            threshold) until their response ends. A request that finds no room within the wait
            (0–{MAX_LARGE_WAIT_SECS} s) is refused with HTTP 503 and asked to retry. A request larger than the
            whole budget runs alone. Applies to the next request without a restart.
          </p>

          {largeBodyChanged && (
            <div className="flex gap-2">
              <Button size="sm" onClick={saveLargeBody} disabled={!largeBodyValid}>
                Save
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => setEditLargeBody(largeBody)}
              >
                Reset
              </Button>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Resource Limits */}
      {!hideResourceLimits && (
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">Resource Limits</CardTitle>
          <CardDescription>
            Control resource usage for server features
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="max-coding-sessions">Max Concurrent Coding Agent Sessions</Label>
              <Input
                id="max-coding-sessions"
                type="number"
                min={0}
                value={editMaxCodingSessions}
                onChange={(e) => setEditMaxCodingSessions(parseInt(e.target.value) || 0)}
              />
              <p className="text-xs text-muted-foreground">
                Maximum number of coding agent sessions running at the same time. Set to 0 for unlimited.
              </p>
            </div>
          </div>

          {editMaxCodingSessions !== maxCodingSessions && (
            <div className="flex gap-2">
              <Button size="sm" onClick={saveMaxCodingSessions}>
                Save
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => setEditMaxCodingSessions(maxCodingSessions)}
              >
                Reset
              </Button>
            </div>
          )}
        </CardContent>
      </Card>
      )}

    </div>
  )
}
