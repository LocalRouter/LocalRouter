/**
 * HowToConnect Component
 *
 * Displays connection instructions for LLM and MCP with tabs for different methods.
 * Used in both client detail view and creation wizard.
 *
 * When a template is set, shows a "Quick Setup" tab with Launch/Configure buttons
 * and template-specific setup instructions.
 */

import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Copy, Check, Eye, RefreshCw, Cpu, Terminal, Globe, Key, FileJson, Loader2, Rocket, Settings2, ExternalLink, CheckCircle2, XCircle, RefreshCcw } from "lucide-react"
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
import { Switch } from "@/components/ui/switch"
import { CLIENT_TEMPLATES, resolveTemplatePlaceholders } from "./ClientTemplates"
import type { ClientTemplate } from "./ClientTemplates"
import ServiceIcon from "@/components/ServiceIcon"
import type { ClientMode, AppCapabilities, LaunchResult, GetAppCapabilitiesParams, TryItOutAppParams, ToggleClientSyncConfigParams, SyncClientConfigParams } from "@/types/tauri-commands"

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
  templateId?: string | null
  clientMode?: ClientMode
  syncConfig?: boolean
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

// Quick Setup tab content for template-based clients
function QuickSetupTab({
  template,
  clientId,
  baseUrl,
  secret,
  homeDir,
  configDir,
  models,
  syncConfig,
}: {
  template: ClientTemplate
  clientId: string
  baseUrl: string
  secret: string | null
  homeDir: string
  configDir: string
  models: Array<{ id: string }>
  syncConfig: boolean
}) {
  const [capabilities, setCapabilities] = useState<AppCapabilities | null>(null)
  const [checkingInstall, setCheckingInstall] = useState(true)
  const [tryingOut, setTryingOut] = useState(false)
  const [syncing, setSyncing] = useState(false)
  const [syncEnabled, setSyncEnabled] = useState(syncConfig)
  const [result, setResult] = useState<LaunchResult | null>(null)

  useEffect(() => {
    setSyncEnabled(syncConfig)
  }, [syncConfig])

  useEffect(() => {
    const fetchCapabilities = async () => {
      try {
        setCheckingInstall(true)
        const caps = await invoke<AppCapabilities>("get_app_capabilities", {
          templateId: template.id,
        } satisfies GetAppCapabilitiesParams)
        setCapabilities(caps)
      } catch (error) {
        console.error("Failed to check app capabilities:", error)
      } finally {
        setCheckingInstall(false)
      }
    }
    fetchCapabilities()
  }, [template.id])

  const handleTryItOut = async () => {
    try {
      setTryingOut(true)
      setResult(null)
      const res = await invoke<LaunchResult>("try_it_out_app", {
        clientId,
      } satisfies TryItOutAppParams)
      setResult(res)
      if (res.success) {
        toast.success("Run the command below in your terminal")
      } else {
        toast.error(res.message)
      }
    } catch (error) {
      toast.error(`Failed: ${error}`)
    } finally {
      setTryingOut(false)
    }
  }

  const handleToggleSyncConfig = async (enabled: boolean) => {
    try {
      setSyncing(true)
      setResult(null)
      const res = await invoke<LaunchResult | null>("toggle_client_sync_config", {
        clientId,
        enabled,
      } satisfies ToggleClientSyncConfigParams)
      setSyncEnabled(enabled)
      if (enabled && res) {
        setResult(res)
        if (res.success) {
          toast.success("Config sync enabled")
        } else {
          toast.error(res.message)
        }
      } else if (!enabled) {
        toast.success("Config sync disabled")
      }
    } catch (error) {
      toast.error(`Failed to toggle sync: ${error}`)
    } finally {
      setSyncing(false)
    }
  }

  const handleManualSync = async () => {
    try {
      setSyncing(true)
      setResult(null)
      const res = await invoke<LaunchResult | null>("sync_client_config", {
        clientId,
      } satisfies SyncClientConfigParams)
      if (res) {
        setResult(res)
        if (res.success) {
          toast.success("Config synced")
        } else {
          toast.error(res.message)
        }
      }
    } catch (error) {
      toast.error(`Failed to sync: ${error}`)
    } finally {
      setSyncing(false)
    }
  }

  const resolvedSecret = secret || "<your_client_secret>"
  const supportsTryItOut = capabilities?.supports_try_it_out ?? false
  const supportsPermanent = capabilities?.supports_permanent_config ?? false

  return (
    <div className="space-y-4">
      {/* Header with icon and name */}
      <div className="flex items-center gap-3">
        <ServiceIcon service={template.id} size={32} />
        <div>
          <p className="font-medium">{template.name}</p>
          <p className="text-xs text-muted-foreground">{template.description}</p>
        </div>
      </div>

      {/* App status */}
      <div className="rounded-lg border p-3">
        {checkingInstall ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Checking installation...
          </div>
        ) : capabilities?.installed ? (
          <div className="flex items-center gap-2 text-sm">
            <CheckCircle2 className="h-4 w-4 text-green-500" />
            <span>Installed</span>
            {capabilities.binary_path && (
              <code className="text-xs bg-muted px-1.5 py-0.5 rounded ml-1">{capabilities.binary_path}</code>
            )}
          </div>
        ) : (
          <div className="flex items-center gap-2 text-sm">
            <XCircle className="h-4 w-4 text-yellow-500" />
            <span>Not detected</span>
            {template.docsUrl && (
              <a
                href={template.docsUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="text-xs text-primary hover:underline flex items-center gap-1 ml-auto"
              >
                Install <ExternalLink className="h-3 w-3" />
              </a>
            )}
          </div>
        )}
      </div>

      {/* Action buttons */}
      <div className="flex gap-2">
        {supportsTryItOut && (
          <Button onClick={handleTryItOut} disabled={tryingOut || syncing} className="flex-1">
            {tryingOut ? (
              <>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                Preparing...
              </>
            ) : (
              <>
                <Rocket className="mr-2 h-4 w-4" />
                Try It Out
              </>
            )}
          </Button>
        )}
      </div>

      {/* Config sync toggle */}
      {supportsPermanent && (
        <div className="rounded-lg border p-3 space-y-2">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Settings2 className="h-4 w-4 text-muted-foreground" />
              <Label htmlFor="sync-config" className="text-sm font-medium cursor-pointer">Keep config in sync</Label>
            </div>
            <div className="flex items-center gap-2">
              {syncEnabled && (
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-7 w-7"
                  onClick={handleManualSync}
                  disabled={syncing}
                  title="Sync now"
                >
                  {syncing ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <RefreshCcw className="h-3.5 w-3.5" />
                  )}
                </Button>
              )}
              <Switch
                id="sync-config"
                checked={syncEnabled}
                onCheckedChange={handleToggleSyncConfig}
                disabled={syncing}
              />
            </div>
          </div>
          <p className="text-xs text-muted-foreground">
            {syncEnabled
              ? "Config files are kept in sync when models, secrets, or settings change."
              : "Automatically update config files when models or secrets change."}
          </p>
        </div>
      )}

      {/* Mode description */}
      {supportsTryItOut && (
        <p className="text-xs text-muted-foreground">One-time — no files modified</p>
      )}

      {/* Result */}
      {result && (
        <div className={`rounded-lg border p-3 text-sm ${result.success ? "border-green-200 bg-green-50 dark:border-green-900 dark:bg-green-950" : "border-red-200 bg-red-50 dark:border-red-900 dark:bg-red-950"}`}>
          <p>{result.message}</p>
          {result.terminal_command && (
            <div className="mt-2">
              <Label className="text-xs text-muted-foreground">Run in your terminal:</Label>
              <CopyableCodeBlock value={result.terminal_command} className="mt-1" />
            </div>
          )}
          {result.modified_files.length > 0 && (
            <div className="mt-2">
              <p className="text-xs text-muted-foreground">Modified files:</p>
              {result.modified_files.map((f) => (
                <code key={f} className="text-xs block">{f}</code>
              ))}
            </div>
          )}
          {result.backup_files.length > 0 && (
            <div className="mt-1">
              <p className="text-xs text-muted-foreground">Backups:</p>
              {result.backup_files.map((f) => (
                <code key={f} className="text-xs block">{f}</code>
              ))}
            </div>
          )}
        </div>
      )}

      {/* MCP Setup (for apps that support MCP) */}
      {template.supportsMcp && (
        <div className="rounded-lg border p-3 space-y-2">
          <div className="flex items-center gap-2">
            <Terminal className="h-4 w-4 text-muted-foreground" />
            <span className="text-sm font-medium">MCP Proxy</span>
          </div>
          <p className="text-xs text-muted-foreground">
            {template.name} will also connect to LocalRouter's MCP servers and skills.
            {template.setupType !== "generic" ? " This is configured automatically." : ""}
          </p>
          <details className="mt-1">
            <summary className="cursor-pointer text-xs text-muted-foreground hover:text-foreground">View MCP server config</summary>
            <div className="mt-2">
              <CopyableCodeBlock
                value={JSON.stringify({
                  mcpServers: {
                    localrouter: {
                      type: "http",
                      url: baseUrl,
                      headers: {
                        Authorization: `Bearer ${resolvedSecret}`
                      }
                    }
                  }
                }, null, 2)}
              />
            </div>
          </details>
        </div>
      )}

      {/* Manual setup instructions */}
      <details className="rounded-lg border">
        <summary className="cursor-pointer p-3 text-sm font-medium">Manual Setup Instructions</summary>
        <div className="px-3 pb-3 space-y-3">
          {template.setupType === "env_vars" && template.envVars && (
            <div className="space-y-2">
              <p className="text-xs font-medium">LLM Routing</p>
              <p className="text-xs text-muted-foreground">Set these environment variables:</p>
              {template.envVars.map((envVar) => (
                <div key={envVar.name} className="space-y-1">
                  <Label className="text-xs text-muted-foreground">{envVar.name}</Label>
                  <CopyableCode
                    value={resolveTemplatePlaceholders(envVar.value, baseUrl, resolvedSecret, clientId, homeDir, configDir)}
                  />
                </div>
              ))}
            </div>
          )}

          {template.setupType === "config_file" && template.configFile && (
            <div className="space-y-2">
              <p className="text-xs font-medium">LLM Routing</p>
              {template.configFile.description && (
                <p className="text-xs text-muted-foreground">{template.configFile.description}</p>
              )}
              <div className="space-y-1">
                <Label className="text-xs text-muted-foreground">Config File Path</Label>
                <CopyableCode
                  value={resolveTemplatePlaceholders(template.configFile.path, baseUrl, resolvedSecret, clientId, homeDir, configDir)}
                />
              </div>
              <div className="space-y-1">
                <Label className="text-xs text-muted-foreground">Configuration</Label>
                <CopyableCodeBlock
                  value={resolveTemplatePlaceholders(
                    typeof template.configFile.jsonSnippet === 'function'
                      ? template.configFile.jsonSnippet({ models })
                      : template.configFile.jsonSnippet,
                    baseUrl, resolvedSecret, clientId, homeDir, configDir,
                  )}
                />
              </div>
            </div>
          )}

          {template.supportsMcp && (
            <div className="space-y-2 pt-2 border-t">
              <p className="text-xs font-medium">MCP Proxy</p>
              <p className="text-xs text-muted-foreground">Add this MCP server configuration to {template.name}:</p>
              <CopyableCodeBlock
                value={JSON.stringify({
                  mcpServers: {
                    localrouter: {
                      type: "http",
                      url: baseUrl,
                      headers: {
                        Authorization: `Bearer ${resolvedSecret}`
                      }
                    }
                  }
                }, null, 2)}
              />
            </div>
          )}

          {template.manualInstructions && (
            <p className="text-xs text-muted-foreground">
              {resolveTemplatePlaceholders(template.manualInstructions, baseUrl, resolvedSecret, clientId, homeDir, configDir)}
            </p>
          )}

          {template.docsUrl && (
            <a
              href={template.docsUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="text-xs text-primary hover:underline flex items-center gap-1"
            >
              Documentation <ExternalLink className="h-3 w-3" />
            </a>
          )}
        </div>
      </details>
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
  templateId,
  clientMode,
  syncConfig = false,
}: HowToConnectProps) {
  const [showSecret, setShowSecret] = useState(false)
  const [serverConfig, setServerConfig] = useState<ServerConfig | null>(null)
  const [executablePath, setExecutablePath] = useState<string>("")
  const [models, setModels] = useState<Array<{ id: string }>>([])
  const [mcpSubTab, setMcpSubTab] = useState<string>("config")
  const [homeDir, setHomeDir] = useState<string>("")
  const [configDir, setConfigDir] = useState<string>("")

  // Resolve template from ID
  const template: ClientTemplate | null = templateId
    ? CLIENT_TEMPLATES.find(t => t.id === templateId) || null
    : null

  const hasQuickSetup = template && template.setupType !== "generic"
  const showModelsTab = clientMode !== "mcp_only"
  const showMcpTab = clientMode !== "llm_only"

  // Fetch server config, executable path, and home dir
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
    const fetchHomeDir = async () => {
      try {
        const dir = await invoke<string>("get_home_dir")
        setHomeDir(dir)
      } catch (error) {
        console.error("Failed to fetch home dir:", error)
      }
    }
    const fetchConfigDir = async () => {
      try {
        const dir = await invoke<string>("get_config_dir")
        setConfigDir(dir)
      } catch (error) {
        console.error("Failed to fetch config dir:", error)
      }
    }
    fetchServerConfig()
    fetchExecutablePath()
    fetchHomeDir()
    fetchConfigDir()
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
  const quotedBinaryPath = `"${binaryPath}"`

  const maskedSecret = "••••••••••••••••••••••••••••••••"

  // Generate API Key JSON config
  const apiKeyJsonConfig = (masked: boolean) => JSON.stringify({
    mcpServers: {
      localrouter: {
        url: baseUrl,
        type: "http",
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
        type: "http",
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

  // Determine available tabs
  const tabCount = (hasQuickSetup ? 1 : 0) + (showModelsTab ? 1 : 0) + (showMcpTab ? 1 : 0)
  // Use static Tailwind classes (dynamic interpolation doesn't work with purging)
  const gridColsClass = tabCount === 1 ? "grid-cols-1" : tabCount === 2 ? "grid-cols-2" : "grid-cols-3"
  const defaultTab = hasQuickSetup ? "quick-setup" : showModelsTab ? "models" : "mcp"

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
        <Tabs defaultValue={defaultTab}>
          <TabsList className={`mb-4 grid w-full ${gridColsClass}`}>
            {hasQuickSetup && (
              <TabsTrigger value="quick-setup" className="gap-2">
                <Rocket className="h-4 w-4" />
                Quick Setup
              </TabsTrigger>
            )}
            {showModelsTab && (
              <TabsTrigger value="models" className="gap-2">
                <Cpu className="h-4 w-4" />
                Models
              </TabsTrigger>
            )}
            {showMcpTab && (
              <TabsTrigger value="mcp" className="gap-2">
                <Terminal className="h-4 w-4" />
                MCP
              </TabsTrigger>
            )}
          </TabsList>

          {/* Quick Setup Tab */}
          {hasQuickSetup && template && (
            <TabsContent value="quick-setup">
              <QuickSetupTab
                template={template}
                clientId={clientId}
                baseUrl={baseUrl}
                secret={secret}
                homeDir={homeDir}
                configDir={configDir}
                models={models}
                syncConfig={syncConfig}
              />
            </TabsContent>
          )}

          {/* Models Tab - OpenAI-compatible API */}
          {showModelsTab && (
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
          )}

          {/* MCP Tab - Three auth methods, each with Config/JSON sub-tabs */}
          {showMcpTab && (
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
                  <Tabs value={mcpSubTab} onValueChange={setMcpSubTab}>
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
                  <Tabs value={mcpSubTab} onValueChange={setMcpSubTab}>
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
                  <Tabs value={mcpSubTab} onValueChange={setMcpSubTab}>
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
          )}
        </Tabs>
      </CardContent>
    </Card>
  )
}
