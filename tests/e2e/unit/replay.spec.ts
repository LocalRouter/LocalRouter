import { test, expect } from '@playwright/test'
import { sumReplayCosts } from '../../../src/utils/replay'
import type { ReplayCost } from '../../../src/types/tauri-commands'
const cost = (overrides: Partial<ReplayCost> = {}): ReplayCost => ({
  reported_usd: null,
  estimated_usd: null,
  unknown_calls: 0,
  classifier_reported_usd: null,
  classifier_estimated_usd: null,
  classifier_cost_unknown: false,
  ...overrides,
})
test('empty and unknown costs never become a zero-dollar quote', () => {
  expect(sumReplayCosts([]).reported_usd).toBeNull()
  expect(sumReplayCosts([cost({ unknown_calls: 1 })])).toEqual(
    cost({ unknown_calls: 1 }),
  )
})
test('known partial spend remains separate from estimates and classifier charges', () => {
  expect(
    sumReplayCosts([
      cost({ reported_usd: 0.2, classifier_reported_usd: 0.03 }),
      cost({
        estimated_usd: 0.1,
        classifier_estimated_usd: 0.01,
        unknown_calls: 2,
        classifier_cost_unknown: true,
      }),
    ]),
  ).toEqual(
    cost({
      reported_usd: 0.2,
      estimated_usd: 0.1,
      classifier_reported_usd: 0.03,
      classifier_estimated_usd: 0.01,
      unknown_calls: 2,
      classifier_cost_unknown: true,
    }),
  )
})
test('explicit free calls remain distinguishable from missing prices', () => {
  expect(sumReplayCosts([cost({ estimated_usd: 0 })]).estimated_usd).toBe(0)
})
