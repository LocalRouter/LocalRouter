import { test, expect } from '@playwright/test'
import type { ApiPathSupport, ProviderFeatureSupport, SupportLevel } from '../../../src/types/tauri-commands'
import { modelProviders, providerSectionAvailable, sectionNavigation, visibleTryItOutSections } from '../../../src/utils/try-it-out-sections'

const paths = (patch: Partial<ApiPathSupport> = {}): ApiPathSupport => ({
  chat_completions: 'supported', completions: 'translated', responses: 'translated',
  messages: 'translated', messages_count_tokens: 'not_supported', batches: 'not_supported',
  native_batch_cancel: false, native_batch_cleanup: false,
  search: 'not_supported', ocr: 'not_supported', rerank: 'not_supported', videos: 'not_supported', realtime: 'not_supported',
  ...patch,
})
const features = (entries: [string, SupportLevel][]): ProviderFeatureSupport => ({
  provider_type: 'fixture', provider_instance: 'fixture', model_features: [], optimization_features: [],
  endpoints: entries.map(([endpoint, support]) => ({ name: endpoint, endpoint, support, notes: null })),
})
const ids = (sections: ReturnType<typeof visibleTryItOutSections>) => sections.map(section => section.id)

test('unknown and failed capability lookups keep panels discoverable; known denials hide them', () => {
  const unknown = visibleTryItOutSections({ localClient: true, candidates: [undefined] })
  expect(ids(unknown)).toContain('realtime')
  expect(ids(unknown)).toContain('images')
  expect(providerSectionAvailable('images', { features: features([
    ['/v1/images/generations', 'not_supported'], ['/v1/images/edits', 'not_implemented'],
  ]) })).toBe(false)
  expect(providerSectionAvailable('images', { features: features([
    ['/v1/images/generations', 'not_supported'], ['/v1/images/edits', 'supported'],
  ]) })).toBe(true)
  expect(providerSectionAvailable('messages', { paths: paths() })).toBe(true)
  expect(providerSectionAvailable('ocr', { paths: paths() })).toBe(false)
})

test('native path support is authoritative even when the older feature matrix omits it', () => {
  const report = { paths: paths({ ocr: 'supported', videos: 'partial', realtime: 'supported' }), features: features([]) }
  const visible = ids(visibleTryItOutSections({ localClient: true, candidates: [report], nativeOperations: ['ocr', 'video', 'realtime'] }))
  expect(visible).toEqual(expect.arrayContaining(['ocr', 'videos', 'realtime']))
  expect(visible).not.toContain('rerank')
})

test('known client grants hide native operations and search even with unknown provider data', () => {
  const visible = ids(visibleTryItOutSections({ localClient: true, candidates: [undefined], nativeOperations: [], upstreamTools: { allowed_tools: ['web_fetch'], default_search_model: null, max_tool_calls: 2 } }))
  for (const section of ['ocr', 'rerank', 'videos', 'realtime', 'search']) expect(visible).not.toContain(section)
  expect(visible).toEqual(expect.arrayContaining(['files', 'replay', 'batches', 'documents']))
})

test('native panels use any eligible provider independently of the selected chat provider', () => {
  const chat = { paths: paths() }
  const video = { paths: paths({ videos: 'supported' }) }
  expect(ids(visibleTryItOutSections({ localClient: true, selected: chat, candidates: [chat, video], nativeOperations: ['video'] }))).toContain('videos')
  expect(ids(visibleTryItOutSections({ localClient: true, selected: video, candidates: [chat], nativeOperations: ['video'] }))).not.toContain('videos')
  expect(ids(visibleTryItOutSections({ localClient: true, candidates: [chat, undefined], nativeOperations: ['video'] }))).toContain('videos')
})

test('direct and wrapped connections exclude local client workflows', () => {
  const visible = ids(visibleTryItOutSections({ localClient: false, candidates: [undefined] }))
  for (const section of ['files', 'replay', 'batches', 'search', 'documents', 'ocr', 'rerank', 'videos', 'realtime']) expect(visible).not.toContain(section)
  expect(visible).toEqual(expect.arrayContaining(['chat', 'messages', 'decisions', 'images']))
})

test('all chat transports must be known unavailable before chat and local extraction disappear', () => {
  const unavailable = { paths: paths({ chat_completions: 'not_supported', responses: 'not_supported', completions: 'not_implemented' }) }
  expect(providerSectionAvailable('chat', unavailable)).toBe(false)
  expect(providerSectionAvailable('documents', unavailable)).toBe(false)
  expect(providerSectionAvailable('chat', { paths: paths({ chat_completions: 'not_supported', responses: 'translated', completions: 'not_supported' }) })).toBe(true)
})

test('provider resolution preserves namespaces, longest prefixes and ambiguous raw model IDs', () => {
  const providers = ['gateway', 'gateway/team', 'other']
  const cached = [{ id: 'shared', provider: 'gateway' }, { id: 'shared', provider: 'other' }, { id: 'hidden', provider: 'disabled' }]
  expect(modelProviders('gateway/team/vendor/model', providers, cached)).toEqual(['gateway/team'])
  expect(modelProviders('shared', providers, cached)).toEqual(['gateway', 'other'])
  expect(modelProviders('localrouter/auto', providers, cached)).toEqual([])
  expect(modelProviders('hidden', providers, cached)).toEqual([])
})

test('only common panels are initially tabs; selected secondary panels remain visible', () => {
  const visible = visibleTryItOutSections({ localClient: true, candidates: [] })
  const initial = sectionNavigation(visible, 'chat')
  expect(ids(initial.tabs)).toEqual(['chat', 'images', 'embeddings', 'speech', 'transcribe'])
  expect(ids(initial.more)).toContain('replay')
  const replay = sectionNavigation(visible, 'replay')
  expect(replay.selected?.id).toBe('replay')
  expect(ids(replay.tabs)).toContain('replay')
  expect(ids(sectionNavigation(visible, 'videos').tabs)).not.toContain('replay')
})

test('revoked or unsupported active panels fall back to an available tab; empty sets are safe', () => {
  const visible = visibleTryItOutSections({ localClient: false, candidates: [] })
  expect(sectionNavigation(visible, 'replay').selected?.id).toBe('chat')
  expect(sectionNavigation(visible.filter(section => section.id === 'decisions'), 'chat').selected?.id).toBe('decisions')
  expect(sectionNavigation([], 'chat')).toEqual({ selected: undefined, tabs: [], more: [] })
})
