import type {
  UsageAccountView,
  UsageDataSource,
  UsagePaceStatus,
  UsageWindowView,
} from '@/types/tauri-commands'

const money2 = new Intl.NumberFormat('en-US', {
  style: 'currency',
  currency: 'USD',
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
})
const money0 = new Intl.NumberFormat('en-US', {
  style: 'currency',
  currency: 'USD',
  maximumFractionDigits: 0,
})
const compact = new Intl.NumberFormat('en-US', {
  notation: 'compact',
  maximumFractionDigits: 1,
})

/** `$0.42`, `$12.30`, `$1,204` (whole dollars from $1,000). */
export function formatUsd(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return '—'
  return Math.abs(value) >= 1000 ? money0.format(value) : money2.format(value)
}

/** Rounded down so a window with room left never reads as 100%. */
export function formatPercent(value: number): string {
  return `${Math.floor(Math.max(0, value) + 1e-6)}%`
}

export function formatCount(value: number | null | undefined): string {
  if (value === null || value === undefined) return '—'
  return value >= 10_000 ? compact.format(value) : Math.round(value).toLocaleString('en-US')
}

/** `2d 4h`, `3h 12m`, `45m`, `<1m` until (or since) a unix time. */
export function formatDuration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds))
  if (s < 60) return '<1m'
  const d = Math.floor(s / 86_400)
  const h = Math.floor((s % 86_400) / 3_600)
  const m = Math.floor((s % 3_600) / 60)
  if (d > 0) return h > 0 ? `${d}d ${h}h` : `${d}d`
  if (h > 0) return m > 0 ? `${h}h ${m}m` : `${h}h`
  return `${m}m`
}

/** `2m ago` / `just now`. */
export function formatAgo(unixSecs: number, nowMs: number): string {
  const s = nowMs / 1000 - unixSecs
  if (s < 45) return 'just now'
  return `${formatDuration(s)} ago`
}

/** Local weekday + time, e.g. `Tue 09:00`. */
export function formatClock(unixSecs: number): string {
  return new Date(unixSecs * 1000).toLocaleString(undefined, {
    weekday: 'short',
    hour: '2-digit',
    minute: '2-digit',
  })
}

/** One-line forecast for a window. */
export function paceSummary(window: UsageWindowView, nowMs: number): string {
  if (window.stale) return 'Window reset — waiting for new data'
  if (window.used_percent >= 100) return 'Limit reached'
  if (window.limit_eta !== null) {
    return `Limit in ~${formatDuration(window.limit_eta - nowMs / 1000)} at this pace`
  }
  if (window.projected_percent !== null) {
    return `On pace for ${formatPercent(window.projected_percent)} at reset`
  }
  return 'Too early to project'
}

export const PACE_LABEL: Record<UsagePaceStatus, string> = {
  ok: 'On track',
  warning: 'Near limit',
  over: 'Over pace',
}

export const SOURCE_LABEL: Record<UsageDataSource, string> = {
  proxy_headers: 'HTTPS proxy',
  proxy_usage_response: 'HTTPS proxy (/usage)',
  gateway_headers: 'LocalRouter providers',
  provider_api: 'Provider usage API',
  cli_login: 'CLI login',
}

/**
 * Bar heights (0–1) for a window's slots: linear from 0 to twice the
 * even-pace share per slot (a full bar = 2× pace), like the Claude Code
 * status line. Future slots are `null`.
 */
export function slotHeights(window: UsageWindowView): (number | null)[] {
  const n = window.slots.length
  if (n === 0) return []
  const share = 100 / n
  return window.slots.map((points, i) => {
    if (i > window.current_slot) return null
    return Math.min(1, Math.max(0, points / (2 * share)))
  })
}

/**
 * Accounts worth a card: not hidden, and with limits or traffic to show. An
 * API account known only from rate-limit headers (e.g. a provider health
 * check) without any requests is left to Settings.
 */
export function visibleAccounts(accounts: UsageAccountView[]): UsageAccountView[] {
  return accounts.filter(
    (a) =>
      !a.hidden &&
      (a.windows.length > 0 ||
        a.credits !== null ||
        a.spend.requests_30d > 0 ||
        (a.kind === 'subscription' && a.quotas.length > 0)),
  )
}

/** The headline window: weekly when present, else the first. */
export function headlineWindow(account: UsageAccountView): UsageWindowView | null {
  return (
    account.windows.find((w) => w.id === 'seven_day') ??
    account.windows.find((w) => w.id === 'monthly') ??
    account.windows[0] ??
    null
  )
}

/**
 * Default tray label for a usage window (mirror of
 * `lr_config::TraySource::usage_label_seed`): provider initial + window, e.g.
 * `A7D`, `O5H`, `AF7` (weekly Fable cap), `GHM` (Copilot monthly).
 */
export function usageLabelSeed(account: string, window: string): string {
  const provider = account.split(':')[0] ?? account
  const prefix = provider === 'github-copilot' ? 'GH' : provider.slice(0, 1).toUpperCase()
  let suffix: string
  if (window === 'five_hour') suffix = '5H'
  else if (window === 'seven_day') suffix = '7D'
  else if (window === 'monthly') suffix = 'M'
  else if (window.startsWith('seven_day_')) suffix = `${(window.slice(10, 11) || 'W').toUpperCase()}7`
  else suffix = window.slice(0, 2)
  return `${prefix}${suffix}`.replace(/[^A-Za-z0-9]/g, '').toUpperCase().slice(0, 4)
}
