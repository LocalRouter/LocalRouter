import { test, expect } from '@playwright/test'
import type { PushRelayLimits, PushSourceInfo } from '../../../src/types/tauri-commands'
import { LIMIT_GROUPS, noPushReason } from '../../../src/views/push-relay/push-relay-utils'
import { CLAUDE_CHANNELS_LAUNCH_COMMAND, claudeCodeStdioConfig } from '../../../src/components/client/claude-code-bridge-config'
import { defaultPushRelayLimits } from '../../../website/src/components/demo/mockPushRelay'

const source = (patch: Partial<PushSourceInfo> = {}): PushSourceInfo => ({
  server_id: 's', server_name: 'S', enabled: true, snapshot_status: 'current',
  protocol_version: '2025-11-25', observed_at: null, channels: false, resource_subscribe: false,
  events: [], has_consumable_push: false, ...patch,
})

test('the limits editor covers every Push Relay limit exactly once', () => {
  const edited = LIMIT_GROUPS.flatMap(g => g.rows.map(r => r.key as string))
  expect(new Set(edited).size).toBe(edited.length)
  // The retry list is edited separately; every other field is numeric and listed.
  const all = Object.keys(defaultPushRelayLimits).filter(k => k !== 'webhook_retry_delays_secs')
  expect(edited.sort()).toEqual(all.sort())
  for (const g of LIMIT_GROUPS) for (const r of g.rows) {
    const value = defaultPushRelayLimits[r.key as keyof PushRelayLimits] as number
    expect(Number.isInteger(value / r.scale) || r.scale === 1, `${r.key} default shows as a whole ${r.unit}`).toBe(true)
  }
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
