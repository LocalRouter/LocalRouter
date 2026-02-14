import { useState, useEffect, useCallback, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Shield, RefreshCw, Plus, Trash2, Loader2, Pencil, FlaskConical, CheckCircle2, XCircle, ChevronDown, ChevronRight, AlertTriangle, Download, Brain, Unplug, FolderOpen, ExternalLink } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/Toggle"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Textarea } from "@/components/ui/textarea"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import type {
  GuardrailsConfig,
  GuardrailSourceConfig,
  GuardrailSourceStatus,
  GuardrailSourceDetails,
  CustomGuardrailRule,
  GuardrailCheckResult,
  GuardrailModelInfo,
  ModelDownloadProgress,
  AddGuardrailSourceParams,
  AddCustomGuardrailRuleParams,
  UpdateCustomGuardrailRuleParams,
  RemoveCustomGuardrailRuleParams,
  TestGuardrailInputParams,
  UpdateGuardrailsConfigParams,
  GetGuardrailSourceDetailsParams,
  DownloadGuardrailModelParams,
  GetGuardrailModelStatusParams,
  UnloadGuardrailModelParams,
  OpenPathParams,
} from "@/types/tauri-commands"

/** Auto-derive a slug ID from a label */
function deriveIdFromLabel(label: string): string {
  return label.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "")
}

export function GuardrailsTab() {
  const [config, setConfig] = useState<GuardrailsConfig>({
    enabled: false,
    scan_requests: true,
    scan_responses: false,
    sources: [],
    min_popup_severity: "medium",
    update_interval_hours: 24,
    custom_rules: [],
  })
  const [sourcesStatus, setSourcesStatus] = useState<GuardrailSourceStatus[]>([])
  const [isLoading, setIsLoading] = useState(true)
  const [updatingSource, setUpdatingSource] = useState<string | null>(null)
  const [updatingAll, setUpdatingAll] = useState(false)

  // Add/Edit source dialog state
  const [showSourceDialog, setShowSourceDialog] = useState(false)
  const [editingSource, setEditingSource] = useState<GuardrailSourceConfig | null>(null)
  const [sourceForm, setSourceForm] = useState({ id: "", label: "", sourceType: "regex", url: "", dataPaths: "", branch: "main" })
  const [savingSource, setSavingSource] = useState(false)

  // Collapsible source details
  const [expandedSource, setExpandedSource] = useState<string | null>(null)
  const [sourceDetails, setSourceDetails] = useState<GuardrailSourceDetails | null>(null)
  const [loadingDetails, setLoadingDetails] = useState(false)

  // Custom rules state
  const [showCustomRuleDialog, setShowCustomRuleDialog] = useState(false)
  const [editingRule, setEditingRule] = useState<CustomGuardrailRule | null>(null)
  const [customRuleForm, setCustomRuleForm] = useState({
    id: "", name: "", pattern: "", category: "prompt_injection", severity: "medium", direction: "input", enabled: true,
  })
  const [patternValid, setPatternValid] = useState<boolean | null>(null)
  const [savingRule, setSavingRule] = useState(false)

  // ML model state
  const [modelStatuses, setModelStatuses] = useState<Record<string, GuardrailModelInfo>>({})
  const [downloadingModels, setDownloadingModels] = useState<Record<string, ModelDownloadProgress>>({})
  const [hfTokens, setHfTokens] = useState<Record<string, string>>({})
  const unlistenRef = useRef<(() => void) | null>(null)

  // Test rules state
  const [testText, setTestText] = useState("")
  const [testResult, setTestResult] = useState<GuardrailCheckResult | null>(null)
  const [testing, setTesting] = useState(false)
  const [testRan, setTestRan] = useState(false)

  const loadConfig = useCallback(async () => {
    try {
      const result = await invoke<GuardrailsConfig>("get_guardrails_config")
      setConfig(result)
    } catch (err) {
      console.error("Failed to load guardrails config:", err)
      toast.error("Failed to load guardrails configuration")
    } finally {
      setIsLoading(false)
    }
  }, [])

  const loadSourcesStatus = useCallback(async () => {
    try {
      const result = await invoke<GuardrailSourceStatus[]>("get_guardrail_sources_status")
      setSourcesStatus(result)
    } catch {
      // Engine may not be initialized yet - that's OK
    }
  }, [])

  const loadModelStatuses = useCallback(async (sources: GuardrailSourceConfig[]) => {
    const modelSources = sources.filter(s => s.source_type === "model")
    for (const source of modelSources) {
      try {
        const info = await invoke<GuardrailModelInfo>("get_guardrail_model_status", {
          sourceId: source.id,
        } satisfies GetGuardrailModelStatusParams as Record<string, unknown>)
        setModelStatuses(prev => ({ ...prev, [source.id]: info }))
      } catch {
        // Model manager may not be available
      }
    }
  }, [])

  useEffect(() => {
    loadConfig()
    loadSourcesStatus()
  }, [loadConfig, loadSourcesStatus])

  // Load model statuses when config is loaded
  useEffect(() => {
    if (config.sources.length > 0) {
      loadModelStatuses(config.sources)
    }
  }, [config.sources, loadModelStatuses])

  // Listen for model download progress events
  useEffect(() => {
    let cancelled = false
    listen<ModelDownloadProgress>("guardrail-model-download-progress", (event) => {
      if (cancelled) return
      const progress = event.payload
      setDownloadingModels(prev => ({ ...prev, [progress.source_id]: progress }))
      if (progress.progress >= 1.0) {
        // Download complete, refresh model status
        setTimeout(() => {
          setDownloadingModels(prev => {
            const next = { ...prev }
            delete next[progress.source_id]
            return next
          })
          loadModelStatuses(config.sources)
        }, 500)
      }
    }).then(unlisten => {
      if (cancelled) {
        unlisten()
      } else {
        unlistenRef.current = unlisten
      }
    })
    return () => {
      cancelled = true
      if (unlistenRef.current) {
        unlistenRef.current()
        unlistenRef.current = null
      }
    }
  }, [config.sources, loadModelStatuses])

  const saveConfig = async (newConfig: GuardrailsConfig) => {
    try {
      await invoke("update_guardrails_config", {
        configJson: JSON.stringify(newConfig),
      } satisfies UpdateGuardrailsConfigParams as Record<string, unknown>)
      setConfig(newConfig)
      toast.success("GuardRails configuration saved")
    } catch (err) {
      console.error("Failed to save guardrails config:", err)
      toast.error("Failed to save configuration")
    }
  }

  const toggleEnabled = (enabled: boolean) => {
    saveConfig({ ...config, enabled })
  }

  const toggleScanRequests = (scan_requests: boolean) => {
    saveConfig({ ...config, scan_requests })
  }

  const toggleScanResponses = (scan_responses: boolean) => {
    saveConfig({ ...config, scan_responses })
  }

  const setMinSeverity = (min_popup_severity: string) => {
    saveConfig({ ...config, min_popup_severity })
  }

  const setUpdateInterval = (update_interval_hours: string) => {
    saveConfig({ ...config, update_interval_hours: parseInt(update_interval_hours) || 24 })
  }

  const toggleSource = (sourceId: string, enabled: boolean) => {
    const newSources = config.sources.map((s) =>
      s.id === sourceId ? { ...s, enabled } : s
    )
    saveConfig({ ...config, sources: newSources })
  }

  const handleUpdateSource = async (sourceId: string) => {
    setUpdatingSource(sourceId)
    try {
      const ruleCount = await invoke<number>("update_guardrail_source", { sourceId })
      await loadSourcesStatus()
      const status = sourcesStatus.find((s) => s.id === sourceId)
      const pathErrors = status?.path_errors || []
      if (ruleCount > 0 && pathErrors.length > 0) {
        toast.success(`Source updated: ${ruleCount} rules loaded, ${pathErrors.length} path(s) failed`)
      } else if (ruleCount > 0) {
        toast.success(`Source updated: ${ruleCount} rules loaded`)
      } else {
        toast.warning("Source updated: 0 rules loaded (source may use unsupported format)")
      }
    } catch (err) {
      console.error("Failed to update source:", err)
      toast.error(`Failed to update source: ${err}`)
    } finally {
      setUpdatingSource(null)
    }
  }

  const handleUpdateAll = async () => {
    setUpdatingAll(true)
    try {
      await invoke("update_all_guardrail_sources")
      toast.success("All sources updated")
      await loadSourcesStatus()
    } catch (err) {
      console.error("Failed to update all sources:", err)
      toast.error(`Failed to update sources: ${err}`)
    } finally {
      setUpdatingAll(false)
    }
  }

  const handleDownloadModel = async (sourceId: string) => {
    try {
      setDownloadingModels(prev => ({ ...prev, [sourceId]: { source_id: sourceId, current_file: "Initializing...", progress: 0, bytes_downloaded: 0, total_bytes: 0, bytes_per_second: 0 } }))
      const token = hfTokens[sourceId] || null
      await invoke("download_guardrail_model", {
        sourceId,
        hfToken: token || undefined,
      } satisfies DownloadGuardrailModelParams as Record<string, unknown>)
    } catch (err) {
      console.error("Failed to download model:", err)
      toast.error(`Failed to download model: ${err}`)
      setDownloadingModels(prev => {
        const next = { ...prev }
        delete next[sourceId]
        return next
      })
    }
  }

  const handleUnloadModel = async (sourceId: string) => {
    try {
      await invoke("unload_guardrail_model", {
        sourceId,
      } satisfies UnloadGuardrailModelParams as Record<string, unknown>)
      toast.success("Model unloaded from memory")
      await loadModelStatuses(config.sources)
    } catch (err) {
      console.error("Failed to unload model:", err)
      toast.error(`Failed to unload model: ${err}`)
    }
  }

  const handleConfidenceThresholdChange = (sourceId: string, threshold: number) => {
    const newSources = config.sources.map(s =>
      s.id === sourceId ? { ...s, confidence_threshold: threshold } : s
    )
    saveConfig({ ...config, sources: newSources })
  }

  const formatBytes = (bytes: number): string => {
    if (bytes === 0) return "~346 MB"
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(0)} MB`
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`
  }

  // Issue 1: Auto-derive ID from label
  const openAddSourceDialog = () => {
    setEditingSource(null)
    setSourceForm({ id: "", label: "", sourceType: "regex", url: "", dataPaths: "", branch: "main" })
    setShowSourceDialog(true)
  }

  // Issue 2: Open edit dialog for custom sources
  const openEditSourceDialog = (source: GuardrailSourceConfig) => {
    setEditingSource(source)
    setSourceForm({
      id: source.id,
      label: source.label,
      sourceType: source.source_type,
      url: source.url,
      dataPaths: source.data_paths.join(", "),
      branch: source.branch,
    })
    setShowSourceDialog(true)
  }

  const handleSaveSource = async () => {
    setSavingSource(true)
    try {
      if (editingSource) {
        // Edit existing source: update via config
        const newSources = config.sources.map((s) =>
          s.id === editingSource.id
            ? {
                ...s,
                label: sourceForm.label,
                source_type: sourceForm.sourceType,
                url: sourceForm.url,
                data_paths: sourceForm.dataPaths.split(",").map((p) => p.trim()).filter(Boolean),
                branch: sourceForm.branch || "main",
              }
            : s
        )
        await saveConfig({ ...config, sources: newSources })
        toast.success(`Source "${sourceForm.label}" updated`)
      } else {
        // Add new source
        const id = deriveIdFromLabel(sourceForm.label)
        await invoke("add_guardrail_source", {
          id,
          label: sourceForm.label,
          sourceType: sourceForm.sourceType,
          url: sourceForm.url,
          dataPaths: sourceForm.dataPaths.split(",").map((p) => p.trim()).filter(Boolean),
          branch: sourceForm.branch || "main",
        } satisfies AddGuardrailSourceParams as Record<string, unknown>)
        toast.success(`Source "${sourceForm.label}" added`)
        await loadConfig()
      }
      setShowSourceDialog(false)
      await loadSourcesStatus()
    } catch (err) {
      console.error("Failed to save source:", err)
      toast.error(`Failed to save source: ${err}`)
    } finally {
      setSavingSource(false)
    }
  }

  const handleRemoveSource = async (sourceId: string) => {
    try {
      await invoke("remove_guardrail_source", { sourceId })
      toast.success("Source removed")
      await loadConfig()
      await loadSourcesStatus()
    } catch (err) {
      console.error("Failed to remove source:", err)
      toast.error(`Failed to remove source: ${err}`)
    }
  }

  // Issue 3: Collapsible source detail panel
  const toggleSourceDetails = async (sourceId: string) => {
    if (expandedSource === sourceId) {
      setExpandedSource(null)
      setSourceDetails(null)
      return
    }

    setExpandedSource(sourceId)
    setLoadingDetails(true)
    try {
      const details = await invoke<GuardrailSourceDetails>("get_guardrail_source_details", {
        sourceId,
      } satisfies GetGuardrailSourceDetailsParams as Record<string, unknown>)
      setSourceDetails(details)
    } catch {
      setSourceDetails(null)
    } finally {
      setLoadingDetails(false)
    }
  }

  const getStatusForSource = (sourceId: string): GuardrailSourceStatus | undefined => {
    return sourcesStatus.find((s) => s.id === sourceId)
  }

  const formatRelativeTime = (isoString: string): string => {
    const diff = Date.now() - new Date(isoString).getTime()
    const minutes = Math.floor(diff / 60000)
    if (minutes < 1) return "just now"
    if (minutes < 60) return `${minutes}m ago`
    const hours = Math.floor(minutes / 60)
    if (hours < 24) return `${hours}h ago`
    const days = Math.floor(hours / 24)
    return `${days}d ago`
  }

  // Custom rule handlers
  const validatePattern = (pattern: string) => {
    if (!pattern) { setPatternValid(null); return }
    try { new RegExp(pattern); setPatternValid(true) } catch { setPatternValid(false) }
  }

  const openAddCustomRule = () => {
    const id = `custom_${Date.now()}`
    setCustomRuleForm({ id, name: "", pattern: "", category: "prompt_injection", severity: "medium", direction: "input", enabled: true })
    setEditingRule(null)
    setPatternValid(null)
    setShowCustomRuleDialog(true)
  }

  const openEditCustomRule = (rule: CustomGuardrailRule) => {
    setCustomRuleForm({ ...rule })
    setEditingRule(rule)
    setPatternValid(true)
    setShowCustomRuleDialog(true)
  }

  const handleSaveCustomRule = async () => {
    setSavingRule(true)
    try {
      const ruleJson = JSON.stringify(customRuleForm)
      if (editingRule) {
        await invoke("update_custom_guardrail_rule", { ruleJson } satisfies UpdateCustomGuardrailRuleParams as Record<string, unknown>)
        toast.success(`Rule "${customRuleForm.name}" updated`)
      } else {
        await invoke("add_custom_guardrail_rule", { ruleJson } satisfies AddCustomGuardrailRuleParams as Record<string, unknown>)
        toast.success(`Rule "${customRuleForm.name}" added`)
      }
      setShowCustomRuleDialog(false)
      await loadConfig()
    } catch (err) {
      toast.error(`Failed to save rule: ${err}`)
    } finally {
      setSavingRule(false)
    }
  }

  const handleRemoveCustomRule = async (ruleId: string) => {
    try {
      await invoke("remove_custom_guardrail_rule", { ruleId } satisfies RemoveCustomGuardrailRuleParams as Record<string, unknown>)
      toast.success("Custom rule removed")
      await loadConfig()
    } catch (err) {
      toast.error(`Failed to remove rule: ${err}`)
    }
  }

  const handleToggleCustomRule = async (rule: CustomGuardrailRule) => {
    try {
      const updated = { ...rule, enabled: !rule.enabled }
      await invoke("update_custom_guardrail_rule", { ruleJson: JSON.stringify(updated) } satisfies UpdateCustomGuardrailRuleParams as Record<string, unknown>)
      await loadConfig()
    } catch (err) {
      toast.error(`Failed to toggle rule: ${err}`)
    }
  }

  const handleTestInput = async () => {
    if (!testText.trim()) return
    setTesting(true)
    setTestResult(null)
    setTestRan(true)
    try {
      const result = await invoke<GuardrailCheckResult>("test_guardrail_input", { text: testText } satisfies TestGuardrailInputParams as Record<string, unknown>)
      setTestResult(result)
    } catch (err) {
      toast.error(`Test failed: ${err}`)
    } finally {
      setTesting(false)
    }
  }

  /** Issue 5G: Render status badge for a source */
  const renderSourceStatusBadge = (status: GuardrailSourceStatus | undefined) => {
    if (!status) return null

    if (status.download_state === "error") {
      return (
        <Badge variant="destructive" className="text-[10px]">
          Error
        </Badge>
      )
    }

    if (status.download_state === "downloading") {
      return (
        <Badge variant="secondary" className="text-[10px]">
          <Loader2 className="h-2.5 w-2.5 mr-1 animate-spin" />
          Downloading
        </Badge>
      )
    }

    if (status.download_state === "ready" || status.download_state === "not_downloaded") {
      const hasErrors = status.path_errors && status.path_errors.length > 0

      if (status.rule_count > 0 && hasErrors) {
        return (
          <Badge className="text-[10px] bg-amber-500 text-white">
            {status.rule_count} rules, {status.path_errors.length} failed
          </Badge>
        )
      }

      if (status.rule_count > 0) {
        return (
          <Badge className="text-[10px] bg-emerald-500 text-white font-mono">
            {status.rule_count} rules
          </Badge>
        )
      }

      if (status.download_state === "ready" && status.rule_count === 0) {
        return (
          <Badge className="text-[10px] bg-amber-500 text-white">
            0 rules
          </Badge>
        )
      }
    }

    return null
  }

  if (isLoading) {
    return <div className="text-sm text-muted-foreground">Loading...</div>
  }

  return (
    <div className="space-y-4">
      {/* Master toggle */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Shield className="h-5 w-5 text-red-500" />
              <CardTitle>GuardRails</CardTitle>
            </div>
            <Switch checked={config.enabled} onCheckedChange={toggleEnabled} />
          </div>
          <CardDescription>
            Scan LLM requests and responses for prompt injection, jailbreaks, PII leakage, and code injection.
            When a rule triggers, a popup lets you Allow or Deny.
          </CardDescription>
        </CardHeader>
      </Card>

      {config.enabled && (
        <>
          {/* Scan settings */}
          <Card>
            <CardHeader>
              <CardTitle className="text-base">Scan Settings</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="flex items-center justify-between">
                <div>
                  <Label>Scan Requests</Label>
                  <p className="text-xs text-muted-foreground">
                    Inspect outgoing prompts before sending to provider
                  </p>
                </div>
                <Switch
                  checked={config.scan_requests}
                  onCheckedChange={toggleScanRequests}
                />
              </div>
              <div className="flex items-center justify-between">
                <div>
                  <Label>Scan Responses</Label>
                  <p className="text-xs text-muted-foreground">
                    Inspect provider responses for sensitive content
                  </p>
                </div>
                <Switch
                  checked={config.scan_responses}
                  onCheckedChange={toggleScanResponses}
                />
              </div>
              <div className="flex items-center justify-between">
                <div>
                  <Label>Min Popup Severity</Label>
                  <p className="text-xs text-muted-foreground">
                    Only show popup for matches at or above this severity
                  </p>
                </div>
                <Select
                  value={config.min_popup_severity}
                  onValueChange={setMinSeverity}
                >
                  <SelectTrigger className="w-32">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="low">Low</SelectItem>
                    <SelectItem value="medium">Medium</SelectItem>
                    <SelectItem value="high">High</SelectItem>
                    <SelectItem value="critical">Critical</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="flex items-center justify-between">
                <div>
                  <Label>Update Interval</Label>
                  <p className="text-xs text-muted-foreground">
                    Auto-update sources every N hours (0 = manual only)
                  </p>
                </div>
                <Select
                  value={String(config.update_interval_hours)}
                  onValueChange={setUpdateInterval}
                >
                  <SelectTrigger className="w-32">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="0">Manual only</SelectItem>
                    <SelectItem value="6">6 hours</SelectItem>
                    <SelectItem value="12">12 hours</SelectItem>
                    <SelectItem value="24">24 hours</SelectItem>
                    <SelectItem value="72">3 days</SelectItem>
                    <SelectItem value="168">Weekly</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </CardContent>
          </Card>

          {/* Sources */}
          <Card>
            <CardHeader>
              <div className="flex items-center justify-between">
                <div>
                  <CardTitle className="text-base">Rule Sources</CardTitle>
                  <CardDescription>
                    Enable or disable rule sources. Predefined sources are downloaded from GitHub.
                  </CardDescription>
                </div>
                <div className="flex gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleUpdateAll}
                    disabled={updatingAll}
                  >
                    {updatingAll ? (
                      <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
                    ) : (
                      <RefreshCw className="h-3.5 w-3.5 mr-1.5" />
                    )}
                    Update All
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={openAddSourceDialog}
                  >
                    <Plus className="h-3.5 w-3.5 mr-1.5" />
                    Add Source
                  </Button>
                </div>
              </div>
            </CardHeader>
            <CardContent>
              <div className="space-y-1">
                {config.sources.map((source) => {
                  const status = getStatusForSource(source.id)
                  const isUpdating = updatingSource === source.id
                  const isExpanded = expandedSource === source.id
                  const isModel = source.source_type === "model"
                  const modelInfo = isModel ? modelStatuses[source.id] : null
                  const downloadProgress = downloadingModels[source.id]
                  const isDownloading = downloadProgress !== undefined
                  return (
                    <div key={source.id} className="border-b last:border-0">
                      <div className="flex items-center justify-between py-2">
                        <div
                          className="flex-1 min-w-0 cursor-pointer"
                          onClick={() => toggleSourceDetails(source.id)}
                        >
                          <div className="flex items-center gap-2">
                            {isExpanded ? (
                              <ChevronDown className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                            ) : (
                              <ChevronRight className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                            )}
                            <span className="font-medium text-sm">{source.label}</span>
                            <Badge variant="outline" className="text-[10px]">
                              {source.source_type}
                            </Badge>
                            {!source.predefined && (
                              <Badge variant="secondary" className="text-[10px]">
                                Custom
                              </Badge>
                            )}
                            {isModel ? (
                              <>
                                {modelInfo?.loaded && (
                                  <Badge className="text-[10px] bg-emerald-500 text-white">
                                    <Brain className="h-2.5 w-2.5 mr-1" />
                                    Loaded
                                  </Badge>
                                )}
                                {modelInfo && !modelInfo.loaded && modelInfo.download_state === "ready" && (
                                  <Badge variant="secondary" className="text-[10px]">
                                    Downloaded
                                  </Badge>
                                )}
                                {modelInfo?.error_message && (
                                  <Badge variant="destructive" className="text-[10px]">
                                    Error
                                  </Badge>
                                )}
                              </>
                            ) : (
                              renderSourceStatusBadge(status)
                            )}
                          </div>
                          <div className="flex items-center gap-2 mt-0.5 ml-5">
                            {source.url && (
                              <p className="text-xs text-muted-foreground truncate">
                                {source.url}
                              </p>
                            )}
                            {!isModel && status?.last_updated && (
                              <p className="text-xs text-muted-foreground whitespace-nowrap">
                                Refreshed {formatRelativeTime(status.last_updated)}
                              </p>
                            )}
                            {isModel && modelInfo && (
                              <p className="text-xs text-muted-foreground whitespace-nowrap">
                                {formatBytes(modelInfo.size_bytes)}
                                {modelInfo.architecture && ` \u00b7 ${modelInfo.architecture.toUpperCase()}`}
                              </p>
                            )}
                          </div>
                        </div>
                        <div className="flex items-center gap-2 ml-3">
                          {isModel ? (
                            <>
                              {(!modelInfo || modelInfo.download_state === "not_downloaded" || modelInfo.download_state === "error") && !isDownloading && (
                                <Button
                                  variant="ghost"
                                  size="sm"
                                  className="h-7 px-2"
                                  onClick={() => handleDownloadModel(source.id)}
                                  title="Download model (~346 MB)"
                                >
                                  <Download className="h-3.5 w-3.5" />
                                </Button>
                              )}
                              {isDownloading && (
                                <div className="flex items-center gap-1.5">
                                  <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                                  <span className="text-[10px] text-muted-foreground font-mono">
                                    {downloadProgress.bytes_downloaded > 0
                                      ? `${formatBytes(downloadProgress.bytes_downloaded)}${downloadProgress.bytes_per_second > 0 ? ` · ${formatBytes(downloadProgress.bytes_per_second)}/s` : ""}`
                                      : `${Math.round(downloadProgress.progress * 100)}%`}
                                  </span>
                                </div>
                              )}
                              {modelInfo?.loaded && (
                                <Button
                                  variant="ghost"
                                  size="sm"
                                  className="h-7 px-2"
                                  onClick={() => handleUnloadModel(source.id)}
                                  title="Unload model from memory"
                                >
                                  <Unplug className="h-3.5 w-3.5" />
                                </Button>
                              )}
                            </>
                          ) : (
                            <>
                              {source.id !== "builtin" && (
                                <Button
                                  variant="ghost"
                                  size="sm"
                                  className="h-7 px-2"
                                  onClick={() => handleUpdateSource(source.id)}
                                  disabled={isUpdating || !source.enabled}
                                  title="Update this source"
                                >
                                  {isUpdating ? (
                                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                  ) : (
                                    <RefreshCw className="h-3.5 w-3.5" />
                                  )}
                                </Button>
                              )}
                            </>
                          )}
                          {!source.predefined && (
                            <>
                              <Button
                                variant="ghost"
                                size="sm"
                                className="h-7 px-2"
                                onClick={() => openEditSourceDialog(source)}
                                title="Edit this source"
                              >
                                <Pencil className="h-3.5 w-3.5" />
                              </Button>
                              <Button
                                variant="ghost"
                                size="sm"
                                className="h-7 px-2 text-destructive hover:text-destructive"
                                onClick={() => handleRemoveSource(source.id)}
                                title="Remove this source"
                              >
                                <Trash2 className="h-3.5 w-3.5" />
                              </Button>
                            </>
                          )}
                          <Switch
                            checked={source.enabled}
                            onCheckedChange={(checked) => toggleSource(source.id, checked)}
                          />
                        </div>
                      </div>

                      {/* Download progress bar for model sources */}
                      {isModel && isDownloading && (
                        <div className="ml-5 mb-2">
                          <div className="h-1.5 bg-muted rounded-full overflow-hidden">
                            <div
                              className="h-full bg-blue-500 rounded-full transition-all duration-300"
                              style={{ width: `${Math.round(downloadProgress.progress * 100)}%` }}
                            />
                          </div>
                        </div>
                      )}

                      {/* Model confidence threshold (shown in expanded panel) */}
                      {isModel && isExpanded && !loadingDetails && (
                        <div className="ml-5 mb-2 space-y-2">
                          <div className="flex items-center gap-3 text-xs">
                            <Label className="text-xs text-muted-foreground whitespace-nowrap">Confidence threshold:</Label>
                            <input
                              type="range"
                              min="0.1"
                              max="0.99"
                              step="0.05"
                              value={source.confidence_threshold}
                              onChange={(e) => handleConfidenceThresholdChange(source.id, parseFloat(e.target.value))}
                              className="w-24 h-1.5 accent-blue-500"
                            />
                            <span className="font-mono text-muted-foreground w-8">{source.confidence_threshold.toFixed(2)}</span>
                            {modelInfo?.error_message && (
                              <span className="text-red-500 text-[10px] truncate">
                                <AlertTriangle className="h-3 w-3 inline mr-0.5" />
                                {modelInfo.error_message}
                              </span>
                            )}
                          </div>
                          {source.requires_auth && (!modelInfo || modelInfo.download_state === "not_downloaded" || modelInfo.download_state === "error") && (
                            <div className="space-y-1.5">
                              <div className="flex items-center gap-2">
                                <Input
                                  type="password"
                                  placeholder="hf_..."
                                  value={hfTokens[source.id] || ""}
                                  onChange={(e) => setHfTokens(prev => ({ ...prev, [source.id]: e.target.value }))}
                                  className="h-7 text-xs font-mono max-w-xs"
                                />
                              </div>
                              <p className="text-[10px] text-muted-foreground">
                                This model requires a HuggingFace token.{" "}
                                <a
                                  href="https://huggingface.co/settings/tokens"
                                  target="_blank"
                                  rel="noopener noreferrer"
                                  className="text-blue-500 hover:underline"
                                >
                                  Get your token
                                </a>
                                {" "}after accepting the model license.
                              </p>
                            </div>
                          )}
                          {!source.requires_auth && (!modelInfo || modelInfo.download_state === "not_downloaded") && (
                            <p className="text-[10px] text-muted-foreground">
                              Open model — no authentication required.
                            </p>
                          )}
                        </div>
                      )}

                      {/* Issue 3: Collapsible detail panel */}
                      {isExpanded && (
                        <div className="ml-5 mb-3 pl-3 border-l-2 border-muted space-y-2 text-xs">
                          {loadingDetails ? (
                            <div className="flex items-center gap-2 py-2 text-muted-foreground">
                              <Loader2 className="h-3 w-3 animate-spin" />
                              Loading details...
                            </div>
                          ) : sourceDetails && sourceDetails.id === source.id ? (
                            <>
                              {/* Combined source URL: repo/tree/branch/path */}
                              {sourceDetails.url && (
                                <div className="flex items-center gap-1">
                                  <span className="text-muted-foreground">Source: </span>
                                  <a
                                    href={(() => {
                                      const base = sourceDetails.url.replace(/\/$/, "")
                                      const isGitHub = base.includes("github.com")
                                      if (!isGitHub) return base
                                      const pathSuffix = sourceDetails.data_paths.length > 0
                                        ? `/${sourceDetails.data_paths[0]}`
                                        : ""
                                      return `${base}/tree/${sourceDetails.branch}${pathSuffix}`
                                    })()}
                                    target="_blank"
                                    rel="noopener noreferrer"
                                    className="text-blue-500 hover:underline inline-flex items-center gap-0.5"
                                  >
                                    {(() => {
                                      const base = sourceDetails.url.replace(/\/$/, "").replace(/^https?:\/\//, "")
                                      const pathSuffix = sourceDetails.data_paths.length > 0
                                        ? `/tree/${sourceDetails.branch}/${sourceDetails.data_paths.join(", ")}`
                                        : `/tree/${sourceDetails.branch}`
                                      return `${base}${pathSuffix}`
                                    })()}
                                    <ExternalLink className="h-3 w-3 flex-shrink-0" />
                                  </a>
                                </div>
                              )}
                              <div>
                                <span className="text-muted-foreground">Type: </span>
                                <span>{sourceDetails.source_type}</span>
                              </div>
                              {/* Cache location with open folder button */}
                              {sourceDetails.cache_dir && (
                                <div className="flex items-center gap-1">
                                  <span className="text-muted-foreground">Cache: </span>
                                  <span className="font-mono text-[10px]">{sourceDetails.cache_dir}</span>
                                  <Button
                                    variant="ghost"
                                    size="sm"
                                    className="h-5 w-5 p-0"
                                    onClick={() => invoke("open_path", { path: sourceDetails.cache_dir } satisfies OpenPathParams as Record<string, unknown>)}
                                    title="Open cache folder"
                                  >
                                    <FolderOpen className="h-3 w-3" />
                                  </Button>
                                </div>
                              )}
                              {sourceDetails.raw_files.length > 0 && (
                                <div>
                                  <span className="text-muted-foreground">Cached files: </span>
                                  <span>{sourceDetails.raw_files.length}</span>
                                </div>
                              )}
                              <div>
                                <span className="text-muted-foreground">Compiled rules: </span>
                                <span className="font-mono">{sourceDetails.compiled_rules_count}</span>
                              </div>
                              {sourceDetails.error_message && (
                                <div className="text-red-500">
                                  <AlertTriangle className="h-3 w-3 inline mr-1" />
                                  {sourceDetails.error_message}
                                </div>
                              )}
                              {sourceDetails.path_errors.length > 0 && (
                                <div className="space-y-1">
                                  <span className="text-amber-500 font-medium">Path errors ({sourceDetails.path_errors.length}):</span>
                                  {sourceDetails.path_errors.map((pe, i) => (
                                    <div key={i} className="ml-2 text-muted-foreground">
                                      <span className="font-mono">{pe.path}</span>: {pe.detail}
                                    </div>
                                  ))}
                                </div>
                              )}
                              {/* All rules in scrollable container */}
                              {sourceDetails.sample_rules.length > 0 && (
                                <div className="space-y-1">
                                  <span className="text-muted-foreground">Rules ({sourceDetails.sample_rules.length}):</span>
                                  <div className="max-h-48 overflow-y-auto space-y-1 pr-1">
                                    {sourceDetails.sample_rules.map((rule, i) => (
                                      <div key={i} className="ml-2 bg-muted/50 rounded px-2 py-1">
                                        <span className="font-medium">{rule.name}</span>
                                        <code className="block font-mono text-[10px] text-muted-foreground truncate mt-0.5">
                                          {rule.pattern}
                                        </code>
                                      </div>
                                    ))}
                                  </div>
                                </div>
                              )}
                            </>
                          ) : (
                            <p className="text-muted-foreground py-1">No details available</p>
                          )}
                        </div>
                      )}
                    </div>
                  )
                })}
                {config.sources.length === 0 && (
                  <p className="text-sm text-muted-foreground text-center py-4">
                    No sources configured. Built-in rules are always active.
                  </p>
                )}
              </div>
            </CardContent>
          </Card>

          {/* Custom Rules */}
          <Card>
            <CardHeader>
              <div className="flex items-center justify-between">
                <div>
                  <CardTitle className="text-base">Custom Rules</CardTitle>
                  <CardDescription>
                    Add your own regex patterns to detect specific content.
                  </CardDescription>
                </div>
                <Button variant="outline" size="sm" onClick={openAddCustomRule}>
                  <Plus className="h-3.5 w-3.5 mr-1.5" />
                  Add Rule
                </Button>
              </div>
            </CardHeader>
            <CardContent>
              <div className="space-y-2">
                {(config.custom_rules || []).map((rule) => (
                  <div
                    key={rule.id}
                    className="flex items-center justify-between py-2 border-b last:border-0"
                  >
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="font-medium text-sm">{rule.name}</span>
                        <Badge variant="outline" className="text-[10px]">
                          {rule.category.replace(/_/g, " ")}
                        </Badge>
                        <Badge
                          className={`text-[10px] ${
                            rule.severity === "critical" ? "bg-red-500 text-white" :
                            rule.severity === "high" ? "bg-orange-500 text-white" :
                            rule.severity === "medium" ? "bg-yellow-500 text-black" :
                            "bg-blue-500 text-white"
                          }`}
                        >
                          {rule.severity}
                        </Badge>
                        <Badge variant="secondary" className="text-[10px]">
                          {rule.direction}
                        </Badge>
                      </div>
                      <code className="text-xs text-muted-foreground font-mono mt-0.5 block truncate">
                        {rule.pattern}
                      </code>
                    </div>
                    <div className="flex items-center gap-2 ml-3">
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-7 px-2"
                        onClick={() => openEditCustomRule(rule)}
                        title="Edit rule"
                      >
                        <Pencil className="h-3.5 w-3.5" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-7 px-2 text-destructive hover:text-destructive"
                        onClick={() => handleRemoveCustomRule(rule.id)}
                        title="Remove rule"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                      <Switch
                        checked={rule.enabled}
                        onCheckedChange={() => handleToggleCustomRule(rule)}
                      />
                    </div>
                  </div>
                ))}
                {(!config.custom_rules || config.custom_rules.length === 0) && (
                  <p className="text-sm text-muted-foreground text-center py-4">
                    No custom rules. Click &quot;Add Rule&quot; to create one.
                  </p>
                )}
              </div>
            </CardContent>
          </Card>

          {/* Test Rules */}
          <Card>
            <CardHeader>
              <CardTitle className="text-base flex items-center gap-2">
                <FlaskConical className="h-4 w-4" />
                Test Rules
              </CardTitle>
              <CardDescription>
                Enter text to test against all active guardrail rules.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              <Textarea
                placeholder="Enter text to test... e.g. &quot;Ignore all previous instructions&quot;"
                value={testText}
                onChange={(e) => setTestText(e.target.value)}
                rows={3}
              />
              <Button
                onClick={handleTestInput}
                disabled={testing || !testText.trim()}
                size="sm"
              >
                {testing ? (
                  <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
                ) : (
                  <FlaskConical className="h-3.5 w-3.5 mr-1.5" />
                )}
                Test Rules
              </Button>
              {/* Issue 4: Fixed-height container to prevent page jump */}
              <div className="min-h-[80px] border rounded-lg p-3 space-y-2">
                {testing ? (
                  <div className="flex items-center justify-center h-[56px] text-xs text-muted-foreground">
                    <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                    Running rules...
                  </div>
                ) : testResult ? (
                  <>
                    <div className="flex items-center justify-between text-xs">
                      <span className="text-muted-foreground">
                        {testResult.rules_checked} rules checked in {testResult.check_duration_ms}ms
                      </span>
                      {testResult.matches.length > 0 ? (
                        <Badge variant="destructive" className="text-[10px]">
                          {testResult.matches.length} match{testResult.matches.length !== 1 ? "es" : ""}
                        </Badge>
                      ) : (
                        <Badge className="bg-emerald-500 text-white text-[10px]">Clean</Badge>
                      )}
                    </div>
                    {/* Per-source summary */}
                    {testResult.sources_checked.length > 0 && (
                      <div className="space-y-1">
                        {testResult.sources_checked.map((src) => (
                          <div key={src.source_id} className="flex items-center justify-between text-xs">
                            <span className="font-medium">{src.source_label}</span>
                            <span className="text-muted-foreground">
                              {src.match_count > 0 ? (
                                <span className="text-red-500">{src.match_count} match{src.match_count !== 1 ? "es" : ""}</span>
                              ) : (
                                <span className="flex items-center gap-1 text-emerald-500">
                                  <CheckCircle2 className="h-3 w-3" /> clean
                                </span>
                              )}
                              {" / "}{src.rules_checked} rules
                            </span>
                          </div>
                        ))}
                      </div>
                    )}
                    {/* Individual matches */}
                    {testResult.matches.length > 0 && (
                      <div className="space-y-1.5 mt-2">
                        {testResult.matches.map((match, i) => (
                          <div key={i} className="bg-muted/50 rounded px-2 py-1.5 text-xs space-y-0.5">
                            <div className="flex items-center gap-1.5">
                              <span className={`px-1.5 py-0.5 rounded text-[10px] font-bold ${
                                match.severity === "critical" ? "bg-red-500 text-white" :
                                match.severity === "high" ? "bg-orange-500 text-white" :
                                match.severity === "medium" ? "bg-yellow-500 text-black" :
                                "bg-blue-500 text-white"
                              }`}>
                                {match.severity.toUpperCase()}
                              </span>
                              <span className="font-medium">{match.rule_name}</span>
                              <span className="text-muted-foreground ml-auto text-[10px]">{match.source_label}</span>
                            </div>
                            {match.matched_text && (
                              <code className="block font-mono text-[10px] bg-red-50 dark:bg-red-950/30 text-red-700 dark:text-red-300 px-1.5 py-0.5 rounded truncate">
                                {match.matched_text}
                              </code>
                            )}
                          </div>
                        ))}
                      </div>
                    )}
                  </>
                ) : (
                  <div className="flex items-center justify-center h-[56px] text-xs text-muted-foreground">
                    {testRan ? "No results" : "Run a test to see results"}
                  </div>
                )}
              </div>
            </CardContent>
          </Card>
        </>
      )}

      {/* Add/Edit Custom Source Dialog */}
      <Dialog open={showSourceDialog} onOpenChange={setShowSourceDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{editingSource ? "Edit Source" : "Add Custom Source"}</DialogTitle>
            <DialogDescription>
              {editingSource
                ? "Update the source configuration."
                : "Add a custom guardrail rule source from a GitHub repository."}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3 py-2">
            <div>
              <Label className="text-xs">Label</Label>
              <Input
                placeholder="My Custom Rules"
                value={sourceForm.label}
                onChange={(e) => {
                  const label = e.target.value
                  setSourceForm({
                    ...sourceForm,
                    label,
                    // Issue 1: Auto-derive ID for new sources
                    ...(!editingSource ? { id: deriveIdFromLabel(label) } : {}),
                  })
                }}
                className="mt-1"
              />
              {!editingSource && sourceForm.label && (
                <p className="text-[10px] text-muted-foreground mt-0.5">
                  ID: <span className="font-mono">{deriveIdFromLabel(sourceForm.label)}</span>
                </p>
              )}
            </div>
            <div>
              <Label className="text-xs">Type</Label>
              <Select
                value={sourceForm.sourceType}
                onValueChange={(v) => setSourceForm({ ...sourceForm, sourceType: v })}
              >
                <SelectTrigger className="mt-1">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="regex">Regex</SelectItem>
                  <SelectItem value="yara">YARA</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div>
              <Label className="text-xs">GitHub Repository URL</Label>
              <Input
                placeholder="https://github.com/owner/repo"
                value={sourceForm.url}
                onChange={(e) => setSourceForm({ ...sourceForm, url: e.target.value })}
                className="mt-1"
              />
            </div>
            <div>
              <Label className="text-xs">Data Paths (comma separated)</Label>
              <Input
                placeholder="rules/, patterns/injection"
                value={sourceForm.dataPaths}
                onChange={(e) => setSourceForm({ ...sourceForm, dataPaths: e.target.value })}
                className="mt-1"
              />
              <p className="text-[10px] text-muted-foreground mt-0.5">
                Paths can be files or directories. Directories are auto-expanded via GitHub API.
              </p>
            </div>
            <div>
              <Label className="text-xs">Branch</Label>
              <Input
                placeholder="main"
                value={sourceForm.branch}
                onChange={(e) => setSourceForm({ ...sourceForm, branch: e.target.value })}
                className="mt-1"
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowSourceDialog(false)}>
              Cancel
            </Button>
            <Button
              onClick={handleSaveSource}
              disabled={savingSource || !sourceForm.label || !sourceForm.url}
            >
              {savingSource && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}
              {editingSource ? "Update Source" : "Add Source"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Add/Edit Custom Rule Dialog */}
      <Dialog open={showCustomRuleDialog} onOpenChange={setShowCustomRuleDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{editingRule ? "Edit Custom Rule" : "Add Custom Rule"}</DialogTitle>
            <DialogDescription>
              Define a regex pattern to detect specific content in LLM traffic.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3 py-2">
            <div>
              <Label className="text-xs">Name</Label>
              <Input
                placeholder="Detect internal IDs"
                value={customRuleForm.name}
                onChange={(e) => setCustomRuleForm({ ...customRuleForm, name: e.target.value })}
                className="mt-1"
              />
            </div>
            <div>
              <Label className="text-xs">Pattern (regex)</Label>
              <div className="relative mt-1">
                <Input
                  placeholder="INTERNAL-\d+"
                  value={customRuleForm.pattern}
                  onChange={(e) => {
                    setCustomRuleForm({ ...customRuleForm, pattern: e.target.value })
                    validatePattern(e.target.value)
                  }}
                  className={`font-mono pr-8 ${patternValid === false ? "border-red-500" : patternValid === true ? "border-emerald-500" : ""}`}
                />
                {patternValid === true && (
                  <CheckCircle2 className="absolute right-2 top-1/2 -translate-y-1/2 h-4 w-4 text-emerald-500" />
                )}
                {patternValid === false && (
                  <XCircle className="absolute right-2 top-1/2 -translate-y-1/2 h-4 w-4 text-red-500" />
                )}
              </div>
              {patternValid === false && (
                <p className="text-[10px] text-red-500 mt-0.5">Invalid regex pattern</p>
              )}
            </div>
            <div className="grid grid-cols-3 gap-2">
              <div>
                <Label className="text-xs">Category</Label>
                <Select
                  value={customRuleForm.category}
                  onValueChange={(v) => setCustomRuleForm({ ...customRuleForm, category: v })}
                >
                  <SelectTrigger className="mt-1">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="prompt_injection">Prompt Injection</SelectItem>
                    <SelectItem value="jailbreak">Jailbreak</SelectItem>
                    <SelectItem value="pii_leakage">PII Leakage</SelectItem>
                    <SelectItem value="code_injection">Code Injection</SelectItem>
                    <SelectItem value="data_exfiltration">Data Exfiltration</SelectItem>
                    <SelectItem value="other">Other</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div>
                <Label className="text-xs">Severity</Label>
                <Select
                  value={customRuleForm.severity}
                  onValueChange={(v) => setCustomRuleForm({ ...customRuleForm, severity: v })}
                >
                  <SelectTrigger className="mt-1">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="low">Low</SelectItem>
                    <SelectItem value="medium">Medium</SelectItem>
                    <SelectItem value="high">High</SelectItem>
                    <SelectItem value="critical">Critical</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div>
                <Label className="text-xs">Direction</Label>
                <Select
                  value={customRuleForm.direction}
                  onValueChange={(v) => setCustomRuleForm({ ...customRuleForm, direction: v })}
                >
                  <SelectTrigger className="mt-1">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="input">Input</SelectItem>
                    <SelectItem value="output">Output</SelectItem>
                    <SelectItem value="both">Both</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowCustomRuleDialog(false)}>
              Cancel
            </Button>
            <Button
              onClick={handleSaveCustomRule}
              disabled={savingRule || !customRuleForm.name || !customRuleForm.pattern || patternValid === false}
            >
              {savingRule && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}
              {editingRule ? "Update Rule" : "Add Rule"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
