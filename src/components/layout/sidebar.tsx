import * as React from "react"
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import {
  LayoutDashboard,
  Users,
  Database,
  Settings,
  Router,
  ScrollText,
  FlaskConical,
} from "lucide-react"
import { cn } from "@/lib/utils"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  TooltipProvider,
} from "@/components/ui/tooltip"

interface ServerConfig {
  host: string
  port: number
  actual_port?: number
  enable_cors: boolean
}

export type View = 'dashboard' | 'clients' | 'resources' | 'logs' | 'settings' | 'try-it-out'

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

const navItems: NavItem[] = [
  { id: 'dashboard', icon: LayoutDashboard, label: 'Dashboard', shortcut: '⌘1' },
  { id: 'clients', icon: Users, label: 'Clients', shortcut: '⌘2' },
  { id: 'resources', icon: Database, label: 'Resources', shortcut: '⌘3' },
  { id: 'logs', icon: ScrollText, label: 'Logs', shortcut: '⌘4' },
  { id: 'try-it-out', icon: FlaskConical, label: 'Try It Out', shortcut: '⌘5' },
  { id: 'settings', icon: Settings, label: 'Settings', shortcut: '⌘6' },
]

export function Sidebar({ activeView, onViewChange }: SidebarProps) {
  const [serverStatus, setServerStatus] = React.useState<'running' | 'stopped'>('stopped')
  const [serverPort, setServerPort] = React.useState<number | null>(null)

  // Load server status and config
  React.useEffect(() => {
    const loadServerInfo = async () => {
      try {
        const [status, config] = await Promise.all([
          invoke<string>('get_server_status'),
          invoke<ServerConfig>('get_server_config'),
        ])
        setServerStatus(status as 'running' | 'stopped')
        setServerPort(config.actual_port ?? config.port)
      } catch (error) {
        console.error('Failed to load server info:', error)
      }
    }

    loadServerInfo()

    // Listen for server status changes
    const unlistenStatus = listen<string>('server-status-changed', (event) => {
      setServerStatus(event.payload as 'running' | 'stopped')
      // Reload to get updated port info
      loadServerInfo()
    })

    // Listen for config changes (port might change)
    const unlistenConfig = listen('config-changed', () => {
      loadServerInfo()
    })

    // Listen for server restart events (restart doesn't emit status-changed)
    const unlistenRestartCompleted = listen('server-restart-completed', () => {
      loadServerInfo()
    })

    const unlistenRestartFailed = listen('server-restart-failed', () => {
      loadServerInfo()
    })

    return () => {
      unlistenStatus.then(fn => fn())
      unlistenConfig.then(fn => fn())
      unlistenRestartCompleted.then(fn => fn())
      unlistenRestartFailed.then(fn => fn())
    }
  }, [])

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
            onViewChange('logs')
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
        {/* Logo */}
        <div className="flex h-12 items-center justify-center border-b">
          <Tooltip>
            <TooltipTrigger asChild>
              <div className="flex h-8 w-8 items-center justify-center rounded-md bg-primary text-primary-foreground">
                <Router className="h-4 w-4" />
              </div>
            </TooltipTrigger>
            <TooltipContent side="right" sideOffset={8}>
              <p className="font-semibold">LocalRouter AI</p>
            </TooltipContent>
          </Tooltip>
        </div>

        {/* Navigation */}
        <nav className="flex flex-1 flex-col gap-1 p-2">
          {navItems.map((item) => {
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

        {/* Status indicator */}
        <div className="border-t p-2">
          <Tooltip>
            <TooltipTrigger asChild>
              <div className="flex h-8 w-8 items-center justify-center">
                <div className={cn(
                  "h-2 w-2 rounded-full",
                  serverStatus === 'running' ? "bg-green-500" : "bg-red-500"
                )} />
              </div>
            </TooltipTrigger>
            <TooltipContent side="right" sideOffset={8}>
              <p>
                {serverStatus === 'running'
                  ? `Server running on port ${serverPort ?? '...'}`
                  : 'Server stopped'}
              </p>
            </TooltipContent>
          </Tooltip>
        </div>
      </aside>
    </TooltipProvider>
  )
}
