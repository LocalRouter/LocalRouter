import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Plus, Trash2, Wrench, Cloud } from "lucide-react"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { Switch } from "@/components/ui/Toggle"
import type {
  SafetyModelConfig,
  ProviderInstanceInfo,
  CategoryMappingEntry,
  AddSafetyModelParams,
} from "@/types/tauri-commands"

const MODEL_TYPES = [
  { value: "llama_guard", label: "Llama Guard" },
  { value: "shield_gemma", label: "ShieldGemma" },
  { value: "nemotron", label: "Nemotron" },
  { value: "granite_guardian", label: "Granite Guardian" },
  { value: "custom", label: "Custom" },
]

type ExecutionMode = "custom_download" | "provider"

interface AddSafetyModelDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onModelAdded: () => void
}

export function AddSafetyModelDialog({
  open,
  onOpenChange,
  onModelAdded,
}: AddSafetyModelDialogProps) {
  const [label, setLabel] = useState("")
  const [modelType, setModelType] = useState("custom")
  const [executionMode, setExecutionMode] = useState<ExecutionMode>("custom_download")

  // Provider mode fields
  const [providerId, setProviderId] = useState("")
  const [modelName, setModelName] = useState("")
  const [providers, setProviders] = useState<ProviderInstanceInfo[]>([])

  // Custom download (HuggingFace) mode fields
  const [hfRepoId, setHfRepoId] = useState("")
  const [ggufFilename, setGgufFilename] = useState("")
  const [requiresAuth, setRequiresAuth] = useState(false)

  // Custom output mapping
  const [promptTemplate, setPromptTemplate] = useState("")
  const [safeIndicator, setSafeIndicator] = useState("safe")
  const [outputRegex, setOutputRegex] = useState("")
  const [categoryMapping, setCategoryMapping] = useState<CategoryMappingEntry[]>([])

  const [saving, setSaving] = useState(false)

  useEffect(() => {
    if (open) {
      invoke<ProviderInstanceInfo[]>("list_provider_instances")
        .then(setProviders)
        .catch(() => {})
    }
  }, [open])

  const addMappingRow = () => {
    setCategoryMapping(prev => [...prev, { native_label: "", safety_category: "" }])
  }

  const removeMappingRow = (index: number) => {
    setCategoryMapping(prev => prev.filter((_, i) => i !== index))
  }

  const updateMappingRow = (index: number, field: keyof CategoryMappingEntry, value: string) => {
    setCategoryMapping(prev =>
      prev.map((row, i) => (i === index ? { ...row, [field]: value } : row))
    )
  }

  const resetForm = () => {
    setLabel("")
    setModelType("custom")
    setExecutionMode("custom_download")
    setProviderId("")
    setModelName("")
    setHfRepoId("")
    setGgufFilename("")
    setRequiresAuth(false)
    setPromptTemplate("")
    setSafeIndicator("safe")
    setOutputRegex("")
    setCategoryMapping([])
  }

  const handleSave = async () => {
    if (!label.trim()) {
      toast.error("Model label is required")
      return
    }

    let resolvedHfRepoId: string | null = null
    let resolvedGgufFilename: string | null = null

    if (executionMode === "custom_download") {
      resolvedHfRepoId = hfRepoId || null
      resolvedGgufFilename = ggufFilename || null
    }

    setSaving(true)
    try {
      const config: SafetyModelConfig = {
        id: "",
        label: label.trim(),
        model_type: modelType,
        provider_id: executionMode === "provider" ? (providerId || null) : null,
        model_name: executionMode === "provider" ? (modelName || null) : null,
        hf_repo_id: resolvedHfRepoId,
        gguf_filename: resolvedGgufFilename,
        requires_auth: requiresAuth,
        confidence_threshold: null,
        enabled_categories: null,
        predefined: false,
        execution_mode: executionMode,
        prompt_template: modelType === "custom" && promptTemplate ? promptTemplate : null,
        safe_indicator: modelType === "custom" && safeIndicator ? safeIndicator : null,
        output_regex: modelType === "custom" && outputRegex ? outputRegex : null,
        category_mapping: modelType === "custom" && categoryMapping.length > 0 ? categoryMapping : null,
        memory_mb: null,
        latency_ms: null,
        disk_size_mb: null,
      }

      await invoke("add_safety_model", {
        configJson: JSON.stringify(config),
      } satisfies AddSafetyModelParams as Record<string, unknown>)

      toast.success("Safety model added")
      resetForm()
      onOpenChange(false)
      onModelAdded()
    } catch (err) {
      toast.error(`Failed to add model: ${err}`)
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Add Custom Safety Model</DialogTitle>
          <DialogDescription>
            Add a custom safety model via HuggingFace download or external provider.
            For pre-configured models, use the dropdown picker instead.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          {/* Label */}
          <div>
            <Label className="text-xs">Display Name</Label>
            <Input
              placeholder="e.g. My Custom Guard"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              className="mt-1 h-8 text-xs"
            />
          </div>

          {/* Model Type */}
          <div>
            <Label className="text-xs">Model Type</Label>
            <select
              value={modelType}
              onChange={(e) => setModelType(e.target.value)}
              className="mt-1 w-full h-8 text-xs rounded-md border border-input bg-background px-3"
            >
              {MODEL_TYPES.map((t) => (
                <option key={t.value} value={t.value}>
                  {t.label}
                </option>
              ))}
            </select>
          </div>

          {/* Execution Mode */}
          <div>
            <Label className="text-xs">Execution Mode</Label>
            <div className="mt-1 flex gap-4">
              <label className="flex items-center gap-1.5 text-xs cursor-pointer">
                <input
                  type="radio"
                  name="executionMode"
                  value="custom_download"
                  checked={executionMode === "custom_download"}
                  onChange={() => setExecutionMode("custom_download")}
                  className="accent-blue-500"
                />
                <Wrench className="h-3 w-3" /> Custom Download
              </label>
              <label className="flex items-center gap-1.5 text-xs cursor-pointer">
                <input
                  type="radio"
                  name="executionMode"
                  value="provider"
                  checked={executionMode === "provider"}
                  onChange={() => setExecutionMode("provider")}
                  className="accent-blue-500"
                />
                <Cloud className="h-3 w-3" /> Provider
              </label>
            </div>
          </div>

          {/* Custom Download: manual HF fields */}
          {executionMode === "custom_download" && (
            <div className="space-y-3 border rounded-lg p-3">
              <div>
                <Label className="text-xs">HuggingFace Repo ID</Label>
                <Input
                  placeholder="e.g. QuantFactory/shieldgemma-2b-GGUF"
                  value={hfRepoId}
                  onChange={(e) => setHfRepoId(e.target.value)}
                  className="mt-1 h-8 text-xs font-mono"
                />
              </div>
              <div>
                <Label className="text-xs">GGUF Filename</Label>
                <Input
                  placeholder="e.g. shieldgemma-2b.Q4_K_M.gguf"
                  value={ggufFilename}
                  onChange={(e) => setGgufFilename(e.target.value)}
                  className="mt-1 h-8 text-xs font-mono"
                />
              </div>
              <div className="flex items-center gap-2">
                <Switch
                  checked={requiresAuth}
                  onCheckedChange={setRequiresAuth}
                />
                <Label className="text-xs">Requires HuggingFace Auth (gated model)</Label>
              </div>
            </div>
          )}

          {/* Provider mode fields */}
          {executionMode === "provider" && (
            <div className="space-y-3 border rounded-lg p-3">
              <div>
                <Label className="text-xs">Provider</Label>
                <select
                  value={providerId}
                  onChange={(e) => setProviderId(e.target.value)}
                  className="mt-1 w-full h-8 text-xs rounded-md border border-input bg-background px-3"
                >
                  <option value="">Select a provider...</option>
                  {providers
                    .filter((p) => p.enabled)
                    .map((p) => (
                      <option key={p.instance_name} value={p.instance_name}>
                        {p.instance_name} ({p.provider_type})
                      </option>
                    ))}
                </select>
              </div>
              <div>
                <Label className="text-xs">Model Name</Label>
                <Input
                  placeholder="e.g. llama-guard3:1b"
                  value={modelName}
                  onChange={(e) => setModelName(e.target.value)}
                  className="mt-1 h-8 text-xs"
                />
              </div>
            </div>
          )}

          {/* Custom output mapping (only for custom model_type) */}
          {modelType === "custom" && (
            <div className="space-y-3 border rounded-lg p-3">
              <p className="text-xs font-medium text-muted-foreground">Custom Output Mapping</p>
              <div>
                <Label className="text-xs">Prompt Template</Label>
                <Textarea
                  placeholder="Classify the following content for safety:\n\n{content}\n\nRespond with: safe or unsafe"
                  value={promptTemplate}
                  onChange={(e) => setPromptTemplate(e.target.value)}
                  rows={3}
                  className="mt-1 text-xs font-mono"
                />
                <p className="text-[10px] text-muted-foreground mt-0.5">
                  Use {"\\{content\\}"} as placeholder for the text to check
                </p>
              </div>
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <Label className="text-xs">Safe Indicator</Label>
                  <Input
                    placeholder="e.g. safe"
                    value={safeIndicator}
                    onChange={(e) => setSafeIndicator(e.target.value)}
                    className="mt-1 h-8 text-xs font-mono"
                  />
                </div>
                <div>
                  <Label className="text-xs">Output Regex</Label>
                  <Input
                    placeholder="e.g. category:\s*(\w+)"
                    value={outputRegex}
                    onChange={(e) => setOutputRegex(e.target.value)}
                    className="mt-1 h-8 text-xs font-mono"
                  />
                </div>
              </div>

              {/* Category mapping table */}
              <div>
                <div className="flex items-center justify-between mb-1">
                  <Label className="text-xs">Category Mapping</Label>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-6 px-2 text-[10px]"
                    onClick={addMappingRow}
                  >
                    <Plus className="h-3 w-3 mr-1" />
                    Add
                  </Button>
                </div>
                {categoryMapping.length > 0 ? (
                  <div className="space-y-1.5">
                    {categoryMapping.map((row, i) => (
                      <div key={i} className="flex items-center gap-2">
                        <Input
                          placeholder="Native label"
                          value={row.native_label}
                          onChange={(e) => updateMappingRow(i, "native_label", e.target.value)}
                          className="h-7 text-[11px] font-mono flex-1"
                        />
                        <span className="text-xs text-muted-foreground">→</span>
                        <Input
                          placeholder="Safety category"
                          value={row.safety_category}
                          onChange={(e) => updateMappingRow(i, "safety_category", e.target.value)}
                          className="h-7 text-[11px] font-mono flex-1"
                        />
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-7 w-7 p-0 text-muted-foreground hover:text-destructive"
                          onClick={() => removeMappingRow(i)}
                        >
                          <Trash2 className="h-3 w-3" />
                        </Button>
                      </div>
                    ))}
                  </div>
                ) : (
                  <p className="text-[10px] text-muted-foreground">
                    No mappings configured. Add rows to map model outputs to safety categories.
                  </p>
                )}
              </div>
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" size="sm" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button size="sm" onClick={handleSave} disabled={saving || !label.trim()}>
            {saving ? "Adding..." : "Add Model"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
