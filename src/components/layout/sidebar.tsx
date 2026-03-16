import * as React from "react"
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import {
  Users,
  Settings,
  RefreshCw,
  Bug,
  ChevronsLeft,
  ChevronsRight,
  ChevronDown,
  ChevronRight,
  Zap,
} from "lucide-react"
import { FEATURES } from "@/constants/features"
import { ProvidersIcon, McpIcon, SkillsIcon, CodingAgentsIcon, StoreIcon } from "@/components/icons/category-icons"
import { Logo } from "@/components/Logo"
import { cn } from "@/lib/utils"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  TooltipProvider,
} from "@/components/ui/tooltip"
import type { SetSidebarExpandedParams } from "@/types/tauri-commands"

type AggregateHealthStatus = 'red' | 'yellow' | 'green'
type ItemHealthStatus = 'healthy' | 'degraded' | 'unhealthy' | 'ready' | 'pending' | 'disabled'

interface ItemHealth {
  name: string
  status: ItemHealthStatus
  latency_ms?: number
  error?: string
  last_checked: string
}

interface HealthCacheState {
  server_running: boolean
  server_host?: string
  server_port?: number
  providers: Record<string, ItemHealth>
  mcp_servers: Record<string, ItemHealth>
  last_refresh?: string
  aggregate_status: AggregateHealthStatus
}

export type View = 'dashboard' | 'clients' | 'resources' | 'mcp-servers' | 'catalog-compression' | 'response-rag' | 'skills'
  | 'coding-agents' | 'marketplace' | 'guardrails' | 'strong-weak' | 'compression' | 'json-repair'
  | 'secret-scanning' | 'memory' | 'optimize-overview' | 'settings' | 'debug'

interface SidebarProps {
  activeView: View
  onViewChange: (view: View) => void
}

interface NavItem {
  id: View
  icon: React.ElementType
  label: string
  shortcut?: string
}

interface NavHeading {
  heading: string
}

interface NavCollapsible {
  id: View
  icon: React.ElementType
  label: string
  shortcut?: string
  children: NavItem[]
}

type NavEntry = NavItem | NavHeading | NavCollapsible

function isNavHeading(entry: NavEntry): entry is NavHeading {
  return 'heading' in entry
}

function isNavCollapsible(entry: NavEntry): entry is NavCollapsible {
  return 'children' in entry
}

const clientNavItems: NavItem[] = [
  { id: 'clients', icon: Users, label: 'Clients', shortcut: '⌘2' },
]

const resourceNavEntries: NavEntry[] = [
  { id: 'resources', icon: ProvidersIcon, label: 'LLMs', shortcut: '⌘3' },
  { id: 'mcp-servers', icon: McpIcon, label: 'MCPs', shortcut: '⌘4' },
  { id: 'skills', icon: SkillsIcon, label: 'Skills', shortcut: '⌘5' },
  { id: 'coding-agents', icon: CodingAgentsIcon, label: 'Coding Agents', shortcut: '⌘6' },
  { id: 'marketplace', icon: StoreIcon, label: 'Marketplace', shortcut: '⌘7' },
  {
    id: 'optimize-overview', icon: Zap, label: 'Optimize', shortcut: '⌘8',
    children: [
      { id: FEATURES.guardrails.viewId as View, icon: FEATURES.guardrails.icon, label: FEATURES.guardrails.shortName },
      { id: FEATURES.secretScanning.viewId as View, icon: FEATURES.secretScanning.icon, label: FEATURES.secretScanning.shortName },
      { id: FEATURES.jsonRepair.viewId as View, icon: FEATURES.jsonRepair.icon, label: FEATURES.jsonRepair.shortName },
      { id: FEATURES.compression.viewId as View, icon: FEATURES.compression.icon, label: FEATURES.compression.shortName },
      { id: FEATURES.routing.viewId as View, icon: FEATURES.routing.icon, label: FEATURES.routing.shortName },
      { id: FEATURES.catalogCompression.viewId as View, icon: FEATURES.catalogCompression.icon, label: FEATURES.catalogCompression.shortName },
      { id: FEATURES.responseRag.viewId as View, icon: FEATURES.responseRag.icon, label: FEATURES.responseRag.shortName },
      { id: FEATURES.memory.viewId as View, icon: FEATURES.memory.icon, label: FEATURES.memory.shortName },
    ],
  },
]

const bottomNavItems: NavItem[] = [
  ...(import.meta.env.DEV ? [{ id: 'debug' as View, icon: Bug, label: 'Debug' }] : []),
  { id: 'settings', icon: Settings, label: 'Settings', shortcut: '⌘9' },
]

export function Sidebar({ activeView, onViewChange }: SidebarProps) {
  const [healthState, setHealthState] = React.useState<HealthCacheState | null>(null)
  const [isRefreshing, setIsRefreshing] = React.useState(false)
  const [expanded, setExpanded] = React.useState(true) // default expanded, will load from config
  const [collapsibleOpen, setCollapsibleOpen] = React.useState<Record<string, boolean>>({})

  // Auto-expand collapsible when a child view is active
  React.useEffect(() => {
    for (const entry of resourceNavEntries) {
      if (isNavCollapsible(entry)) {
        const hasActiveChild = entry.children.some(child => child.id === activeView)
        if (hasActiveChild) {
          setCollapsibleOpen(prev => ({ ...prev, [entry.id]: true }))
        }
      }
    }
  }, [activeView])

  // Load sidebar expanded state from config
  React.useEffect(() => {
    invoke<boolean>('get_sidebar_expanded')
      .then(setExpanded)
      .catch((error) => console.error('Failed to load sidebar state:', error))
  }, [])

  // Load health cache state
  React.useEffect(() => {
    const loadHealthState = async () => {
      try {
        const state = await invoke<HealthCacheState>('get_health_cache')
        setHealthState(state)
      } catch (error) {
        console.error('Failed to load health state:', error)
      }
    }

    loadHealthState()

    // Listen for health status changes
    const unlistenHealth = listen<HealthCacheState>('health-status-changed', (event) => {
      setHealthState(event.payload)
    })

    // Also listen for server status changes to update immediately
    const unlistenStatus = listen<string>('server-status-changed', () => {
      loadHealthState()
    })

    // Listen for config changes (port might change)
    const unlistenConfig = listen('config-changed', () => {
      loadHealthState()
    })

    // Listen for server restart events
    const unlistenRestartCompleted = listen('server-restart-completed', () => {
      loadHealthState()
    })

    const unlistenRestartFailed = listen('server-restart-failed', () => {
      loadHealthState()
    })

    return () => {
      unlistenHealth.then(fn => fn())
      unlistenStatus.then(fn => fn())
      unlistenConfig.then(fn => fn())
      unlistenRestartCompleted.then(fn => fn())
      unlistenRestartFailed.then(fn => fn())
    }
  }, [])

  // Handle manual refresh
  const handleRefresh = async () => {
    if (isRefreshing) return
    setIsRefreshing(true)
    try {
      await invoke('refresh_all_health')
    } catch (error) {
      console.error('Failed to refresh health:', error)
    } finally {
      setTimeout(() => setIsRefreshing(false), 1000)
    }
  }

  // Toggle sidebar expanded state
  const toggleExpanded = async () => {
    const newExpanded = !expanded
    setExpanded(newExpanded)
    try {
      await invoke('set_sidebar_expanded', { expanded: newExpanded } satisfies SetSidebarExpandedParams)
    } catch (error) {
      console.error('Failed to save sidebar state:', error)
    }
  }

  // Check if any items are in pending/loading state
  const hasAnyPending = (): boolean => {
    if (!healthState) return true
    for (const health of Object.values(healthState.providers)) {
      if (health.status === 'pending') return true
    }
    for (const health of Object.values(healthState.mcp_servers)) {
      if (health.status === 'pending') return true
    }
    return false
  }

  // Get status color
  const getStatusColor = (status: AggregateHealthStatus | undefined): string => {
    switch (status) {
      case 'green': return 'bg-green-500'
      case 'yellow': return 'bg-yellow-500'
      case 'red': return 'bg-red-500'
      default: return 'bg-gray-400'
    }
  }

  // Get item status color
  const getItemStatusColor = (status: ItemHealthStatus): string => {
    switch (status) {
      case 'healthy': return 'bg-green-500'
      case 'ready': return 'bg-green-400'
      case 'degraded': return 'bg-yellow-500'
      case 'unhealthy': return 'bg-red-500'
      case 'pending': return 'bg-gray-400'
      case 'disabled': return 'bg-gray-400'
      default: return 'bg-gray-400'
    }
  }

  // Format status for display
  const formatStatus = (status: ItemHealthStatus): string => {
    return status.charAt(0).toUpperCase() + status.slice(1)
  }

  // Set up keyboard shortcuts
  React.useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey) {
        switch (e.key) {
          case '1':
            e.preventDefault()
            onViewChange('dashboard')
            break
          case '2':
            e.preventDefault()
            onViewChange('clients')
            break
          case '3':
            e.preventDefault()
            onViewChange('resources')
            break
          case '4':
            e.preventDefault()
            onViewChange('mcp-servers')
            break
          case '5':
            e.preventDefault()
            onViewChange('skills')
            break
          case '6':
            e.preventDefault()
            onViewChange('coding-agents')
            break
          case '7':
            e.preventDefault()
            onViewChange('marketplace')
            break
          case '8':
            e.preventDefault()
            onViewChange('optimize-overview')
            break
          case '9':
            e.preventDefault()
            onViewChange('settings')
            break
        }
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [onViewChange])

  const renderNavEntry = (entry: NavEntry, index: number) => {
    if (isNavHeading(entry)) {
      if (!expanded) {
        return <div key={`heading-${index}`} className="my-1 h-px bg-border" />
      }
      return (
        <div key={`heading-${index}`} className="px-2 pt-2 pb-0.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground/70">
          {entry.heading}
        </div>
      )
    }
    if (isNavCollapsible(entry)) {
      return renderNavCollapsible(entry)
    }
    return renderNavItem(entry)
  }

  const renderNavCollapsible = (group: NavCollapsible) => {
    const isOpen = collapsibleOpen[group.id] ?? false
    const isGroupActive = activeView === group.id
    const hasActiveChild = group.children.some(child => child.id === activeView)
    const Icon = group.icon

    const toggleOpen = () => {
      setCollapsibleOpen(prev => ({ ...prev, [group.id]: !prev[group.id] }))
    }

    const parentButton = (
      <div
        role="button"
        tabIndex={0}
        className={cn(
          "flex items-center rounded-md transition-colors h-8 w-full gap-2 whitespace-nowrap px-2 cursor-pointer",
          isGroupActive
            ? "bg-accent text-accent-foreground"
            : hasActiveChild
              ? "text-accent-foreground"
              : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
        )}
        onClick={() => {
          onViewChange(group.id)
          if (!isOpen) {
            setCollapsibleOpen(prev => ({ ...prev, [group.id]: true }))
          }
        }}
        onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onViewChange(group.id) } }}
      >
        <Icon className="h-4 w-4 shrink-0" />
        <span className="truncate text-left text-sm flex-1">{group.label}</span>
        <button
          className="shrink-0 p-0.5 rounded hover:bg-accent-foreground/10"
          onClick={(e) => {
            e.stopPropagation()
            toggleOpen()
          }}
        >
          {isOpen
            ? <ChevronDown className="h-3 w-3" />
            : <ChevronRight className="h-3 w-3" />
          }
        </button>
      </div>
    )

    if (!expanded) {
      // Collapsed sidebar: just show the parent icon
      return (
        <Tooltip key={group.id}>
          <TooltipTrigger asChild>
            <button
              onClick={() => onViewChange(group.id)}
              className={cn(
                "flex items-center rounded-md transition-colors h-8 w-full gap-2 whitespace-nowrap px-2",
                (isGroupActive || hasActiveChild)
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
              )}
            >
              <Icon className="h-4 w-4 shrink-0" />
            </button>
          </TooltipTrigger>
          <TooltipContent side="right" sideOffset={8}>
            <div className="flex items-center gap-2">
              <span>{group.label}</span>
              {group.shortcut && (
                <kbd className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                  {group.shortcut}
                </kbd>
              )}
            </div>
          </TooltipContent>
        </Tooltip>
      )
    }

    const parentContent = group.shortcut ? (
      <Tooltip>
        <TooltipTrigger asChild>
          {parentButton}
        </TooltipTrigger>
        <TooltipContent side="right" sideOffset={8}>
          <kbd className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
            {group.shortcut}
          </kbd>
        </TooltipContent>
      </Tooltip>
    ) : parentButton

    return (
      <div key={group.id}>
        {parentContent}
        {isOpen && (
          <div className="ml-3 border-l border-border/50 pl-1 mt-0.5 space-y-0.5">
            {group.children.map(renderNavItem)}
          </div>
        )}
      </div>
    )
  }

  const renderNavItem = (item: NavItem) => {
    const Icon = item.icon
    const isActive = activeView === item.id
    const showTooltip = !expanded || !!item.shortcut

    const button = (
      <button
        onClick={() => onViewChange(item.id)}
        className={cn(
          "flex items-center rounded-md transition-colors h-8 w-full gap-2 whitespace-nowrap px-2",
          isActive
            ? "bg-accent text-accent-foreground"
            : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
        )}
      >
        <Icon className="h-4 w-4 shrink-0" />
        <span className="truncate text-left text-sm">{item.label}</span>
      </button>
    )

    if (!showTooltip) {
      return <React.Fragment key={item.id}>{button}</React.Fragment>
    }

    return (
      <Tooltip key={item.id}>
        <TooltipTrigger asChild>
          {button}
        </TooltipTrigger>
        <TooltipContent side="right" sideOffset={8}>
          <div className="flex items-center gap-2">
            {!expanded && <span>{item.label}</span>}
            {item.shortcut && (
              <kbd className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                {item.shortcut}
              </kbd>
            )}
          </div>
        </TooltipContent>
      </Tooltip>
    )
  }

  return (
    <TooltipProvider delayDuration={0}>
      <aside
        className={cn(
          "group/sidebar relative flex h-full flex-col border-r bg-background transition-[width] duration-200 ease-in-out",
          expanded ? "w-40" : "w-12"
        )}
      >
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
        {/* Logo - Dashboard */}
        <div className="flex h-12 items-center border-b px-2">
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                onClick={() => onViewChange('dashboard')}
                className={cn(
                  "flex items-center rounded-md transition-colors h-8 w-full gap-2 px-2 whitespace-nowrap",
                  activeView === 'dashboard'
                    ? "bg-accent text-accent-foreground"
                    : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
                )}
              >
                <Logo className="h-4 w-4 shrink-0" />
                <span className="truncate text-sm font-semibold">LocalRouter</span>
              </button>
            </TooltipTrigger>
            <TooltipContent side="right" sideOffset={8}>
              <div className="flex items-center gap-2">
                {!expanded && <span className="font-semibold">Dashboard</span>}
                <kbd className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                  ⌘1
                </kbd>
              </div>
            </TooltipContent>
          </Tooltip>
        </div>

        {/* Main Navigation */}
        <nav className="flex flex-1 flex-col gap-1 overflow-y-auto min-h-0 p-2">
          {/* Client section */}
          {clientNavItems.map(renderNavItem)}

          {/* Resources section */}
          {resourceNavEntries.map(renderNavEntry)}

          {/* Spacer to push bottom items down */}
          <div className="flex-1" />

          {/* Bottom Navigation */}
          {bottomNavItems.map(renderNavItem)}
        </nav>

        {/* Status indicator */}
        <div className="flex items-center border-t p-2">
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                onClick={handleRefresh}
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md hover:bg-accent transition-colors"
                disabled={isRefreshing || hasAnyPending()}
              >
                {isRefreshing || hasAnyPending() ? (
                  <RefreshCw className="h-3 w-3 animate-spin text-muted-foreground" />
                ) : (
                  <div className={cn(
                    "h-2 w-2 rounded-full transition-colors",
                    getStatusColor(healthState?.server_running === false ? 'red' : healthState?.aggregate_status)
                  )} />
                )}
              </button>
            </TooltipTrigger>
            <TooltipContent side="right" sideOffset={8} className="max-w-xs">
              <div className="space-y-2 text-xs">
                {/* Server status */}
                <div>
                  <span className="font-medium text-muted-foreground">LocalRouter LLM & MCP Server</span>
                </div>
                <div className="flex items-center gap-2 pl-2">
                  <div className={cn(
                    "h-1.5 w-1.5 rounded-full",
                    healthState?.server_running ? "bg-green-500" : "bg-red-500"
                  )} />
                  <span className="truncate flex-1">
                    {healthState?.server_host ?? '...'}:{healthState?.server_port ?? '...'}
                  </span>
                  <span className="text-muted-foreground text-[10px]">
                    {healthState?.server_running ? 'Running' : 'Stopped'}
                  </span>
                </div>

                {/* Providers section */}
                {healthState && Object.keys(healthState.providers).length > 0 && (
                  <>
                    <div className="border-t pt-2">
                      <span className="font-medium text-muted-foreground">LLM Providers</span>
                    </div>
                    {Object.entries(healthState.providers).map(([name, health]) => (
                      <div key={name} className="flex items-center gap-2 pl-2">
                        <div className={cn(
                          "h-1.5 w-1.5 rounded-full",
                          getItemStatusColor(health.status)
                        )} />
                        <span className="truncate flex-1">{health.name || name}</span>
                        <span className="text-muted-foreground text-[10px]">
                          {formatStatus(health.status)}
                        </span>
                      </div>
                    ))}
                  </>
                )}

                {/* MCP Servers section */}
                {healthState && Object.keys(healthState.mcp_servers).length > 0 && (
                  <>
                    <div className="border-t pt-2">
                      <span className="font-medium text-muted-foreground">MCP Servers</span>
                    </div>
                    {Object.entries(healthState.mcp_servers).map(([id, health]) => (
                      <div key={id} className="flex items-center gap-2 pl-2">
                        <div className={cn(
                          "h-1.5 w-1.5 rounded-full",
                          getItemStatusColor(health.status)
                        )} />
                        <span className="truncate flex-1">{health.name || id}</span>
                        <span className="text-muted-foreground text-[10px]">
                          {formatStatus(health.status)}
                        </span>
                      </div>
                    ))}
                  </>
                )}

                {/* Refresh hint */}
                <div className="border-t pt-2 text-muted-foreground text-[10px]">
                  Click to refresh
                </div>
              </div>
            </TooltipContent>
          </Tooltip>
        </div>
        </div>

        {/* Expand/collapse toggle - centered, visible on sidebar hover */}
        <button
          onClick={toggleExpanded}
          className="absolute right-0 top-1/2 z-10 -translate-y-1/2 translate-x-1/2 flex h-6 w-6 items-center justify-center rounded-full border bg-background text-muted-foreground shadow-sm opacity-0 group-hover/sidebar:opacity-100 transition-opacity hover:bg-accent hover:text-accent-foreground"
        >
          {expanded ? (
            <ChevronsLeft className="h-3 w-3" />
          ) : (
            <ChevronsRight className="h-3 w-3" />
          )}
        </button>
      </aside>
    </TooltipProvider>
  )
}
