/**
 * ClientModelsTab Component
 *
 * Models configuration tab for a client.
 * Features:
 * 1. Strategy section - strategy selection
 * 2. Rate Limits section - nested under strategy with tree connector
 * 3. Model configuration - nested under strategy with tree connector
 * 4. Model Permissions - when using specific models (not "all")
 */

import { useState, useEffect, useCallback, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
// DEPRECATED: Route, AlertTriangle unused - Strategy UI hidden
import { /* Route, AlertTriangle, */ Gauge } from "lucide-react"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/Card"
// DEPRECATED: Badge, Button, Select, Alert unused - Strategy selector hidden
// import { Badge } from "@/components/ui/Badge"
// import { Button } from "@/components/ui/Button"
// import {
//   Select,
//   SelectContent,
//   SelectItem,
//   SelectTrigger,
//   SelectValue,
// } from "@/components/ui/Select"
// import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { StrategyModelConfiguration, StrategyConfig } from "@/components/strategy"
import RateLimitEditor, { StrategyRateLimit } from "@/components/strategies/RateLimitEditor"
import type { ModelPermissions } from "@/components/permissions"


interface Client {
  id: string
  name: string
  client_id: string
  strategy_id: string
  model_permissions: ModelPermissions
}

interface ModelsTabProps {
  client: Client
  onUpdate: () => void
  initialMode?: "forced" | "multi" | "prioritized" | null
  onViewChange?: (view: string, subTab?: string | null) => void
}

export function ClientModelsTab({
  client,
  onUpdate,
  initialMode: _initialMode,
  onViewChange,
}: ModelsTabProps) {
  const [strategies, setStrategies] = useState<StrategyConfig[]>([])
  const [loading, setLoading] = useState(true)
  const [savingRateLimits, setSavingRateLimits] = useState(false)

  // Debounce ref for rate limit updates
  const rateLimitTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    loadStrategies()
  }, [])

  // Cleanup debounce timeout on unmount
  useEffect(() => {
    return () => {
      if (rateLimitTimeoutRef.current) {
        clearTimeout(rateLimitTimeoutRef.current)
      }
    }
  }, [])

  const loadStrategies = async (showLoading = true) => {
    try {
      if (showLoading) {
        setLoading(true)
      }
      const strategiesList = await invoke<StrategyConfig[]>("list_strategies")
      setStrategies(strategiesList)
    } catch (error) {
      console.error("Failed to load strategies:", error)
    } finally {
      if (showLoading) {
        setLoading(false)
      }
    }
  }

  // Get the current strategy
  const currentStrategy = strategies.find((s) => s.id === client.strategy_id)

  // DEPRECATED: Strategy selector hidden - 1:1 client-to-strategy relationship
  // const isSharedStrategy =
  //   currentStrategy && currentStrategy.parent !== client.id
  // const ownedStrategies = strategies.filter((s) => s.parent === client.id)
  //
  // const handleStrategyChange = async (newStrategyId: string) => {
  //   try {
  //     await invoke("assign_client_strategy", {
  //       clientId: client.id,
  //       strategyId: newStrategyId,
  //     })
  //     toast.success("Strategy assigned")
  //     onUpdate()
  //     loadStrategies(false)
  //   } catch (error) {
  //     console.error("Failed to assign strategy:", error)
  //     toast.error("Failed to assign strategy")
  //   }
  // }
  //
  // const handleCreatePersonalStrategy = async () => {
  //   try {
  //     const newStrategy = await invoke<StrategyConfig>("create_strategy", {
  //       name: `${client.name} Strategy`,
  //       parent: client.id,
  //     })
  //     toast.success("Personal strategy created")
  //     await handleStrategyChange(newStrategy.id)
  //     loadStrategies(false)
  //   } catch (error) {
  //     console.error("Failed to create personal strategy:", error)
  //     toast.error("Failed to create personal strategy")
  //   }
  // }

  // Handle rate limits change with debouncing
  const handleRateLimitsChange = useCallback((limits: StrategyRateLimit[]) => {
    if (!currentStrategy) return

    // Update local state immediately for responsive UI
    setStrategies(prev => prev.map(s =>
      s.id === currentStrategy.id
        ? { ...s, rate_limits: limits }
        : s
    ))

    // Clear existing timeout
    if (rateLimitTimeoutRef.current) {
      clearTimeout(rateLimitTimeoutRef.current)
    }

    // Debounce the API call
    rateLimitTimeoutRef.current = setTimeout(async () => {
      setSavingRateLimits(true)
      try {
        await invoke("update_strategy", {
          strategyId: currentStrategy.id,
          name: null,
          allowedModels: null,
          autoConfig: null,
          rateLimits: limits,
        })
        toast.success("Rate limits updated")
        onUpdate()
      } catch (error) {
        console.error("Failed to update rate limits:", error)
        toast.error("Failed to update rate limits")
        // Reload to restore correct state
        loadStrategies(false)
      } finally {
        setSavingRateLimits(false)
      }
    }, 500)
  }, [currentStrategy, onUpdate])

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
    <div>
      {/* DEPRECATED: Strategy selector hidden - 1:1 client-to-strategy relationship */}
      {/* <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Route className="h-5 w-5" />
            Strategy
          </CardTitle>
          <CardDescription>
            Choose an existing strategy or{" "}
            {onViewChange ? (
              <button
                onClick={() => onViewChange("resources", "strategies")}
                className="text-primary hover:underline"
              >
                create a new one in Resources
              </button>
            ) : (
              "create a new one in Resources"
            )}
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
                  const isOwned = strategy.parent === client.id

                  return (
                    <SelectItem key={strategy.id} value={strategy.id}>
                      <div className="flex items-center gap-2 w-full">
                        <span className="flex-1">{strategy.name}</span>
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

          {isSharedStrategy && (
            <Alert>
              <AlertTriangle className="h-4 w-4" />
              <AlertTitle>Shared Strategy</AlertTitle>
              <AlertDescription>
                This strategy is shared with other clients. Changes you make here
                will affect all clients using this strategy.
              </AlertDescription>
            </Alert>
          )}
        </CardContent>
      </Card> */}

      {/* Model configuration sections */}
      {client.strategy_id && (
        <div className="space-y-4">
          {/* Rate Limits */}
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2 text-base">
                <Gauge className="h-4 w-4" />
                Rate Limits
              </CardTitle>
              <CardDescription>
                Set usage limits to control costs and prevent abuse
              </CardDescription>
            </CardHeader>
            <CardContent>
              {currentStrategy && (
                <RateLimitEditor
                  limits={currentStrategy.rate_limits || []}
                  onChange={handleRateLimitsChange}
                  disabled={savingRateLimits}
                />
              )}
            </CardContent>
          </Card>

          {/* Model Configuration - with unified permissions when using Allowed Models mode */}
          <StrategyModelConfiguration
            strategyId={client.strategy_id}
            readOnly={false}
            onSave={() => {
              onUpdate()
              loadStrategies(false)
            }}
            clientContext={{
              clientId: client.client_id,
              modelPermissions: client.model_permissions,
              onClientUpdate: onUpdate,
            }}
            onTabChange={onViewChange}
          />
        </div>
      )}
    </div>
  )
}
