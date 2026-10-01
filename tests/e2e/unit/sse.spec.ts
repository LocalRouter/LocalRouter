import { test, expect } from '@playwright/test'
import { readSseData } from '../../../src/lib/sse'

function streamOf(chunks: Uint8Array[]) {
  return new ReadableStream<Uint8Array>({ start(controller) {
    for (const chunk of chunks) controller.enqueue(chunk)
    controller.close()
  } })
}

test('SSE parsing preserves split UTF-8 and CRLF frames across arbitrary bytes', async () => {
  const bytes = new TextEncoder().encode('event: delta\r\ndata: {"text":"🎉"}\r\n\r\ndata: next\n\n')
  const stream = streamOf([...bytes].map(byte => new Uint8Array([byte])))
  const messages = []
  for await (const data of readSseData(stream)) messages.push(data)
  expect(messages).toEqual(['{"text":"🎉"}', 'next'])
  expect(stream.locked).toBe(false)
})

test('SSE joins multiline data, ignores comments, and preserves significant spaces', async () => {
  const stream = streamOf([new TextEncoder().encode(': ping\n\ndata: first\ndata:  second \n\n')])
  const messages = []
  for await (const data of readSseData(stream)) messages.push(data)
  expect(messages).toEqual(['first\n second '])
})

test('stopping an SSE consumer cancels and unlocks the underlying stream', async () => {
  let cancelled = false
  const stream = new ReadableStream<Uint8Array>({
    start(controller) { controller.enqueue(new TextEncoder().encode('data: first\n\n')) },
    cancel() { cancelled = true },
  })
  for await (const data of readSseData(stream)) {
    expect(data).toBe('first')
    break
  }
  expect(cancelled).toBe(true)
  expect(stream.locked).toBe(false)
})
