import { test, expect } from '@playwright/test'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { McpClientWrapper } from '../../../src/lib/mcp-client'

// Stub the SDK boundary; no transport is ever started and no request is sent.
const original = {
  connect: Client.prototype.connect,
  close: Client.prototype.close,
  listTools: Client.prototype.listTools,
  listResources: Client.prototype.listResources,
  listPrompts: Client.prototype.listPrompts,
}
let connected: Client[]
let closed: Client[]
test.beforeEach(() => {
  connected = []
  closed = []
  Client.prototype.connect = async function () { connected.push(this) }
  Client.prototype.close = async function () { closed.push(this); this.onclose?.() }
})
test.afterEach(() => { Object.assign(Client.prototype, original) })
const wrapper = () => new McpClientWrapper({ serverPort: 3625, clientToken: 'unit-test-only' })

test('reads every tools page, including an empty intermediate cursor', async () => {
  const client = wrapper()
  await client.connect()
  const cursors: (string | undefined)[] = []
  Client.prototype.listTools = async args => {
    cursors.push(args?.cursor)
    return args?.cursor === undefined
      ? { tools: [{ name: 'first', inputSchema: { type: 'object' } }], nextCursor: '' }
      : { tools: [{ name: 'second', inputSchema: { type: 'object' } }] }
  }
  expect((await client.listTools()).map(tool => tool.name)).toEqual(['first', 'second'])
  expect(cursors).toEqual([undefined, ''])
  await client.disconnect()
})

test('reads every resources and prompts page', async () => {
  const client = wrapper()
  await client.connect()
  Client.prototype.listResources = async args => args?.cursor
    ? { resources: [{ uri: 'test://two', name: 'two' }] }
    : { resources: [{ uri: 'test://one', name: 'one' }], nextCursor: 'next' }
  Client.prototype.listPrompts = async args => args?.cursor
    ? { prompts: [{ name: 'two' }] }
    : { prompts: [{ name: 'one' }], nextCursor: 'next' }
  expect((await client.listResources()).map(resource => resource.name)).toEqual(['one', 'two'])
  expect((await client.listPrompts()).map(prompt => prompt.name)).toEqual(['one', 'two'])
  await client.disconnect()
})

test('repeated pagination cursors fail instead of looping forever', async () => {
  const client = wrapper()
  await client.connect()
  Client.prototype.listTools = async () => ({ tools: [], nextCursor: 'same' })
  await expect(client.listTools()).rejects.toThrow('repeated pagination cursor')
  await client.disconnect()
})

test('failed handshakes close the SDK client and allow a clean retry', async () => {
  Client.prototype.connect = async function () { connected.push(this); throw new Error('handshake failed') }
  const client = wrapper()
  await expect(client.connect()).rejects.toThrow('handshake failed')
  expect(closed).toContain(connected[0])
  expect(client.getState()).toMatchObject({ isConnected: false, isConnecting: false, error: 'handshake failed' })
  Client.prototype.connect = async function () { connected.push(this) }
  await client.connect()
  expect(client.getState().isConnected).toBe(true)
  await client.disconnect()
})

test('a cancelled handshake cannot overwrite a newer connection', async () => {
  let finishFirst!: () => void
  const firstPending = new Promise<void>(resolve => { finishFirst = resolve })
  Client.prototype.connect = async function () {
    connected.push(this)
    if (connected.length === 1) await firstPending
  }
  const client = wrapper()
  const first = client.connect()
  await client.disconnect()
  await client.connect()
  finishFirst()
  await first
  expect(client.getState()).toMatchObject({ isConnected: true, isConnecting: false, error: null })
  expect(closed).not.toContain(connected[1])
  await client.disconnect()
})

test('transport closure clears connected state and stale capabilities', async () => {
  const client = wrapper()
  await client.connect()
  connected[0].onclose?.()
  expect(client.getState()).toMatchObject({ isConnected: false, isConnecting: false })
  expect(client.getState().capabilities).toBeUndefined()
  await expect(client.listTools()).rejects.toThrow('Not connected')
})
