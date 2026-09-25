import { useCallback, useEffect, useRef, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { open } from "@tauri-apps/plugin-shell"
import { toast } from "sonner"
import {
  CheckCircle,
  XCircle,
  AlertCircle,
  Loader2,
  RefreshCw,
  Copy,
  Play,
  Square,
  Terminal,
  ExternalLink,
  ScrollText,
} from "lucide-react"
import { listenSafe } from "@/hooks/useTauriListener"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card"
import type {
  EngineInstallFinishedEvent,
  EngineInstallOutputEvent,
  EngineProcessInfo,
  EngineStatus,
  EngineInstallOptionView,
} from "@/types/tauri-commands"

/** Engine recipe for each Local Embedded provider type. */
export const EMBEDDED_PROVIDER_RECIPES: Record<string, string> = {
  llamacpp_embedded: "llamacpp",
  laya: "laya",
  kev: "kev",
  von: "von",
  decider: "decider",
}

/** Engines run through `uv tool run`: installing uv is enough, and their
 *  install option only pre-downloads packages. */
const RUNS_THROUGH_UV = new Set(["kev", "decider"])

export function isEmbeddedProviderType(providerType: string): boolean {
  return providerType in EMBEDDED_PROVIDER_RECIPES
}

interface EmbeddedEngineTabProps {
  providerType: string
  instanceName: string
  /** Optional executable path from the provider's settings. */
  binaryPath?: string | null
}

const MAX_OUTPUT_LINES = 400

/** Install commands for one recipe, with Copy / Install and live output. */
function InstallSection({
  status,
  onFinished,
}: {
  status: EngineStatus
  onFinished: () => void
}) {
  const [runId, setRunId] = useState<string | null>(null)
  const [output, setOutput] = useState<string[]>([])
  const [result, setResult] = useState<string | null>(null)
  const runIdRef = useRef<string | null>(null)
  const outputRef = useRef<HTMLPreElement>(null)

  useEffect(() => {
    const out = listenSafe<EngineInstallOutputEvent>("engine-install-output", (e) => {
      if (e.payload.run_id !== runIdRef.current) return
      setOutput((prev) => {
        const next = [...prev, e.payload.line]
        return next.length > MAX_OUTPUT_LINES ? next.slice(-MAX_OUTPUT_LINES) : next
      })
    })
    const done = listenSafe<EngineInstallFinishedEvent>("engine-install-finished", (e) => {
      if (e.payload.run_id !== runIdRef.current) return
      runIdRef.current = null
      setRunId(null)
      const p = e.payload
      if (p.cancelled) setResult("Cancelled.")
      else if (p.error) setResult(`Failed: ${p.error}`)
      else if (p.exit_code === 0) setResult("Finished successfully.")
      else setResult(`Exited with code ${p.exit_code ?? "unknown"}.`)
      onFinished()
    })
    return () => {
      out.cleanup()
      done.cleanup()
    }
  }, [onFinished])

  useEffect(() => {
    outputRef.current?.scrollTo({ top: outputRef.current.scrollHeight })
  }, [output])

  const runOption = async (option: EngineInstallOptionView) => {
    setOutput([`$ ${option.command}`])
    setResult(null)
    try {
      const id = await invoke<string>("engine_install", {
        recipeId: status.recipe,
        optionId: option.id,
      })
      runIdRef.current = id
      setRunId(id)
    } catch (err) {
      setResult(`Failed: ${err}`)
    }
  }

  const cancel = async () => {
    if (runId) await invoke("engine_install_cancel", { runId })
  }

  const copy = async (command: string) => {
    await navigator.clipboard.writeText(command)
    toast.success("Command copied")
  }

  const [primary, ...others] = [
    ...status.install.filter((o) => o.recommended),
    ...status.install.filter((o) => !o.recommended),
  ]
  const [showOthers, setShowOthers] = useState(false)

  const renderOption = (option: EngineInstallOptionView) => (
    <div key={option.id} className="space-y-2 rounded-md border p-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-sm font-medium">{option.label}</span>
        {option.recommended && <Badge variant="secondary">Recommended</Badge>}
        {!option.program_found && (
          <Badge variant="outline" className="text-muted-foreground">
            {option.program} not found
          </Badge>
        )}
        {option.needs_sudo && (
          <Badge variant="outline" className="text-muted-foreground">
            Needs sudo: run in a terminal
          </Badge>
        )}
      </div>
      <div className="flex items-start gap-2">
        <pre className="min-w-0 flex-1 whitespace-pre-wrap break-all rounded bg-muted px-3 py-2 font-mono text-xs">
          {option.command}
        </pre>
        <Button
          variant="outline"
          size="sm"
          onClick={() => copy(option.command)}
          aria-label={`Copy ${option.label} command`}
        >
          <Copy className="h-4 w-4" />
        </Button>
        {option.runnable && (
          <Button
            size="sm"
            onClick={() => runOption(option)}
            disabled={!!runId}
            title={option.program_found ? undefined : `${option.program} was not found on PATH`}
          >
            <Play className="mr-1 h-4 w-4" />
            Install
          </Button>
        )}
      </div>
      {option.notes && <p className="text-xs text-muted-foreground">{option.notes}</p>}
    </div>
  )

  return (
    <div className="space-y-3">
      {primary && renderOption(primary)}
      {others.length > 0 && (
        <div className="space-y-2">
          <button
            type="button"
            className="text-xs text-muted-foreground underline-offset-2 hover:underline"
            onClick={() => setShowOthers((v) => !v)}
          >
            {showOthers ? "Hide other ways" : `Other ways (${others.length})`}
          </button>
          {showOthers && others.map(renderOption)}
        </div>
      )}
      {(output.length > 0 || runId) && (
        <div className="space-y-2">
          <div className="flex items-center gap-2 text-sm">
            <Terminal className="h-4 w-4" />
            {runId ? (
              <>
                <Loader2 className="h-4 w-4 animate-spin" /> Running…
                <Button variant="outline" size="sm" onClick={cancel}>
                  Cancel
                </Button>
              </>
            ) : (
              <span className="text-muted-foreground">{result}</span>
            )}
          </div>
          <pre
            ref={outputRef}
            className="max-h-64 overflow-y-auto overflow-x-hidden whitespace-pre-wrap break-all rounded bg-muted px-3 py-2 font-mono text-xs"
            aria-live="polite"
          >
            {output.join("\n")}
          </pre>
        </div>
      )}
    </div>
  )
}

export function EmbeddedEngineTab({ providerType, instanceName, binaryPath }: EmbeddedEngineTabProps) {
  const recipeId = EMBEDDED_PROVIDER_RECIPES[providerType]
  const [status, setStatus] = useState<EngineStatus | null>(null)
  const [requirements, setRequirements] = useState<EngineStatus[]>([])
  const [loading, setLoading] = useState(false)
  const [processes, setProcesses] = useState<EngineProcessInfo[]>([])
  const [logsFor, setLogsFor] = useState<string | null>(null)
  const [logs, setLogs] = useState<string[]>([])

  const keyPrefix = `${providerType}:${instanceName}`

  const refresh = useCallback(
    async (rescanPath: boolean) => {
      if (!recipeId) return
      setLoading(true)
      try {
        const s = await invoke<EngineStatus>("engine_status", {
          recipeId,
          binaryPath: binaryPath || null,
          refresh: rescanPath,
        })
        setStatus(s)
        const reqs = await Promise.all(
          s.requirements
            .filter((r) => !r.found)
            .map((r) => invoke<EngineStatus>("engine_status", { recipeId: r.recipe, refresh: false })),
        )
        setRequirements(reqs)
      } catch (err) {
        toast.error(`Could not check the engine: ${err}`)
      } finally {
        setLoading(false)
      }
    },
    [recipeId, binaryPath],
  )

  const loadProcesses = useCallback(async () => {
    try {
      const all = await invoke<EngineProcessInfo[]>("engine_processes")
      setProcesses(all.filter((p) => p.key === keyPrefix || p.key.startsWith(`${keyPrefix}:`)))
    } catch {
      setProcesses([])
    }
  }, [keyPrefix])

  useEffect(() => {
    refresh(false)
    loadProcesses()
    const timer = setInterval(loadProcesses, 3000)
    return () => clearInterval(timer)
  }, [refresh, loadProcesses])

  useEffect(() => {
    if (!logsFor) return
    let cancelled = false
    const load = async () => {
      const lines = await invoke<string[]>("engine_logs", { key: logsFor }).catch(() => [])
      if (!cancelled) setLogs(lines)
    }
    load()
    const timer = setInterval(load, 2000)
    return () => {
      cancelled = true
      clearInterval(timer)
    }
  }, [logsFor])

  const onInstallFinished = useCallback(() => {
    refresh(true)
  }, [refresh])

  if (!recipeId) return null

  const stop = async (key: string) => {
    await invoke("engine_stop", { key })
    loadProcesses()
  }

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <div className="flex items-start justify-between gap-4">
            <div>
              <CardTitle className="text-base">Engine</CardTitle>
              <CardDescription>
                LocalRouter runs this engine on your machine. Install it once with your package
                manager; LocalRouter finds it on your PATH.
              </CardDescription>
            </div>
            <Button variant="outline" size="sm" onClick={() => refresh(true)} disabled={loading}>
              {loading ? (
                <Loader2 className="mr-1 h-4 w-4 animate-spin" />
              ) : (
                <RefreshCw className="mr-1 h-4 w-4" />
              )}
              Refresh
            </Button>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          {status && !status.supported && (
            <div className="flex items-start gap-2 text-sm text-amber-600 dark:text-amber-400">
              <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
              <span>{status.unsupported_reason}</span>
            </div>
          )}
          {status?.supported && (
            <div className="flex items-start gap-2 text-sm">
              {status.found ? (
                <CheckCircle className="mt-0.5 h-4 w-4 shrink-0 text-green-600" />
              ) : (
                <XCircle className="mt-0.5 h-4 w-4 shrink-0 text-red-500" />
              )}
              <div>
                {status.found ? (
                  <>
                    Found <span className="font-mono">{status.binary}</span>
                    {status.version && <> {status.version}</>}
                    {status.build != null && status.version !== String(status.build) && (
                      <> (build {status.build})</>
                    )}{" "}
                    at <span className="break-all font-mono text-xs">{status.path}</span>
                  </>
                ) : (
                  <>{status.display_name} is not installed (not found on PATH).</>
                )}
              </div>
            </div>
          )}
          {status?.supported &&
            requirements.map((req) => (
              <div key={req.recipe} className="space-y-2 border-t pt-4">
                <p className="text-sm font-medium">Step 1: install {req.display_name}</p>
                <p className="text-xs text-muted-foreground">
                  {status.display_name} needs {req.display_name}, which was not found on your
                  PATH.
                </p>
                <InstallSection status={req} onFinished={onInstallFinished} />
              </div>
            ))}
          {status?.supported && (!status.found || RUNS_THROUGH_UV.has(status.recipe)) && (
            <div className="space-y-2 border-t pt-4">
              <p className="text-sm font-medium">
                {requirements.length > 0 ? "Step 2: " : ""}
                {RUNS_THROUGH_UV.has(status.recipe)
                  ? `Prepare ${status.display_name} (optional)`
                  : `Install ${status.display_name}`}
              </p>
              <p className="text-xs text-muted-foreground">
                Run the command in a terminal, or click Install to run it here. Click Refresh
                afterwards.
              </p>
              <InstallSection status={status} onFinished={onInstallFinished} />
            </div>
          )}
          {status && (
            <button
              type="button"
              className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:underline"
              onClick={() => open(status.docs_url)}
            >
              <ExternalLink className="h-3 w-3" /> Installation docs
            </button>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">Running processes</CardTitle>
          <CardDescription>
            Engines start on the first request for a downloaded model (or when you click Load in
            the Models tab) and stop after being idle.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          {processes.length === 0 ? (
            <p className="text-sm text-muted-foreground">No engine is running.</p>
          ) : (
            processes.map((p) => (
              <div key={p.key} className="min-w-0 space-y-1 rounded-md border p-3 text-sm">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-medium">{p.label}</span>
                  <Badge
                    variant={p.state === "running" ? "secondary" : "outline"}
                    className={p.state === "failed" ? "text-red-500" : undefined}
                  >
                    {p.state}
                  </Badge>
                  {p.port != null && (
                    <span className="text-xs text-muted-foreground">port {p.port}</span>
                  )}
                  {p.uptime_secs != null && (
                    <span className="text-xs text-muted-foreground">
                      up {Math.round(p.uptime_secs / 60)} min
                    </span>
                  )}
                  {p.restarts > 0 && (
                    <span className="text-xs text-muted-foreground">{p.restarts} restarts</span>
                  )}
                  <div className="ml-auto flex gap-2">
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => setLogsFor(logsFor === p.key ? null : p.key)}
                    >
                      <ScrollText className="mr-1 h-4 w-4" />
                      Logs
                    </Button>
                    {p.state === "running" && (
                      <Button variant="outline" size="sm" onClick={() => stop(p.key)}>
                        <Square className="mr-1 h-4 w-4" />
                        Stop
                      </Button>
                    )}
                  </div>
                </div>
                {p.last_error && p.state !== "running" && (
                  <p className="whitespace-pre-wrap break-words text-xs text-red-500">
                    {p.last_error}
                  </p>
                )}
                {logsFor === p.key && (
                  <pre className="max-h-64 overflow-y-auto overflow-x-hidden whitespace-pre-wrap break-all rounded bg-muted px-3 py-2 font-mono text-xs">
                    {logs.join("\n") || "No output yet."}
                  </pre>
                )}
              </div>
            ))
          )}
        </CardContent>
      </Card>
    </div>
  )
}
