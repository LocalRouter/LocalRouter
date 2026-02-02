/**
 * Unified service icon component for providers, MCP servers, and other services.
 * Uses local image files stored in public/icons/
 *
 * Usage:
 *   <ServiceIcon service="openai" size={24} />
 *   <ServiceIcon service="github" size={24} />
 *   <ServiceIcon service="GitHub MCP Server" size={24} />
 */

import { ProvidersIcon, McpIcon } from './icons/category-icons'

interface ServiceIconProps {
  /** Service identifier - can be a provider type, MCP server name, or service ID */
  service: string
  size?: number
  className?: string
  /** If true, shows generic server icon as fallback instead of emoji */
  fallbackToServerIcon?: boolean
}

// Mapping of service identifiers to icon files
// Keys are lowercase for case-insensitive matching
const ICON_MAP: Record<string, string> = {
  // LLM Providers
  ollama: 'ollama.png',
  lmstudio: 'lmstudio.png',
  openai: 'openai.png',
  anthropic: 'anthropic.png',
  gemini: 'gemini.png',
  groq: 'groq.png',
  mistral: 'mistral.png',
  cohere: 'cohere.png',
  togetherai: 'togetherai.png',
  perplexity: 'perplexity.png',
  deepinfra: 'deepinfra.png',
  cerebras: 'cerebras.png',
  xai: 'xai.png',
  openrouter: 'openrouter.png',
  'openai_compatible': 'openai.png',
  'openai-compatible': 'openai.png',

  // OAuth/Subscription Providers
  'github-copilot': 'github.png',
  'openai-codex': 'openai.png',
  'openai-chatgpt-plus': 'openai.png',
  'anthropic-claude': 'anthropic.png',

  // VCS Services
  github: 'github.png',
  gitlab: 'gitlab.png',
  git: 'git.png',

  // Productivity Services
  slack: 'slack.png',
  jira: 'jira.png',
  notion: 'notion.png',
  confluence: 'confluence.png',
  'google-drive': 'google-drive.png',
  'google drive': 'google-drive.png',
  gdrive: 'google-drive.png',
  figma: 'figma.png',
  obsidian: 'obsidian.svg',

  // Databases
  postgres: 'postgres.png',
  postgresql: 'postgres.png',
  supabase: 'supabase.png',
  sqlite: 'sqlite-banner.gif',

  // Search Services
  brave: 'brave.svg',
  'brave-search': 'brave.svg',
}

// Emoji fallbacks for services
const EMOJI_MAP: Record<string, string> = {
  // LLM Providers
  ollama: '🦙',
  lmstudio: '💻',
  openai: '🤖',
  anthropic: '🧠',
  gemini: '✨',
  groq: '⚡',
  mistral: '🌬️',
  cohere: '🎯',
  togetherai: '🤝',
  perplexity: '🔍',
  deepinfra: '🏗️',
  cerebras: '🧮',
  xai: '🚀',
  openrouter: '🌐',
  'openai_compatible': '🔌',
  'openai-compatible': '🔌',

  // OAuth Providers
  'github-copilot': '🐙',
  'openai-codex': '🤖',
  'openai-chatgpt-plus': '🤖',
  'anthropic-claude': '🧠',

  // VCS
  github: '🐙',
  gitlab: '🦊',
  git: '🔗',

  // Productivity
  slack: '💬',
  jira: '📋',
  notion: '📑',
  confluence: '📄',
  'google-drive': '📁',
  figma: '🎨',
  obsidian: '🧠',

  // Databases
  postgres: '🐘',
  postgresql: '🐘',
  supabase: '🗄️',
  sqlite: '💾',

  // Search
  brave: '🦁',
  'brave-search': '🦁',

  // Generic MCP categories
  filesystem: '📂',
  fetch: '🌐',
  time: '⏰',
  memory: '💭',
  everything: '🧪',
}

/**
 * Normalize and find a matching icon for a service name
 */
function findMatch(service: string): { iconFile?: string; emoji?: string } {
  const normalized = service.toLowerCase().trim()

  // Direct match
  if (ICON_MAP[normalized]) {
    return { iconFile: ICON_MAP[normalized], emoji: EMOJI_MAP[normalized] }
  }

  // Pattern match - check if the service name contains a known key
  for (const [key, iconFile] of Object.entries(ICON_MAP)) {
    if (normalized.includes(key)) {
      return { iconFile, emoji: EMOJI_MAP[key] }
    }
  }

  // Emoji-only match
  for (const [key, emoji] of Object.entries(EMOJI_MAP)) {
    if (normalized.includes(key)) {
      return { emoji }
    }
  }

  return {}
}

export default function ServiceIcon({
  service,
  size = 32,
  className = '',
  fallbackToServerIcon = false
}: ServiceIconProps) {
  const { iconFile, emoji } = findMatch(service)

  // If we have an icon file, render it with fallback
  if (iconFile) {
    const padding = Math.max(4, size * 0.15)
    const containerSize = size + padding * 2

    return (
      <span
        className="inline-flex items-center justify-center rounded-lg dark:bg-white/90"
        style={{ width: containerSize, height: containerSize }}
      >
        <img
          src={`/icons/${iconFile}`}
          alt={`${service} logo`}
          width={size}
          height={size}
          className={className}
          onError={(e) => {
            const target = e.target as HTMLImageElement
            target.style.display = 'none'
            if (target.nextElementSibling) {
              (target.nextElementSibling as HTMLElement).style.display = 'inline-flex'
            }
          }}
        />
        <span
          className={`items-center justify-center ${className}`}
          style={{ display: 'none' }}
        >
          {emoji
            ? <span style={{ fontSize: `${size * 0.6}px`, lineHeight: '1' }}>{emoji}</span>
            : fallbackToServerIcon
              ? <McpIcon style={{ width: size * 0.6, height: size * 0.6 }} />
              : <ProvidersIcon style={{ width: size * 0.6, height: size * 0.6 }} />
          }
        </span>
      </span>
    )
  }

  // Emoji fallback
  if (emoji) {
    return (
      <span className={className} style={{ fontSize: `${size * 0.6}px`, lineHeight: '1' }}>
        {emoji}
      </span>
    )
  }

  // Ultimate fallback - use category icons matching the sidebar
  if (fallbackToServerIcon) {
    return <McpIcon className={className} style={{ width: size * 0.6, height: size * 0.6 }} />
  }

  return <ProvidersIcon className={className} style={{ width: size * 0.6, height: size * 0.6 }} />
}

// Re-export for backwards compatibility
export { ServiceIcon }
