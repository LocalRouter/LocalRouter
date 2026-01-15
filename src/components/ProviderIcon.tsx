/**
 * Provider icon component that displays actual provider logos
 * Uses local PNG files stored in public/icons/providers/
 */

interface ProviderIconProps {
  providerId: string
  size?: number
  className?: string
}

// Mapping of provider IDs to local icon file names
const ICON_MAP: Record<string, string> = {
  // Regular providers
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
  openai_compatible: 'openai.png',  // Use OpenAI logo as fallback

  // OAuth providers
  'github-copilot': 'github.png',
  'openai-codex': 'openai.png',
  'anthropic-claude': 'anthropic.png',
}

// Emoji fallbacks for providers without logos
const EMOJI_FALLBACK: Record<string, string> = {
  // Regular providers
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
  openai_compatible: '🔌',

  // OAuth providers
  'github-copilot': '🐙',
  'openai-codex': '🤖',
  'anthropic-claude': '🧠',
}

export default function ProviderIcon({ providerId, size = 32, className = '' }: ProviderIconProps) {
  const iconFile = ICON_MAP[providerId]
  const emoji = EMOJI_FALLBACK[providerId] || '📦'

  // If we have an icon mapping, use the local file with emoji fallback
  if (iconFile) {
    return (
      <span className="inline-flex items-center justify-center" style={{ width: size, height: size }}>
        <img
          src={`/icons/providers/${iconFile}`}
          alt={`${providerId} logo`}
          width={size}
          height={size}
          className={className}
          onError={(e) => {
            // Fallback to emoji if image fails to load
            const target = e.target as HTMLImageElement
            target.style.display = 'none'
            if (target.nextElementSibling) {
              (target.nextElementSibling as HTMLElement).style.display = 'inline-block'
            }
          }}
        />
        <span
          className={className}
          style={{ fontSize: `${size * 0.6}px`, lineHeight: '1', display: 'none' }}
        >
          {emoji}
        </span>
      </span>
    )
  }

  // Fallback to emoji
  return (
    <span className={className} style={{ fontSize: `${size * 0.6}px`, lineHeight: '1' }}>
      {emoji}
    </span>
  )
}
