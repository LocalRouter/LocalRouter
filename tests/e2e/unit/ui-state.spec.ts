import { expect, test } from '@playwright/test'
import { UiStateStore, type UiStateBackend, type UiStateMirror } from '../../../src/lib/ui-state-store'
import { decodeCollapsed, decodeDetailSize, decodeDock } from '../../../src/views/dashboard/section-state'

class FakeBackend implements UiStateBackend {
  saves: [string, unknown][] = []
  failLoad = false
  failSave = false
  constructor(public data: Record<string, unknown> = {}) {}
  async load() {
    if (this.failLoad) throw new Error('no backend')
    return { ...this.data }
  }
  async save(key: string, value: unknown) {
    if (this.failSave) throw new Error('write failed')
    this.saves.push([key, value])
    this.data[key] = value
  }
}

class FakeMirror implements UiStateMirror {
  constructor(public items: Record<string, string> = {}) {}
  getItem(key: string) {
    return key in this.items ? this.items[key] : null
  }
  setItem(key: string, value: string) {
    this.items[key] = value
  }
}

test('the backend value is the source of truth and is mirrored to localStorage', async () => {
  const backend = new FakeBackend({ 'traffic.collapsed': true })
  const mirror = new FakeMirror({ 'traffic.collapsed': '0' })
  const store = new UiStateStore(backend, mirror)
  await store.init()
  expect(store.get('traffic.collapsed')).toBe(true)
  expect(mirror.items['traffic.collapsed']).toBe('true')
  expect(backend.saves).toEqual([])
})

test('a legacy localStorage value missing from the backend is adopted and written through', async () => {
  const backend = new FakeBackend()
  const mirror = new FakeMirror({
    'localrouter.dashboard.traffic.collapsed': '1',
    'monitor.detailDock': 'right',
    'monitor.filter': '{"status":"error","search":null,"event_types":null}',
  })
  const store = new UiStateStore(backend, mirror)
  await store.init()
  expect(decodeCollapsed(store.get('localrouter.dashboard.traffic.collapsed'))).toBe(true)
  expect(decodeDock(store.get('monitor.detailDock'))).toBe('right')
  expect(store.get('monitor.filter')).toEqual({ status: 'error', search: null, event_types: null })
  expect(store.get('never.saved')).toBeUndefined()
  await store.whenIdle()
  expect(backend.data).toEqual({
    'localrouter.dashboard.traffic.collapsed': 1,
    'monitor.detailDock': 'right',
    'monitor.filter': { status: 'error', search: null, event_types: null },
  })
  // Adopted once: reading again sends nothing new.
  store.get('monitor.detailDock')
  await store.whenIdle()
  expect(backend.saves).toHaveLength(3)
})

test('writes go to both stores, skip unchanged values and notify subscribers', async () => {
  const backend = new FakeBackend()
  const mirror = new FakeMirror()
  const store = new UiStateStore(backend, mirror)
  await store.init()
  let notified = 0
  const unsubscribe = store.subscribe('k', () => notified++)
  store.set('k', true)
  store.set('k', true)
  await store.whenIdle()
  expect(backend.saves).toEqual([['k', true]])
  expect(mirror.items.k).toBe('true')
  expect(notified).toBe(1)
  unsubscribe()
  store.set('k', false)
  expect(notified).toBe(1)
  await store.whenIdle()
  expect(backend.data.k).toBe(false)
})

test('rapid writes reach the backend in order and coalesce to the latest value', async () => {
  const backend = new FakeBackend()
  const store = new UiStateStore(backend, null)
  await store.init()
  store.set('a', 1)
  store.set('a', 2)
  store.set('b', 'x')
  store.set('a', 3)
  await store.whenIdle()
  expect(backend.data).toEqual({ a: 3, b: 'x' })
  // Queued writes for one key collapse to its latest value.
  expect(backend.saves).toEqual([['a', 3], ['b', 'x']])
  // While a write is in flight, later values queue behind it.
  store.set('a', 4)
  store.set('a', 5)
  await store.whenIdle()
  expect(backend.saves.slice(2).map(([, value]) => value).at(-1)).toBe(5)
  expect(backend.data.a).toBe(5)
})

test('a value set while the backend map loads is not overwritten by it', async () => {
  const backend = new FakeBackend({ k: 'old' })
  const store = new UiStateStore(backend, null)
  store.set('k', 'new')
  await store.init()
  await store.whenIdle()
  expect(store.get('k')).toBe('new')
  expect(backend.data.k).toBe('new')
})

test('reads before the backend loads come from localStorage and are stable', async () => {
  const backend = new FakeBackend({ f: { a: 1 } })
  const mirror = new FakeMirror({ f: '{"a":0}' })
  const store = new UiStateStore(backend, mirror)
  const first = store.get('f')
  expect(first).toEqual({ a: 0 })
  expect(store.get('f')).toBe(first)
  let notified = 0
  store.subscribe('f', () => notified++)
  await store.init()
  expect(notified).toBe(1)
  expect(store.get('f')).toEqual({ a: 1 })
})

test('when the backend is unavailable localStorage keeps working', async () => {
  const backend = new FakeBackend()
  backend.failLoad = true
  const mirror = new FakeMirror({ k: '1' })
  const store = new UiStateStore(backend, mirror)
  await store.init()
  expect(store.get('k')).toBe(1)
  store.set('k', 0)
  await store.whenIdle()
  expect(mirror.items.k).toBe('0')
  expect(backend.saves).toEqual([])
})

test('a slow backend load times out instead of blocking startup', async () => {
  const backend: UiStateBackend = { load: () => new Promise(() => {}), save: async () => {} }
  const store = new UiStateStore(backend, new FakeMirror({ k: 'true' }), 20)
  await store.init()
  expect(store.get('k')).toBe(true)
})

test('a failed backend write keeps the value in memory and localStorage', async () => {
  const backend = new FakeBackend()
  backend.failSave = true
  const mirror = new FakeMirror()
  const store = new UiStateStore(backend, mirror)
  await store.init()
  store.set('k', 'v')
  await store.whenIdle()
  expect(store.get('k')).toBe('v')
  expect(mirror.items.k).toBe('"v"')
})

test('section decoders accept current and legacy values and reject the rest', () => {
  expect(decodeCollapsed(true)).toBe(true)
  expect(decodeCollapsed('0')).toBe(false)
  expect(decodeCollapsed('maybe')).toBeUndefined()
  expect(decodeDock('bottom')).toBe('bottom')
  expect(decodeDock('left')).toBeUndefined()
  expect(decodeDetailSize(42)).toBe(42)
  expect(decodeDetailSize('60')).toBe(60)
  expect(decodeDetailSize(10)).toBeUndefined()
  expect(decodeDetailSize('')).toBeUndefined()
})
