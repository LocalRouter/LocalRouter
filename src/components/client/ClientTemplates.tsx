/**
 * Client template definitions and selection grid.
 *
 * Follows the McpServerTemplates pattern for consistent UX.
 * Templates define app-specific connection configurations for
 * coding assistants, IDEs, and other LLM/MCP-consuming apps.
 */

import React from 'react'
import ServiceIcon from '../ServiceIcon'
import type { ClientMode } from '@/types/tauri-commands'

export type ClientTemplateCategory = 'coding_assistants' | 'ide_extensions' | 'ides' | 'chat' | 'automation' | 'cli'

export interface ClientTemplateEnvVar {
  name: string
  /** Placeholders: {{BASE_URL}}, {{CLIENT_SECRET}}, {{CLIENT_ID}} */
  value: string
  description?: string
}

export interface SnippetContext {
  models: Array<{ id: string }>
}

export interface ClientTemplateConfigFile {
  /** With {{HOME_DIR}} or {{CONFIG_DIR}} placeholder */
  path: string
  /** Template JSON with placeholders. Can be a function for dynamic content (e.g. model lists). */
  jsonSnippet: string | ((context: SnippetContext) => string)
  description?: string
}

export interface ClientTemplate {
  id: string
  name: string
  description: string
  category: ClientTemplateCategory
  icon: string
  defaultMode: ClientMode
  setupType: 'env_vars' | 'config_file' | 'generic'
  envVars?: ClientTemplateEnvVar[]
  configFile?: ClientTemplateConfigFile
  manualInstructions?: string
  docsUrl?: string
  supportsMcp: boolean
  supportsLlm: boolean
  binaryNames?: string[]
}

export const CLIENT_TEMPLATES: ClientTemplate[] = [
  // === Coding Assistants (CLI) ===
  {
    id: 'claude-code',
    name: 'Claude Code',
    description: 'Anthropic\'s CLI coding assistant with MCP support.',
    category: 'coding_assistants',
    icon: 'anthropic',
    defaultMode: 'both',
    setupType: 'env_vars',
    envVars: [
      { name: 'ANTHROPIC_BASE_URL', value: '{{BASE_URL}}', description: 'LocalRouter API endpoint' },
      { name: 'ANTHROPIC_API_KEY', value: '{{CLIENT_SECRET}}', description: 'Client secret' },
    ],
    docsUrl: 'https://docs.anthropic.com/en/docs/claude-code',
    supportsMcp: true,
    supportsLlm: true,
    binaryNames: ['claude'],
  },
  {
    id: 'codex',
    name: 'Codex',
    description: 'OpenAI\'s CLI coding assistant.',
    category: 'coding_assistants',
    icon: 'openai',
    defaultMode: 'both',
    setupType: 'env_vars',
    envVars: [
      { name: 'OPENAI_BASE_URL', value: '{{BASE_URL}}', description: 'LocalRouter API endpoint' },
      { name: 'OPENAI_API_KEY', value: '{{CLIENT_SECRET}}', description: 'Client secret' },
    ],
    docsUrl: 'https://github.com/openai/codex',
    supportsMcp: true,
    supportsLlm: true,
    binaryNames: ['codex'],
  },
  {
    id: 'aider',
    name: 'Aider',
    description: 'AI pair programming in the terminal.',
    category: 'coding_assistants',
    icon: 'aider',
    defaultMode: 'llm_only',
    setupType: 'env_vars',
    envVars: [
      { name: 'OPENAI_API_BASE', value: '{{BASE_URL}}', description: 'LocalRouter API endpoint' },
      { name: 'OPENAI_API_KEY', value: '{{CLIENT_SECRET}}', description: 'Client secret' },
    ],
    docsUrl: 'https://aider.chat',
    supportsMcp: false,
    supportsLlm: true,
    binaryNames: ['aider'],
  },
  {
    id: 'opencode',
    name: 'OpenCode',
    description: 'Terminal-based coding assistant with multi-provider support.',
    category: 'coding_assistants',
    icon: 'opencode',
    defaultMode: 'both',
    setupType: 'config_file',
    configFile: {
      path: '{{CONFIG_DIR}}/opencode/opencode.json',
      jsonSnippet: ({ models }) => {
        const modelsMap: Record<string, { name: string }> = {}
        for (const m of models) {
          modelsMap[m.id] = { name: m.id }
        }
        return JSON.stringify({
          provider: {
            localrouter: {
              npm: '@ai-sdk/openai-compatible',
              name: 'LocalRouter',
              options: { baseURL: '{{BASE_URL}}/v1', apiKey: '{{CLIENT_SECRET}}' },
              models: modelsMap,
            },
          },
          mcp: {
            localrouter: {
              type: 'remote',
              url: '{{BASE_URL}}',
              headers: {
                Authorization: 'Bearer {{CLIENT_SECRET}}',
              },
            },
          },
        }, null, 2)
      },
      description: 'Adds LocalRouter as a provider and MCP server.',
    },
    docsUrl: 'https://opencode.ai',
    supportsMcp: true,
    supportsLlm: true,
    binaryNames: ['opencode'],
  },
  {
    id: 'droid',
    name: 'Droid',
    description: 'AI coding assistant by Factory.',
    category: 'coding_assistants',
    icon: 'droid',
    defaultMode: 'both',
    setupType: 'config_file',
    configFile: {
      path: '{{HOME_DIR}}/.factory/settings.json',
      jsonSnippet: JSON.stringify({
        customModels: [{
          model: 'localrouter',
          baseUrl: '{{BASE_URL}}',
          apiKey: '{{CLIENT_SECRET}}',
          provider: 'generic-chat-completion-api',
        }],
      }, null, 2),
      description: 'Adds LocalRouter as a custom model provider.',
    },
    docsUrl: 'https://www.factory.ai',
    supportsMcp: true,
    supportsLlm: true,
    binaryNames: ['droid'],
  },
  {
    id: 'goose',
    name: 'Goose',
    description: 'Block\'s agentic coding assistant (desktop & CLI).',
    category: 'coding_assistants',
    icon: 'goose',
    defaultMode: 'both',
    setupType: 'env_vars',
    envVars: [
      { name: 'OPENAI_BASE_URL', value: '{{BASE_URL}}', description: 'LocalRouter API endpoint' },
      { name: 'OPENAI_API_KEY', value: '{{CLIENT_SECRET}}', description: 'Client secret' },
    ],
    manualInstructions: 'Alternatively, in Goose Desktop: Settings > Configure Provider > add an OpenAI-compatible provider with the base URL and API key above.',
    docsUrl: 'https://block.github.io/goose',
    supportsMcp: true,
    supportsLlm: true,
    binaryNames: ['goose'],
  },
  {
    id: 'openclaw',
    name: 'OpenClaw',
    description: 'AI assistant bridging messaging apps to coding agents.',
    category: 'coding_assistants',
    icon: 'openclaw',
    defaultMode: 'llm_only',
    setupType: 'config_file',
    configFile: {
      path: '{{HOME_DIR}}/.openclaw/openclaw.json',
      jsonSnippet: JSON.stringify({
        models: {
          providers: {
            localrouter: {
              baseUrl: '{{BASE_URL}}',
              apiKey: '{{CLIENT_SECRET}}',
              api: 'openai-completions',
            },
          },
        },
      }, null, 2),
      description: 'Adds LocalRouter as an OpenAI-compatible provider.',
    },
    docsUrl: 'https://docs.openclaw.ai',
    supportsMcp: false,
    supportsLlm: true,
    binaryNames: ['openclaw', 'clawdbot'],
  },

  // === IDE Extensions (VS Code) ===
  {
    id: 'cline',
    name: 'Cline',
    description: 'AI coding assistant VS Code extension with agentic capabilities.',
    category: 'ide_extensions',
    icon: 'cline',
    defaultMode: 'llm_only',
    setupType: 'generic',
    manualInstructions: 'In Cline settings: set API Provider to "OpenAI Compatible", set Base URL to {{BASE_URL}}, and enter your client secret as the API key.',
    docsUrl: 'https://docs.cline.bot',
    supportsMcp: false,
    supportsLlm: true,
  },
  {
    id: 'roo-code',
    name: 'Roo Code',
    description: 'AI coding assistant VS Code extension (fork of Cline).',
    category: 'ide_extensions',
    icon: 'roo-code',
    defaultMode: 'llm_only',
    setupType: 'generic',
    manualInstructions: 'In Roo Code settings: set API Provider to "OpenAI Compatible", set Base URL to {{BASE_URL}}, and enter your client secret as the API key.',
    docsUrl: 'https://github.com/RooVetGit/Roo-Code',
    supportsMcp: false,
    supportsLlm: true,
  },
  {
    id: 'vscode-continue',
    name: 'VS Code + Continue',
    description: 'Continue extension for VS Code with OpenAI-compatible providers.',
    category: 'ide_extensions',
    icon: 'vscode-continue',
    defaultMode: 'llm_only',
    setupType: 'generic',
    manualInstructions: 'In Continue settings, add an OpenAI-compatible provider with base URL {{BASE_URL}} and API key set to your client secret.',
    docsUrl: 'https://docs.continue.dev',
    supportsMcp: false,
    supportsLlm: true,
  },

  // === IDEs & Editors ===
  {
    id: 'cursor',
    name: 'Cursor',
    description: 'AI-first code editor built on VS Code.',
    category: 'ides',
    icon: 'cursor',
    defaultMode: 'both',
    setupType: 'config_file',
    configFile: {
      path: '{{CONFIG_DIR}}/Cursor/User/settings.json',
      jsonSnippet: JSON.stringify({
        'openai.apiBaseUrl': '{{BASE_URL}}',
        'openai.apiKey': '{{CLIENT_SECRET}}',
      }, null, 2),
      description: 'Configures Cursor to use LocalRouter as its OpenAI-compatible backend.',
    },
    docsUrl: 'https://cursor.com',
    supportsMcp: true,
    supportsLlm: true,
    binaryNames: ['cursor'],
  },
  {
    id: 'windsurf',
    name: 'Windsurf',
    description: 'AI-powered code editor by Codeium.',
    category: 'ides',
    icon: 'windsurf',
    defaultMode: 'llm_only',
    setupType: 'generic',
    manualInstructions: 'Configure Windsurf to use an OpenAI-compatible API endpoint. Set the base URL to {{BASE_URL}} and use your client secret as the API key.',
    docsUrl: 'https://codeium.com/windsurf',
    supportsMcp: false,
    supportsLlm: true,
    binaryNames: ['windsurf'],
  },
  {
    id: 'zed',
    name: 'Zed',
    description: 'Modern code editor with built-in LLM provider support.',
    category: 'ides',
    icon: 'zed',
    defaultMode: 'llm_only',
    setupType: 'generic',
    manualInstructions: 'In Zed: click the star icon > Configure > LLM Providers > add an OpenAI-compatible provider with host URL {{BASE_URL}} and your client secret as the API key.',
    docsUrl: 'https://zed.dev',
    supportsMcp: false,
    supportsLlm: true,
    binaryNames: ['zed'],
  },
  {
    id: 'jetbrains',
    name: 'JetBrains',
    description: 'IntelliJ, PyCharm, and other JetBrains IDEs with AI assistant.',
    category: 'ides',
    icon: 'jetbrains',
    defaultMode: 'llm_only',
    setupType: 'generic',
    manualInstructions: 'In your JetBrains IDE: open the AI chat sidebar > Set up Local Models > add an OpenAI-compatible provider with host URL {{BASE_URL}} and your client secret as the API key.',
    docsUrl: 'https://www.jetbrains.com/ai',
    supportsMcp: false,
    supportsLlm: true,
  },
  {
    id: 'xcode',
    name: 'Xcode',
    description: 'Apple\'s IDE with AI intelligence features (macOS only).',
    category: 'ides',
    icon: 'xcode',
    defaultMode: 'llm_only',
    setupType: 'generic',
    manualInstructions: 'In Xcode 26+: Settings > Intelligence > Internet Hosted > enter URL {{BASE_URL}} and your client secret as the API key, then click Add.',
    docsUrl: 'https://developer.apple.com/xcode',
    supportsMcp: false,
    supportsLlm: true,
  },

  // === Chat UIs ===
  {
    id: 'open-webui',
    name: 'Open WebUI',
    description: 'Self-hosted web UI for LLMs with OpenAI-compatible API.',
    category: 'chat',
    icon: 'open-webui',
    defaultMode: 'llm_only',
    setupType: 'generic',
    manualInstructions: 'In Open WebUI settings, add an OpenAI-compatible connection. Set the API URL to {{BASE_URL}} and the API key to your client secret.',
    docsUrl: 'https://openwebui.com',
    supportsMcp: false,
    supportsLlm: true,
  },
  {
    id: 'lobechat',
    name: 'LobeChat',
    description: 'Modern chat UI with multi-provider support.',
    category: 'chat',
    icon: 'lobechat',
    defaultMode: 'llm_only',
    setupType: 'generic',
    manualInstructions: 'In LobeChat provider settings, add an OpenAI-compatible provider with endpoint {{BASE_URL}} and your client secret as the API key.',
    docsUrl: 'https://lobechat.com',
    supportsMcp: false,
    supportsLlm: true,
  },
  {
    id: 'onyx',
    name: 'Onyx',
    description: 'Self-hosted chat with agents, RAG, and MCP support.',
    category: 'chat',
    icon: 'onyx',
    defaultMode: 'both',
    setupType: 'generic',
    manualInstructions: 'In Onyx setup: select an OpenAI-compatible LLM provider, set the API URL to {{BASE_URL}} and the API key to your client secret.',
    docsUrl: 'https://docs.onyx.app',
    supportsMcp: true,
    supportsLlm: true,
  },

  // === Automation & Notebooks ===
  {
    id: 'marimo',
    name: 'marimo',
    description: 'Reactive Python notebook with AI chat and code completion.',
    category: 'automation',
    icon: 'marimo',
    defaultMode: 'llm_only',
    setupType: 'generic',
    manualInstructions: 'In marimo: User Settings > AI tab > configure an OpenAI-compatible provider with base URL {{BASE_URL}} and your client secret as the API key.',
    docsUrl: 'https://marimo.io',
    supportsMcp: false,
    supportsLlm: true,
    binaryNames: ['marimo'],
  },
  {
    id: 'n8n',
    name: 'n8n',
    description: 'Low-code workflow automation with AI nodes.',
    category: 'automation',
    icon: 'n8n',
    defaultMode: 'llm_only',
    setupType: 'generic',
    manualInstructions: 'In n8n: Create Credential > select OpenAI-compatible > set Base URL to {{BASE_URL}} and API key to your client secret.',
    docsUrl: 'https://docs.n8n.io',
    supportsMcp: false,
    supportsLlm: true,
  },
]

export const CUSTOM_CLIENT_TEMPLATE: ClientTemplate = {
  id: 'custom',
  name: 'Custom',
  description: 'Manual setup for any OpenAI-compatible application.',
  category: 'cli',
  icon: 'custom',
  defaultMode: 'both',
  setupType: 'generic',
  supportsMcp: true,
  supportsLlm: true,
}

// Category metadata for display
const CATEGORY_INFO: Record<ClientTemplateCategory, { title: string; description: string }> = {
  coding_assistants: { title: 'Coding Assistants', description: 'CLI-based AI coding tools' },
  ide_extensions: { title: 'IDE Extensions', description: 'AI coding extensions for VS Code' },
  ides: { title: 'IDEs & Editors', description: 'AI-powered code editors' },
  chat: { title: 'Chat UIs', description: 'Web-based chat interfaces' },
  automation: { title: 'Automation & Notebooks', description: 'Workflow automation and data science' },
  cli: { title: 'Other', description: 'Custom and generic clients' },
}

const CATEGORY_ORDER: ClientTemplateCategory[] = [
  'coding_assistants',
  'ide_extensions',
  'ides',
  'chat',
  'automation',
  'cli',
]

interface ClientTemplatesProps {
  onSelectTemplate: (template: ClientTemplate) => void
}

export const ClientTemplates: React.FC<ClientTemplatesProps> = ({ onSelectTemplate }) => {
  // Group templates by category
  const templatesByCategory = CATEGORY_ORDER.reduce((acc, category) => {
    const templates = CLIENT_TEMPLATES.filter(t => t.category === category)
    if (templates.length > 0) {
      acc[category] = templates
    }
    return acc
  }, {} as Record<ClientTemplateCategory, ClientTemplate[]>)

  const TemplateButton = ({ template }: { template: ClientTemplate }) => (
    <button
      onClick={() => onSelectTemplate(template)}
      className="flex flex-col items-center gap-2 p-4 rounded-lg border-2 border-muted hover:border-primary hover:bg-accent transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2"
    >
      <ServiceIcon service={template.id} size={40} />
      <div className="text-center">
        <p className="font-medium text-sm">{template.name}</p>
        <p className="text-xs text-muted-foreground line-clamp-2 mt-0.5">
          {template.description}
        </p>
      </div>
    </button>
  )

  return (
    <div className="space-y-6">
      {CATEGORY_ORDER.map(category => {
        const templates = templatesByCategory[category]
        if (!templates) return null
        const info = CATEGORY_INFO[category]
        return (
          <div key={category} className="space-y-3">
            <div>
              <h3 className="text-sm font-semibold">{info.title}</h3>
              <p className="text-xs text-muted-foreground">{info.description}</p>
            </div>
            <div className="grid grid-cols-2 sm:grid-cols-3 gap-3">
              {templates.map((template) => (
                <TemplateButton key={template.id} template={template} />
              ))}
            </div>
          </div>
        )
      })}

    </div>
  )
}

/** Resolve placeholders in template strings */
export function resolveTemplatePlaceholders(
  text: string,
  baseUrl: string,
  clientSecret: string,
  clientId: string,
  homeDir?: string,
  configDir?: string,
): string {
  let result = text
    .replace(/\{\{BASE_URL\}\}/g, baseUrl)
    .replace(/\{\{CLIENT_SECRET\}\}/g, clientSecret)
    .replace(/\{\{CLIENT_ID\}\}/g, clientId)
  if (homeDir) {
    result = result.replace(/\{\{HOME_DIR\}\}/g, homeDir)
  }
  if (configDir) {
    result = result.replace(/\{\{CONFIG_DIR\}\}/g, configDir)
  }
  return result
}
