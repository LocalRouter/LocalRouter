import * as React from "react"
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import {
  Users,
  Database,
  Settings,
  Router,
  FlaskConical,
  ServerCog,
  RefreshCw,
} from "lucide-react"
import { cn } from "@/lib/utils"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  TooltipProvider,
} from "@/components/ui/tooltip"

type AggregateHealthStatus = 'red' | 'yellow' | 'green'
type ItemHealthStatus = 'healthy' | 'degraded' | 'unhealthy' | 'ready' | 'pending'

interface ItemHealth {
  name: string
  status: ItemHealthStatus
  latency_ms?: number
  error?: string
  last_checked: string
}

interface HealthCacheState {
  server_running: boolean
  server_port?: number
  providers: Record<string, ItemHealth>
  mcp_servers: Record<string, ItemHealth>
  last_refresh?: string
  aggregate_status: AggregateHealthStatus
}

export type View = 'dashboard' | 'clients' | 'resources' | 'mcp-servers' | 'settings' | 'try-it-out'

interface SidebarProps {
  activeView: View
  onViewChange: (view: View) => void
}

interface NavItem {
  id: View
  icon: React.ElementType
  label: string
  shortcut: string
}

const mainNavItems: NavItem[] = [
  { id: 'clients', icon: Users, label: 'Clients', shortcut: '⌘2' },
  { id: 'resources', icon: Database, label: 'LLM Providers', shortcut: '⌘3' },
  { id: 'mcp-servers', icon: ServerCog, label: 'MCP Servers', shortcut: '⌘4' },
]

const bottomNavItems: NavItem[] = [
  { id: 'try-it-out', icon: FlaskConical, label: 'Try It Out', shortcut: '⌘5' },
  { id: 'settings', icon: Settings, label: 'Settings', shortcut: '⌘6' },
]

export function Sidebar({ activeView, onViewChange }: SidebarProps) {
  const [healthState, setHealthState] = React.useState<HealthCacheState | null>(null)
  const [isRefreshing, setIsRefreshing] = React.useState(false)

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
      // State will be updated via the health-status-changed event
    } catch (error) {
      console.error('Failed to refresh health:', error)
    } finally {
      // Reset refreshing state after a delay to show the animation
      setTimeout(() => setIsRefreshing(false), 1000)
    }
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
            onViewChange('try-it-out')
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

  return (
    <TooltipProvider delayDuration={0}>
      <aside className="flex h-full w-12 flex-col border-r bg-background">
        {/* Logo - Dashboard */}
        <div className="flex h-12 items-center justify-center border-b">
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                onClick={() => onViewChange('dashboard')}
                className={cn(
                  "flex h-8 w-8 items-center justify-center rounded-md transition-colors",
                  activeView === 'dashboard'
                    ? "bg-primary text-primary-foreground"
                    : "bg-primary/80 text-primary-foreground hover:bg-primary"
                )}
              >
                <Router className="h-4 w-4" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right" sideOffset={8}>
              <div className="flex items-center gap-2">
                <span className="font-semibold">Dashboard</span>
                <kbd className="ml-auto rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                  ⌘1
                </kbd>
              </div>
            </TooltipContent>
          </Tooltip>
        </div>

        {/* Main Navigation */}
        <nav className="flex flex-1 flex-col gap-1 p-2">
          {mainNavItems.map((item) => {
            const Icon = item.icon
            const isActive = activeView === item.id

            return (
              <Tooltip key={item.id}>
                <TooltipTrigger asChild>
                  <button
                    onClick={() => onViewChange(item.id)}
                    className={cn(
                      "flex h-8 w-8 items-center justify-center rounded-md transition-colors",
                      isActive
                        ? "bg-accent text-accent-foreground"
                        : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
                    )}
                  >
                    <Icon className="h-4 w-4" />
                    <span className="sr-only">{item.label}</span>
                  </button>
                </TooltipTrigger>
                <TooltipContent side="right" sideOffset={8}>
                  <div className="flex items-center gap-2">
                    <span>{item.label}</span>
                    <kbd className="ml-auto rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                      {item.shortcut}
                    </kbd>
                  </div>
                </TooltipContent>
              </Tooltip>
            )
          })}
        </nav>

        {/* Bottom Navigation */}
        <nav className="flex flex-col gap-1 p-2">
          {bottomNavItems.map((item) => {
            const Icon = item.icon
            const isActive = activeView === item.id

            return (
              <Tooltip key={item.id}>
                <TooltipTrigger asChild>
                  <button
                    onClick={() => onViewChange(item.id)}
                    className={cn(
                      "flex h-8 w-8 items-center justify-center rounded-md transition-colors",
                      isActive
                        ? "bg-accent text-accent-foreground"
                        : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
                    )}
                  >
                    <Icon className="h-4 w-4" />
                    <span className="sr-only">{item.label}</span>
                  </button>
                </TooltipTrigger>
                <TooltipContent side="right" sideOffset={8}>
                  <div className="flex items-center gap-2">
                    <span>{item.label}</span>
                    <kbd className="ml-auto rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                      {item.shortcut}
                    </kbd>
                  </div>
                </TooltipContent>
              </Tooltip>
            )
          })}
        </nav>

        {/* Status indicator with enhanced health tooltip */}
        <div className="border-t p-2">
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                onClick={handleRefresh}
                className="flex h-8 w-8 items-center justify-center rounded-md hover:bg-accent transition-colors"
                disabled={isRefreshing}
              >
                {isRefreshing ? (
                  <RefreshCw className="h-3 w-3 animate-spin text-muted-foreground" />
                ) : (
                  <div className={cn(
                    "h-2 w-2 rounded-full transition-colors",
                    getStatusColor(healthState?.aggregate_status)
                  )} />
                )}
              </button>
            </TooltipTrigger>
            <TooltipContent side="right" sideOffset={8} className="max-w-xs">
              <div className="space-y-2 text-xs">
                {/* Server status */}
                <div className="flex items-center gap-2">
                  <div className={cn(
                    "h-2 w-2 rounded-full",
                    healthState?.server_running ? "bg-green-500" : "bg-red-500"
                  )} />
                  <span className="font-medium">Server</span>
                  <span className="text-muted-foreground ml-auto">
                    {healthState?.server_running
                      ? `Port ${healthState.server_port ?? '...'}`
                      : 'Stopped'}
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
                  Click to refresh health status
                </div>
              </div>
            </TooltipContent>
          </Tooltip>
        </div>
      </aside>
    </TooltipProvider>
  )
}
