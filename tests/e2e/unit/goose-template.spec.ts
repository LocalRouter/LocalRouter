import { expect, test } from '@playwright/test'
import { CLIENT_TEMPLATES, resolveTemplatePlaceholders } from '../../../src/components/client/ClientTemplates'

const goose = CLIENT_TEMPLATES.find(template => template.id === 'goose')!

function providerFor(models: Array<{ id: string }>) {
  const snippet = goose.configFile!.jsonSnippet
  return JSON.parse(resolveTemplatePlaceholders(
    typeof snippet === 'function' ? snippet({ models }) : snippet,
    'http://127.0.0.1:3625', 'client-secret', 'client-id', '/home/test', '/home/test/.config',
  ))
}

test('Goose manual configuration advertises only the supplied permitted models', () => {
  const modelIds = ['gpt-6.1-terra', 'provider/model-with-"quotes"']
  const provider = providerFor(modelIds.map(id => ({ id })))

  expect(provider.models.map((model: { name: string }) => model.name)).toEqual(modelIds)
  expect(provider.models.every((model: object) => !('context_limit' in model))).toBe(true)
  expect(provider.base_url).toBe('http://127.0.0.1:3625')
  expect(provider.base_path).toBe('chat/completions')
  expect(provider.headers.Authorization).toBe('Bearer client-secret')
  expect(provider.dynamic_models).toBe(false)
})

test('Goose does not invent an auto model when the client has no permitted models', () => {
  expect(providerFor([]).models).toEqual([])
  expect(providerFor([{ id: 'localrouter/auto' }]).models).toEqual([{ name: 'localrouter/auto' }])
})
