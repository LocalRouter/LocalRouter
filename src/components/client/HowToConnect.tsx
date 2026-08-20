/**
 * HowToConnect Component
 *
 * Displays connection instructions for LLM and MCP with tabs for different methods.
 * Used in both client detail view and creation wizard.
 *
 * When a template is set, shows a "Quick Setup" tab with Launch/Configure buttons
 * and template-specific setup instructions.
 */

import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Copy, Check, Eye, RefreshCw, Cpu, Terminal, Globe, Key, FileJson, Loader2, Rocket, Settings2, ExternalLink, CheckCircle2, XCircle, RefreshCcw, BookOpen, AlertTriangle, Info, ShieldCheck, Network, Play, Square } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { ExperimentalBadge } from "@/components/shared/ExperimentalBadge"
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
import { isValidHttpUrl } from "@/utils/url"
import { listenSafe } from "@/hooks/useTauriListener"
import type { LlmMode, McpMode, AppCapabilities, LaunchResult, GetAppCapabilitiesParams, TryItOutAppParams, ToggleClientSyncConfigParams, SyncClientConfigParams, ProxySetupInfo, GetClientProxySetupParams, ConfigureClientProxyParams, UnconfigureClientProxyParams, CaTrustStatus, ReverseProxySetupInfo, ReverseListenerState, GetClientReverseProxySetupParams, ConfigureClientReverseProxyParams, UnconfigureClientReverseProxyParams, StartClientReverseProxyParams, StopClientReverseProxyParams } from "@/types/tauri-commands"

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
  llmMode?: LlmMode
  mcpMode?: McpMode
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
    <div className="flex items-center gap-2 min-w-0">
      <code className="flex-1 min-w-0 p-3 text-sm bg-muted rounded-md font-mono break-all">
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
          className="shrink-0"
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
        className="shrink-0"
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
    <div className={`relative min-w-0 ${className}`}>
      <pre className="text-xs font-mono bg-muted p-3 pr-10 rounded-lg whitespace-pre-wrap break-words overflow-hidden">
        {value}
      </pre>
      <Button
        variant="outline"
        size="icon"
        className="absolute top-2 right-2 h-7 w-7 shrink-0"
        onClick={handleCopy}
        title="Copy to clipboard"
      >
        {copied ? <Check className="h-3 w-3 text-green-500" /> : <Copy className="h-3 w-3" />}
      </Button>
    </div>
  )
}

// Helper component to display a LaunchResult (shared between Temporary and Auto tabs)
function LaunchResultDisplay({ result }: { result: LaunchResult }) {
  return (
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
            <code key={f} className="text-xs block break-all">{f}</code>
          ))}
        </div>
      )}
      {result.backup_files.length > 0 && (
        <div className="mt-1">
          <p className="text-xs text-muted-foreground">Backups:</p>
          {result.backup_files.map((f) => (
            <code key={f} className="text-xs block break-all">{f}</code>
          ))}
        </div>
      )}
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
  const [temporaryResult, setTemporaryResult] = useState<LaunchResult | null>(null)
  const [autoResult, setAutoResult] = useState<LaunchResult | null>(null)

  useEffect(() => {
    setSyncEnabled(syncConfig)
  }, [syncConfig])

  const refreshCapabilities = async () => {
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

  useEffect(() => {
    refreshCapabilities()
  }, [template.id])

  // Auto-fetch temporary launch command when capabilities indicate support
  useEffect(() => {
    if (!capabilities?.supports_try_it_out) return
    const fetchLaunchCommand = async () => {
      try {
        setTryingOut(true)
        const res = await invoke<LaunchResult>("try_it_out_app", {
          clientId,
        } satisfies TryItOutAppParams)
        setTemporaryResult(res)
      } catch (error) {
        console.error("Failed to fetch launch command:", error)
      } finally {
        setTryingOut(false)
      }
    }
    fetchLaunchCommand()
  }, [capabilities, clientId])

  const handleTryItOut = async () => {
    try {
      setTryingOut(true)
      setTemporaryResult(null)
      const res = await invoke<LaunchResult>("try_it_out_app", {
        clientId,
      } satisfies TryItOutAppParams)
      setTemporaryResult(res)
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
      setAutoResult(null)
      const res = await invoke<LaunchResult | null>("toggle_client_sync_config", {
        clientId,
        enabled,
      } satisfies ToggleClientSyncConfigParams)
      setSyncEnabled(enabled)
      if (enabled && res) {
        setAutoResult(res)
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
      setAutoResult(null)
      const res = await invoke<LaunchResult | null>("sync_client_config", {
        clientId,
      } satisfies SyncClientConfigParams)
      if (res) {
        setAutoResult(res)
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

  const innerTabCount = 1 + (supportsTryItOut ? 1 : 0) + (supportsPermanent ? 1 : 0)
  const innerGridCols = innerTabCount === 1 ? "grid-cols-1" : innerTabCount === 2 ? "grid-cols-2" : "grid-cols-3"

  const [innerTab, setInnerTab] = useState("manual")
  useEffect(() => {
    setInnerTab(supportsPermanent ? "auto" : supportsTryItOut ? "temporary" : "manual")
  }, [supportsPermanent, supportsTryItOut])

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
            <div className="flex items-center gap-2 ml-auto">
              {template.docsUrl && isValidHttpUrl(template.docsUrl) && (
                <a
                  href={template.docsUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-xs text-primary hover:underline flex items-center gap-1"
                >
                  Install <ExternalLink className="h-3 w-3" />
                </a>
              )}
              <Button
                variant="ghost"
                size="sm"
                className="h-6 w-6 p-0"
                onClick={refreshCapabilities}
              >
                <RefreshCw className="h-3.5 w-3.5" />
              </Button>
            </div>
          </div>
        )}
      </div>

      {/* Inner tabs: Manual / Temporary / Auto */}
      <Tabs value={innerTab} onValueChange={setInnerTab}>
        <TabsList className={`mb-4 grid w-full ${innerGridCols}`}>
          {supportsPermanent && (
            <TabsTrigger value="auto" className="text-xs gap-1">
              <RefreshCcw className="h-3 w-3" />
              Auto
              <ExperimentalBadge className="ml-1 text-[10px] px-1.5 py-0" />
            </TabsTrigger>
          )}
          {supportsTryItOut && (
            <TabsTrigger value="temporary" className="text-xs gap-1">
              <Rocket className="h-3 w-3" />
              Quick Start
            </TabsTrigger>
          )}
          <TabsTrigger value="manual" className="text-xs gap-1">
            <BookOpen className="h-3 w-3" />
            Manual
          </TabsTrigger>
        </TabsList>

        {/* Auto tab */}
        {supportsPermanent && (
          <TabsContent value="auto" className="space-y-4">
            <p className="text-xs text-muted-foreground">
              LocalRouter automatically configures {template.name} and keeps it in sync.
            </p>

            {/* What auto-sync will do */}
            <div className="rounded-lg border p-3 space-y-2.5">
              <p className="text-xs font-medium">When enabled, LocalRouter will:</p>
              <ul className="space-y-2">
                {template.configFile && (
                  <li className="flex items-start gap-2 text-xs text-muted-foreground">
                    <FileJson className="h-3.5 w-3.5 mt-0.5 shrink-0 text-foreground" />
                    <span>
                      Write configuration to{" "}
                      <code className="bg-muted px-1 py-0.5 rounded break-all">
                        {resolveTemplatePlaceholders(template.configFile.path, baseUrl, resolvedSecret, clientId, homeDir, configDir)}
                      </code>
                    </span>
                  </li>
                )}
                {template.envVars && template.envVars.length > 0 && (
                  <li className="flex items-start gap-2 text-xs text-muted-foreground">
                    <Key className="h-3.5 w-3.5 mt-0.5 shrink-0 text-foreground" />
                    <span>Set API base URL and credentials ({template.envVars.map(e => e.name).join(", ")})</span>
                  </li>
                )}
                <li className="flex items-start gap-2 text-xs text-muted-foreground">
                  <Cpu className="h-3.5 w-3.5 mt-0.5 shrink-0 text-foreground" />
                  <span>Configure available models from this client's routing strategy</span>
                </li>
                {template.supportsMcp && (
                  <li className="flex items-start gap-2 text-xs text-muted-foreground">
                    <Terminal className="h-3.5 w-3.5 mt-0.5 shrink-0 text-foreground" />
                    <span>
                      Set up MCP proxy connection to LocalRouter's servers and skills
                      {template.mcpConfigFile && (
                        <> via <code className="bg-muted px-1 py-0.5 rounded break-all">{resolveTemplatePlaceholders(template.mcpConfigFile.path, baseUrl, resolvedSecret, clientId, homeDir, configDir)}</code></>
                      )}
                    </span>
                  </li>
                )}
                <li className="flex items-start gap-2 text-xs text-muted-foreground">
                  <RefreshCcw className="h-3.5 w-3.5 mt-0.5 shrink-0 text-foreground" />
                  <span>Re-sync automatically when models, secrets, or server settings change</span>
                </li>
              </ul>
              {template.mcpNote && (
                <div className="flex items-start gap-2 text-xs text-muted-foreground mt-2 pt-2 border-t">
                  <AlertTriangle className="h-3.5 w-3.5 mt-0.5 shrink-0 text-yellow-500" />
                  <span>{template.mcpNote}</span>
                </div>
              )}
            </div>

            {/* Toggle */}
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
            </div>

            {/* Warning about overwrites */}
            <div className="flex items-start gap-2 text-xs text-muted-foreground">
              <AlertTriangle className="h-3.5 w-3.5 mt-0.5 shrink-0 text-yellow-500" />
              <span>Manual edits to managed config files may be overwritten during sync.</span>
            </div>

            {autoResult && <LaunchResultDisplay result={autoResult} />}
          </TabsContent>
        )}

        {/* Quick Start tab */}
        {supportsTryItOut && (
          <TabsContent value="temporary" className="space-y-4">
            <p className="text-xs text-muted-foreground">
              Launch {template.name} with LocalRouter pre-configured. No files are modified — settings are passed via environment variables.
            </p>

            {tryingOut ? (
              <div className="rounded-lg border p-3">
                <div className="flex items-center gap-2 text-sm text-muted-foreground">
                  <Loader2 className="h-4 w-4 animate-spin" />
                  Generating launch command...
                </div>
              </div>
            ) : temporaryResult ? (
              <div className="space-y-3">
                {temporaryResult.success && temporaryResult.terminal_command ? (
                  <div className="space-y-1.5">
                    <Label className="text-xs text-muted-foreground">Run in your terminal:</Label>
                    <CopyableCodeBlock value={temporaryResult.terminal_command} />
                  </div>
                ) : (
                  <div className="rounded-lg border border-red-200 bg-red-50 dark:border-red-900 dark:bg-red-950 p-3 text-sm">
                    <p>{temporaryResult.message}</p>
                  </div>
                )}
                <Button variant="ghost" size="sm" onClick={handleTryItOut} disabled={tryingOut} className="text-xs">
                  <RefreshCw className="mr-1.5 h-3 w-3" />
                  Regenerate
                </Button>
              </div>
            ) : null}

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
              </div>
            )}
          </TabsContent>
        )}

        {/* Manual tab */}
        <TabsContent value="manual" className="space-y-3">
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
              {template.mcpConfigFile ? (
                <>
                  {template.mcpConfigFile.description && (
                    <p className="text-xs text-muted-foreground">{template.mcpConfigFile.description}</p>
                  )}
                  <div className="space-y-1">
                    <Label className="text-xs text-muted-foreground">Config File Path</Label>
                    <CopyableCode
                      value={resolveTemplatePlaceholders(template.mcpConfigFile.path, baseUrl, resolvedSecret, clientId, homeDir, configDir)}
                    />
                  </div>
                  <div className="space-y-1">
                    <Label className="text-xs text-muted-foreground">Configuration</Label>
                    <CopyableCodeBlock
                      value={resolveTemplatePlaceholders(
                        typeof template.mcpConfigFile.jsonSnippet === 'function'
                          ? template.mcpConfigFile.jsonSnippet({ models })
                          : template.mcpConfigFile.jsonSnippet,
                        baseUrl, resolvedSecret, clientId, homeDir, configDir,
                      )}
                    />
                  </div>
                  {template.mcpNote && (
                    <div className="flex items-start gap-2 text-xs text-muted-foreground">
                      <AlertTriangle className="h-3.5 w-3.5 mt-0.5 shrink-0 text-yellow-500" />
                      <span>{template.mcpNote}</span>
                    </div>
                  )}
                </>
              ) : (
                <>
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
                </>
              )}
            </div>
          )}

          {template.manualInstructions && (
            <p className="text-xs text-muted-foreground">
              {resolveTemplatePlaceholders(template.manualInstructions, baseUrl, resolvedSecret, clientId, homeDir, configDir)}
            </p>
          )}

          {template.docsUrl && isValidHttpUrl(template.docsUrl) && (
            <a
              href={template.docsUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="text-xs text-primary hover:underline flex items-center gap-1"
            >
              Documentation <ExternalLink className="h-3 w-3" />
            </a>
          )}
        </TabsContent>
      </Tabs>
    </div>
  )
}

// LLM connection instructions for a client in an HTTPS-proxy mode. Replaces the
// native ANTHROPIC_BASE_URL setup — the tool keeps talking to the provider, but
// its traffic is routed through LocalRouter (via HTTPS_PROXY) for inspection.
/**
 * Reverse-proxy setup: LocalRouter takes over a local provider's port.
 *
 * The UI's job here is to make a three-part state legible, because that is what
 * actually goes wrong: the provider has to have moved, LocalRouter has to hold
 * the original port, and the provider instance in LocalRouter's own config has
 * to point at the new address. Each piece is shown separately rather than
 * collapsed into one "configured" boolean.
 */
function ReverseProxySetup({
  clientUuid,
  template,
}: {
  clientUuid: string
  template: ClientTemplate | null
}) {
  const [info, setInfo] = useState<ReverseProxySetupInfo | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [configuring, setConfiguring] = useState(false)
  const [undoing, setUndoing] = useState(false)
  const [busyListener, setBusyListener] = useState(false)
  const [result, setResult] = useState<LaunchResult | null>(null)
  const [innerTab, setInnerTab] = useState("auto")

  const load = useCallback(() => {
    return invoke<ReverseProxySetupInfo>("get_client_reverse_proxy_setup", {
      clientId: clientUuid,
    } satisfies GetClientReverseProxySetupParams)
      .then((data) => { setInfo(data); setError(null) })
      .catch((e) => setError(String(e)))
  }, [clientUuid])

  useEffect(() => {
    // Clear the previous client's state so a result banner can never appear
    // under a client it didn't apply to.
    setInfo(null)
    setError(null)
    setResult(null)
    load()
    const l = listenSafe("clients-changed", load)
    return () => l.cleanup()
  }, [clientUuid, load])

  useEffect(() => {
    if (info) setInnerTab(info.supports_auto ? "auto" : "manual")
  }, [info?.supports_auto]) // eslint-disable-line react-hooks/exhaustive-deps

  const handleConfigure = async () => {
    try {
      setConfiguring(true)
      setResult(null)
      const res = await invoke<LaunchResult>("configure_client_reverse_proxy", {
        clientId: clientUuid,
      } satisfies ConfigureClientReverseProxyParams)
      setResult(res)
      if (res.success) toast.success(`${info?.provider_label ?? "Provider"} wrapped`)
      else toast.error("Setup did not complete — see the details below")
      await load()
    } catch (e) {
      toast.error(`Failed: ${e}`)
    } finally {
      setConfiguring(false)
    }
  }

  const handleUndo = async () => {
    try {
      setUndoing(true)
      setResult(null)
      const res = await invoke<LaunchResult>("unconfigure_client_reverse_proxy", {
        clientId: clientUuid,
      } satisfies UnconfigureClientReverseProxyParams)
      setResult(res)
      toast.success("Reverted")
      await load()
    } catch (e) {
      toast.error(`Failed: ${e}`)
    } finally {
      setUndoing(false)
    }
  }

  const handleListener = async (start: boolean) => {
    try {
      setBusyListener(true)
      await invoke<ReverseListenerState>(
        start ? "start_client_reverse_proxy" : "stop_client_reverse_proxy",
        start
          ? ({ clientId: clientUuid } satisfies StartClientReverseProxyParams)
          : ({ clientId: clientUuid } satisfies StopClientReverseProxyParams),
      )
      toast.success(start ? "Listener started" : "Listener stopped")
      await load()
    } catch (e) {
      toast.error(String(e))
    } finally {
      setBusyListener(false)
    }
  }

  if (error) return <p className="text-sm text-destructive">Failed to load setup: {error}</p>
  if (!info) return <p className="text-sm text-muted-foreground">Loading reverse-proxy setup…</p>

  const providerName = info.provider_label || template?.name || "your provider"
  const appAddress = `http://${info.listen_host}:${info.listen_port}`
  const listening = info.listener.running

  const StatusRow = ({ ok, label, detail }: { ok: boolean; label: string; detail: string }) => (
    <div className="flex items-start gap-2">
      {ok ? (
        <CheckCircle2 className="h-4 w-4 mt-px shrink-0 text-green-600 dark:text-green-500" />
      ) : (
        <XCircle className="h-4 w-4 mt-px shrink-0 text-muted-foreground" />
      )}
      <div className="min-w-0">
        <p className="text-xs font-medium">{label}</p>
        <p className="text-[11px] text-muted-foreground break-all">{detail}</p>
      </div>
    </div>
  )

  return (
    <div className="rounded-lg border p-4 space-y-4">
      <div className="flex items-center gap-2">
        <Network className="h-4 w-4 text-muted-foreground" />
        <span className="text-sm font-medium">Reverse Proxy — wrapping {providerName}</span>
      </div>

      <div className="rounded-md border bg-muted/40 p-2.5 text-xs text-muted-foreground">
        Your apps keep pointing at <code className="bg-muted px-1 py-0.5 rounded">{appAddress}</code>{" "}
        exactly as before. LocalRouter answers there and forwards everything to {providerName} at{" "}
        <code className="bg-muted px-1 py-0.5 rounded">{info.upstream_url}</code>, recording each
        call in the Monitor along the way. Nothing needs to change in the apps themselves.
      </div>

      {/* The three pieces that have to line up, each shown on its own. */}
      <div className="grid gap-2.5 sm:grid-cols-3 rounded-md border p-3">
        <StatusRow
          ok={listening}
          label={listening ? "LocalRouter is listening" : "Not listening"}
          detail={
            listening
              ? `Holding port ${info.listen_port} for your apps`
              : info.listener.error ?? `Port ${info.listen_port} is not bound yet`
          }
        />
        <StatusRow
          ok={info.upstream_reachable}
          label={info.upstream_reachable ? `${providerName} relocated` : `${providerName} not found`}
          detail={
            info.upstream_reachable
              ? `Answering at ${info.upstream_url}`
              : `Nothing is answering at ${info.upstream_url} yet`
          }
        />
        <StatusRow
          ok={!!info.provider_instance}
          label={info.provider_instance ? "Provider linked" : "No linked provider"}
          detail={
            info.provider_instance
              ? `'${info.provider_instance}' follows the move automatically`
              : "LocalRouter's own provider list won't be updated"
          }
        />
      </div>

      {info.notes.length > 0 && (
        <ul className="space-y-1.5 text-xs text-muted-foreground">
          {info.notes.map((note, i) => (
            <li key={i} className="flex gap-1.5">
              <Info className="h-3.5 w-3.5 shrink-0 mt-px" />
              <span className="whitespace-pre-line">{note}</span>
            </li>
          ))}
        </ul>
      )}

      <Tabs value={innerTab} onValueChange={setInnerTab}>
        <TabsList className={`mb-4 grid w-full ${info.supports_auto ? "grid-cols-2" : "grid-cols-1"}`}>
          {info.supports_auto && (
            <TabsTrigger value="auto" className="text-xs gap-1">
              <RefreshCcw className="h-3 w-3" />
              Auto
            </TabsTrigger>
          )}
          <TabsTrigger value="manual" className="text-xs gap-1">
            <BookOpen className="h-3 w-3" />
            Manual
          </TabsTrigger>
        </TabsList>

        {info.supports_auto && (
          <TabsContent value="auto" className="space-y-3">
            <p className="text-xs text-muted-foreground">
              LocalRouter moves {providerName} to port{" "}
              <code className="bg-muted px-1 py-0.5 rounded">
                {info.upstream_url.split(":").pop()}
              </code>
              , points its own provider entry at the new address, then binds port{" "}
              <code className="bg-muted px-1 py-0.5 rounded">{info.listen_port}</code>.
              {info.restart_hint && ` ${info.restart_hint}`}
            </p>
            {info.auto_commands.length > 0 && (
              <div className="rounded-md bg-muted/60 p-2 space-y-1">
                <p className="text-[11px] text-muted-foreground">This runs:</p>
                {info.auto_commands.map((cmd, i) => (
                  <code key={i} className="block text-[11px] font-mono break-all">{cmd}</code>
                ))}
              </div>
            )}
            <div className="flex flex-wrap items-center gap-2">
              <Button size="sm" onClick={handleConfigure} disabled={configuring || undoing}>
                {configuring ? <Loader2 className="h-3.5 w-3.5 mr-2 animate-spin" /> : <Settings2 className="h-3.5 w-3.5 mr-2" />}
                Wrap {providerName}
              </Button>
              {info.supports_undo && (
                <Button size="sm" variant="outline" onClick={handleUndo} disabled={configuring || undoing}>
                  {undoing && <Loader2 className="h-3.5 w-3.5 mr-2 animate-spin" />}
                  Undo
                </Button>
              )}
            </div>
          </TabsContent>
        )}

        <TabsContent value="manual" className="space-y-3">
          {info.manual_steps.length > 0 && (
            <ol className="space-y-1.5 text-xs list-decimal list-inside text-muted-foreground">
              {info.manual_steps.map((step, i) => (
                <li key={i} className="whitespace-pre-line">{step}</li>
              ))}
            </ol>
          )}
          {info.oneoff_command && (
            <div className="space-y-1.5">
              <Label className="text-xs">Or start {providerName} on the new port for this session</Label>
              <CopyableCode value={info.oneoff_command} />
            </div>
          )}
          <p className="text-[11px] text-muted-foreground">
            Once {providerName} answers on {info.upstream_url}, start the listener below.
          </p>
        </TabsContent>
      </Tabs>

      {/* The listener is separately controllable: for providers LocalRouter
          can't relocate, moving the provider and binding the port are two
          different acts by two different parties. */}
      <div className="flex flex-wrap items-center gap-2 border-t pt-3">
        {listening ? (
          <Button size="sm" variant="outline" onClick={() => handleListener(false)} disabled={busyListener}>
            {busyListener ? <Loader2 className="h-3.5 w-3.5 mr-2 animate-spin" /> : <Square className="h-3.5 w-3.5 mr-2" />}
            Stop listener
          </Button>
        ) : (
          <Button size="sm" variant="outline" onClick={() => handleListener(true)} disabled={busyListener}>
            {busyListener ? <Loader2 className="h-3.5 w-3.5 mr-2 animate-spin" /> : <Play className="h-3.5 w-3.5 mr-2" />}
            Start listener
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={() => load()} disabled={busyListener}>
          <RefreshCw className="h-3.5 w-3.5 mr-2" />
          Recheck
        </Button>
      </div>

      {result && (
        <div className={`rounded-md border p-2.5 text-xs whitespace-pre-line ${result.success ? "" : "border-destructive/50 text-destructive"}`}>
          {result.message}
          {result.terminal_command && (
            <div className="mt-2">
              <CopyableCode value={result.terminal_command} />
            </div>
          )}
        </div>
      )}
    </div>
  )
}

function ProxyLlmSetup({
  clientUuid,
  template,
}: {
  clientUuid: string
  template: ClientTemplate | null
}) {
  const [info, setInfo] = useState<ProxySetupInfo | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [configuring, setConfiguring] = useState(false)
  const [removing, setRemoving] = useState(false)
  const [autoResult, setAutoResult] = useState<LaunchResult | null>(null)
  const [caTrust, setCaTrust] = useState<CaTrustStatus | null>(null)
  const [trusting, setTrusting] = useState(false)

  // Which tools LocalRouter can configure, and their caveats, are decided by
  // the backend plan (src-tauri/src/launcher/proxy_setup.rs) — not duplicated
  // here, so the two can't drift.
  const supportsAuto = info?.supports_auto ?? false
  const [innerTab, setInnerTab] = useState("temporary")
  useEffect(() => {
    setInnerTab(supportsAuto ? "auto" : "temporary")
  }, [supportsAuto])

  const loadTrust = useCallback(() => {
    invoke<CaTrustStatus>("get_proxy_ca_trust_status")
      .then(setCaTrust)
      .catch(() => setCaTrust(null))
  }, [])

  useEffect(() => {
    let cancelled = false
    // Drop the previous client's plan and result banner before loading the
    // new one, so a "Configured …" message can't appear under a client it
    // didn't apply to.
    setInfo(null)
    setError(null)
    setAutoResult(null)
    const load = () => {
      invoke<ProxySetupInfo>("get_client_proxy_setup", { clientId: clientUuid } satisfies GetClientProxySetupParams)
        .then((data) => { if (!cancelled) { setInfo(data); setError(null) } })
        .catch((e) => { if (!cancelled) setError(String(e)) })
    }
    load()
    loadTrust()
    const l = listenSafe("clients-changed", load)
    return () => { cancelled = true; l.cleanup() }
  }, [clientUuid, loadTrust])

  const handleAutoConfigure = async () => {
    try {
      setConfiguring(true)
      setAutoResult(null)
      const res = await invoke<LaunchResult>("configure_client_proxy", { clientId: clientUuid } satisfies ConfigureClientProxyParams)
      setAutoResult(res)
      if (res.success) toast.success("Proxy configured")
      else toast.error(res.message)
    } catch (e) {
      toast.error(`Failed: ${e}`)
    } finally {
      setConfiguring(false)
    }
  }

  const handleRemoveConfig = async () => {
    try {
      setRemoving(true)
      setAutoResult(null)
      const res = await invoke<LaunchResult>("unconfigure_client_proxy", { clientId: clientUuid } satisfies UnconfigureClientProxyParams)
      setAutoResult(res)
      if (res.success) toast.success("Proxy configuration removed")
      else toast.error(res.message)
    } catch (e) {
      toast.error(`Failed: ${e}`)
    } finally {
      setRemoving(false)
    }
  }

  const handleTrustCa = async (trust: boolean) => {
    try {
      setTrusting(true)
      const msg = await invoke<string>(trust ? "trust_proxy_ca" : "untrust_proxy_ca")
      toast.success(msg)
      loadTrust()
    } catch (e) {
      toast.error(String(e))
    } finally {
      setTrusting(false)
    }
  }

  if (error) return <p className="text-sm text-destructive">Failed to load proxy setup: {error}</p>
  if (!info) return <p className="text-sm text-muted-foreground">Loading proxy setup…</p>

  // The backend builds the tool-specific command where one exists. The generic
  // fallback is only valid when the tool actually reads a CA env var — tools
  // that trust the OS store report null, and splicing that in would produce a
  // command that isn't a valid shell assignment.
  const binary = template?.binaryNames?.[0]
  const oneoff = info.oneoff_command
    ?? (info.proxy_url && info.ca_env_var
      ? `HTTPS_PROXY=${info.proxy_url} ${info.ca_env_var}=${info.ca_cert_path} ${binary ?? "<your-tool>"}`
      : null)

  const innerTabCount = 2 + (supportsAuto ? 1 : 0)
  const innerGridCols = innerTabCount === 3 ? "grid-cols-3" : "grid-cols-2"
  const toolName = template?.name ?? "your tool"

  return (
    <div className="rounded-lg border p-4 space-y-4">
      <div className="flex items-center gap-2">
        <Globe className="h-4 w-4 text-muted-foreground" />
        <span className="text-sm font-medium">HTTPS Inspection Proxy</span>
      </div>

      <div className="rounded-md border border-amber-500/40 bg-amber-500/5 p-2 text-xs text-muted-foreground">
        LocalRouter decrypts this client's LLM traffic to show it in the Monitor, then forwards it
        unchanged — credentials pass straight through and are never stored. Trust the root CA below
        only on machines you control.
      </div>

      {!info.running && (
        <p className="text-xs text-destructive">The proxy listener is not running.</p>
      )}

      {/* Caveats for this specific tool, straight from the backend plan. */}
      {info.notes.length > 0 && (
        <ul className="space-y-1.5 text-xs text-muted-foreground">
          {info.notes.map((note, i) => (
            <li key={i} className="flex gap-1.5">
              <Info className="h-3.5 w-3.5 shrink-0 mt-px text-muted-foreground" />
              <span>{note}</span>
            </li>
          ))}
        </ul>
      )}

      {/* Tools with no CA setting of their own need the root CA in the OS
          trust store. That is a consequential change, so it is its own
          deliberate action rather than part of Configure. */}
      {info.requires_system_ca && (
        <div className="rounded-md border border-amber-500/40 bg-amber-500/5 p-2.5 space-y-2">
          <p className="text-xs">
            <span className="font-medium">{toolName} has no certificate setting</span> — it trusts
            whatever your operating system trusts. LocalRouter&apos;s root CA has to be added there
            for interception to work.
          </p>
          <p className="text-[11px] text-muted-foreground">
            A trusted root certificate can vouch for any website, so only do this on a machine you
            control. You can undo it here at any time.
          </p>
          {caTrust?.can_manage ? (
            <div className="flex items-center gap-2">
              {caTrust.state === "trusted" ? (
                <>
                  <span className="text-xs flex items-center gap-1 text-green-600 dark:text-green-500">
                    <CheckCircle2 className="h-3.5 w-3.5" /> Trusted
                  </span>
                  <Button size="sm" variant="outline" onClick={() => handleTrustCa(false)} disabled={trusting}>
                    {trusting && <Loader2 className="h-3.5 w-3.5 mr-2 animate-spin" />}
                    Remove trust
                  </Button>
                </>
              ) : (
                <Button size="sm" variant="outline" onClick={() => handleTrustCa(true)} disabled={trusting}>
                  {trusting ? <Loader2 className="h-3.5 w-3.5 mr-2 animate-spin" /> : <ShieldCheck className="h-3.5 w-3.5 mr-2" />}
                  Trust LocalRouter&apos;s CA
                </Button>
              )}
            </div>
          ) : (
            caTrust?.manual_instructions && (
              <p className="text-[11px] text-muted-foreground">{caTrust.manual_instructions}</p>
            )
          )}
        </div>
      )}

      <Tabs value={innerTab} onValueChange={setInnerTab}>
        <TabsList className={`mb-4 grid w-full ${innerGridCols}`}>
          {supportsAuto && (
            <TabsTrigger value="auto" className="text-xs gap-1">
              <RefreshCcw className="h-3 w-3" />
              Auto
            </TabsTrigger>
          )}
          <TabsTrigger value="temporary" className="text-xs gap-1">
            <Rocket className="h-3 w-3" />
            Quick Start
          </TabsTrigger>
          <TabsTrigger value="manual" className="text-xs gap-1">
            <BookOpen className="h-3 w-3" />
            Manual
          </TabsTrigger>
        </TabsList>

        {/* Auto: LocalRouter writes the tool's own config file. */}
        {supportsAuto && (
          <TabsContent value="auto" className="space-y-3">
            <p className="text-xs text-muted-foreground">
              LocalRouter writes the proxy configuration to{" "}
              <code className="bg-muted px-1 py-0.5 rounded">{info.settings_file}</code>, preserving
              your other settings and saving a backup first.
              {info.restart_hint && ` ${info.restart_hint}`}
            </p>
            <div className="flex items-center gap-2">
              <Button size="sm" onClick={handleAutoConfigure} disabled={configuring || removing || !info.running}>
                {configuring ? <Loader2 className="h-3.5 w-3.5 mr-2 animate-spin" /> : <Settings2 className="h-3.5 w-3.5 mr-2" />}
                Configure {toolName}
              </Button>
              {info.supports_undo && (
                <Button size="sm" variant="outline" onClick={handleRemoveConfig} disabled={configuring || removing}>
                  {removing && <Loader2 className="h-3.5 w-3.5 mr-2 animate-spin" />}
                  Remove
                </Button>
              )}
            </div>
            {autoResult && (
              <div className="rounded-md border p-2 text-xs space-y-1">
                <div className="flex items-start gap-1.5">
                  {autoResult.success
                    ? <CheckCircle2 className="h-3.5 w-3.5 shrink-0 mt-px text-green-500" />
                    : <XCircle className="h-3.5 w-3.5 shrink-0 mt-px text-destructive" />}
                  <span>{autoResult.message}</span>
                </div>
                {autoResult.backup_files.length > 0 && (
                  <p className="text-muted-foreground">Backed up: {autoResult.backup_files.join(", ")}</p>
                )}
              </div>
            )}
          </TabsContent>
        )}

        {/* Quick Start: one-off CLI command */}
        <TabsContent value="temporary" className="space-y-2">
          <p className="text-xs text-muted-foreground">
            Run {toolName} once through the proxy — no files changed:
          </p>
          {oneoff ? (
            <CopyableCode value={oneoff} />
          ) : (
            <p className="text-xs text-destructive">
              {info.running
                ? `${toolName} has no launch-time proxy option — use the Auto or Manual tab.`
                : "Proxy not running."}
            </p>
          )}
          {supportsAuto && (
            <p className="text-[11px] text-muted-foreground">
              This covers a single session. Use Auto or Manual to make it permanent (and to cover
              background agents).
            </p>
          )}
        </TabsContent>

        {/* Manual: the parameters */}
        <TabsContent value="manual" className="space-y-3">
          {info.proxy_url && (
            <div className="space-y-1.5">
              <Label className="text-xs text-muted-foreground">HTTPS_PROXY</Label>
              <CopyableCode value={info.proxy_url} />
            </div>
          )}
          <div className="space-y-1.5">
            <Label className="text-xs text-muted-foreground">
              {info.ca_env_var
                ? `${info.ca_env_var} (root CA to trust)`
                : "Root CA to trust (via your system trust store)"}
            </Label>
            <CopyableCode value={info.ca_cert_path} />
          </div>
          {info.settings_json && info.settings_file && (
            <div className="space-y-1.5">
              <Label className="text-xs text-muted-foreground">
                Or merge into <code>{info.settings_file}</code>
              </Label>
              <CopyableCode value={info.settings_json} />
            </div>
          )}
        </TabsContent>
      </Tabs>
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
  llmMode,
  mcpMode,
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

  const isLlmProxy = llmMode === "proxy"
  const isLlmReverseProxy = llmMode === "reverse_proxy"
  const hasQuickSetup = template && template.setupType !== "generic"
  // Native LLM connect info is shown only for the gateway; proxy clients get the
  // ProxyLlmSetup block instead.
  const showModelsTab = llmMode === "gateway"
  // Direct MCP connect info only for the MCP gateway (not via-LLM/off).
  const showMcpTab = mcpMode === "gateway"

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

  // Fetch models filtered by client's strategy via the real API endpoint.
  // Only the native gateway can call /v1/models — proxy/off clients would 403.
  useEffect(() => {
    if (!secret || !serverConfig || llmMode !== "gateway") return
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
  }, [secret, serverConfig, llmMode])

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

  // For custom/generic clients (no quick setup), determine tab layout
  const manualTabCount = (showModelsTab ? 1 : 0) + (showMcpTab ? 1 : 0)
  const manualGridCols = manualTabCount === 1 ? "grid-cols-1" : "grid-cols-2"
  const defaultManualTab = showModelsTab ? "models" : "mcp"

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
      <CardContent className="space-y-4">
        {/* LLM proxy modes: show proxy instructions instead of the native
            ANTHROPIC_BASE_URL setup. MCP (if a gateway) still renders below. */}
        {isLlmProxy && <ProxyLlmSetup clientUuid={clientUuid} template={template} />}

        {/* Reverse proxy: the client's "connection details" are the provider's
            own address, so there is nothing to copy into an app — the setup
            block is the whole story. */}
        {isLlmReverseProxy && <ReverseProxySetup clientUuid={clientUuid} template={template} />}

        {/* Template-based clients (native gateway): show Quick Setup directly */}
        {!isLlmProxy && !isLlmReverseProxy && hasQuickSetup && template ? (
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
        ) : (showModelsTab || showMcpTab) ? (
        /* Custom/generic clients: show Models and MCP tabs */
        <Tabs defaultValue={defaultManualTab}>
          <TabsList className={`mb-4 grid w-full ${manualGridCols}`}>
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
                            <CopyableCode value="LOCALROUTER_CLIENT_SECRET" />
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
        ) : null}
      </CardContent>
    </Card>
  )
}
