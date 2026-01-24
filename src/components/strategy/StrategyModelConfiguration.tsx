/**
 * StrategyModelConfiguration Component
 *
 * Main component for configuring model routing for a strategy.
 * Contains two sections:
 * 1. Allowed Models - Hierarchical selection of which models are permitted
 * 2. Auto Router - Enable localrouter/auto with prioritized models and optional
 *    Strong/Weak routing (RouteLLM) displayed side-by-side
 *
 * Used in:
 * - Client -> Models tab
 * - Resources -> Model Routing tab
 */

import {useCallback, useEffect, useRef, useState} from "react"
import {invoke} from "@tauri-apps/api/core"
import {listen} from "@tauri-apps/api/event"
import {Bot, Brain, MessageSquareWarning} from "lucide-react"
import {Card, CardContent, CardDescription, CardHeader, CardTitle,} from "@/components/ui/Card"
import {Switch} from "@/components/ui/Toggle"
import {Input} from "@/components/ui/Input"
import {Badge} from "@/components/ui/Badge"
import {cn} from "@/lib/utils"
import {AllowedModelsSelection, AllowedModelsSelector, Model,} from "./AllowedModelsSelector"
import {DragThresholdModelSelector} from "./DragThresholdModelSelector"
import {ThresholdSlider} from "@/components/routellm/ThresholdSlider"
import {ROUTELLM_REQUIREMENTS, RouteLLMState, RouteLLMStatus, RouteLLMTestResult} from "@/components/routellm/types"

// Strategy configuration types
export interface AutoModelConfig {
    enabled: boolean
    prioritized_models: [string, string][]
    available_models: [string, string][]
    routellm_config?: RouteLLMConfig
}

export interface RouteLLMConfig {
    enabled: boolean
    threshold: number
    weak_models: [string, string][]
}

export interface StrategyConfig {
    id: string
    name: string
    parent: string | null
    allowed_models: AllowedModelsSelection
    auto_config: AutoModelConfig | null
    rate_limits: any[]
}

interface StrategyModelConfigurationProps {
    /**
     * Strategy ID to configure
     */
    strategyId: string
    /**
     * Whether the configuration is read-only
     */
    readOnly?: boolean
    /**
     * Callback when configuration is saved
     */
    onSave?: () => void
    /**
     * Additional CSS classes
     */
    className?: string
}

export function StrategyModelConfiguration({
                                               strategyId,
                                               readOnly = false,
                                               onSave,
                                               className,
                                           }: StrategyModelConfigurationProps) {
    const [strategy, setStrategy] = useState<StrategyConfig | null>(null)
    const [models, setModels] = useState<Model[]>([])
    const [loading, setLoading] = useState(true)
    const [saving, setSaving] = useState(false)

    // RouteLLM test state
    const [routellmStatus, setRoutellmStatus] = useState<RouteLLMStatus | null>(null)
    const [testPrompt, setTestPrompt] = useState("")
    const [isTesting, setIsTesting] = useState(false)
    const [testResult, setTestResult] = useState<RouteLLMTestResult | null>(null)

    // Debounce refs for update operations
    const updateTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
    const pendingUpdatesRef = useRef<Partial<StrategyConfig> | null>(null)

    // Load data on mount
    useEffect(() => {
        loadData()
    }, [strategyId])

    // Cleanup debounce timeout on unmount
    useEffect(() => {
        return () => {
            if (updateTimeoutRef.current) {
                clearTimeout(updateTimeoutRef.current)
            }
        }
    }, [])

    // Load RouteLLM status and listen for updates
    useEffect(() => {
        const loadRouteLLMStatus = async () => {
            try {
                const status = await invoke<RouteLLMStatus>("routellm_get_status")
                setRoutellmStatus(status)
            } catch (error) {
                console.error("Failed to load RouteLLM status:", error)
            }
        }

        loadRouteLLMStatus()

        // Listen for download events to update status
        const unlistenComplete = listen("routellm-download-complete", () => {
            loadRouteLLMStatus()
        })

        // Poll status while testing or initializing
        const interval = setInterval(() => {
            if (routellmStatus?.state === 'initializing' || routellmStatus?.state === 'downloading') {
                loadRouteLLMStatus()
            }
        }, 1000)

        return () => {
            unlistenComplete.then((fn) => fn())
            clearInterval(interval)
        }
    }, [routellmStatus?.state])

    const loadData = async () => {
        setLoading(true)
        try {
            const [strategyData, modelsData] = await Promise.all([
                invoke<StrategyConfig>("get_strategy", {strategyId}),
                invoke<Model[]>("list_all_models"),
            ])
            setStrategy(strategyData)
            setModels(modelsData)
        } catch (error) {
            console.error("Failed to load strategy:", error)
        } finally {
            setLoading(false)
        }
    }

    // Update strategy on backend with debouncing to prevent race conditions
    const updateStrategy = useCallback((updates: Partial<StrategyConfig>) => {
        if (!strategy || readOnly) return

        // Merge with pending updates
        pendingUpdatesRef.current = {
            ...pendingUpdatesRef.current,
            ...updates,
        }

        // Update local state immediately for responsive UI
        setStrategy((prev) => (prev ? {...prev, ...updates} : null))

        // Clear existing timeout
        if (updateTimeoutRef.current) {
            clearTimeout(updateTimeoutRef.current)
        }

        // Debounce the API call
        updateTimeoutRef.current = setTimeout(async () => {
            const pendingUpdates = pendingUpdatesRef.current
            pendingUpdatesRef.current = null

            if (!pendingUpdates) return

            setSaving(true)
            try {
                await invoke("update_strategy", {
                    strategyId: strategy.id,
                    name: null,
                    allowedModels: pendingUpdates.allowed_models || null,
                    autoConfig: pendingUpdates.auto_config !== undefined ? pendingUpdates.auto_config : null,
                    rateLimits: null,
                })
                onSave?.()
            } catch (error) {
                console.error("Failed to update strategy:", error)
            } finally {
                setSaving(false)
            }
        }, 300) // 300ms debounce
    }, [strategy, readOnly, onSave])

    // Handler for allowed models change
    const handleAllowedModelsChange = (selection: AllowedModelsSelection) => {
        updateStrategy({allowed_models: selection})
    }

    // Handler for auto config toggle
    const handleAutoConfigToggle = (enabled: boolean) => {
        const newConfig: AutoModelConfig = {
            enabled,
            prioritized_models: strategy?.auto_config?.prioritized_models || [],
            available_models: strategy?.auto_config?.available_models || [],
            routellm_config: strategy?.auto_config?.routellm_config,
        }
        updateStrategy({auto_config: newConfig})
    }

    // Handler for prioritized models change
    const handlePrioritizedModelsChange = (models: [string, string][]) => {
        if (!strategy?.auto_config) return
        updateStrategy({
            auto_config: {
                ...strategy.auto_config,
                prioritized_models: models,
            },
        })
    }

    // Handler for RouteLLM toggle
    const handleRouteLLMToggle = (enabled: boolean) => {
        if (!strategy?.auto_config) return

        const newRouteLLMConfig: RouteLLMConfig = {
            enabled,
            threshold: strategy.auto_config.routellm_config?.threshold ?? 0.3,
            weak_models: strategy.auto_config.routellm_config?.weak_models ?? [],
        }

        updateStrategy({
            auto_config: {
                ...strategy.auto_config,
                routellm_config: newRouteLLMConfig,
            },
        })
    }

    // Handler for threshold change
    const handleThresholdChange = (threshold: number) => {
        if (!strategy?.auto_config?.routellm_config) return
        updateStrategy({
            auto_config: {
                ...strategy.auto_config,
                routellm_config: {
                    ...strategy.auto_config.routellm_config,
                    threshold,
                },
            },
        })
    }

    // Handler for weak models change
    const handleWeakModelsChange = (models: [string, string][]) => {
        if (!strategy?.auto_config?.routellm_config) return
        updateStrategy({
            auto_config: {
                ...strategy.auto_config,
                routellm_config: {
                    ...strategy.auto_config.routellm_config,
                    weak_models: models,
                },
            },
        })
    }

    // Handler for testing a prompt
    const handleTest = async () => {
        if (!testPrompt.trim() || !strategy?.auto_config?.routellm_config) return

        setIsTesting(true)
        setTestResult(null)
        try {
            const result = await invoke<RouteLLMTestResult>("routellm_test_prediction", {
                prompt: testPrompt.trim(),
                threshold: strategy.auto_config.routellm_config.threshold,
            })
            setTestResult(result)
            // Refresh status after test (it may have loaded the model)
            const status = await invoke<RouteLLMStatus>("routellm_get_status")
            setRoutellmStatus(status)
        } catch (err: any) {
            console.error("Test failed:", err)
        } finally {
            setIsTesting(false)
        }
    }

    // Get status display info
    const getStatusInfo = (state: RouteLLMState) => {
        switch (state) {
            case "not_downloaded":
                return {label: "Not Downloaded", variant: "secondary" as const, icon: "⬇️"}
            case "downloading":
                return {label: "Downloading...", variant: "default" as const, icon: "⏳"}
            case "downloaded_not_running":
                return {label: "Ready", variant: "outline" as const, icon: "⏸️"}
            case "initializing":
                return {label: "Initializing...", variant: "default" as const, icon: "🔄"}
            case "started":
                return {label: "Active", variant: "success" as const, icon: "✓"}
            default:
                return {label: "Unknown", variant: "secondary" as const, icon: "?"}
        }
    }

    if (loading) {
        return (
            <div className={cn("space-y-4", className)}>
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

    if (!strategy) {
        return (
            <div className={cn("space-y-4", className)}>
                <Card>
                    <CardContent className="py-8">
                        <div className="text-center text-destructive">
                            Strategy not found
                        </div>
                    </CardContent>
                </Card>
            </div>
        )
    }

    const autoConfig = strategy.auto_config
    const routellmConfig = autoConfig?.routellm_config

    return (
        <div className={cn("space-y-4", className)}>
            {/* Section 1: Allowed Models */}
            <Card>
                <CardHeader>
                    <CardTitle className="text-base">Allowed Models</CardTitle>
                    <CardDescription>
                        Select which models client can access
                    </CardDescription>
                </CardHeader>
                <CardContent>
                    <AllowedModelsSelector
                        models={models}
                        value={strategy.allowed_models}
                        onChange={handleAllowedModelsChange}
                        disabled={readOnly || saving}
                    />
                </CardContent>
            </Card>

            {/* Section 2: Auto Router & Strong/Weak Routing - Side by side */}
            <div className="grid gap-4 grid-cols-1 lg:grid-cols-2">
                {/* Left: Auto Router (Strong Models) */}
                <Card>
                    <CardHeader>
                        <div className="flex items-center justify-between">
                            <div className="flex items-center gap-3">
                                <div className="p-2 rounded-lg bg-primary/10">
                                    <Bot className="h-4 w-4 text-primary"/>
                                </div>
                                <div>
                                    <CardTitle className="text-base flex items-center gap-2">
                                        Auto Router
                                        <CardDescription>
                                            Choose
                                            {" "}
                                        <code className="text-xs bg-muted px-1.5 py-0.5 rounded font-mono">
                                            localrouter/auto
                                        </code>
                                            {" "}
                                            model.
                                        </CardDescription>
                                    </CardTitle>
                                    <CardDescription>
                                        Prioritize models to try in case of failures. (e.g. outage, context limit, policy violation)
                                        Span multiple online providers and fallback to local models.
                                    </CardDescription>
                                </div>
                            </div>
                            <Switch
                                checked={autoConfig?.enabled ?? false}
                                onCheckedChange={handleAutoConfigToggle}
                                disabled={readOnly || saving}
                            />
                        </div>
                    </CardHeader>
                    <CardContent className="space-y-4">
                        {!autoConfig?.enabled ? (
                            <div className="text-center py-8 text-muted-foreground text-sm">
                                Enable to configure prioritized model fallback
                            </div>
                        ) : (
                            <>
                                <DragThresholdModelSelector
                                    availableModels={models}
                                    enabledModels={autoConfig.prioritized_models}
                                    onChange={handlePrioritizedModelsChange}
                                    disabled={readOnly || saving}
                                />
                            </>
                        )}
                    </CardContent>
                </Card>

                {/* Right: Strong/Weak Routing (Weak Models) */}
                <Card>
                    <CardHeader>
                        <div className="flex items-center justify-between">
                            <div className="flex items-center gap-3">
                                <div className="p-2 rounded-lg bg-purple-500/10">
                                    <Brain className="h-4 w-4 text-purple-500"/>
                                </div>
                                <div>
                                    <CardTitle className="text-base flex items-center gap-2">
                                        Weak Model
                                        <span
                                            className="text-xs px-1.5 py-0.5 rounded bg-purple-500/20 text-purple-700 dark:text-purple-300 font-medium">
                                            EXPERIMENTAL
                                        </span>
                                    </CardTitle>
                                    <CardDescription>
                                        Use weaker models for simpler prompts for faster and cheaper results.
                                        Every request is determined for complexity using local Machine Learning model.
                                    </CardDescription>
                                </div>
                            </div>
                            <Switch
                                checked={routellmConfig?.enabled ?? false}
                                onCheckedChange={handleRouteLLMToggle}
                                disabled={readOnly || saving || !autoConfig?.enabled}
                            />
                        </div>
                    </CardHeader>
                    <CardContent className="space-y-4">
                        {!autoConfig?.enabled ? (
                            <div className="text-center py-8 text-muted-foreground text-sm">
                                Enable Auto Router first
                            </div>
                        ) : !routellmConfig?.enabled ? (
                            <div className="space-y-4">
                                {/* Resource Requirements - shown when disabled */}
                                <div className="p-3 rounded-lg bg-amber-500/10 border border-amber-500/20">
                                    <div className="flex items-start gap-2">
                                        <MessageSquareWarning
                                            className="h-4 w-4 text-amber-600 dark:text-amber-400 mt-0.5 shrink-0"/>
                                        <div className="text-xs text-amber-700 dark:text-amber-300">
                                            <p className="font-medium mb-2">Resource Requirements</p>
                                            <div className="grid grid-cols-2 gap-x-4 gap-y-1">
                                                <span>Disk Space:</span>
                                                <span>{ROUTELLM_REQUIREMENTS.DISK_GB} GB</span>
                                                <span>Memory:</span>
                                                <span>{ROUTELLM_REQUIREMENTS.MEMORY_GB} GB</span>
                                                <span>Cold Start:</span>
                                                <span>{ROUTELLM_REQUIREMENTS.COLD_START_SECS}s</span>
                                                <span>Per-request:</span>
                                                <span>{ROUTELLM_REQUIREMENTS.PER_REQUEST_MS}ms</span>
                                            </div>
                                        </div>
                                    </div>
                                </div>
                            </div>
                        ) : (
                            <>
                                <DragThresholdModelSelector
                                    availableModels={models}
                                    enabledModels={routellmConfig.weak_models}
                                    onChange={handleWeakModelsChange}
                                    disabled={readOnly || saving}
                                />

                                {/* Threshold Slider */}
                                <ThresholdSlider
                                    value={routellmConfig.threshold}
                                    onChange={handleThresholdChange}
                                />

                                {/* Test It Out Section */}
                                <div className="mt-4 pt-4 border-t border-border/50">
                                    <div className="space-y-3">
                                        <div className="flex items-center justify-between">
                                            <span
                                                className="text-xs font-medium text-muted-foreground uppercase tracking-wide">
                                                Test It Out
                                            </span>
                                            {routellmStatus && (
                                                <div className="flex items-center gap-1.5">
                                                    <span
                                                        className="text-sm">{getStatusInfo(routellmStatus.state).icon}</span>
                                                    <Badge variant={getStatusInfo(routellmStatus.state).variant}
                                                           className="text-xs">
                                                        {getStatusInfo(routellmStatus.state).label}
                                                    </Badge>
                                                </div>
                                            )}
                                        </div>

                                        <Input
                                            value={testPrompt}
                                            onChange={(e) => setTestPrompt(e.target.value)}
                                            placeholder="Type a prompt and press Enter..."
                                            onKeyDown={(e) => e.key === "Enter" && !isTesting && handleTest()}
                                            disabled={isTesting || routellmStatus?.state === "not_downloaded" || routellmStatus?.state === "downloading"}
                                            className="text-sm"
                                        />

                                        {isTesting && (
                                            <div className="flex items-center gap-2 text-sm text-muted-foreground">
                                                <span className="animate-spin">🔄</span>
                                                <span>
                                                    {routellmStatus?.state === "initializing"
                                                        ? "Loading model..."
                                                        : "Testing..."}
                                                </span>
                                            </div>
                                        )}

                                        {testResult && !isTesting && (
                                            <div className="p-3 bg-muted rounded-lg space-y-2">
                                                <div className="flex items-center justify-between">
                                                    <span className="text-xs text-muted-foreground">
                                                        Score:{" "}
                                                        <span className="font-mono text-primary">
                                                            {testResult.win_rate.toFixed(3)}
                                                        </span>
                                                    </span>
                                                    <Badge variant={testResult.is_strong ? "default" : "secondary"}>
                                                        {testResult.is_strong ? "STRONG" : "weak"} model
                                                    </Badge>
                                                </div>
                                                <div className="w-full bg-background rounded h-1.5 overflow-hidden">
                                                    <div
                                                        className="h-full bg-gradient-to-r from-green-500 to-orange-500"
                                                        style={{width: `${testResult.win_rate * 100}%`}}
                                                    />
                                                </div>
                                                <div className="text-xs text-muted-foreground">
                                                    Latency: {testResult.latency_ms}ms
                                                </div>
                                            </div>
                                        )}

                                        {routellmStatus?.state === "not_downloaded" && (
                                            <p className="text-xs text-muted-foreground">
                                                Download the RouteLLM model in Settings → RouteLLM to test predictions.
                                            </p>
                                        )}
                                    </div>
                                </div>
                            </>
                        )}
                    </CardContent>
                </Card>
            </div>
        </div>
    )
}
