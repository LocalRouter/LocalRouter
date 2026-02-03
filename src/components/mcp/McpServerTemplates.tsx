import React, { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import ServiceIcon from '../ServiceIcon'

export type McpTemplateCategory = 'version_control' | 'productivity' | 'files_data' | 'databases' | 'search_web' | 'utilities' | 'development'

export interface McpServerTemplate {
  id: string
  name: string
  description: string
  /** Category for grouping in the UI */
  category: McpTemplateCategory
  /** Template ID used for icon lookup (e.g., 'github', 'slack') */
  icon: string
  transport: 'Stdio' | 'Sse'
  command?: string
  args?: string[]
  url?: string
  authMethod: 'none' | 'bearer' | 'oauth_browser'
  defaultScopes?: string[]
  setupInstructions?: string
  docsUrl?: string
  devOnly?: boolean
}

// Placeholder that will be replaced with the user's home directory
const HOME_DIR_PLACEHOLDER = '{{HOME_DIR}}'

export const MCP_SERVER_TEMPLATES: McpServerTemplate[] = [
  // === Version Control & Code ===
  {
    id: 'github',
    name: 'GitHub',
    description: 'Repositories, issues, pull requests, and code search',
    category: 'version_control',
    icon: '🐙',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-github'],
    authMethod: 'bearer',
    setupInstructions: "Create a Personal Access Token at https://github.com/settings/tokens with 'repo' scope. Add as GITHUB_PERSONAL_ACCESS_TOKEN environment variable.",
    docsUrl: 'https://github.com/modelcontextprotocol/servers',
    defaultScopes: ['repo', 'read:user'],
  },
  {
    id: 'gitlab',
    name: 'GitLab',
    description: 'Projects, merge requests, and CI/CD pipelines',
    category: 'version_control',
    icon: '🦊',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-gitlab'],
    authMethod: 'bearer',
    setupInstructions: 'Create a Personal Access Token at gitlab.com/-/user_settings/personal_access_tokens with api and read_user scopes. Add as GITLAB_PERSONAL_ACCESS_TOKEN environment variable.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers/tree/main/src/gitlab',
  },
  {
    id: 'git',
    name: 'Git',
    description: 'Local Git repository operations',
    category: 'version_control',
    icon: '🔗',
    transport: 'Stdio',
    command: 'uvx',
    args: ['mcp-server-git'],
    authMethod: 'none',
    setupInstructions: 'No credentials needed. Operates on local Git repositories.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers',
  },

  // === Productivity & Collaboration ===
  {
    id: 'slack',
    name: 'Slack',
    description: 'Messages, channels, and workspace history',
    category: 'productivity',
    icon: '💬',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-slack'],
    authMethod: 'bearer',
    setupInstructions: 'Create a Bot User OAuth Token at https://api.slack.com/apps with scopes: channels:history, channels:read, chat:write, reactions:write, users:read, users:read.email. Add as SLACK_BOT_TOKEN environment variable.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers',
    defaultScopes: ['channels:history', 'channels:read', 'chat:write', 'reactions:write', 'users:read', 'users:read.email'],
  },
  {
    id: 'jira',
    name: 'Jira',
    description: 'Issues, workflows, and project management',
    category: 'productivity',
    icon: '📋',
    transport: 'Stdio',
    command: 'uvx',
    args: ['jira-mcp'],
    authMethod: 'bearer',
    setupInstructions: 'Create an API token at https://id.atlassian.com/manage-profile/security/api-tokens. Add JIRA_HOST (your-domain.atlassian.net), JIRA_EMAIL, and JIRA_API_TOKEN environment variables.',
    docsUrl: 'https://github.com/CamdenClark/jira-mcp',
    defaultScopes: ['read:jira-user', 'read:jira-work', 'write:jira-work', 'read:me'],
  },
  {
    id: 'notion',
    name: 'Notion',
    description: 'Pages, databases, and workspace content',
    category: 'productivity',
    icon: '📑',
    transport: 'Stdio',
    command: 'uvx',
    args: ['mcp-notion'],
    authMethod: 'bearer',
    setupInstructions: 'Create an integration at https://www.notion.so/my-integrations. Copy the Internal Integration Secret as NOTION_API_KEY. Optional: set NOTION_VERSION (default 2022-06-28).',
    docsUrl: 'https://github.com/notion-mcp/notion-mcp',
    defaultScopes: ['read:user:email', 'read:database', 'read:page'],
  },
  {
    id: 'confluence',
    name: 'Confluence',
    description: 'Pages, spaces, and documentation',
    category: 'productivity',
    icon: '📄',
    transport: 'Stdio',
    command: 'uvx',
    args: ['confluence-mcp-server'],
    authMethod: 'bearer',
    setupInstructions: 'Create API token at https://id.atlassian.com/manage-profile/security/api-tokens. Add CONFLUENCE_BASE_URL, CONFLUENCE_USERNAME/EMAIL, and CONFLUENCE_API_TOKEN environment variables.',
    docsUrl: 'https://github.com/aaronsb/confluence-cloud-mcp',
    defaultScopes: ['read:confluence', 'write:confluence'],
  },
  {
    id: 'google-drive',
    name: 'Google Drive',
    description: 'Files and documents in Google Drive',
    category: 'productivity',
    icon: '📁',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-gdrive'],
    authMethod: 'oauth_browser',
    setupInstructions: 'Create a Google Cloud project, enable Google Drive API, create OAuth 2.0 Desktop credentials. Download JSON credentials, rename to gcp-oauth.keys.json, and run authentication flow.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers',
  },
  {
    id: 'figma',
    name: 'Figma',
    description: 'Design files and layers for code generation',
    category: 'productivity',
    icon: '🎨',
    transport: 'Sse',
    url: 'https://mcp.figma.com/sse',
    authMethod: 'oauth_browser',
    setupInstructions: 'For remote: URL-based OAuth authentication. For local: Enable desktop MCP server in Figma app Dev Mode, access at http://127.0.0.1:3845/sse',
    docsUrl: 'https://help.figma.com/hc/en-us/articles/35281350665623',
  },

  // === File & Data Access ===
  {
    id: 'filesystem',
    name: 'Filesystem',
    description: 'Local file read, write, and manipulation',
    category: 'files_data',
    icon: '📂',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-filesystem', HOME_DIR_PLACEHOLDER],
    authMethod: 'none',
    setupInstructions: 'Provide path to allowed directory. Server restricts access to this directory and subdirectories.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers',
  },
  {
    id: 'obsidian',
    name: 'Obsidian',
    description: 'Markdown notes in Obsidian vaults',
    category: 'files_data',
    icon: '🧠',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', 'obsidian-mcp-server', '/path/to/your/vault'],
    authMethod: 'none',
    setupInstructions: 'Point to your local Obsidian vault directory. No credentials needed.',
    docsUrl: 'https://github.com/marcelmarais/obsidian-mcp-server',
  },

  // === Databases ===
  {
    id: 'postgres',
    name: 'PostgreSQL',
    description: 'SQL queries and database analysis',
    category: 'databases',
    icon: '🐘',
    transport: 'Stdio',
    command: 'uvx',
    args: ['mcp-server-postgres'],
    authMethod: 'bearer',
    setupInstructions: 'Create environment variable DATABASE_URL=postgresql://user:password@host/database. Optional: PG_HOST, PG_PORT, PG_DATABASE, PG_USER, PG_PASSWORD.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers',
  },
  {
    id: 'sqlite',
    name: 'SQLite',
    description: 'Local SQLite database queries',
    category: 'databases',
    icon: '💾',
    transport: 'Stdio',
    command: 'uvx',
    args: ['mcp-sqlite', '/path/to/database.db'],
    authMethod: 'none',
    setupInstructions: 'Provide path to your SQLite database file. No credentials needed.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers',
  },
  {
    id: 'supabase',
    name: 'Supabase',
    description: 'PostgreSQL, storage, and Supabase services',
    category: 'databases',
    icon: '🗄️',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@supabase/mcp-server-supabase@latest', '--read-only'],
    authMethod: 'bearer',
    setupInstructions: 'Create personal access token at https://supabase.com/dashboard/account/tokens. Add SUPABASE_ACCESS_TOKEN environment variable. Add --project-ref=<your-project-ref> to args.',
    docsUrl: 'https://github.com/supabase/supabase-mcp-server',
    defaultScopes: ['read:database', 'read:storage'],
  },

  // === Search & Web ===
  {
    id: 'perplexity',
    name: 'Perplexity',
    description: 'Web search with citations',
    category: 'search_web',
    icon: '🔍',
    transport: 'Stdio',
    command: 'uvx',
    args: ['perplexity-mcp'],
    authMethod: 'bearer',
    setupInstructions: 'Get API key from https://www.perplexity.ai/settings/api. Add PERPLEXITY_API_KEY environment variable. Set PERPLEXITY_MODEL (e.g., sonar).',
    docsUrl: 'https://docs.perplexity.ai/guides/mcp-server',
    defaultScopes: ['search:web'],
  },
  {
    id: 'brave-search',
    name: 'Brave Search',
    description: 'Web search via Brave Search API',
    category: 'search_web',
    icon: '🦁',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-brave-search'],
    authMethod: 'bearer',
    setupInstructions: 'Get API key from https://brave.com/search/api and add as BRAVE_API_KEY environment variable.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers',
  },
  {
    id: 'fetch',
    name: 'Fetch',
    description: 'Web content to markdown conversion',
    category: 'search_web',
    icon: '🌐',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-fetch'],
    authMethod: 'none',
    setupInstructions: 'No credentials needed. Fetches web content and converts to markdown format.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers',
  },

  // === Utilities ===
  {
    id: 'time',
    name: 'Time',
    description: 'Timezone conversion utilities',
    category: 'utilities',
    icon: '⏰',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-time'],
    authMethod: 'none',
    setupInstructions: 'No credentials needed. Provides timezone and time conversion tools.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers',
  },
  {
    id: 'memory',
    name: 'Memory',
    description: 'Persistent knowledge graph memory',
    category: 'utilities',
    icon: '💭',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-memory'],
    authMethod: 'none',
    setupInstructions: 'No credentials needed. Maintains persistent semantic knowledge graphs.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers',
  },

  // === Development & Testing ===
  {
    id: 'everything',
    name: 'Everything Demo',
    description: 'Reference server for testing MCP features',
    category: 'development',
    icon: '🧪',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-everything'],
    authMethod: 'none',
    setupInstructions: 'No credentials needed. For testing and protocol feature demonstration.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers',
  },
]

export const CUSTOM_MCP_TEMPLATE: McpServerTemplate = {
  id: 'custom',
  name: 'Custom',
  description: 'Configure a custom MCP server manually',
  category: 'utilities',
  icon: '⚙️',
  transport: 'Stdio',
  authMethod: 'none',
}

interface McpServerTemplatesProps {
  onSelectTemplate: (template: McpServerTemplate) => void
}

// Category metadata for display
const CATEGORY_INFO: Record<McpTemplateCategory, { title: string; description: string }> = {
  version_control: { title: 'Version Control', description: 'Git repositories, code hosting, and version control systems' },
  productivity: { title: 'Productivity & Collaboration', description: 'Team communication, project management, and documentation' },
  files_data: { title: 'Files & Data', description: 'Local file access and note-taking systems' },
  databases: { title: 'Databases', description: 'SQL databases and data storage services' },
  search_web: { title: 'Search & Web', description: 'Web search and content retrieval' },
  utilities: { title: 'Utilities', description: 'Helper tools and system utilities' },
  development: { title: 'Development & Testing', description: 'Development tools and testing servers' },
}

// Order categories for display
const CATEGORY_ORDER: McpTemplateCategory[] = [
  'version_control',
  'productivity',
  'files_data',
  'databases',
  'search_web',
  'utilities',
  'development',
]

export const McpServerTemplates: React.FC<McpServerTemplatesProps> = ({ onSelectTemplate }) => {
  const isDev = import.meta.env.DEV
  const visibleTemplates = MCP_SERVER_TEMPLATES.filter(t => !t.devOnly || isDev)
  const [homeDir, setHomeDir] = useState<string | null>(null)

  // Fetch user's home directory on mount
  useEffect(() => {
    invoke<string>('get_home_dir')
      .then(setHomeDir)
      .catch((err) => console.error('Failed to get home directory:', err))
  }, [])

  // Replace placeholders in template args with actual values
  const resolveTemplate = (template: McpServerTemplate): McpServerTemplate => {
    if (!template.args || !homeDir) return template

    const resolvedArgs = template.args.map(arg =>
      arg === HOME_DIR_PLACEHOLDER ? homeDir : arg
    )

    return { ...template, args: resolvedArgs }
  }

  const handleSelectTemplate = (template: McpServerTemplate) => {
    onSelectTemplate(resolveTemplate(template))
  }

  // Group templates by category
  const templatesByCategory = CATEGORY_ORDER.reduce((acc, category) => {
    const templates = visibleTemplates.filter(t => t.category === category)
    if (templates.length > 0) {
      acc[category] = templates
    }
    return acc
  }, {} as Record<McpTemplateCategory, McpServerTemplate[]>)

  const TemplateButton = ({ template }: { template: McpServerTemplate }) => (
    <button
      onClick={() => handleSelectTemplate(template)}
      className="flex flex-col items-center gap-2 p-4 rounded-lg border-2 border-muted hover:border-primary hover:bg-accent transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2"
    >
      <ServiceIcon service={template.id} size={40} fallbackToServerIcon />
      <div className="text-center">
        <p className="font-medium text-sm">{template.name}</p>
        <p className="text-xs text-muted-foreground line-clamp-2 mt-0.5">
          {template.description}
        </p>
      </div>
    </button>
  )

  const TemplateSection = ({ category, templates }: {
    category: McpTemplateCategory
    templates: McpServerTemplate[]
  }) => {
    const info = CATEGORY_INFO[category]
    return (
      <div className="space-y-3">
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
  }

  return (
    <div className="space-y-6">
      {CATEGORY_ORDER.map(category => {
        const templates = templatesByCategory[category]
        if (!templates) return null
        return <TemplateSection key={category} category={category} templates={templates} />
      })}
    </div>
  )
}
