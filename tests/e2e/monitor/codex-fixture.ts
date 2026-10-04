// Protocol-shaped synthetic capture: no real prompts, identities, or credentials.
export const codexAnswer = '{"outcome":"allow","rationale":"Synthetic assessment"}'
export const codexRequest = {
  type: 'response.create', model: 'codex-auto-review',
  input: [{ type: 'message', role: 'user', content: [{ type: 'input_text', text: 'Assess this synthetic request.' }] }],
}
export const codexStream = [
  { type: 'codex.rate_limits', rate_limits: { allowed: true } },
  { type: 'response.created', response: { id: 'resp_test', output: [], error: null } },
  { type: 'response.output_item.done', output_index: 0, item: { type: 'reasoning', summary: [], encrypted_content: 'synthetic-ciphertext' } },
  { type: 'response.output_item.added', output_index: 1, item: { type: 'message', role: 'assistant', content: [] } },
  { type: 'response.output_text.delta', output_index: 1, content_index: 0, delta: 'incomplete' },
  { type: 'response.output_text.done', output_index: 1, content_index: 0, text: codexAnswer },
  { type: 'response.content_part.done', output_index: 1, content_index: 0, part: { type: 'output_text', text: codexAnswer } },
  { type: 'response.output_item.done', output_index: 1, item: { type: 'message', role: 'assistant', content: [{ type: 'output_text', text: codexAnswer }] } },
  { type: 'response.completed', response: { id: 'resp_test', status: 'completed', output: [], error: null, usage: { input_tokens: 100, output_tokens: 20 } } },
].map(event => `data: ${JSON.stringify(event)}\n\n`).join('')
export const codexTruncated = { _truncated: true, _original_size: 73_215, _preview: '{"id":"resp_test","error":null,"output":[],' }
