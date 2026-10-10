import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { toast } from 'sonner'
import {
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  Gauge,
  KeyRound,
  OctagonAlert,
  RefreshCw,
  Settings2,
} from 'lucide-react'
import ProviderIcon from '@/components/ProviderIcon'
import { Button } from '@/components/ui/Button'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import type {
  UpdateUsageTrackingConfigParams,
  UsageAccountView,
  UsagePaceStatus,
  UsageQuotaView,
  UsageTrackingConfig,
  UsageWindowView,
} from '@/types/tauri-commands'
import {
  formatAgo,
  formatClock,
  formatCount,
  formatDuration,
  formatPercent,
  formatUsd,
  PACE_LABEL,
  paceSummary,
  slotHeights,
  SOURCE_LABEL,
  visibleAccounts,
} from './usage-format'
import { useUsageLimits } from './use-usage-limits'

const COLLAPSED_KEY = 'localrouter.dashboard.usage-limits.collapsed'

/** Status colors (reserved for pace; paired with an icon and tooltip label). */
const PACE_FILL: Record<UsagePaceStatus, string> = {
  ok: 'bg-emerald-500',
  warning: 'bg-amber-500',
  over: 'bg-red-500',
}
const PACE_TEXT: Record<UsagePaceStatus, string> = {
  ok: 'text-emerald-600 dark:text-emerald-400',
  warning: 'text-amber-600 dark:text-amber-400',
  over: 'text-red-600 dark:text-red-400',
}

/** Short window names for the one-line rows. */
function shortLabel(w: UsageWindowView): string {
  if (w.id === 'five_hour') return '5h'
  if (w.id === 'seven_day') return 'Week'
  if (w.id === 'monthly') return 'Month'
  if (w.id.startsWith('seven_day_')) return `Week · ${w.label.split('·').pop()?.trim() ?? ''}`
  return w.label
}

interface UsageLimitsProps {
  className?: string
  onOpenSettings?: () => void
}

/** Dashboard strip: one compact card per subscription / API account. */
export function UsageLimits({ className, onOpenSettings }: UsageLimitsProps) {
  const { snapshot, now, refresh, reload } = useUsageLimits()
  const [collapsed, setCollapsed] = useState(
    () => localStorage.getItem(COLLAPSED_KEY) === '1',
  )
  const [refreshing, setRefreshing] = useState(false)

  const toggle = () => {
    setCollapsed((c) => {
      localStorage.setItem(COLLAPSED_KEY, c ? '0' : '1')
      return !c
    })
  }

  const accounts = snapshot ? visibleAccounts(snapshot.accounts) : []
  const subscriptions = accounts.filter((a) => a.kind === 'subscription')
  const others = accounts.filter((a) => a.kind !== 'subscription')

  const onRefresh = async () => {
    setRefreshing(true)
    try {
      await refresh()
    } finally {
      window.setTimeout(() => setRefreshing(false), 600)
    }
  }

  return (
    <TooltipProvider delayDuration={150}>
      <section
        className={cn('overflow-hidden rounded-xl border bg-card', className)}
        aria-label="Usage limits"
      >
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 px-4 py-2">
          <button
            type="button"
            onClick={toggle}
            className="flex items-center gap-1.5 text-sm font-semibold"
            aria-expanded={!collapsed}
          >
            {collapsed ? (
              <ChevronRight className="h-4 w-4 text-muted-foreground" />
            ) : (
              <ChevronDown className="h-4 w-4 text-muted-foreground" />
            )}
            <Gauge className="h-4 w-4 text-muted-foreground" />
            Usage limits
          </button>
          {collapsed && <CollapsedSummary accounts={subscriptions} />}
          {!collapsed && snapshot?.enabled && subscriptions.length === 0 && (
            <CliLoginsButton onEnabled={reload} />
          )}
          <div className="ml-auto flex items-center gap-1">
            {onOpenSettings && (
              <Button
                variant="ghost"
                size="sm"
                className="h-7 w-7 p-0"
                onClick={onOpenSettings}
                aria-label="Usage settings"
              >
                <Settings2 className="h-3.5 w-3.5" />
              </Button>
            )}
            <Button
              variant="ghost"
              size="sm"
              className="h-7 w-7 p-0"
              onClick={onRefresh}
              disabled={!snapshot?.enabled}
              aria-label="Refresh usage"
            >
              <RefreshCw className={cn('h-3.5 w-3.5', refreshing && 'animate-spin')} />
            </Button>
          </div>
        </div>

        {!collapsed && snapshot !== null && (
          <div className="px-4 pb-3">
            {!snapshot.enabled ? (
              <DisabledState onEnabled={reload} />
            ) : accounts.length === 0 ? (
              <p className="text-xs text-muted-foreground">No usage seen yet.</p>
            ) : (
              <div className="grid gap-2 [grid-template-columns:repeat(auto-fill,minmax(270px,1fr))]">
                {[...subscriptions, ...others].map((a) => (
                  <AccountCard key={a.id} account={a} now={now} />
                ))}
              </div>
            )}
          </div>
        )}
      </section>
    </TooltipProvider>
  )
}

function CollapsedSummary({ accounts }: { accounts: UsageAccountView[] }) {
  const items = accounts
    .map((a) => ({ a, w: a.windows.find((w) => w.id === 'seven_day') }))
    .filter((x): x is { a: UsageAccountView; w: UsageWindowView } => !!x.w)
  if (items.length === 0) return null
  return (
    <div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
      {items.map(({ a, w }) => (
        <span key={a.id} className="flex items-center gap-1.5">
          <ProviderIcon providerId={a.provider} size={14} />
          <strong className="font-medium tabular-nums text-foreground">
            {w.stale ? '—' : formatPercent(w.used_percent)}
          </strong>
        </span>
      ))}
    </div>
  )
}

async function setConfig(change: Partial<UsageTrackingConfig>) {
  const current = await invoke<UsageTrackingConfig>('get_usage_tracking_config')
  await invoke('update_usage_tracking_config', {
    config: { ...current, ...change },
  } satisfies UpdateUsageTrackingConfigParams)
}

function DisabledState({ onEnabled }: { onEnabled: () => void }) {
  return (
    <div className="flex items-center gap-2 text-xs text-muted-foreground">
      Usage tracking is off.
      <Button
        size="sm"
        variant="outline"
        className="h-6 px-2 text-xs"
        onClick={async () => {
          try {
            await setConfig({ enabled: true })
            onEnabled()
          } catch (e) {
            toast.error(`Failed to enable usage tracking: ${e}`)
          }
        }}
      >
        Turn on
      </Button>
    </div>
  )
}

/** Offered while no subscription is known and CLI logins are not in use. */
function CliLoginsButton({ onEnabled }: { onEnabled: () => void }) {
  const [cliLogins, setCliLogins] = useState<boolean | null>(null)
  useEffect(() => {
    invoke<UsageTrackingConfig>('get_usage_tracking_config')
      .then((c) => setCliLogins(c.read_cli_logins))
      .catch(() => setCliLogins(null))
  }, [])
  if (cliLogins !== false) return null
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          size="sm"
          variant="outline"
          className="h-6 px-2 text-xs"
          onClick={async () => {
            try {
              await setConfig({ read_cli_logins: true })
              setCliLogins(true)
              onEnabled()
            } catch (e) {
              toast.error(`Failed to update settings: ${e}`)
            }
          }}
        >
          <KeyRound className="mr-1 h-3 w-3" />
          Use Claude Code &amp; Codex logins
        </Button>
      </TooltipTrigger>
      <TooltipContent className="max-w-xs text-xs">
        Ask Anthropic and OpenAI for your subscription usage with the CLIs&apos; saved logins on
        this computer.
      </TooltipContent>
    </Tooltip>
  )
}

function StatusBadge({ status }: { status: string | null }) {
  if (status === 'rejected') {
    return (
      <span className="flex items-center gap-1 rounded-full bg-red-500/10 px-1.5 text-[11px] font-medium text-red-700 dark:text-red-300">
        <OctagonAlert className="h-3 w-3" /> Limited
      </span>
    )
  }
  if (status === 'allowed_warning') {
    return (
      <span className="flex items-center gap-1 rounded-full bg-amber-500/10 px-1.5 text-[11px] font-medium text-amber-700 dark:text-amber-300">
        <AlertTriangle className="h-3 w-3" /> Near limit
      </span>
    )
  }
  return null
}

function AccountCard({ account: a, now }: { account: UsageAccountView; now: number }) {
  const updated = Math.max(a.last_seen, ...a.windows.map((w) => w.updated_at))
  const footer = valueLine(a)
  return (
    <article className="flex flex-col gap-1.5 rounded-lg border bg-background/40 px-3 py-2">
      <header className="flex items-center gap-1.5">
        <ProviderIcon providerId={a.provider} size={16} className="shrink-0" />
        <Tooltip>
          <TooltipTrigger asChild>
            <h3 className="truncate text-sm font-semibold">{a.title}</h3>
          </TooltipTrigger>
          <TooltipContent className="max-w-xs text-xs">
            {a.kind === 'subscription' ? 'Subscription' : 'API usage'}
            {a.plan_overridden && ' · plan set by you'} · via{' '}
            {a.sources.map((s) => SOURCE_LABEL[s]).join(', ') || 'LocalRouter'} · updated{' '}
            {formatAgo(updated, now)}
          </TooltipContent>
        </Tooltip>
        {a.monthly_price_usd !== null && (
          <span className="shrink-0 text-[11px] text-muted-foreground">
            {formatUsd(a.monthly_price_usd)}/mo
          </span>
        )}
        <span className="ml-auto">
          <StatusBadge status={a.status} />
        </span>
      </header>

      {a.windows.map((w) => (
        <WindowRow key={w.id} window={w} now={now} />
      ))}
      {a.quotas.slice(0, 3).map((q) => (
        <QuotaRow key={q.id} quota={q} now={now} />
      ))}
      {a.credits && <CreditsRow credits={a.credits} />}

      {footer && (
        <Tooltip>
          <TooltipTrigger asChild>
            <p className="truncate text-[11px] text-muted-foreground">{footer.text}</p>
          </TooltipTrigger>
          <TooltipContent className="max-w-xs text-xs">{footer.hint}</TooltipContent>
        </Tooltip>
      )}
    </article>
  )
}

const ROW = 'grid grid-cols-[4.5rem_1fr_2.75rem_3.5rem] items-center gap-2 text-xs'

/** One line: label · meter (even-pace tick, projection shade) · % · reset. */
function WindowRow({ window: w, now }: { window: UsageWindowView; now: number }) {
  const used = Math.min(100, Math.max(0, w.used_percent))
  const projected =
    w.projected_percent !== null ? Math.min(100, Math.max(used, w.projected_percent)) : null
  const resetsIn = w.resets_at !== null ? w.resets_at - now / 1000 : null
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div className={ROW}>
          <span className="truncate text-muted-foreground">{shortLabel(w)}</span>
          <div
            className="relative h-1.5 overflow-hidden rounded-full bg-muted"
            role="meter"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={w.stale ? undefined : Math.round(w.used_percent)}
            aria-label={`${w.label} usage, ${PACE_LABEL[w.pace]}`}
          >
            {!w.stale && projected !== null && projected > used && (
              <div
                className={cn('absolute inset-y-0 left-0 rounded-full opacity-25', PACE_FILL[w.pace])}
                style={{ width: `${projected}%` }}
              />
            )}
            {!w.stale && (
              <div
                className={cn('absolute inset-y-0 left-0 rounded-full', PACE_FILL[w.pace])}
                style={{ width: `${used}%` }}
              />
            )}
            {w.elapsed_fraction !== null && !w.stale && (
              <div
                className="absolute inset-y-0 w-0.5 bg-foreground/60"
                style={{ left: `calc(${Math.min(100, w.elapsed_fraction * 100)}% - 1px)` }}
              />
            )}
          </div>
          <span
            className={cn(
              'text-right font-semibold tabular-nums',
              !w.stale && w.pace !== 'ok' && PACE_TEXT[w.pace],
            )}
          >
            {w.stale ? '—' : formatPercent(w.used_percent)}
          </span>
          <span className="text-right text-[11px] text-muted-foreground tabular-nums">
            {resetsIn !== null && !w.stale ? formatDuration(resetsIn) : ''}
          </span>
        </div>
      </TooltipTrigger>
      <TooltipContent className="max-w-xs space-y-1 text-xs">
        <p className="font-medium">
          {w.label}: {w.used_percent.toFixed(1)}% · {PACE_LABEL[w.pace]}
        </p>
        <p>{paceSummary(w, now)}</p>
        {w.elapsed_fraction !== null && !w.stale && (
          <p className="text-muted-foreground">
            Tick = even pace ({(w.elapsed_fraction * 100).toFixed(0)}% of the window elapsed);
            shade = projection.
          </p>
        )}
        {w.resets_at !== null && <p>Resets {formatClock(w.resets_at)}</p>}
        {w.slots.length > 1 && <SlotBars window={w} />}
        {w.api_equivalent_usd !== null && w.api_equivalent_usd > 0 && (
          <p>API value of this window&apos;s traffic: {formatUsd(w.api_equivalent_usd)}</p>
        )}
        {w.history.length > 0 && (
          <p>
            Previous window peaked at{' '}
            {formatPercent(w.history[w.history.length - 1].peak_percent)}
          </p>
        )}
        <p className="text-muted-foreground">
          {SOURCE_LABEL[w.source]}, {formatAgo(w.updated_at, now)}
        </p>
      </TooltipContent>
    </Tooltip>
  )
}

/** One bar per hour (5-hour windows) or day (weekly), like the status line. */
function SlotBars({ window: w }: { window: UsageWindowView }) {
  const heights = slotHeights(w)
  const start = w.resets_at !== null && w.window_secs !== null ? w.resets_at - w.window_secs : null
  const daily = (w.slot_secs ?? 0) >= 86_400
  return (
    <div className="flex items-end gap-1 pt-1">
      {heights.map((h, i) => {
        const slotStart = start !== null && w.slot_secs !== null ? start + i * w.slot_secs : null
        const label =
          slotStart !== null
            ? new Date(slotStart * 1000).toLocaleString(
                undefined,
                daily ? { weekday: 'narrow' } : { hour: 'numeric' },
              )
            : String(i + 1)
        return (
          <div key={i} className="flex w-5 flex-col items-center gap-0.5">
            <div className="flex h-6 w-full items-end">
              {h === null ? (
                <div className="h-px w-full bg-muted-foreground/30" />
              ) : (
                <div
                  className={cn(
                    'w-full rounded-t-[2px]',
                    i === w.current_slot ? 'bg-foreground/70' : 'bg-foreground/40',
                  )}
                  style={{ height: `${Math.max(8, h * 100)}%` }}
                />
              )}
            </div>
            <span className="text-[9px] text-muted-foreground">{label}</span>
          </div>
        )
      })}
    </div>
  )
}

function QuotaRow({ quota: q, now }: { quota: UsageQuotaView; now: number }) {
  const used = q.used_percent
  return (
    <div className={ROW}>
      <span className="truncate text-muted-foreground">{q.label}</span>
      <div className="h-1.5 overflow-hidden rounded-full bg-muted">
        {used !== null && (
          <div
            className={cn('h-full rounded-full', used >= 90 ? 'bg-amber-500' : 'bg-foreground/40')}
            style={{ width: `${Math.min(100, used)}%` }}
          />
        )}
      </div>
      <span className="text-right tabular-nums">{formatCount(q.remaining)}</span>
      <span className="text-right text-[11px] text-muted-foreground tabular-nums">
        {q.resets_at !== null && q.resets_at > now / 1000
          ? formatDuration(q.resets_at - now / 1000)
          : ''}
      </span>
    </div>
  )
}

function CreditsRow({ credits: c }: { credits: NonNullable<UsageAccountView['credits']> }) {
  const amount = (v: number | null) =>
    c.currency === 'USD' || c.currency === null ? formatUsd(v) : `${v ?? '—'} ${c.currency}`
  let text: string
  if (c.limit_usd !== null && c.used_usd !== null) {
    text = `${amount(c.used_usd)} / ${amount(c.limit_usd)}`
  } else if (c.balance_usd !== null) {
    text = c.currency === null ? `${c.balance_usd} left` : `${amount(c.balance_usd)} left`
  } else if (c.used_usd !== null) {
    text = `${amount(c.used_usd)} used`
  } else {
    text = c.unlimited ? 'Unlimited' : '—'
  }
  return (
    <div className="flex items-baseline gap-2 text-xs">
      <span className="text-muted-foreground">{c.label}</span>
      <span className="ml-auto tabular-nums">{text}</span>
    </div>
  )
}

/** Money line: API-equivalent value vs. the plan's price, or API spend. */
function valueLine(a: UsageAccountView): { text: string; hint: string } | null {
  if (a.kind === 'subscription') {
    if (a.spend.last_30d_usd <= 0) return null
    const multiple = a.value_multiplier !== null ? ` · ${a.value_multiplier.toFixed(1)}× plan` : ''
    return {
      text: `API value 30d ${formatUsd(a.spend.last_30d_usd)}${multiple}`,
      hint: 'What the traffic LocalRouter saw would have cost at API list prices.',
    }
  }
  if (a.spend.requests_30d <= 0) return null
  return {
    text: `${formatUsd(a.spend.last_24h_usd)} today · ${formatUsd(a.spend.month_to_date_usd)} this month`,
    hint: `${formatCount(a.spend.requests_30d)} requests, ${formatCount(a.spend.tokens_30d)} tokens in 30 days, at list prices.`,
  }
}
