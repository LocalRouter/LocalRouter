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
  jan: 'jan.png',
  gpt4all: 'gpt4all.png',
  localai: 'localai.png',
  llamacpp: 'llamacpp.png',
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
  github_models: 'github.png',
  nvidia_nim: 'nvidia.svg',
  nvidia: 'nvidia.svg',
  cloudflare_ai: 'cloudflare.svg',
  llm7: 'llm7.png',
  kluster_ai: 'kluster.svg',
  huggingface: 'huggingface.svg',
  'hugging-face': 'huggingface.svg',
  zhipu: 'zhipu.svg',

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
  'google-workspace': 'google-workspace.svg',
  gws: 'google-workspace.svg',
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

  // Cloud & Infrastructure
  'aws-core': 'aws.svg',
  'aws-docs': 'aws.svg',
  aws: 'aws.svg',
  gcloud: 'gcloud.svg',
  'google-cloud': 'gcloud.svg',
  kubernetes: 'kubernetes.svg',
  kubectl: 'kubernetes.svg',
  docker: 'docker.svg',
  cloudflare: 'cloudflare.svg',

  // Client Templates (coding assistants, IDEs, chat UIs, automation)
  'claude-code': 'anthropic.png',
  codex: 'openai.png',
  aider: 'aider.png',
  cursor: 'cursor.svg',
  windsurf: 'windsurf.svg',
  'vscode-continue': 'continue.png',
  opencode: 'opencode.png',
  droid: 'droid.svg',
  'open-webui': 'open-webui.png',
  lobechat: 'lobechat.png',
  goose: 'goose.png',
  openclaw: 'openclaw.png',
  cline: 'cline.png',
  'roo-code': 'roo-code.png',
  jetbrains: 'jetbrains.svg',
  marimo: 'marimo.svg',
  n8n: 'n8n.png',
  onyx: 'onyx.svg',
  xcode: 'xcode.png',
  zed: 'zed.png',
}

// Emoji fallbacks for services
const EMOJI_MAP: Record<string, string> = {
  // LLM Providers
  ollama: '🦙',
  lmstudio: '💻',
  jan: '👋',
  gpt4all: '🌿',
  localai: '🦙',
  llamacpp: '🦙',
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
  github_models: '🐙',
  nvidia_nim: '💚',
  nvidia: '💚',
  cloudflare_ai: '🔶',
  llm7: '🐴',
  kluster_ai: '🔗',
  huggingface: '🤗',
  'hugging-face': '🤗',
  zhipu: '🇨🇳',
  digitalocean: '🌊',

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
  'google-workspace': '🏢',
  gws: '🏢',
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

  // Client Templates
  'claude-code': '🧠',
  codex: '💻',
  aider: '🤖',
  cursor: '🖱️',
  windsurf: '🏄',
  'vscode-continue': '🔧',
  opencode: '📝',
  droid: '🤖',
  'open-webui': '🌐',
  lobechat: '💬',
  goose: '🪿',
  openclaw: '🦞',
  cline: '🤖',
  'roo-code': '🦘',
  jetbrains: '🧠',
  marimo: '📓',
  n8n: '⚡',
  onyx: '💎',
  xcode: '🔨',
  zed: '⚡',
  custom: '⚙️',
  // Cloud & Infrastructure
  'aws-core': '☁️',
  'aws-docs': '📖',
  aws: '☁️',
  gcloud: '🌤️',
  'google-cloud': '🌤️',
  kubernetes: '☸️',
  kubectl: '☸️',
  docker: '🐳',
  cloudflare: '🔶',

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
          className={`items-center justify-center text-foreground dark:text-gray-800 ${className}`}
          style={{ display: 'none' }}
        >
          {emoji
            ? <span style={{ fontSize: `${size}px`, lineHeight: '1' }}>{emoji}</span>
            : fallbackToServerIcon
              ? <McpIcon style={{ width: size, height: size }} />
              : <ProvidersIcon style={{ width: size, height: size }} />
          }
        </span>
      </span>
    )
  }

  // Emoji fallback - use same container as icons for consistent alignment
  if (emoji) {
    const padding = Math.max(4, size * 0.15)
    const containerSize = size + padding * 2

    return (
      <span
        className={`inline-flex items-center justify-center ${className}`}
        style={{ width: containerSize, height: containerSize }}
      >
        <span style={{ fontSize: `${size}px`, lineHeight: '1' }}>
          {emoji}
        </span>
      </span>
    )
  }

  // Ultimate fallback - use category icons with same container
  const padding = Math.max(4, size * 0.15)
  const containerSize = size + padding * 2

  if (fallbackToServerIcon) {
    return (
      <span
        className={`inline-flex items-center justify-center text-foreground ${className}`}
        style={{ width: containerSize, height: containerSize }}
      >
        <McpIcon style={{ width: size, height: size }} />
      </span>
    )
  }

  return (
    <span
      className={`inline-flex items-center justify-center text-foreground ${className}`}
      style={{ width: containerSize, height: containerSize }}
    >
      <ProvidersIcon style={{ width: size, height: size }} />
    </span>
  )
}

// Re-export for backwards compatibility
export { ServiceIcon }
