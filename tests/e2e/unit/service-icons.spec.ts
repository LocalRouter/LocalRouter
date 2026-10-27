import { expect, test } from '@playwright/test'
import { resolveServiceIcon, serviceIconImageClass } from '../../../src/components/ServiceIcon'

test('service names resolve their own bundled logos before shorter brand names', () => {
  for (const [service, file] of [
    [' TypeSafe ', 'typesafe.png'],
    ['vercel_ai_gateway', 'vercel.svg'],
    ['LLM Gateway', 'llmgateway.svg'],
    ['DigitalOcean MCP Server', 'digitalocean.svg'],
    ['GitHub MCP Server', 'github.svg'],
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


test('monochrome bundled marks become white in dark mode without recoloring full-color or discovered icons', () => {
  for (const file of ['openai.png', 'anthropic.png', 'gemini.png', 'github.svg', 'ollama.png', 'mistral.png', 'vercel.svg']) {
    expect(serviceIconImageClass(`/icons/${file}`)).toBe('object-contain dark:brightness-0 dark:invert')
  }
  for (const source of ['/icons/slack.png', '/icons/google-drive.png', '/icons/typesafe.png', 'data:image/png;base64,example', undefined]) {
    expect(serviceIconImageClass(source)).toBe('object-contain')
  }
})
