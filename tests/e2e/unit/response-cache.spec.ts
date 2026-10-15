import { test, expect } from '@playwright/test'
import { localCacheReceipt } from '../../../src/utils/response-cache'
const body = (receipt: unknown) => ({
  extensions: { 'localrouter.response_cache': receipt },
})
test('local cache receipts distinguish served usage from explicitly zero new billing', () => {
  const receipt = localCacheReceipt(
    body({
      scope: 'resolved_model_call',
      hit: true,
      upstream_spend_usd: 0,
      billed_input_tokens: 0,
      billed_output_tokens: 0,
      age_ms: 20,
      served_usage: { prompt_tokens: 1000 },
      avoided_cost_estimate_usd: 0.05,
    }),
  )
  expect(receipt?.served_usage).toEqual({ prompt_tokens: 1000 })
  expect(receipt?.upstream_spend_usd).toBe(0)
  expect(receipt?.avoided_cost_estimate_usd).toBe(0.05)
})
test('missing, malformed or billable captures cannot invent a free cache hit', () => {
  for (const value of [
    null,
    {},
    { hit: true },
    {
      hit: true,
      upstream_spend_usd: 1,
      billed_input_tokens: 0,
      billed_output_tokens: 0,
    },
    {
      scope: 'resolved_model_call',
      hit: true,
      upstream_spend_usd: 0,
      billed_input_tokens: 10,
      billed_output_tokens: 0,
    },
    { mode: 'use' },
  ])
    expect(localCacheReceipt(body(value))).toBeNull()
})
test('unknown avoided prices stay unknown and do not become guaranteed savings', () => {
  const receipt = localCacheReceipt(
    body({
      scope: 'resolved_model_call',
      hit: true,
      upstream_spend_usd: 0,
      billed_input_tokens: 0,
      billed_output_tokens: 0,
      age_ms: -1,
      avoided_cost_estimate_usd: NaN,
    }),
  )
  expect(receipt?.avoided_cost_estimate_usd).toBeNull()
  expect(receipt?.age_ms).toBeNull()
})
