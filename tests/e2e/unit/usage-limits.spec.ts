import { expect, test } from '@playwright/test'
import {
  breakdownHint,
  breakdownSummary,
  formatDuration,
  formatDurationLong,
  formatPercent,
  formatUsd,
  headlineWindow,
  paceSummary,
  resetDetail,
  routedSpend,
  windowBreakdown,
  usageLabelSeed,
  visibleAccounts,
  awaitingLimits,
  displayTitle,
  limitsUnreported,
  quotaDetails,
  quotaShortLabel,
} from '../../../src/views/dashboard/usage-format'
import type {
  UsageAccountView,
  UsageQuotaView,
  UsageWindowView,
} from '../../../src/types/tauri-commands'

const NOW = Date.parse('2026-10-10T12:00:00Z')
const nowSecs = NOW / 1000

const win = (changes: Partial<UsageWindowView> = {}): UsageWindowView => ({
  id: 'seven_day',
  label: 'Weekly',
  used_percent: 45,
  resets_at: nowSecs + 3 * 86_400,
  window_secs: 604_800,
  updated_at: nowSecs - 60,
  source: 'proxy_headers',
  stale: false,
  elapsed_fraction: 4 / 7,
  projected_percent: 78.75,
  limit_eta: null,
  pace: 'warning',
  slots: [10, 12, 8, 15, 0, 0, 0],
  current_slot: 4,
  slot_secs: 86_400,
  slots_basis: 'traffic',
  api_equivalent_usd: 120,
  plan_share_usd: 20.7,
  history: [],
  ...changes,
})

const account = (changes: Partial<UsageAccountView> = {}): UsageAccountView => ({
  id: 'anthropic:subscription',
  provider: 'anthropic',
  provider_label: 'Anthropic',
  kind: 'subscription',
  title: 'Claude Max 20x',
  login_label: null,
  plan: 'default_claude_max_20x',
  plan_label: 'Max 20x',
  monthly_price_usd: 200,
  plan_overridden: false,
  status: 'allowed',
  sources: ['proxy_headers'],
  first_seen: 0,
  last_seen: nowSecs,
  hidden: false,
  windows: [win()],
  quotas: [],
  credits: null,
  reports_limits: true,
  spend: {
    last_24h_usd: 0, last_7d_usd: 0, last_30d_usd: 0, month_to_date_usd: 0,
    requests_30d: 0, tokens_30d: 0, daily_usd: [],
  },
  value_multiplier: null,
  ...changes,
})

test('formats percent down, money and durations', () => {
  expect(formatPercent(99.99)).toBe('99%')
  expect(formatPercent(-3)).toBe('0%')
  expect(formatUsd(3.456)).toBe('$3.46')
  expect(formatUsd(1234.5)).toBe('$1,235')
  expect(formatUsd(null)).toBe('—')
  expect(formatDuration(30)).toBe('<1m')
  expect(formatDuration(3 * 3600 + 12 * 60)).toBe('3h 12m')
  expect(formatDuration(2 * 86_400 + 4 * 3600)).toBe('2d 4h')
})

test('pace summary covers projection, eta, limit and reset', () => {
  expect(paceSummary(win(), NOW)).toBe('On pace for 78% at reset')
  expect(paceSummary(win({ limit_eta: nowSecs + 7200, pace: 'over' }), NOW)).toBe(
    'Limit in ~2h at this pace',
  )
  expect(paceSummary(win({ used_percent: 100 }), NOW)).toBe('Limit reached')
  expect(paceSummary(win({ stale: true }), NOW)).toContain('reset')
  expect(paceSummary(win({ projected_percent: null }), NOW)).toBe('Too early to project')
})

test('reset times are spelled out for the tooltip', () => {
  expect(formatDurationLong(30)).toBe('less than a minute')
  expect(formatDurationLong(60)).toBe('1 minute')
  expect(formatDurationLong(3600)).toBe('1 hour')
  expect(formatDurationLong(3 * 3600 + 12 * 60)).toBe('3 hours 12 minutes')
  expect(formatDurationLong(5 * 86_400 + 15 * 3600 + 59 * 60)).toBe('5 days 15 hours')
  expect(formatDurationLong(86_400)).toBe('1 day')
  const reset = nowSecs + 5 * 86_400 + 15 * 3600
  expect(resetDetail(reset, NOW)).toMatch(/^Resets in 5 days 15 hours · /)
  expect(resetDetail(nowSecs - 10, NOW)).toMatch(/^Reset /)
})

/** Unix seconds for a local wall-clock time, so labels do not depend on the TZ. */
const local = (y: number, mo: number, d: number, h = 0) => new Date(y, mo, d, h).getTime() / 1000

test('weekly windows break down per day, labelled from the first day', () => {
  // A week from Tue 6 Oct 2026 14:00 (local), now in its fifth day.
  const start = local(2026, 9, 6, 14)
  const b = windowBreakdown(win({ resets_at: start + 604_800 }))!
  expect(b.unit).toBe('day')
  expect(b.bars.map((x) => x.label).join(' ')).toBe('T W T F S S M')
  expect(b.bars.map((x) => x.name)).toEqual(['Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun', 'Mon'])
  // Full bar = twice the even-pace share (100/7 per day).
  expect(b.bars[0].height).toBeCloseTo(10 / (2 * (100 / 7)))
  expect(b.bars[0].points).toBe(10)
  expect(b.bars[4]).toMatchObject({ height: 0, current: true })
  expect(b.bars[5].height).toBeNull()
  expect(b.bars[6].height).toBeNull()
  expect(b.bars.filter((x) => x.current)).toHaveLength(1)
  expect(breakdownSummary(b)).toBe(
    'Per day: Tue 10%, Wed 12%, Thu 8%, Fri 15%, Sat 0%, Sun ahead, Mon ahead',
  )
  expect(breakdownHint(win(), b.unit)).toContain("LocalRouter's traffic")
  expect(breakdownHint(win({ slots_basis: 'readings' }), b.unit)).toContain('reading changes')
})

test('5-hour windows break down per hour, monthly ones per week', () => {
  const start = local(2026, 9, 10, 9)
  const five = windowBreakdown(
    win({
      id: 'five_hour',
      resets_at: start + 18_000,
      window_secs: 18_000,
      slots: [4, 50, 2, 0, 0],
      current_slot: 2,
      slot_secs: 3_600,
    }),
  )!
  expect(five.unit).toBe('hour')
  expect(five.bars.map((x) => x.label)).toEqual(['09', '10', '11', '12', '13'])
  expect(five.bars[0].name).toBe('09:00')
  // 50 points in one hour of a 5-hour window is over twice the pace (40): capped.
  expect(five.bars[1].height).toBe(1)
  expect(five.bars[2].current).toBe(true)
  expect(five.bars[3].height).toBeNull()

  const mStart = local(2026, 9, 1)
  const days = Array.from({ length: 30 }, (_, i) => (i < 10 ? 2 : 0))
  const month = windowBreakdown(
    win({
      id: 'monthly',
      resets_at: mStart + 30 * 86_400,
      window_secs: 30 * 86_400,
      slots: days,
      current_slot: 9,
      slot_secs: 86_400,
    }),
  )!
  expect(month.unit).toBe('week')
  expect(month.bars.map((x) => x.label)).toEqual(['1', '8', '15', '22', '29'])
  expect(month.bars[0].name).toBe('Oct 1')
  expect(month.bars[0].points).toBe(14)
  // Week 2 holds the current day (day 10): only days up to it count.
  expect(month.bars[1]).toMatchObject({ points: 6, current: true })
  expect(month.bars[2].height).toBeNull()
  // The last, two-day bar is still a bar of its own.
  expect(month.bars).toHaveLength(5)
})

test('windows without a known span have no breakdown', () => {
  expect(windowBreakdown(win({ slots: [] }))).toBeNull()
  expect(windowBreakdown(win({ resets_at: null }))).toBeNull()
  expect(windowBreakdown(win({ slot_secs: null }))).toBeNull()
})

test('visible accounts skip hidden and empty ones; headline prefers weekly', () => {
  const empty = account({ id: 'groq:api', windows: [], kind: 'api' })
  const quotasOnly = account({
    id: 'cerebras:api',
    windows: [],
    kind: 'api',
    quotas: [{ id: 'requests', label: 'Requests', limit: 30, remaining: 30, used_percent: 0, resets_at: null, window_secs: null, updated_at: 0 }],
  })
  const hidden = account({ id: 'openai:subscription', hidden: true })
  const shown = account()
  // A subscription seen without limits yet (no traffic carried them, its
  // usage endpoint not asked) still gets a card.
  const waiting = account({ id: 'openai:subscription:0a1b2c3d4e', windows: [] })
  expect(visibleAccounts([empty, quotasOnly, hidden, shown, waiting]).map((a) => a.id)).toEqual([
    'anthropic:subscription',
    'openai:subscription:0a1b2c3d4e',
  ])
  expect(awaitingLimits(waiting)).toBe(true)
  expect(awaitingLimits(shown)).toBe(false)
  const five = win({ id: 'five_hour' })
  expect(headlineWindow(account({ windows: [five, win()] }))?.id).toBe('seven_day')
  expect(headlineWindow(account({ windows: [five] }))?.id).toBe('five_hour')
  expect(headlineWindow(account({ windows: [] }))).toBeNull()
})

test('usage tray labels mirror the backend seeds', () => {
  expect(usageLabelSeed('anthropic:subscription', 'seven_day')).toBe('A7D')
  expect(usageLabelSeed('openai:subscription', 'five_hour')).toBe('O5H')
  expect(usageLabelSeed('anthropic:subscription', 'seven_day_fable')).toBe('AF7')
  expect(usageLabelSeed('github-copilot:subscription', 'monthly')).toBe('GHM')
})

test('routed spend is shown for API keys only, never as subscription value', () => {
  const spend = {
    last_24h_usd: 0.12, last_7d_usd: 1.3, last_30d_usd: 4.95, month_to_date_usd: 2,
    requests_30d: 312, tokens_30d: 1_200_000, daily_usd: [],
  }
  expect(routedSpend(account({ spend, value_multiplier: 4.2 }))).toBeNull()
  const api = account({ kind: 'api', id: 'openai:api', spend, monthly_price_usd: null })
  const r = routedSpend(api)!
  expect(r.periods).toEqual([['24h', 0.12], ['7d', 1.3], ['30d', 4.95]])
  expect(r.note).toBe('312 req · 30d')
  expect(r.label).toContain('Spend via LocalRouter')
  expect(routedSpend({ ...api, spend: { ...spend, requests_30d: 0 } })).toBeNull()
})

test('two accounts with the same title are told apart by their login', () => {
  const a = account({ id: 'openai:subscription', title: 'ChatGPT Pro', login_label: 'me@example.com' })
  const b = account({ id: 'openai:subscription:0a1b2c3d4e', title: 'ChatGPT Pro' })
  const c = account({ id: 'anthropic:subscription' })
  expect(displayTitle(a, [a, b, c])).toBe('ChatGPT Pro · me@example.com')
  expect(displayTitle(b, [a, b, c])).toBe('ChatGPT Pro · 0a1b2c3d4e')
  expect(displayTitle(c, [a, b, c])).toBe('Claude Max 20x')
  expect(displayTitle(a, [a, c])).toBe('ChatGPT Pro')
})

const quota = (changes: Partial<UsageQuotaView> = {}): UsageQuotaView => ({
  id: 'requests_day',
  label: 'Requests / day',
  limit: 14_400,
  remaining: 13_212,
  used_percent: 8.25,
  resets_at: nowSecs + 3 * 3600,
  window_secs: 86_400,
  updated_at: nowSecs - 30,
  ...changes,
})

test('rate limits get short row labels and spelled-out tooltips', () => {
  expect(quotaShortLabel(quota())).toBe('Req/day')
  expect(
    quotaShortLabel(quota({ id: 'tokens_minute', label: 'Tokens / minute', window_secs: 60 })),
  ).toBe('Tokens/min')
  expect(
    quotaShortLabel(
      quota({ id: 'free_requests_day', label: 'Free-model requests / day', window_secs: 86_400 }),
    ),
  ).toBe('Free req/day')
  // Limits without a period, and metrics without a short form, keep their label.
  expect(quotaShortLabel(quota({ id: 'requests', label: 'Requests', window_secs: null }))).toBe(
    'Requests',
  )
  expect(
    quotaShortLabel(
      quota({ id: 'priority_input_tokens', label: 'Priority Input Tokens', window_secs: null }),
    ),
  ).toBe('Priority Input Tokens')

  expect(quotaDetails(quota(), NOW)).toEqual([
    'Requests per day',
    '13,212 of 14,400 left (8% used)',
    'Full again in 3h',
    'Updated just now',
  ])
  const bare = quotaDetails(
    quota({ id: 'rate', label: 'Rate limit', window_secs: null, limit: null, resets_at: null }),
    NOW,
  )
  expect(bare).toEqual(['Rate limit', '13,212 left', 'Updated just now'])
})

test('providers that report no limits get a spend-only card, not a wait message', () => {
  const spend = {
    last_24h_usd: 0.5, last_7d_usd: 1, last_30d_usd: 2, month_to_date_usd: 1,
    requests_30d: 12, tokens_30d: 9_000, daily_usd: [],
  }
  const gemini = account({
    id: 'gemini:api', kind: 'api', windows: [], reports_limits: false, spend,
  })
  expect(visibleAccounts([gemini]).map((a) => a.id)).toEqual(['gemini:api'])
  expect(awaitingLimits(gemini)).toBe(false)
  expect(limitsUnreported(gemini)).toBe(true)
  const groq = account({ id: 'groq:api', kind: 'api', windows: [], spend })
  expect(awaitingLimits(groq)).toBe(true)
  expect(limitsUnreported(groq)).toBe(false)
  expect(limitsUnreported({ ...gemini, quotas: [quota()] })).toBe(false)
})
