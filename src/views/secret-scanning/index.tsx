import { useState, useEffect, useCallback, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Loader2, ExternalLink } from "lucide-react"
import { FEATURES } from "@/constants/features"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { Badge } from "@/components/ui/Badge"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/Toggle"
import { Label } from "@/components/ui/label"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { cn } from "@/lib/utils"
import { listenSafe } from "@/hooks/useTauriListener"
import { InfoTooltip } from "@/components/ui/info-tooltip"
import { SamplePopupButton } from "@/components/shared/SamplePopupButton"
import type {
  SecretScanningConfig,
  SecretScanAction,
  SecretScanResult,
  SecretRuleMetadata,
  ClientFeatureStatus,
  GetFeatureClientsStatusParams,
} from "@/types/tauri-commands"

interface SecretScanningViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

const ACTION_LABELS: Record<SecretScanAction, { label: string; description: string }> = {
  off: { label: "Off", description: "No scanning" },
  ask: { label: "Ask", description: "Block the request and show a popup for user decision" },
  notify: { label: "Notify", description: "Allow the request but show a notification" },
}

const BUTTON_STYLES: Record<SecretScanAction, string> = {
  ask: "bg-amber-500 text-white",
  notify: "bg-blue-500 text-white",
  off: "bg-red-500 text-white",
}

const DEFAULT_TEST_INPUT = [
  "My AWS key is AKIAIOSFODNN7EXAMPLE",
  "GitHub token: ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij",
  "password = 'super_s3cret_P@ssw0rd!'",
  "UUID (should NOT match): 6c2bffca-115a-44d4-a981-0d01a9e4ef08",
].join("\n")

export function SecretScanningView({ activeSubTab, onTabChange }: SecretScanningViewProps) {
  const [config, setConfig] = useState<SecretScanningConfig | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [testInput, setTestInput] = useState(DEFAULT_TEST_INPUT)
  const [testResult, setTestResult] = useState<SecretScanResult | null>(null)
  const [testLoading, setTestLoading] = useState(false)
  const [patterns, setPatterns] = useState<SecretRuleMetadata[]>([])
  const [clientStatuses, setClientStatuses] = useState<ClientFeatureStatus[]>([])
  const [scanVersion, setScanVersion] = useState(0) // bumped on config save to trigger re-scan

  const tab = activeSubTab || "info"

  const handleTabChange = (newTab: string) => {
    onTabChange?.("secret-scanning", newTab)
  }

  const loadConfig = useCallback(async () => {
    try {
      const data = await invoke<SecretScanningConfig>("get_secret_scanning_config")
      setConfig(data)
    } catch (err) {
      console.error("Failed to load secret scanning config:", err)
      toast.error("Failed to load secret scanning configuration")
    } finally {
      setIsLoading(false)
    }
  }, [])

  const loadPatterns = useCallback(async () => {
    try {
      const data = await invoke<SecretRuleMetadata[]>("get_secret_scanning_patterns")
      setPatterns(data)
    } catch (err) {
      console.error("Failed to load patterns:", err)
    }
  }, [])

  const loadClientStatuses = useCallback(async () => {
    try {
      const data = await invoke<ClientFeatureStatus[]>("get_feature_clients_status", {
        feature: "secret_scanning",
      } satisfies GetFeatureClientsStatusParams)
      setClientStatuses(data)
    } catch (err) {
      console.error("Failed to load client statuses:", err)
    }
  }, [])

  useEffect(() => {
    loadConfig()
    loadPatterns()
    loadClientStatuses()
  }, [loadConfig, loadPatterns, loadClientStatuses])

  useEffect(() => {
    const listeners = [
      listenSafe("clients-changed", loadClientStatuses),
      listenSafe("config-changed", loadClientStatuses),
    ]
    return () => { listeners.forEach(l => l.cleanup()) }
  }, [loadClientStatuses])

  const saveConfig = async (newConfig: SecretScanningConfig) => {
    try {
      await invoke("update_secret_scanning_config", { configJson: JSON.stringify(newConfig) })
      setConfig(newConfig)
      setScanVersion(v => v + 1)
      toast.success("Secret scanning configuration saved")
    } catch (err) {
      console.error("Failed to save secret scanning config:", err)
      toast.error("Failed to save configuration")
    }
  }

  const updateConfig = (updates: Partial<SecretScanningConfig>) => {
    if (!config) return
    saveConfig({ ...config, ...updates })
  }

  // Debounced auto-scan: re-triggers on input change, tab switch, or config save
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  useEffect(() => {
    if (tab !== "try-it-out" || !testInput.trim()) {
      if (!testInput.trim()) setTestResult(null)
      return
    }
    setTestLoading(true)
    if (debounceRef.current) clearTimeout(debounceRef.current)
    debounceRef.current = setTimeout(async () => {
      try {
        const result = await invoke<SecretScanResult>("test_secret_scan", {
          input: testInput,
        })
        result.findings.sort((a, b) =>
          a.category !== b.category
            ? a.category.localeCompare(b.category)
            : a.rule_id.localeCompare(b.rule_id)
        )
        setTestResult(result)
      } catch {
        // Silently ignore during typing
      } finally {
        setTestLoading(false)
      }
    }, 300)
    return () => { if (debounceRef.current) clearTimeout(debounceRef.current) }
  }, [testInput, tab, scanVersion])

  if (isLoading || !config) {
    return (
      <div className="flex flex-col h-full min-h-0 max-w-5xl">
        <div className="flex-shrink-0 pb-4">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <FEATURES.secretScanning.icon className={`h-6 w-6 ${FEATURES.secretScanning.color}`} />
            Secret Scanning
          </h1>
          <p className="text-sm text-muted-foreground">Loading...</p>
        </div>
      </div>
    )
  }

  const isEnabled = config.action !== "off"

  // Group patterns by category
  const patternsByCategory = patterns.reduce<Record<string, SecretRuleMetadata[]>>((acc, p) => {
    const cat = p.category
    if (!acc[cat]) acc[cat] = []
    acc[cat].push(p)
    return acc
  }, {})

  return (
    <div className="flex flex-col h-full min-h-0 max-w-5xl">
      <div className="flex-shrink-0 pb-4">
        <div className="flex items-center gap-2">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <FEATURES.secretScanning.icon className={`h-6 w-6 ${FEATURES.secretScanning.color}`} />
            Secret Scanning
          </h1>
        </div>
        <p className="text-sm text-muted-foreground">
          Detect potential secrets in outbound LLM requests before they reach providers
        </p>
      </div>

      <Tabs value={tab} onValueChange={handleTabChange} className="flex flex-col flex-1 min-h-0">
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="info"><TAB_ICONS.info className={TAB_ICON_CLASS} />Info</TabsTrigger>
          <TabsTrigger value="try-it-out"><TAB_ICONS.tryItOut className={TAB_ICON_CLASS} />Try It Out</TabsTrigger>
          <TabsTrigger value="settings"><TAB_ICONS.settings className={TAB_ICON_CLASS} />Settings</TabsTrigger>
        </TabsList>

        {/* Info Tab */}
        <TabsContent value="info" className="flex-1 min-h-0 mt-4 overflow-y-auto">
          <div className="space-y-4 max-w-2xl">
            <Card>
              <CardHeader>
                <div className="flex items-center justify-between">
                  <div>
                    <CardTitle className="text-base">Secret Scanning Action (Default)</CardTitle>
                    <CardDescription>
                      What to do when a potential secret is detected in an outbound request.
                      This applies to all clients unless overridden per-client.
                    </CardDescription>
                  </div>
                  <InfoTooltip content={
                    <div className="space-y-1">
                      <p><strong>Ask</strong> — Block the request and prompt for decision</p>
                      <p><strong>Notify</strong> — Allow the request but show a warning</p>
                      <p><strong>Off</strong> — No scanning performed</p>
                    </div>
                  }>
                    <div className="inline-flex rounded-md border border-border bg-muted/50">
                      {(Object.keys(ACTION_LABELS) as SecretScanAction[]).map((key, i, arr) => {
                        const isActive = config.action === key
                        return (
                          <button
                            key={key}
                            type="button"
                            onClick={() => updateConfig({ action: key })}
                            className={cn(
                              "px-2 py-0.5 text-xs transition-colors font-medium",
                              isActive
                                ? BUTTON_STYLES[key]
                                : "text-muted-foreground hover:text-foreground hover:bg-muted",
                              i === 0 && "rounded-l-md",
                              i === arr.length - 1 && "rounded-r-md"
                            )}
                          >
                            {ACTION_LABELS[key].label}
                          </button>
                        )
                      })}
                    </div>
                  </InfoTooltip>
                </div>
              </CardHeader>
              {clientStatuses.length > 0 && (
                <CardContent className="pt-0">
                  <div className="border-t pt-3 space-y-1.5">
                    {clientStatuses.map((s) => {
                      const action = (s.effective_value ?? config.action) as SecretScanAction
                      return (
                        <div
                          key={s.client_id}
                          className="flex items-center justify-between py-1 px-2 rounded-md hover:bg-muted/50 group"
                        >
                          <div className="flex items-center gap-2 min-w-0">
                            {onTabChange ? (
                              <button
                                onClick={() => onTabChange("clients", `${s.client_id}|optimize`)}
                                className="text-sm font-medium truncate hover:underline text-left"
                              >
                                {s.client_name}
                              </button>
                            ) : (
                              <span className="text-sm font-medium truncate">{s.client_name}</span>
                            )}
                            {s.source === "override" && (
                              <Badge variant="outline" className="text-[10px] px-1 py-0 shrink-0">
                                Override
                              </Badge>
                            )}
                          </div>
                          <div className="flex items-center gap-2 shrink-0">
                            <Badge
                              variant="outline"
                              className={cn(
                                "text-[10px] px-1.5 py-0",
                                action === "ask" && "border-amber-500/50 text-amber-600",
                                action === "notify" && "border-blue-500/50 text-blue-600",
                                action === "off" && "border-red-500/50 text-red-600",
                              )}
                            >
                              {ACTION_LABELS[action].label}
                            </Badge>
                            {onTabChange && (
                              <button
                                onClick={() => onTabChange("clients", `${s.client_id}|optimize`)}
                                className="opacity-0 group-hover:opacity-100 text-muted-foreground hover:text-foreground transition-opacity"
                                title="Go to client settings"
                              >
                                <ExternalLink className="h-3.5 w-3.5" />
                              </button>
                            )}
                          </div>
                        </div>
                      )
                    })}
                  </div>
                </CardContent>
              )}
            </Card>

            <Card>
              <CardHeader>
                <div className="flex items-center justify-between">
                  <CardTitle className="text-base">Scanner Status</CardTitle>
                  <SamplePopupButton popupType="secret_scan" />
                </div>
              </CardHeader>
              <CardContent>
                <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-sm">
                  <span className="text-muted-foreground">Status:</span>
                  <Badge variant={isEnabled ? "success" : "secondary"}>
                    {isEnabled ? "Active" : "Disabled"}
                  </Badge>
                  <span className="text-muted-foreground">Action:</span>
                  <span className="font-medium">{ACTION_LABELS[config.action].label}</span>
                  <span className="text-muted-foreground">Built-in Patterns:</span>
                  <span>{patterns.filter(p => !p.id.startsWith("custom-")).length}</span>
                </div>
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle className="text-base">How It Works</CardTitle>
              </CardHeader>
              <CardContent className="text-sm text-muted-foreground space-y-2">
                <p>
                  Secret scanning runs <strong>before</strong> the request is sent to the LLM provider.
                  It uses a multi-stage detection pipeline:
                </p>
                <ol className="list-decimal list-inside space-y-1 pl-2">
                  <li><strong>Keyword pre-filter</strong> &mdash; Fast Aho-Corasick scan to identify candidate rules</li>
                  <li><strong>Regex matching</strong> &mdash; {patterns.length} patterns for AWS, GCP, GitHub, Stripe, JWT, etc.</li>
                  <li><strong>Entropy filtering</strong> &mdash; Shannon entropy check per-rule to discard placeholder values</li>
                </ol>
              </CardContent>
            </Card>

            {/* Pattern List */}
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-base">Detection Patterns ({patterns.length})</CardTitle>
                <CardDescription>
                  Built-in patterns derived from common API key, token, and credential formats.
                  Custom rules are added in the Settings tab.
                </CardDescription>
              </CardHeader>
              <CardContent className="pt-0">
                <div className="max-h-80 overflow-y-auto space-y-3 pr-1">
                  {Object.entries(patternsByCategory).sort(([a], [b]) => a.localeCompare(b)).map(([category, rules]) => (
                    <div key={category}>
                      <h4 className="text-xs font-semibold text-muted-foreground mb-1">{category}</h4>
                      <div className="space-y-1">
                        {rules.map(rule => (
                          <div key={rule.id} className="bg-muted/50 rounded px-2 py-1 text-[11px]">
                            <div className="flex items-center justify-between">
                              <span className="font-medium">{rule.description}</span>
                              {rule.entropy_threshold !== null && (
                                <span className="text-muted-foreground">entropy &ge; {rule.entropy_threshold}</span>
                              )}
                            </div>
                            <code className="text-[10px] text-muted-foreground block truncate" title={rule.regex}>{rule.regex}</code>
                            {rule.keywords.length > 0 && (
                              <span className="text-[10px] text-muted-foreground">Keywords: {rule.keywords.join(", ")}</span>
                            )}
                          </div>
                        ))}
                      </div>
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>
          </div>
        </TabsContent>

        {/* Try It Out Tab */}
        <TabsContent value="try-it-out" className="flex-1 min-h-0 mt-4 overflow-y-auto">
          <div className="flex items-center justify-between mb-4 pb-4 border-b">
            <div>
              <span className="text-sm font-medium">Approval Popup Preview</span>
              <p className="text-xs text-muted-foreground mt-0.5">
                Preview the popup shown when secrets are detected with an &ldquo;Ask&rdquo; action
              </p>
            </div>
            <SamplePopupButton popupType="secret_scan" />
          </div>
          <div className="space-y-4 max-w-2xl">
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-base">Test Scanner</CardTitle>
                <CardDescription>
                  Paste text to test the secret scanner. Scans automatically as you type.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                <textarea
                  value={testInput}
                  onChange={(e) => setTestInput(e.target.value)}
                  placeholder="Paste text containing potential secrets..."
                  className="w-full h-32 font-mono text-xs p-2 border rounded bg-background resize-y"
                />
                {testLoading && (
                  <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                    <Loader2 className="h-3 w-3 animate-spin" />
                    Scanning...
                  </div>
                )}
                {testResult && (
                  <div className="space-y-2">
                    <div className="flex items-center gap-2 text-xs">
                      <Badge variant={testResult.findings.length > 0 ? "destructive" : "success"}>
                        {testResult.findings.length} finding(s)
                      </Badge>
                      <span className="text-muted-foreground">
                        {testResult.rules_evaluated} rules in {testResult.scan_duration_ms}ms
                      </span>
                    </div>
                    {testResult.findings.map((f, i) => (
                      <div key={`${f.rule_id}-${i}`} className="bg-muted/50 rounded px-2 py-1.5 text-xs space-y-0.5">
                        <div className="flex items-center justify-between">
                          <span className="font-semibold">{f.rule_description}</span>
                          <Badge variant="outline" className="text-[10px]">{f.category}</Badge>
                        </div>
                        <code className="text-[10px] block bg-background/50 rounded px-1 py-0.5 truncate">
                          {f.matched_text}
                        </code>
                        <code className="text-[10px] block text-muted-foreground truncate" title={f.regex_pattern}>
                          Pattern: {f.regex_pattern}
                        </code>
                        <div className="flex items-center gap-2 text-[10px] text-muted-foreground">
                          <span>Entropy: <span className="font-mono">{f.entropy.toFixed(2)}</span></span>
                          {f.rule_entropy_threshold !== null && (
                            <span>Threshold: <span className="font-mono">{f.rule_entropy_threshold}</span></span>
                          )}
                          {f.keywords.length > 0 && (
                            <span className="ml-auto">Keywords: {f.keywords.join(", ")}</span>
                          )}
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </CardContent>
            </Card>
          </div>
        </TabsContent>

        {/* Settings Tab */}
        <TabsContent value="settings" className="flex-1 min-h-0 mt-4 overflow-y-auto">
          <div className="space-y-4 max-w-2xl">
            {/* Scan Options */}
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-base">Scan Options</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="flex items-center justify-between">
                  <div>
                    <div className="flex items-center gap-1.5">
                      <Label>Scan system messages</Label>
                      <InfoTooltip
                        content="When disabled, only user and assistant messages are scanned. Enable this if you want to catch secrets in system prompts, but note that system messages often intentionally contain API keys for tool use."
                      />
                    </div>
                    <p className="text-xs text-muted-foreground">
                      System messages may contain intentional API keys or credentials
                    </p>
                  </div>
                  <Switch
                    checked={config.scan_system_messages}
                    onCheckedChange={(checked) => updateConfig({ scan_system_messages: checked })}
                  />
                </div>
              </CardContent>
            </Card>

          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}
