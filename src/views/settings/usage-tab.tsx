import { useCallback, useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Gauge, KeyRound, RefreshCw, Trash2 } from "lucide-react"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/Toggle"
import { Input } from "@/components/ui/Input"
import { Button } from "@/components/ui/Button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
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
import ProviderIcon from "@/components/ProviderIcon"
import { useTauriListener } from "@/hooks/useTauriListener"
import type {
  ForgetUsageAccountParams,
  UpdateUsageTrackingConfigParams,
  UsageAccountView,
  UsagePollStatus,
  UsageSnapshot,
  UsageTrackingConfig,
} from "@/types/tauri-commands"
import { formatAgo, formatUsd } from "@/views/dashboard/usage-format"

const ACTIVE_INTERVALS: { value: number; label: string }[] = [
  { value: 60, label: "1 min" },
  { value: 300, label: "5 min" },
  { value: 900, label: "15 min" },
  { value: 1800, label: "30 min" },
]
const IDLE_INTERVALS: { value: number; label: string }[] = [
  { value: 1800, label: "30 min" },
  { value: 3600, label: "1 hour" },
  { value: 10800, label: "3 hours" },
  { value: 21600, label: "6 hours" },
]

export function UsageTab() {
  const [config, setConfigState] = useState<UsageTrackingConfig | null>(null)
  const [snapshot, setSnapshot] = useState<UsageSnapshot | null>(null)
  const [statuses, setStatuses] = useState<UsagePollStatus[]>([])
  const [forgetting, setForgetting] = useState<UsageAccountView | null>(null)
  const [now, setNow] = useState(() => Date.now())

  const load = useCallback(async () => {
    try {
      const [cfg, snap, st] = await Promise.all([
        invoke<UsageTrackingConfig>("get_usage_tracking_config"),
        invoke<UsageSnapshot>("get_usage_limits"),
        invoke<UsagePollStatus[]>("get_usage_poll_status"),
      ])
      setConfigState(cfg)
      setSnapshot(snap)
      setStatuses(st)
      setNow(Date.now())
    } catch (e) {
      console.error("Failed to load usage settings:", e)
    }
  }, [])

  useEffect(() => {
    load()
  }, [load])

  useTauriListener("usage-limits-changed", () => {
    invoke<UsageSnapshot>("get_usage_limits").then(setSnapshot).catch(() => {})
    invoke<UsagePollStatus[]>("get_usage_poll_status").then(setStatuses).catch(() => {})
  })

  const save = async (next: UsageTrackingConfig) => {
    const previous = config
    setConfigState(next)
    try {
      await invoke("update_usage_tracking_config", {
        config: next,
      } satisfies UpdateUsageTrackingConfigParams)
      setSnapshot(await invoke<UsageSnapshot>("get_usage_limits"))
    } catch (e) {
      setConfigState(previous)
      toast.error(`Failed to save usage settings: ${e}`)
    }
  }

  if (!config) return null
  const update = (change: Partial<UsageTrackingConfig>) => save({ ...config, ...change })
  const accounts = snapshot?.accounts ?? []
  const refresh = async () => {
    try {
      await invoke("refresh_usage_limits")
      toast.success("Asking providers for usage…")
      window.setTimeout(load, 3000)
    } catch (e) {
      toast.error(`Failed to refresh: ${e}`)
    }
  }

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-2">
            <Gauge className="h-4 w-4" />
            Usage tracking
          </CardTitle>
          <CardDescription>
            Subscription windows and API rate limits, read from the traffic LocalRouter carries.
            Menu bar items: Appearance → Tray Stats.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <SwitchRow
            id="usage-enabled"
            label="Track usage limits"
            description="Windows, rate limits and request costs per account."
            checked={config.enabled}
            onChange={(enabled) => update({ enabled })}
          />
          <SwitchRow
            id="usage-poll-providers"
            label="Ask connected providers"
            description="ChatGPT Plus/Pro, GitHub Copilot and OpenRouter usage endpoints."
            checked={config.poll_provider_apis}
            disabled={!config.enabled}
            onChange={(poll_provider_apis) => update({ poll_provider_apis })}
          />
          <SwitchRow
            id="usage-cli-logins"
            label="Use Claude Code and Codex logins"
            description="Ask Anthropic and OpenAI for subscription usage with the CLIs' saved logins. Used only for that request; never stored."
            checked={config.read_cli_logins}
            disabled={!config.enabled}
            onChange={(read_cli_logins) => update({ read_cli_logins })}
            icon={<KeyRound className="h-3.5 w-3.5 text-muted-foreground" />}
          />
          <div className="flex flex-wrap items-center gap-3">
            <Label className="text-sm">Check every</Label>
            <IntervalSelect
              value={config.poll_interval_secs}
              options={ACTIVE_INTERVALS}
              disabled={!config.enabled}
              onChange={(poll_interval_secs) => update({ poll_interval_secs })}
            />
            <span className="text-sm text-muted-foreground">while in use,</span>
            <IntervalSelect
              value={config.idle_poll_interval_secs}
              options={IDLE_INTERVALS}
              disabled={!config.enabled}
              onChange={(idle_poll_interval_secs) => update({ idle_poll_interval_secs })}
            />
            <span className="text-sm text-muted-foreground">when idle</span>
            <Button variant="outline" size="sm" onClick={refresh} disabled={!config.enabled}>
              <RefreshCw className="mr-1.5 h-3.5 w-3.5" />
              Check now
            </Button>
          </div>
          {statuses.length > 0 && (
            <div className="rounded-md border divide-y text-xs">
              {statuses.map((s) => (
                <div key={s.id} className="flex flex-wrap items-center gap-2 px-3 py-2">
                  <span className="font-medium">{s.label}</span>
                  <span className="ml-auto text-muted-foreground">
                    {s.last_error
                      ? `Failed: ${s.last_error}`
                      : s.last_success
                        ? `OK, ${formatAgo(s.last_success, now)}`
                        : "Not checked yet"}
                  </span>
                </div>
              ))}
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">Accounts &amp; plans</CardTitle>
          <CardDescription>
            Override a detected plan or its monthly price (used for value estimates).
          </CardDescription>
        </CardHeader>
        <CardContent>
          {accounts.length === 0 ? (
            <p className="text-sm text-muted-foreground">No accounts seen yet.</p>
          ) : (
            <div className="divide-y rounded-md border">
              {accounts.map((a) => (
                <AccountRow
                  key={a.id}
                  account={a}
                  config={config}
                  onChange={save}
                  onForget={() => setForgetting(a)}
                />
              ))}
            </div>
          )}
        </CardContent>
      </Card>

      <AlertDialog open={forgetting !== null} onOpenChange={(open) => !open && setForgetting(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Forget {forgetting?.title}?</AlertDialogTitle>
            <AlertDialogDescription>
              Removes the recorded usage windows, history and spend for this account. It reappears
              when new usage is seen.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={async () => {
                if (!forgetting) return
                try {
                  await invoke("forget_usage_account", {
                    accountId: forgetting.id,
                  } satisfies ForgetUsageAccountParams)
                  await load()
                } catch (e) {
                  toast.error(`Failed to forget account: ${e}`)
                }
                setForgetting(null)
              }}
            >
              Forget
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function IntervalSelect({
  value,
  options,
  disabled,
  onChange,
}: {
  value: number
  options: { value: number; label: string }[]
  disabled?: boolean
  onChange: (value: number) => void
}) {
  const all = options.some((o) => o.value === value)
    ? options
    : [...options, { value, label: `${Math.round(value / 60)} min` }]
  return (
    <Select value={String(value)} disabled={disabled} onValueChange={(v) => onChange(Number(v))}>
      <SelectTrigger className="w-28">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {all.map((o) => (
          <SelectItem key={o.value} value={String(o.value)}>
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

function SwitchRow({
  id,
  label,
  description,
  checked,
  disabled,
  onChange,
  icon,
}: {
  id: string
  label: string
  description?: string
  checked: boolean
  disabled?: boolean
  onChange: (value: boolean) => void
  icon?: React.ReactNode
}) {
  return (
    <div className="flex items-start justify-between gap-4">
      <div className="space-y-0.5">
        <Label htmlFor={id} className="text-sm flex items-center gap-1.5">
          {icon}
          {label}
        </Label>
        {description && <p className="text-xs text-muted-foreground">{description}</p>}
      </div>
      <Switch id={id} checked={checked} disabled={disabled} onCheckedChange={onChange} />
    </div>
  )
}

function AccountRow({
  account: a,
  config,
  onChange,
  onForget,
}: {
  account: UsageAccountView
  config: UsageTrackingConfig
  onChange: (next: UsageTrackingConfig) => void
  onForget: () => void
}) {
  const override = config.plans[a.id]
  const setOverride = (plan: string | null, price: number | null) => {
    const plans = { ...config.plans }
    if (plan === null && price === null) delete plans[a.id]
    else plans[a.id] = { plan, monthly_price_usd: price }
    onChange({ ...config, plans })
  }
  const hidden = config.hidden_accounts.includes(a.id)
  return (
    <div className="flex flex-wrap items-center gap-3 px-3 py-2.5">
      <ProviderIcon providerId={a.provider} size={18} />
      <div className="min-w-40 flex-1">
        <div className="text-sm font-medium">{a.title}</div>
        <div className="text-xs text-muted-foreground">
          {a.id}
          {a.monthly_price_usd !== null && ` · ${formatUsd(a.monthly_price_usd)}/mo`}
        </div>
      </div>
      {a.kind === "subscription" && (
        <>
          <Input
            className="w-28"
            placeholder={a.plan_label ?? "Plan"}
            defaultValue={override?.plan ?? ""}
            aria-label="Plan name"
            onBlur={(e) => {
              const plan = e.target.value.trim() || null
              if (plan !== (override?.plan ?? null)) {
                setOverride(plan, override?.monthly_price_usd ?? null)
              }
            }}
          />
          <Input
            className="w-24"
            type="number"
            min={0}
            step={1}
            placeholder={a.monthly_price_usd !== null ? String(a.monthly_price_usd) : "$/mo"}
            defaultValue={override?.monthly_price_usd ?? ""}
            aria-label="Monthly price in USD"
            onBlur={(e) => {
              const raw = e.target.value.trim()
              const price = raw === "" ? null : Number(raw)
              if (price !== null && (!Number.isFinite(price) || price < 0)) return
              if (price !== (override?.monthly_price_usd ?? null)) {
                setOverride(override?.plan ?? null, price)
              }
            }}
          />
        </>
      )}
      <div className="flex items-center gap-1.5">
        <Label htmlFor={`hide-${a.id}`} className="text-xs text-muted-foreground">
          Show
        </Label>
        <Switch
          id={`hide-${a.id}`}
          checked={!hidden}
          onCheckedChange={(show) =>
            onChange({
              ...config,
              hidden_accounts: show
                ? config.hidden_accounts.filter((h) => h !== a.id)
                : [...config.hidden_accounts, a.id],
            })
          }
        />
      </div>
      <Button
        variant="ghost"
        size="sm"
        className="h-8 w-8 p-0"
        aria-label={`Forget ${a.title}`}
        onClick={onForget}
      >
        <Trash2 className="h-4 w-4" />
      </Button>
    </div>
  )
}
