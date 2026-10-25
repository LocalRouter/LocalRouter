import { expect, test } from '@playwright/test'
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks'
import { discoverResourceIcon, resourceIconOrigin, renameResourceIcon, forgetResourceIcon } from '../../../src/utils/resource-icons'

// The cache persists image data and origins, never endpoint paths or secrets.
test.beforeEach(() => {
  const values = new Map<string, string>()
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
  } })
  Object.defineProperty(globalThis, 'window', { configurable: true, value: {} })
})
test.afterEach(() => clearMocks())

test('normalizes endpoint origins and rejects command/file/data URLs', () => {
  expect(resourceIconOrigin('https://user:secret@example.com:8443/v1?token=secret')).toBe('https://example.com:8443')
  for (const url of [undefined, 'npx server', 'file:///tmp/icon', 'data:image/png,test', 'javascript:alert(1)']) {
    expect(resourceIconOrigin(url)).toBeUndefined()
  }
})

test('discovers once per origin and never passes or persists API secrets', async () => {
  const calls: unknown[] = []
  mockIPC((command, args) => {
    calls.push([command, args])
    return 'data:image/png;base64,test'
  })
  await discoverResourceIcon('provider:cached', 'https://user:secret@example.com/private?token=hidden')
  await discoverResourceIcon('provider:cached', 'https://example.com/v2')
  expect(calls).toEqual([['discover_resource_icon', { endpointUrl: 'https://example.com' }]])
  const stored = localStorage.getItem('localrouter-resource-icons-v1')!
  expect(stored).toContain('data:image/png;base64,test')
  expect(stored).not.toMatch(/secret|private|token|hidden/)
})

test('a stale lookup cannot replace an updated endpoint or a local transport', async () => {
  let completeOld!: (value: string) => void
  mockIPC((_command, args) => args?.endpointUrl === 'https://old.example'
    ? new Promise<string>(resolve => { completeOld = resolve })
    : 'data:image/png;base64,new')
  const first = discoverResourceIcon('mcp:race', 'https://old.example/mcp')
  await discoverResourceIcon('mcp:race', 'https://new.example/mcp')
  completeOld('data:image/png;base64,old')
  await first
  expect(localStorage.getItem('localrouter-resource-icons-v1')).toContain('base64,new')
  expect(localStorage.getItem('localrouter-resource-icons-v1')).not.toContain('old.example')
  await discoverResourceIcon('mcp:race')
  expect(localStorage.getItem('localrouter-resource-icons-v1')).not.toContain('mcp:race')
})

test('missing favicon and IPC errors do not fail resource creation', async () => {
  mockIPC(() => null)
  await expect(discoverResourceIcon('mcp:none', 'https://no-icon.example')).resolves.toBeUndefined()
  mockIPC(() => { throw new Error('offline') })
  await expect(discoverResourceIcon('mcp:offline', 'https://offline.example')).resolves.toBeUndefined()
})


test('renaming preserves cached branding and deleting removes it', async () => {
  mockIPC(() => 'data:image/png;base64,rename')
  await discoverResourceIcon('provider:before', 'https://rename.example')
  renameResourceIcon('provider:before', 'provider:after')
  expect(localStorage.getItem('localrouter-resource-icons-v1')).toContain('provider:after')
  expect(localStorage.getItem('localrouter-resource-icons-v1')).not.toContain('provider:before')
  forgetResourceIcon('provider:after')
  expect(localStorage.getItem('localrouter-resource-icons-v1')).not.toContain('provider:after')
})
