
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Plus, ArrowLeft, Settings2, Users, Zap, Gauge } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog"

interface Strategy {
  id: string
  name: string
  parent: string | null
  auto_config?: {
    enabled: boolean
    mode: string
    prioritized_models: string[]
    weak_models: string[]
    threshold?: number
  }
  rate_limits: any[]
}

interface Client {
  id: string
  name: string
  strategy_id: string
  enabled: boolean
}

interface RoutingTabProps {
  selectedStrategyId: string | null
  onSelectStrategy: (id: string | null) => void
}

export function RoutingTab({ selectedStrategyId, onSelectStrategy }: RoutingTabProps) {
  const [strategies, setStrategies] = useState<Strategy[]>([])
  const [clients, setClients] = useState<Client[]>([])
  const [loading, setLoading] = useState(true)
  const [isCreateOpen, setIsCreateOpen] = useState(false)
  const [newStrategyName, setNewStrategyName] = useState("")
  const [isCreating, setIsCreating] = useState(false)

  useEffect(() => {
    loadData()
  }, [])

  const loadData = async () => {
    setLoading(true)
    try {
      const [strategiesData, clientsData] = await Promise.all([
        invoke<Strategy[]>("list_strategies"),
        invoke<Client[]>("list_clients"),
      ])
      setStrategies(strategiesData)
      setClients(clientsData)
    } catch (error) {
      console.error("Failed to load routing data:", error)
      toast.error("Failed to load strategy data")
    } finally {
      setLoading(false)
    }
  }

  const createNewStrategy = async () => {
    if (!newStrategyName.trim()) return

    setIsCreating(true)
    try {
      await invoke("create_strategy", { name: newStrategyName.trim(), parent: null })
      toast.success("Strategy created")
      setNewStrategyName("")
      setIsCreateOpen(false)
      await loadData()
    } catch (error: any) {
      console.error("Failed to create strategy:", error)
      toast.error(`Failed to create: ${error.message || error}`)
    } finally {
      setIsCreating(false)
    }
  }

  const deleteStrategy = async (strategyId: string, strategyName: string) => {
    if (!confirm(`Delete strategy "${strategyName}"? This cannot be undone.`)) return

    try {
      await invoke("delete_strategy", { strategyId })
      toast.success("Strategy deleted")
      if (selectedStrategyId === strategyId) {
        onSelectStrategy(null)
      }
      await loadData()
    } catch (error: any) {
      console.error("Failed to delete strategy:", error)
      toast.error(`Failed to delete: ${error.message || error}`)
    }
  }

  // If viewing a strategy detail, render the detail view
  if (selectedStrategyId) {
    const strategy = strategies.find((s) => s.id === selectedStrategyId)
    if (!strategy && !loading) {
      return (
        <div className="text-center py-12 text-muted-foreground">
          <p>Strategy not found</p>
          <Button
            variant="outline"
            size="sm"
            className="mt-4"
            onClick={() => onSelectStrategy(null)}
          >
            Back to list
          </Button>
        </div>
      )
    }

    if (strategy) {
      return (
        <StrategyDetail
          strategy={strategy}
          clients={clients.filter((c) => c.strategy_id === strategy.id)}
          onBack={() => onSelectStrategy(null)}
          onUpdate={loadData}
          onDelete={() => deleteStrategy(strategy.id, strategy.name)}
        />
      )
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <p className="text-muted-foreground">Loading strategies...</p>
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold">Strategies</h2>
          <p className="text-sm text-muted-foreground">
            Manage reusable strategy configurations with auto-selection and rate limits
          </p>
        </div>
        <Dialog open={isCreateOpen} onOpenChange={setIsCreateOpen}>
          <DialogTrigger asChild>
            <Button size="sm">
              <Plus className="h-4 w-4 mr-1" />
              Create Strategy
            </Button>
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Create Strategy</DialogTitle>
              <DialogDescription>
                Create a new strategy that can be assigned to clients
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-4 py-4">
              <div className="space-y-2">
                <Label htmlFor="strategy-name">Strategy Name</Label>
                <Input
                  id="strategy-name"
                  value={newStrategyName}
                  onChange={(e) => setNewStrategyName(e.target.value)}
                  placeholder="e.g., Production, Development"
                />
              </div>
            </div>
            <DialogFooter>
              <Button variant="outline" onClick={() => setIsCreateOpen(false)}>
                Cancel
              </Button>
              <Button
                onClick={createNewStrategy}
                disabled={!newStrategyName.trim() || isCreating}
              >
                {isCreating ? "Creating..." : "Create"}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </div>

      <Card>
        <CardContent className="p-0">
          {strategies.length === 0 ? (
            <div className="text-center py-12 text-muted-foreground">
              No strategies found. Create one to get started.
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Clients</TableHead>
                  <TableHead>Auto Selection</TableHead>
                  <TableHead>Rate Limits</TableHead>
                  <TableHead>Type</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {strategies.map((strategy) => {
                  const clientsUsing = clients.filter(
                    (c) => c.strategy_id === strategy.id
                  )

                  return (
                    <TableRow
                      key={strategy.id}
                      className="cursor-pointer"
                      onClick={() => onSelectStrategy(strategy.id)}
                    >
                      <TableCell>
                        <div>
                          <p className="font-medium">{strategy.name}</p>
                          <p className="text-xs text-muted-foreground font-mono">
                            {strategy.id}
                          </p>
                        </div>
                      </TableCell>
                      <TableCell>{clientsUsing.length}</TableCell>
                      <TableCell>
                        {strategy.auto_config?.enabled ? (
                          <Badge variant="success" className="text-xs">
                            Enabled
                          </Badge>
                        ) : (
                          <Badge variant="secondary" className="text-xs">
                            Disabled
                          </Badge>
                        )}
                      </TableCell>
                      <TableCell>{strategy.rate_limits.length}</TableCell>
                      <TableCell>
                        {strategy.parent ? (
                          <Badge variant="default" className="text-xs">
                            Owned
                          </Badge>
                        ) : (
                          <Badge variant="outline" className="text-xs">
                            Shared
                          </Badge>
                        )}
                      </TableCell>
                    </TableRow>
                  )
                })}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </div>
  )
}

// Strategy Detail Component
interface StrategyDetailProps {
  strategy: Strategy
  clients: Client[]
  onBack: () => void
  onUpdate: () => void
  onDelete: () => void
}

function StrategyDetail({
  strategy,
  clients,
  onBack,
  onUpdate,
  onDelete,
}: StrategyDetailProps) {
  const [isUpdating, setIsUpdating] = useState(false)
  const [editName, setEditName] = useState(strategy.name)

  const updateStrategyName = async () => {
    if (!editName.trim() || editName === strategy.name) return

    setIsUpdating(true)
    try {
      await invoke("update_strategy", {
        strategyId: strategy.id,
        name: editName.trim(),
      })
      toast.success("Strategy updated")
      onUpdate()
    } catch (error: any) {
      toast.error(`Failed to update: ${error.message || error}`)
    } finally {
      setIsUpdating(false)
    }
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-start justify-between">
        <div className="flex items-center gap-4">
          <Button variant="ghost" size="sm" onClick={onBack}>
            <ArrowLeft className="h-4 w-4 mr-1" />
            Back
          </Button>
          <div>
            <h2 className="text-xl font-bold">{strategy.name}</h2>
            <p className="text-sm text-muted-foreground font-mono">{strategy.id}</p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {strategy.parent ? (
            <Badge>Owned</Badge>
          ) : (
            <Badge variant="outline">Shared</Badge>
          )}
          <Button variant="destructive" size="sm" onClick={onDelete}>
            Delete
          </Button>
        </div>
      </div>

      {/* Overview Stats */}
      <div className="grid gap-4 md:grid-cols-3">
        <Card>
          <CardContent className="pt-6">
            <div className="flex items-center gap-3">
              <Users className="h-8 w-8 text-muted-foreground" />
              <div>
                <p className="text-2xl font-bold">{clients.length}</p>
                <p className="text-xs text-muted-foreground">Clients Using</p>
              </div>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardContent className="pt-6">
            <div className="flex items-center gap-3">
              <Zap className="h-8 w-8 text-muted-foreground" />
              <div>
                <p className="text-2xl font-bold">
                  {strategy.auto_config?.enabled ? "On" : "Off"}
                </p>
                <p className="text-xs text-muted-foreground">Auto Selection</p>
              </div>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardContent className="pt-6">
            <div className="flex items-center gap-3">
              <Gauge className="h-8 w-8 text-muted-foreground" />
              <div>
                <p className="text-2xl font-bold">{strategy.rate_limits.length}</p>
                <p className="text-xs text-muted-foreground">Rate Limits</p>
              </div>
            </div>
          </CardContent>
        </Card>
      </div>

      {/* Configuration */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-2">
            <Settings2 className="h-4 w-4" />
            Configuration
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="strategy-name">Strategy Name</Label>
            <div className="flex gap-2">
              <Input
                id="strategy-name"
                value={editName}
                onChange={(e) => setEditName(e.target.value)}
              />
              <Button
                size="sm"
                onClick={updateStrategyName}
                disabled={isUpdating || editName === strategy.name || !editName.trim()}
              >
                {isUpdating ? "Saving..." : "Save"}
              </Button>
            </div>
          </div>

          {strategy.auto_config && (
            <div className="p-4 bg-muted rounded-lg space-y-2">
              <p className="text-sm font-medium">Auto Selection Configuration</p>
              <div className="grid gap-2 text-sm">
                <div className="flex justify-between">
                  <span className="text-muted-foreground">Status</span>
                  <span>{strategy.auto_config.enabled ? "Enabled" : "Disabled"}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-muted-foreground">Mode</span>
                  <span className="capitalize">{strategy.auto_config.mode}</span>
                </div>
                {strategy.auto_config.threshold !== undefined && (
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Threshold</span>
                    <span>{strategy.auto_config.threshold}</span>
                  </div>
                )}
              </div>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Clients Using This Strategy */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">Clients Using This Strategy</CardTitle>
          <CardDescription>
            {clients.length} client{clients.length !== 1 ? "s" : ""} assigned
          </CardDescription>
        </CardHeader>
        <CardContent>
          {clients.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No clients are using this strategy yet.
            </p>
          ) : (
            <div className="space-y-2">
              {clients.map((client) => (
                <div
                  key={client.id}
                  className="flex items-center justify-between p-2 rounded border"
                >
                  <div>
                    <p className="font-medium text-sm">{client.name}</p>
                    <p className="text-xs text-muted-foreground font-mono">
                      {client.id}
                    </p>
                  </div>
                  <Badge variant={client.enabled ? "success" : "secondary"}>
                    {client.enabled ? "Enabled" : "Disabled"}
                  </Badge>
                </div>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
