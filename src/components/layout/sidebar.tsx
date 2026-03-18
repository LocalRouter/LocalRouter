import * as React from "react"
import { invoke } from '@tauri-apps/api/core'
import { listenSafe } from '@/hooks/useTauriListener'
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
  Activity,
} from "lucide-react"
import { FEATURES, INDEXING_CHILDREN, type FeatureKey } from "@/constants/features"
import { ProvidersIcon, McpIcon, SkillsIcon, CodingAgentsIcon, StoreIcon } from "@/components/icons/category-icons"
import ServiceIcon from "@/components/ServiceIcon"
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

export type View = 'dashboard' | 'monitor' | 'clients' | 'resources' | 'mcp-servers' | 'catalog-compression' | 'response-rag' | 'skills'
  | 'coding-agents' | 'marketplace' | 'guardrails' | 'strong-weak' | 'compression' | 'json-repair'
  | 'secret-scanning' | 'memory' | 'indexing' | 'optimize-overview' | 'settings' | 'debug'

interface SidebarProps {
  activeView: View
  activeSubTab: string | null
  onViewChange: (view: View, subTab?: string | null) => void
  dynamicGroups?: {
    clients?: NavDynamicChild[]
    providers?: NavDynamicChild[]
    mcpServers?: NavDynamicChild[]
  }
}

interface NavItem {
  id: View
  icon: React.ElementType
  label: string
  shortcut?: string
  subItems?: NavItem[]
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

interface NavDynamicChild {
  subTab: string
  label: string
  /** Service identifier for ServiceIcon (template_id, provider_type, server name) */
  iconService?: string
}

interface NavStaticChild {
  view: View
  icon: React.ElementType
  label: string
}

type NavEntry = NavItem | NavHeading | NavCollapsible

function isNavHeading(entry: NavEntry): entry is NavHeading {
  return 'heading' in entry
}

function isNavCollapsible(entry: NavEntry): entry is NavCollapsible {
  return 'children' in entry
}

const MAX_SIDEBAR_CHILDREN = 10

const mcpStaticChildren: NavStaticChild[] = [
  { view: 'skills', icon: SkillsIcon, label: 'Skills' },
  { view: 'coding-agents', icon: CodingAgentsIcon, label: 'Coding Agents' },
  { view: 'marketplace', icon: StoreIcon, label: 'Marketplace' },
]

const NON_INDEXING_FEATURES: FeatureKey[] = ['guardrails', 'secretScanning', 'jsonRepair', 'compression', 'routing']

const resourceNavEntries: NavEntry[] = [
  {
    id: 'optimize-overview', icon: Zap, label: 'Optimize', shortcut: '⌘5',
    children: [
      ...NON_INDEXING_FEATURES.map((key) => ({
        id: FEATURES[key].viewId as View,
        icon: FEATURES[key].icon,
        label: FEATURES[key].shortName,
      })),
      {
        id: 'indexing' as View,
        icon: FEATURES.indexing.icon,
        label: FEATURES.indexing.shortName,
        subItems: INDEXING_CHILDREN.map((key) => ({
          id: FEATURES[key].viewId as View,
          icon: FEATURES[key].icon,
          label: FEATURES[key].shortName,
        })),
      },
    ],
  },
]

const bottomNavItems: NavItem[] = [
  ...(import.meta.env.DEV ? [{ id: 'debug' as View, icon: Bug, label: 'Debug' }] : []),
  { id: 'settings', icon: Settings, label: 'Settings', shortcut: '⌘6' },
]

export function Sidebar({ activeView, activeSubTab, onViewChange, dynamicGroups }: SidebarProps) {
  const [healthState, setHealthState] = React.useState<HealthCacheState | null>(null)
  const [isRefreshing, setIsRefreshing] = React.useState(false)
  const [expanded, setExpanded] = React.useState(true) // default expanded, will load from config
  const [collapsibleOpen, setCollapsibleOpen] = React.useState<Record<string, boolean>>({})

  // Auto-expand collapsible when a child view is active
  React.useEffect(() => {
    // Static collapsibles (Optimize)
    for (const entry of resourceNavEntries) {
      if (isNavCollapsible(entry)) {
        const hasActiveChild = entry.children.some(child =>
          child.id === activeView || child.subItems?.some(sub => sub.id === activeView)
        )
        if (hasActiveChild) {
          setCollapsibleOpen(prev => ({ ...prev, [entry.id]: true }))
        }
      }
    }
    // Dynamic collapsibles (Clients, LLMs, MCPs)
    if (activeView === 'clients' && activeSubTab && activeSubTab !== 'settings' && !activeSubTab.startsWith('add/')) {
      setCollapsibleOpen(prev => ({ ...prev, clients: true }))
    }
    if (activeView === 'resources' && activeSubTab?.startsWith('providers/') && activeSubTab.length > 'providers/'.length && !activeSubTab.startsWith('providers/add/')) {
      setCollapsibleOpen(prev => ({ ...prev, resources: true }))
    }
    if (activeView === 'mcp-servers' && activeSubTab && !activeSubTab.startsWith('add/')) {
      setCollapsibleOpen(prev => ({ ...prev, 'mcp-servers': true }))
    }
    // Expand MCPs when a built-in subpage is active
    if (activeView === 'skills' || activeView === 'coding-agents' || activeView === 'marketplace') {
      setCollapsibleOpen(prev => ({ ...prev, 'mcp-servers': true }))
    }
  }, [activeView, activeSubTab])

  // Load sidebar expanded state from config
  React.useEffect(() => {
    invoke<boolean>('get_sidebar_expanded')
      .then(setExpanded)
      .catch((error) => console.error('Failed to load sidebar state:', error))
  }, [])

  // Load health cache state and trigger initial health check
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

    // Trigger initial health check to populate the cache
    // (periodic checks may be disabled, so we need at least one check on startup)
    invoke('refresh_all_health').catch(() => {})

    // Listen for health status changes
    const listeners = [
      listenSafe<HealthCacheState>('health-status-changed', (event) => {
        setHealthState(event.payload)
      }),
      listenSafe<string>('server-status-changed', () => {
        loadHealthState()
      }),
      listenSafe('config-changed', () => {
        loadHealthState()
      }),
      listenSafe('server-restart-completed', () => {
        loadHealthState()
      }),
      listenSafe('server-restart-failed', () => {
        loadHealthState()
      }),
    ]

    return () => {
      listeners.forEach(l => l.cleanup())
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
            onViewChange('optimize-overview')
            break
          case '6':
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
    const hasActiveChild = group.children.some(child =>
      child.id === activeView || child.subItems?.some(sub => sub.id === activeView)
    )
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
          if (isGroupActive) {
            toggleOpen()
          } else {
            onViewChange(group.id)
            if (!isOpen) {
              setCollapsibleOpen(prev => ({ ...prev, [group.id]: true }))
            }
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
            {group.children.map(child => {
              if (child.subItems) {
                return (
                  <div key={child.id}>
                    {renderNavItem(child)}
                    <div className="ml-3 border-l border-border/50 pl-1 mt-0.5 space-y-0.5">
                      {child.subItems.map(renderNavItem)}
                    </div>
                  </div>
                )
              }
              return renderNavItem(child)
            })}
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
          "flex items-center rounded-md transition-colors h-8 shrink-0 w-full gap-2 whitespace-nowrap px-2",
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

  const renderNavDynamicCollapsible = (
    groupId: View,
    GroupIcon: React.ElementType,
    label: string,
    shortcut: string,
    children: NavDynamicChild[],
    staticPrefixItems?: NavStaticChild[],
  ) => {
    const isOpen = collapsibleOpen[groupId] ?? false
    const activeChildSubTab = (() => {
      if (activeView !== groupId || !activeSubTab) return null
      return children.find(child =>
        activeSubTab === child.subTab ||
        activeSubTab.startsWith(child.subTab + '|') ||
        activeSubTab.startsWith(child.subTab + '/')
      )?.subTab ?? null
    })()
    const hasActiveStaticChild = staticPrefixItems?.some(item => activeView === item.view) ?? false
    const hasActiveChild = !!activeChildSubTab || hasActiveStaticChild
    const isGroupActive = activeView === groupId && !hasActiveChild

    const toggleOpen = () => {
      setCollapsibleOpen(prev => ({ ...prev, [groupId]: !prev[groupId] }))
    }

    const displayChildren = children.slice(0, MAX_SIDEBAR_CHILDREN)
    const hasMore = children.length > MAX_SIDEBAR_CHILDREN

    if (!expanded) {
      return (
        <Tooltip key={groupId}>
          <TooltipTrigger asChild>
            <button
              onClick={() => onViewChange(groupId)}
              className={cn(
                "flex items-center rounded-md transition-colors h-8 w-full gap-2 whitespace-nowrap px-2",
                (isGroupActive || hasActiveChild)
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
              )}
            >
              <GroupIcon className="h-4 w-4 shrink-0" />
            </button>
          </TooltipTrigger>
          <TooltipContent side="right" sideOffset={8}>
            <div className="flex items-center gap-2">
              <span>{label}</span>
              <kbd className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                {shortcut}
              </kbd>
            </div>
          </TooltipContent>
        </Tooltip>
      )
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
          if (isGroupActive) {
            toggleOpen()
          } else {
            onViewChange(groupId)
            if (!isOpen) {
              setCollapsibleOpen(prev => ({ ...prev, [groupId]: true }))
            }
          }
        }}
        onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onViewChange(groupId) } }}
      >
        <GroupIcon className="h-4 w-4 shrink-0" />
        <span className="truncate text-left text-sm flex-1">{label}</span>
        {(children.length > 0 || (staticPrefixItems && staticPrefixItems.length > 0)) && (
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
        )}
      </div>
    )

    return (
      <div key={groupId}>
        <Tooltip>
          <TooltipTrigger asChild>
            {parentButton}
          </TooltipTrigger>
          <TooltipContent side="right" sideOffset={8}>
            <kbd className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
              {shortcut}
            </kbd>
          </TooltipContent>
        </Tooltip>
        {isOpen && (children.length > 0 || (staticPrefixItems && staticPrefixItems.length > 0)) && (
          <div className="ml-3 border-l border-border/50 pl-1 mt-0.5 space-y-0.5">
            {staticPrefixItems?.map(item => {
              const StaticIcon = item.icon
              const isActive = activeView === item.view
              return (
                <button
                  key={item.view}
                  onClick={() => onViewChange(item.view)}
                  className={cn(
                    "flex items-center rounded-md transition-colors h-8 w-full whitespace-nowrap px-2 gap-2",
                    isActive
                      ? "bg-accent text-accent-foreground"
                      : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
                  )}
                >
                  <span className="shrink-0 inline-flex items-center justify-center w-[24px] h-[24px]">
                    <StaticIcon className="h-5 w-5" />
                  </span>
                  <span className="truncate text-left text-sm">{item.label}</span>
                </button>
              )
            })}
            {displayChildren.map(child => {
              const isActive = activeChildSubTab === child.subTab
              return (
                <button
                  key={child.subTab}
                  onClick={() => onViewChange(groupId, child.subTab)}
                  className={cn(
                    "flex items-center rounded-md transition-colors h-8 w-full whitespace-nowrap px-2 gap-2",
                    isActive
                      ? "bg-accent text-accent-foreground"
                      : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
                  )}
                >
                  {child.iconService && (
                    <span className="shrink-0 grayscale dark:invert opacity-70 [&_span]:!bg-transparent">
                      <ServiceIcon service={child.iconService} size={16} fallbackToServerIcon={groupId === 'mcp-servers'} />
                    </span>
                  )}
                  <span className="truncate text-left text-sm">{child.label}</span>
                </button>
              )
            })}
            {hasMore && (
              <button
                onClick={() => onViewChange(groupId)}
                className="flex items-center rounded-md transition-colors h-8 w-full whitespace-nowrap px-2 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
              >
                <span className="text-sm italic">Show all ({children.length})</span>
              </button>
            )}
          </div>
        )}
      </div>
    )
  }

  return (
    <TooltipProvider delayDuration={0}>
      <aside
        className={cn(
          "group/sidebar relative flex h-full flex-col border-r bg-background transition-[width] duration-200 ease-in-out",
          expanded ? "w-48" : "w-12"
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
          {/* Monitor */}
          {renderNavItem({ id: 'monitor' as View, icon: Activity, label: 'Monitor' })}

          {/* Dynamic collapsible sections */}
          {renderNavDynamicCollapsible('clients', Users, 'Clients', '⌘2', dynamicGroups?.clients ?? [])}
          {renderNavDynamicCollapsible('resources', ProvidersIcon, 'LLMs', '⌘3', dynamicGroups?.providers ?? [])}
          {renderNavDynamicCollapsible('mcp-servers', McpIcon, 'MCPs', '⌘4', dynamicGroups?.mcpServers ?? [], mcpStaticChildren)}

          {/* Static resource items */}
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
                disabled={isRefreshing}
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
