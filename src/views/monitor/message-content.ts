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
