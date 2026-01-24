/**
 * StrategiesPanel Component
 *
 * Panel for managing routing strategies in LLM Providers -> Model Routing tab.
 * Shows list of strategies with detail view for configuration.
 */

import { useState, useEffect, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Plus, Route, Users } from "lucide-react"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  ResizablePanelGroup,
  ResizablePanel,
  ResizableHandle,
} from "@/components/ui/resizable"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Label } from "@/components/ui/label"
import {
  EntityActions,
  commonActions,
} from "@/components/shared/entity-actions"
import { cn } from "@/lib/utils"
import { StrategyModelConfiguration, StrategyConfig } from "@/components/strategy"

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
}

interface StrategiesPanelProps {
  selectedId: string | null
  onSelect: (id: string | null) => void
  onNavigateToClient?: (clientId: string) => void
}

export function StrategiesPanel({
  selectedId,
  onSelect,
  onNavigateToClient,
}: StrategiesPanelProps) {
  const [strategies, setStrategies] = useState<StrategyConfig[]>([])
  const [clients, setClients] = useState<Client[]>([])
  const [loading, setLoading] = useState(true)
  const [search, setSearch] = useState("")

  // Dialog states
  const [createDialogOpen, setCreateDialogOpen] = useState(false)
  const [editDialogOpen, setEditDialogOpen] = useState(false)
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false)
  const [newStrategyName, setNewStrategyName] = useState("")
  const [strategyToEdit, setStrategyToEdit] = useState<StrategyConfig | null>(null)
  const [strategyToDelete, setStrategyToDelete] = useState<StrategyConfig | null>(null)

  useEffect(() => {
    loadData()

    const unsubscribe = listen("config-changed", () => {
      loadData()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [])

  const loadData = async () => {
    try {
      setLoading(true)
      const [strategiesList, clientsList] = await Promise.all([
        invoke<StrategyConfig[]>("list_strategies"),
        invoke<Client[]>("list_clients"),
      ])
      setStrategies(strategiesList)
      setClients(clientsList)
    } catch (error) {
      console.error("Failed to load data:", error)
    } finally {
      setLoading(false)
    }
  }

  // Get clients using a specific strategy
  const getClientsForStrategy = (strategyId: string): Client[] => {
    return clients.filter((c) => c.strategy_id === strategyId)
  }

  // Filter strategies by search
  const filteredStrategies = useMemo(() => {
    if (!search) return strategies
    return strategies.filter(
      (s) =>
        s.name.toLowerCase().includes(search.toLowerCase()) ||
        s.id.toLowerCase().includes(search.toLowerCase())
    )
  }, [strategies, search])

  const selectedStrategy = strategies.find((s) => s.id === selectedId)
  const strategyClients = selectedId ? getClientsForStrategy(selectedId) : []

  // Create new strategy
  const handleCreateStrategy = async () => {
    if (!newStrategyName.trim()) return
    try {
      await invoke("create_strategy", { name: newStrategyName.trim() })
      toast.success("Strategy created")
      setNewStrategyName("")
      setCreateDialogOpen(false)
      loadData()
    } catch (error) {
      console.error("Failed to create strategy:", error)
      toast.error("Failed to create strategy")
    }
  }

  // Rename strategy (from edit dialog)
  const handleRenameStrategy = async () => {
    if (!strategyToEdit || !newStrategyName.trim()) return
    try {
      await invoke("update_strategy", {
        strategyId: strategyToEdit.id,
        name: newStrategyName.trim(),
        allowedModels: null,
        autoConfig: null,
        rateLimits: null,
      })
      toast.success("Strategy renamed")
      setNewStrategyName("")
      setStrategyToEdit(null)
      setEditDialogOpen(false)
      loadData()
    } catch (error) {
      console.error("Failed to rename strategy:", error)
      toast.error("Failed to rename strategy")
    }
  }

  // Delete strategy
  const handleDeleteStrategy = async () => {
    if (!strategyToDelete) return

    const usingClients = getClientsForStrategy(strategyToDelete.id)
    if (usingClients.length > 0) {
      toast.error(
        `Cannot delete strategy "${strategyToDelete.name}" - it's used by ${usingClients.length} client(s)`
      )
      setStrategyToDelete(null)
      setDeleteDialogOpen(false)
      return
    }

    try {
      await invoke("delete_strategy", { strategyId: strategyToDelete.id })
      toast.success("Strategy deleted")
      if (selectedId === strategyToDelete.id) {
        onSelect(null)
      }
      loadData()
    } catch (error) {
      console.error("Failed to delete strategy:", error)
      toast.error("Failed to delete strategy")
    } finally {
      setStrategyToDelete(null)
      setDeleteDialogOpen(false)
    }
  }

  // Open edit dialog
  const openEditDialog = (strategy: StrategyConfig) => {
    setStrategyToEdit(strategy)
    setNewStrategyName(strategy.name)
    setEditDialogOpen(true)
  }

  // Open delete dialog
  const openDeleteDialog = (strategy: StrategyConfig) => {
    setStrategyToDelete(strategy)
    setDeleteDialogOpen(true)
  }

  // Handle client click - navigate to client detail
  const handleClientClick = (client: Client) => {
    if (onNavigateToClient) {
      onNavigateToClient(client.id)
    }
  }

  return (
    <>
      <ResizablePanelGroup direction="horizontal" className="h-full rounded-lg border">
        {/* List Panel */}
        <ResizablePanel defaultSize={35} minSize={25}>
          <div className="flex flex-col h-full">
            <div className="p-4 border-b space-y-3">
              <div className="flex items-center gap-2">
                <Input
                  placeholder="Search strategies..."
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  className="flex-1"
                />
                <Button size="icon" onClick={() => setCreateDialogOpen(true)}>
                  <Plus className="h-4 w-4" />
                </Button>
              </div>
            </div>
            <ScrollArea className="flex-1">
              <div className="p-2 space-y-1">
                {loading ? (
                  <p className="text-sm text-muted-foreground p-4">Loading...</p>
                ) : filteredStrategies.length === 0 ? (
                  <p className="text-sm text-muted-foreground p-4">
                    No strategies found
                  </p>
                ) : (
                  filteredStrategies.map((strategy) => {
                    const clientCount = getClientsForStrategy(strategy.id).length
                    const isOwned = !!strategy.parent

                    return (
                      <div
                        key={strategy.id}
                        onClick={() => onSelect(strategy.id)}
                        className={cn(
                          "flex items-center gap-3 p-3 rounded-md cursor-pointer",
                          selectedId === strategy.id
                            ? "bg-accent"
                            : "hover:bg-muted"
                        )}
                      >
                        <Route className="h-4 w-4 text-muted-foreground shrink-0" />
                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-2">
                            <p className="font-medium truncate">{strategy.name}</p>
                            {isOwned && (
                              <Badge variant="outline" className="text-xs">
                                Owned
                              </Badge>
                            )}
                          </div>
                          <p className="text-xs text-muted-foreground">
                            {clientCount} client{clientCount !== 1 ? "s" : ""}
                          </p>
                        </div>
                      </div>
                    )
                  })
                )}
              </div>
            </ScrollArea>
          </div>
        </ResizablePanel>

        <ResizableHandle withHandle />

        {/* Detail Panel */}
        <ResizablePanel defaultSize={65}>
          {selectedStrategy ? (
            <ScrollArea className="h-full">
              <div className="p-6 space-y-6">
                {/* Header */}
                <div className="flex items-start justify-between">
                  <div>
                    <h2 className="text-xl font-bold">{selectedStrategy.name}</h2>
                    <p className="text-sm text-muted-foreground mt-1">
                      {selectedStrategy.parent
                        ? "Client-owned strategy"
                        : "Shared routing strategy"}
                    </p>
                  </div>
                  <EntityActions
                    actions={[
                      commonActions.edit(() => openEditDialog(selectedStrategy)),
                      {
                        ...commonActions.delete(() => openDeleteDialog(selectedStrategy)),
                        disabled: strategyClients.length > 0,
                      },
                    ]}
                  />
                </div>

                {/* Clients Using This Strategy */}
                <Card>
                  <CardHeader className="pb-3">
                    <CardTitle className="text-sm flex items-center gap-2">
                      <Users className="h-4 w-4" />
                      Clients Using This Strategy
                    </CardTitle>
                    <CardDescription>
                      {strategyClients.length > 0
                        ? "Changes to this strategy will affect all listed clients"
                        : "No clients are using this strategy"}
                    </CardDescription>
                  </CardHeader>
                  {strategyClients.length > 0 && (
                    <CardContent>
                      <div className="flex flex-wrap gap-2">
                        {strategyClients.map((client) => (
                          <Badge
                            key={client.id}
                            variant={client.enabled ? "default" : "secondary"}
                            className="cursor-pointer hover:bg-primary/80 transition-colors"
                            onClick={() => handleClientClick(client)}
                          >
                            {client.name}
                            {!client.enabled && " (disabled)"}
                          </Badge>
                        ))}
                      </div>
                    </CardContent>
                  )}
                </Card>

                {/* Strategy Configuration */}
                <StrategyModelConfiguration
                  strategyId={selectedStrategy.id}
                  readOnly={false}
                  onSave={loadData}
                />
              </div>
            </ScrollArea>
          ) : (
            <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-4">
              <Route className="h-12 w-12 opacity-30" />
              <div className="text-center">
                <p className="font-medium">Select a strategy to configure</p>
                <p className="text-sm">
                  or create a new one with the + button
                </p>
              </div>
            </div>
          )}
        </ResizablePanel>
      </ResizablePanelGroup>

      {/* Create Strategy Dialog */}
      <Dialog open={createDialogOpen} onOpenChange={setCreateDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create New Strategy</DialogTitle>
            <DialogDescription>
              Create a reusable routing strategy that can be assigned to multiple clients.
            </DialogDescription>
          </DialogHeader>
          <div className="grid gap-4 py-4">
            <div className="grid gap-2">
              <Label htmlFor="strategy-name">Strategy Name</Label>
              <Input
                id="strategy-name"
                placeholder="e.g., Development, Production, Cost Optimized"
                value={newStrategyName}
                onChange={(e) => setNewStrategyName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleCreateStrategy()
                }}
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setCreateDialogOpen(false)}>
              Cancel
            </Button>
            <Button onClick={handleCreateStrategy} disabled={!newStrategyName.trim()}>
              Create Strategy
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Edit Strategy Dialog */}
      <Dialog open={editDialogOpen} onOpenChange={setEditDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Edit Strategy</DialogTitle>
            <DialogDescription>
              Update the strategy name. Use the configuration panel below to modify allowed models and routing settings.
            </DialogDescription>
          </DialogHeader>
          <div className="grid gap-4 py-4">
            <div className="grid gap-2">
              <Label htmlFor="edit-strategy-name">Strategy Name</Label>
              <Input
                id="edit-strategy-name"
                value={newStrategyName}
                onChange={(e) => setNewStrategyName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleRenameStrategy()
                }}
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setEditDialogOpen(false)}>
              Cancel
            </Button>
            <Button
              onClick={handleRenameStrategy}
              disabled={!newStrategyName.trim()}
            >
              Save
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation Dialog */}
      <AlertDialog open={deleteDialogOpen} onOpenChange={setDeleteDialogOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Strategy</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to delete "{strategyToDelete?.name}"? This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleDeleteStrategy}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
