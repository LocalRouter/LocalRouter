/** Read captured chat payloads without confusing metadata or reasoning with an answer. */
export function contentText(content: unknown): string {
  if (typeof content === 'string') return content
  if (!Array.isArray(content)) return ''
  return content.map(block => {
    if (!block || typeof block !== 'object') return ''
    return typeof block.text === 'string' ? block.text : ''
  }).filter(Boolean).join('\n')
}

export function requestMessages(body: Record<string, unknown> | undefined): Record<string, unknown>[] {
  if (!body) return []
  if (Array.isArray(body.messages)) return body.messages.filter(isRecord)
  if (Array.isArray(body.input) && body.input.some(item => isRecord(item) && item.role)) {
    return body.input.filter(isRecord)
  }
  const input = body.prompt ?? body.input
  return typeof input === 'string' ? [{ role: 'user', content: input }] : []
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value != null && typeof value === 'object' && !Array.isArray(value)
}

/** Normalize display only; the original payload remains available verbatim. */
export function responseMessages(body: Record<string, unknown> | undefined): Record<string, unknown>[] {
  if (!body) return []
  if (Array.isArray(body.choices)) {
    return body.choices.filter(isRecord).flatMap(choice => {
      if (isRecord(choice.message)) return [choice.message]
      if (typeof choice.text === 'string') return [{ role: 'assistant', content: choice.text }]
      return []
    })
  }
  if (Array.isArray(body.output)) {
    return body.output.filter(isRecord).flatMap(item => {
      if (item.type === 'message') return [item]
      if (item.type === 'function_call') return [{ role: 'assistant', tool_calls: [{ id: item.call_id, function: { name: item.name, arguments: item.arguments } }] }]
      return []
    })
  }
  if (Array.isArray(body.content)) {
    const calls = body.content.filter(isRecord).filter(block => block.type === 'tool_use')
    return [{
      role: 'assistant', content: body.content,
      tool_calls: calls.map(block => ({ id: block.id, function: { name: block.name, arguments: block.input } })),
    }]
  }
  const text = body.output_text ?? body.text
  return typeof text === 'string' ? [{ role: 'assistant', content: text }] : []
}

/** Recover a full request from raw capture when its parsed body hit the size cap. */
export function capturedRequestBody(body: Record<string, unknown> | undefined, raw?: string): Record<string, unknown> | undefined {
  if (body && !body._truncated) return body
  return parseObject(raw) ?? body
}

export function capturedExcerpt(body: Record<string, unknown> | undefined, field: 'question' | 'answer'): string {
  if (!body?._truncated || !isRecord(body._monitor_preview)) return ''
  const text = body._monitor_preview[field]
  return typeof text === 'string' ? text : ''
}

function parseObject(raw?: string): Record<string, unknown> | undefined {
  if (!raw) return undefined
  try {
    const value: unknown = JSON.parse(raw)
    return isRecord(value) ? value : undefined
  } catch { return undefined }
}

function hasVisibleMessage(messages: Record<string, unknown>[]): boolean {
  return messages.some(message => contentText(message.content) || (Array.isArray(message.tool_calls) && message.tool_calls.length))
}

/** Older Codex captures may have an empty/truncated final body but complete
 * output items in the raw stream. Done snapshots replace deltas to avoid repeats.
 * Metadata and encrypted reasoning are never interpreted as assistant text. */
export function capturedResponseMessages(body: Record<string, unknown> | undefined, raw?: string): Record<string, unknown>[] {
  const messages = responseMessages(body)
  if (hasVisibleMessage(messages) || !raw) return messages
  const plain = responseMessages(parseObject(raw))
  if (hasVisibleMessage(plain)) return plain
  const items = new Map<number, Record<string, unknown>>()
  const parts = new Map<number, Map<number, Record<string, unknown>>>()
  for (const frame of raw.split(/\r?\n\r?\n/)) {
    const value = parseObject(frame.split(/\r?\n/).filter(line => line.startsWith('data:')).map(line => line.slice(5).trimStart()).join('\n'))
    if (!value || typeof value.type !== 'string') continue
    const index = typeof value.output_index === 'number' ? value.output_index : 0
    if (['response.completed', 'response.incomplete', 'response.failed'].includes(value.type) && isRecord(value.response)) {
      const terminal = responseMessages(value.response)
      if (hasVisibleMessage(terminal)) return terminal
    } else if (['response.output_item.added', 'response.output_item.done'].includes(value.type) && isRecord(value.item)) {
      items.set(index, value.item)
    } else if (['response.content_part.added', 'response.content_part.done', 'response.output_text.delta', 'response.output_text.done'].includes(value.type)) {
      const partIndex = typeof value.content_index === 'number' ? value.content_index : 0
      const itemParts = parts.get(index) ?? new Map<number, Record<string, unknown>>()
      parts.set(index, itemParts)
      if (isRecord(value.part)) itemParts.set(partIndex, value.part)
      else {
        const part = itemParts.get(partIndex) ?? { type: 'output_text', text: '' }
        if (value.type === 'response.output_text.done' && typeof value.text === 'string') part.text = value.text
        else if (typeof value.delta === 'string') part.text = String(part.text ?? '') + value.delta
        itemParts.set(partIndex, part)
      }
    }
  }
  for (const [index, itemParts] of parts) {
    const item = items.get(index) ?? { type: 'message', role: 'assistant' }
    const existing = new Map<number, Record<string, unknown>>()
    if (Array.isArray(item.content)) item.content.forEach((part, partIndex) => { if (isRecord(part)) existing.set(partIndex, part) })
    for (const [partIndex, part] of itemParts) {
      if (!existing.has(partIndex) || existing.get(partIndex)?.text === '') existing.set(partIndex, part)
    }
    item.content = [...existing].sort(([a], [b]) => a - b).map(([, part]) => part)
    items.set(index, item)
  }
  return responseMessages({ output: [...items].sort(([a], [b]) => a - b).map(([, item]) => item) })
}
