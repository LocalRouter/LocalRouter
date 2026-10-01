import { test, expect } from '@playwright/test'
import { mkdtempSync, mkdirSync, writeFileSync, symlinkSync, rmSync, realpathSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { matchesFilter, mergeMonitorEvents } from '../../../src/views/monitor/monitor-events'
import type { MonitorEventSummary, MonitorEventFilter } from '../../../src/types/tauri-commands'
import { resolveSharedIcon } from '../../../website/shared-icons'
import { emit, listen, once } from '../../../website/src/stubs/tauri-api-event'
import { isValidHttpUrl } from '../../../src/utils/url'

const event = (id: string, sequence: number, status: MonitorEventSummary['status'] = 'pending'): MonitorEventSummary => ({
  id, sequence, status, timestamp: '2026-10-01T12:00:00Z', event_type: 'llm_call',
  session_id: null, client_id: null, client_name: null, duration_ms: null, summary: 'Example request',
})
const filter = (fields: Partial<MonitorEventFilter>): MonitorEventFilter => ({
  event_types: null, session_id: null, client_id: null, status: null, search: null, ...fields,
})

test('completed events enter the status filter when their update arrives', () => {
  expect(mergeMonitorEvents([], [event('a', 1, 'complete')], filter({ status: 'complete' }))).toHaveLength(1)
})

test('live updates override stale snapshots, remove nonmatches, and deduplicate', () => {
  const result = mergeMonitorEvents([event('a', 1), event('b', 2)], [event('a', 1, 'complete'), event('c', 3)], filter({ status: 'pending' }))
  expect(result.map(item => item.id)).toEqual(['c', 'b'])
  expect(mergeMonitorEvents([event('a', 1)], [event('a', 1, 'complete')])).toEqual([event('a', 1, 'complete')])
})

test('monitor merge uses sequence order and bounds the visible list', () => {
  expect(mergeMonitorEvents([event('a', 1), event('c', 3)], [event('b', 2)], null, 2).map(item => item.id)).toEqual(['c', 'b'])
})

test('monitor filter dimensions and case-insensitive search are preserved', () => {
  expect(matchesFilter(event('a', 1), filter({ event_types: [], search: 'EXAMPLE' }))).toBe(true)
  expect(matchesFilter(event('a', 1), filter({ event_types: ['auth_error'] }))).toBe(false)
  expect(matchesFilter(event('a', 1), filter({ client_id: 'another' }))).toBe(false)
  expect(matchesFilter(event('a', 1), filter({ session_id: 'another' }))).toBe(false)
})

test('icon resolution refuses traversal, malformed paths, and escaping symlinks', () => {
  const dir = mkdtempSync(path.join(tmpdir(), 'localrouter-icons-'))
  try {
    const icons = path.join(dir, 'icons')
    mkdirSync(icons)
    writeFileSync(path.join(icons, 'safe.svg'), '<svg/>')
    writeFileSync(path.join(dir, 'secret.svg'), 'secret')
    symlinkSync(path.join(dir, 'secret.svg'), path.join(icons, 'escape.svg'))
    expect(resolveSharedIcon(icons, '/safe.svg?v=1')).toBe(realpathSync(path.join(icons, 'safe.svg')))
    for (const url of ['/../secret.svg', '/%2e%2e/secret.svg', '/..%2fsecret.svg', '/..%5csecret.svg', '/escape.svg', '/missing.svg', '/safe.svg%00', '/%invalid', '/safe.ts', '/']) {
      expect(resolveSharedIcon(icons, url), url).toBeNull()
    }
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('once receives only one of several already queued notifications', async () => {
  const values: unknown[] = []
  await once('unit-once', event => values.push(event.payload))
  await emit('unit-once', 1)
  await emit('unit-once', 2)
  await new Promise(resolve => setTimeout(resolve, 20))
  expect(values).toEqual([1])
})

test('unlisten cancels queued notifications without removing replacement listeners', async () => {
  const values: unknown[] = []
  const remove = await listen('unit-cleanup', event => values.push(event.payload))
  await emit('unit-cleanup', 'stale')
  remove()
  const removeNext = await listen('unit-cleanup', event => values.push(event.payload))
  await emit('unit-cleanup', 'current')
  await new Promise(resolve => setTimeout(resolve, 20))
  removeNext()
  expect(values).toEqual(['current'])
})

test('external links accept web URLs and reject executable schemes', () => {
  expect(isValidHttpUrl('https://example.com')).toBe(true)
  for (const url of ['javascript:alert(1)', 'data:text/html,hello', 'file:///etc/passwd', 'not a URL']) {
    expect(isValidHttpUrl(url)).toBe(false)
  }
})
