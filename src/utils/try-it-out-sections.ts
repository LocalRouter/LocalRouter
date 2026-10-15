import type { ApiPathSupport, NativeOperation, ProviderFeatureSupport, SupportLevel, UpstreamToolsPolicy } from '@/types/tauri-commands'

export const TRY_IT_OUT_SECTIONS = [
  { id: 'chat', label: 'Chat', common: true },
  { id: 'images', label: 'Images', common: true },
  { id: 'embeddings', label: 'Embeddings', common: true },
  { id: 'speech', label: 'Speech', common: true },
  { id: 'transcribe', label: 'Transcribe', common: true },
  { id: 'replay', label: 'Replay / Compare' },
  { id: 'messages', label: 'Messages' },
  { id: 'decisions', label: 'Decisions' },
  { id: 'systemone', label: 'System One' },
  { id: 'files', label: 'Files' },
  { id: 'batches', label: 'Batches' },
  { id: 'search', label: 'Search' },
  { id: 'documents', label: 'Documents' },
  { id: 'ocr', label: 'OCR' },
  { id: 'rerank', label: 'Rerank' },
  { id: 'videos', label: 'Video' },
  { id: 'realtime', label: 'Realtime' },
] as const

export type TryItOutSection = typeof TRY_IT_OUT_SECTIONS[number]['id']
export interface ProviderSupport {
  paths?: ApiPathSupport
  features?: ProviderFeatureSupport
}

const available = (level?: SupportLevel) => level !== 'not_supported' && level !== 'not_implemented'
const endpoints = {
  decisions: '/v1/decisions', systemone: '/v1/systemone', images: '/v1/images/generations',
  embeddings: '/v1/embeddings', speech: '/v1/audio/speech', transcribe: '/v1/audio/transcriptions',
} as const

export function providerSectionAvailable(section: TryItOutSection, report?: ProviderSupport): boolean {
  if (section === 'chat' || section === 'documents') {
    return !report?.paths || [report.paths.chat_completions, report.paths.responses, report.paths.completions].some(available)
  }
  if (section === 'messages' || section === 'batches' || section === 'search' || section === 'ocr' || section === 'rerank' || section === 'videos' || section === 'realtime') {
    return available(report?.paths?.[section])
  }
  if (section === 'images') {
    return ['/v1/images/generations', '/v1/images/edits'].some(endpoint =>
      available(report?.features?.endpoints.find(item => item.endpoint === endpoint)?.support))
  }
  if (section in endpoints) {
    const endpoint = endpoints[section as keyof typeof endpoints]
    return available(report?.features?.endpoints.find(item => item.endpoint === endpoint)?.support)
  }
  return true
}

/** A provider prefix wins over ambiguous raw IDs. Unresolved aliases stay unknown. */
export function modelProviders(model: string, providers: string[], cached: { id: string; provider: string }[]): string[] {
  const prefix = providers.filter(name => model.startsWith(`${name}/`)).sort((a, b) => b.length - a.length)[0]
  if (prefix) return [prefix]
  return [...new Set(cached.filter(item => item.id === model && providers.includes(item.provider)).map(item => item.provider))]
}

export function visibleTryItOutSections({ localClient, selected, candidates, nativeOperations, upstreamTools }: {
  localClient: boolean
  selected?: ProviderSupport
  // Undefined entries represent unresolved models/failed lookups, never known denial.
  candidates: (ProviderSupport | undefined)[]
  nativeOperations?: NativeOperation[]
  upstreamTools?: UpstreamToolsPolicy
}) {
  const anyProvider = (section: TryItOutSection) => candidates.length === 0 || candidates.some(report => providerSectionAvailable(section, report))
  return TRY_IT_OUT_SECTIONS.filter(({ id }) => {
    if (id === 'files' || id === 'replay') return localClient
    if (id === 'search' || id === 'documents' || id === 'batches' || id === 'ocr' || id === 'rerank' || id === 'videos' || id === 'realtime') {
      if (!localClient) return false
      const operation = id === 'videos' ? 'video' : id
      if ((operation === 'ocr' || operation === 'rerank' || operation === 'video' || operation === 'realtime') && nativeOperations && !nativeOperations.includes(operation)) return false
      if (id === 'search' && upstreamTools && !upstreamTools.allowed_tools.includes('web_search')) return false
      return anyProvider(id)
    }
    return providerSectionAvailable(id, selected)
  })
}

export function sectionNavigation(visible: ReturnType<typeof visibleTryItOutSections>, active: string) {
  const selected = visible.find(section => section.id === active) ?? visible[0]
  return {
    selected,
    tabs: visible.filter(section => ('common' in section && section.common) || section === selected),
    more: visible.filter(section => !('common' in section && section.common)),
  }
}
