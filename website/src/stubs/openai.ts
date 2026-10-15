import { mockData } from '../components/demo/mockData'
import { mockUpstreamTools, mockClientPrivacy, mockClientSecurity, mockClientSecrets } from '../components/demo/mockClientPolicies'
// Stub for openai package in demo mode
// Bundled fixtures only; API operations without a demo fixture remain unavailable.

class DemoError extends Error {
  constructor() {
    super('This is a demo - API calls are not available. Download LocalRouter to try it out!')
    this.name = 'DemoError'
  }
}

// Mock streaming response that throws an error
async function* mockStream(): AsyncGenerator<any, void, unknown> {
  throw new DemoError()
}

// Mock chat completions
const mockChatCompletions = {
  create: async (_options: any) => {
    // If streaming, return an async iterator that throws
    if (_options?.stream) {
      return mockStream()
    }
    throw new DemoError()
  },
}

// Mock images
const mockImages = {
  generate: async () => {
    throw new DemoError()
  },
  edit: async () => {
    throw new DemoError()
  },
}

// Mock embeddings
const mockEmbeddings = {
  create: async () => {
    throw new DemoError()
  },
}

// Mock completions
const mockCompletions = {
  create: async () => {
    throw new DemoError()
  },
}

// Mock models
const mockModels = {
  list: async () => ({ object: 'list' as const, data: mockData.models.map(model => ({ id: `${model.provider}/${model.id}`, object: 'model' as const, owned_by: model.provider })) }),
}

// ---------------------------------------------------------------------------
// System One (POST /systemone): deterministic, realistic answers so the
// Try It Out "System One" tab works in the demo.
// ---------------------------------------------------------------------------

type ApiPromise<T> = Promise<T> & {
  withResponse: () => Promise<{ data: T; response: Response; request_id: string | null }>
  asResponse: () => Promise<Response>
}

// Mirrors the OpenAI SDK's APIPromise: awaitable for the body, with
// `.withResponse()` to also read the HTTP response (headers).
function apiPromise<T>(run: () => Promise<{ data: T; response: Response }>): ApiPromise<T> {
  const result = run()
  const promise = result.then(r => r.data) as ApiPromise<T>
  // Callers may only use withResponse(); don't report the body promise as unhandled.
  promise.catch(() => {})
  promise.withResponse = () => result.then(r => ({ ...r, request_id: null }))
  promise.asResponse = () => result.then(r => r.response)
  return promise
}

class DemoApiError extends Error {
  status: number
  constructor(status: number, message: string) {
    super(`${status} ${message}`)
    this.name = 'APIError'
    this.status = status
  }
}

// Model ids served natively by the demo's System One (decision) providers.
const NATIVE_DECISION_MODELS = new Set([
  'laya:en', 'laya:multilingual', 'english', 'multilingual', 'typed-decisions', 'kev-latest', 'jev-latest', 'jev-preview', 'jev-1.13.0',
])

function normalize(weights: number[]): number[] {
  const total = weights.reduce((a, b) => a + b, 0)
  return weights.map(w => Math.round((w / total) * 10000) / 10000)
}

// 1 - normalized entropy: 1 when certain, 0 when uniform.
function confidenceOf(probs: number[]): number {
  if (probs.length < 2) return 1
  const entropy = -probs.reduce((h, p) => (p > 0 ? h + p * Math.log(p) : h), 0)
  return Math.round((1 - entropy / Math.log(probs.length)) * 100) / 100
}

function answerQuestion(question: any): Record<string, unknown> {
  switch (question?.type) {
    case 'choice': {
      const keys = Object.keys(question.criteria ?? {})
      if (keys.length === 0) throw new DemoApiError(400, 'choice criteria must have at least one option')
      // First option clearly ahead, the rest tailing off.
      const probs = normalize(keys.map((_, i) => (i === 0 ? 6 : 1 / i)))
      return {
        type: 'choice',
        choice: keys[0],
        confidence: confidenceOf(probs),
        probabilities: Object.fromEntries(keys.map((k, i) => [k, probs[i]])),
      }
    }
    case 'score': {
      const levels: string[] = Array.isArray(question.criteria) ? question.criteria : []
      if (levels.length < 2 || levels.length > 10) throw new DemoApiError(400, 'score criteria must have 2-10 levels')
      // A bell curve centred two thirds of the way up the scale.
      const peak = (levels.length - 1) * 0.66
      const probs = normalize(levels.map((_, i) => Math.exp(-((i - peak) ** 2) / 1.2)))
      const score = Math.round(probs.reduce((acc, p, i) => acc + i * p, 0) * 100) / 100
      return {
        type: 'score',
        score,
        confidence: confidenceOf(probs),
        legend: Object.fromEntries(levels.map((l, i) => [String(i), l])),
        probabilities: Object.fromEntries(probs.map((p, i) => [String(i), p])),
      }
    }
    case 'noul':
      return { type: 'noul', noul: 0.91 }
    default:
      throw new DemoApiError(400, `unknown question type '${question?.type}'`)
  }
}

function systemOneResponse(body: any): { data: any; response: Response } {
  if (!body || typeof body !== 'object' || body.state === undefined) {
    throw new DemoApiError(400, 'state is required')
  }
  const questions = body.questions
  if (!questions || typeof questions !== 'object' || Object.keys(questions).length === 0) {
    throw new DemoApiError(400, 'questions must contain at least one question')
  }
  const model: string = typeof body.model === 'string' && body.model ? body.model : 'ollaya-local/laya:en'
  const bareModel = model.includes('/') ? model.slice(model.indexOf('/') + 1) : model
  const backend = NATIVE_DECISION_MODELS.has(bareModel) ? 'native' : 'letter_logprobs'
  const data = {
    model,
    answers: Object.fromEntries(Object.entries(questions).map(([id, q]) => [id, answerQuestion(q)])),
    usage: {
      input_tokens: Math.ceil(JSON.stringify(body).length / 4),
      output_tokens: Object.keys(questions).length,
    },
  }
  const response = new Response(JSON.stringify(data), {
    status: 200,
    headers: {
      'content-type': 'application/json',
      'x-localrouter-systemone-backend': backend,
      'x-localrouter-generation-id': 'gen-demo-systemone',
    },
  })
  return { data, response }
}

// Mock OpenAI class
class OpenAI {
  chat = {
    completions: mockChatCompletions,
  }
  images = mockImages
  embeddings = mockEmbeddings
  completions = mockCompletions
  models = mockModels
  baseURL: string
  apiKey: string

  constructor(config?: any) {
    this.baseURL = config?.baseURL ?? 'http://localhost:3625/v1'
    this.apiKey = config?.apiKey ?? 'demo-key'
  }

  // Generic request method. System One and ranked search use local fixtures.
  post<T = any>(path: string, opts?: { body?: unknown; signal?: AbortSignal; maxRetries?: number }): ApiPromise<T> {
    return apiPromise<T>(async () => {
      if (opts?.signal?.aborted) throw new DOMException('Cancelled', 'AbortError')
      if (path.replace(/\/+$/, '') === '/chat/completions') {
        const clientId = [...mockClientSecrets.entries()].find(([, secret]) => secret === this.apiKey)?.[0]
        const client = mockData.clients.find(client => client.id === clientId)
        const security = clientId ? mockClientSecurity.get(clientId) : null
        if (!client?.enabled || security?.hasCredential === false || (security?.expiry && Date.now() >= Date.parse(security.expiry))) throw new DemoApiError(401, 'Demo client credential is inactive')
        const body = opts?.body as { model?: string; extensions?: Record<string, unknown> } | undefined
        if (!body?.extensions?.['localrouter.documents']) throw new DemoError()
        const data = { id: 'document-demo', model: body.model, choices: [{ message: { role: 'assistant', content: 'Illustrative document response. The demo does not send or analyze the selected file.' }, finish_reason: 'stop', index: 0 }], extensions: { 'localrouter.document_processing': { demo: true, cost_usd: null, input_usage_reported: false } } }
        return { data: data as T, response: new Response(JSON.stringify(data), { headers: { 'content-type': 'application/json' } }) }
      }
      if (path.replace(/\/+$/, '') === '/search') {
        const clientId = [...mockClientSecrets.entries()].find(([, secret]) => secret === this.apiKey)?.[0]
        const client = mockData.clients.find(client => client.id === clientId)
        const security = clientId ? mockClientSecurity.get(clientId) : null
        if (!client?.enabled || security?.hasCredential === false || (security?.expiry && Date.now() >= Date.parse(security.expiry))) throw new DemoApiError(401, 'Demo client credential is inactive')
        const policy = mockUpstreamTools.get(client.id)
        if (!policy?.allowed_tools.includes('web_search')) throw new DemoApiError(400, 'Enable web search for this demo client in Settings')
        const privacy = mockClientPrivacy.get(client.id)
        if (privacy?.local_only || privacy?.no_retention) throw new DemoApiError(400, 'Strict privacy does not permit this upstream search chain')

        const body = opts?.body as { query?: unknown; model?: string; search_type?: unknown } | undefined
        if (!body || typeof body.query !== 'string' || !body.query.trim()) throw new DemoApiError(400, 'Provide a nonempty search query')
        const model = body.model || policy.default_search_model
        if (!model) throw new DemoApiError(400, 'Provide a qualified search model or configure a client search default')
        if (body.search_type !== undefined && !['web', 'fast', 'people'].includes(String(body.search_type))) throw new DemoApiError(400, 'Unsupported search type')
        const downstream = model.slice(model.indexOf('/') + 1)
        const mode = body.search_type ?? (downstream === 'perplexity/perplexity-search-fast' ? 'fast' : 'web')
        if (downstream !== 'search' && (mode === 'people' || downstream !== `perplexity/perplexity-search${mode === 'fast' ? '-fast' : ''}`)) throw new DemoApiError(400, 'Search model must match the supported search type')
        const data = {
          id: 'search-demo', model,
          results: [{ title: 'LocalRouter repository (demo result)', url: 'https://github.com/LocalRouter/LocalRouter', snippet: 'Illustrative local demo result. This demo does not run a live web search.' }],
          extensions: { 'localrouter.search': { demo: true, billing_unit: 'request', upstream_requests: 0, cost_usd: null, cost_source: 'unknown' } },
        }
        return { data: data as T, response: new Response(JSON.stringify(data), { headers: { 'content-type': 'application/json' } }) }
      }
      if (path.replace(/\/+$/, '') !== '/systemone') throw new DemoError()
      // Roughly a local decision model's latency.
      await new Promise(resolve => setTimeout(resolve, 350))
      return systemOneResponse(opts?.body) as { data: T; response: Response }
    })
  }
}

export default OpenAI
export { OpenAI }
