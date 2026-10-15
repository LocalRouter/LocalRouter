import type {
  UsageAccountView,
  UsageDataSource,
  UsagePaceStatus,
  UsageQuotaView,
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

/** `5 days 15 hours`, `3 hours 12 minutes`, `45 minutes`: {@link formatDuration} spelled out. */
export function formatDurationLong(seconds: number): string {
  const s = Math.max(0, Math.round(seconds))
  if (s < 60) return 'less than a minute'
  const d = Math.floor(s / 86_400)
  const h = Math.floor((s % 86_400) / 3_600)
  const m = Math.floor((s % 3_600) / 60)
  const unit = (n: number, name: string) => `${n} ${name}${n === 1 ? '' : 's'}`
  if (d > 0) return h > 0 ? `${unit(d, 'day')} ${unit(h, 'hour')}` : unit(d, 'day')
  if (h > 0) return m > 0 ? `${unit(h, 'hour')} ${unit(m, 'minute')}` : unit(h, 'hour')
  return unit(m, 'minute')
}

/** Tooltip line for a window's reset: `Resets in 5 days 15 hours · Mon 14:00`. */
export function resetDetail(resetsAt: number, nowMs: number): string {
  const left = resetsAt - nowMs / 1000
  if (left <= 0) return `Reset ${formatClock(resetsAt)}`
  return `Resets in ${formatDurationLong(left)} · ${formatClock(resetsAt)}`
}

const WEEKDAY_INITIAL = ['S', 'M', 'T', 'W', 'T', 'F', 'S']
const WEEKDAY_SHORT = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']
const MONTH_SHORT = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

/** Windows with more daily slots than this (monthly) are shown per week. */
const MAX_DAILY_BARS = 10

export type BreakdownUnit = 'hour' | 'day' | 'week'

export interface BreakdownBar {
  /** 0–1, where a full bar is twice the even-pace share; `null` while still ahead. */
  height: number | null
  /** Percentage points of the window used in this bar's span. */
  points: number
  /** Axis label: weekday initial, two-digit hour, or day of month. */
  label: string
  /** Spelled-out span start for screen readers: `Tue`, `14:00`, `Oct 8`. */
  name: string
  current: boolean
}

export interface WindowBreakdown {
  unit: BreakdownUnit
  bars: BreakdownBar[]
}

/**
 * The window split into bars, oldest first and ending at the reset: one per
 * hour for 5-hour windows, per day for weekly ones (labelled `T W T F S S M`
 * from the window's first day), per week for monthly ones. Labels use local
 * time. `null` when the window has no breakdown (unknown start, or stale).
 */
export function windowBreakdown(window: UsageWindowView): WindowBreakdown | null {
  const n = window.slots.length
  if (n < 2 || window.slot_secs === null || window.resets_at === null || window.window_secs === null) {
    return null
  }
  const start = window.resets_at - window.window_secs
  const daily = window.slot_secs >= 86_400
  const group = daily && n > MAX_DAILY_BARS ? 7 : 1
  const unit: BreakdownUnit = group > 1 ? 'week' : daily ? 'day' : 'hour'
  const share = (100 * group) / n
  const bars: BreakdownBar[] = []
  for (let first = 0; first < n; first += group) {
    const last = Math.min(n, first + group) - 1
    let points = 0
    for (let i = first; i <= last && i <= window.current_slot; i++) {
      points += Math.max(0, window.slots[i])
    }
    const at = new Date((start + first * window.slot_secs) * 1000)
    const hour = String(at.getHours()).padStart(2, '0')
    bars.push({
      height: first > window.current_slot ? null : Math.min(1, points / (2 * share)),
      points,
      label:
        unit === 'hour' ? hour : unit === 'day' ? WEEKDAY_INITIAL[at.getDay()] : String(at.getDate()),
      name:
        unit === 'hour'
          ? `${hour}:00`
          : unit === 'day'
            ? WEEKDAY_SHORT[at.getDay()]
            : `${MONTH_SHORT[at.getMonth()]} ${at.getDate()}`,
      current: window.current_slot >= first && window.current_slot <= last,
    })
  }
  return { unit, bars }
}

/** Screen-reader summary of a breakdown: `Per day: Tue 8%, Wed 11%, …, Mon ahead`. */
export function breakdownSummary(breakdown: WindowBreakdown): string {
  const per = { hour: 'Per hour', day: 'Per day', week: 'Per week' }[breakdown.unit]
  const parts = breakdown.bars.map((b) =>
    b.height === null ? `${b.name} ahead` : `${b.name} ${formatPercent(b.points)}`,
  )
  return `${per}: ${parts.join(', ')}`
}

/** Tooltip line saying what the bars are and where they come from. */
export function breakdownHint(window: UsageWindowView, unit: BreakdownUnit): string {
  const per = { hour: 'hour', day: 'day', week: 'week' }[unit]
  if (window.slots_basis === 'traffic') {
    return `Bars: usage per ${per}, split by LocalRouter's traffic`
  }
  return `Bars: usage per ${per}, from reading changes`
}

/**
 * Accounts worth a card: not hidden, and with limits or traffic to show. A
 * subscription gets a card as soon as it is seen (its limits may only come
 * with the next request or poll); an API account known only from rate-limit
 * headers (e.g. a provider health check) without any requests is left to
 * Settings.
 */
export function visibleAccounts(accounts: UsageAccountView[]): UsageAccountView[] {
  return accounts.filter(
    (a) =>
      !a.hidden &&
      (a.kind === 'subscription' ||
        a.windows.length > 0 ||
        a.credits !== null ||
        a.spend.requests_30d > 0),
  )
}

function hasLimits(account: UsageAccountView): boolean {
  return account.windows.length > 0 || account.quotas.length > 0 || account.credits !== null
}

/**
 * Whether an account has no limits, credits or quotas to show yet, from a
 * provider that does report them.
 */
export function awaitingLimits(account: UsageAccountView): boolean {
  return account.reports_limits && !hasLimits(account)
}

/** Whether the provider reports no limits at all (its card shows spend only). */
export function limitsUnreported(account: UsageAccountView): boolean {
  return !account.reports_limits && !hasLimits(account)
}

const QUOTA_METRIC_SHORT: Record<string, string> = {
  requests: 'Req',
  tokens: 'Tokens',
  input_tokens: 'Input',
  output_tokens: 'Output',
  free_requests: 'Free req',
}
const QUOTA_PERIOD_SHORT: Record<number, string> = {
  1: 's',
  60: 'min',
  3_600: 'hour',
  86_400: 'day',
  2_592_000: 'mo',
}
const QUOTA_PERIOD_LONG: Record<number, string> = {
  1: 'second',
  60: 'minute',
  3_600: 'hour',
  86_400: 'day',
  2_592_000: 'month',
}

/** The quota's metric id without its period suffix (`tokens_minute` → `tokens`). */
function quotaMetric(quota: UsageQuotaView): string {
  return quota.window_secs === null
    ? quota.id
    : quota.id.replace(/_(second|minute|hour|day|month)$/, '')
}

/**
 * Row label for a rate limit, short enough for the card's label column:
 * `Req/min`, `Tokens/day`, or the full label for limits without a period.
 */
export function quotaShortLabel(quota: UsageQuotaView): string {
  if (quota.window_secs === null) return quota.label
  const metric = QUOTA_METRIC_SHORT[quotaMetric(quota)]
  const period = QUOTA_PERIOD_SHORT[quota.window_secs]
  return metric && period ? `${metric}/${period}` : quota.label
}

/** Tooltip lines for a rate limit. */
export function quotaDetails(quota: UsageQuotaView, nowMs: number): string[] {
  const period = quota.window_secs !== null ? QUOTA_PERIOD_LONG[quota.window_secs] : undefined
  const metric = quota.label.split(' / ')[0]
  const exact = (v: number) => Math.round(v).toLocaleString('en-US')
  const lines = [period ? `${metric} per ${period}` : quota.label]
  if (quota.remaining !== null && quota.limit !== null) {
    const used = quota.used_percent !== null ? ` (${formatPercent(quota.used_percent)} used)` : ''
    lines.push(`${exact(quota.remaining)} of ${exact(quota.limit)} left${used}`)
  } else if (quota.remaining !== null) {
    lines.push(`${exact(quota.remaining)} left`)
  } else if (quota.limit !== null) {
    lines.push(`Limit ${exact(quota.limit)}`)
  }
  if (quota.resets_at !== null && quota.resets_at > nowMs / 1000) {
    lines.push(`Full again in ${formatDuration(quota.resets_at - nowMs / 1000)}`)
  }
  lines.push(`Updated ${formatAgo(quota.updated_at, nowMs)}`)
  return lines
}

/**
 * Card title: the account's title, plus who the login is when another shown
 * account has the same title (two ChatGPT accounts on the same plan).
 */
export function displayTitle(account: UsageAccountView, shown: UsageAccountView[]): string {
  const twin = shown.some((a) => a.id !== account.id && a.title === account.title)
  if (!twin) return account.title
  const who = account.login_label ?? account.id.split(':')[2] ?? null
  return who ? `${account.title} · ${who}` : account.title
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

export interface RoutedSpend {
  label: string
  /** `[period, USD]`, shortest first. */
  periods: [string, number][]
  /** Short context at the end of the row. */
  note: string | null
  /** Tooltip lines. */
  hint: string[]
}

/**
 * What the requests LocalRouter sent with an API key cost at list prices,
 * over the last 24 hours, 7 days and 30 days. Subscriptions have none: their
 * card shows the plan's own limits.
 */
export function routedSpend(account: UsageAccountView): RoutedSpend | null {
  const s = account.spend
  if (account.kind === 'subscription' || s.requests_30d <= 0) return null
  return {
    label: 'Spend via LocalRouter',
    periods: [
      ['24h', s.last_24h_usd],
      ['7d', s.last_7d_usd],
      ['30d', s.last_30d_usd],
    ],
    note: `${formatCount(s.requests_30d)} req · 30d`,
    hint: [
      'Requests LocalRouter sent with this key, at list prices. The bill may differ.',
      `${formatCount(s.requests_30d)} requests, ${formatCount(s.tokens_30d)} tokens in 30 days.`,
    ],
  }
}
