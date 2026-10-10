import { expect, test } from '@playwright/test'
import {
  formatDuration,
  formatPercent,
  formatUsd,
  headlineWindow,
  paceSummary,
  slotHeights,
  usageLabelSeed,
  visibleAccounts,
} from '../../../src/views/dashboard/usage-format'
import type { UsageAccountView, UsageWindowView } from '../../../src/types/tauri-commands'

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

test('slot heights scale to twice the even-pace share and mark the future', () => {
  const h = slotHeights(win())
  // Even pace per day is 100/7 ≈ 14.3 points; a full bar is 2× that.
  expect(h[0]).toBeCloseTo(10 / (2 * (100 / 7)))
  expect(h[4]).toBe(0)
  expect(h[5]).toBeNull()
  expect(h[6]).toBeNull()
  expect(slotHeights(win({ slots: [] }))).toEqual([])
})

test('visible accounts skip hidden and empty ones; headline prefers weekly', () => {
  const empty = account({ id: 'groq:api', windows: [], kind: 'api' })
  const quotasOnly = account({
    id: 'cerebras:api',
    windows: [],
    kind: 'api',
    quotas: [{ id: 'requests', label: 'Requests', limit: 30, remaining: 30, used_percent: 0, resets_at: null, updated_at: 0 }],
  })
  const hidden = account({ id: 'openai:subscription', hidden: true })
  const shown = account()
  expect(visibleAccounts([empty, quotasOnly, hidden, shown]).map((a) => a.id)).toEqual([
    'anthropic:subscription',
  ])
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
