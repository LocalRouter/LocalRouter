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
  /** If true, uses the standard MCP server icon for services without a logo */
  fallbackToServerIcon?: boolean
}

// Mapping of service identifiers to icon files
// Keys are lowercase for case-insensitive matching
const ICON_MAP: Record<string, string | null> = {
  // LLM Providers
  ollama: 'ollama.png',
  lmstudio: 'lmstudio.png',
  jan: 'jan.png',
  gpt4all: 'gpt4all.png',
  localai: 'localai.png',
  llamacpp: 'llamacpp.png',
  fastflowlm: 'fastflowlm.png', // official asset; see public/icons/SOURCES.md
  // Local Embedded providers (official upstream assets; see public/icons/SOURCES.md)
  llamacpp_embedded: 'llamacpp.png',
  ollaya: 'ollaya.svg',
  laya: 'laya.svg',
  sdcpp_embedded: 'sdcpp.png',
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
  llmgateway: 'llmgateway.svg', // official asset; see public/icons/SOURCES.md
  'llm gateway': 'llmgateway.svg',
  typesafe: 'typesafe.png',
  vercel_ai_gateway: 'vercel.svg',
  vercel: 'vercel.svg',
  // Generic endpoints do not carry a provider brand. Null also prevents
  // their names from matching the shorter OpenAI key.
  openai_compatible: null,
  'openai-compatible': null,
  'openai compatible': null,
  systemone_compatible: null,
  'systemone-compatible': null,
  'system one compatible': null,
  github_models: 'github.png',
  nvidia_nim: 'nvidia.svg',
  nvidia: 'nvidia.svg',
  cloudflare_ai: 'cloudflare.svg',
  llm7: 'llm7.png',
  kluster_ai: 'kluster.svg',
  huggingface: 'huggingface.svg',
  'hugging-face': 'huggingface.svg',
  zhipu: 'zhipu.svg',
  digitalocean: 'digitalocean.svg',
  opencode_zen: 'opencode.png',
  opencode_go: 'opencode.png',

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
  atlassian: 'atlassian.svg',
  datadog: 'datadog.png',
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
  pi: 'pi.svg',
  cline: 'cline.png',
  'roo-code': 'roo-code.png',
  jetbrains: 'jetbrains.svg',
  marimo: 'marimo.svg',
  n8n: 'n8n.png',
  onyx: 'onyx.svg',
  xcode: 'xcode.png',
  zed: 'zed.png',
}

/**
 * Keys too short to match as a substring of another service name: "pi" would
 * otherwise claim "pinecone", "openapi", "copilot" and similar names.
 */
const EXACT_MATCH_ONLY = new Set(['pi'])

// Prefer the most specific service name: e.g. DigitalOcean over Git.
const ICON_PATTERNS = Object.entries(ICON_MAP).sort(([a], [b]) => b.length - a.length)

/** Normalize a service name and resolve its locally bundled logo. */
export function resolveServiceIcon(service: string): string | undefined {
  const normalized = service.toLowerCase().trim()

  if (Object.prototype.hasOwnProperty.call(ICON_MAP, normalized)) {
    return ICON_MAP[normalized] ?? undefined
  }

  for (const [key, iconFile] of ICON_PATTERNS) {
    if (!EXACT_MATCH_ONLY.has(key) && normalized.includes(key)) {
      return iconFile ?? undefined
    }
  }

  return undefined
}

export default function ServiceIcon({
  service,
  size = 32,
  className = '',
  fallbackToServerIcon = false
}: ServiceIconProps) {
  const iconFile = resolveServiceIcon(service)

  // Reset image/fallback DOM state when the resolved logo changes.
  if (iconFile) {
    const padding = Math.max(4, size * 0.15)
    const containerSize = size + padding * 2

    return (
      <span
        key={iconFile}
        className="inline-flex items-center justify-center rounded-lg dark:bg-white/90"
        style={{ width: containerSize, height: containerSize }}
      >
        <img
          src={`/icons/${iconFile}`}
          alt={`${service} logo`}
          width={size}
          height={size}
          className={`object-contain ${className}`}
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
          {fallbackToServerIcon
            ? <McpIcon style={{ width: size, height: size }} />
            : <ProvidersIcon style={{ width: size, height: size }} />
          }
        </span>
      </span>
    )
  }

  // Services without a logo use the established category icon.
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
