import { test, expect } from '@playwright/test'
import type { PushEventDecision, PushSourceInfo } from '../../../src/types/tauri-commands'
import { PUSH_DECISION_LABELS, pushDecisionDelivered, pushDecisionLabel, pushPayloadEdited } from '../../../src/views/monitor/push-event'
import { formatRelativeTime } from '../../../src/views/push-relay/format'
import { noPushReason } from '../../../src/views/push-relay/push-relay-utils'
import { CLAUDE_CHANNELS_LAUNCH_COMMAND, claudeCodeStdioConfig } from '../../../src/components/client/claude-code-bridge-config'

const source = (patch: Partial<PushSourceInfo> = {}): PushSourceInfo => ({
  server_id: 's', server_name: 'S', enabled: true, snapshot_status: 'current',
  protocol_version: '2025-11-25', observed_at: null, channels: false, resource_subscribe: false,
  events: [], has_consumable_push: false, receiving_clients: 1, ...patch,
})

test('sources without consumable push say why', () => {
  expect(noPushReason(source({ has_consumable_push: true }))).toBeNull()
  expect(noPushReason(source({ snapshot_status: 'none' }))).toMatch(/Not observed/)
  expect(noPushReason(source({ snapshot_status: 'stale' }))).toMatch(/reconnect/)
  expect(noPushReason(source({ enabled: false }))).toBe('Disabled')
  expect(noPushReason(source({
    events: [{ name: 'invoice.paid', description: null, modes: ['webhook'], supported: false }],
  }))).toBe('Webhook only — not supported')
  expect(noPushReason(source())).toBe('No push offered')
})

test('Claude Code defaults to the STDIO bridge with the channels flag', () => {
  const config = JSON.parse(claudeCodeStdioConfig('/bin/lr', 'cid', 'sec'))
  expect(config.mcpServers.localrouter).toEqual({
    type: 'stdio',
    command: '/bin/lr',
    args: ['--mcp-bridge', '--client-id', 'cid'],
    env: { LOCALROUTER_CLIENT_SECRET: 'sec' },
  })
  expect(CLAUDE_CHANNELS_LAUNCH_COMMAND).toBe('claude --dangerously-load-development-channels server:localrouter')
})

test('Monitor push records label every decision and detect edits structurally', () => {
  const decisions: PushEventDecision[] = ['pending', 'allowed', 'asked_allowed', 'asked_edited', 'denied', 'off', 'hold_cap', 'too_large']
  for (const d of decisions) expect(PUSH_DECISION_LABELS[d]).toBeTruthy()
  expect(pushDecisionLabel('something_new')).toBe('something_new')
  // Records stored before the Off vocabulary still display
  expect(pushDecisionLabel('blocked')).toBe(PUSH_DECISION_LABELS.off)
  expect(pushDecisionDelivered('asked_edited')).toBe(true)
  expect(pushDecisionDelivered('denied')).toBe(false)
  expect(pushPayloadEdited({ a: 1, b: 2 }, { b: 2, a: 1 })).toBe(false)
  expect(pushPayloadEdited({ text: 'x' }, { text: 'y' })).toBe(true)
  expect(pushPayloadEdited({ text: 'x' }, undefined)).toBe(false)
})

test('relative times read naturally in both directions', () => {
  const now = Date.parse('2026-10-30T12:00:00Z')
  expect(formatRelativeTime('2026-10-30T11:57:00Z', now)).toBe('3 min ago')
  expect(formatRelativeTime('2026-10-30T14:00:00Z', now)).toBe('in 2 h')
  expect(formatRelativeTime('2026-10-30T12:00:02Z', now)).toBe('just now')
  expect(formatRelativeTime('not a date', now)).toBe('not a date')
})
