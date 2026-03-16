import { useState, useEffect, useCallback, useRef, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { RefreshCw, CheckCircle2, XCircle, Loader2, Download } from "lucide-react"
import { FEATURES } from "@/constants/features"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { Badge } from "@/components/ui/Badge"
import { ExperimentalBadge } from "@/components/shared/ExperimentalBadge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/Toggle"
import { Input } from "@/components/ui/Input"
import { PresetSlider } from "@/components/ui/PresetSlider"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { cn } from "@/lib/utils"
import { FeatureClientsCard } from "@/components/shared/FeatureClientsCard"
import type { PromptCompressionConfig, CompressionStatus, CompressionTestResult, TestCompressionParams } from "@/types/tauri-commands"
import { COMPRESSION_PRESETS, COMPRESSION_REQUIREMENTS } from "@/components/compression/types"

const DEFAULT_TEST_TEXT = `The user reported an error described as "a persistent connection timeout occurring on the main authentication service endpoint" when trying to log in. Here is the relevant code that handles the authentication flow:

\`\`\`python
def authenticate(user, password):
    hash = bcrypt.hashpw(password.encode(), bcrypt.gensalt())
    return db.verify(user, hash)
\`\`\`

The core problem is that \`bcrypt.gensalt()\` generates a completely new random salt on every single call, so the resulting hash will never match the stored value in the database. The recommended fix is to use \`bcrypt.checkpw()\` instead, which handles the salt extraction internally and compares correctly. As the official documentation clearly states: "The checkpw function is the recommended way to compare a plaintext password against a previously stored hashed value and it will return True only if the two values match correctly." The \`verify\` method in the database access layer should also be updated to follow this pattern.`

interface CompressionViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function CompressionView({ activeSubTab, onTabChange }: CompressionViewProps) {
  const [config, setConfig] = useState<PromptCompressionConfig | null>(null)
  const [status, setStatus] = useState<CompressionStatus | null>(null)
  const [statusLoading, setStatusLoading] = useState(true)
  const [installing, setInstalling] = useState(false)
  const [saving, setSaving] = useState(false)
  const [testInput, setTestInput] = useState(DEFAULT_TEST_TEXT)
  const [testRate, setTestRate] = useState(0.8)
  const [testResult, setTestResult] = useState<CompressionTestResult | null>(null)
  const [testLoading, setTestLoading] = useState(false)
  const [preserveQuoted, setPreserveQuoted] = useState(true)
  const [compressionNotice, setCompressionNotice] = useState(true)
  const [showAnnotated, setShowAnnotated] = useState(true)

  const tab = activeSubTab || "info"

  const handleTabChange = (newTab: string) => {
    onTabChange?.("compression", newTab)
  }

  const loadConfig = useCallback(async () => {
    try {
      const data = await invoke<PromptCompressionConfig>("get_compression_config")
      setConfig(data)
      setPreserveQuoted(data.preserve_quoted_text)
      setCompressionNotice(data.compression_notice)
    } catch (err) {
      console.error("Failed to load compression config:", err)
    }
  }, [])

  const loadStatus = useCallback(async () => {
    setStatusLoading(true)
    try {
      const data = await invoke<CompressionStatus>("get_compression_status")
      setStatus(data)
    } catch (err) {
      console.error("Failed to load compression status:", err)
    } finally {
      setStatusLoading(false)
    }
  }, [])

  useEffect(() => {
    loadConfig()
    loadStatus()
  }, [loadConfig, loadStatus])

  const updateConfig = async (updates: Partial<PromptCompressionConfig>) => {
    if (!config) return
    setSaving(true)
    const newConfig = { ...config, ...updates }
    try {
      await invoke("update_compression_config", { configJson: JSON.stringify(newConfig) })
      setConfig(newConfig)
      // Rebuild engine if enabled state or model changed
      if ("enabled" in updates || "model_size" in updates) {
        await invoke("rebuild_compression_engine")
        await loadStatus()
      }
    } catch (err) {
      toast.error(`Failed to update config: ${err}`)
    } finally {
      setSaving(false)
    }
  }

  const runTest = useCallback(async (text: string, rate: number, pq: boolean, cn: boolean) => {
    if (!text.trim()) {
      setTestResult(null)
      return
    }
    setTestLoading(true)
    try {
      const result = await invoke<CompressionTestResult>("test_compression", {
        text,
        rate,
        preserveQuoted: pq,
        compressionNotice: cn,
      } satisfies TestCompressionParams as Record<string, unknown>)
      setTestResult(result)
    } catch (err) {
      toast.error(`Compression test failed: ${err}`)
    } finally {
      setTestLoading(false)
    }
  }, [])

  // Debounced auto-compress on input or rate changes
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current)
    debounceRef.current = setTimeout(() => {
      runTest(testInput, testRate, preserveQuoted, compressionNotice)
    }, 500)
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current)
    }
  }, [testInput, testRate, preserveQuoted, compressionNotice, runTest])

  // Split input into words with preceding whitespace for rendering with kept/deleted styling
  const inputTokens = useMemo(() => {
    const tokens: { word: string; precedingWs: string }[] = []
    const regex = /(\s*?)(\S+)/g
    let match
    while ((match = regex.exec(testInput)) !== null) {
      tokens.push({ word: match[2], precedingWs: match[1] })
    }
    return tokens
  }, [testInput])

  return (
    <div className="flex flex-col h-full min-h-0 gap-4 max-w-5xl">
      <div className="flex-shrink-0">
        <div className="flex items-center gap-3 mb-1">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <FEATURES.compression.icon className={`h-6 w-6 ${FEATURES.compression.color}`} />
            Prompt Compression
          </h1>
          <ExperimentalBadge />
        </div>
        <p className="text-sm text-muted-foreground">
          Reduce input tokens using LLMLingua-2 extractive compression for the OpenAI-compatible proxy
        </p>
      </div>

      <Tabs
        value={tab}
        onValueChange={handleTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="info"><TAB_ICONS.info className={TAB_ICON_CLASS} />Info</TabsTrigger>
          <TabsTrigger value="try-it-out"><TAB_ICONS.tryItOut className={TAB_ICON_CLASS} />Try It Out</TabsTrigger>
          <TabsTrigger value="settings"><TAB_ICONS.settings className={TAB_ICON_CLASS} />Settings</TabsTrigger>
        </TabsList>

        {/* Info Tab */}
        <TabsContent value="info" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-2xl">
            {/* Warning */}
            <Card className="border-orange-600/50 bg-orange-500/5">
              <CardHeader className="pb-3">
                <CardTitle className="text-sm text-orange-900 dark:text-orange-400">Important Considerations</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2 text-sm text-muted-foreground">
                <p>
                  Compression compacts only <strong>older messages</strong> in the conversation history before they reach the LLM. The model will <strong>not see
                  the exact text you sent</strong> &mdash; it receives a shortened version with less important tokens stripped out. Recent messages are preserved as-is.
                </p>
                <ul className="list-disc list-inside space-y-1 ml-2">
                  <li>Not recommended when exact wording, specific details, or precise instructions matter</li>
                  <li>Best suited for <strong>conversational settings</strong> where older messages provide general context</li>
                  <li>May degrade performance on tasks requiring careful attention to every word (e.g. code generation, legal text, structured data)</li>
                  <li>Quoted strings and code blocks can be <strong>force-preserved</strong> during compression (enabled by default in Settings)</li>
                </ul>
              </CardContent>
            </Card>

            {/* Model Loaded (in-memory status) */}
            <Card>
              <CardHeader className="pb-3">
                <div className="flex items-center justify-between">
                  <CardTitle className="text-base">Model Status</CardTitle>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={loadStatus}
                    className="h-7 w-7 p-0"
                    disabled={statusLoading}
                  >
                    {statusLoading ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <RefreshCw className="h-3.5 w-3.5" />
                    )}
                  </Button>
                </div>
                <CardDescription>
                  LLMLingua-2 runs natively via Candle (pure-Rust ML framework). No external dependencies required.
                </CardDescription>
              </CardHeader>
              <CardContent>
                {statusLoading && !status ? (
                  <div className="flex items-center gap-2 text-sm text-muted-foreground">
                    <Loader2 className="h-4 w-4 animate-spin" />
                    Checking model status...
                  </div>
                ) : status ? (
                  <div className={cn("flex items-center gap-2.5", !status.model_loaded && "opacity-45")}>
                    {status.model_loaded ? (
                      <CheckCircle2 className="h-4 w-4 text-green-600 dark:text-green-400 shrink-0" />
                    ) : (
                      <XCircle className="h-4 w-4 text-muted-foreground shrink-0" />
                    )}
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <p className="text-sm font-medium">Model Loaded in Memory</p>
                        {status.model_loaded ? (
                          <Badge variant="success" className="text-[10px] px-1 py-0">in memory</Badge>
                        ) : (
                          <Badge variant="secondary" className="text-[10px] px-1 py-0">not loaded</Badge>
                        )}
                      </div>
                      <p className="text-xs text-muted-foreground">
                        {status.model_loaded
                          ? "BERT model is loaded into memory and ready for compression requests."
                          : status.model_downloaded
                            ? "Model is downloaded but not loaded into memory. It will be loaded automatically on the first compression request."
                            : "Model is not downloaded yet. Go to Settings to download it."}
                      </p>
                    </div>
                  </div>
                ) : null}
              </CardContent>
            </Card>

            {/* Global Enable Compression */}
            {config && (
              <Card>
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <CardTitle className="text-base">Default: Prompt Compression</CardTitle>
                    <Switch
                      checked={config.enabled}
                      onCheckedChange={(enabled) => updateConfig({ enabled })}
                      disabled={saving}
                    />
                  </div>
                  <CardDescription>
                    Uses{" "}
                    <a href="https://github.com/microsoft/LLMLingua" target="_blank" rel="noopener noreferrer" className="text-blue-500 hover:underline">LLMLingua-2</a>{" "}
                    token-level compression to reduce input tokens for <code className="px-1 py-0.5 rounded bg-muted text-xs">/v1/chat/completions</code> requests.
                    Extractive compression keeps exact original tokens &mdash; no hallucination possible.
                  </CardDescription>
                  <p className="text-xs text-muted-foreground mt-1">
                    This is the global default. Individual clients can override this in their Compression tab
                    (enable compression for specific clients even when globally off, or disable it for specific clients when globally on).
                    Only applies to the OpenAI-compatible proxy (MCP gateway uses Context Management).
                  </p>
                </CardHeader>
              </Card>
            )}

            <FeatureClientsCard feature="prompt_compression" clientTab="optimize" onNavigateToClient={onTabChange} />
          </div>
        </TabsContent>

        {/* Try it out Tab */}
        <TabsContent value="try-it-out" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-2xl">
            {/* Input */}
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Input</CardTitle>
                <CardDescription>
                  Enter text to see how LLMLingua-2 compresses it. The compression service will
                  start automatically if not already running.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <textarea
                  className="w-full h-80 p-3 rounded-md border bg-background text-sm font-mono resize-y"
                  placeholder="Paste a prompt or conversation text here..."
                  value={testInput}
                  onChange={(e) => setTestInput(e.target.value)}
                />
              </CardContent>
            </Card>

            {/* Settings */}
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Settings</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <PresetSlider
                  label="Compression Rate"
                  value={testRate}
                  onChange={setTestRate}
                  presets={COMPRESSION_PRESETS}
                  min={0}
                  max={1}
                  step={0.01}
                  minLabel="More compression"
                  maxLabel="More tokens preserved"
                  formatValue={(v) => `${Math.round(v * 100)}%`}
                />

                <div className="flex items-center gap-6">
                  <div className="flex items-center gap-2">
                    <Switch
                      checked={preserveQuoted}
                      onCheckedChange={setPreserveQuoted}
                    />
                    <label className="text-sm">Preserve quoted content</label>
                  </div>
                  <div className="flex items-center gap-2">
                    <Switch
                      checked={compressionNotice}
                      onCheckedChange={setCompressionNotice}
                    />
                    <label className="text-sm">Show compression notice</label>
                  </div>
                </div>
              </CardContent>
            </Card>

            {/* Output */}
            <Card>
              <CardHeader>
                <div className="flex items-center justify-between">
                  <div className={cn("flex items-center gap-3", testLoading && "opacity-50")}>
                    <CardTitle className="text-base">Output</CardTitle>
                    {testLoading && (
                      <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
                    )}
                    {testResult && (
                      <>
                        <Badge variant="success" className="text-sm px-2.5 py-0.5">
                          {Math.round((testResult.compressed_tokens / testResult.original_tokens) * 100)}% of original
                        </Badge>
                        <span className="text-sm text-muted-foreground">
                          {testResult.original_tokens} → {testResult.compressed_tokens} tokens
                          {testResult.protected_indices.length > 0 && (
                            <> ({testResult.protected_indices.length} protected)</>
                          )}
                        </span>
                      </>
                    )}
                  </div>
                  {testResult && (
                    <div className="flex items-center gap-2">
                      <label className="text-xs text-muted-foreground">Annotated</label>
                      <Switch
                        checked={showAnnotated}
                        onCheckedChange={setShowAnnotated}
                      />
                    </div>
                  )}
                </div>
              </CardHeader>
              <CardContent className="space-y-4">
                {testResult ? (
                  <>
                    {showAnnotated ? (
                    /* Annotated view */
                    <div>
                      <div className="w-full h-80 p-3 rounded-md border bg-muted text-sm font-mono overflow-y-auto whitespace-pre-wrap">
                        {(() => {
                          const keptSet = new Set(testResult.kept_indices)
                          const protectedSet = new Set(testResult.protected_indices)
                          return (
                            <>
                              {compressionNotice && (
                                <span className="text-blue-600 dark:text-blue-400 font-semibold">[abridged] </span>
                              )}
                              {inputTokens.map(({ word, precedingWs }, idx) => (
                                <span key={idx}>
                                  {precedingWs}
                                  {keptSet.has(idx) ? (
                                    protectedSet.has(idx) ? (
                                      <span
                                        className="bg-purple-500/15 text-purple-900 dark:text-purple-300 rounded px-0.5"
                                        title="Protected (quoted/code content)"
                                      >{word}</span>
                                    ) : (
                                      <span>{word}</span>
                                    )
                                  ) : (
                                    <span className="line-through text-red-500/40">{word}</span>
                                  )}
                                </span>
                              ))}
                            </>
                          )
                        })()}
                      </div>
                      <div className="flex items-center gap-4 text-xs text-muted-foreground mt-1.5">
                        {compressionNotice && (
                          <span className="flex items-center gap-1.5">
                            <span className="inline-block w-3 h-3 rounded bg-blue-500/15 border border-blue-500/30 text-[8px] font-bold text-blue-600 dark:text-blue-400 leading-none text-center">a</span>
                            Abridged
                          </span>
                        )}
                        <span className="flex items-center gap-1.5">
                          <span className="inline-block w-3 h-3 rounded bg-purple-500/15 border border-purple-500/30" />
                          Protected
                        </span>
                        <span className="flex items-center gap-1.5">
                          <span className="inline-block w-3 h-3 rounded bg-foreground/10 border border-foreground/20" />
                          Kept
                        </span>
                        <span className="flex items-center gap-1.5">
                          <span className="inline-block w-3 h-3 rounded bg-red-500/10 border border-red-500/30 line-through" />
                          Removed
                        </span>
                      </div>
                    </div>
                    ) : (
                    /* Compressed view (what LLM sees) */
                    <div>
                      <div className="w-full h-80 p-3 rounded-md border bg-muted text-sm font-mono overflow-y-auto whitespace-pre-wrap">
                        {compressionNotice && "[abridged] "}
                        {testResult.compressed_text}
                      </div>
                    </div>
                    )}
                  </>
                ) : (
                  <div className="w-full h-80 p-3 rounded-md border bg-muted text-sm font-mono overflow-y-auto whitespace-pre-wrap">
                    <span className="text-muted-foreground">
                      {testInput.trim() ? "" : "Enter text above to see compression results..."}
                    </span>
                  </div>
                )}
              </CardContent>
            </Card>
          </div>
        </TabsContent>

        {/* Settings Tab */}
        <TabsContent value="settings" className="flex-1 min-h-0 mt-4">
          {config && (
            <div className="space-y-4 max-w-2xl">
              {/* Model Selection */}
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">Model</CardTitle>
                  <CardDescription>
                    LLMLingua-2 uses a BERT encoder for token classification. The model runs natively
                    via Candle (pure-Rust ML) with Metal acceleration on macOS. Changing the model
                    requires downloading new weights and reloading.
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  <RadioGroup
                    value={config.model_size}
                    onValueChange={(v) => updateConfig({ model_size: v as "bert" | "xlm-roberta" })}
                    className="space-y-2"
                  >
                    <label
                      className={cn(
                        "flex items-center gap-3 p-2 rounded-md cursor-pointer hover:bg-muted/50 transition-colors",
                        config.model_size === "bert" && "bg-muted"
                      )}
                    >
                      <RadioGroupItem value="bert" />
                      <div className="flex-1">
                        <p className="text-sm font-medium">BERT Base Multilingual Cased</p>
                        <p className="text-xs text-muted-foreground">Good balance of speed and quality</p>
                      </div>
                      <Badge variant="secondary" className="text-xs">660 MB</Badge>
                    </label>
                    <label
                      className={cn(
                        "flex items-center gap-3 p-2 rounded-md cursor-pointer hover:bg-muted/50 transition-colors",
                        config.model_size === "xlm-roberta" && "bg-muted"
                      )}
                    >
                      <RadioGroupItem value="xlm-roberta" />
                      <div className="flex-1">
                        <p className="text-sm font-medium">XLM-RoBERTa Large</p>
                        <p className="text-xs text-muted-foreground">Best quality, multilingual</p>
                      </div>
                      <Badge variant="secondary" className="text-xs">2.2 GB</Badge>
                    </label>
                  </RadioGroup>

                  {/* Model Downloaded Status */}
                  {status && (
                    <div className={cn("flex items-center gap-2.5 pt-2 border-t", !status.model_downloaded && "opacity-45")}>
                      {status.model_downloaded ? (
                        <CheckCircle2 className="h-4 w-4 text-green-600 dark:text-green-400 shrink-0" />
                      ) : (
                        <XCircle className="h-4 w-4 text-muted-foreground shrink-0" />
                      )}
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <p className="text-sm font-medium">Model Downloaded</p>
                          {status.model_downloaded ? (
                            <Badge variant="success" className="text-[10px] px-1 py-0">
                              {status.model_size_bytes ? `${(status.model_size_bytes / 1024 / 1024).toFixed(0)} MB` : "ready"}
                            </Badge>
                          ) : (
                            <Badge variant="secondary" className="text-[10px] px-1 py-0">not downloaded</Badge>
                          )}
                        </div>
                        <p className="text-xs text-muted-foreground truncate">
                          {status.model_repo}
                        </p>
                      </div>
                      {!status.model_downloaded && (
                        <Button
                          variant="outline"
                          size="sm"
                          className="shrink-0 ml-2"
                          disabled={installing}
                          onClick={async () => {
                            setInstalling(true)
                            try {
                              await invoke("install_compression")
                              toast.success("Compression model downloaded")
                              await loadStatus()
                            } catch (err) {
                              toast.error(`Download failed: ${err}`)
                            } finally {
                              setInstalling(false)
                            }
                          }}
                        >
                          {installing ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin mr-1.5" />
                          ) : (
                            <Download className="h-3.5 w-3.5 mr-1.5" />
                          )}
                          {installing ? "Downloading..." : "Download"}
                        </Button>
                      )}
                    </div>
                  )}

                  {/* Resource Requirements */}
                  {(() => {
                    const reqs = COMPRESSION_REQUIREMENTS[config.model_size as keyof typeof COMPRESSION_REQUIREMENTS];
                    return (
                      <div className="grid grid-cols-2 gap-3 text-xs text-muted-foreground pt-2 border-t">
                        <div>
                          <span>Cold Start:</span>{" "}
                          <span className="font-medium text-foreground">{reqs.COLD_START_SECS}s</span>
                        </div>
                        <div>
                          <span>Disk Space:</span>{" "}
                          <span className="font-medium text-foreground">{reqs.DISK_GB} GB</span>
                        </div>
                        <div>
                          <span>Latency:</span>{" "}
                          <span className="font-medium text-foreground">{reqs.PER_REQUEST_MS}ms per message</span>
                        </div>
                        <div>
                          <span>Memory:</span>{" "}
                          <span className="font-medium text-foreground">{reqs.MEMORY_GB} GB (when loaded)</span>
                        </div>
                      </div>
                    );
                  })()}
                </CardContent>
              </Card>

              {/* Compression Rate */}
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">Default Compression Rate</CardTitle>
                  <CardDescription>
                    Controls how aggressively tokens are removed. Lower values mean more compression
                    (fewer tokens kept). Clients can override this per-client.
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <PresetSlider
                    label="Compression rate"
                    value={config.default_rate}
                    onChange={(v) => setConfig(prev => prev ? { ...prev, default_rate: v } : prev)}
                    onCommit={(v) => updateConfig({ default_rate: v })}
                    presets={COMPRESSION_PRESETS}
                    min={0}
                    max={1}
                    step={0.01}
                    minLabel="More compression"
                    maxLabel="More tokens preserved"
                    formatValue={(v) => `${Math.round(v * 100)}%`}
                    disabled={saving}
                  />
                </CardContent>
              </Card>

              {/* Message Handling */}
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">Message Handling</CardTitle>
                  <CardDescription>
                    Controls which messages get compressed. Compression only activates once a conversation
                    reaches the minimum message count. The most recent messages are always kept intact
                    to preserve immediate context quality. Messages shorter than the minimum word count
                    are skipped entirely.
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  <div className="grid grid-cols-2 gap-4">
                    <div className="space-y-1.5">
                      <label className="text-sm font-medium">Minimum messages</label>
                      <div className="flex gap-2 items-center">
                        <Input
                          type="number"
                          defaultValue={config.min_messages}
                          key={`min-${config.min_messages}`}
                          onBlur={(e) => {
                            const v = parseInt(e.target.value)
                            if (!isNaN(v) && v >= 0 && v !== config.min_messages) {
                              updateConfig({ min_messages: v })
                            }
                          }}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") (e.target as HTMLInputElement).blur()
                          }}
                          className="w-24"
                          min={0}
                        />
                        <span className="text-xs text-muted-foreground">to activate</span>
                      </div>
                    </div>
                    <div className="space-y-1.5">
                      <label className="text-sm font-medium">Preserve recent</label>
                      <div className="flex gap-2 items-center">
                        <Input
                          type="number"
                          defaultValue={config.preserve_recent}
                          key={`preserve-${config.preserve_recent}`}
                          onBlur={(e) => {
                            const v = parseInt(e.target.value)
                            if (!isNaN(v) && v >= 0 && v !== config.preserve_recent) {
                              updateConfig({ preserve_recent: v })
                            }
                          }}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") (e.target as HTMLInputElement).blur()
                          }}
                          className="w-24"
                          min={0}
                        />
                        <span className="text-xs text-muted-foreground">uncompressed</span>
                      </div>
                    </div>
                  </div>
                  <div className="space-y-1.5">
                    <label className="text-sm font-medium">Min message size</label>
                    <div className="flex gap-2 items-center">
                      <Input
                        type="number"
                        defaultValue={config.min_message_words}
                        key={`minwords-${config.min_message_words}`}
                        onBlur={(e) => {
                          const v = parseInt(e.target.value)
                          if (!isNaN(v) && v >= 1 && v !== config.min_message_words) {
                            updateConfig({ min_message_words: v })
                          }
                        }}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") (e.target as HTMLInputElement).blur()
                        }}
                        className="w-24"
                        min={1}
                      />
                      <span className="text-xs text-muted-foreground">words (shorter messages are skipped)</span>
                    </div>
                  </div>
                </CardContent>
              </Card>

              {/* Compress System Prompt */}
              <Card>
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <CardTitle className="text-base">Compress System Prompts</CardTitle>
                    <Switch
                      checked={config.compress_system_prompt}
                      onCheckedChange={(v) => updateConfig({ compress_system_prompt: v })}
                      disabled={saving}
                    />
                  </div>
                  <CardDescription>
                    Also compress system prompt messages. Disabled by default since system prompts
                    contain critical instructions.
                  </CardDescription>
                </CardHeader>
              </Card>

              {/* Preserve Quoted & Code Content */}
              <Card>
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <CardTitle className="text-base">Preserve Quoted & Code Content</CardTitle>
                    <Switch
                      checked={config.preserve_quoted_text}
                      onCheckedChange={(v) => updateConfig({ preserve_quoted_text: v })}
                      disabled={saving}
                    />
                  </div>
                  <CardDescription>
                    Force-keep words inside quoted strings, inline code, and fenced code blocks during
                    compression. Prevents corruption of exact text within delimiters. Supports Unicode
                    quotes, guillemets, CJK brackets, and more.
                  </CardDescription>
                </CardHeader>
              </Card>

              {/* Compression Notice */}
              <Card>
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <CardTitle className="text-base">Compression Notice</CardTitle>
                    <Switch
                      checked={config.compression_notice}
                      onCheckedChange={(v) => updateConfig({ compression_notice: v })}
                      disabled={saving}
                    />
                  </div>
                  <CardDescription>
                    Prepend <code className="px-1 py-0.5 rounded bg-muted text-xs">[abridged]</code> to
                    each compressed message to signal the content is not verbatim. Useful for models that
                    may be confused by compressed text.
                  </CardDescription>
                </CardHeader>
              </Card>

            </div>
          )}
        </TabsContent>
      </Tabs>
    </div>
  )
}
