import { useCallback, useEffect, useMemo, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { AlertCircle, Info, Loader2, Pencil, Plus, RefreshCw, Search, Trash2, X } from "lucide-react"
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
import { Alert, AlertDescription } from "@/components/ui/alert"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card"
import { Checkbox } from "@/components/ui/checkbox"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Input } from "@/components/ui/Input"
import { Switch } from "@/components/ui/Toggle"
import { Textarea } from "@/components/ui/textarea"
import { listenSafe } from "@/hooks/useTauriListener"
import { cn } from "@/lib/utils"
import type {
  GetProviderModelSettingsParams,
  ListProviderModelsManagedParams,
  ManagedModelInfo,
  ModelCapabilityKind,
  ModelEntry,
  ProviderManagedModels,
  ProviderModelSettings,
  SetProviderModelSettingsParams,
} from "@/types/tauri-commands"

/** Capabilities offered in the override editor, in display order. */
const CAPABILITIES: { kind: ModelCapabilityKind; label: string }[] = [
  { kind: "chat", label: "Chat" },
  { kind: "completion", label: "Completion" },
  { kind: "function_calling", label: "Tool calling" },
  { kind: "vision", label: "Vision" },
  { kind: "embedding", label: "Embeddings" },
  { kind: "audio", label: "Audio input" },
  { kind: "text_to_speech", label: "Text to speech" },
  { kind: "image_generation", label: "Image generation" },
  { kind: "decision", label: "Decisions" },
]

const CAPABILITY_LABEL: Partial<Record<ModelCapabilityKind, string>> = Object.fromEntries(
  CAPABILITIES.map((c) => [c.kind, c.label]),
)

function formatContext(tokens: number): string {
  if (!tokens) return ""
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(tokens % 1_000_000 ? 1 : 0)}M`
  if (tokens >= 1000) return `${Math.round(tokens / 1000)}k`
  return String(tokens)
}

function formatPrice(n: number): string {
  return n === 0 ? "$0" : `$${n < 0.01 ? n.toPrecision(2) : n.toFixed(2)}`
}

/** Case-insensitive match on id and display name. */
export function modelMatchesSearch(model: { id: string; name: string }, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  return model.id.toLowerCase().includes(q) || model.name.toLowerCase().includes(q)
}

interface ProviderModelsTabProps {
  instanceName: string
  /** The provider's config value of the "Discover models" switch */
  discoveryValue?: string
  /** Save the "Discover models" switch (a provider config field) */
  onDiscoveryChange: (enabled: boolean) => void
  /** Prefill the search box (e.g. from the command palette) */
  initialSearch?: string
}

/**
 * A provider's models: discovery, which models are offered, manual models
 * and per-model overrides (name, context window, capabilities, price).
 */
export function ProviderModelsTab({
  instanceName,
  discoveryValue,
  onDiscoveryChange,
  initialSearch,
}: ProviderModelsTabProps) {
  const [data, setData] = useState<ProviderManagedModels | null>(null)
  const [settings, setSettings] = useState<ProviderModelSettings>({ entries: [] })
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [saving, setSaving] = useState(false)
  const [search, setSearch] = useState(initialSearch ?? "")
  const [addOpen, setAddOpen] = useState(false)
  const [addText, setAddText] = useState("")
  const [editing, setEditing] = useState<ManagedModelInfo | null>(null)
  const [removing, setRemoving] = useState<ManagedModelInfo | null>(null)

  useEffect(() => {
    if (initialSearch !== undefined) setSearch(initialSearch)
  }, [initialSearch])

  const load = useCallback(
    async (refresh = false) => {
      try {
        const [models, current] = await Promise.all([
          invoke<ProviderManagedModels>("list_provider_models_managed", {
            instanceName,
            refresh,
          } satisfies ListProviderModelsManagedParams),
          invoke<ProviderModelSettings>("get_provider_model_settings", {
            instanceName,
          } satisfies GetProviderModelSettingsParams),
        ])
        setData(models)
        setSettings(current)
      } catch (err) {
        toast.error(`Could not load models: ${err}`)
      } finally {
        setLoading(false)
        setRefreshing(false)
      }
    },
    [instanceName],
  )

  useEffect(() => {
    setLoading(true)
    load()
    // A provider config change (e.g. the discovery switch) lists again.
    const changed = listenSafe("providers-changed", () => load())
    return () => changed.cleanup()
  }, [load])

  const save = async (next: ProviderModelSettings) => {
    setSaving(true)
    const previous = settings
    setSettings(next)
    try {
      const models = await invoke<ProviderManagedModels>("set_provider_model_settings", {
        instanceName,
        settings: next,
      } satisfies SetProviderModelSettingsParams)
      setData(models)
      return true
    } catch (err) {
      setSettings(previous)
      toast.error(`Could not save: ${err}`)
      return false
    } finally {
      setSaving(false)
    }
  }

  const entries = settings.entries ?? []
  const entryFor = (id: string) => entries.find((e) => e.id === id)

  /** Settings with `patch` applied to each listed model's entry. */
  const withEntries = (ids: string[], patch: (e: ModelEntry) => ModelEntry | null): ProviderModelSettings => {
    const wanted = new Set(ids)
    const next: ModelEntry[] = []
    for (const e of entries) {
      if (wanted.has(e.id)) {
        wanted.delete(e.id)
        const p = patch({ ...e })
        if (p) next.push(p)
      } else {
        next.push(e)
      }
    }
    for (const id of wanted) {
      const p = patch({ id })
      if (p) next.push(p)
    }
    return { ...settings, entries: next }
  }

  const models = data?.models ?? []
  const filtered = useMemo(() => models.filter((m) => modelMatchesSearch(m, search)), [models, search])
  const togglable = filtered.filter((m) => m.source !== "missing")
  const offeredCount = models.filter((m) => m.enabled && m.source !== "missing").length
  const missingCount = models.filter((m) => m.source === "missing").length

  const setEnabled = (ids: string[], enabled: boolean) =>
    save(withEntries(ids, (e) => ({ ...e, enabled })))

  const addModels = async () => {
    const ids = Array.from(
      new Set(
        addText
          .split(/[\n,]/)
          .map((s) => s.trim())
          .filter(Boolean),
      ),
    )
    if (ids.length === 0) return
    const ok = await save(withEntries(ids, (e) => ({ ...e, manual: true, enabled: e.enabled ?? null })))
    if (ok) {
      toast.success(ids.length === 1 ? `Added ${ids[0]}` : `Added ${ids.length} models`)
      setAddText("")
      setAddOpen(false)
    }
  }

  const removeModel = async (model: ManagedModelInfo) => {
    // A manual model that the provider also lists stays, as a listed model.
    const ok = await save(withEntries([model.id], () => null))
    if (ok) toast.success(`Removed ${model.id}`)
  }

  if (loading && !data) {
    return (
      <div className="flex items-center gap-2 text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        <span>Loading models...</span>
      </div>
    )
  }

  // The config value updates as soon as the switch moves; the listing
  // follows once the provider is updated.
  const discoveryOn =
    data?.has_discovery_switch && discoveryValue !== undefined
      ? discoveryValue !== "false"
      : (data?.discovery_enabled ?? true)

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader className="pb-3">
          <div className="flex items-start justify-between gap-4">
            <div>
              <CardTitle className="text-sm">Model list</CardTitle>
              <CardDescription>
                {data?.has_discovery_switch
                  ? "Where this provider's models come from and which new ones are offered."
                  : "Which models this provider offers to clients."}
              </CardDescription>
            </div>
            <Button
              variant="outline"
              size="sm"
              disabled={refreshing || !discoveryOn}
              onClick={() => {
                setRefreshing(true)
                load(true)
              }}
            >
              <RefreshCw className={cn("h-3.5 w-3.5 mr-1.5", refreshing && "animate-spin")} />
              Refresh
            </Button>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          {data?.has_discovery_switch && (
            <div className="flex items-center justify-between gap-4">
              <div>
                <p className="text-sm font-medium">Discover models</p>
                <p className="text-xs text-muted-foreground">
                  Ask the server which models it has. Turn off for servers without a model list
                  (GET /models); then only the models you add are offered.
                </p>
              </div>
              <Switch
                checked={discoveryOn}
                onCheckedChange={(on) => onDiscoveryChange(on)}
                aria-label="Discover models"
              />
            </div>
          )}
          <div className="flex items-center justify-between gap-4">
            <div>
              <p className="text-sm font-medium">Offer new models</p>
              <p className="text-xs text-muted-foreground">
                Models the provider lists that you have not turned on or off yourself.
              </p>
            </div>
            <Switch
              checked={data?.show_new_models ?? true}
              disabled={saving || !discoveryOn}
              onCheckedChange={(on) => save({ ...settings, show_new_models: on })}
              aria-label="Offer new models"
            />
          </div>
          {data?.new_models_hidden_by_provider && (
            <Alert>
              <Info className="h-4 w-4" />
              <AlertDescription className="text-xs">
                LocalRouter could not tell which models are installed (no <code>flm</code> command
                found and no model folder on this machine), so the list below is everything the
                server supports. Turn on the models you have installed; requesting one that is not
                installed starts a download.
              </AlertDescription>
            </Alert>
          )}
          {data?.discovery_error && (
            <Alert variant="destructive">
              <AlertCircle className="h-4 w-4" />
              <AlertDescription className="text-xs break-all">
                Listing models failed: {data.discovery_error}
              </AlertDescription>
            </Alert>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <div className="flex items-start justify-between gap-4">
            <div>
              <CardTitle className="text-sm">Models</CardTitle>
              <CardDescription>
                {offeredCount} of {models.length - missingCount} offered
                {data && data.hidden_count > 0 ? ` · ${data.hidden_count} hidden` : ""}
                {missingCount > 0 ? ` · ${missingCount} not listed now` : ""}
              </CardDescription>
            </div>
            <Button size="sm" onClick={() => setAddOpen(true)}>
              <Plus className="h-3.5 w-3.5 mr-1.5" />
              Add models
            </Button>
          </div>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="flex flex-wrap items-center gap-2">
            <div className="relative flex-1 min-w-[200px]">
              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
              <Input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder="Filter by model id or name"
                className="pl-8 pr-8 h-8 text-sm"
                aria-label="Filter models"
              />
              {search && (
                <button
                  type="button"
                  onClick={() => setSearch("")}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                  aria-label="Clear filter"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              )}
            </div>
            <span className="text-xs text-muted-foreground whitespace-nowrap">
              {filtered.length} of {models.length} models
            </span>
            <Button
              variant="outline"
              size="sm"
              className="h-8"
              disabled={saving || togglable.length === 0}
              onClick={() => setEnabled(togglable.map((m) => m.id), true)}
            >
              Enable {search ? "shown" : "all"}
            </Button>
            <Button
              variant="outline"
              size="sm"
              className="h-8"
              disabled={saving || togglable.length === 0}
              onClick={() => setEnabled(togglable.map((m) => m.id), false)}
            >
              Disable {search ? "shown" : "all"}
            </Button>
          </div>

          {models.length === 0 ? (
            <p className="text-sm text-muted-foreground py-4 text-center">
              {discoveryOn
                ? "The provider lists no models. Add the ones it serves with Add models."
                : "Discovery is off. Add the models this server serves with Add models."}
            </p>
          ) : filtered.length === 0 ? (
            <p className="text-sm text-muted-foreground py-4 text-center">
              No models match &ldquo;{search}&rdquo;
            </p>
          ) : (
            <div className="divide-y rounded-md border">
              {filtered.map((model) => (
                <ModelRow
                  key={model.id}
                  model={model}
                  disabled={saving}
                  onToggle={(on) => setEnabled([model.id], on)}
                  onEdit={() => setEditing(model)}
                  onRemove={() => setRemoving(model)}
                />
              ))}
            </div>
          )}
        </CardContent>
      </Card>

      <Dialog open={addOpen} onOpenChange={setAddOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Add models</DialogTitle>
            <DialogDescription>
              Model ids exactly as the provider expects them, one per line. They are offered even
              when the provider does not list them.
            </DialogDescription>
          </DialogHeader>
          <Textarea
            value={addText}
            onChange={(e) => setAddText(e.target.value)}
            placeholder={"meta-llama/Llama-3.3-70B-Instruct\nqwen3:8b"}
            rows={5}
            className="font-mono text-xs"
            autoFocus
          />
          <DialogFooter>
            <Button variant="outline" onClick={() => setAddOpen(false)}>
              Cancel
            </Button>
            <Button onClick={addModels} disabled={saving || !addText.trim()}>
              Add
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {editing && (
        <OverrideDialog
          model={editing}
          entry={entryFor(editing.id)}
          saving={saving}
          onClose={() => setEditing(null)}
          onSave={async (patch) => {
            const ok = await save(withEntries([editing.id], (e) => patch(e)))
            if (ok) setEditing(null)
          }}
        />
      )}

      <AlertDialog open={!!removing} onOpenChange={(open) => !open && setRemoving(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {removing?.source === "missing" ? "Forget settings" : "Remove model"}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {removing?.source === "missing"
                ? `Forget the settings saved for ${removing?.id}? The provider no longer lists it.`
                : removing?.source === "manual_and_discovered"
                  ? `Remove your settings for ${removing?.id}? The provider lists it, so it stays as a listed model.`
                  : `Remove ${removing?.id}? Requests for it will no longer be routed to this provider.`}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (removing) removeModel(removing)
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

function ModelRow({
  model,
  disabled,
  onToggle,
  onEdit,
  onRemove,
}: {
  model: ManagedModelInfo
  disabled: boolean
  onToggle: (on: boolean) => void
  onEdit: () => void
  onRemove: () => void
}) {
  const missing = model.source === "missing"
  const manual = model.source === "manual" || model.source === "manual_and_discovered"
  const hasPrice = model.input_price_per_million != null || model.output_price_per_million != null
  return (
    <div className={cn("flex items-center gap-3 px-3 py-2", !model.enabled && "bg-muted/30")}>
      <Switch
        checked={model.enabled && !missing}
        disabled={disabled || missing}
        onCheckedChange={onToggle}
        aria-label={`Offer ${model.id}`}
      />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2 min-w-0">
          <p className={cn("text-sm font-medium truncate", !model.enabled && "text-muted-foreground")}>
            {model.name}
          </p>
          {manual && (
            <Badge variant="info" className="text-[10px] px-1.5 py-0">
              Added
            </Badge>
          )}
          {missing && (
            <Badge variant="warning" className="text-[10px] px-1.5 py-0">
              Not listed
            </Badge>
          )}
          {model.has_overrides && (
            <Badge variant="outline" className="text-[10px] px-1.5 py-0">
              Edited
            </Badge>
          )}
        </div>
        {model.name !== model.id && (
          <p className="text-xs text-muted-foreground font-mono truncate">{model.id}</p>
        )}
        <div className="flex flex-wrap gap-1 mt-1">
          {model.capabilities
            .filter((c) => c !== "completion" && CAPABILITY_LABEL[c])
            .map((c) => (
              <span key={c} className="text-[10px] rounded bg-muted px-1.5 py-0.5 text-muted-foreground">
                {CAPABILITY_LABEL[c]}
              </span>
            ))}
        </div>
      </div>
      <div className="flex items-center gap-2 shrink-0 text-xs text-muted-foreground">
        {hasPrice && (
          <span className="whitespace-nowrap" title="Price per 1M tokens (input / output)">
            {formatPrice(model.input_price_per_million ?? 0)} / {formatPrice(model.output_price_per_million ?? 0)} per 1M
          </span>
        )}
        {model.context_window > 0 && (
          <Badge variant="secondary" className="text-xs whitespace-nowrap">
            {formatContext(model.context_window)} ctx
          </Badge>
        )}
        <Button variant="ghost" size="icon" className="h-7 w-7" onClick={onEdit} aria-label={`Edit ${model.id}`}>
          <Pencil className="h-3.5 w-3.5" />
        </Button>
        {(manual || missing) && (
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={onRemove}
            aria-label={`Remove ${model.id}`}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        )}
      </div>
    </div>
  )
}

function OverrideDialog({
  model,
  entry,
  saving,
  onClose,
  onSave,
}: {
  model: ManagedModelInfo
  entry: ModelEntry | undefined
  saving: boolean
  onClose: () => void
  onSave: (patch: (e: ModelEntry) => ModelEntry) => void
}) {
  const [name, setName] = useState(entry?.display_name ?? "")
  const [context, setContext] = useState(entry?.context_window ? String(entry.context_window) : "")
  const [overrideCaps, setOverrideCaps] = useState(entry?.capabilities != null)
  const [caps, setCaps] = useState<ModelCapabilityKind[]>(entry?.capabilities ?? model.capabilities)
  const [input, setInput] = useState(entry?.pricing ? String(entry.pricing.input_per_million) : "")
  const [output, setOutput] = useState(entry?.pricing ? String(entry.pricing.output_per_million) : "")

  const contextValue = context.trim() ? Number(context) : null
  const contextInvalid = contextValue != null && (!Number.isInteger(contextValue) || contextValue <= 0)
  const priceSet = input.trim() !== "" || output.trim() !== ""
  const inputValue = input.trim() ? Number(input) : 0
  const outputValue = output.trim() ? Number(output) : 0
  const priceInvalid =
    priceSet && (!Number.isFinite(inputValue) || !Number.isFinite(outputValue) || inputValue < 0 || outputValue < 0)

  const toggleCap = (kind: ModelCapabilityKind, on: boolean) =>
    setCaps((prev) => (on ? [...prev.filter((c) => c !== kind), kind] : prev.filter((c) => c !== kind)))

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Edit model</DialogTitle>
          <DialogDescription className="font-mono text-xs break-all">{model.id}</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <div className="grid grid-cols-2 gap-3">
            <div className="col-span-2">
              <label className="block text-xs font-medium mb-1">Display name</label>
              <Input value={name} onChange={(e) => setName(e.target.value)} placeholder={model.id} />
            </div>
            <div className="col-span-2">
              <label className="block text-xs font-medium mb-1">Context window (tokens)</label>
              <Input
                type="number"
                min={1}
                value={context}
                onChange={(e) => setContext(e.target.value)}
                placeholder={String(model.context_window || "")}
              />
              {contextInvalid && <p className="text-xs text-destructive mt-1">Enter a whole number of tokens.</p>}
            </div>
            <div>
              <label className="block text-xs font-medium mb-1">Input price per 1M tokens (USD)</label>
              <Input type="number" min={0} step="any" value={input} onChange={(e) => setInput(e.target.value)} placeholder="Not set" />
            </div>
            <div>
              <label className="block text-xs font-medium mb-1">Output price per 1M tokens (USD)</label>
              <Input type="number" min={0} step="any" value={output} onChange={(e) => setOutput(e.target.value)} placeholder="Not set" />
            </div>
            {priceInvalid && <p className="col-span-2 text-xs text-destructive">Prices must be zero or more.</p>}
            {priceSet && !priceInvalid && (inputValue > 0 || outputValue > 0) && (
              <p className="col-span-2 text-xs text-muted-foreground">
                Used for cost tracking. On a local or subscription provider, a price above zero makes this
                model count as paid in free-tier-only strategies.
              </p>
            )}
          </div>
          <div>
            <div className="flex items-center gap-2 mb-2">
              <Checkbox
                id="override-caps"
                checked={overrideCaps}
                onCheckedChange={(v) => setOverrideCaps(v === true)}
              />
              <label htmlFor="override-caps" className="text-xs font-medium">
                Set capabilities by hand (replaces the detected ones)
              </label>
            </div>
            <div className={cn("grid grid-cols-3 gap-2", !overrideCaps && "opacity-50")}>
              {CAPABILITIES.map((c) => (
                <label key={c.kind} className="flex items-center gap-2 text-xs">
                  <Checkbox
                    checked={caps.includes(c.kind)}
                    disabled={!overrideCaps}
                    onCheckedChange={(v) => toggleCap(c.kind, v === true)}
                  />
                  {c.label}
                </label>
              ))}
            </div>
          </div>
        </div>
        <DialogFooter className="gap-2 sm:justify-between">
          <Button
            variant="ghost"
            disabled={saving || !entry || !(entry.display_name || entry.context_window || entry.capabilities || entry.pricing)}
            onClick={() =>
              onSave((e) => ({ ...e, display_name: null, context_window: null, capabilities: null, pricing: null }))
            }
          >
            Reset to listed values
          </Button>
          <div className="flex gap-2">
            <Button variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button
              disabled={saving || contextInvalid || priceInvalid}
              onClick={() =>
                onSave((e) => ({
                  ...e,
                  display_name: name.trim() || null,
                  context_window: contextValue,
                  capabilities: overrideCaps ? caps : null,
                  pricing: priceSet ? { input_per_million: inputValue, output_per_million: outputValue } : null,
                }))
              }
            >
              Save
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
