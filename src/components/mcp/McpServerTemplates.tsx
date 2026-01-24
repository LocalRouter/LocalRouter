import React, { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'

export interface McpServerTemplate {
  id: string
  name: string
  description: string
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
  {
    id: 'github',
    name: 'GitHub Copilot MCP Server',
    description: 'Access GitHub repositories, issues, PRs, and workflows via Copilot',
    icon: '🐙',
    transport: 'Sse',
    url: 'https://api.githubcopilot.com/mcp',
    authMethod: 'oauth_browser',
    defaultScopes: ['repo', 'read:user'],
    setupInstructions: 'Create a GitHub OAuth App at github.com/settings/developers with callback URL: http://localhost:8080/callback',
    docsUrl: 'https://docs.github.com/en/copilot',
  },
  {
    id: 'gitlab',
    name: 'GitLab MCP Server',
    description: 'Manage GitLab projects, merge requests, and CI/CD pipelines',
    icon: '🦊',
    transport: 'Sse',
    url: 'https://gitlab.com/api/v4/mcp',
    authMethod: 'oauth_browser',
    defaultScopes: ['api', 'read_user'],
    setupInstructions: 'Create a GitLab application at gitlab.com/-/profile/applications with callback URL: http://localhost:8080/callback',
    docsUrl: 'https://docs.gitlab.com/ee/integration/oauth_provider.html',
  },
  {
    id: 'filesystem',
    name: 'Filesystem MCP Server',
    description: 'Access local files and directories',
    icon: '📁',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-filesystem', HOME_DIR_PLACEHOLDER],
    authMethod: 'none',
    setupInstructions: 'Change the path to the directory you want to give access to',
    docsUrl: 'https://github.com/modelcontextprotocol/servers',
  },
  {
    id: 'everything',
    name: 'Everything MCP Server',
    description: 'All-in-one MCP server with multiple capabilities for testing',
    icon: '🧪',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-everything'],
    authMethod: 'none',
    setupInstructions: 'No additional setup required. This server provides prompts, resources, and tools for testing MCP integrations.',
    docsUrl: 'https://github.com/modelcontextprotocol/servers/tree/main/src/everything',
    devOnly: true,
  },
  {
    id: 'postgres',
    name: 'PostgreSQL MCP Server',
    description: 'Query and manage PostgreSQL databases',
    icon: '🐘',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-postgres'],
    authMethod: 'none',
    setupInstructions: 'Configure database connection in environment variables after creation',
    docsUrl: 'https://github.com/modelcontextprotocol/servers',
  },
  {
    id: 'brave-search',
    name: 'Brave Search MCP Server',
    description: 'Web search capabilities via Brave Search API',
    icon: '🔍',
    transport: 'Stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-brave-search'],
    authMethod: 'none',
    setupInstructions: 'Get API key from brave.com/search/api and add as environment variable',
    docsUrl: 'https://github.com/modelcontextprotocol/servers',
  },
]

interface McpServerTemplatesProps {
  onSelectTemplate: (template: McpServerTemplate) => void
}

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

  return (
    <div className="space-y-4">
      <div>
        <p className="text-sm text-muted-foreground mb-4">
          Select a pre-configured template to get started quickly.
        </p>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {visibleTemplates.map((template) => (
          <Card key={template.id} className="hover:border-blue-500 dark:hover:border-blue-400 transition-colors cursor-pointer">
            <div className="p-4" onClick={() => handleSelectTemplate(template)}>
              <div className="flex items-start gap-3">
                <div className="text-3xl flex-shrink-0">{template.icon}</div>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center justify-between mb-1">
                    <h4 className="font-semibold text-gray-900 dark:text-gray-100">
                      {template.name}
                    </h4>
                    <div className="flex gap-2">
                      {template.devOnly && (
                        <span className="px-2 py-0.5 text-xs rounded-full bg-yellow-200 dark:bg-yellow-900 text-yellow-700 dark:text-yellow-300">
                          Dev
                        </span>
                      )}
                      <span className="px-2 py-0.5 text-xs rounded-full bg-gray-200 dark:bg-gray-700 text-gray-700 dark:text-gray-300">
                        {template.transport}
                      </span>
                      {template.authMethod === 'oauth_browser' && (
                        <span className="px-2 py-0.5 text-xs rounded-full bg-blue-200 dark:bg-blue-900 text-blue-700 dark:text-blue-300">
                          OAuth
                        </span>
                      )}
                    </div>
                  </div>
                  <p className="text-sm text-gray-600 dark:text-gray-400 mb-2">
                    {template.description}
                  </p>
                  {template.docsUrl && (
                    <a
                      href={template.docsUrl}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-xs text-blue-500 dark:text-blue-400 hover:underline"
                      onClick={(e) => e.stopPropagation()}
                    >
                      View Documentation →
                    </a>
                  )}
                </div>
              </div>
              <Button
                onClick={(e) => {
                  e.stopPropagation()
                  handleSelectTemplate(template)
                }}
                variant="secondary"
                className="w-full mt-3"
              >
                Use This Template
              </Button>
            </div>
          </Card>
        ))}
      </div>

    </div>
  )
}
