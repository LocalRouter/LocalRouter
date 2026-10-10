import type { LlmApi, LlmProtocol } from '../../types/tauri-commands'

export const LLM_API_LABELS: Record<LlmApi, string> = {
  chat_completions: 'Chat Completions',
  completions: 'Completions (legacy)',
  responses: 'Responses',
  anthropic_messages: 'Anthropic Messages',
  gemini_generate_content: 'Gemini generateContent',
  cohere_chat: 'Cohere Chat',
  ollama_chat: 'Ollama chat',
  system_one: 'System One',
  embeddings: 'Embeddings',
  moderations: 'Moderations',
  images: 'Images',
  audio: 'Audio',
}

/** Compact labels for the event list. */
export const LLM_API_SHORT: Record<LlmApi, string> = {
  chat_completions: 'Chat',
  completions: 'Completions',
  responses: 'Responses',
  anthropic_messages: 'Messages',
  gemini_generate_content: 'Gemini',
  cohere_chat: 'Cohere',
  ollama_chat: 'Ollama',
  system_one: 'System One',
  embeddings: 'Embeddings',
  moderations: 'Moderation',
  images: 'Images',
  audio: 'Audio',
}

/** Mirror of `LlmApi::from_path` for events recorded before the API was stored. */
export function apiFromPath(path: string | null | undefined): LlmApi | null {
  if (!path) return null
  const p = path.split('?')[0].replace(/\/+$/, '')
  if (p.endsWith('/chat/completions')) return 'chat_completions'
  if (p.endsWith('/completions')) return 'completions'
  if (p.endsWith('/responses') || p.includes('/responses/')) return 'responses'
  if (p.endsWith('/messages')) return 'anthropic_messages'
  if (p.includes(':generateContent') || p.includes(':streamGenerateContent')) return 'gemini_generate_content'
  if (p.endsWith('/v2/chat')) return 'cohere_chat'
  if (p.endsWith('/api/chat') || p.endsWith('/api/generate')) return 'ollama_chat'
  if (p.endsWith('/systemone')) return 'system_one'
  if (p.endsWith('/embeddings')) return 'embeddings'
  if (p.endsWith('/moderations')) return 'moderations'
  if (p.includes('/images/')) return 'images'
  if (p.includes('/audio/')) return 'audio'
  return null
}

export interface ApiFlow {
  /** The API the client called. */
  client: LlmApi | null
  /** The API LocalRouter spoke upstream; unknown until a provider answers. */
  upstream: LlmApi | null
  /** Both sides are known and differ. */
  translated: boolean
}

export function llmApiFlow(data: {
  client_api?: LlmApi | null
  upstream_api?: LlmApi | null
  endpoint?: string | null
  protocol?: LlmProtocol | null
}): ApiFlow {
  const client =
    data.client_api ??
    apiFromPath(data.endpoint) ??
    (data.protocol === 'anthropic' ? 'anthropic_messages' : data.protocol === 'system_one' ? 'system_one' : null)
  const upstream = data.upstream_api ?? null
  return { client, upstream, translated: client != null && upstream != null && client !== upstream }
}
