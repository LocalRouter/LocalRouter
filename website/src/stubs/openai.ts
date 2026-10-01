// Stub for openai package in demo mode
// Always throws a demo error when trying to make API calls

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
  list: async () => {
    throw new DemoError()
  },
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

  // Generic request method (the real SDK's `client.post`). Only System One is
  // simulated; everything else behaves like the other demo stubs.
  post<T = any>(path: string, opts?: { body?: unknown }): ApiPromise<T> {
    return apiPromise<T>(async () => {
      if (path.replace(/\/+$/, '') !== '/systemone') throw new DemoError()
      // Roughly a local decision model's latency.
      await new Promise(resolve => setTimeout(resolve, 350))
      return systemOneResponse(opts?.body) as { data: T; response: Response }
    })
  }
}

export default OpenAI
export { OpenAI }
