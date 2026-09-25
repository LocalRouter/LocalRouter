import { useCallback, useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { open } from "@tauri-apps/plugin-shell"
import { open as openFileDialog } from "@tauri-apps/plugin-dialog"
import { toast } from "sonner"
import {
  AlertCircle,
  Download,
  ExternalLink,
  FileUp,
  Heart,
  Loader2,
  Lock,
  Pause,
  Pencil,
  Play,
  Search,
  Square,
  Trash2,
  X,
} from "lucide-react"
import { listenSafe } from "@/hooks/useTauriListener"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/Input"
import { Progress } from "@/components/ui/progress"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/Select"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
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
import { HuggingFaceAccountCard } from "@/components/providers/HuggingFaceAccountCard"
import type {
  DownloadFinishedEvent,
  DownloadJobView,
  EmbeddedModelState,
  FitVerdict,
  GgufVariant,
  HardwareInfo,
  HubModelSummary,
  HubPage,
  KvCacheType,
  LibraryChangedEvent,
  LibraryEntry,
  LocalLibraryView,
  LocalModelKind,
  LocalModelsDownloadIdParams,
  LocalModelsDownloadStartParams,
  LocalModelsImportParams,
  LocalModelsInspectRemoteParams,
  LocalModelsLoadParams,
  LocalModelsRemoveParams,
  LocalModelsRenameParams,
  LocalModelsRepoParams,
  LocalModelsSearchParams,
  LocalModelsStatesParams,
  LocalRepoDetails,
  RemoteModelInspection,
} from "@/types/tauri-commands"

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null) return "unknown size"
  if (bytes < 1024) return `${bytes} B`
  const units = ["KB", "MB", "GB", "TB"]
  let value = bytes / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit++
  }
  return `${value.toFixed(value >= 100 || unit === 0 ? 0 : 1)} ${units[unit]}`
}

function formatCount(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return String(n)
}

function formatContext(tokens: number | null): string | null {
  if (!tokens) return null
  return tokens >= 1024 ? `${Math.round(tokens / 1024)}K ctx` : `${tokens} ctx`
}

function formatEta(job: DownloadJobView): string | null {
  if (job.state !== "running" || job.speed_bps <= 0 || job.bytes_total <= job.bytes_done) return null
  const secs = Math.round((job.bytes_total - job.bytes_done) / job.speed_bps)
  if (secs < 60) return `${secs}s left`
  if (secs < 3600) return `${Math.round(secs / 60)} min left`
  return `${(secs / 3600).toFixed(1)} h left`
}

const KIND_LABELS: Record<LocalModelKind, string> = {
  chat: "Chat",
  completion: "Completion",
  embedding: "Embedding",
  reranker: "Reranker",
  projector: "Vision projector",
  adapter: "Adapter",
}

/** Kinds llama.cpp serves as models. */
const SERVABLE_KINDS = new Set<LocalModelKind>(["chat", "completion", "embedding"])

const VERDICTS: Record<FitVerdict, { label: string; variant: "success" | "warning" | "destructive" | "outline" }> = {
  fits: { label: "Fits", variant: "success" },
  tight: { label: "Tight", variant: "warning" },
  too_large: { label: "Won't fit", variant: "destructive" },
  unknown: { label: "Unknown", variant: "outline" },
}

const SORTS: { value: NonNullable<LocalModelsSearchParams["sort"]>; label: string }[] = [
  { value: "trendingScore", label: "Trending" },
  { value: "downloads", label: "Most downloads" },
  { value: "likes", label: "Most likes" },
  { value: "lastModified", label: "Recently updated" },
]

// ---------------------------------------------------------------------------
// Library
// ---------------------------------------------------------------------------

function LibrarySection({
  instanceName,
  library,
  onChanged,
}: {
  instanceName: string
  library: LocalLibraryView | null
  onChanged: () => void
}) {
  const [states, setStates] = useState<Record<string, EmbeddedModelState>>({})
  const [loading, setLoading] = useState<Set<string>>(new Set())
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null)
  const [removing, setRemoving] = useState<LibraryEntry | null>(null)
  const [deleteFiles, setDeleteFiles] = useState(true)
  const [importing, setImporting] = useState(false)

  const loadStates = useCallback(async () => {
    try {
      const list = await invoke<EmbeddedModelState[]>("local_models_states", {
        instanceName,
      } satisfies LocalModelsStatesParams)
      setStates(Object.fromEntries(list.map((s) => [s.model, s])))
    } catch {
      setStates({})
    }
  }, [instanceName])

  useEffect(() => {
    loadStates()
    const timer = setInterval(loadStates, 3000)
    return () => clearInterval(timer)
  }, [loadStates])

  const load = async (entry: LibraryEntry) => {
    setLoading((prev) => new Set(prev).add(entry.id))
    try {
      await invoke("local_models_load", {
        instanceName,
        model: entry.id,
      } satisfies LocalModelsLoadParams)
      toast.success(`${entry.display_name} is loaded`)
    } catch (err) {
      toast.error(`Could not load ${entry.display_name}: ${err}`)
    } finally {
      setLoading((prev) => {
        const next = new Set(prev)
        next.delete(entry.id)
        return next
      })
      loadStates()
    }
  }

  const unload = async (entry: LibraryEntry) => {
    try {
      await invoke("local_models_unload", {
        instanceName,
        model: entry.id,
      } satisfies LocalModelsLoadParams)
    } catch (err) {
      toast.error(`Could not unload ${entry.display_name}: ${err}`)
    }
    loadStates()
  }

  const saveRename = async () => {
    if (!renaming) return
    try {
      await invoke("local_models_rename", {
        id: renaming.id,
        displayName: renaming.name,
      } satisfies LocalModelsRenameParams)
      setRenaming(null)
      onChanged()
    } catch (err) {
      toast.error(`${err}`)
    }
  }

  const confirmRemove = async () => {
    if (!removing) return
    const entry = removing
    setRemoving(null)
    try {
      await invoke("local_models_remove", {
        id: entry.id,
        deleteFiles: entry.source.type === "hugging_face" && deleteFiles,
      } satisfies LocalModelsRemoveParams)
      toast.success(`Removed ${entry.display_name}`)
    } catch (err) {
      toast.error(`Could not remove ${entry.display_name}: ${err}`)
    }
    onChanged()
  }

  const importFile = async () => {
    let selected: string | string[] | null
    try {
      selected = await openFileDialog({
        multiple: false,
        directory: false,
        title: "Import a GGUF model",
        filters: [{ name: "GGUF model", extensions: ["gguf"] }],
      })
    } catch (err) {
      toast.error(`Could not open the file picker: ${err}`)
      return
    }
    if (!selected || typeof selected !== "string") return
    setImporting(true)
    try {
      const entry = await invoke<LibraryEntry>("local_models_import", {
        path: selected,
      } satisfies LocalModelsImportParams)
      toast.success(`Imported ${entry.display_name}`)
      onChanged()
    } catch (err) {
      toast.error(`Could not import: ${err}`)
    } finally {
      setImporting(false)
    }
  }

  const stateBadge = (entry: LibraryEntry) => {
    if (loading.has(entry.id)) {
      return (
        <Badge variant="info">
          <Loader2 className="mr-1 h-3 w-3 animate-spin" />
          Loading
        </Badge>
      )
    }
    const s = states[entry.id]
    if (s?.state === "running") return <Badge variant="success">Loaded</Badge>
    if (s?.state === "failed") return <Badge variant="destructive">Failed</Badge>
    return <Badge variant="outline">Unloaded</Badge>
  }

  const entries = library?.entries ?? []

  return (
    <Card>
      <CardHeader>
        <div className="flex items-start justify-between gap-4">
          <div>
            <CardTitle className="text-base">Library</CardTitle>
            <CardDescription>
              Models on this machine. They load on the first request and unload when idle; Load
              starts one now.
              {library && (
                <>
                  {" "}
                  {formatBytes(library.disk_usage_bytes)} used in{" "}
                  <span className="break-all font-mono text-xs">{library.storage_dir}</span>.
                </>
              )}
            </CardDescription>
          </div>
          <Button variant="outline" size="sm" onClick={importFile} disabled={importing}>
            {importing ? (
              <Loader2 className="mr-1 h-4 w-4 animate-spin" />
            ) : (
              <FileUp className="mr-1 h-4 w-4" />
            )}
            Import GGUF file
          </Button>
        </div>
      </CardHeader>
      <CardContent className="space-y-2">
        {library === null ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" /> Loading…
          </div>
        ) : entries.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No models yet. Download one from Hugging Face below, or import a GGUF file.
          </p>
        ) : (
          entries.map((entry) => {
            const servable = SERVABLE_KINDS.has(entry.kind)
            const running = states[entry.id]?.state === "running"
            const s = states[entry.id]
            return (
              <div key={entry.id} className="space-y-1 rounded-md border p-3 text-sm">
                <div className="flex flex-wrap items-center gap-2">
                  {renaming?.id === entry.id ? (
                    <div className="flex items-center gap-2">
                      <Input
                        className="h-8 w-64"
                        value={renaming.name}
                        autoFocus
                        onChange={(e) => setRenaming({ id: entry.id, name: e.target.value })}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") saveRename()
                          if (e.key === "Escape") setRenaming(null)
                        }}
                        aria-label="Model name"
                      />
                      <Button size="sm" onClick={saveRename} disabled={!renaming.name.trim()}>
                        Save
                      </Button>
                      <Button variant="ghost" size="sm" onClick={() => setRenaming(null)}>
                        Cancel
                      </Button>
                    </div>
                  ) : (
                    <span className="font-medium">{entry.display_name}</span>
                  )}
                  {servable && stateBadge(entry)}
                  <div className="ml-auto flex gap-2">
                    {servable &&
                      (running ? (
                        <Button variant="outline" size="sm" onClick={() => unload(entry)}>
                          <Square className="mr-1 h-4 w-4" />
                          Unload
                        </Button>
                      ) : (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => load(entry)}
                          disabled={loading.has(entry.id)}
                        >
                          <Play className="mr-1 h-4 w-4" />
                          Load
                        </Button>
                      ))}
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setRenaming({ id: entry.id, name: entry.display_name })}
                      aria-label={`Rename ${entry.display_name}`}
                    >
                      <Pencil className="h-4 w-4" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => {
                        setDeleteFiles(true)
                        setRemoving(entry)
                      }}
                      aria-label={`Remove ${entry.display_name}`}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>
                </div>
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                  <span className="font-mono">{entry.id}</span>
                  <span>{KIND_LABELS[entry.kind]}</span>
                  {entry.quant && <span>{entry.quant}</span>}
                  <span>{formatBytes(entry.size_bytes)}</span>
                  {formatContext(entry.context_length) && <span>{formatContext(entry.context_length)}</span>}
                  {entry.has_tools && <span>Tools</span>}
                  {entry.projector_path && <span>Vision</span>}
                  <span>
                    {entry.source.type === "hugging_face" ? entry.source.repo : "Imported file"}
                  </span>
                  {!servable && <span>Not served (not a chat, completion or embedding model)</span>}
                </div>
                {s?.state === "failed" && s.last_error && (
                  <p className="whitespace-pre-wrap text-xs text-red-500">{s.last_error}</p>
                )}
              </div>
            )
          })
        )}
      </CardContent>

      <AlertDialog open={removing !== null} onOpenChange={(o) => !o && setRemoving(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove {removing?.display_name}?</AlertDialogTitle>
            <AlertDialogDescription>
              {removing?.source.type === "hugging_face"
                ? "The model is unloaded and removed from the library."
                : "The model is unloaded and removed from the library. The imported file stays where it is."}
            </AlertDialogDescription>
          </AlertDialogHeader>
          {removing?.source.type === "hugging_face" && (
            <label className="flex items-center gap-2 text-sm">
              <Checkbox checked={deleteFiles} onCheckedChange={(v) => setDeleteFiles(v === true)} />
              Also delete the downloaded files ({formatBytes(removing.size_bytes)})
            </label>
          )}
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={confirmRemove}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              Remove
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Card>
  )
}

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------

function DownloadsSection({ jobs, onCleared }: { jobs: DownloadJobView[]; onCleared: () => void }) {
  if (jobs.length === 0) return null

  const act = async (cmd: string, id: string) => {
    try {
      await invoke(cmd, { id } satisfies LocalModelsDownloadIdParams)
    } catch (err) {
      toast.error(`${err}`)
    }
  }

  const clearFinished = async () => {
    await invoke("local_models_downloads_clear").catch(() => {})
    onCleared()
  }

  const hasFinished = jobs.some((j) => ["done", "failed", "cancelled"].includes(j.state))

  return (
    <Card>
      <CardHeader>
        <div className="flex items-start justify-between gap-4">
          <div>
            <CardTitle className="text-base">Downloads</CardTitle>
            <CardDescription>
              Downloads are verified against Hugging Face's checksums. Paused downloads resume
              where they stopped.
            </CardDescription>
          </div>
          {hasFinished && (
            <Button variant="outline" size="sm" onClick={clearFinished}>
              Clear finished
            </Button>
          )}
        </div>
      </CardHeader>
      <CardContent className="space-y-3">
        {jobs.map((job) => {
          const pct = job.bytes_total > 0 ? Math.min(100, (job.bytes_done / job.bytes_total) * 100) : 0
          const eta = formatEta(job)
          const active = job.state === "queued" || job.state === "running" || job.state === "verifying"
          return (
            <div key={job.id} className="space-y-2 rounded-md border p-3 text-sm">
              <div className="flex flex-wrap items-center gap-2">
                <span className="font-medium">{job.repo}</span>
                <Badge
                  variant={
                    job.state === "done"
                      ? "success"
                      : job.state === "failed"
                        ? "destructive"
                        : job.state === "cancelled" || job.state === "paused"
                          ? "outline"
                          : "info"
                  }
                >
                  {job.state}
                </Badge>
                <div className="ml-auto flex gap-2">
                  {active && job.state !== "verifying" && (
                    <Button variant="outline" size="sm" onClick={() => act("local_models_download_pause", job.id)}>
                      <Pause className="mr-1 h-4 w-4" />
                      Pause
                    </Button>
                  )}
                  {(job.state === "paused" || job.state === "failed") && (
                    <Button variant="outline" size="sm" onClick={() => act("local_models_download_resume", job.id)}>
                      <Play className="mr-1 h-4 w-4" />
                      {job.state === "failed" ? "Retry" : "Resume"}
                    </Button>
                  )}
                  {job.state !== "done" && job.state !== "cancelled" && job.state !== "verifying" && (
                    <Button variant="ghost" size="sm" onClick={() => act("local_models_download_cancel", job.id)}>
                      <X className="mr-1 h-4 w-4" />
                      Cancel
                    </Button>
                  )}
                </div>
              </div>
              <div className="text-xs text-muted-foreground">
                {(job.current_file ?? job.files.join(", "))}
              </div>
              {job.state !== "done" && job.state !== "cancelled" && (
                <>
                  <Progress value={pct} />
                  <div className="flex flex-wrap gap-x-3 text-xs text-muted-foreground">
                    <span>
                      {formatBytes(job.bytes_done)} of {formatBytes(job.bytes_total)}
                    </span>
                    {job.state === "running" && job.speed_bps > 0 && (
                      <span>{formatBytes(job.speed_bps)}/s</span>
                    )}
                    {eta && <span>{eta}</span>}
                  </div>
                </>
              )}
              {job.error && <p className="whitespace-pre-wrap text-xs text-red-500">{job.error}</p>}
            </div>
          )
        })}
      </CardContent>
    </Card>
  )
}

// ---------------------------------------------------------------------------
// Repo drawer
// ---------------------------------------------------------------------------

type InspectState = RemoteModelInspection | "loading" | { error: string }

/** Where a repo variant stands: from the download jobs and the library. */
type VariantStatus =
  | { kind: "none" }
  | { kind: "active"; job: DownloadJobView }
  | { kind: "failed"; job: DownloadJobView }
  | { kind: "downloaded" }

function variantStatus(
  repo: string,
  variant: GgufVariant,
  jobs: DownloadJobView[],
  library: LocalLibraryView | null,
): VariantStatus {
  const covers = (files: string[]) => variant.files.every((f) => files.includes(f))
  const matching = jobs.filter(
    (j) => j.repo === repo && j.state !== "cancelled" && covers(j.files),
  )
  const job = matching[matching.length - 1]
  if (job && !["done", "failed"].includes(job.state)) return { kind: "active", job }
  const inLibrary = library?.entries.some(
    (e) =>
      e.source.type === "hugging_face" &&
      e.source.repo === repo &&
      variant.files.some((f) => (e.source as { files: string[] }).files.includes(f)),
  )
  if (inLibrary || job?.state === "done") return { kind: "downloaded" }
  if (job?.state === "failed") return { kind: "failed", job }
  return { kind: "none" }
}

function RepoDialog({
  repo,
  kvCache,
  jobs,
  library,
  onClose,
}: {
  repo: string | null
  kvCache: KvCacheType | null
  jobs: DownloadJobView[]
  library: LocalLibraryView | null
  onClose: () => void
}) {
  const [details, setDetails] = useState<LocalRepoDetails | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [inspections, setInspections] = useState<Record<string, InspectState>>({})
  const [starting, setStarting] = useState(false)

  useEffect(() => {
    setDetails(null)
    setError(null)
    setSelected(new Set())
    setInspections({})
    if (!repo) return
    let cancelled = false
    invoke<LocalRepoDetails>("local_models_repo", { repo } satisfies LocalModelsRepoParams)
      .then((d) => !cancelled && setDetails(d))
      .catch((err) => !cancelled && setError(String(err)))
    return () => {
      cancelled = true
    }
  }, [repo])

  const inspect = async (variant: GgufVariant) => {
    if (!details) return
    setInspections((prev) => ({ ...prev, [variant.name]: "loading" }))
    try {
      const result = await invoke<RemoteModelInspection>("local_models_inspect_remote", {
        repo: details.id,
        revision: details.sha,
        path: variant.files[0],
        sizeBytes: variant.size_bytes ?? 0,
        kvCache,
      } satisfies LocalModelsInspectRemoteParams)
      setInspections((prev) => ({ ...prev, [variant.name]: result }))
    } catch (err) {
      setInspections((prev) => ({ ...prev, [variant.name]: { error: String(err) } }))
    }
  }

  const download = async (variants: GgufVariant[]) => {
    if (!details || variants.length === 0) return
    setStarting(true)
    try {
      await invoke<string>("local_models_download_start", {
        repo: details.id,
        revision: details.sha,
        files: variants.flatMap((v) => v.files),
      } satisfies LocalModelsDownloadStartParams)
      toast.success(`Downloading ${variants.map((v) => v.name).join(" + ")}`)
      setSelected(new Set())
    } catch (err) {
      toast.error(`Could not start the download: ${err}`)
    } finally {
      setStarting(false)
    }
  }

  const toggle = (name: string, on: boolean) => {
    setSelected((prev) => {
      const next = new Set(prev)
      if (on) next.add(name)
      else next.delete(name)
      return next
    })
  }

  const selectedVariants = details?.variants.filter((v) => selected.has(v.name)) ?? []

  return (
    <Dialog open={repo !== null} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-h-[85vh] max-w-3xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="break-all">{repo}</DialogTitle>
          <DialogDescription>
            {details
              ? [details.pipeline_tag, details.license && `License: ${details.license}`]
                  .filter(Boolean)
                  .join(" · ") || "GGUF files in this repository"
              : "Loading repository…"}
          </DialogDescription>
        </DialogHeader>

        {error && (
          <div className="flex items-start gap-2 text-sm text-red-500">
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
            <span>{error}</span>
          </div>
        )}
        {!details && !error && (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" /> Loading…
          </div>
        )}

        {details && (
          <div className="space-y-4">
            {details.gated && (
              <div className="space-y-2 rounded-md border border-amber-500/40 bg-amber-500/5 p-3 text-sm">
                <div className="flex items-center gap-2 font-medium">
                  <Lock className="h-4 w-4" />
                  Gated model
                </div>
                <p className="text-muted-foreground">
                  Accept the model's terms on huggingface.co, then sign in to Hugging Face here to
                  download it.
                </p>
                {details.gate_prompt && (
                  <p className="whitespace-pre-wrap text-xs text-muted-foreground">
                    {details.gate_prompt}
                  </p>
                )}
                <Button variant="outline" size="sm" onClick={() => open(details.repo_url)}>
                  <ExternalLink className="mr-1 h-4 w-4" />
                  Request access on huggingface.co
                </Button>
                <HuggingFaceAccountCard description="Downloads of gated models use this account." />
              </div>
            )}

            {details.variants.length === 0 ? (
              <p className="text-sm text-muted-foreground">This repository has no GGUF files.</p>
            ) : (
              <div className="space-y-2">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <p className="text-xs text-muted-foreground">
                    Smaller quantizations use less memory at some cost in quality. For vision,
                    select a model and its projector (mmproj) file together.
                  </p>
                  {selectedVariants.length > 0 && (
                    <Button size="sm" onClick={() => download(selectedVariants)} disabled={starting}>
                      <Download className="mr-1 h-4 w-4" />
                      Download selected ({selectedVariants.length})
                    </Button>
                  )}
                </div>
                {details.variants.map((variant) => {
                  const insp = inspections[variant.name]
                  const status = variantStatus(details.id, variant, jobs, library)
                  const busy = status.kind === "active" || status.kind === "downloaded"
                  const percent =
                    status.kind === "active" && status.job.bytes_total > 0
                      ? Math.floor((status.job.bytes_done / status.job.bytes_total) * 100)
                      : null
                  return (
                    <div key={variant.name} className="space-y-1 rounded-md border p-3 text-sm">
                      <div className="flex flex-wrap items-center gap-2">
                        <Checkbox
                          checked={selected.has(variant.name)}
                          disabled={!variant.complete || busy}
                          onCheckedChange={(v) => toggle(variant.name, v === true)}
                          aria-label={`Select ${variant.name}`}
                        />
                        <span className="break-all font-medium">{variant.name}</span>
                        {variant.quant && <Badge variant="secondary">{variant.quant}</Badge>}
                        <span className="text-xs text-muted-foreground">
                          {formatBytes(variant.size_bytes)}
                          {variant.files.length > 1 && ` in ${variant.files.length} parts`}
                        </span>
                        {status.kind === "active" && (
                          <Badge variant="secondary">
                            {status.job.state === "paused" ? (
                              "Paused"
                            ) : (
                              <>
                                <Loader2 className="mr-1 h-3 w-3 animate-spin" />
                                {status.job.state === "verifying" ? "Verifying" : "Downloading"}
                                {percent != null && ` ${percent}%`}
                              </>
                            )}
                          </Badge>
                        )}
                        {status.kind === "downloaded" && (
                          <Badge variant="secondary" className="text-green-700 dark:text-green-400">
                            Downloaded
                          </Badge>
                        )}
                        {status.kind === "failed" && (
                          <Badge variant="outline" className="text-red-500">
                            Download failed
                          </Badge>
                        )}
                        {insp && insp !== "loading" && !("error" in insp) && (
                          <>
                            <Badge variant={VERDICTS[insp.fit.verdict].variant}>
                              {VERDICTS[insp.fit.verdict].label}
                            </Badge>
                            <span className="text-xs text-muted-foreground">
                              {KIND_LABELS[insp.kind]}
                            </span>
                          </>
                        )}
                        <div className="ml-auto flex gap-2">
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() => inspect(variant)}
                            disabled={insp === "loading" || !variant.complete}
                          >
                            {insp === "loading" && <Loader2 className="mr-1 h-4 w-4 animate-spin" />}
                            Check fit
                          </Button>
                          <Button
                            size="sm"
                            onClick={() => download([variant])}
                            disabled={starting || !variant.complete || busy}
                            title={
                              status.kind === "active"
                                ? "Downloading: see Downloads on the Models tab"
                                : undefined
                            }
                          >
                            {status.kind === "active" ? (
                              <Loader2 className="mr-1 h-4 w-4 animate-spin" />
                            ) : (
                              <Download className="mr-1 h-4 w-4" />
                            )}
                            {status.kind === "downloaded"
                              ? "Downloaded"
                              : status.kind === "active"
                                ? "Downloading"
                                : status.kind === "failed"
                                  ? "Retry"
                                  : "Download"}
                          </Button>
                        </div>
                      </div>
                      {!variant.complete && (
                        <p className="text-xs text-muted-foreground">Some split parts are missing.</p>
                      )}
                      {status.kind === "active" && (
                        <Progress
                          value={percent ?? 0}
                          className="h-1.5"
                          aria-label={`Download progress of ${variant.name}`}
                        />
                      )}
                      {status.kind === "failed" && status.job.error && (
                        <p className="whitespace-pre-wrap break-words text-xs text-red-500">
                          {status.job.error}
                        </p>
                      )}
                      {insp && insp !== "loading" && "error" in insp && (
                        <p className="text-xs text-red-500">{insp.error}</p>
                      )}
                      {insp && insp !== "loading" && !("error" in insp) && (
                        <p className="text-xs text-muted-foreground">
                          Needs about {formatBytes(insp.fit.total_bytes)} at{" "}
                          {formatContext(insp.fit.context_length)}
                          {insp.fit.budget_bytes != null && <> of {formatBytes(insp.fit.budget_bytes)} available</>}
                          {insp.max_context != null && <>; up to {formatContext(insp.max_context)} fits</>}
                          {insp.summary.architecture && <> · {insp.summary.architecture}</>}
                          {insp.summary.chat_template_mentions_tools && <> · tool calling</>}
                        </p>
                      )}
                    </div>
                  )
                })}
              </div>
            )}
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}

// ---------------------------------------------------------------------------
// Discover
// ---------------------------------------------------------------------------

function DiscoverSection({
  kvCache,
  jobs,
  library,
}: {
  kvCache: KvCacheType | null
  jobs: DownloadJobView[]
  library: LocalLibraryView | null
}) {
  const [query, setQuery] = useState("")
  const [ggufOnly, setGgufOnly] = useState(true)
  const [sort, setSort] = useState<NonNullable<LocalModelsSearchParams["sort"]>>("trendingScore")
  const [results, setResults] = useState<HubModelSummary[] | null>(null)
  const [cursor, setCursor] = useState<string | null>(null)
  const [searching, setSearching] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [openRepo, setOpenRepo] = useState<string | null>(null)
  const [hardware, setHardware] = useState<HardwareInfo | null>(null)

  useEffect(() => {
    invoke<HardwareInfo>("local_models_hardware").then(setHardware).catch(() => {})
  }, [])

  const search = async (more: boolean) => {
    setSearching(true)
    setError(null)
    try {
      const page = await invoke<HubPage>("local_models_search", {
        query: query.trim() || null,
        filters: ggufOnly ? ["gguf"] : [],
        sort,
        cursor: more ? cursor : null,
      } satisfies LocalModelsSearchParams)
      setResults((prev) => (more && prev ? [...prev, ...page.models] : page.models))
      setCursor(page.next_cursor)
    } catch (err) {
      setError(String(err))
    } finally {
      setSearching(false)
    }
  }

  const budget = hardware?.gpu_budget_bytes ?? hardware?.available_ram_bytes ?? null

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">Hugging Face</CardTitle>
        <CardDescription>
          Search GGUF models on huggingface.co. Nothing is sent until you search.
          {hardware && budget != null && (
            <>
              {" "}
              This machine: {formatBytes(hardware.total_ram_bytes)} memory, about {formatBytes(budget)}{" "}
              {hardware.unified_memory ? "usable by the GPU" : "available"} for models.
            </>
          )}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <form
          className="flex flex-wrap items-center gap-2"
          onSubmit={(e) => {
            e.preventDefault()
            search(false)
          }}
        >
          <Input
            className="min-w-[200px] flex-1"
            placeholder="Search models, e.g. qwen3 8b"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label="Search Hugging Face"
          />
          <Select value={sort} onValueChange={(v) => setSort(v as typeof sort)}>
            <SelectTrigger className="w-44" aria-label="Sort">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {SORTS.map((s) => (
                <SelectItem key={s.value} value={s.value}>
                  {s.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <label className="flex items-center gap-2 text-sm">
            <Checkbox checked={ggufOnly} onCheckedChange={(v) => setGgufOnly(v === true)} />
            GGUF only
          </label>
          <Button type="submit" size="sm" disabled={searching}>
            {searching ? <Loader2 className="mr-1 h-4 w-4 animate-spin" /> : <Search className="mr-1 h-4 w-4" />}
            Search
          </Button>
        </form>

        {error && (
          <div className="flex items-start gap-2 text-sm text-red-500">
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
            <span>{error}</span>
          </div>
        )}

        {results !== null && results.length === 0 && !searching && (
          <p className="text-sm text-muted-foreground">No models found.</p>
        )}

        {results && results.length > 0 && (
          <div className="space-y-1">
            {results.map((m) => (
              <button
                key={m.id}
                type="button"
                className="flex w-full flex-wrap items-center gap-2 rounded-md border px-3 py-2 text-left text-sm hover:bg-accent"
                onClick={() => setOpenRepo(m.id)}
              >
                <span className="break-all font-medium">{m.id}</span>
                {m.gated && (
                  <Badge variant="warning">
                    <Lock className="mr-1 h-3 w-3" />
                    Gated
                  </Badge>
                )}
                <span className="ml-auto flex items-center gap-3 text-xs text-muted-foreground">
                  {m.architecture && <span>{m.architecture}</span>}
                  {m.parameters != null && <span>{(m.parameters / 1e9).toFixed(1)}B params</span>}
                  <span className="flex items-center gap-1">
                    <Download className="h-3 w-3" />
                    {formatCount(m.downloads)}
                  </span>
                  <span className="flex items-center gap-1">
                    <Heart className="h-3 w-3" />
                    {formatCount(m.likes)}
                  </span>
                </span>
              </button>
            ))}
            {cursor && (
              <Button variant="outline" size="sm" onClick={() => search(true)} disabled={searching}>
                {searching && <Loader2 className="mr-1 h-4 w-4 animate-spin" />}
                Load more
              </Button>
            )}
          </div>
        )}
      </CardContent>
      <RepoDialog
        repo={openRepo}
        kvCache={kvCache}
        jobs={jobs}
        library={library}
        onClose={() => setOpenRepo(null)}
      />
    </Card>
  )
}

// ---------------------------------------------------------------------------
// Tab
// ---------------------------------------------------------------------------

interface LocalModelsTabProps {
  instanceName: string
  /** The provider's KV cache setting, used for memory estimates. */
  kvCache?: string | null
}

/** Models tab of the llama.cpp Local Embedded provider. */
export function LocalModelsTab({ instanceName, kvCache }: LocalModelsTabProps) {
  const [library, setLibrary] = useState<LocalLibraryView | null>(null)
  const [jobs, setJobs] = useState<DownloadJobView[]>([])

  const loadLibrary = useCallback(async () => {
    try {
      setLibrary(await invoke<LocalLibraryView>("local_models_library"))
    } catch (err) {
      toast.error(`Could not read the model library: ${err}`)
    }
  }, [])

  const loadJobs = useCallback(async () => {
    setJobs(await invoke<DownloadJobView[]>("local_models_downloads").catch(() => []))
  }, [])

  useEffect(() => {
    loadLibrary()
    loadJobs()
    const progress = listenSafe<DownloadJobView>("local-model-download-progress", (e) => {
      const job = e.payload
      setJobs((prev) => {
        const i = prev.findIndex((j) => j.id === job.id)
        if (i === -1) return [...prev, job]
        const next = prev.slice()
        next[i] = job
        return next
      })
    })
    const finished = listenSafe<DownloadFinishedEvent>("local-model-download-finished", (e) => {
      const { job, added_models, library_error } = e.payload
      if (job.state === "done") {
        if (library_error) toast.error(`Downloaded ${job.repo}, but it could not be added: ${library_error}`)
        else if (added_models.length === 0) toast.warning(`Downloaded ${job.repo}, but it contains no usable model`)
        else toast.success(`Downloaded ${job.repo}`)
      } else if (job.state === "failed") {
        toast.error(`Download of ${job.repo} failed${job.error ? `: ${job.error}` : ""}`)
      }
      loadLibrary()
    })
    const changed = listenSafe<LibraryChangedEvent>("local-models-library-changed", () => loadLibrary())
    return () => {
      progress.cleanup()
      finished.cleanup()
      changed.cleanup()
    }
  }, [loadLibrary, loadJobs])

  const kv: KvCacheType | null =
    kvCache === "f16" || kvCache === "q8_0" || kvCache === "q4_0" ? kvCache : null

  return (
    <div className="space-y-6">
      <LibrarySection instanceName={instanceName} library={library} onChanged={loadLibrary} />
      <DownloadsSection jobs={jobs} onCleared={loadJobs} />
      <DiscoverSection kvCache={kv} jobs={jobs} library={library} />
      <HuggingFaceAccountCard />
    </div>
  )
}
