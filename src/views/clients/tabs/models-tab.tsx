/**
 * ClientModelsTab Component
 *
 * Models configuration tab for a client.
 * Features:
 * 1. Strategy selection (default client strategy or shared strategies)
 * 2. Embedded StrategyModelConfiguration for configuring the selected strategy
 */

import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Route, AlertTriangle, ExternalLink } from "lucide-react"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { StrategyModelConfiguration, StrategyConfig } from "@/components/strategy"

interface Client {
  id: string
  name: string
  client_id: string
  strategy_id: string
}

interface ModelsTabProps {
  client: Client
  onUpdate: () => void
  initialMode?: "forced" | "multi" | "prioritized" | null
}

export function ClientModelsTab({
  client,
  onUpdate,
  initialMode: _initialMode,
}: ModelsTabProps) {
  const [strategies, setStrategies] = useState<StrategyConfig[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    loadStrategies()
  }, [])

  const loadStrategies = async () => {
    try {
      setLoading(true)
      const strategiesList = await invoke<StrategyConfig[]>("list_strategies")
      setStrategies(strategiesList)
    } catch (error) {
      console.error("Failed to load strategies:", error)
    } finally {
      setLoading(false)
    }
  }

  // Get the current strategy
  const currentStrategy = strategies.find((s) => s.id === client.strategy_id)

  // Check if using a shared strategy (not owned by this client)
  const isSharedStrategy =
    currentStrategy && currentStrategy.parent !== client.id

  // Get owned strategies (this client's personal strategy)
  const ownedStrategies = strategies.filter((s) => s.parent === client.id)

  // Handle strategy change
  const handleStrategyChange = async (newStrategyId: string) => {
    try {
      await invoke("assign_client_strategy", {
        clientId: client.id,
        strategyId: newStrategyId,
      })
      toast.success("Strategy assigned")
      onUpdate()
    } catch (error) {
      console.error("Failed to assign strategy:", error)
      toast.error("Failed to assign strategy")
    }
  }

  // Handle create personal strategy
  const handleCreatePersonalStrategy = async () => {
    try {
      const newStrategy = await invoke<StrategyConfig>("create_strategy", {
        name: `${client.name} Strategy`,
        parent: client.id,
      })
      toast.success("Personal strategy created")
      await handleStrategyChange(newStrategy.id)
      loadStrategies()
    } catch (error) {
      console.error("Failed to create personal strategy:", error)
      toast.error("Failed to create personal strategy")
    }
  }

  if (loading) {
    return (
      <div className="space-y-6">
        <Card>
          <CardContent className="py-8">
            <div className="text-center text-muted-foreground">
              Loading strategy configuration...
            </div>
          </CardContent>
        </Card>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      {/* Strategy Selection */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Route className="h-5 w-5" />
            Routing Strategy
          </CardTitle>
          <CardDescription>
            Select which routing strategy controls this client's model access and
            routing behavior
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center gap-4">
            <Select
              value={client.strategy_id}
              onValueChange={handleStrategyChange}
            >
              <SelectTrigger className="flex-1">
                <SelectValue placeholder="Select a strategy" />
              </SelectTrigger>
              <SelectContent className="min-w-[300px]">
                {strategies.map((strategy) => {
                  const isDefault = strategy.id === "default"
                  const isOwned = strategy.parent === client.id

                  return (
                    <SelectItem key={strategy.id} value={strategy.id}>
                      <div className="flex items-center gap-2 w-full">
                        <span className="flex-1">{strategy.name}</span>
                        {isDefault && (
                          <Badge variant="secondary" className="text-xs shrink-0">
                            Default
                          </Badge>
                        )}
                        {isOwned && (
                          <Badge variant="outline" className="text-xs shrink-0">
                            Personal
                          </Badge>
                        )}
                      </div>
                    </SelectItem>
                  )
                })}
              </SelectContent>
            </Select>

            {ownedStrategies.length === 0 && (
              <Button variant="outline" onClick={handleCreatePersonalStrategy}>
                Create Personal Strategy
              </Button>
            )}
          </div>

          {/* Shared Strategy Warning */}
          {isSharedStrategy && (
            <Alert>
              <AlertTriangle className="h-4 w-4" />
              <AlertTitle>Shared Strategy</AlertTitle>
              <AlertDescription>
                This strategy is shared with other clients. Changes you make here
                will affect all clients using this strategy.
                <Button
                  variant="link"
                  className="h-auto p-0 ml-1"
                  onClick={handleCreatePersonalStrategy}
                >
                  Create a personal strategy instead
                  <ExternalLink className="h-3 w-3 ml-1" />
                </Button>
              </AlertDescription>
            </Alert>
          )}

          {/* Current Strategy Info */}
          {currentStrategy && (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <span>Currently using:</span>
              <Badge
                variant={
                  currentStrategy.parent === client.id
                    ? "default"
                    : currentStrategy.id === "default"
                    ? "secondary"
                    : "outline"
                }
              >
                {currentStrategy.name}
              </Badge>
              {currentStrategy.parent === client.id && (
                <span className="text-xs">(Personal strategy for this client)</span>
              )}
            </div>
          )}
        </CardContent>
      </Card>

      {/* Strategy Configuration */}
      {client.strategy_id && (
        <StrategyModelConfiguration
          strategyId={client.strategy_id}
          readOnly={false}
          onSave={onUpdate}
        />
      )}
    </div>
  )
}
