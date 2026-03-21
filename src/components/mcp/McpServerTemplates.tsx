import React, { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import ServiceIcon from '../ServiceIcon'
import { Tooltip, TooltipTrigger, TooltipContent, TooltipProvider } from '../ui/tooltip'
import { cn } from '@/lib/utils'
import type { AvailableRuntimes } from '@/types/tauri-commands'

export type McpTemplateCategory = 'version_control' | 'productivity' | 'files_data' | 'databases' | 'search_web' | 'cloud_infra' | 'utilities' | 'development'

export interface TemplateField {
  /** For env_var: the env var key. For arg: used as {{id}} placeholder in args */
  id: string
  label: string
  placeholder: string
  type: 'env_var' | 'arg'
  secret?: boolean
  /** Default: true */
  required?: boolean
  helpText?: string
  defaultValue?: string
}

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
  /** User-configurable fields shown in the simplified template form */
  fields?: TemplateField[]
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
    description: 'Reference GitHub MCP server for repositories, issues, and pull requests using a personal access token.',
    category: 'version_control',
    icon: '🐙',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-github'],
    authMethod: 'none',
    fields: [
      { id: 'GITHUB_PERSONAL_ACCESS_TOKEN', label: 'Personal Access Token', type: 'env_var', placeholder: 'ghp_...', secret: true, helpText: "Create at https://github.com/settings/tokens with 'repo' scope" },
    ],
    setupInstructions: "Create a GitHub Personal Access Token (classic) at https://github.com/settings/tokens with at least 'repo' (and optionally 'read:user') scope.",
    docsUrl: 'https://www.npmjs.com/package/@modelcontextprotocol/server-github',
  },
  {
    id: 'git',
    name: 'Git',
    description: 'Git MCP server exposing tools for status, diff, add, commit, branch, and more against local repositories.',
    category: 'version_control',
    icon: '🔗',
    transport: 'Stdio',
    command: 'uvx',
    args: ['mcp-server-git', '--repository', '{{repo_path}}'],
    authMethod: 'none',
    fields: [
      { id: 'repo_path', label: 'Repository Path', type: 'arg', placeholder: '/path/to/your/git/repo' },
    ],
    setupInstructions: 'Point this server at a local Git repository. No auth env vars are required.',
    docsUrl: 'https://pypi.org/project/mcp-server-git/',
  },
  {
    id: 'git-mcp-server',
    name: 'Git MCP Server (@cyanheads)',
    description: 'Community Git MCP server with 25+ tools covering clone, branch, diff, log, stash, worktrees, and more.',
    category: 'version_control',
    icon: '🔗',
    transport: 'Stdio',
    command: 'npx',
    args: ['@cyanheads/git-mcp-server'],
    authMethod: 'none',
    setupInstructions: 'Ensure Git is installed. Add server with command: npx, args: [@cyanheads/git-mcp-server]. Optionally set MCP_LOG_LEVEL and GIT_SIGN_COMMITS env vars. Uses your local Git credentials.',
    docsUrl: 'https://www.npmjs.com/package/@cyanheads/git-mcp-server',
  },

  // === Productivity & Collaboration ===
  {
    id: 'notion',
    name: 'Notion',
    description: 'Official Notion MCP server exposing the Notion API (search, databases, pages, comments) to MCP clients.',
    category: 'productivity',
    icon: '📑',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@notionhq/notion-mcp-server'],
    authMethod: 'none',
    fields: [
      { id: 'NOTION_TOKEN', label: 'Integration Secret', type: 'env_var', placeholder: 'ntn_...', secret: true, helpText: 'Create at https://www.notion.so/profile/integrations' },
    ],
    setupInstructions: 'Create an internal Notion integration and copy its secret. Share relevant pages/databases with that integration.',
    docsUrl: 'https://github.com/makenotion/notion-mcp-server',
  },

  {
    id: 'google-workspace',
    name: 'Google Workspace',
    description: 'Official Google Workspace CLI exposing Drive, Gmail, Calendar, Docs, Sheets, and more as MCP tools via stdio.',
    category: 'productivity',
    icon: '🏢',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@googleworkspace/cli', 'mcp'],
    authMethod: 'none',
    setupInstructions: 'Install via npx @googleworkspace/cli. Requires Node.js 18+ and a Google Cloud project with OAuth credentials. Run "gws auth login" first to authenticate. Use args like ["mcp", "-s", "drive,gmail,calendar"] to limit exposed services, or ["mcp", "-s", "all"] for everything. Add "--tool-mode", "compact" for fewer tools (~26 vs 200+).',
    docsUrl: 'https://github.com/googleworkspace/cli',
  },

  // === File & Data Access ===
  {
    id: 'filesystem',
    name: 'Filesystem',
    description: 'Secure filesystem access with configurable allowed directories and rich file tooling (list, read, write, move, search, metadata).',
    category: 'files_data',
    icon: '📂',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-filesystem', '{{directory}}'],
    authMethod: 'none',
    fields: [
      { id: 'directory', label: 'Allowed Directory', type: 'arg', placeholder: '/path/to/directory', defaultValue: HOME_DIR_PLACEHOLDER },
    ],
    setupInstructions: 'Choose which local directory the AI should be allowed to access.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem',
  },

  // === Databases ===
  {
    id: 'postgres',
    name: 'PostgreSQL',
    description: 'Read-only PostgreSQL server that exposes schema metadata as resources and a query tool for safe SQL queries.',
    category: 'databases',
    icon: '🐘',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-postgres', '{{connection_url}}'],
    authMethod: 'none',
    fields: [
      { id: 'connection_url', label: 'Connection URL', type: 'arg', placeholder: 'postgresql://user:pass@host:5432/dbname' },
    ],
    setupInstructions: 'Provide a PostgreSQL connection URL. Credentials are embedded in the URL.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers/tree/main/src/postgres',
  },
  {
    id: 'supabase-hosted',
    name: 'Supabase (hosted)',
    description: 'Hosted Supabase MCP server that connects AI tools to Supabase projects over HTTP with browser-based OAuth.',
    category: 'databases',
    icon: '🗄️',
    transport: 'Sse',
    url: 'https://mcp.supabase.com/mcp',
    authMethod: 'oauth_browser',
    setupInstructions: 'Configure a remote server with url: "https://mcp.supabase.com/mcp". The client will open a browser window to log into Supabase and grant org/project access. For CI, generate a personal access token and send as Authorization header.',
    docsUrl: 'https://supabase.com/docs/guides/getting-started/mcp',
  },
  {
    id: 'supabase-npx',
    name: 'Supabase (local)',
    description: 'Community-maintained Supabase MCP server package that connects to a Supabase project via access token.',
    category: 'databases',
    icon: '🗄️',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@supabase/mcp-server-supabase@latest', '--read-only', '--project-ref={{project_ref}}'],
    authMethod: 'none',
    fields: [
      { id: 'project_ref', label: 'Project Reference', type: 'arg', placeholder: 'your-project-ref' },
      { id: 'SUPABASE_ACCESS_TOKEN', label: 'Access Token', type: 'env_var', placeholder: 'sbp_...', secret: true, helpText: 'Create in your Supabase dashboard' },
    ],
    setupInstructions: 'Create a Supabase personal access token in your dashboard. Read-only mode is enabled by default.',
    docsUrl: 'https://github.com/supabase-community/supabase-mcp',
  },

  // === Search & Web ===
  {
    id: 'brave-search',
    name: 'Brave Search',
    description: 'Official Brave Search MCP server with web, local, image, video, news search and AI summarization.',
    category: 'search_web',
    icon: '🦁',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@brave/brave-search-mcp-server', '--transport', 'stdio'],
    authMethod: 'none',
    fields: [
      { id: 'BRAVE_API_KEY', label: 'API Key', type: 'env_var', placeholder: 'BSA...', secret: true, helpText: 'Get from https://brave.com/search/api/' },
    ],
    setupInstructions: 'Sign up for a Brave Search API account and generate an API key from the developer dashboard.',
    docsUrl: 'https://github.com/brave/brave-search-mcp-server',
  },
  {
    id: 'fetch',
    name: 'Fetch',
    description: 'Python-based web fetcher that retrieves URLs and converts HTML to markdown, with options for truncation and proxy tuning.',
    category: 'search_web',
    icon: '🌐',
    transport: 'Stdio',
    command: 'uvx',
    args: ['mcp-server-fetch'],
    authMethod: 'none',
    setupInstructions: 'Install uv or Python 3.8+. Use uvx mcp-server-fetch as the command. No auth env vars required. Add --user-agent=... or --proxy-url=... args for customization.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers/tree/main/src/fetch',
  },

  // === Cloud & Infrastructure ===
  {
    id: 'aws-core',
    name: 'AWS',
    description: 'Official AWS Labs Core MCP server with tools for AWS service operations, resource management, and CLI integration.',
    category: 'cloud_infra',
    icon: '☁️',
    transport: 'Stdio',
    command: 'uvx',
    args: ['awslabs-core-mcp-server'],
    authMethod: 'none',
    setupInstructions: 'Requires Python 3.10+ and uv. Uses your existing AWS credentials (AWS_PROFILE, AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY, or SSO). Set env: { "AWS_PROFILE": "your-profile" } or configure credentials via aws configure. Optionally set AWS_REGION.',
    docsUrl: 'https://github.com/awslabs/mcp/tree/main/src/core-mcp-server',
  },
  {
    id: 'aws-docs',
    name: 'AWS Documentation',
    description: 'AWS Labs Documentation MCP server for searching and reading official AWS documentation, best practices, and API references.',
    category: 'cloud_infra',
    icon: '📖',
    transport: 'Stdio',
    command: 'uvx',
    args: ['awslabs-aws-documentation-mcp-server'],
    authMethod: 'none',
    setupInstructions: 'Requires Python 3.10+ and uv. No AWS credentials needed — this server reads public AWS documentation. Run via uvx awslabs-aws-documentation-mcp-server.',
    docsUrl: 'https://github.com/awslabs/mcp/tree/main/src/aws-documentation-mcp-server',
  },
  {
    id: 'gcloud',
    name: 'Google Cloud',
    description: 'Official Google Cloud MCP server for interacting with GCP APIs including Compute, Storage, IAM, and more.',
    category: 'cloud_infra',
    icon: '🌤️',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@google-cloud/gcloud-mcp'],
    authMethod: 'none',
    setupInstructions: 'Requires Node.js 18+ and the gcloud CLI. Authenticate first with "gcloud auth application-default login". Set env: { "GCLOUD_PROJECT": "your-project-id" } to target a specific project. Uses Application Default Credentials.',
    docsUrl: 'https://github.com/GoogleCloudPlatform/gcloud-mcp',
  },
  {
    id: 'kubernetes',
    name: 'Kubernetes',
    description: 'MCP server for interacting with Kubernetes clusters via kubectl — manage pods, deployments, services, and more.',
    category: 'cloud_infra',
    icon: '☸️',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', 'mcp-server-kubernetes'],
    authMethod: 'none',
    setupInstructions: 'Requires kubectl configured with a valid kubeconfig. Uses your current kubectl context by default. Set env: { "KUBECONFIG": "/path/to/kubeconfig" } to use a specific config file.',
    docsUrl: 'https://github.com/Flux159/mcp-server-kubernetes',
  },
  {
    id: 'docker',
    name: 'Docker',
    description: 'MCP server for managing Docker containers — run commands, view logs, monitor resources, and manage container lifecycle.',
    category: 'cloud_infra',
    icon: '🐳',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', 'mcp-server-docker'],
    authMethod: 'none',
    setupInstructions: 'Requires Docker to be installed and running. The server uses your local Docker socket. No additional auth needed. Docker Desktop users can also use the built-in "docker mcp" gateway.',
    docsUrl: 'https://github.com/adamdude828/mcp-server-docker',
  },
  {
    id: 'cloudflare',
    name: 'Cloudflare',
    description: 'Official Cloudflare MCP server for managing Workers, KV, R2, D1, and other Cloudflare services.',
    category: 'cloud_infra',
    icon: '🔶',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@cloudflare/mcp-server-cloudflare'],
    authMethod: 'none',
    fields: [
      { id: 'CLOUDFLARE_API_TOKEN', label: 'API Token', type: 'env_var', placeholder: 'your-cloudflare-api-token', secret: true, helpText: 'Create at https://dash.cloudflare.com/profile/api-tokens' },
    ],
    setupInstructions: 'Generate a Cloudflare API token with appropriate permissions.',
    docsUrl: 'https://github.com/cloudflare/mcp-server-cloudflare',
  },

  // === Utilities ===
  {
    id: 'time',
    name: 'Time',
    description: 'Time MCP server providing current time and timezone conversion using IANA timezone names with automatic system timezone detection.',
    category: 'utilities',
    icon: '⏰',
    transport: 'Stdio',
    command: 'uvx',
    args: ['mcp-server-time'],
    authMethod: 'none',
    setupInstructions: 'Install Python and run via uvx mcp-server-time. Optionally configure timezone via --local-timezone argument or LOCAL_TIMEZONE env var. No API keys required.',
    docsUrl: 'https://pypi.org/project/mcp-server-time/',
  },
  {
    id: 'sequential-thinking',
    name: 'Sequential Thinking',
    description: 'Reasoning-focused MCP server that structures multi-step thought processes with branching and revision for complex problem solving.',
    category: 'utilities',
    icon: '🧠',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-sequential-thinking'],
    authMethod: 'none',
    setupInstructions: 'Run npx -y @modelcontextprotocol/server-sequential-thinking. Exposes a sequential_thinking tool for managing thought steps. No external auth needed.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers/tree/main/src/sequentialthinking',
  },

  // === Development & Testing ===
  {
    id: 'everything',
    name: 'Everything Demo',
    description: 'Reference/test MCP server that exercises prompts, tools, resources, and transports; ideal for learning and debugging.',
    category: 'development',
    icon: '🧪',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-everything'],
    authMethod: 'none',
    setupInstructions: 'Install Node.js. Run npx -y @modelcontextprotocol/server-everything as a stdio server. No authentication or environment variables required.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers/tree/main/src/everything',
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
  cloud_infra: { title: 'Cloud & Infrastructure', description: 'Cloud platforms, containers, and infrastructure management' },
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
  'cloud_infra',
  'utilities',
  'development',
]

/** Check if a template's command requires a specific runtime */
function getRequiredRuntime(template: McpServerTemplate): keyof AvailableRuntimes | null {
  if (template.command === 'npx') return 'npx'
  if (template.command === 'uvx') return 'uvx'
  if (template.command === 'docker') return 'docker'
  return null
}

const RUNTIME_LABELS: Record<keyof AvailableRuntimes, { name: string; install: string }> = {
  npx: { name: 'npx (Node.js)', install: 'Install Node.js from https://nodejs.org' },
  uvx: { name: 'uvx (uv/Python)', install: 'Install uv from https://docs.astral.sh/uv' },
  docker: { name: 'Docker', install: 'Install Docker from https://docker.com' },
}

export const McpServerTemplates: React.FC<McpServerTemplatesProps> = ({ onSelectTemplate }) => {
  const isDev = import.meta.env.DEV
  const visibleTemplates = MCP_SERVER_TEMPLATES.filter(t => !t.devOnly || isDev)
  const [homeDir, setHomeDir] = useState<string | null>(null)
  const [runtimes, setRuntimes] = useState<AvailableRuntimes | null>(null)

  // Fetch user's home directory and available runtimes on mount
  useEffect(() => {
    invoke<string>('get_home_dir')
      .then(setHomeDir)
      .catch((err) => console.error('Failed to get home directory:', err))
    invoke<AvailableRuntimes>('detect_available_runtimes')
      .then(setRuntimes)
      .catch((err) => console.error('Failed to detect runtimes:', err))
  }, [])

  // Replace placeholders in template args and field defaults with actual values
  const resolveTemplate = (template: McpServerTemplate): McpServerTemplate => {
    if (!homeDir) return template

    const resolvedArgs = template.args?.map(arg =>
      arg.includes(HOME_DIR_PLACEHOLDER) ? arg.replace(HOME_DIR_PLACEHOLDER, homeDir) : arg
    )

    const resolvedFields = template.fields?.map(field => ({
      ...field,
      defaultValue: field.defaultValue?.replace(HOME_DIR_PLACEHOLDER, homeDir),
    }))

    return {
      ...template,
      ...(resolvedArgs && { args: resolvedArgs }),
      ...(resolvedFields && { fields: resolvedFields }),
    }
  }

  const handleSelectTemplate = (template: McpServerTemplate) => {
    onSelectTemplate(resolveTemplate(template))
  }

  const isRuntimeAvailable = (template: McpServerTemplate): boolean => {
    if (!runtimes) return true // Still loading, assume available
    const required = getRequiredRuntime(template)
    if (!required) return true // SSE or no command needed
    return runtimes[required]
  }

  // Group templates by category (show all, disabled ones handled per-button)
  const templatesByCategory = CATEGORY_ORDER.reduce((acc, category) => {
    const templates = visibleTemplates.filter(t => t.category === category)
    if (templates.length > 0) {
      acc[category] = templates
    }
    return acc
  }, {} as Record<McpTemplateCategory, McpServerTemplate[]>)

  const TemplateButton = ({ template }: { template: McpServerTemplate }) => {
    const available = isRuntimeAvailable(template)
    const requiredRuntime = getRequiredRuntime(template)
    const runtimeInfo = requiredRuntime ? RUNTIME_LABELS[requiredRuntime] : null

    const button = (
      <button
        onClick={() => available && handleSelectTemplate(template)}
        disabled={!available}
        className={cn(
          "flex flex-col items-center gap-2 p-4 rounded-lg border-2 transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2",
          available
            ? "border-muted hover:border-primary hover:bg-accent"
            : "border-muted opacity-50 cursor-not-allowed"
        )}
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

    if (!available && runtimeInfo) {
      return (
        <Tooltip>
          <TooltipTrigger asChild>{button}</TooltipTrigger>
          <TooltipContent side="top" className="max-w-64">
            <p>Requires {runtimeInfo.name}</p>
            <p className="text-xs text-muted-foreground">{runtimeInfo.install}</p>
          </TooltipContent>
        </Tooltip>
      )
    }

    return button
  }

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
    <TooltipProvider delayDuration={300}>
      <div className="space-y-6">
        {CATEGORY_ORDER.map(category => {
          const templates = templatesByCategory[category]
          if (!templates) return null
          return <TemplateSection key={category} category={category} templates={templates} />
        })}
      </div>
    </TooltipProvider>
  )
}
