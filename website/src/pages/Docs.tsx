import { useState, useEffect, useRef, useCallback } from 'react'
import { useParams, useNavigate, Link } from 'react-router-dom'
import Markdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import docsContent from './docs-content'
import { DocEmbed } from '../components/docs/DocEmbeds'
import {
  BookOpen,
  Rocket,
  Users,
  Server,
  Route,
  Gauge,
  Wrench,
  Sparkles,
  ShieldCheck,
  Shield,
  Store,
  Activity,
  FileText,
  Lock,
  Code,
  Network,
  Hash,
  Menu,
  X,
  ChevronLeft,
  ChevronRight,
  Terminal,
  Database,
  Minimize2,
  Zap,
  ScanSearch,
  Brain,
  MessagesSquare,
} from 'lucide-react'

// --- Section data ---

interface DocSubSection {
  id: string
  title: string
  children?: { id: string; title: string }[]
}

interface DocSection {
  id: string
  title: string
  icon: React.ReactNode
  subsections: DocSubSection[]
}

const sections: DocSection[] = [
  {
    id: 'introduction',
    title: 'Introduction',
    icon: <BookOpen className="h-4 w-4" />,
    subsections: [
      { id: 'what-is-localrouter', title: 'What is LocalRouter' },
      { id: 'key-concepts', title: 'Key Concepts' },
      { id: 'architecture-overview', title: 'Architecture Overview' },
    ],
  },
  {
    id: 'getting-started',
    title: 'Getting Started',
    icon: <Rocket className="h-4 w-4" />,
    subsections: [
      { id: 'installation', title: 'Installation', children: [
        { id: 'install-macos', title: 'macOS' },
        { id: 'install-windows', title: 'Windows' },
        { id: 'install-linux', title: 'Linux' },
      ]},
      { id: 'first-run', title: 'First Run' },
      { id: 'configuring-first-provider', title: 'Configuring Your First Provider' },
      { id: 'pointing-apps', title: 'Pointing Apps to localhost:3625' },
    ],
  },
  {
    id: 'clients',
    title: 'Clients',
    icon: <Users className="h-4 w-4" />,
    subsections: [
      { id: 'clients-overview', title: 'Overview' },
      { id: 'creating-client-keys', title: 'Creating Client Keys' },
      { id: 'authentication-methods', title: 'Authentication Methods', children: [
        { id: 'auth-api-key', title: 'API Key' },
        { id: 'auth-oauth', title: 'OAuth Browser Flow' },
        { id: 'auth-stdio', title: 'STDIO' },
      ]},
      { id: 'scoped-permissions', title: 'Scoped Permissions', children: [
        { id: 'model-restrictions', title: 'Model Restrictions' },
        { id: 'provider-restrictions', title: 'Provider Restrictions' },
        { id: 'mcp-server-restrictions', title: 'MCP Server Restrictions' },
      ]},
    ],
  },
  {
    id: 'providers',
    title: 'Providers',
    icon: <Server className="h-4 w-4" />,
    subsections: [
      { id: 'supported-providers', title: 'Supported Providers' },
      { id: 'adding-provider-keys', title: 'Adding Provider API Keys' },
      { id: 'provider-health-checks', title: 'Provider Health Checks', children: [
        { id: 'circuit-breaker', title: 'Circuit Breaker' },
        { id: 'latency-tracking', title: 'Latency Tracking' },
      ]},
      { id: 'feature-adapters', title: 'Feature Adapters', children: [
        { id: 'prompt-caching', title: 'Prompt Caching' },
        { id: 'json-mode', title: 'JSON Mode' },
        { id: 'structured-outputs', title: 'Structured Outputs' },
        { id: 'logprobs', title: 'Logprobs' },
      ]},
    ],
  },
  {
    id: 'model-selection-routing',
    title: 'Model Selection & Routing',
    icon: <Route className="h-4 w-4" />,
    subsections: [
      { id: 'auto-routing', title: 'Auto Routing (model: "auto")' },
      { id: 'routellm-classifier', title: 'RouteLLM Classifier', children: [
        { id: 'strong-weak-classification', title: 'Strong / Weak Classification' },
      ]},
      { id: 'fallback-chains', title: 'Fallback Chains', children: [
        { id: 'provider-failover', title: 'Provider Failover' },
        { id: 'offline-fallback', title: 'Offline Fallback (Ollama / LMStudio)' },
      ]},
      { id: 'routing-strategies', title: 'Routing Strategies', children: [
        { id: 'strategy-lowest-cost', title: 'Lowest Cost' },
        { id: 'strategy-highest-performance', title: 'Highest Performance' },
        { id: 'strategy-local-first', title: 'Local First' },
        { id: 'strategy-remote-first', title: 'Remote First' },
      ]},
      { id: 'free-tier-mode', title: 'Free-Tier Mode', children: [
        { id: 'free-tier-types', title: 'Free-Tier Types' },
        { id: 'free-tier-override', title: 'Provider Override' },
        { id: 'free-tier-set-usage', title: 'Set Usage' },
        { id: 'free-tier-tracking', title: 'Usage Tracking' },
        { id: 'free-tier-backoff', title: 'Backoff & Exhaustion' },
      ]},
      { id: 'error-classification', title: 'Error Classification', children: [
        { id: 'error-rate-limited', title: 'Rate Limited' },
        { id: 'error-policy-violation', title: 'Policy Violation' },
        { id: 'error-context-length', title: 'Context Length Exceeded' },
      ]},
    ],
  },
  {
    id: 'rate-limiting',
    title: 'Rate Limiting',
    icon: <Gauge className="h-4 w-4" />,
    subsections: [
      { id: 'request-rate-limits', title: 'Request Rate Limits' },
      { id: 'token-limits', title: 'Token Limits (Input / Output)' },
      { id: 'cost-limits', title: 'Cost Limits (USD / Month)' },
      { id: 'per-key-vs-per-router', title: 'Per-Key vs Per-Router Limits' },
    ],
  },
  {
    id: 'unified-mcp-gateway',
    title: 'Unified MCP Gateway',
    icon: <Wrench className="h-4 w-4" />,
    subsections: [
      { id: 'mcp-overview', title: 'Overview & Architecture' },
      { id: 'tool-namespacing', title: 'Tool Namespacing (server__tool)' },
      { id: 'transport-types', title: 'Transport Types', children: [
        { id: 'transport-stdio', title: 'STDIO' },
        { id: 'transport-sse', title: 'SSE' },
        { id: 'transport-streamable-http', title: 'Streamable HTTP' },
      ]},
      { id: 'deferred-tool-loading', title: 'Deferred Tool Loading' },
      { id: 'virtual-search-tool', title: 'Virtual Search Tool' },
      { id: 'session-management', title: 'Session Management' },
      { id: 'response-caching', title: 'Response Caching' },
      { id: 'partial-failure-handling', title: 'Partial Failure Handling' },
      { id: 'mcp-oauth', title: 'MCP OAuth Browser Authentication', children: [
        { id: 'oauth-pkce-flow', title: 'OAuth 2.0 + PKCE Flow' },
        { id: 'oauth-auto-discovery', title: 'Auto-Discovery' },
        { id: 'oauth-token-refresh', title: 'Token Refresh' },
      ]},
    ],
  },
  {
    id: 'skills',
    title: 'Skills',
    icon: <Sparkles className="h-4 w-4" />,
    subsections: [
      { id: 'what-are-skills', title: 'What Are Skills' },
      { id: 'skills-as-mcp-tools', title: 'Skills as MCP Tools' },
      { id: 'multi-step-workflows', title: 'Multi-Step Workflows' },
      { id: 'skill-whitelisting', title: 'Per-Client Skill Whitelisting' },
    ],
  },
  {
    id: 'coding-agents',
    title: 'AI Coding Agents',
    icon: <Terminal className="h-4 w-4" />,
    subsections: [
      { id: 'coding-agents-overview', title: 'Overview' },
      { id: 'coding-agents-supported', title: 'Supported Agents' },
      { id: 'coding-agents-mcp-tools', title: 'MCP Tool Interface', children: [
        { id: 'coding-agents-tool-start', title: 'Starting Sessions' },
        { id: 'coding-agents-tool-say', title: 'Sending Messages' },
        { id: 'coding-agents-tool-status', title: 'Checking Status' },
        { id: 'coding-agents-tool-respond', title: 'Responding to Questions' },
        { id: 'coding-agents-tool-interrupt', title: 'Interrupting Sessions' },
        { id: 'coding-agents-tool-list', title: 'Listing Sessions' },
      ]},
      { id: 'coding-agents-approvals', title: 'Approval & Elicitation Flow', children: [
        { id: 'coding-agents-elicitation', title: 'Elicitation (Direct)' },
        { id: 'coding-agents-polling', title: 'Polling (Fallback)' },
      ]},
      { id: 'coding-agents-session-lifecycle', title: 'Session Lifecycle' },
      { id: 'coding-agents-permissions', title: 'Per-Client Permissions' },
    ],
  },
  {
    id: 'mcp-via-llm',
    title: 'MCP via LLM',
    icon: <Zap className="h-4 w-4" />,
    subsections: [
      { id: 'mcp-via-llm-overview', title: 'Overview' },
      { id: 'mcp-via-llm-how-it-works', title: 'How It Works', children: [
        { id: 'mcp-via-llm-injection', title: 'Tool Injection' },
        { id: 'mcp-via-llm-agentic-loop', title: 'Agentic Loop' },
        { id: 'mcp-via-llm-mixed-tools', title: 'Mixed Tool Execution' },
      ]},
      { id: 'mcp-via-llm-sessions', title: 'Session Management' },
      { id: 'mcp-via-llm-config', title: 'Configuration', children: [
        { id: 'mcp-via-llm-config-options', title: 'Options' },
        { id: 'mcp-via-llm-config-per-client', title: 'Per-Client Override' },
      ]},
      { id: 'mcp-via-llm-client-modes', title: 'Client Modes' },
    ],
  },
  {
    id: 'firewall',
    title: 'Firewall',
    icon: <ShieldCheck className="h-4 w-4" />,
    subsections: [
      { id: 'approval-flow', title: 'Runtime Approval Flow', children: [
        { id: 'allow-once', title: 'Allow Once' },
        { id: 'allow-session', title: 'Allow for Session' },
        { id: 'deny', title: 'Deny' },
      ]},
      { id: 'request-inspection', title: 'Request Inspection & Modification' },
      { id: 'approval-policies', title: 'Granular Approval Policies', children: [
        { id: 'policy-per-client', title: 'Per-Client' },
        { id: 'policy-per-model', title: 'Per-Model' },
        { id: 'policy-per-mcp', title: 'Per-MCP Server' },
        { id: 'policy-per-skill', title: 'Per-Skill' },
      ]},
    ],
  },
  {
    id: 'guardrails',
    title: 'GuardRails',
    icon: <Shield className="h-4 w-4" />,
    subsections: [
      { id: 'content-safety-scanning', title: 'Content Safety Scanning' },
      { id: 'detection-types', title: 'Detection Types', children: [
        { id: 'detect-prompt-injection', title: 'Prompt Injection' },
        { id: 'detect-jailbreak', title: 'Jailbreak Attempts' },
        { id: 'detect-pii', title: 'PII Leakage' },
        { id: 'detect-code-injection', title: 'Code Injection' },
      ]},
      { id: 'detection-sources', title: 'Detection Sources', children: [
        { id: 'source-builtin', title: 'Built-in Rules' },
        { id: 'source-presidio', title: 'Microsoft Presidio' },
        { id: 'source-llm-guard', title: 'LLM Guard' },
      ]},
      { id: 'parallel-scanning', title: 'Parallel Scanning' },
      { id: 'moderation-endpoint', title: 'Moderation Endpoint' },
    ],
  },
  {
    id: 'secret-scanning',
    title: 'Secret Scanning',
    icon: <ScanSearch className="h-4 w-4" />,
    subsections: [
      { id: 'secret-scanning-overview', title: 'Overview' },
      { id: 'secret-scan-pipeline', title: 'Detection Pipeline' },
      { id: 'secret-scan-categories', title: 'Secret Categories' },
      { id: 'secret-scan-actions', title: 'Actions (Ask / Notify / Off)' },
      { id: 'secret-scan-approval-flow', title: 'Firewall Approval Flow' },
      { id: 'secret-scan-allowlist', title: 'Allowlist & Tuning' },
    ],
  },
  {
    id: 'context-management',
    title: 'Context Management',
    icon: <Database className="h-4 w-4" />,
    subsections: [
      { id: 'context-management-overview', title: 'Overview' },
      { id: 'catalog-compression', title: 'Catalog Compression', children: [
        { id: 'compression-phase-1', title: 'Phase 1: Description Compression' },
        { id: 'compression-phase-2', title: 'Phase 2: Server Deferral' },
        { id: 'compression-phase-3', title: 'Phase 3: List Truncation' },
      ]},
      { id: 'search-based-activation', title: 'Search-Based Activation' },
      { id: 'response-compression', title: 'Response Compression' },
      { id: 'context-management-config', title: 'Configuration', children: [
        { id: 'context-thresholds', title: 'Threshold Settings' },
        { id: 'context-per-client', title: 'Per-Client Override' },
      ]},
    ],
  },
  {
    id: 'prompt-compression',
    title: 'Prompt Compression',
    icon: <Minimize2 className="h-4 w-4" />,
    subsections: [
      { id: 'prompt-compression-overview', title: 'Overview' },
      { id: 'llmlingua-2', title: 'LLMLingua-2 Engine', children: [
        { id: 'extractive-classification', title: 'Extractive Token Classification' },
        { id: 'compression-models', title: 'Model Options' },
      ]},
      { id: 'compression-pipeline', title: 'Compression Pipeline' },
      { id: 'prompt-compression-config', title: 'Configuration', children: [
        { id: 'compression-rate', title: 'Compression Rate' },
        { id: 'compression-message-settings', title: 'Message Settings' },
        { id: 'compression-per-client', title: 'Per-Client Override' },
      ]},
    ],
  },
  {
    id: 'memory',
    title: 'Memory',
    icon: <Brain className="h-4 w-4" />,
    subsections: [
      { id: 'memory-overview', title: 'Overview' },
      { id: 'memory-architecture', title: 'Architecture' },
      { id: 'memory-modes', title: 'Supported Modes' },
      { id: 'memory-recall-tool', title: 'MemoryRecall Tool' },
      { id: 'memory-sessions', title: 'Sessions & Conversations' },
      { id: 'memory-compaction', title: 'Compaction & Summarization' },
      { id: 'memory-privacy', title: 'Privacy' },
      { id: 'memory-config', title: 'Configuration' },
    ],
  },
  {
    id: 'marketplace',
    title: 'Marketplace',
    icon: <Store className="h-4 w-4" />,
    subsections: [
      { id: 'marketplace-overview', title: 'Overview' },
      { id: 'registry-sources', title: 'Registry Sources', children: [
        { id: 'registry-official', title: 'Official Registry' },
        { id: 'registry-community', title: 'Community Registry' },
        { id: 'registry-private', title: 'Private Registries' },
      ]},
      { id: 'mcp-exposed-search', title: 'MCP-Exposed Search' },
      { id: 'gated-installation', title: 'Gated Installation' },
    ],
  },
  {
    id: 'monitoring',
    title: 'Monitoring & Logging',
    icon: <Activity className="h-4 w-4" />,
    subsections: [
      { id: 'access-logs', title: 'Access Log Writer' },
      { id: 'in-memory-metrics', title: 'In-Memory Metrics', children: [
        { id: 'metrics-time-series', title: 'Time-Series Data' },
        { id: 'metrics-dimensions', title: 'Per-Key, Per-Provider, Global' },
        { id: 'metrics-percentiles', title: 'Latency Percentiles (P50, P95, P99)' },
      ]},
      { id: 'historical-log-parser', title: 'Historical Log Parser' },
      { id: 'graph-data', title: 'Graph Data Generation' },
    ],
  },
  {
    id: 'configuration',
    title: 'Configuration',
    icon: <FileText className="h-4 w-4" />,
    subsections: [
      { id: 'yaml-config', title: 'YAML Config Structure' },
      { id: 'config-file-location', title: 'Config File Locations', children: [
        { id: 'config-macos', title: 'macOS' },
        { id: 'config-linux', title: 'Linux' },
        { id: 'config-windows', title: 'Windows' },
      ]},
      { id: 'config-migration', title: 'Config Migration' },
      { id: 'environment-variables', title: 'Environment Variables' },
    ],
  },
  {
    id: 'privacy-security',
    title: 'Privacy & Security',
    icon: <Lock className="h-4 w-4" />,
    subsections: [
      { id: 'local-only-design', title: 'Local-Only by Design' },
      { id: 'zero-telemetry', title: 'Zero Telemetry' },
      { id: 'keychain-storage', title: 'OS Keychain Storage' },
      { id: 'content-security-policy', title: 'Content Security Policy' },
      { id: 'open-source-license', title: 'Open Source (AGPL-3.0)' },
    ],
  },
  {
    id: 'api-openai-gateway',
    title: 'API Reference: OpenAI Gateway',
    icon: <Code className="h-4 w-4" />,
    subsections: [
      { id: 'openai-authentication', title: 'Authentication' },
      { id: 'openai-models', title: 'GET /models' },
      { id: 'openai-chat-completions', title: 'POST /chat/completions' },
      { id: 'openai-completions', title: 'POST /completions' },
      { id: 'openai-embeddings', title: 'POST /embeddings' },
      { id: 'openai-audio-transcriptions', title: 'POST /audio/transcriptions' },
      { id: 'openai-audio-translations', title: 'POST /audio/translations' },
      { id: 'openai-audio-speech', title: 'POST /audio/speech' },
      { id: 'openai-moderations', title: 'POST /moderations' },
      { id: 'openai-image-generations', title: 'POST /images/generations' },
      { id: 'openai-health', title: 'GET /health' },
      { id: 'openai-spec', title: 'GET /openapi.json' },
      { id: 'openai-streaming', title: 'Streaming (SSE)' },
      { id: 'openai-errors', title: 'Error Responses' },
    ],
  },
  {
    id: 'api-mcp-gateway',
    title: 'API Reference: MCP Gateway',
    icon: <Network className="h-4 w-4" />,
    subsections: [
      { id: 'mcp-endpoint', title: 'POST /' },
      { id: 'mcp-tool-namespacing', title: 'Tool Namespacing Convention' },
      { id: 'mcp-session-lifecycle', title: 'Session Lifecycle' },
      { id: 'mcp-authentication', title: 'Authentication' },
      { id: 'mcp-methods', title: 'Supported MCP Methods', children: [
        { id: 'mcp-tools-list', title: 'tools/list' },
        { id: 'mcp-tools-call', title: 'tools/call' },
        { id: 'mcp-resources-list', title: 'resources/list' },
        { id: 'mcp-resources-read', title: 'resources/read' },
        { id: 'mcp-prompts-list', title: 'prompts/list' },
        { id: 'mcp-prompts-get', title: 'prompts/get' },
      ]},
      { id: 'mcp-error-handling', title: 'Error Handling & Partial Failures' },
    ],
  },
]

// --- Sidebar groups ---

interface SidebarGroup {
  label: string
  sectionIds: string[]
}

const sidebarGroups: SidebarGroup[] = [
  { label: 'Getting Started', sectionIds: ['introduction', 'getting-started'] },
  { label: 'Core Features', sectionIds: ['clients', 'providers', 'model-selection-routing', 'rate-limiting'] },
  { label: 'MCP & Extensions', sectionIds: ['unified-mcp-gateway', 'mcp-via-llm', 'skills', 'coding-agents', 'memory', 'marketplace'] },
  { label: 'Optimization', sectionIds: ['context-management', 'prompt-compression'] },
  { label: 'Security', sectionIds: ['firewall', 'guardrails', 'secret-scanning', 'privacy-security'] },
  { label: 'Operations', sectionIds: ['monitoring', 'configuration'] },
  { label: 'API Reference', sectionIds: ['api-openai-gateway', 'api-mcp-gateway'] },
]

const sectionMap = new Map(sections.map((s) => [s.id, s]))

// --- Sidebar ---

function SidebarNav({
  activeSection,
  activeSubsection,
  onItemClick,
}: {
  activeSection: string
  activeSubsection: string
  onItemClick?: () => void
}) {
  return (
    <nav className="p-4 space-y-4">
      {sidebarGroups.map((group) => (
        <div key={group.label}>
          <div className="px-3 mb-1.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground/60">
            {group.label}
          </div>
          <div className="space-y-0.5">
            {group.sectionIds.map((sectionId) => {
              const section = sectionMap.get(sectionId)
              if (!section) return null
              const isActive = activeSection === sectionId
              return (
                <div key={sectionId}>
                  <Link
                    to={`/docs/${sectionId}`}
                    onClick={onItemClick}
                    className={`
                      w-full flex items-center gap-2 px-3 py-2 rounded-md text-sm text-left transition-colors
                      ${isActive
                        ? 'bg-accent text-foreground font-medium'
                        : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground'
                      }
                    `}
                  >
                    {section.icon}
                    {section.title}
                  </Link>
                  {isActive && section.subsections.length > 0 && (
                    <div className="ml-5 mt-0.5 mb-1 space-y-0.5 border-l border-border pl-3">
                      {section.subsections.map((sub) => (
                        <a
                          key={sub.id}
                          href={`#${sub.id}`}
                          onClick={onItemClick}
                          data-subsection-id={sub.id}
                          className={`block py-1 text-xs transition-colors truncate ${
                            activeSubsection === sub.id
                              ? 'text-foreground font-medium'
                              : 'text-muted-foreground hover:text-foreground'
                          }`}
                        >
                          {sub.title}
                        </a>
                      ))}
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        </div>
      ))}
    </nav>
  )
}

function Sidebar({
  activeSection,
  activeSubsection,
  mobileOpen,
  onMobileClose,
  sidebarRef,
}: {
  activeSection: string
  activeSubsection: string
  mobileOpen: boolean
  onMobileClose: () => void
  sidebarRef: React.RefObject<HTMLElement | null>
}) {
  return (
    <>
      {/* Mobile overlay */}
      {mobileOpen && (
        <div
          className="fixed inset-0 z-40 bg-black/50 lg:hidden"
          onClick={onMobileClose}
        />
      )}

      {/* Mobile sidebar — fixed overlay */}
      <aside
        className={`
          fixed top-16 bottom-0 z-50 w-72 border-r bg-background overflow-y-auto
          transition-transform duration-200 lg:hidden
          ${mobileOpen ? 'translate-x-0' : '-translate-x-full'}
        `}
      >
        <div className="flex items-center justify-between p-4 border-b">
          <span className="font-semibold">Documentation</span>
          <button onClick={onMobileClose} className="p-1 rounded hover:bg-accent">
            <X className="h-5 w-5" />
          </button>
        </div>
        <SidebarNav activeSection={activeSection} activeSubsection={activeSubsection} onItemClick={onMobileClose} />
      </aside>

      {/* Desktop sidebar */}
      <aside
        ref={sidebarRef}
        className="hidden lg:block w-72 shrink-0 border-r bg-background overflow-y-auto"
      >
        <SidebarNav activeSection={activeSection} activeSubsection={activeSubsection} />
      </aside>
    </>
  )
}

// --- Section Renderer ---

const mdComponents = {
  p: ({ children, ...props }: React.HTMLAttributes<HTMLParagraphElement>) => (
    <p className="text-sm text-muted-foreground leading-relaxed mb-3" {...props}>{children}</p>
  ),
  ul: ({ children, ...props }: React.HTMLAttributes<HTMLUListElement>) => (
    <ul className="text-sm text-muted-foreground list-disc ml-5 mb-3 space-y-1" {...props}>{children}</ul>
  ),
  ol: ({ children, ...props }: React.HTMLAttributes<HTMLOListElement>) => (
    <ol className="text-sm text-muted-foreground list-decimal ml-5 mb-3 space-y-1" {...props}>{children}</ol>
  ),
  li: ({ children, ...props }: React.HTMLAttributes<HTMLLIElement>) => (
    <li className="leading-relaxed" {...props}>{children}</li>
  ),
  code: ({ children, className, ...props }: React.HTMLAttributes<HTMLElement>) => {
    if (className?.includes('language-')) {
      return <code className="text-xs font-mono whitespace-pre" {...props}>{children}</code>
    }
    return (
      <code className="bg-muted px-1.5 py-0.5 rounded text-xs font-mono" {...props}>{children}</code>
    )
  },
  pre: ({ children, ...props }: React.HTMLAttributes<HTMLPreElement>) => (
    <pre className="mb-3 bg-muted rounded-lg p-4 text-xs font-mono overflow-x-auto whitespace-pre [&>code]:bg-transparent [&>code]:p-0 [&>code]:rounded-none" {...props}>{children}</pre>
  ),
  strong: ({ children, ...props }: React.HTMLAttributes<HTMLElement>) => (
    <strong className="text-foreground font-medium" {...props}>{children}</strong>
  ),
  table: ({ children, ...props }: React.HTMLAttributes<HTMLTableElement>) => (
    <div className="overflow-x-auto mb-3">
      <table className="text-sm w-full border-collapse" {...props}>{children}</table>
    </div>
  ),
  th: ({ children, ...props }: React.HTMLAttributes<HTMLTableCellElement>) => (
    <th className="text-left text-xs font-medium text-muted-foreground border-b px-3 py-2" {...props}>{children}</th>
  ),
  td: ({ children, ...props }: React.HTMLAttributes<HTMLTableCellElement>) => (
    <td className="border-b px-3 py-2 text-muted-foreground" {...props}>{children}</td>
  ),
}

function DocContent({ id }: { id: string }) {
  const content = docsContent[id]
  if (!content) return null
  return (
    <div className="mt-2 mb-4">
      <Markdown remarkPlugins={[remarkGfm]} components={mdComponents}>
        {content}
      </Markdown>
      <DocEmbed id={id} />
    </div>
  )
}

function SectionContent({ section }: { section: DocSection }) {
  const hasAnyContent = section.subsections.some(
    (sub) => docsContent[sub.id] || sub.children?.some((c) => docsContent[c.id])
  )

  return (
    <div>
      <div className="flex items-center gap-3 mb-6">
        <div className="flex items-center justify-center h-10 w-10 rounded-lg bg-primary/10 text-primary">
          {section.icon}
        </div>
        <h2 className="text-2xl font-bold tracking-tight">{section.title}</h2>
      </div>

      <DocContent id={section.id} />

      <div className="space-y-6">
        {section.subsections.map((sub) => (
          <div key={sub.id} id={sub.id} className="scroll-mt-20">
            <h3 className="text-lg font-semibold flex items-center gap-2 mb-2">
              <Hash className="h-4 w-4 text-muted-foreground/50" />
              {sub.title}
            </h3>

            <DocContent id={sub.id} />

            {sub.children && (
              <div className="ml-6 space-y-3">
                {sub.children.map((child) => (
                  <div key={child.id} id={child.id} className="scroll-mt-20">
                    <h4 className="text-sm font-medium text-muted-foreground flex items-center gap-2">
                      <span className="h-1.5 w-1.5 rounded-full bg-muted-foreground/50" />
                      {child.title}
                    </h4>
                    <DocContent id={child.id} />
                  </div>
                ))}
              </div>
            )}
          </div>
        ))}

        {!hasAnyContent && (
          <div className="rounded-lg border border-dashed border-muted-foreground/25 p-6 text-center">
            <p className="text-sm text-muted-foreground">
              Content coming soon.
            </p>
          </div>
        )}
      </div>
    </div>
  )
}

// --- Prev / Next navigation ---

function PrevNextNav({ sectionId }: { sectionId: string }) {
  const currentIndex = sections.findIndex((s) => s.id === sectionId)
  const prev = currentIndex > 0 ? sections[currentIndex - 1] : null
  const next = currentIndex < sections.length - 1 ? sections[currentIndex + 1] : null

  return (
    <div className="mt-12 pt-6 border-t flex items-center justify-between gap-4">
      {prev ? (
        <Link
          to={`/docs/${prev.id}`}
          className="group flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground transition-colors"
        >
          <ChevronLeft className="h-4 w-4 transition-transform group-hover:-translate-x-0.5" />
          <div className="text-right">
            <div className="text-[11px] uppercase tracking-wider text-muted-foreground/60 mb-0.5">Previous</div>
            <div className="font-medium">{prev.title}</div>
          </div>
        </Link>
      ) : <div />}
      {next ? (
        <Link
          to={`/docs/${next.id}`}
          className="group flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground transition-colors text-right"
        >
          <div>
            <div className="text-[11px] uppercase tracking-wider text-muted-foreground/60 mb-0.5">Next</div>
            <div className="font-medium">{next.title}</div>
          </div>
          <ChevronRight className="h-4 w-4 transition-transform group-hover:translate-x-0.5" />
        </Link>
      ) : <div />}
    </div>
  )
}

// --- Main Docs Page ---

export default function Docs() {
  const { sectionId } = useParams<{ sectionId: string }>()
  const navigate = useNavigate()
  const [mobileOpen, setMobileOpen] = useState(false)
  const [activeSubsection, setActiveSubsection] = useState('')
  const sidebarRef = useRef<HTMLElement>(null)

  const currentSectionId = sectionId || 'introduction'
  const currentSection = sectionMap.get(currentSectionId)

  // Redirect to introduction if section not found
  useEffect(() => {
    if (!currentSection) {
      navigate('/docs/introduction', { replace: true })
    }
  }, [currentSection, navigate])

  // Scroll to hash anchor on load or section change
  useEffect(() => {
    const hash = window.location.hash.replace('#', '')
    if (hash) {
      // Wait for content to render
      setTimeout(() => {
        const el = document.getElementById(hash)
        if (el) el.scrollIntoView({ behavior: 'smooth' })
      }, 100)
    } else {
      window.scrollTo(0, 0)
    }
  }, [currentSectionId])

  // Track active subsection on scroll
  const handleSubsectionChange = useCallback((id: string) => {
    setActiveSubsection(id)
    // Auto-scroll sidebar only (not the page) to keep active item visible
    const sidebar = sidebarRef.current
    if (!sidebar) return
    const el = sidebar.querySelector(`[data-subsection-id="${id}"]`) as HTMLElement | null
    if (!el) return
    const sidebarRect = sidebar.getBoundingClientRect()
    const elRect = el.getBoundingClientRect()
    if (elRect.top < sidebarRect.top || elRect.bottom > sidebarRect.bottom) {
      sidebar.scrollTop += elRect.top - sidebarRect.top - sidebarRect.height / 2
    }
  }, [])

  useEffect(() => {
    if (!currentSection) return
    const subIds = currentSection.subsections.map((s) => s.id)
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            handleSubsectionChange(entry.target.id)
          }
        }
      },
      { rootMargin: '-80px 0px -70% 0px' }
    )
    subIds.forEach((id) => {
      const el = document.getElementById(id)
      if (el) observer.observe(el)
    })
    return () => observer.disconnect()
  }, [currentSection, handleSubsectionChange])

  if (!currentSection) return null

  return (
    <div className="flex flex-1 min-h-0">
      <Sidebar
        activeSection={currentSectionId}
        activeSubsection={activeSubsection}
        mobileOpen={mobileOpen}
        onMobileClose={() => setMobileOpen(false)}
        sidebarRef={sidebarRef}
      />

      {/* Mobile menu toggle */}
      <button
        onClick={() => setMobileOpen(true)}
        className="fixed bottom-6 left-6 z-30 lg:hidden flex items-center gap-2 px-4 py-2 rounded-full bg-primary text-primary-foreground shadow-lg"
      >
        <Menu className="h-4 w-4" />
        <span className="text-sm font-medium">Menu</span>
      </button>

      {/* Content */}
      <main className="flex-1 min-w-0 overflow-y-auto px-6 py-10 lg:px-12 lg:py-12 [&>*]:max-w-4xl">
        <SectionContent section={currentSection} />
        <PrevNextNav sectionId={currentSectionId} />
      </main>
    </div>
  )
}
