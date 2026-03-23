import { useState, useEffect, useRef, useCallback } from 'react'
import { useParams, useNavigate, Link } from 'react-router-dom'
import Markdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import researchContent from './research-content'
import {
  Zap,
  Wrench,
  Braces,
  Coins,
  Minimize2,
  Hash,
  Menu,
  X,
  ChevronLeft,
  ChevronRight,
  FlaskConical,

} from 'lucide-react'

// --- Paper data ---

interface PaperSubSection {
  id: string
  title: string
}

interface Paper {
  id: string
  title: string
  icon: React.ReactNode
  tagline: string
  status: 'published'
  subsections: PaperSubSection[]
}

const papers: Paper[] = [
  {
    id: 'unified-mcp-gateway',
    title: 'Unified MCP Gateway',
    icon: <Wrench className="h-4 w-4" />,
    tagline: 'Aggregating multiple MCP servers with progressive catalog compression',
    status: 'published',
    subsections: [
      { id: 'unified-gateway-abstract', title: 'Abstract' },
      { id: 'unified-gateway-problem', title: 'Problem Statement' },
      { id: 'unified-gateway-architecture', title: 'Architecture' },
      { id: 'unified-gateway-compression', title: 'Progressive Catalog Compression' },
      { id: 'unified-gateway-context-mode', title: 'Context Management & FTS5' },
      { id: 'unified-gateway-virtual-servers', title: 'Virtual Servers' },
      { id: 'unified-gateway-caching', title: 'Adaptive Caching' },
      { id: 'unified-gateway-parallel-pipeline', title: 'Parallel Processing Pipeline' },
      { id: 'unified-gateway-results', title: 'Results' },
    ],
  },
  {
    id: 'mcp-via-llm',
    title: 'MCP via LLM',
    icon: <Zap className="h-4 w-4" />,
    tagline: 'Server-side agentic orchestration for transparent MCP tool execution',
    status: 'published',
    subsections: [
      { id: 'mcp-via-llm-abstract', title: 'Abstract' },
      { id: 'mcp-via-llm-problem', title: 'Problem Statement' },
      { id: 'mcp-via-llm-approach', title: 'Approach' },
      { id: 'mcp-via-llm-mixed', title: 'Mixed Tool Execution' },
      { id: 'mcp-via-llm-streaming', title: 'Streaming Support' },
      { id: 'mcp-via-llm-synthesis', title: 'Resource & Prompt Synthesis' },
      { id: 'mcp-via-llm-sessions', title: 'Session Management' },
      { id: 'mcp-via-llm-results', title: 'Results' },
    ],
  },
  {
    id: 'json-healing',
    title: 'Streaming JSON Response Healing',
    icon: <Braces className="h-4 w-4" />,
    tagline: 'Single-pass O(n) streaming repair with minimal buffering',
    status: 'published',
    subsections: [
      { id: 'json-healing-abstract', title: 'Abstract' },
      { id: 'json-healing-problem', title: 'Problem Statement' },
      { id: 'json-healing-algorithm', title: 'Algorithm Design' },
      { id: 'json-healing-buffering', title: 'Minimal Buffering Strategy' },
      { id: 'json-healing-schema', title: 'JSON Schema Coercion' },
      { id: 'json-healing-streaming-example', title: 'Streaming Example' },
      { id: 'json-healing-integration', title: 'Integration' },
      { id: 'json-healing-results', title: 'Results' },
    ],
  },
  {
    id: 'compression-preservation',
    title: 'Quote-Aware Prompt Compression',
    icon: <Minimize2 className="h-4 w-4" />,
    tagline: 'Preserving code and quoted content during LLMLingua-2 compression',
    status: 'published',
    subsections: [
      { id: 'compression-preservation-abstract', title: 'Abstract' },
      { id: 'compression-preservation-problem', title: 'Problem Statement' },
      { id: 'compression-preservation-algorithm', title: 'Algorithm' },
      { id: 'compression-preservation-detection', title: 'Detection State Machine' },
      { id: 'compression-preservation-edge-cases', title: 'Edge Cases' },
      { id: 'compression-preservation-notice', title: 'Compression Notice' },
      { id: 'compression-preservation-visualization', title: 'UI Visualization' },
      { id: 'compression-preservation-performance', title: 'Performance' },
      { id: 'compression-preservation-status', title: 'Status' },
    ],
  },
  {
    id: 'free-tier-fallback',
    title: 'Free-Tier Mode with Paid Fallback',
    icon: <Coins className="h-4 w-4" />,
    tagline: 'Maximizing free API usage with coordinated backoff and consent-aware fallback',
    status: 'published',
    subsections: [
      { id: 'free-tier-abstract', title: 'Abstract' },
      { id: 'free-tier-problem', title: 'Problem Statement' },
      { id: 'free-tier-taxonomy', title: 'Free-Tier Taxonomy' },
      { id: 'free-tier-tracking', title: 'Usage Tracking' },
      { id: 'free-tier-backoff', title: 'Coordinated Backoff' },
      { id: 'free-tier-fallback', title: 'Fallback Modes' },
      { id: 'free-tier-routing', title: 'Router Integration' },
      { id: 'free-tier-results', title: 'Results' },
    ],
  },
]

const paperMap = new Map(papers.map((p) => [p.id, p]))

// --- Markdown components ---

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

// --- Content renderer ---

function ResearchContent({ id }: { id: string }) {
  const content = researchContent[id]
  if (!content) return null
  return (
    <div className="mt-2 mb-4">
      <Markdown remarkPlugins={[remarkGfm]} components={mdComponents}>
        {content}
      </Markdown>
    </div>
  )
}

// --- Sidebar ---

function SidebarNav({
  activePaper,
  activeSubsection,
  onItemClick,
}: {
  activePaper: string
  activeSubsection: string
  onItemClick?: () => void
}) {
  return (
    <nav className="p-4 space-y-4">
      <div>
        <div className="px-3 mb-1.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground/60">
          Papers
        </div>
        <div className="space-y-0.5">
          {papers.map((paper) => {
            const isActive = activePaper === paper.id
            return (
              <div key={paper.id}>
                <Link
                  to={`/research/${paper.id}`}
                  onClick={onItemClick}
                  className={`
                    w-full flex items-center gap-2 px-3 py-2 rounded-md text-sm text-left transition-colors
                    ${isActive
                      ? 'bg-accent text-foreground font-medium'
                      : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground'
                    }
                  `}
                >
                  {paper.icon}
                  <span className="flex-1 truncate">{paper.title}</span>
                </Link>
                {isActive && paper.subsections.length > 0 && (
                  <div className="ml-5 mt-0.5 mb-1 space-y-0.5 border-l border-border pl-3">
                    {paper.subsections.map((sub) => (
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
    </nav>
  )
}

function Sidebar({
  activePaper,
  activeSubsection,
  mobileOpen,
  onMobileClose,
  sidebarRef,
}: {
  activePaper: string
  activeSubsection: string
  mobileOpen: boolean
  onMobileClose: () => void
  sidebarRef: React.Ref<HTMLElement>
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
          <span className="font-semibold">Research</span>
          <button onClick={onMobileClose} className="p-1 rounded hover:bg-accent">
            <X className="h-5 w-5" />
          </button>
        </div>
        <SidebarNav activePaper={activePaper} activeSubsection={activeSubsection} onItemClick={onMobileClose} />
      </aside>

      {/* Desktop sidebar */}
      <aside
        ref={sidebarRef}
        className="hidden lg:block w-72 shrink-0 border-r bg-background overflow-y-auto"
      >
        <SidebarNav activePaper={activePaper} activeSubsection={activeSubsection} />
      </aside>
    </>
  )
}

// --- Paper content ---

function PaperContent({ paper }: { paper: Paper }) {
  const hasAnyContent = paper.subsections.some((sub) => researchContent[sub.id])

  return (
    <div>
      <div className="flex items-center gap-3 mb-2">
        <div className="flex items-center justify-center h-10 w-10 rounded-lg bg-primary/10 text-primary">
          {paper.icon}
        </div>
        <div>
          <h2 className="text-2xl font-bold tracking-tight">{paper.title}</h2>
        </div>
      </div>
      <p className="text-sm text-muted-foreground mb-6 ml-[52px]">{paper.tagline}</p>

      <div className="space-y-6">
        {paper.subsections.map((sub) => (
          <div key={sub.id} id={sub.id} className="scroll-mt-20">
            <h3 className="text-lg font-semibold flex items-center gap-2 mb-2">
              <Hash className="h-4 w-4 text-muted-foreground/50" />
              {sub.title}
            </h3>
            <ResearchContent id={sub.id} />
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

function PrevNextNav({ paperId }: { paperId: string }) {
  const currentIndex = papers.findIndex((p) => p.id === paperId)
  const prev = currentIndex > 0 ? papers[currentIndex - 1] : null
  const next = currentIndex < papers.length - 1 ? papers[currentIndex + 1] : null

  return (
    <div className="mt-12 pt-6 border-t flex items-center justify-between gap-4">
      {prev ? (
        <Link
          to={`/research/${prev.id}`}
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
          to={`/research/${next.id}`}
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

// --- Overview page ---

function ResearchOverview() {
  return (
    <div>
      <div className="flex items-center gap-3 mb-2">
        <div className="flex items-center justify-center h-10 w-10 rounded-lg bg-primary/10 text-primary">
          <FlaskConical className="h-5 w-5" />
        </div>
        <h2 className="text-2xl font-bold tracking-tight">Research</h2>
      </div>
      <p className="text-sm text-muted-foreground mb-8 ml-[52px]">
        Novel approaches and technical deep-dives from LocalRouter's development.
      </p>

      <div className="grid gap-4">
        {papers.map((paper) => (
          <Link
            key={paper.id}
            to={`/research/${paper.id}`}
            className="group block rounded-lg border p-5 transition-colors hover:bg-accent/50 hover:border-accent"
          >
            <div className="flex items-start gap-4">
              <div className="flex items-center justify-center h-10 w-10 rounded-lg bg-primary/10 text-primary shrink-0 mt-0.5">
                {paper.icon}
              </div>
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2 mb-1">
                  <h3 className="font-semibold text-foreground group-hover:text-primary transition-colors">
                    {paper.title}
                  </h3>
                </div>
                <p className="text-sm text-muted-foreground">{paper.tagline}</p>
                <div className="mt-2 text-xs text-muted-foreground/60">
                  {paper.subsections.length} sections
                </div>
              </div>
            </div>
          </Link>
        ))}
      </div>
    </div>
  )
}

// --- Main Research Page ---

export default function Research() {
  const { paperId } = useParams<{ paperId: string }>()
  const navigate = useNavigate()
  const [mobileOpen, setMobileOpen] = useState(false)
  const [activeSubsection, setActiveSubsection] = useState('')
  const sidebarRef = useRef<HTMLElement>(null)

  const currentPaperId = paperId || ''
  const currentPaper = paperMap.get(currentPaperId)

  // Redirect to overview if paper not found (but not if on /research root)
  useEffect(() => {
    if (paperId && !currentPaper) {
      navigate('/research', { replace: true })
    }
  }, [paperId, currentPaper, navigate])

  // Scroll to hash anchor on load or paper change
  useEffect(() => {
    const hash = window.location.hash.replace('#', '')
    if (hash) {
      setTimeout(() => {
        const el = document.getElementById(hash)
        if (el) el.scrollIntoView({ behavior: 'smooth' })
      }, 100)
    } else {
      window.scrollTo(0, 0)
    }
  }, [currentPaperId])

  // Track active subsection on scroll
  const handleSubsectionChange = useCallback((id: string) => {
    setActiveSubsection(id)
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
    if (!currentPaper) return
    const subIds = currentPaper.subsections.map((s) => s.id)
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
  }, [currentPaper, handleSubsectionChange])

  return (
    <div className="flex flex-1 min-h-0">
      <Sidebar
        activePaper={currentPaperId}
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
        {currentPaper ? (
          <>
            <PaperContent paper={currentPaper} />
            <PrevNextNav paperId={currentPaperId} />
          </>
        ) : (
          <ResearchOverview />
        )}
      </main>
    </div>
  )
}
