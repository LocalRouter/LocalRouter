import { test, expect } from '@playwright/test'
import type { McpServerPushStatus, PushEventDecision, PushEventIngressStatus, PushSourceInfo } from '../../../src/types/tauri-commands'
import { PUSH_DECISION_LABELS, pushDecisionDelivered, pushDecisionLabel, pushPayloadEdited } from '../../../src/views/monitor/push-event'
import { formatRelativeTime } from '../../../src/views/push-relay/format'
import {
  capabilitiesFooter,
  eventIssue,
  eventModesLabel,
  extrasLine,
  noPushReason,
  offersLine,
  pushLiveLine,
} from '../../../src/views/push-relay/push-relay-utils'
import { CLAUDE_CHANNELS_LAUNCH_COMMAND, claudeCodeStdioConfig } from '../../../src/components/client/claude-code-bridge-config'

const source = (patch: Partial<PushSourceInfo> = {}): PushSourceInfo => ({
  server_id: 's', server_name: 'S', enabled: true, snapshot_status: 'current',
  protocol_version: '2025-11-25', observed_at: null, channels: false, resource_subscribe: false,
  events: [], has_consumable_push: false, receiving_clients: 1, ...patch,
})

test('sources without consumable push say why', () => {
  expect(noPushReason(source({ has_consumable_push: true }))).toBeNull()
  expect(noPushReason(source({ snapshot_status: 'none' }))).toMatch(/Not known yet/)
  expect(noPushReason(source({ snapshot_status: 'stale' }))).toMatch(/reconnects/)
  expect(noPushReason(source({ enabled: false }))).toBe('Disabled')
  expect(noPushReason(source({
    events: [{ name: 'invoice.paid', description: null, modes: ['webhook'], supported: false }],
  }))).toBe('Webhook only — not supported')
  expect(noPushReason(source())).toBe("Doesn't offer push")
})

test('what a server offers is stated once, without webhook for usable types', () => {
  expect(offersLine(source())).toBe("This server doesn't offer push.")
  expect(offersLine(source({ snapshot_status: 'none' }))).toBe('Not known yet — appears after the server first connects.')
  expect(offersLine(source({ snapshot_status: 'stale' }))).toMatch(/reconnects/)
  expect(offersLine(source({ channels: true }))).toBeNull()
  expect(eventModesLabel({ modes: ['push', 'poll', 'webhook'] })).toBe('push, poll')
  expect(eventModesLabel({ modes: ['poll'] })).toBe('poll')
  expect(eventModesLabel({ modes: ['webhook'] })).toBe('webhook only — not supported')
  expect(extrasLine(source())).toBeNull()
  expect(extrasLine(source({ channels: true, resource_subscribe: true })))
    .toBe('Claude Code channel messages and resource update notifications')
  const now = Date.parse('2026-10-30T12:00:00Z')
  expect(capabilitiesFooter(source({ observed_at: '2026-10-30T11:59:24Z' }), now))
    .toBe('Capabilities checked 36 s ago · MCP 2025-11-25')
  expect(capabilitiesFooter(source({ protocol_version: null }), now)).toBeNull()
})

const status = (patch: Partial<McpServerPushStatus> = {}): McpServerPushStatus => ({
  server_id: 's', relay_running: true, listening: false, event_types: [], events_received: 0,
  last_event_at: null, consuming_clients: 0, consuming_sessions: 0, ...patch,
})
const ingress = (patch: Partial<PushEventIngressStatus>): PushEventIngressStatus => ({
  event: 'message.received', ingress_mode: null, state: 'idle', last_error: null, subscriptions: 0,
  events_received: 0, last_event_at: null, ...patch,
})

test('the live status line is green only while LocalRouter actually listens', () => {
  const now = Date.parse('2026-10-30T12:00:00Z')
  const pushing = source({
    has_consumable_push: true,
    events: [{ name: 'message.received', description: null, modes: ['push', 'poll', 'webhook'], supported: true }],
  })
  // Loading, or nothing to listen to: no line (no green for a fresh snapshot)
  expect(pushLiveLine(pushing, null, 'allow', now)).toBeNull()
  expect(pushLiveLine(source(), status(), 'allow', now)).toBeNull()

  expect(pushLiveLine(pushing, status({
    listening: true, events_received: 3, last_event_at: '2026-10-30T11:58:00Z', consuming_clients: 2,
    event_types: [ingress({ ingress_mode: 'push', state: 'listening', subscriptions: 1, events_received: 3 })],
  }), 'allow', now)).toEqual({ tone: 'listening', text: 'Listening via push stream · 3 events received · last 2 min ago · 2 clients' })

  expect(pushLiveLine(pushing, status({
    listening: true, consuming_clients: 1,
    event_types: [ingress({ ingress_mode: 'poll', state: 'listening', subscriptions: 1 })],
  }), 'allow', now)).toEqual({ tone: 'listening', text: 'Listening via polling · no events yet · 1 client' })

  const reconnecting = status({
    event_types: [ingress({ ingress_mode: 'push', state: 'reconnecting', last_error: 'upstream stream ended', subscriptions: 1 })],
  })
  expect(pushLiveLine(pushing, reconnecting, 'allow', now))
    .toEqual({ tone: 'warning', text: 'Reconnecting — upstream stream ended' })
  expect(eventIssue(reconnecting, 'message.received')).toBe('Reconnecting — upstream stream ended')
  expect(eventIssue(reconnecting, 'other')).toBeNull()

  expect(pushLiveLine(pushing, status({ event_types: [ingress({ state: 'connecting', subscriptions: 1 })] }), 'allow', now)?.tone)
    .toBe('idle')

  // Not listening: say why, and keep what arrived since start
  expect(pushLiveLine(pushing, status(), 'allow', now))
    .toEqual({ tone: 'idle', text: 'Not listening — no client is receiving these events right now' })
  expect(pushLiveLine(pushing, status({ events_received: 1, last_event_at: '2026-10-30T11:00:00Z' }), 'allow', now)?.text)
    .toBe('Not listening — no client is receiving these events right now · 1 event received · last 1 h ago')
  expect(pushLiveLine({ ...pushing, receiving_clients: 0 }, status(), 'off', now))
    .toEqual({ tone: 'idle', text: 'Push is off for all clients' })
  expect(pushLiveLine({ ...pushing, receiving_clients: 0 }, status(), 'allow', now)?.text)
    .toBe('Not listening — no client may receive these events')

  expect(pushLiveLine(pushing, status({ relay_running: false }), 'allow', now))
    .toEqual({ tone: 'error', text: 'Push Relay is not running' })
  expect(pushLiveLine({ ...pushing, enabled: false }, status(), 'allow', now)?.tone).toBe('idle')
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
