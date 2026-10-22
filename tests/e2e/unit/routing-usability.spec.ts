import { test, expect } from '@playwright/test'
import { filterPolicyModels, knownPrice } from '../../../src/components/strategy/policy-model-catalog'
import { gatewayBaseUrl, gatewayPortExplanation } from '../../../src/utils/gateway-connection'
import type { DetailedModelInfo, ServerConfigInfo } from '../../../src/types/tauri-commands'

const model = (provider: string, id: string, free?: boolean): DetailedModelInfo => ({ provider_instance: provider, model_id: id, display_name: 'Helpful model', provider_type: 'openai', capabilities: ['chat'], supports_streaming: true, context_window: 4096, free_tier_model: free == null ? null : { is_free: free, decided_by: 'provider', rule_index: null, reason: 'fixture' } })

test('policy model search combines words, provider filters and canonical per-model free status without mutating priority', () => {
  const models = [model('zeta', 'free', true), model('alpha', 'paid', false), model('alpha', 'free-looking-name'), model('alpha', 'free', true)]
  expect(filterPolicyModels(models, 'ALPHA helpful', '', false).map(m => m.model_id)).toEqual(['free', 'free-looking-name', 'paid'])
  expect(filterPolicyModels(models, 'free', 'alpha', true).map(m => m.model_id)).toEqual(['free'])
  expect(filterPolicyModels(models, '', '', true).map(m => m.provider_instance)).toEqual(['alpha', 'zeta'])
  expect(models[0].provider_instance).toBe('zeta')
  expect(filterPolicyModels(models, 'absent', '', false)).toEqual([])
})

test('unknown and invalid prices stay unknown while a real zero remains known', () => {
  for (const p of [undefined, null, -1, NaN, Infinity]) expect(knownPrice(p)).toBe(false)
  expect(knownPrice(0)).toBe(true)
  expect(knownPrice(0.001)).toBe(true)
})

test('connection details use bound ports, support wildcard IPv4/IPv6, and explain dynamic or occupied ports', () => {
  const config: ServerConfigInfo = { host: '127.0.0.1', port: 3625, actual_port: 3627, enable_cors: true }
  expect(gatewayBaseUrl(config)).toBe('http://127.0.0.1:3627')
  expect(gatewayPortExplanation(config)).toContain('3625 was occupied')
  expect(gatewayBaseUrl({ ...config, host: '0.0.0.0' })).toContain('127.0.0.1:3627')
  expect(gatewayBaseUrl({ ...config, host: '::' })).toBe('http://[::1]:3627')
  expect(gatewayBaseUrl({ ...config, host: '::1' })).toBe('http://[::1]:3627')
  expect(gatewayPortExplanation({ ...config, actual_port: null })).toBeNull()
  expect(gatewayPortExplanation({ ...config, actual_port: 3625 })).toBeNull()
  expect(gatewayPortExplanation({ ...config, port: 0 })).toContain('operating system assigned')
})

test('profile review respects model and provider overrides without implying permissions from a profile', async () => {
  const { profileModelRefs, profilePermissionIssues } = await import('../../../src/components/strategy/profile-review')
  const snapshot = { model_name: 'auto', prioritized_models: [['p', 'm'], ['q', 'other'], ['p', 'm']], available_models: [['p', 'available']], routing_policy: null } as unknown as import('../../../src/types/tauri-commands').RouteSnapshot
  expect(profileModelRefs(snapshot)).toEqual([['p', 'm'], ['q', 'other']])
  expect(profilePermissionIssues(snapshot, { global: 'off', providers: { p: 'allow' }, models: { p__m: 'ask' } })).toEqual(['Model requires approval: p/m', 'Model not permitted: q/other'])
})
