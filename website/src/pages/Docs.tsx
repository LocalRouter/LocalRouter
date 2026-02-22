import { useState, useEffect } from 'react'
import { useLocation } from 'react-router-dom'
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
  ChevronRight,
  Menu,
  X,
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
      { id: 'custom-regex-rules', title: 'Custom Regex Rules' },
      { id: 'parallel-scanning', title: 'Parallel Scanning' },
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
      { id: 'openai-models', title: 'GET /v1/models' },
      { id: 'openai-chat-completions', title: 'POST /v1/chat/completions' },
      { id: 'openai-completions', title: 'POST /v1/completions' },
      { id: 'openai-embeddings', title: 'POST /v1/embeddings' },
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
      { id: 'mcp-endpoint', title: 'POST /mcp' },
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

// --- Sidebar ---

function Sidebar({
  activeSection,
  onSectionClick,
  mobileOpen,
  onMobileClose,
}: {
  activeSection: string
  onSectionClick: (id: string) => void
  mobileOpen: boolean
  onMobileClose: () => void
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

      <aside
        className={`
          fixed top-16 bottom-0 z-50 w-72 border-r bg-background overflow-y-auto
          transition-transform duration-200
          lg:sticky lg:top-16 lg:z-0 lg:translate-x-0 lg:h-[calc(100vh-4rem)]
          ${mobileOpen ? 'translate-x-0' : '-translate-x-full'}
        `}
      >
        <div className="flex items-center justify-between p-4 border-b lg:hidden">
          <span className="font-semibold">Documentation</span>
          <button onClick={onMobileClose} className="p-1 rounded hover:bg-accent">
            <X className="h-5 w-5" />
          </button>
        </div>
        <nav className="p-4 space-y-1">
          {sections.map((section) => (
            <button
              key={section.id}
              onClick={() => {
                onSectionClick(section.id)
                onMobileClose()
              }}
              className={`
                w-full flex items-center gap-2 px-3 py-2 rounded-md text-sm text-left transition-colors
                ${activeSection === section.id
                  ? 'bg-accent text-foreground font-medium'
                  : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground'
                }
              `}
            >
              {section.icon}
              {section.title}
            </button>
          ))}
        </nav>
      </aside>
    </>
  )
}

// --- Section Renderer ---

function SectionContent({ section }: { section: DocSection }) {
  return (
    <section id={section.id} className="scroll-mt-20 mb-16">
      <div className="flex items-center gap-3 mb-6">
        <div className="flex items-center justify-center h-10 w-10 rounded-lg bg-primary/10 text-primary">
          {section.icon}
        </div>
        <h2 className="text-2xl font-bold tracking-tight">{section.title}</h2>
      </div>

      <div className="space-y-6">
        {section.subsections.map((sub) => (
          <div key={sub.id} id={sub.id} className="scroll-mt-20">
            <h3 className="text-lg font-semibold flex items-center gap-2 mb-2">
              <ChevronRight className="h-4 w-4 text-muted-foreground" />
              {sub.title}
            </h3>

            {sub.children && (
              <div className="ml-6 space-y-3">
                {sub.children.map((child) => (
                  <div key={child.id} id={child.id} className="scroll-mt-20">
                    <h4 className="text-sm font-medium text-muted-foreground flex items-center gap-2">
                      <span className="h-1.5 w-1.5 rounded-full bg-muted-foreground/50" />
                      {child.title}
                    </h4>
                  </div>
                ))}
              </div>
            )}
          </div>
        ))}

        <div className="rounded-lg border border-dashed border-muted-foreground/25 p-6 text-center">
          <p className="text-sm text-muted-foreground">
            Content coming soon.
          </p>
        </div>
      </div>

      <div className="mt-8 border-b" />
    </section>
  )
}

// --- Main Docs Page ---

export default function Docs() {
  const [activeSection, setActiveSection] = useState('introduction')
  const [mobileOpen, setMobileOpen] = useState(false)
  const location = useLocation()

  // Handle hash navigation on load
  useEffect(() => {
    const hash = location.hash.replace('#', '')
    if (hash) {
      setActiveSection(hash)
      const el = document.getElementById(hash)
      if (el) {
        setTimeout(() => el.scrollIntoView({ behavior: 'smooth' }), 100)
      }
    }
  }, [location.hash])

  // Track active section on scroll
  useEffect(() => {
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            const sectionId = entry.target.id
            const matchedSection = sections.find((s) => s.id === sectionId)
            if (matchedSection) {
              setActiveSection(sectionId)
            }
          }
        }
      },
      { rootMargin: '-80px 0px -70% 0px' }
    )

    sections.forEach((section) => {
      const el = document.getElementById(section.id)
      if (el) observer.observe(el)
    })

    return () => observer.disconnect()
  }, [])

  const handleSectionClick = (id: string) => {
    setActiveSection(id)
    const el = document.getElementById(id)
    if (el) {
      el.scrollIntoView({ behavior: 'smooth' })
    }
    window.history.replaceState(null, '', `#${id}`)
  }

  return (
    <div className="flex min-h-[calc(100vh-4rem)]">
      <Sidebar
        activeSection={activeSection}
        onSectionClick={handleSectionClick}
        mobileOpen={mobileOpen}
        onMobileClose={() => setMobileOpen(false)}
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
      <main className="flex-1 min-w-0 px-6 py-10 lg:px-12 lg:py-12 max-w-4xl">
        <div className="mb-12">
          <h1 className="text-4xl font-bold tracking-tight">Documentation</h1>
          <p className="mt-3 text-lg text-muted-foreground">
            Learn how to configure and use LocalRouter — the local API gateway for LLMs, MCPs, and Skills.
          </p>
        </div>

        {sections.map((section) => (
          <SectionContent key={section.id} section={section} />
        ))}
      </main>
    </div>
  )
}
