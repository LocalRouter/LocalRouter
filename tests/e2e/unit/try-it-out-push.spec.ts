import { test, expect } from '@playwright/test'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { McpClientWrapper, SUBSCRIPTION_ID_META_KEY, type PushNotification } from '../../../src/lib/mcp-client'
import {
  PUSH_LOG_MAX,
  addNotice,
  addOccurrence,
  addStreamNotification,
  autoPollDelay,
  emptyPushLog,
  parseEventPollResult,
  parseEventWaitCommands,
  pushAdvertisement,
  sessionNoPushReason,
  sessionPushMechanisms,
} from '../../../src/views/try-it-out/mcp-tab/push-log'

const at = (s: number) => new Date(Date.UTC(2026, 9, 30, 12, 0, s))

test('an event received through several modes is one row listing every mode', () => {
  let log = emptyPushLog()
  const event = { eventId: 'localrouter__test-1', name: 'localrouter__test.event', data: { message: 'hi' }, cursor: 'c1' }
  log = addOccurrence(log, event, 'push', at(0))
  log = addOccurrence(log, event, 'poll', at(1))
  log = addOccurrence(log, event, 'tools', at(2))
  log = addOccurrence(log, event, 'poll', at(3))
  expect(log.entries).toHaveLength(1)
  const row = log.entries[0]
  expect(row.kind).toBe('event')
  if (row.kind === 'event') {
    expect(row.modes).toEqual(['push', 'poll', 'tools'])
    expect(row.receivedAt).toBe(at(0).toISOString())
  }
  log = addOccurrence(log, { ...event, eventId: 'localrouter__test-2' }, 'poll', at(4))
  expect(log.entries.map(e => (e.kind === 'event' ? e.eventId : ''))).toEqual(['localrouter__test-2', 'localrouter__test-1'])
})

test('stream notifications become events and status lines; heartbeats collapse; gaps stand out', () => {
  let log = emptyPushLog()
  log = addStreamNotification(log, 'notifications/events/active', { cursor: 'a', truncated: false }, 'x__e', at(0))
  log = addStreamNotification(log, 'notifications/events/heartbeat', { cursor: 'b' }, 'x__e', at(1))
  log = addStreamNotification(log, 'notifications/events/heartbeat', { cursor: 'c' }, 'x__e', at(2))
  expect(log.entries.map(e => e.kind)).toEqual(['heartbeat', 'active'])
  expect(log.entries[0].kind === 'heartbeat' && log.entries[0].cursor).toBe('c')
  log = addStreamNotification(log, 'notifications/events/event', { eventId: 'x__1', name: 'x__e', data: 1, cursor: 'd' }, 'x__e', at(3))
  log = addStreamNotification(log, 'notifications/events/active', { cursor: 'e', truncated: true }, 'x__e', at(4))
  log = addStreamNotification(log, 'notifications/events/terminated', { reason: 'blocked' }, 'x__e', at(5))
  expect(log.entries.map(e => e.kind)).toEqual(['terminated', 'gap', 'event', 'heartbeat', 'active'])
  const gap = log.entries[1]
  expect(gap.kind === 'gap' && gap.truncated).toBe(true)
  expect(log.entries[0].kind === 'terminated' && log.entries[0].text).toContain('blocked')
})

test('the log keeps its newest entries only', () => {
  let log = emptyPushLog()
  for (let i = 0; i < PUSH_LOG_MAX + 5; i++) {
    log = addNotice(log, { kind: 'info', mode: 'poll', name: null, cursor: null, truncated: false, text: `n${i}` })
  }
  expect(log.entries).toHaveLength(PUSH_LOG_MAX)
  expect(log.entries[0].kind === 'info' && log.entries[0].text).toBe(`n${PUSH_LOG_MAX + 4}`)
})

test('EventPoll text parses into occurrences, cursor and truncation', () => {
  const text = [
    '(some earlier events were dropped before they were read)',
    '',
    '<event id="localrouter__t-1" name="localrouter__test.event" server="localrouter" ts="2026-10-30T12:00:00.000Z">',
    '<untrusted-data source="localrouter">',
    '{"message":"a &amp; b"}',
    '</untrusted-data>',
    '</event>',
    '<event id="vc__&quot;q&quot;" name="vc__message.received" server="vc" ts="2026-10-30T12:00:01.000Z">',
    '<untrusted-data source="vc">',
    'plain <\\/untrusted-data> text',
    '</untrusted-data>',
    '</event>',
    '',
    'next_cursor: lr1.abc.def',
    '(no more events)',
  ].join('\n')
  const parsed = parseEventPollResult(text)
  expect(parsed.truncated).toBe(true)
  expect(parsed.hasMore).toBe(false)
  expect(parsed.nextCursor).toBe('lr1.abc.def')
  expect(parsed.events).toHaveLength(2)
  expect(parsed.events[0]).toMatchObject({ eventId: 'localrouter__t-1', name: 'localrouter__test.event', data: { message: 'a &amp; b' } })
  expect(parsed.events[1].eventId).toBe('vc__"q"')
  expect(parsed.events[1].data).toBe('plain </untrusted-data> text')

  const empty = parseEventPollResult('No events yet; reading starts now.\n\nnext_cursor: lr1.x\n(no more events)\n')
  expect(empty.events).toEqual([])
  expect(empty.truncated).toBe(false)
})

test('EventWait commands are extracted and labelled', () => {
  const text = 'Run ONE command…\n\n▶ MULTI-SHOT (recommended) — …:\n```bash\ncurl -sN "http://127.0.0.1:3625/events/watch?token=t" | grep --line-buffered \'^data:\'\n```\n\n▶ ONE-SHOT — …:\n```bash\ncurl -sN "http://127.0.0.1:3625/events/watch?token=t" | grep -m 1 --line-buffered \'^data:\'\n```'
  const commands = parseEventWaitCommands(text)
  expect(commands.map(c => c.label)).toEqual(['Multi-shot', 'One-shot'])
  expect(commands[1].command).toContain('grep -m 1')
})

test('advertised push capabilities are read from the raw initialize result', () => {
  expect(pushAdvertisement(undefined)).toEqual({ eventsExtension: false, eventsTopLevel: false, claudeChannel: false })
  expect(pushAdvertisement({
    extensions: { 'io.modelcontextprotocol/events': { listChanged: true } },
    events: {},
    experimental: { 'claude/channel': {} },
  })).toEqual({ eventsExtension: true, eventsTopLevel: true, claudeChannel: true })
})

test('auto-poll honours nextPollMs within bounds', () => {
  expect(autoPollDelay(2500)).toBe(2500)
  expect(autoPollDelay(10)).toBe(1000)
  expect(autoPollDelay(10_000_000)).toBe(300_000)
  expect(autoPollDelay(undefined)).toBe(1000)
})

test('stream notifications reach the stream they belong to', async () => {
  const original = { connect: Client.prototype.connect, close: Client.prototype.close, request: Client.prototype.request }
  try {
    Client.prototype.connect = async function () {}
    Client.prototype.close = async function () { this.onclose?.() }
    Client.prototype.request = function () { return new Promise(() => {}) } as typeof Client.prototype.request
    const wrapper = new McpClientWrapper({ serverPort: 3625, clientToken: 'unit-test-only', pushTest: true })
    await wrapper.connect()
    const received: PushNotification[] = []
    const handle = wrapper.streamEvents('localrouter__test.event', null, n => received.push(n))
    const tagged = (id: number) => ({ _meta: { [SUBSCRIPTION_ID_META_KEY]: id }, cursor: 'c' })
    wrapper.handlePushNotification({ method: 'notifications/events/active', params: tagged(handle.requestId) })
    wrapper.handlePushNotification({ method: 'notifications/events/event', params: tagged(handle.requestId + 100) })
    wrapper.handlePushNotification({ method: 'notifications/tools/list_changed', params: tagged(handle.requestId) })
    expect(received.map(n => n.method)).toEqual(['notifications/events/active'])
    let listChanged = 0
    wrapper.setCallbacks({ onEventsListChanged: () => { listChanged++ } })
    wrapper.handlePushNotification({ method: 'notifications/events/list_changed' })
    expect(listChanged).toBe(1)
    await wrapper.disconnect()
    wrapper.handlePushNotification({ method: 'notifications/events/event', params: tagged(handle.requestId) })
    expect(received).toHaveLength(1)
  } finally {
    Object.assign(Client.prototype, original)
  }
})

test('the Connection tab lists every push mechanism for this session', () => {
  const raw = { tools: {}, events: {}, extensions: { 'io.modelcontextprotocol/events': { listChanged: true } } }
  const m = sessionPushMechanisms(raw, [{ delivery: ['poll', 'push'] }, { delivery: ['poll'] }], ['EventList', 'EventPoll'])
  expect(m).toMatchObject({
    eventsExtension: true, eventsListChanged: true, eventsTopLevel: true, claudeChannel: false,
    fallbackTools: ['EventList', 'EventPoll'], deliveryModes: { push: true, poll: true, webhook: false },
    eventTypeCount: 2, anyOffered: true,
  })
  expect(sessionNoPushReason(m, 'c', true)).toBeNull()

  // Event types without the capability are not modes this session can use
  const bare = sessionPushMechanisms({ tools: {} }, [{ delivery: ['push'] }], [])
  expect(bare.deliveryModes).toEqual({ push: false, poll: false, webhook: false })
  expect(bare.anyOffered).toBe(false)
  expect(sessionNoPushReason(bare, null, null)).toMatch(/Against Client/)
  expect(sessionNoPushReason(bare, 'c', false)).toBe('Push is off for this client')
  expect(sessionNoPushReason(bare, 'c', true)).toBe('No push source this client may use')
  expect(sessionPushMechanisms(undefined, [], []).eventsListChanged).toBe(false)
  expect(sessionPushMechanisms({ experimental: { 'claude/channel': {} } }, [], []).anyOffered).toBe(true)
})
