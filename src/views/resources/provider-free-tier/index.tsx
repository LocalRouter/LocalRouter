import { useCallback, useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { AlertCircle, Coins, Loader2, Pencil, RotateCcw } from "lucide-react"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card"
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Input } from "@/components/ui/Input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/Select"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import type {
  FreeTierKind,
  ProviderFreeTierStatus,
  ResetProviderFreeTierUsageParams,
  SetProviderFreeTierParams,
  SetProviderFreeTierUsageParams,
} from "@/types/tauri-commands"
import { FreeModelRulesEditor } from "./FreeModelRulesEditor"
import { FREE_TIER_DESCRIPTIONS, FREE_TIER_LABELS, asFreeModelsOnly } from "./rules"

interface ProviderFreeTierTabProps {
  instanceName: string
  /** The provider type's free tier description (from its factory) */
  typeDescription?: string | null
  /** The provider type's default free tier kind; the description is shown
   * only while the provider uses it */
  typeDefaultKind?: string | null
  status: ProviderFreeTierStatus | undefined
  /** Reload the provider's free tier status */
  onReload: () => void
}

const KIND_OPTIONS: [FreeTierKind["kind"], string][] = [
  ["none", "No Free Tier"],
  ["always_free_local", "Always Free (Local)"],
  ["subscription", "Subscription"],
  ["rate_limited_free", "Rate Limited"],
  ["credit_based", "Credit Based"],
  ["free_models_only", "Free Models Only"],
]

function defaultKind(kind: FreeTierKind["kind"], current: FreeTierKind): FreeTierKind {
  if (kind === current.kind) return current
  switch (kind) {
    case "always_free_local":
      return { kind: "always_free_local" }
    case "subscription":
      return { kind: "subscription" }
    case "rate_limited_free":
      return { kind: "rate_limited_free", max_rpm: 30, max_rpd: 14400, max_tpm: 6000, max_tpd: 0, max_monthly_calls: 0, max_monthly_tokens: 0 }
    case "credit_based":
      return { kind: "credit_based", budget_usd: 5.0, reset_period: "monthly", detection: { type: "local_only" } }
    case "free_models_only":
      return { kind: "free_models_only", free_model_patterns: [], excluded_models: [], rules: [], max_rpm: 0 }
    default:
      return { kind: "none" }
  }
}

function sameKind(a: FreeTierKind, b: FreeTierKind): boolean {
  const norm = (k: FreeTierKind) =>
    k.kind === "free_models_only"
      ? { ...k, excluded_models: k.excluded_models ?? [], rules: k.rules ?? [] }
      : k
  // null and missing mean the same (e.g. a cleared note)
  const text = (k: FreeTierKind) => JSON.stringify(norm(k), (_key, v) => (v === null ? undefined : v))
  return text(a) === text(b)
}

/** A provider's Free Tier tab: its free tier, which models are free, usage. */
export function ProviderFreeTierTab({ instanceName, typeDescription, typeDefaultKind, status, onReload }: ProviderFreeTierTabProps) {
  // Unsaved changes to the provider's free tier (null: none)
  const [draft, setDraft] = useState<FreeTierKind | null>(null)
  const [editingType, setEditingType] = useState(false)
  const [saving, setSaving] = useState(false)
  const [usageDialogOpen, setUsageDialogOpen] = useState(false)
  // Rule problems in the draft (saving is refused until they are fixed)
  const [ruleErrors, setRuleErrors] = useState(0)

  useEffect(() => {
    setDraft(null)
    setEditingType(false)
  }, [instanceName])

  const saved = status?.free_tier ?? { kind: "none" as const }
  const current = draft ?? saved
  const dirty = draft != null && !sameKind(draft, saved)
  const freeModels = asFreeModelsOnly(current)
  const blocked = freeModels != null && ruleErrors > 0

  const save = useCallback(async (freeTier: FreeTierKind | null) => {
    setSaving(true)
    try {
      await invoke("set_provider_free_tier", {
        providerInstance: instanceName,
        freeTier,
      } satisfies SetProviderFreeTierParams)
      toast.success(freeTier ? "Free tier saved" : "Reset to provider default")
      setDraft(null)
      setEditingType(false)
      onReload()
    } catch (error) {
      toast.error(`Failed to save free tier: ${error}`)
    } finally {
      setSaving(false)
    }
  }, [instanceName, onReload])

  const resetUsage = async () => {
    try {
      await invoke("reset_provider_free_tier_usage", {
        providerInstance: instanceName,
      } satisfies ResetProviderFreeTierUsageParams)
      toast.success("Free tier usage reset")
      onReload()
    } catch (error) {
      console.error("Failed to reset free tier usage:", error)
      toast.error("Failed to reset free tier usage")
    }
  }

  if (!status) {
    return (
      <div className="flex items-center gap-2 text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        <span>Loading free tier status...</span>
      </div>
    )
  }

  const kind = current.kind

  return (
    <div className="space-y-6">
      {/* Configuration */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Coins className="h-4 w-4" />
            Free Tier Configuration
          </CardTitle>
          <CardDescription>Configure how this provider's free tier is tracked</CardDescription>
          {typeDescription && !status.is_user_override && typeDefaultKind === saved.kind && (
            <p className="text-sm text-muted-foreground mt-1 whitespace-pre-line">{typeDescription}</p>
          )}
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium">Type</span>
            <TooltipProvider delayDuration={300}>
              <Tooltip>
                <TooltipTrigger asChild>
                  <span>
                    <Badge
                      variant={kind === "always_free_local" || kind === "subscription" ? "default" : kind === "none" ? "secondary" : "outline"}
                      className="cursor-help"
                    >
                      {FREE_TIER_LABELS[kind] ?? "No Free Tier"}
                    </Badge>
                  </span>
                </TooltipTrigger>
                <TooltipContent side="bottom" className="max-w-xs text-xs">
                  {FREE_TIER_DESCRIPTIONS[kind]}
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
            {status.is_user_override && <Badge variant="outline" className="text-xs">Override</Badge>}
            {dirty && <Badge variant="warning" className="text-xs">Unsaved</Badge>}
            <Button variant="ghost" size="sm" onClick={() => setEditingType(!editingType)}>
              <Pencil className="h-3.5 w-3.5 mr-1" />
              {editingType ? "Close" : "Edit"}
            </Button>
          </div>

          {editingType && (
            <div className="space-y-3 p-3 rounded-md border bg-muted/30">
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-muted-foreground">Free Tier Type</label>
                <Select value={kind} onValueChange={(v) => setDraft(defaultKind(v as FreeTierKind["kind"], saved))}>
                  <SelectTrigger className="h-8 text-xs">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {KIND_OPTIONS.map(([value, label]) => (
                      <SelectItem key={value} value={value} className="text-xs py-2">
                        <div>
                          <div className="font-medium">{label}</div>
                          <div className="text-[11px] text-muted-foreground font-normal mt-0.5 whitespace-normal break-words">{FREE_TIER_DESCRIPTIONS[value]}</div>
                        </div>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              {current.kind === "rate_limited_free" && (
                <div className="space-y-3">
                  <div className="grid grid-cols-2 gap-2">
                    {([
                      ["max_rpm", "Requests per Minute (RPM)"],
                      ["max_rpd", "Requests per Day (RPD)"],
                      ["max_tpm", "Tokens per Minute (TPM)"],
                      ["max_tpd", "Tokens per Day (TPD)"],
                      ["max_monthly_calls", "Monthly Call Limit"],
                      ["max_monthly_tokens", "Monthly Token Limit"],
                    ] as const).map(([field, label]) => (
                      <div key={field} className="space-y-1">
                        <label className="text-xs text-muted-foreground">{label}</label>
                        <Input
                          type="number"
                          className="h-7 text-xs"
                          value={current[field]}
                          onChange={(e) => setDraft({ ...current, [field]: parseInt(e.target.value) || 0 })}
                        />
                      </div>
                    ))}
                  </div>
                  <p className="text-xs text-muted-foreground">Set to 0 to disable tracking for that limit. The router also reads rate limit headers from provider responses when available, which take precedence over these configured limits.</p>
                </div>
              )}

              {current.kind === "credit_based" && (
                <div className="space-y-2">
                  <div className="grid grid-cols-2 gap-2">
                    <div className="space-y-1">
                      <label className="text-xs text-muted-foreground">Credit Budget (USD)</label>
                      <Input
                        type="number"
                        step="0.01"
                        className="h-7 text-xs"
                        value={current.budget_usd}
                        onChange={(e) => setDraft({ ...current, budget_usd: parseFloat(e.target.value) || 0 })}
                      />
                    </div>
                    <div className="space-y-1">
                      <label className="text-xs text-muted-foreground">Reset Period</label>
                      <Select
                        value={current.reset_period}
                        onValueChange={(value: "daily" | "monthly" | "never") => setDraft({ ...current, reset_period: value })}
                      >
                        <SelectTrigger className="h-7 text-xs">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="daily" className="text-xs">Daily</SelectItem>
                          <SelectItem value="monthly" className="text-xs">Monthly</SelectItem>
                          <SelectItem value="never" className="text-xs">One-time (no reset)</SelectItem>
                        </SelectContent>
                      </Select>
                    </div>
                  </div>
                  <p className="text-xs text-muted-foreground">The router estimates cost from token usage and model pricing. When estimated spend reaches the budget, the provider is skipped. Use "Set Usage" below to sync with your actual balance from the provider dashboard.</p>
                </div>
              )}

              {current.kind === "free_models_only" && (
                <div className="space-y-1">
                  <label className="text-xs text-muted-foreground">Requests per Minute (RPM) on free models</label>
                  <Input
                    type="number"
                    className="h-7 text-xs w-40"
                    value={current.max_rpm}
                    onChange={(e) => setDraft({ ...current, max_rpm: parseInt(e.target.value) || 0 })}
                  />
                  <p className="text-xs text-muted-foreground">Only requests to free models count. Set to 0 to disable. Choose the free models below.</p>
                </div>
              )}
            </div>
          )}

          {(dirty || editingType || status.is_user_override) && (
            <div className="flex flex-wrap items-center gap-2">
              {(dirty || editingType) && (
                <Button size="sm" disabled={!dirty || saving || blocked} onClick={() => save(current)}>
                  {saving && <Loader2 className="h-3.5 w-3.5 mr-1 animate-spin" />}
                  Save
                </Button>
              )}
              {dirty && (
                <Button variant="outline" size="sm" onClick={() => setDraft(null)}>
                  Discard changes
                </Button>
              )}
              {status.is_user_override && (
                <Button variant="outline" size="sm" disabled={saving} onClick={() => save(null)}>
                  <RotateCcw className="h-3.5 w-3.5 mr-1" />
                  Reset to Default
                </Button>
              )}
            </div>
          )}
        </CardContent>
      </Card>

      {/* Which models are free */}
      {freeModels && (
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">Free models</CardTitle>
            <CardDescription>
              Which of this provider's models free-tier-only strategies may use. Changes apply when you save.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <FreeModelRulesEditor
              instanceName={instanceName}
              value={freeModels}
              onChange={setDraft}
              dirty={dirty}
              onErrorCount={setRuleErrors}
            />
            {dirty && (
              <div className="sticky bottom-0 flex flex-wrap items-center gap-2 border-t bg-card pt-3">
                <Button size="sm" disabled={saving || blocked} onClick={() => save(current)}>
                  {saving && <Loader2 className="h-3.5 w-3.5 mr-1 animate-spin" />}
                  Save
                </Button>
                <Button variant="outline" size="sm" onClick={() => setDraft(null)}>
                  Discard changes
                </Button>
                {blocked && (
                  <span className="text-xs text-red-600 dark:text-red-400">
                    Fix the {ruleErrors === 1 ? "rule error" : `${ruleErrors} rule errors`} above to save.
                  </span>
                )}
              </div>
            )}
          </CardContent>
        </Card>
      )}

      {/* Usage & Status */}
      <UsageCard status={status} onReset={resetUsage} onSetUsage={() => setUsageDialogOpen(true)} />

      <SetUsageDialog
        open={usageDialogOpen}
        onOpenChange={setUsageDialogOpen}
        instanceName={instanceName}
        status={status}
        onSaved={onReload}
      />
    </div>
  )
}

function UsageBar({ label, used, limit, format = (n) => String(n) }: { label: string; used: number; limit: number; format?: (n: number) => string }) {
  const ratio = limit > 0 ? used / limit : 0
  return (
    <div>
      <div className="flex justify-between text-xs mb-1">
        <span className="text-muted-foreground">{label}</span>
        <span>{format(used)} / {format(limit)}</span>
      </div>
      <div className="h-1.5 rounded-full bg-muted overflow-hidden">
        <div
          className={`h-full rounded-full ${ratio > 0.9 ? "bg-red-500" : ratio > 0.7 ? "bg-amber-500" : "bg-green-500"}`}
          style={{ width: `${Math.min(100, ratio * 100)}%` }}
        />
      </div>
    </div>
  )
}

function UsageCard({ status, onReset, onSetUsage }: { status: ProviderFreeTierStatus; onReset: () => void; onSetUsage: () => void }) {
  const kind = status.free_tier?.kind
  const tokens = (n: number) => n.toLocaleString()
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm">Usage & Status</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <span className="text-sm font-medium">Free Tier</span>
            <span className={`text-sm ${status.has_capacity ? "text-green-600" : "text-red-600"}`}>
              {status.status_message || (status.has_capacity ? "Available" : "Exhausted")}
            </span>
          </div>

          {kind === "rate_limited_free" && (
            <div className="space-y-3 pt-2 border-t">
              <span className="text-sm font-medium">Usage</span>
              {status.rate_rpm_limit != null && <UsageBar label="Requests/min" used={status.rate_rpm_used ?? 0} limit={status.rate_rpm_limit} />}
              {status.rate_rpd_limit != null && <UsageBar label="Requests/day" used={status.rate_rpd_used ?? 0} limit={status.rate_rpd_limit} />}
              {status.rate_tpm_limit != null && <UsageBar label="Tokens/min" used={status.rate_tpm_used ?? 0} limit={status.rate_tpm_limit} format={tokens} />}
              {status.rate_tpd_limit != null && <UsageBar label="Tokens/day" used={status.rate_tpd_used ?? 0} limit={status.rate_tpd_limit} format={tokens} />}
              {status.rate_monthly_calls_limit != null && <UsageBar label="Monthly calls" used={status.rate_monthly_calls_used ?? 0} limit={status.rate_monthly_calls_limit} />}
              {status.rate_monthly_tokens_limit != null && <UsageBar label="Monthly tokens" used={status.rate_monthly_tokens_used ?? 0} limit={status.rate_monthly_tokens_limit} format={tokens} />}
            </div>
          )}

          {kind === "credit_based" && (
            <div className="space-y-3 pt-2 border-t">
              <span className="text-sm font-medium">Credits</span>
              <div>
                <div className="flex justify-between text-xs mb-1">
                  <span className="text-muted-foreground">Used</span>
                  <span>${(status.credit_used_usd ?? 0).toFixed(4)} / ${(status.credit_budget_usd ?? 0).toFixed(2)}</span>
                </div>
                <div className="h-1.5 rounded-full bg-muted overflow-hidden">
                  <div
                    className={`h-full rounded-full ${
                      status.credit_remaining_usd != null && status.credit_budget_usd != null && status.credit_budget_usd > 0
                        ? (status.credit_remaining_usd / status.credit_budget_usd < 0.1 ? "bg-red-500" :
                           status.credit_remaining_usd / status.credit_budget_usd < 0.3 ? "bg-amber-500" :
                           "bg-green-500")
                        : "bg-muted"
                    }`}
                    style={{
                      width: `${status.credit_budget_usd && status.credit_budget_usd > 0
                        ? Math.min(100, ((status.credit_used_usd ?? 0) / status.credit_budget_usd) * 100)
                        : 0}%`,
                    }}
                  />
                </div>
              </div>
              {status.credit_remaining_usd != null && (
                <p className="text-xs text-muted-foreground">${status.credit_remaining_usd.toFixed(4)} remaining</p>
              )}
            </div>
          )}

          {kind === "free_models_only" && status.rate_rpm_limit != null && (
            <div className="space-y-3 pt-2 border-t">
              <UsageBar label="Requests/min on free models" used={status.rate_rpm_used ?? 0} limit={status.rate_rpm_limit} />
            </div>
          )}

          {status.is_backed_off && (
            <div className="flex items-center gap-2 pt-2 border-t text-amber-600">
              <AlertCircle className="h-4 w-4" />
              <span className="text-sm">
                {status.backoff_reason ?? "Rate limited"}
                {status.backoff_retry_after_secs != null && ` (available in ${status.backoff_retry_after_secs}s)`}
              </span>
            </div>
          )}

          {(kind === "rate_limited_free" || kind === "credit_based" || kind === "free_models_only") && (
            <div className="flex gap-2 pt-2 border-t">
              <Button variant="outline" size="sm" onClick={onReset}>Reset Usage</Button>
              {kind !== "free_models_only" && (
                <Button variant="outline" size="sm" onClick={onSetUsage}>Set Usage</Button>
              )}
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  )
}

function SetUsageDialog({
  open,
  onOpenChange,
  instanceName,
  status,
  onSaved,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  instanceName: string
  status: ProviderFreeTierStatus
  onSaved: () => void
}) {
  const kind = status.free_tier?.kind
  const [values, setValues] = useState({ creditUsedUsd: "", creditRemainingUsd: "", dailyRequests: "", monthlyRequests: "", monthlyTokens: "" })

  useEffect(() => {
    if (!open) return
    setValues({
      creditUsedUsd: status.credit_used_usd != null ? String(status.credit_used_usd) : "",
      creditRemainingUsd: status.credit_remaining_usd != null ? String(status.credit_remaining_usd) : "",
      dailyRequests: status.rate_rpd_used != null ? String(status.rate_rpd_used) : "",
      monthlyRequests: status.rate_monthly_calls_used != null ? String(status.rate_monthly_calls_used) : "",
      monthlyTokens: "",
    })
  }, [open, status])

  const save = async () => {
    const num = (v: string, parse: (s: string) => number) => (v ? parse(v) : null)
    const credit = kind === "credit_based"
    try {
      await invoke("set_provider_free_tier_usage", {
        providerInstance: instanceName,
        creditUsedUsd: credit ? num(values.creditUsedUsd, parseFloat) : null,
        creditRemainingUsd: credit ? num(values.creditRemainingUsd, parseFloat) : null,
        dailyRequests: credit ? null : num(values.dailyRequests, (s) => parseInt(s)),
        monthlyRequests: credit ? null : num(values.monthlyRequests, (s) => parseInt(s)),
        monthlyTokens: credit ? null : num(values.monthlyTokens, (s) => parseInt(s)),
      } satisfies SetProviderFreeTierUsageParams)
      toast.success("Usage updated")
      onOpenChange(false)
      onSaved()
    } catch (error) {
      console.error("Failed to set usage:", error)
      toast.error("Failed to set usage")
    }
  }

  const field = (key: keyof typeof values, label: string, help: string, placeholder: string, step?: string) => (
    <div className="space-y-1.5">
      <label className="text-sm font-medium">{label}</label>
      <Input
        type="number"
        step={step}
        placeholder={placeholder}
        value={values[key]}
        onChange={(e) => setValues((prev) => ({ ...prev, [key]: e.target.value }))}
      />
      <p className="text-xs text-muted-foreground">{help}</p>
    </div>
  )

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Set Usage</DialogTitle>
        </DialogHeader>
        <p className="text-sm text-muted-foreground">
          Manually adjust the tracked usage counters for this provider. This is useful when the router's local tracking has drifted from your actual usage, or to sync with the balance shown on your provider dashboard. Leave fields empty to keep them unchanged.
        </p>
        <div className="space-y-4">
          {kind === "credit_based" && (
            <>
              {field("creditUsedUsd", "Credits Used (USD)", "How much of the credit budget has been consumed so far in the current period. The router uses this to calculate remaining capacity.", "0.00", "0.01")}
              {field("creditRemainingUsd", "Credits Remaining (USD)", "The actual remaining balance from your provider dashboard. This overrides the router's calculated estimate and is the most accurate way to sync usage.", "e.g. 4.50", "0.01")}
            </>
          )}
          {kind === "rate_limited_free" && (
            <>
              {field("dailyRequests", "Daily Requests Used", "Number of requests made today. Resets automatically at the start of each day (midnight UTC, or midnight PT for Gemini).", "0")}
              {field("monthlyRequests", "Monthly Requests Used", "Number of requests made this month. Only relevant for providers with monthly call caps (e.g. Cohere: 1,000/month). Resets on the 1st of each month.", "0")}
              {field("monthlyTokens", "Monthly Tokens Used", "Total tokens used this month. Only relevant for providers with monthly token caps (e.g. Mistral: 1B tokens/month). Resets on the 1st of each month.", "0")}
            </>
          )}
          <div className="flex justify-end gap-2 pt-2">
            <Button variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
            <Button onClick={save}>Save</Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
