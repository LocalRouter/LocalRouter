import { useCallback, useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { AlertCircle, CheckCircle, Download, Loader2, Play, ScrollText, Square, Trash2, X } from "lucide-react"
import { Progress } from "@/components/ui/progress"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card"
import { HuggingFaceAccountCard } from "@/components/providers/HuggingFaceAccountCard"
import type {
  EmbeddedCatalogModel,
  EmbeddedModelState,
  LocalModelsEngineCatalogParams,
  LocalModelsEngineDownloadParams,
  LocalModelsLoadParams,
  LocalModelsStatesParams,
} from "@/types/tauri-commands"

interface EngineModelsTabProps {
  providerType: string
  instanceName: string
  enabled: boolean
}

/**
 * Models of an engine that downloads its own checkpoints (Laya, Kev, Von,
 * Decider). Downloads happen only here; requests for a model that is not
 * downloaded fail instead of downloading.
 */
export function EngineModelsTab({ providerType, instanceName, enabled }: EngineModelsTabProps) {
  const [catalog, setCatalog] = useState<EmbeddedCatalogModel[] | null>(null)
  const [states, setStates] = useState<EmbeddedModelState[]>([])
  const [busy, setBusy] = useState<string | null>(null)
  const [logsFor, setLogsFor] = useState<string | null>(null)
  const [removing, setRemoving] = useState<EmbeddedCatalogModel | null>(null)
  const [logs, setLogs] = useState<string[]>([])

  const refresh = useCallback(async () => {
    try {
      const [c, s] = await Promise.all([
        invoke<EmbeddedCatalogModel[]>("local_models_engine_catalog", {
          instanceName,
        } satisfies LocalModelsEngineCatalogParams),
        invoke<EmbeddedModelState[]>("local_models_states", {
          instanceName,
        } satisfies LocalModelsStatesParams),
      ])
      setCatalog(c)
      setStates(s)
    } catch (err) {
      toast.error(`Could not load models: ${err}`)
    }
  }, [instanceName])

  useEffect(() => {
    refresh()
    const timer = setInterval(refresh, 2000)
    return () => clearInterval(timer)
  }, [refresh])

  // The download runs the engine; its output is the progress.
  const downloadKey = (model: string) => `${providerType}:${instanceName}:download:${model}`

  useEffect(() => {
    if (!logsFor) return
    let cancelled = false
    const load = async () => {
      const lines = await invoke<string[]>("engine_logs", { key: downloadKey(logsFor) }).catch(
        () => [],
      )
      if (!cancelled) setLogs(lines.slice(-40))
    }
    load()
    const timer = setInterval(load, 2000)
    return () => {
      cancelled = true
      clearInterval(timer)
    }
    // downloadKey only depends on props already listed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [logsFor, providerType, instanceName])

  const run = async (model: string, action: () => Promise<unknown>, done?: string) => {
    setBusy(model)
    try {
      await action()
      if (done) toast.success(done)
    } catch (err) {
      toast.error(String(err))
    } finally {
      setBusy(null)
      refresh()
    }
  }

  const download = (model: string) =>
    run(model, () =>
      invoke("local_models_engine_download", {
        instanceName,
        model,
      } satisfies LocalModelsEngineDownloadParams),
    ).then(() => setLogsFor(model))

  const cancel = (model: string) =>
    run(model, () =>
      invoke("local_models_engine_download_cancel", {
        instanceName,
        model,
      } satisfies LocalModelsEngineDownloadParams),
    )

  const load = (model: string) =>
    run(
      model,
      () => invoke("local_models_load", { instanceName, model } satisfies LocalModelsLoadParams),
      `${model} is loaded`,
    )

  const unload = (model: string) =>
    run(model, () =>
      invoke("local_models_unload", { instanceName, model } satisfies LocalModelsLoadParams),
    )

  const remove = (model: string) =>
    run(
      model,
      () =>
        invoke("local_models_engine_remove", {
          instanceName,
          model,
        } satisfies LocalModelsEngineDownloadParams),
      `Removed ${model}`,
    )

  const stateOf = (model: string) => states.find((s) => s.model === model)

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Models</CardTitle>
          <CardDescription>
            Download a model before using it. The engine fetches it from Hugging Face; requests
            for a model that is not downloaded fail instead of downloading it.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          {catalog === null ? (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" /> Loading…
            </div>
          ) : (
            catalog.map((m) => {
              const state = stateOf(m.id)
              const loaded = state?.state === "running"
              return (
                <div key={m.id} className="min-w-0 space-y-2 rounded-md border p-3 text-sm">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-medium">{m.name}</span>
                    <span className="font-mono text-xs text-muted-foreground">{m.id}</span>
                    <span className="text-xs text-muted-foreground">{m.download_size}</span>
                    {m.downloading ? (
                      <Badge variant="secondary">
                        <Loader2 className="mr-1 h-3 w-3 animate-spin" />
                        Downloading
                        {m.progress != null && ` ${Math.floor(m.progress * 100)}%`}
                      </Badge>
                    ) : m.downloaded ? (
                      <Badge variant="secondary" className="text-green-700 dark:text-green-400">
                        <CheckCircle className="mr-1 h-3 w-3" />
                        Downloaded
                      </Badge>
                    ) : (
                      <Badge variant="outline">Not downloaded</Badge>
                    )}
                    {loaded && <Badge variant="secondary">Loaded</Badge>}
                    <div className="ml-auto flex flex-wrap gap-2">
                      {m.downloaded && m.removable && !loaded && (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => setRemoving(m)}
                          disabled={busy === m.id}
                        >
                          <Trash2 className="mr-1 h-4 w-4" />
                          Remove
                        </Button>
                      )}
                      {m.downloading ? (
                        <>
                          {m.progress == null && (
                            <Button
                              variant="outline"
                              size="sm"
                              onClick={() => setLogsFor(logsFor === m.id ? null : m.id)}
                            >
                              <ScrollText className="mr-1 h-4 w-4" />
                              Progress
                            </Button>
                          )}
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() => cancel(m.id)}
                            disabled={busy === m.id}
                          >
                            <X className="mr-1 h-4 w-4" />
                            Cancel
                          </Button>
                        </>
                      ) : m.downloaded ? (
                        loaded ? (
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() => unload(m.id)}
                            disabled={busy === m.id}
                          >
                            <Square className="mr-1 h-4 w-4" />
                            Unload
                          </Button>
                        ) : (
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() => load(m.id)}
                            disabled={busy === m.id || !enabled}
                            title={enabled ? undefined : "Enable the provider to load models"}
                          >
                            {busy === m.id ? (
                              <Loader2 className="mr-1 h-4 w-4 animate-spin" />
                            ) : (
                              <Play className="mr-1 h-4 w-4" />
                            )}
                            Load
                          </Button>
                        )
                      ) : (
                        <Button
                          size="sm"
                          onClick={() => download(m.id)}
                          disabled={busy === m.id || !enabled}
                          title={enabled ? undefined : "Enable the provider to download models"}
                        >
                          <Download className="mr-1 h-4 w-4" />
                          Download
                        </Button>
                      )}
                    </div>
                  </div>
                  {m.guidance && <p className="text-xs text-muted-foreground">{m.guidance}</p>}
                  {m.downloading && m.progress != null && (
                    <Progress
                      value={m.progress * 100}
                      className="h-1.5"
                      aria-label={`Download progress of ${m.name}`}
                    />
                  )}
                  {m.download_error && !m.downloading && (
                    <p className="flex items-start gap-1 whitespace-pre-wrap break-words text-xs text-red-500">
                      <AlertCircle className="mt-0.5 h-3 w-3 shrink-0" />
                      Download failed: {m.download_error}
                    </p>
                  )}
                  {state?.last_error && state.state !== "running" && (
                    <p className="whitespace-pre-wrap break-words text-xs text-red-500">
                      {state.last_error}
                    </p>
                  )}
                  {logsFor === m.id && m.downloading && (
                    <pre className="max-h-64 overflow-y-auto overflow-x-hidden whitespace-pre-wrap break-all rounded bg-muted px-3 py-2 font-mono text-xs">
                      {logs.join("\n") || "Starting the engine…"}
                    </pre>
                  )}
                </div>
              )
            })
          )}
        </CardContent>
      </Card>

      <HuggingFaceAccountCard description="Optional. Downloads use this account (for gated or private checkpoints and higher rate limits). Shared by all Local Embedded providers." />

      <AlertDialog open={removing !== null} onOpenChange={(o) => !o && setRemoving(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove {removing?.name}?</AlertDialogTitle>
            <AlertDialogDescription>
              Its downloaded files are deleted, except files another downloaded model still uses
              and your own library files. You can download it again later.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (removing) remove(removing.id)
                setRemoving(null)
              }}
            >
              Remove
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
