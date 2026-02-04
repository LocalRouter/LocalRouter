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

// Mock OpenAI class
class OpenAI {
  chat = {
    completions: mockChatCompletions,
  }
  images = mockImages
  embeddings = mockEmbeddings
  completions = mockCompletions
  models = mockModels

  constructor(_config?: any) {
    // Accept config but don't do anything with it
  }
}

export default OpenAI
export { OpenAI }
