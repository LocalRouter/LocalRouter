/**
 * HowToConnect Component
 *
 * Displays connection instructions for LLM and MCP with tabs for different methods.
 * Used in both client detail view and creation wizard.
 */

import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Copy, Check, Eye, RefreshCw, Cpu, Terminal, Globe, Key, FileJson, Loader2 } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { Label } from "@/components/ui/label"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog"

interface ServerConfig {
  host: string
  port: number
  actual_port?: number
  enable_cors: boolean
}

interface HowToConnectProps {
  clientId: string
  clientUuid: string
  secret: string | null
  loadingSecret?: boolean
  showRotateCredentials?: boolean
  onRotate?: () => void
  rotating?: boolean
  className?: string
}

// Helper component for copyable code blocks
function CopyableCode({
  value,
  masked = false,
  showValue = true,
  onToggleShow,
  loading = false,
}: {
  value: string
  masked?: boolean
  showValue?: boolean
  onToggleShow?: () => void
  loading?: boolean
}) {
  const [copied, setCopied] = useState(false)
  const maskedValue = "••••••••••••••••••••••••••••••••"

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(value)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
      toast.success("Copied to clipboard")
    } catch {
      toast.error("Failed to copy")
    }
  }

  return (
    <div className="flex items-center gap-2">
      <code className="flex-1 p-3 text-sm bg-muted rounded-md font-mono break-all">
        {loading ? (
          <span className="flex items-center gap-2 text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Loading...
          </span>
        ) : masked ? (showValue ? value : maskedValue) : value}
      </code>
      {masked && onToggleShow && !showValue && (
        <Button
          variant="outline"
          size="icon"
          onClick={onToggleShow}
          title="Show"
          disabled={loading || !value}
        >
          <Eye className="h-4 w-4" />
        </Button>
      )}
      <Button
        variant="outline"
        size="icon"
        onClick={handleCopy}
        title="Copy to clipboard"
        disabled={loading || !value}
      >
        {copied ? <Check className="h-4 w-4 text-green-500" /> : <Copy className="h-4 w-4" />}
      </Button>
    </div>
  )
}

// Helper component for copyable multi-line code blocks
function CopyableCodeBlock({ value, copyValue, className = "" }: { value: string; copyValue?: string; className?: string }) {
  const [copied, setCopied] = useState(false)

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(copyValue ?? value)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
      toast.success("Copied to clipboard")
    } catch {
      toast.error("Failed to copy")
    }
  }

  return (
    <div className={`relative group ${className}`}>
      <pre className="text-xs font-mono bg-muted p-3 rounded-lg overflow-x-auto whitespace-pre">
        {value}
      </pre>
      <Button
        variant="ghost"
        size="sm"
        className="absolute top-2 right-2 opacity-0 group-hover:opacity-100 transition-opacity h-7 px-2"
        onClick={handleCopy}
      >
        {copied ? <Check className="h-3 w-3 text-green-500" /> : <Copy className="h-3 w-3" />}
      </Button>
    </div>
  )
}

export function HowToConnect({
  clientId,
  clientUuid,
  secret,
  loadingSecret = false,
  showRotateCredentials = true,
  onRotate,
  rotating = false,
  className,
}: HowToConnectProps) {
  const [showSecret, setShowSecret] = useState(false)
  const [serverConfig, setServerConfig] = useState<ServerConfig | null>(null)
  const [executablePath, setExecutablePath] = useState<string>("")
  const [models, setModels] = useState<Array<{ id: string }>>([])

  // Fetch server config and executable path
  useEffect(() => {
    const fetchServerConfig = async () => {
      try {
        const config = await invoke<ServerConfig>("get_server_config")
        setServerConfig(config)
      } catch (error) {
        console.error("Failed to fetch server config:", error)
      }
    }
    const fetchExecutablePath = async () => {
      try {
        const path = await invoke<string>("get_executable_path")
        setExecutablePath(path)
      } catch (error) {
        console.error("Failed to fetch executable path:", error)
      }
    }
    fetchServerConfig()
    fetchExecutablePath()
  }, [])

  // Fetch models filtered by client's strategy via the real API endpoint
  useEffect(() => {
    if (!secret || !serverConfig) return
    const port = serverConfig.actual_port ?? serverConfig.port ?? 3625
    const host = serverConfig.host ?? "127.0.0.1"
    const url = `http://${host}:${port}/v1/models`
    const fetchModels = async () => {
      try {
        const res = await fetch(url, {
          headers: { Authorization: `Bearer ${secret}` },
        })
        if (!res.ok) return
        const body = await res.json()
        setModels(body.data ?? [])
      } catch (error) {
        console.error("Failed to fetch models:", error)
      }
    }
    fetchModels()
  }, [secret, serverConfig])

  // Compute URLs based on server config
  const port = serverConfig?.actual_port ?? serverConfig?.port ?? 3625
  const host = serverConfig?.host ?? "127.0.0.1"
  const baseUrl = `http://${host}:${port}`

  // Binary path from the running executable
  const binaryPath = executablePath || "/path/to/localrouter"
  // Quoted version for shell usage (handles spaces in path)
  const quotedBinaryPath = `"${binaryPath}"`

  const maskedSecret = "••••••••••••••••••••••••••••••••"

  // Generate API Key JSON config
  const apiKeyJsonConfig = (masked: boolean) => JSON.stringify({
    mcpServers: {
      localrouter: {
        url: baseUrl,
        transport: "http",
        headers: {
          Authorization: `Bearer ${masked ? maskedSecret : (secret || "<your_client_secret>")}`
        }
      }
    }
  }, null, 2)

  // Generate OAuth JSON config
  const oauthJsonConfig = (masked: boolean) => JSON.stringify({
    mcpServers: {
      localrouter: {
        url: baseUrl,
        transport: "http",
        clientId: clientUuid,
        clientSecret: masked ? maskedSecret : (secret || "<your_client_secret>")
      }
    }
  }, null, 2)

  // Generate STDIO JSON config
  const stdioJsonConfig = (masked: boolean) => JSON.stringify({
    mcpServers: {
      localrouter: {
        command: binaryPath,
        args: ["--mcp-bridge", "--client-id", clientId],
        env: {
          LOCALROUTER_CLIENT_SECRET: masked ? maskedSecret : (secret || "<your_client_secret>")
        }
      }
    }
  }, null, 2)

  return (
    <Card className={className}>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <CardTitle>How to Connect</CardTitle>
            <CardDescription>
              Connect to LocalRouter using this client's credentials
            </CardDescription>
          </div>
          {showRotateCredentials && onRotate && (
            <AlertDialog>
              <AlertDialogTrigger asChild>
                <Button variant="destructive" size="sm" disabled={rotating}>
                  <RefreshCw className={`h-4 w-4 mr-2 ${rotating ? "animate-spin" : ""}`} />
                  Rotate Credentials
                </Button>
              </AlertDialogTrigger>
              <AlertDialogContent>
                <AlertDialogHeader>
                  <AlertDialogTitle>Rotate Credentials?</AlertDialogTitle>
                  <AlertDialogDescription>
                    This will generate a new client secret and invalidate the current one.
                    <strong className="block mt-2">
                      Both Model API and MCP connections using this client will stop working immediately.
                    </strong>
                    You will need to update all applications using these credentials.
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel>Cancel</AlertDialogCancel>
                  <AlertDialogAction
                    onClick={onRotate}
                    className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                  >
                    Rotate Credentials
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          )}
        </div>
      </CardHeader>
      <CardContent>
        <Tabs defaultValue="models">
          <TabsList className="mb-4 grid w-full grid-cols-2">
            <TabsTrigger value="models" className="gap-2">
              <Cpu className="h-4 w-4" />
              Models
            </TabsTrigger>
            <TabsTrigger value="mcp" className="gap-2">
              <Terminal className="h-4 w-4" />
              MCP
            </TabsTrigger>
          </TabsList>

          {/* Models Tab - OpenAI-compatible API */}
          <TabsContent value="models" className="space-y-4">
            <div className="rounded-lg border p-4 space-y-4">
              <div className="flex items-center gap-2">
                <Globe className="h-4 w-4 text-muted-foreground" />
                <span className="text-sm font-medium">HTTP (OpenAI-compatible)</span>
              </div>

              <div className="space-y-3">
                <div className="space-y-1.5">
                  <Label className="text-xs text-muted-foreground">API Base URL</Label>
                  <CopyableCode value={baseUrl} />
                </div>

                <div className="space-y-1.5">
                  <Label className="text-xs text-muted-foreground">API Key</Label>
                  <CopyableCode
                    value={secret || "Error loading secret"}
                    masked
                    showValue={showSecret}
                    onToggleShow={() => setShowSecret(true)}
                    loading={loadingSecret}
                  />
                </div>
              </div>
            </div>

            {models.length > 0 && (
              <div className="rounded-lg border p-4 space-y-3">
                <div>
                  <p className="text-sm font-medium">Available Models</p>
                  <p className="text-xs text-muted-foreground mt-1">
                    Specify the model in the <code className="text-xs bg-muted px-1 py-0.5 rounded">"model"</code> field of your request body.
                  </p>
                </div>
                <div className="max-h-48 overflow-y-auto rounded-md border">
                  <table className="w-full text-xs">
                    <thead className="bg-muted/50 sticky top-0">
                      <tr>
                        <th className="text-left p-2 font-medium text-muted-foreground">Model</th>
                      </tr>
                    </thead>
                    <tbody>
                      {models.map((model) => (
                        <tr key={model.id} className="border-t border-border/50">
                          <td className="p-2 font-mono">{model.id}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>
            )}
          </TabsContent>

          {/* MCP Tab - Three auth methods, each with Config/JSON sub-tabs */}
          <TabsContent value="mcp" className="space-y-4">
            <Tabs defaultValue="api-key">
              <TabsList className="mb-4 w-full grid grid-cols-3">
                <TabsTrigger value="api-key" className="text-xs gap-1">
                  <Key className="h-3 w-3" />
                  API Key
                </TabsTrigger>
                <TabsTrigger value="oauth" className="text-xs gap-1">
                  <Globe className="h-3 w-3" />
                  OAuth
                </TabsTrigger>
                <TabsTrigger value="stdio" className="text-xs gap-1">
                  <Terminal className="h-3 w-3" />
                  STDIO
                </TabsTrigger>
              </TabsList>

              {/* API Key */}
              <TabsContent value="api-key" className="space-y-4">
                <Tabs defaultValue="config">
                  <TabsList className="mb-3 w-full grid grid-cols-2">
                    <TabsTrigger value="config" className="text-xs gap-1">
                      <Cpu className="h-3 w-3" />
                      Config
                    </TabsTrigger>
                    <TabsTrigger value="json" className="text-xs gap-1">
                      <FileJson className="h-3 w-3" />
                      JSON
                    </TabsTrigger>
                  </TabsList>

                  <TabsContent value="config" className="space-y-4">
                    <div className="rounded-lg border p-4 space-y-4">
                      <div>
                        <p className="text-sm font-medium">HTTP with Bearer Token</p>
                        <p className="text-xs text-muted-foreground mt-1">
                          Direct authentication using the client secret as a bearer token.
                        </p>
                      </div>

                      <div className="space-y-3">
                        <div className="space-y-1.5">
                          <Label className="text-xs text-muted-foreground">Endpoint URL</Label>
                          <CopyableCode value={baseUrl} />
                        </div>

                        <div className="space-y-1.5">
                          <Label className="text-xs text-muted-foreground">API Key</Label>
                          <CopyableCode
                            value={secret || "Error loading secret"}
                            masked
                            showValue={showSecret}
                            onToggleShow={() => setShowSecret(true)}
                            loading={loadingSecret}
                          />
                        </div>
                      </div>
                    </div>
                  </TabsContent>

                  <TabsContent value="json" className="space-y-4">
                    <div className="rounded-lg border p-4 space-y-4">
                      <div>
                        <p className="text-sm font-medium">MCP JSON Configuration</p>
                        <p className="text-xs text-muted-foreground mt-1">
                          Copy this JSON to your MCP client config for API key auth.
                        </p>
                      </div>

                      <div
                        className={`rounded-lg bg-muted/50 p-3 space-y-2${!showSecret ? " cursor-pointer" : ""}`}
                        onClick={!showSecret ? () => setShowSecret(true) : undefined}
                        title={!showSecret ? "Click to reveal secret" : undefined}
                      >
                        <CopyableCodeBlock value={apiKeyJsonConfig(!showSecret)} copyValue={apiKeyJsonConfig(false)} />
                      </div>
                    </div>
                  </TabsContent>
                </Tabs>
              </TabsContent>

              {/* OAuth */}
              <TabsContent value="oauth" className="space-y-4">
                <Tabs defaultValue="config">
                  <TabsList className="mb-3 w-full grid grid-cols-2">
                    <TabsTrigger value="config" className="text-xs gap-1">
                      <Cpu className="h-3 w-3" />
                      Config
                    </TabsTrigger>
                    <TabsTrigger value="json" className="text-xs gap-1">
                      <FileJson className="h-3 w-3" />
                      JSON
                    </TabsTrigger>
                  </TabsList>

                  <TabsContent value="config" className="space-y-4">
                    <div className="rounded-lg border p-4 space-y-4">
                      <div>
                        <p className="text-sm font-medium">HTTP with OAuth 2.0</p>
                        <p className="text-xs text-muted-foreground mt-1">
                          Use OAuth client credentials flow for token-based authentication.
                        </p>
                      </div>

                      <div className="space-y-3">
                        <div className="space-y-1.5">
                          <Label className="text-xs text-muted-foreground">Endpoint URL</Label>
                          <CopyableCode value={baseUrl} />
                        </div>

                        <div className="space-y-1.5">
                          <Label className="text-xs text-muted-foreground">OAuth Token URL</Label>
                          <CopyableCode value={`${baseUrl}/oauth/token`} />
                        </div>

                        <div className="space-y-1.5">
                          <Label className="text-xs text-muted-foreground">Client ID</Label>
                          <CopyableCode value={clientUuid} />
                        </div>

                        <div className="space-y-1.5">
                          <Label className="text-xs text-muted-foreground">Client Secret</Label>
                          <CopyableCode
                            value={secret || "Error loading secret"}
                            masked
                            showValue={showSecret}
                            onToggleShow={() => setShowSecret(true)}
                            loading={loadingSecret}
                          />
                        </div>
                      </div>

                      <div
                        className={`rounded-lg bg-muted/50 p-3 space-y-2${!showSecret ? " cursor-pointer" : ""}`}
                        onClick={!showSecret ? () => setShowSecret(true) : undefined}
                        title={!showSecret ? "Click to reveal secret" : undefined}
                      >
                        <p className="text-xs font-medium">Token Exchange</p>
                        <CopyableCodeBlock
                          value={`POST ${baseUrl}/oauth/token\nContent-Type: application/x-www-form-urlencoded\n\ngrant_type=client_credentials&client_id=${clientUuid}&client_secret=${!showSecret ? maskedSecret : (secret || "<your_client_secret>")}`}
                          copyValue={`POST ${baseUrl}/oauth/token\nContent-Type: application/x-www-form-urlencoded\n\ngrant_type=client_credentials&client_id=${clientUuid}&client_secret=${secret || "<your_client_secret>"}`}
                        />
                      </div>
                    </div>
                  </TabsContent>

                  <TabsContent value="json" className="space-y-4">
                    <div className="rounded-lg border p-4 space-y-4">
                      <div>
                        <p className="text-sm font-medium">MCP JSON Configuration</p>
                        <p className="text-xs text-muted-foreground mt-1">
                          Copy this JSON to your MCP client config for OAuth auth.
                        </p>
                      </div>

                      <div
                        className={`rounded-lg bg-muted/50 p-3 space-y-2${!showSecret ? " cursor-pointer" : ""}`}
                        onClick={!showSecret ? () => setShowSecret(true) : undefined}
                        title={!showSecret ? "Click to reveal secret" : undefined}
                      >
                        <CopyableCodeBlock value={oauthJsonConfig(!showSecret)} copyValue={oauthJsonConfig(false)} />
                      </div>
                    </div>
                  </TabsContent>
                </Tabs>
              </TabsContent>

              {/* STDIO */}
              <TabsContent value="stdio" className="space-y-4">
                <Tabs defaultValue="config">
                  <TabsList className="mb-3 w-full grid grid-cols-2">
                    <TabsTrigger value="config" className="text-xs gap-1">
                      <Cpu className="h-3 w-3" />
                      Config
                    </TabsTrigger>
                    <TabsTrigger value="json" className="text-xs gap-1">
                      <FileJson className="h-3 w-3" />
                      JSON
                    </TabsTrigger>
                  </TabsList>

                  <TabsContent value="config" className="space-y-4">
                    <div className="rounded-lg border p-4 space-y-4">
                      <div>
                        <p className="text-sm font-medium">STDIO Bridge</p>
                        <p className="text-xs text-muted-foreground mt-1">
                          For clients that do not support HTTP transport, connect locally via STDIO bridge.
                        </p>
                      </div>

                      <div className="space-y-3">
                        <div className="space-y-1.5">
                          <Label className="text-xs text-muted-foreground">Command</Label>
                          <CopyableCode value={quotedBinaryPath} />
                        </div>

                        <div className="space-y-1.5">
                          <Label className="text-xs text-muted-foreground">Arguments</Label>
                          <CopyableCode value={`--mcp-bridge --client-id ${clientId}`} />
                        </div>

                        <div className="space-y-1.5">
                          <Label className="text-xs text-muted-foreground">Environment Variable</Label>
                          <div className="flex items-center gap-2">
                            <code className="flex-1 p-3 text-sm bg-muted rounded-md font-mono">
                              LOCALROUTER_CLIENT_SECRET
                            </code>
                          </div>
                          <CopyableCode
                            value={secret || "Error loading secret"}
                            masked
                            showValue={showSecret}
                            onToggleShow={() => setShowSecret(true)}
                            loading={loadingSecret}
                          />
                        </div>
                      </div>
                    </div>
                  </TabsContent>

                  <TabsContent value="json" className="space-y-4">
                    <div className="rounded-lg border p-4 space-y-4">
                      <div>
                        <p className="text-sm font-medium">MCP JSON Configuration</p>
                        <p className="text-xs text-muted-foreground mt-1">
                          Copy this JSON to your MCP client config for STDIO bridge.
                        </p>
                      </div>

                      <div
                        className={`rounded-lg bg-muted/50 p-3 space-y-2${!showSecret ? " cursor-pointer" : ""}`}
                        onClick={!showSecret ? () => setShowSecret(true) : undefined}
                        title={!showSecret ? "Click to reveal secret" : undefined}
                      >
                        <CopyableCodeBlock value={stdioJsonConfig(!showSecret)} copyValue={stdioJsonConfig(false)} />
                      </div>
                    </div>
                  </TabsContent>
                </Tabs>
              </TabsContent>
            </Tabs>
          </TabsContent>
        </Tabs>
      </CardContent>
    </Card>
  )
}
