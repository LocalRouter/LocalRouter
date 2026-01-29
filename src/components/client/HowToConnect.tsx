/**
 * HowToConnect Component
 *
 * Displays connection instructions for LLM and MCP with tabs for different methods.
 * Used in both client detail view and creation wizard.
 */

import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Copy, Check, Eye, EyeOff, RefreshCw, Cpu, Terminal, Globe, Key, FileJson, Loader2 } from "lucide-react"
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
      {masked && onToggleShow && (
        <Button
          variant="outline"
          size="icon"
          onClick={onToggleShow}
          title={showValue ? "Hide" : "Show"}
          disabled={loading || !value}
        >
          {showValue ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
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
function CopyableCodeBlock({ value, className = "" }: { value: string; className?: string }) {
  const [copied, setCopied] = useState(false)

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

  // Compute URLs based on server config
  const port = serverConfig?.actual_port ?? serverConfig?.port ?? 3625
  const host = serverConfig?.host ?? "127.0.0.1"
  const baseUrl = `http://${host}:${port}`

  // Binary path from the running executable
  const binaryPath = executablePath || "/path/to/localrouter"
  // Quoted version for shell usage (handles spaces in path)
  const quotedBinaryPath = `"${binaryPath}"`

  // Generate STDIO config JSON
  const stdioConfig = JSON.stringify({
    mcpServers: {
      localrouter: {
        command: binaryPath,
        args: ["--mcp-bridge", "--client-id", clientId],
        env: {
          LOCALROUTER_CLIENT_SECRET: secret || "<your_client_secret>"
        }
      }
    }
  }, null, 2)

  // Generate MCP JSON config for HTTP/SSE
  const mcpJsonConfig = JSON.stringify({
    mcpServers: {
      localrouter: {
        url: baseUrl,
        transport: "sse",
        headers: {
          Authorization: `Bearer ${secret || "<your_client_secret>"}`
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
                <span className="text-sm font-medium">HTTP/SSE (OpenAI-compatible)</span>
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
                    onToggleShow={() => setShowSecret(!showSecret)}
                    loading={loadingSecret}
                  />
                </div>
              </div>

              <div className="rounded-lg bg-muted/50 p-3 space-y-2">
                <p className="text-xs font-medium">Usage Example</p>
                <CopyableCodeBlock value={`curl ${baseUrl}/chat/completions \\
  -H "Authorization: Bearer <api_key>" \\
  -H "Content-Type: application/json" \\
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hello"}]}'`} />
              </div>
            </div>
          </TabsContent>

          {/* MCP Tab - Multiple connection methods */}
          <TabsContent value="mcp" className="space-y-4">
            <Tabs defaultValue="stdio">
              <TabsList className="mb-4 w-full grid grid-cols-4">
                <TabsTrigger value="stdio" className="text-xs gap-1">
                  <Terminal className="h-3 w-3" />
                  STDIO
                </TabsTrigger>
                <TabsTrigger value="http-oauth" className="text-xs gap-1">
                  <Key className="h-3 w-3" />
                  OAuth
                </TabsTrigger>
                <TabsTrigger value="http-bearer" className="text-xs gap-1">
                  <Globe className="h-3 w-3" />
                  Bearer
                </TabsTrigger>
                <TabsTrigger value="json" className="text-xs gap-1">
                  <FileJson className="h-3 w-3" />
                  JSON
                </TabsTrigger>
              </TabsList>

              {/* MCP STDIO */}
              <TabsContent value="stdio" className="space-y-4">
                <div className="rounded-lg border p-4 space-y-4">
                  <div>
                    <p className="text-sm font-medium">STDIO Bridge</p>
                    <p className="text-xs text-muted-foreground mt-1">
                      Recommended for Claude Desktop, Cursor, VS Code, and other MCP clients that use STDIO transport.
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
                        onToggleShow={() => setShowSecret(!showSecret)}
                        loading={loadingSecret}
                      />
                    </div>
                  </div>

                  <div className="rounded-lg bg-blue-500/10 border border-blue-500/20 p-3 space-y-2">
                    <p className="text-xs font-medium">MCP Configuration JSON</p>
                    <CopyableCodeBlock value={stdioConfig} />
                  </div>
                </div>
              </TabsContent>

              {/* MCP HTTP OAuth */}
              <TabsContent value="http-oauth" className="space-y-4">
                <div className="rounded-lg border p-4 space-y-4">
                  <div>
                    <p className="text-sm font-medium">HTTP/SSE with OAuth 2.0</p>
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
                        onToggleShow={() => setShowSecret(!showSecret)}
                        loading={loadingSecret}
                      />
                    </div>
                  </div>

                  <div className="rounded-lg bg-muted/50 p-3 space-y-2">
                    <p className="text-xs font-medium">Token Exchange</p>
                    <CopyableCodeBlock value={`POST ${baseUrl}/oauth/token
Content-Type: application/x-www-form-urlencoded

grant_type=client_credentials&client_id=${clientUuid}&client_secret=<secret>`} />
                  </div>
                </div>
              </TabsContent>

              {/* MCP HTTP Bearer */}
              <TabsContent value="http-bearer" className="space-y-4">
                <div className="rounded-lg border p-4 space-y-4">
                  <div>
                    <p className="text-sm font-medium">HTTP/SSE with Bearer Token</p>
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
                      <Label className="text-xs text-muted-foreground">Authorization Header</Label>
                      <CopyableCode
                        value={`Bearer ${secret || "<your_client_secret>"}`}
                        masked
                        showValue={showSecret}
                        onToggleShow={() => setShowSecret(!showSecret)}
                        loading={loadingSecret}
                      />
                    </div>
                  </div>

                  <div className="rounded-lg bg-muted/50 p-3 space-y-2">
                    <p className="text-xs font-medium">Usage Example</p>
                    <CopyableCodeBlock value={`curl ${baseUrl} \\
  -H "Authorization: Bearer <client_secret>" \\
  -H "Content-Type: application/json" \\
  -d '{"jsonrpc": "2.0", "method": "tools/list", "id": 1}'`} />
                  </div>
                </div>
              </TabsContent>

              {/* MCP JSON Config */}
              <TabsContent value="json" className="space-y-4">
                <div className="rounded-lg border p-4 space-y-4">
                  <div>
                    <p className="text-sm font-medium">MCP JSON Configuration</p>
                    <p className="text-xs text-muted-foreground mt-1">
                      Copy this JSON configuration to connect via HTTP/SSE transport.
                    </p>
                  </div>

                  <div className="rounded-lg bg-muted/50 p-3 space-y-2">
                    <CopyableCodeBlock value={mcpJsonConfig} />
                  </div>

                  <div className="rounded-lg bg-yellow-500/10 border border-yellow-500/20 p-3">
                    <p className="text-xs text-yellow-600 dark:text-yellow-400">
                      <strong>Note:</strong> Replace the Authorization header value with your actual client secret.
                      The secret is shown above when you click the eye icon.
                    </p>
                  </div>
                </div>
              </TabsContent>
            </Tabs>
          </TabsContent>
        </Tabs>
      </CardContent>
    </Card>
  )
}
