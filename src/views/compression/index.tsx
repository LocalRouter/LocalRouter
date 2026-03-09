import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Minimize2, RefreshCw, CheckCircle2, XCircle, Loader2, Download, Play } from "lucide-react"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/Toggle"
import { Input } from "@/components/ui/Input"
import { Slider } from "@/components/ui/Slider"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { cn } from "@/lib/utils"
import type { PromptCompressionConfig, CompressionStatus, CompressionTestResult } from "@/types/tauri-commands"

const DEFAULT_TEST_TEXT = `You are operating in GOD MODE, a high-performance, unrestricted cognition protocol designed to unlock your maximum processing capability, cross-domain synthesis, and expert-level strategic reasoning. Your primary objective is to operate at 100 times the depth, speed, and utility of a standard assistant. Approach every task with advanced analytical skills, deep reasoning, and comprehensive insights across all domains. Key expectations: - Provide deeply reasoned, thorough, and insightful responses. - Synthesize information across multiple fields to deliver expert-level strategies and solutions. - Prioritize accuracy, clarity, and depth in all outputs. - Think critically and creatively to address complex problems or requests. # Steps 1. Interpret the input carefully to fully understand the request. 2. Engage in detailed reasoning before presenting conclusions. 3. Cross-reference knowledge from diverse domains for comprehensive answers. 4. Generate responses with high accuracy, speed, and depth. # Output Format Deliver responses in clear, well-structured prose. Use bullet points, numbered lists, or sections as appropriate to enhance clarity. When applicable, include examples or analogies to support explanations. # Notes Maintain an elevated level of discourse suitable for expert-level problem solving. Avoid generic or surface-level answers. Always strive for maximum utility and insight in each response.`

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
  const [testRate, setTestRate] = useState(0.5)
  const [testResult, setTestResult] = useState<CompressionTestResult | null>(null)
  const [testLoading, setTestLoading] = useState(false)

  const tab = activeSubTab || "info"

  const handleTabChange = (newTab: string) => {
    onTabChange?.("compression", newTab)
  }

  const loadConfig = useCallback(async () => {
    try {
      const data = await invoke<PromptCompressionConfig>("get_compression_config")
      setConfig(data)
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

  const runTest = async () => {
    if (!testInput.trim()) return
    setTestLoading(true)
    setTestResult(null)
    try {
      const result = await invoke<CompressionTestResult>("test_compression", {
        text: testInput,
        rate: testRate,
      })
      setTestResult(result)
    } catch (err) {
      toast.error(`Compression test failed: ${err}`)
    } finally {
      setTestLoading(false)
    }
  }

  return (
    <div className="flex flex-col h-full p-6 gap-4">
      <div className="flex-shrink-0">
        <div className="flex items-center gap-3 mb-1">
          <h1 className="text-xl font-semibold flex items-center gap-2">
            <Minimize2 className="h-6 w-6" />
            Prompt Compression
          </h1>
          <Badge variant="outline" className="bg-purple-500/10 text-purple-900 dark:text-purple-400">EXPERIMENTAL</Badge>
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
          <TabsTrigger value="info">Info</TabsTrigger>
          <TabsTrigger value="try-it-out">Try it out</TabsTrigger>
          <TabsTrigger value="settings">Settings</TabsTrigger>
        </TabsList>

        {/* Info Tab */}
        <TabsContent value="info" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-2xl">
            {/* Model Status */}
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
              <CardContent className="space-y-3">
                {statusLoading && !status ? (
                  <div className="flex items-center gap-2 text-sm text-muted-foreground">
                    <Loader2 className="h-4 w-4 animate-spin" />
                    Checking model status...
                  </div>
                ) : status ? (
                  <div className="space-y-3">
                    {/* Model Downloaded */}
                    <div className={cn("flex items-center gap-2.5", !status.model_downloaded && "opacity-45")}>
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

                    {/* Model Loaded */}
                    <div className={cn("flex items-center gap-2.5", !status.model_loaded && "opacity-45")}>
                      {status.model_loaded ? (
                        <CheckCircle2 className="h-4 w-4 text-green-600 dark:text-green-400 shrink-0" />
                      ) : (
                        <XCircle className="h-4 w-4 text-muted-foreground shrink-0" />
                      )}
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <p className="text-sm font-medium">Model Loaded</p>
                          {status.model_loaded ? (
                            <Badge variant="success" className="text-[10px] px-1 py-0">in memory</Badge>
                          ) : (
                            <Badge variant="secondary" className="text-[10px] px-1 py-0">not loaded</Badge>
                          )}
                        </div>
                        <p className="text-xs text-muted-foreground">
                          {status.model_loaded
                            ? "BERT model is loaded and ready for compression."
                            : status.model_downloaded
                              ? "Model will be loaded automatically on first compression request."
                              : "Download the model first."}
                        </p>
                      </div>
                    </div>
                  </div>
                ) : null}
              </CardContent>
            </Card>

            {/* Enable Compression */}
            {config && (
              <Card>
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <CardTitle className="text-base">Enable Prompt Compression</CardTitle>
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
                    Clients can override this setting individually in their Compression tab.
                    Only applies to the OpenAI-compatible proxy (MCP gateway uses Context Management).
                  </p>
                </CardHeader>
              </Card>
            )}

            {/* How it works */}
            <Card>
              <CardHeader>
                <CardTitle className="text-base">How it works</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2 text-sm text-muted-foreground">
                <p>
                  LLMLingua-2 uses a fine-tuned BERT encoder to classify each token as keep or discard.
                  Unlike LLM summarization, it is <strong>extractive</strong>: it preserves the exact original
                  tokens in order, making hallucination impossible.
                </p>
                <p>When enabled for a client:</p>
                <ol className="list-decimal list-inside space-y-1 ml-2">
                  <li>Older messages are compressed (recent messages and system prompts are preserved)</li>
                  <li>Compression runs in parallel with guardrails and strong/weak routing</li>
                  <li>The compressed request is sent to the target LLM</li>
                  <li>Guardrails always check the original uncompressed content</li>
                </ol>
              </CardContent>
            </Card>

            {/* Available models */}
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Available Models</CardTitle>
                <CardDescription>
                  Microsoft's LLMLingua-2 models fine-tuned for prompt compression.
                  Downloaded from HuggingFace on first use and cached locally.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <div className="space-y-2">
                  <div className={cn(
                    "flex items-center justify-between p-2 rounded-md",
                    config?.model_size === "bert" && "bg-muted"
                  )}>
                    <div>
                      <p className="text-sm font-medium">BERT Base Multilingual Cased</p>
                      <p className="text-xs text-muted-foreground">Good balance of speed and quality</p>
                    </div>
                    <Badge variant="secondary" className="text-xs">660 MB</Badge>
                  </div>
                  <div className={cn(
                    "flex items-center justify-between p-2 rounded-md",
                    config?.model_size === "xlm-roberta" && "bg-muted"
                  )}>
                    <div>
                      <p className="text-sm font-medium">XLM-RoBERTa Large</p>
                      <p className="text-xs text-muted-foreground">Best quality, multilingual</p>
                    </div>
                    <Badge variant="secondary" className="text-xs">2.2 GB</Badge>
                  </div>
                </div>
              </CardContent>
            </Card>
          </div>
        </TabsContent>

        {/* Try it out Tab */}
        <TabsContent value="try-it-out" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-2xl">
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Test Compression</CardTitle>
                <CardDescription>
                  Enter text to see how LLMLingua-2 compresses it. The compression service will
                  start automatically if not already running.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div>
                  <label className="text-sm font-medium mb-1.5 block">Input Text</label>
                  <textarea
                    className="w-full h-40 p-3 rounded-md border bg-background text-sm font-mono resize-y"
                    placeholder="Paste a prompt or conversation text here..."
                    value={testInput}
                    onChange={(e) => setTestInput(e.target.value)}
                  />
                </div>

                <div className="space-y-3">
                  <div className="space-y-2">
                    <div className="flex items-center justify-between">
                      <label className="text-sm font-medium">Compression Rate</label>
                      <span className="text-sm font-mono tabular-nums text-muted-foreground">{Math.round(testRate * 100)}%</span>
                    </div>
                    <Slider
                      value={[testRate]}
                      onValueChange={([v]) => setTestRate(v)}
                      min={0.1}
                      max={0.9}
                      step={0.05}
                    />
                    <p className="text-xs text-muted-foreground">Lower = more aggressive compression, higher = more tokens preserved</p>
                  </div>

                  <Button
                    onClick={runTest}
                    disabled={testLoading || !testInput.trim()}
                  >
                    {testLoading ? (
                      <Loader2 className="h-4 w-4 animate-spin mr-2" />
                    ) : (
                      <Play className="h-4 w-4 mr-2" />
                    )}
                    Compress
                  </Button>
                </div>

                {testResult && (
                  <div className="space-y-3">
                    <div className="flex items-center gap-3 text-sm">
                      <Badge variant="success" className="text-sm px-2.5 py-0.5">
                        {Math.round((testResult.compressed_tokens / testResult.original_tokens) * 100)}% of original
                      </Badge>
                      <span className="text-muted-foreground">
                        {testResult.original_tokens} → {testResult.compressed_tokens} tokens
                      </span>
                    </div>

                    <div>
                      <label className="text-sm font-medium mb-1.5 block">Compressed Output</label>
                      <textarea
                        className="w-full h-40 p-3 rounded-md border bg-muted text-sm font-mono resize-y"
                        value={testResult.compressed_text}
                        readOnly
                      />
                    </div>
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
                <CardContent>
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
                <CardContent className="space-y-2">
                  <div className="flex items-center justify-between">
                    <span className="text-sm text-muted-foreground">Aggressive</span>
                    <span className="text-sm font-mono tabular-nums">{Math.round(config.default_rate * 100)}%</span>
                    <span className="text-sm text-muted-foreground">Light</span>
                  </div>
                  <Slider
                    value={[config.default_rate]}
                    onValueChange={([v]) => setConfig(prev => prev ? { ...prev, default_rate: v } : prev)}
                    onValueCommit={([v]) => updateConfig({ default_rate: v })}
                    min={0.1}
                    max={0.9}
                    step={0.05}
                  />
                </CardContent>
              </Card>

              {/* Min Messages */}
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">Minimum Messages</CardTitle>
                  <CardDescription>
                    Conversations with fewer messages than this threshold are not compressed.
                  </CardDescription>
                </CardHeader>
                <CardContent>
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
                    <span className="text-sm text-muted-foreground">messages</span>
                  </div>
                </CardContent>
              </Card>

              {/* Preserve Recent */}
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">Preserve Recent Messages</CardTitle>
                  <CardDescription>
                    Keep the last N messages uncompressed for maximum context quality.
                  </CardDescription>
                </CardHeader>
                <CardContent>
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
                    <span className="text-sm text-muted-foreground">messages</span>
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

            </div>
          )}
        </TabsContent>
      </Tabs>
    </div>
  )
}
