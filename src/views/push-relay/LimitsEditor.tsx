import { useEffect, useMemo, useState } from "react"
import { Loader2, RotateCcw } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { cn } from "@/lib/utils"
import type { PushRelayLimits } from "@/types/tauri-commands"
import { LIMIT_GROUPS } from "./push-relay-utils"

function display(value: number, scale: number): string {
  const v = value / scale
  return Number.isInteger(v) ? String(v) : String(Math.round(v * 1000) / 1000)
}

function formatRetries(delays: number[]): string {
  return delays.join(", ")
}

function parseRetries(text: string): number[] | null {
  const parts = text.split(",").map((p) => p.trim()).filter(Boolean)
  const nums = parts.map(Number)
  return nums.every((n) => Number.isFinite(n) && n >= 0 && Number.isInteger(n)) ? nums : null
}

interface LimitsEditorProps {
  limits: PushRelayLimits
  defaults: PushRelayLimits
  /** Persist the whole block; rejects with the backend's validation message. */
  onSave: (limits: PushRelayLimits) => Promise<void>
  disabled?: boolean
}

/** Edit every Push Relay limit, with units and the default beside each. */
export function LimitsEditor({ limits, defaults, onSave, disabled }: LimitsEditorProps) {
  const [draft, setDraft] = useState<Record<string, string>>({})
  const [saving, setSaving] = useState(false)

  const fromLimits = useMemo(() => {
    const d: Record<string, string> = {}
    for (const g of LIMIT_GROUPS) for (const r of g.rows) d[r.key] = display(limits[r.key], r.scale)
    d.webhook_retry_delays_secs = formatRetries(limits.webhook_retry_delays_secs)
    return d
  }, [limits])

  useEffect(() => setDraft(fromLimits), [fromLimits])

  const parsed = useMemo(() => {
    const out: PushRelayLimits = { ...limits }
    const errors: Record<string, string> = {}
    for (const g of LIMIT_GROUPS) {
      for (const r of g.rows) {
        const n = Number(draft[r.key])
        if (draft[r.key] === undefined || draft[r.key].trim() === "" || !Number.isFinite(n) || n < 0) {
          errors[r.key] = "Enter a number"
        } else {
          out[r.key] = Math.round(n * r.scale)
        }
      }
    }
    const retries = parseRetries(draft.webhook_retry_delays_secs ?? "")
    if (retries === null) errors.webhook_retry_delays_secs = "Comma-separated whole seconds"
    else out.webhook_retry_delays_secs = retries
    return { out, errors }
  }, [draft, limits])

  const dirty = Object.keys(fromLimits).some((k) => fromLimits[k] !== draft[k])
  const isDefault = JSON.stringify(limits) === JSON.stringify(defaults)
  const hasErrors = Object.keys(parsed.errors).length > 0

  const save = async (value: PushRelayLimits) => {
    setSaving(true)
    try {
      await onSave(value)
    } finally {
      setSaving(false)
    }
  }

  const row = (key: string, label: string, help: string, unit: string, defaultText: string) => (
    <div key={key} className="grid grid-cols-[1fr_auto] items-center gap-3 py-1.5 border-b border-border/50 last:border-0">
      <div className="min-w-0">
        <label htmlFor={`limit-${key}`} className="text-sm font-medium">{label}</label>
        <p className="text-xs text-muted-foreground">
          {help} · default {defaultText}
        </p>
        {parsed.errors[key] && <p className="text-xs text-destructive">{parsed.errors[key]}</p>}
      </div>
      <div className="flex items-center gap-1.5">
        <Input
          id={`limit-${key}`}
          value={draft[key] ?? ""}
          onChange={(e) => setDraft((d) => ({ ...d, [key]: e.target.value }))}
          className={cn("h-8 text-right", key === "webhook_retry_delays_secs" ? "w-44" : "w-24", parsed.errors[key] && "border-destructive")}
          disabled={disabled || saving}
          inputMode={key === "webhook_retry_delays_secs" ? "text" : "decimal"}
        />
        <span className="text-xs text-muted-foreground w-14">{unit}</span>
      </div>
    </div>
  )

  return (
    <div className="space-y-4">
      {LIMIT_GROUPS.map((group) => (
        <div key={group.title}>
          <p className="text-xs font-medium text-muted-foreground uppercase tracking-wide mb-1">{group.title}</p>
          {group.rows.map((r) =>
            row(r.key, r.label, r.help, r.unit, `${display(defaults[r.key], r.scale)} ${r.unit}`),
          )}
          {group.title === "Webhooks" &&
            row(
              "webhook_retry_delays_secs",
              "Retry delays",
              `${parsed.out.webhook_retry_delays_secs.length + 1} attempts in total; then the subscription is suspended until refreshed`,
              "seconds",
              formatRetries(defaults.webhook_retry_delays_secs),
            )}
        </div>
      ))}
      <div className="flex items-center gap-2 pt-1">
        <Button size="sm" onClick={() => save(parsed.out)} disabled={disabled || saving || !dirty || hasErrors}>
          {saving && <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />}
          Save limits
        </Button>
        {dirty && (
          <Button size="sm" variant="ghost" onClick={() => setDraft(fromLimits)} disabled={saving}>
            Discard changes
          </Button>
        )}
        <Button
          size="sm"
          variant="ghost"
          className="ml-auto"
          onClick={() => save(defaults)}
          disabled={disabled || saving || isDefault}
        >
          <RotateCcw className="h-3.5 w-3.5 mr-1.5" />
          Reset to defaults
        </Button>
      </div>
    </div>
  )
}
