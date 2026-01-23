import * as React from "react"
import {
  LayoutDashboard,
  Users,
  Database,
  Settings,
  Server,
  Key,
  Cpu,
  Route,
  RefreshCw,
} from "lucide-react"
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  CommandShortcut,
} from "@/components/ui/command"
import type { View } from "./sidebar"

interface CommandPaletteProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onViewChange: (view: View, subTab?: string) => void
  // Data for search
  clients?: Array<{ id: string; name: string; client_id: string }>
  providers?: Array<{ instance_name: string; provider_type: string }>
  models?: Array<{ id: string; provider: string }>
  mcpServers?: Array<{ id: string; name: string }>
  strategies?: Array<{ id: string; name: string; parent: string | null }>
}

export function CommandPalette({
  open,
  onOpenChange,
  onViewChange,
  clients = [],
  providers = [],
  models = [],
  mcpServers = [],
  strategies = [],
}: CommandPaletteProps) {
  const runCommand = React.useCallback(
    (command: () => void) => {
      onOpenChange(false)
      command()
    },
    [onOpenChange]
  )

  // Set up keyboard shortcut
  React.useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.key === "k" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault()
        onOpenChange(!open)
      }
    }

    document.addEventListener("keydown", down)
    return () => document.removeEventListener("keydown", down)
  }, [open, onOpenChange])

  return (
    <CommandDialog open={open} onOpenChange={onOpenChange}>
      <CommandInput placeholder="Type a command or search..." />
      <CommandList>
        <CommandEmpty>No results found.</CommandEmpty>

        {/* Navigation */}
        <CommandGroup heading="Navigation">
          <CommandItem
            onSelect={() => runCommand(() => onViewChange('dashboard'))}
          >
            <LayoutDashboard className="mr-2 h-4 w-4" />
            <span>Dashboard</span>
            <CommandShortcut>⌘1</CommandShortcut>
          </CommandItem>
          <CommandItem
            onSelect={() => runCommand(() => onViewChange('clients'))}
          >
            <Users className="mr-2 h-4 w-4" />
            <span>Clients</span>
            <CommandShortcut>⌘2</CommandShortcut>
          </CommandItem>
          <CommandItem
            onSelect={() => runCommand(() => onViewChange('resources'))}
          >
            <Database className="mr-2 h-4 w-4" />
            <span>Resources</span>
            <CommandShortcut>⌘3</CommandShortcut>
          </CommandItem>
          <CommandItem
            onSelect={() => runCommand(() => onViewChange('settings'))}
          >
            <Settings className="mr-2 h-4 w-4" />
            <span>Settings</span>
            <CommandShortcut>⌘4</CommandShortcut>
          </CommandItem>
        </CommandGroup>

        <CommandSeparator />

        {/* Quick access to resource subtabs */}
        <CommandGroup heading="Resources">
          <CommandItem
            onSelect={() => runCommand(() => onViewChange('resources', 'providers'))}
          >
            <Server className="mr-2 h-4 w-4" />
            <span>Providers</span>
          </CommandItem>
          <CommandItem
            onSelect={() => runCommand(() => onViewChange('resources', 'models'))}
          >
            <Cpu className="mr-2 h-4 w-4" />
            <span>Models</span>
          </CommandItem>
          <CommandItem
            onSelect={() => runCommand(() => onViewChange('resources', 'mcp-servers'))}
          >
            <Database className="mr-2 h-4 w-4" />
            <span>MCP Servers</span>
          </CommandItem>
          <CommandItem
            onSelect={() => runCommand(() => onViewChange('resources', 'oauth-clients'))}
          >
            <Key className="mr-2 h-4 w-4" />
            <span>OAuth Clients</span>
          </CommandItem>
        </CommandGroup>

        <CommandSeparator />

        {/* Settings shortcuts */}
        <CommandGroup heading="Settings">
          <CommandItem
            onSelect={() => runCommand(() => onViewChange('settings', 'server'))}
          >
            <Server className="mr-2 h-4 w-4" />
            <span>Server Configuration</span>
          </CommandItem>
          <CommandItem
            onSelect={() => runCommand(() => onViewChange('settings', 'routing'))}
          >
            <Route className="mr-2 h-4 w-4" />
            <span>Routing & Strategies</span>
          </CommandItem>
          <CommandItem
            onSelect={() => runCommand(() => onViewChange('settings', 'routellm'))}
          >
            <Cpu className="mr-2 h-4 w-4" />
            <span>RouteLLM</span>
          </CommandItem>
          <CommandItem
            onSelect={() => runCommand(() => onViewChange('settings', 'updates'))}
          >
            <RefreshCw className="mr-2 h-4 w-4" />
            <span>Updates</span>
          </CommandItem>
        </CommandGroup>

        {/* Clients search */}
        {clients.length > 0 && (
          <>
            <CommandSeparator />
            <CommandGroup heading="Clients">
              {clients.slice(0, 5).map((client) => (
                <CommandItem
                  key={client.id}
                  onSelect={() =>
                    runCommand(() => onViewChange('clients', client.client_id))
                  }
                >
                  <Users className="mr-2 h-4 w-4" />
                  <span>{client.name}</span>
                  <span className="ml-2 text-xs text-muted-foreground">
                    {client.client_id.slice(0, 8)}...
                  </span>
                </CommandItem>
              ))}
            </CommandGroup>
          </>
        )}

        {/* Providers search */}
        {providers.length > 0 && (
          <>
            <CommandSeparator />
            <CommandGroup heading="Providers">
              {providers.slice(0, 5).map((provider) => (
                <CommandItem
                  key={provider.instance_name}
                  onSelect={() =>
                    runCommand(() =>
                      onViewChange('resources', `providers/${provider.instance_name}`)
                    )
                  }
                >
                  <Server className="mr-2 h-4 w-4" />
                  <span>{provider.instance_name}</span>
                  <span className="ml-2 text-xs text-muted-foreground">
                    {provider.provider_type}
                  </span>
                </CommandItem>
              ))}
            </CommandGroup>
          </>
        )}

        {/* Models search */}
        {models.length > 0 && (
          <>
            <CommandSeparator />
            <CommandGroup heading="Models">
              {models.slice(0, 8).map((model) => (
                <CommandItem
                  key={`${model.provider}/${model.id}`}
                  onSelect={() =>
                    runCommand(() =>
                      onViewChange('resources', `models/${model.provider}/${model.id}`)
                    )
                  }
                >
                  <Cpu className="mr-2 h-4 w-4" />
                  <span>{model.id}</span>
                  <span className="ml-2 text-xs text-muted-foreground">
                    {model.provider}
                  </span>
                </CommandItem>
              ))}
            </CommandGroup>
          </>
        )}

        {/* MCP Servers search */}
        {mcpServers.length > 0 && (
          <>
            <CommandSeparator />
            <CommandGroup heading="MCP Servers">
              {mcpServers.slice(0, 5).map((server) => (
                <CommandItem
                  key={server.id}
                  onSelect={() =>
                    runCommand(() =>
                      onViewChange('resources', `mcp-servers/${server.id}`)
                    )
                  }
                >
                  <Database className="mr-2 h-4 w-4" />
                  <span>{server.name}</span>
                </CommandItem>
              ))}
            </CommandGroup>
          </>
        )}

        {/* Strategies search */}
        {strategies.length > 0 && (
          <>
            <CommandSeparator />
            <CommandGroup heading="Strategies">
              {strategies.slice(0, 5).map((strategy) => (
                <CommandItem
                  key={strategy.id}
                  onSelect={() =>
                    runCommand(() =>
                      onViewChange('settings', `routing/${strategy.id}`)
                    )
                  }
                >
                  <Route className="mr-2 h-4 w-4" />
                  <span>{strategy.name}</span>
                  <span className="ml-2 text-xs text-muted-foreground">
                    {strategy.parent ? 'Owned' : 'Shared'}
                  </span>
                </CommandItem>
              ))}
            </CommandGroup>
          </>
        )}
      </CommandList>
    </CommandDialog>
  )
}
