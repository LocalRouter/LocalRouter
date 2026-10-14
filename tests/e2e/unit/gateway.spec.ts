import { test, expect } from '@playwright/test'
import { gatewayAttribution } from '../../../src/utils/gateway'

test('gateway monitor handles partial and malformed captured traffic', () => {
  for (const value of [null, {}, [], { extensions: { 'localrouter.gateway': [] } }, { extensions: { 'localrouter.gateway': { gateway: 'unknown' } } }]) expect(gatewayAttribution(value)).toBeNull()
  const attribution = gatewayAttribution({ extensions: { 'localrouter.gateway': { gateway: 'vercel', cost_usd: -1, inference_provider: {}, routing: [null, 'bad', { provider: 'openai', succeeded: true, apiKeyHash: 'secret' }] } } })
  expect(attribution?.cost_source).toBe('unknown')
  expect(attribution?.inference_provider).toBeNull()
  expect(attribution?.routing).toHaveLength(1)
  expect(JSON.stringify(attribution)).not.toContain('secret')
})

test('gateway monitor preserves genuine zero reported cost and separate provider identity', () => {
  const attribution = gatewayAttribution({ provider: 'my-gateway', extensions: { 'localrouter.gateway': { gateway: 'openrouter', inference_provider: 'OpenAI', cost_usd: 0, cost_source: 'reported', service_tier: 'flex' } } })
  expect(attribution?.cost_usd).toBe(0)
  expect(attribution?.cost_source).toBe('reported')
  expect(attribution?.inference_provider).toBe('OpenAI')
  expect(attribution?.service_tier).toBe('flex')
})
