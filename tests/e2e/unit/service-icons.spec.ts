import { expect, test } from '@playwright/test'
import { resolveServiceIcon } from '../../../src/components/ServiceIcon'

test('service names resolve their own bundled logos before shorter brand names', () => {
  for (const [service, file] of [
    [' TypeSafe ', 'typesafe.png'],
    ['vercel_ai_gateway', 'vercel.svg'],
    ['LLM Gateway', 'llmgateway.svg'],
    ['DigitalOcean MCP Server', 'digitalocean.svg'],
    ['GitHub MCP Server', 'github.png'],
    ['OpenAI ChatGPT Plus', 'openai.png'],
    ['pi', 'pi.svg'],
    ['vocalcord', 'vocalcord.svg'],
    ['netget', 'netget.svg'],
  ]) {
    expect(resolveServiceIcon(service), service).toBe(file)
  }
})

test('generic endpoints and unknown services use category icons without unrelated branding', () => {
  for (const service of [
    'openai_compatible', 'openai-compatible', 'OpenAI Compatible',
    'My OpenAI-compatible endpoint', 'systemone_compatible',
    'kev', 'von', 'decider', 'custom', 'filesystem', 'fetch', 'time',
    'sequential-thinking', 'everything', 'pinecone', 'openapi', 'constructor',
  ]) {
    expect(resolveServiceIcon(service), service).toBeUndefined()
  }
})
