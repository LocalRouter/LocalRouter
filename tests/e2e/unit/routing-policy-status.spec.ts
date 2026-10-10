import { test, expect } from '@playwright/test'
import { isRoutingPolicyIssue, routingPolicyIssue, routingPolicyIssueMessage } from '../../../src/components/strategy/routing-policy-status'
import type { RoutingPolicy } from '../../../src/types/tauri-commands'

// The policy a 0.0.153 strong/weak config migrates to: on, with the weak pool
// as a destination, but no decision model to choose it.
const migrated = (): RoutingPolicy => ({
  version: 1, enabled: true, mode: 'semantic', decision_model: null, question: 'Which route?',
  options: [
    { id: 'thorough', description: 'Strong pool', models: [] },
    { id: 'routine', description: 'Weak pool', models: [['local', 'small']] },
  ],
  default_route: 'thorough', mode_rules: [], min_probability: 0, timeout_ms: 3000, max_context_chars: 2000, history_messages: 2,
})

test('setup issue matches RoutingPolicy::setup_issue', () => {
  const p = migrated()
  expect(routingPolicyIssue(p)).toBe('needs_decision_model')
  expect(routingPolicyIssue({ ...p, decision_model: ['decider', 'm'] })).toBeNull()
  expect(routingPolicyIssue({ ...p, mode: 'client_mode', decision_model: ['decider', 'm'] })).toBe('needs_mode_rule')
  const rules = [{ mode: 'plan', route: 'routine' }]
  expect(routingPolicyIssue({ ...p, mode: 'client_mode', mode_rules: rules })).toBeNull()
  expect(routingPolicyIssue({ ...p, mode_rules: rules })).toBeNull()
  expect(routingPolicyIssue({ ...p, mode_rules: rules, options: p.options.map(o => ({ ...o, models: [] })) })).toBe('needs_destination_models')
  expect(routingPolicyIssue(null)).toBeNull()
})

test('issue message names the fallback route and status codes are recognized', () => {
  expect(routingPolicyIssueMessage('needs_decision_model', 'thorough')).toContain('default route (thorough)')
  expect(isRoutingPolicyIssue('needs_mode_rule')).toBe(true)
  expect(isRoutingPolicyIssue('routing')).toBe(false)
  expect(isRoutingPolicyIssue(undefined)).toBe(false)
})
