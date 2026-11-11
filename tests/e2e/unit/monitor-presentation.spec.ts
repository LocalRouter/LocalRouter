import { codexAnswer, codexRequest, codexStream, codexTruncated } from '../monitor/codex-fixture'
import { test, expect } from '@playwright/test'
import { capturedExcerpt, capturedRequestBody, capturedResponseMessages, contentText, requestMessages, responseMessages } from '../../../src/views/monitor/message-content'
import { eventDurationMs, matchesFilter, mergeMonitorEvents, showEventTypeColumn, singleLinePreview } from '../../../src/views/monitor/monitor-events'
import type { MonitorEventFilter, MonitorEventSummary } from '../../../src/types/tauri-commands'

const event: MonitorEventSummary = {
  id: 'monitor', sequence: 1, timestamp: '2026-10-02T12:00:00Z', event_type: 'llm_call',
  status: 'pending', summary: 'openai/model', question: 'How does retry work?', answer: '',
  session_id: null, client_id: null, client_name: null, duration_ms: null,
}
const filter = (fields: Partial<MonitorEventFilter>): MonitorEventFilter => ({
  event_types: null, session_id: null, client_id: null, status: null, search: null, ...fields,
})

test('pending durations increase from their start time and terminal durations stay fixed', () => {
  const start = Date.parse(event.timestamp)
  expect(eventDurationMs(event, start)).toBe(0)
  expect(eventDurationMs(event, start + 1250)).toBe(1250)
  expect(eventDurationMs({ ...event, duration_ms: 100 }, start + 2500)).toBe(2500)
  for (const status of ['complete', 'error'] as const) {
    expect(eventDurationMs({ ...event, status, duration_ms: 1234 }, start + 9000)).toBe(1234)
    expect(eventDurationMs({ ...event, status }, start + 9000)).toBeNull()
  }
})

test('future and invalid start timestamps do not display negative or NaN durations', () => {
  expect(eventDurationMs(event, Date.parse(event.timestamp) - 1000)).toBe(0)
  expect(eventDurationMs({ ...event, timestamp: 'invalid' }, Date.now())).toBeNull()
  expect(eventDurationMs({ ...event, timestamp: 'invalid', duration_ms: 50 }, Date.now())).toBe(50)
})

test('type column follows enabled types, including empty and all selections', () => {
  expect(showEventTypeColumn(null)).toBe(true)
  expect(showEventTypeColumn([])).toBe(false)
  expect(showEventTypeColumn(['llm_call'])).toBe(false)
  expect(showEventTypeColumn(['llm_call', 'mcp_tool_call'])).toBe(true)
  expect(matchesFilter(event, filter({ event_types: [] }))).toBe(false)
})

test('Secret Scan keeps redaction and restoration visible under one feature filter', () => {
  const redacted: MonitorEventSummary = { ...event, event_type: 'secret_scan', secret_action: 'redact', status: 'complete', summary: 'Secret redacted', answer: 'redact' }
  const restored: MonitorEventSummary = { ...redacted, id: 'restored', sequence: 2, secret_action: 'restore', summary: 'Secret restored', answer: 'restore' }
  const warning: MonitorEventSummary = { ...restored, id: 'warning', sequence: 3, secret_action: 'restore_warning', status: 'error', summary: 'Secret Not Restored', answer: 'response delivered' }
  expect(matchesFilter(warning, filter({ event_types: ['secret_scan'], status: 'error' }))).toBe(true)
  expect(showEventTypeColumn(['secret_scan'])).toBe(true)
  expect(mergeMonitorEvents([redacted], [restored], filter({ event_types: ['secret_scan'] }))).toEqual([restored, redacted])
  expect(matchesFilter(restored, filter({ event_types: ['llm_call'] }))).toBe(false)
})

test('preview whitespace is flattened and a completed answer enters a search filter', () => {
  expect(singleLinePreview('  First\n\r\tsecond  ')).toBe('First second')
  expect(singleLinePreview('')).toBe('—')
  expect(matchesFilter(event, filter({ search: 'RETRY' }))).toBe(true)
  const complete = { ...event, status: 'complete' as const, answer: 'Use backoff.' }
  expect(mergeMonitorEvents([event], [complete], filter({ search: 'BACKOFF' }))).toEqual([complete])
})

test('request bodies support chat, responses, and plain completion prompts', () => {
  const messages = [{ role: 'system', content: 'Context' }, { role: 'user', content: 'Question' }]
  expect(requestMessages({ messages })).toEqual(messages)
  expect(requestMessages({ input: messages })).toEqual(messages)
  expect(requestMessages({ prompt: 'Complete this' })).toEqual([{ role: 'user', content: 'Complete this' }])
  expect(requestMessages(undefined)).toEqual([])
})

test('full responses retain text and tools for OpenAI, Anthropic, and Responses', () => {
  const message = { role: 'assistant', content: 'Full answer', tool_calls: [{ function: { name: 'lookup', arguments: '{}' } }] }
  expect(responseMessages({ choices: [{ message }] })).toEqual([message])
  const anthropic = responseMessages({ content: [
    { type: 'thinking', thinking: 'Reasoning' },
    { type: 'text', text: 'Actual answer' },
    { type: 'tool_use', id: 'call-1', name: 'lookup', input: { query: 'test' } },
  ] })
  expect(contentText(anthropic[0].content)).toBe('Actual answer')
  expect(anthropic[0].tool_calls).toEqual([{ id: 'call-1', function: { name: 'lookup', arguments: { query: 'test' } } }])
  const responses = responseMessages({ output: [
    { type: 'message', role: 'assistant', content: [{ type: 'output_text', text: 'Answer' }] },
    { type: 'function_call', call_id: 'call-2', name: 'lookup', arguments: '{}' },
  ] })
  expect(contentText(responses[0].content)).toBe('Answer')
  expect(responses[1].tool_calls).toEqual([{ id: 'call-2', function: { name: 'lookup', arguments: '{}' } }])
})

test('absent and non-text payloads do not turn into fabricated answers', () => {
  expect(responseMessages(undefined)).toEqual([])
  expect(responseMessages({ error: { message: 'Failed' } })).toEqual([])
  expect(contentText([{ type: 'image', source: 'image bytes' }, null])).toBe('')
  expect(responseMessages({ choices: [{ text: 'Completion' }] })).toEqual([{ role: 'assistant', content: 'Completion' }])
})


test('Codex lite raw captures recover the answer once despite empty terminal output', () => {
  for (const body of [undefined, codexTruncated, { error: null, output: [] }]) {
    const messages = capturedResponseMessages(body, codexStream)
    expect(messages).toHaveLength(1)
    expect(contentText(messages[0].content)).toBe(codexAnswer)
  }
  const messages = requestMessages(capturedRequestBody(codexTruncated, JSON.stringify(codexRequest)))
  expect(contentText(messages[0].content)).toBe('Assess this synthetic request.')
})

test('raw fallback tolerates partial frames and preserves ordered content without metadata', () => {
  const raw = [
    { type: 'response.output_item.added', output_index: 4, item: { type: 'message', role: 'assistant', content: [{ type: 'output_text', text: '' }] } },
    { type: 'response.output_text.delta', output_index: 4, content_index: 1, delta: 'second' },
    { type: 'response.output_text.delta', output_index: 4, content_index: 0, delta: 'first' },
    { type: 'response.output_text.done', output_index: 4, content_index: 0, text: 'first complete' },
    { type: 'response.reasoning_summary_text.delta', delta: 'Do not show as answer' },
  ].map(event => `data: ${JSON.stringify(event)}\r\n\r\n`).join('') + 'data: {broken'
  expect(contentText(capturedResponseMessages(undefined, raw)[0].content)).toBe('first complete\nsecond')
  expect(capturedResponseMessages({ error: null, output: [] }, 'data: {broken')).toEqual([])
  expect(capturedRequestBody(codexTruncated, '{broken')).toBe(codexTruncated)
  expect(capturedResponseMessages({ output_text: 'Complete body' }, codexStream)[0].content).toBe('Complete body')
  expect(capturedResponseMessages(undefined, JSON.stringify({ output_text: 'Plain response' }))[0].content).toBe('Plain response')
})

test('bounded excerpts remain useful without a complete raw capture', () => {
  const body = { ...codexTruncated, _monitor_preview: { question: 'Question excerpt', answer: 'Answer excerpt' } }
  expect(capturedExcerpt(body, 'question')).toBe('Question excerpt')
  expect(capturedExcerpt(body, 'answer')).toBe('Answer excerpt')
  expect(capturedExcerpt(undefined, 'answer')).toBe('')
  expect(capturedExcerpt({ output_text: 'Complete body' }, 'answer')).toBe('')
})


test('storage outage causes and recovery stay searchable under Secret Scan', () => {
  const outage: MonitorEventSummary = { ...event, event_type: 'secret_scan', secret_action: 'storage_warning', status: 'error', question: 'Secret storage write failed: No space left on device (os error 28)', summary: 'Secret Storage Unavailable', answer: 'storage_warning' }
  const recovered: MonitorEventSummary = { ...outage, id: 'recovered', sequence: 2, status: 'complete', secret_action: 'storage_recovered', question: 'Pending mappings and deletions are durable', summary: 'Secret Storage Recovered', answer: 'storage_recovered' }
  expect(matchesFilter(outage, filter({ event_types: ['secret_scan'], search: 'os error 28', status: 'error' }))).toBe(true)
  expect(mergeMonitorEvents([outage], [recovered], filter({ event_types: ['secret_scan'] }))).toEqual([recovered, outage])
  expect(matchesFilter(recovered, filter({ status: 'error' }))).toBe(false)
})

test('standalone Codex search displays commands alongside optional conversation input', () => {
  const commands = { search_query: [{ q: 'synthetic query' }], find: [{ ref_id: 'turn0search0', pattern: 'example' }] }
  for (const input of [undefined, 'conversation context', [{ type: 'message', role: 'user', content: 'conversation context' }]]) {
    const body = { id: 'search-session', model: 'test', commands, input }
    const original = JSON.stringify(body)
    const messages = requestMessages(body)
    expect(contentText(messages[messages.length - 1].content)).toContain('synthetic query')
    expect(contentText(messages[messages.length - 1].content)).toContain('turn0search0')
    if (input) expect(contentText(messages[0].content)).toBe('conversation context')
    expect(JSON.stringify(body)).toBe(original)
  }
  expect(contentText(responseMessages({ encrypted_output: 'opaque-state', output: 'Search answer' })[0].content)).toBe('Search answer')
})
