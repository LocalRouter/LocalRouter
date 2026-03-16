import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { Input } from "@/components/ui/Input"
import { Button } from "@/components/ui/Button"
import { Switch } from "@/components/ui/Toggle"
import { AlertTriangle, CheckCircle2, Circle, FolderOpen, Loader2, XCircle } from "lucide-react"
import type { MemoryConfig, MemorySetupProgress, MemoryStatus, UpdateMemoryConfigParams } from "@/types/tauri-commands"

type SetupStepStatus = "idle" | "checking" | "installing" | "ok" | "error"

interface SetupState {
  python: { status: SetupStepStatus; version?: string; error?: string }
  memsearch: { status: SetupStepStatus; version?: string; error?: string }
  model: { status: SetupStepStatus; error?: string }
}

const defaultConfig: MemoryConfig = {
  embedding: { type: "onnx" as const },
  auto_start_daemon: true,
  search_top_k: 5,
  session_inactivity_minutes: 180,
  max_session_minutes: 480,
  recall_tool_name: "MemoryRecall",
  compaction: null,
}

export function MemoryTab() {
  const [config, setConfig] = useState<MemoryConfig>(defaultConfig)
  const [isLoading, setIsLoading] = useState(true)
  const [, setIsSaving] = useState(false)
  const [isSettingUp, setIsSettingUp] = useState(false)
  const [, setStatus] = useState<MemoryStatus | null>(null)
  const [setup, setSetup] = useState<SetupState>({
    python: { status: "idle" },
    memsearch: { status: "idle" },
    model: { status: "idle" },
  })

  useEffect(() => {
    loadConfig()
    loadStatus()
  }, [])

  // Listen for setup progress events
  useEffect(() => {
    const unlisteners: (() => void)[] = []

    listen<MemorySetupProgress>("memory-setup-progress", (event) => {
      const { step, status: stepStatus, version, error } = event.payload
      setSetup((prev) => ({
        ...prev,
        [step]: { status: stepStatus, version, error },
      }))
    }).then((unlisten) => unlisteners.push(unlisten))

    return () => {
      unlisteners.forEach((fn) => fn())
    }
  }, [])

  const loadConfig = async () => {
    try {
      const result = await invoke<MemoryConfig>("get_memory_config")
      setConfig(result)
    } catch (error) {
      console.error("Failed to load memory config:", error)
    } finally {
      setIsLoading(false)
    }
  }

  const loadStatus = async () => {
    try {
      const result = await invoke<MemoryStatus>("get_memory_status")
      setStatus(result)
      // Pre-populate setup state from status
      setSetup({
        python: { status: result.python_ok ? "ok" : "idle" },
        memsearch: {
          status: result.memsearch_installed ? "ok" : "idle",
          version: result.memsearch_version ?? undefined,
        },
        model: { status: result.model_ready ? "ok" : "idle" },
      })
    } catch (error) {
      console.error("Failed to load memory status:", error)
    }
  }

  const saveConfig = async (newConfig: MemoryConfig) => {
    try {
      setIsSaving(true)
      await invoke("update_memory_config", {
        configJson: JSON.stringify(newConfig),
      } satisfies UpdateMemoryConfigParams)
      setConfig(newConfig)
      toast.success("Memory configuration saved")
    } catch (error: any) {
      toast.error(`Failed to save: ${error.message || error}`)
    } finally {
      setIsSaving(false)
    }
  }

  const runSetup = async () => {
    setIsSettingUp(true)
    try {
      await invoke("memory_setup")
      toast.success("Memory setup complete")
      loadStatus()
    } catch (error: any) {
      toast.error(`Setup failed: ${error.message || error}`)
    } finally {
      setIsSettingUp(false)
    }
  }

  const openMemoryFolder = async () => {
    try {
      await invoke("open_memory_folder")
    } catch (error: any) {
      toast.error(`Failed to open folder: ${error.message || error}`)
    }
  }

  const renderStepIcon = (stepStatus: SetupStepStatus) => {
    switch (stepStatus) {
      case "ok":
        return <CheckCircle2 className="h-4 w-4 text-green-500" />
      case "error":
        return <XCircle className="h-4 w-4 text-destructive" />
      case "checking":
      case "installing":
        return <Loader2 className="h-4 w-4 animate-spin text-blue-500" />
      default:
        return <Circle className="h-4 w-4 text-muted-foreground" />
    }
  }

  if (isLoading) {
    return (
      <div className="flex items-center justify-center p-8">
        <Loader2 className="h-6 w-6 animate-spin" />
      </div>
    )
  }

  return (
    <div className="space-y-4">
      {/* Privacy Warning */}
      <Card className="border-amber-500/30 bg-amber-50/5">
        <CardContent className="pt-4 pb-3">
          <div className="flex items-start gap-2">
            <AlertTriangle className="h-4 w-4 text-amber-500 mt-0.5 shrink-0" />
            <div className="space-y-1">
              <p className="text-sm text-amber-700 dark:text-amber-400">
                When enabled for a client, full conversations are recorded and stored locally.
              </p>
              <button
                onClick={openMemoryFolder}
                className="text-xs text-amber-600 dark:text-amber-500 hover:underline flex items-center gap-1"
              >
                <FolderOpen className="h-3 w-3" />
                Review stored memories
              </button>
            </div>
          </div>
        </CardContent>
      </Card>

      {/* Setup */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">Setup</CardTitle>
          <CardDescription>
            Memory requires Python and the memsearch CLI
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="space-y-2">
            <div className="flex items-center gap-2 text-sm">
              {renderStepIcon(setup.python.status)}
              <span>Python environment</span>
              {setup.python.version && (
                <span className="text-xs text-muted-foreground">{setup.python.version}</span>
              )}
              {setup.python.error && (
                <span className="text-xs text-destructive">{setup.python.error}</span>
              )}
            </div>
            <div className="flex items-center gap-2 text-sm">
              {renderStepIcon(setup.memsearch.status)}
              <span>memsearch CLI</span>
              {setup.memsearch.version && (
                <span className="text-xs text-muted-foreground">{setup.memsearch.version}</span>
              )}
              {setup.memsearch.error && (
                <span className="text-xs text-destructive truncate max-w-[300px]">{setup.memsearch.error}</span>
              )}
            </div>
            <div className="flex items-center gap-2 text-sm">
              {renderStepIcon(setup.model.status)}
              <span>Embedding model</span>
              {setup.model.status === "installing" && (
                <span className="text-xs text-muted-foreground">Downloading...</span>
              )}
              {setup.model.error && (
                <span className="text-xs text-destructive">{setup.model.error}</span>
              )}
            </div>
          </div>
          <Button
            size="sm"
            onClick={runSetup}
            disabled={isSettingUp}
          >
            {isSettingUp ? (
              <>
                <Loader2 className="h-3.5 w-3.5 mr-1 animate-spin" />
                Setting up...
              </>
            ) : (
              "Setup"
            )}
          </Button>
        </CardContent>
      </Card>

      {/* Configuration */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">Configuration</CardTitle>
          <CardDescription>
            Global settings for memory. Enable memory per-client in client settings.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-1.5">
              <Label htmlFor="recall-tool-name" className="text-xs">Tool name</Label>
              <Input
                id="recall-tool-name"
                value={config.recall_tool_name}
                onChange={(e) => {
                  const newConfig = { ...config, recall_tool_name: e.target.value }
                  setConfig(newConfig)
                }}
                onBlur={() => saveConfig(config)}
                className="h-8 text-sm"
                placeholder="MemoryRecall"
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="search-top-k" className="text-xs">Search results</Label>
              <Input
                id="search-top-k"
                type="number"
                min={1}
                max={20}
                value={config.search_top_k}
                onChange={(e) => {
                  const val = parseInt(e.target.value) || 5
                  const newConfig = { ...config, search_top_k: val }
                  setConfig(newConfig)
                }}
                onBlur={() => saveConfig(config)}
                className="h-8 text-sm"
              />
            </div>
          </div>

          <div className="space-y-1.5">
            <Label className="text-xs">Embedding provider</Label>
            <div className="flex gap-4">
              <label className="flex items-center gap-1.5 text-sm cursor-pointer">
                <input
                  type="radio"
                  name="embedding"
                  checked={config.embedding.type === "onnx"}
                  onChange={() => saveConfig({ ...config, embedding: { type: "onnx" } })}
                  className="accent-primary"
                />
                Built-in ONNX
              </label>
              <label className="flex items-center gap-1.5 text-sm cursor-pointer">
                <input
                  type="radio"
                  name="embedding"
                  checked={config.embedding.type === "ollama"}
                  onChange={() =>
                    saveConfig({
                      ...config,
                      embedding: { type: "ollama", provider_id: "", model_name: "nomic-embed-text" },
                    })
                  }
                  className="accent-primary"
                />
                Ollama
              </label>
            </div>
            {config.embedding.type === "ollama" && (
              <div className="grid grid-cols-2 gap-2 mt-2">
                <Input
                  placeholder="Provider ID"
                  value={(config.embedding as any).provider_id || ""}
                  onChange={(e) =>
                    setConfig({
                      ...config,
                      embedding: {
                        type: "ollama",
                        provider_id: e.target.value,
                        model_name: (config.embedding as any).model_name || "nomic-embed-text",
                      },
                    })
                  }
                  onBlur={() => saveConfig(config)}
                  className="h-8 text-sm"
                />
                <Input
                  placeholder="Model name"
                  value={(config.embedding as any).model_name || ""}
                  onChange={(e) =>
                    setConfig({
                      ...config,
                      embedding: {
                        type: "ollama",
                        provider_id: (config.embedding as any).provider_id || "",
                        model_name: e.target.value,
                      },
                    })
                  }
                  onBlur={() => saveConfig(config)}
                  className="h-8 text-sm"
                />
              </div>
            )}
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-1.5">
              <Label htmlFor="inactivity" className="text-xs">Session inactivity timeout (min)</Label>
              <Input
                id="inactivity"
                type="number"
                min={10}
                value={config.session_inactivity_minutes}
                onChange={(e) => {
                  const val = parseInt(e.target.value) || 180
                  setConfig({ ...config, session_inactivity_minutes: val })
                }}
                onBlur={() => saveConfig(config)}
                className="h-8 text-sm"
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="max-session" className="text-xs">Max session duration (min)</Label>
              <Input
                id="max-session"
                type="number"
                min={30}
                value={config.max_session_minutes}
                onChange={(e) => {
                  const val = parseInt(e.target.value) || 480
                  setConfig({ ...config, max_session_minutes: val })
                }}
                onBlur={() => saveConfig(config)}
                className="h-8 text-sm"
              />
            </div>
          </div>
        </CardContent>
      </Card>

      {/* Compaction */}
      <Card>
        <CardHeader className="pb-3">
          <div className="flex items-center justify-between">
            <div>
              <CardTitle className="text-sm">Compaction</CardTitle>
              <CardDescription>
                LLM-based summarization at session end
              </CardDescription>
            </div>
            <Switch
              checked={config.compaction?.enabled ?? false}
              onCheckedChange={(checked) => {
                const newConfig = {
                  ...config,
                  compaction: checked
                    ? { enabled: true, llm_provider: "anthropic", llm_model: null }
                    : null,
                }
                saveConfig(newConfig)
              }}
            />
          </div>
        </CardHeader>
        {config.compaction?.enabled && (
          <CardContent className="space-y-3">
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-1.5">
                <Label htmlFor="llm-provider" className="text-xs">LLM provider</Label>
                <Input
                  id="llm-provider"
                  value={config.compaction.llm_provider}
                  onChange={(e) =>
                    setConfig({
                      ...config,
                      compaction: { ...config.compaction!, llm_provider: e.target.value },
                    })
                  }
                  onBlur={() => saveConfig(config)}
                  className="h-8 text-sm"
                  placeholder="anthropic"
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="llm-model" className="text-xs">Model (optional)</Label>
                <Input
                  id="llm-model"
                  value={config.compaction.llm_model || ""}
                  onChange={(e) =>
                    setConfig({
                      ...config,
                      compaction: {
                        ...config.compaction!,
                        llm_model: e.target.value || null,
                      },
                    })
                  }
                  onBlur={() => saveConfig(config)}
                  className="h-8 text-sm"
                  placeholder="claude-haiku-4-5-20251001"
                />
              </div>
            </div>
          </CardContent>
        )}
      </Card>

      {/* Info */}
      <p className="text-xs text-muted-foreground">
        Memory is enabled per-client in client settings. No global toggle — each client must opt in.
      </p>
    </div>
  )
}
