import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Loader2, Plus } from "lucide-react"
import { Button } from "@/components/ui/Button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import type { GetProviderModelMetadataParams } from "@/types/tauri-commands"
import { pointerToken } from "./rules"

interface MetadataInspectorProps {
  instanceName: string
  /** The model to inspect; closed when null */
  modelId: string | null
  onClose: () => void
  /** Add a rule on this field (JSON Pointer) with its current value */
  onUseField: (pointer: string, value: unknown) => void
}

/**
 * The model's raw entry in the provider's model list, as a tree. Each value
 * can start a "Listing field" rule.
 */
export function MetadataInspector({ instanceName, modelId, onClose, onUseField }: MetadataInspectorProps) {
  const [loading, setLoading] = useState(false)
  const [entry, setEntry] = useState<unknown>(undefined)

  useEffect(() => {
    if (!modelId) return
    let cancelled = false
    setLoading(true)
    setEntry(undefined)
    invoke<unknown>("get_provider_model_metadata", {
      instanceName,
      modelId,
    } satisfies GetProviderModelMetadataParams)
      .then((v) => {
        if (!cancelled) setEntry(v ?? null)
      })
      .catch(() => {
        if (!cancelled) setEntry(null)
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [instanceName, modelId])

  return (
    <Dialog open={modelId != null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Model list entry</DialogTitle>
          <DialogDescription className="font-mono text-xs break-all">{modelId}</DialogDescription>
        </DialogHeader>
        <p className="text-xs text-muted-foreground">
          What the provider's model list says about this model. Pick a value to add a rule that
          compares that field.
        </p>
        <div className="max-h-[60vh] overflow-auto rounded-md border bg-muted/30 p-2 font-mono text-xs">
          {loading ? (
            <div className="flex items-center gap-2 p-2 text-muted-foreground font-sans">
              <Loader2 className="h-4 w-4 animate-spin" />
              Loading...
            </div>
          ) : entry === null || entry === undefined ? (
            <p className="p-2 text-muted-foreground font-sans">
              The provider's model list has no entry for this model (it was added by hand, or the
              provider does not share its model list). Rules on listing fields cannot match it.
            </p>
          ) : (
            <JsonNode
              value={entry}
              pointer=""
              name={null}
              depth={0}
              onUse={(pointer, value) => {
                onUseField(pointer, value)
                onClose()
              }}
            />
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}

function JsonNode({
  value,
  pointer,
  name,
  depth,
  onUse,
}: {
  value: unknown
  pointer: string
  name: string | null
  depth: number
  onUse: (pointer: string, value: unknown) => void
}) {
  const indent = { paddingLeft: `${depth * 14}px` }
  const label = name != null ? <span className="text-muted-foreground">{name}: </span> : null

  if (Array.isArray(value) || (value !== null && typeof value === "object")) {
    const entries: [string, unknown][] = Array.isArray(value)
      ? value.map((v, i) => [String(i), v])
      : Object.entries(value as Record<string, unknown>)
    const [open, close] = Array.isArray(value) ? ["[", "]"] : ["{", "}"]
    return (
      <div>
        <div style={indent} className="py-0.5">
          {label}
          <span className="text-muted-foreground">{entries.length === 0 ? `${open}${close}` : open}</span>
        </div>
        {entries.map(([key, child]) => (
          <JsonNode
            key={key}
            value={child}
            pointer={`${pointer}/${pointerToken(key)}`}
            name={key}
            depth={depth + 1}
            onUse={onUse}
          />
        ))}
        {entries.length > 0 && (
          <div style={indent} className="py-0.5 text-muted-foreground">
            {close}
          </div>
        )}
      </div>
    )
  }

  const shown = typeof value === "string" ? JSON.stringify(value) : String(value)
  return (
    <div style={indent} className="group flex items-center gap-2 rounded py-0.5 pr-1 hover:bg-muted">
      <span className="min-w-0 flex-1 truncate" title={`${pointer}: ${shown}`}>
        {label}
        <span className={typeof value === "string" ? "text-green-700 dark:text-green-400" : "text-blue-700 dark:text-blue-400"}>
          {shown}
        </span>
      </span>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="h-6 px-2 text-[11px] font-sans opacity-60 group-hover:opacity-100"
        onClick={() => onUse(pointer, value)}
        aria-label={`Add a rule on ${pointer}`}
      >
        <Plus className="h-3 w-3 mr-1" />
        Rule
      </Button>
    </div>
  )
}
