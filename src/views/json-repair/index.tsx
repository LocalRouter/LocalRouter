import { useState, useEffect, useCallback, useRef, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Loader2 } from "lucide-react"
import { FEATURES } from "@/constants/features"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/Toggle"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { FeatureClientsCard } from "@/components/shared/FeatureClientsCard"
import type { JsonRepairConfig, JsonRepairTestResult } from "@/types/tauri-commands"

const REPAIR_LABELS: Record<string, string> = {
  stripped_markdown_fences: "Stripped markdown fences",
  stripped_prose: "Stripped surrounding prose",
  syntax_repaired: "Fixed syntax error",
}

function formatRepairString(action: string): string {
  return REPAIR_LABELS[action] ?? action.replace(/_/g, " ")
}

interface JsonRepairViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function JsonRepairView({ activeSubTab, onTabChange }: JsonRepairViewProps) {
  const [config, setConfig] = useState<JsonRepairConfig | null>(null)
  const [saving, setSaving] = useState(false)
  const [testInput, setTestInput] = useState(`\`\`\`json
{
  name: Alice
  'age': '28'
  "score": "  95.5 ",
  "role": "admin", "active": "yes",
  "tags": ["developer" "lead"],
  "extra_field": True,
  "status": "pending"
}
\`\`\``)
  const [testSchema, setTestSchema] = useState(JSON.stringify({
    type: "object",
    properties: {
      name: { type: "string" },
      age: { type: "integer" },
      score: { type: "number" },
      role: { type: "string", enum: ["Admin", "User", "Guest"] },
      active: { type: "boolean" },
      tags: { type: "array", items: { type: "string" } },
      status: { type: "string", enum: ["Pending", "Active", "Inactive"] },
      joined: { type: "string", default: "2026-01-01" }
    },
    required: ["name", "age", "score", "role", "active", "tags", "status", "joined"],
    additionalProperties: false
  }, null, 2))
  const [testResult, setTestResult] = useState<JsonRepairTestResult | null>(null)
  const [testLoading, setTestLoading] = useState(false)

  const tab = activeSubTab || "info"

  const handleTabChange = (newTab: string) => {
    onTabChange?.("json-repair", newTab)
  }

  const loadConfig = useCallback(async () => {
    try {
      const data = await invoke<JsonRepairConfig>("get_json_repair_config")
      setConfig(data)
    } catch (err) {
      console.error("Failed to load JSON repair config:", err)
    }
  }, [])

  useEffect(() => {
    loadConfig()
  }, [loadConfig])

  const updateConfig = async (updates: Partial<JsonRepairConfig>) => {
    if (!config) return
    setSaving(true)
    const newConfig = { ...config, ...updates }
    try {
      await invoke("update_json_repair_config", { configJson: JSON.stringify(newConfig) })
      setConfig(newConfig)
    } catch (err) {
      toast.error(`Failed to update config: ${err}`)
    } finally {
      setSaving(false)
    }
  }

  const runTest = useCallback(async (input: string, schema: string | null) => {
    if (!input.trim()) {
      setTestResult(null)
      return
    }
    setTestLoading(true)
    try {
      const result = await invoke<JsonRepairTestResult>("test_json_repair", {
        content: input,
        schema,
      })
      setTestResult(result)
    } catch {
      // Silently ignore during typing
    } finally {
      setTestLoading(false)
    }
  }, [])

  // Debounced auto-repair on input, schema, or toggle changes
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  useEffect(() => {
    if (tab !== "try-it-out") return
    if (debounceRef.current) clearTimeout(debounceRef.current)
    debounceRef.current = setTimeout(() => {
      runTest(testInput, testSchema)
    }, 300)
    return () => { if (debounceRef.current) clearTimeout(debounceRef.current) }
  }, [testInput, testSchema, tab, runTest])

  // Deduplicate and format repair actions for display
  const formattedRepairs = useMemo(() => {
    if (!testResult) return []
    const labels: string[] = []
    const counts = new Map<string, number>()

    for (const r of testResult.repairs) {
      let label: string
      if (typeof r === "string") {
        label = formatRepairString(r)
      } else if ("type_coerced" in r) {
        label = `${r.type_coerced.path}: ${r.type_coerced.from} → ${r.type_coerced.to}`
      } else if ("extra_field_removed" in r) {
        label = `Removed ${r.extra_field_removed.path}`
      } else if ("default_added" in r) {
        label = `Added default for ${r.default_added.path}`
      } else if ("enum_normalized" in r) {
        label = `${r.enum_normalized.path}: ${r.enum_normalized.from} → ${r.enum_normalized.to}`
      } else {
        label = JSON.stringify(r)
      }
      const prev = counts.get(label) ?? 0
      if (prev === 0) labels.push(label)
      counts.set(label, prev + 1)
    }

    return labels.map(l => {
      const c = counts.get(l)!
      return c > 1 ? `${l} (×${c})` : l
    })
  }, [testResult])

  return (
    <div className="flex flex-col h-full min-h-0 gap-4 max-w-5xl">
      <div className="flex-shrink-0">
        <div className="flex items-center gap-3 mb-1">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <FEATURES.jsonRepair.icon className={`h-6 w-6 ${FEATURES.jsonRepair.color}`} />
            JSON Repair
          </h1>
        </div>
        <p className="text-sm text-muted-foreground">
          Automatically fix malformed JSON responses from LLMs before they reach your application
        </p>
      </div>

      <Tabs
        value={tab}
        onValueChange={handleTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="info"><TAB_ICONS.info className={TAB_ICON_CLASS} />Info</TabsTrigger>
          <TabsTrigger value="try-it-out"><TAB_ICONS.tryItOut className={TAB_ICON_CLASS} />Try It Out</TabsTrigger>
          <TabsTrigger value="settings"><TAB_ICONS.settings className={TAB_ICON_CLASS} />Settings</TabsTrigger>
        </TabsList>

        {/* Info Tab */}
        <TabsContent value="info" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-2xl overflow-y-auto">
            {config && (
              <Card>
                <CardHeader className="pb-3">
                  <div className="flex items-center justify-between">
                    <div>
                      <CardTitle className="text-base">Default: JSON Repair</CardTitle>
                      <CardDescription>
                        Automatically repair JSON responses for requests with <code className="text-xs bg-muted px-1 py-0.5 rounded">response_format: json_object</code> or <code className="text-xs bg-muted px-1 py-0.5 rounded">json_schema</code>.
                        Works inline during streaming with near-zero latency. Individual clients can override this in their settings.
                      </CardDescription>
                    </div>
                    <Switch
                      checked={config.enabled}
                      onCheckedChange={(enabled) => updateConfig({ enabled })}
                      disabled={saving}
                    />
                  </div>
                </CardHeader>
              </Card>
            )}

            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-base">Examples</CardTitle>
                <CardDescription>
                  Syntax repairs fix malformed JSON, schema coercion matches values to your <code className="text-xs bg-muted px-1 py-0.5 rounded">json_schema</code>.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <div className="space-y-3 text-sm max-h-[340px] overflow-y-auto pr-1">
                  <p className="text-xs font-medium text-muted-foreground uppercase tracking-wide">Syntax Repair</p>
                  <div className="grid grid-cols-2 gap-2">
                    <div className="bg-muted rounded-md p-2">
                      <p className="text-xs text-muted-foreground mb-1">Trailing comma</p>
                      <code className="text-xs">{'{"a": 1, "b": 2,}'}</code>
                    </div>
                    <div className="bg-muted rounded-md p-2">
                      <p className="text-xs text-muted-foreground mb-1">Missing comma</p>
                      <code className="text-xs">{'{"a": 1 "b": 2}'}</code>
                    </div>
                    <div className="bg-muted rounded-md p-2">
                      <p className="text-xs text-muted-foreground mb-1">Missing bracket</p>
                      <code className="text-xs">{'{"name": "Alice"'}</code>
                    </div>
                    <div className="bg-muted rounded-md p-2">
                      <p className="text-xs text-muted-foreground mb-1">Single quotes</p>
                      <code className="text-xs">{"{'name': 'Alice'}"}</code>
                    </div>
                    <div className="bg-muted rounded-md p-2">
                      <p className="text-xs text-muted-foreground mb-1">Unquoted keys</p>
                      <code className="text-xs">{'{name: "Alice"}'}</code>
                    </div>
                    <div className="bg-muted rounded-md p-2">
                      <p className="text-xs text-muted-foreground mb-1">Python keywords</p>
                      <code className="text-xs">{'{"a": True, "b": None}'}</code>
                    </div>
                    <div className="bg-muted rounded-md p-2 col-span-2">
                      <p className="text-xs text-muted-foreground mb-1">Markdown fences &amp; prose</p>
                      <code className="text-xs">{'Here is the data: ```json {"name": "Alice"} ```'}</code>
                    </div>
                  </div>
                  <p className="text-xs font-medium text-muted-foreground uppercase tracking-wide pt-2">Schema Coercion</p>
                  <div className="grid grid-cols-2 gap-2">
                    <div className="bg-muted rounded-md p-2">
                      <p className="text-xs text-muted-foreground mb-1">String to integer</p>
                      <code className="text-xs">{'"42"'} &rarr; <span className="text-green-600 dark:text-green-400">42</span></code>
                    </div>
                    <div className="bg-muted rounded-md p-2">
                      <p className="text-xs text-muted-foreground mb-1">String to boolean</p>
                      <code className="text-xs">{'"true"'} &rarr; <span className="text-green-600 dark:text-green-400">true</span></code>
                    </div>
                    <div className="bg-muted rounded-md p-2">
                      <p className="text-xs text-muted-foreground mb-1">Enum normalization</p>
                      <code className="text-xs">{'"active"'} &rarr; <span className="text-green-600 dark:text-green-400">{'"Active"'}</span></code>
                    </div>
                    <div className="bg-muted rounded-md p-2">
                      <p className="text-xs text-muted-foreground mb-1">Extra field removal</p>
                      <code className="text-xs"><span className="text-red-500 line-through">{'"extra": true'}</span></code>
                    </div>
                    <div className="bg-muted rounded-md p-2 col-span-2">
                      <p className="text-xs text-muted-foreground mb-1">Missing required defaults</p>
                      <code className="text-xs">{'{ }'} &rarr; <span className="text-green-600 dark:text-green-400">{'{"status": "active"}'}</span> <span className="text-muted-foreground">(if default defined)</span></code>
                    </div>
                  </div>
                </div>
              </CardContent>
            </Card>

            <FeatureClientsCard feature="json_repair" clientTab="optimize" onNavigateToClient={onTabChange} />
          </div>
        </TabsContent>

        {/* Try it out Tab */}
        <TabsContent value="try-it-out" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 overflow-y-auto">
            <Card>
              <CardHeader className="pb-3">
                <div className="flex items-center justify-between">
                  <div>
                    <CardTitle className="text-base">Test JSON Repair</CardTitle>
                    <CardDescription>
                      Paste malformed JSON and provide a schema to test repair
                    </CardDescription>
                  </div>
                  {testLoading && <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />}
                </div>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="grid grid-cols-2 gap-4">
                  <div>
                    <label className="text-sm font-medium mb-1.5 block">Input (malformed JSON)</label>
                    <textarea
                      value={testInput}
                      onChange={(e) => setTestInput(e.target.value)}
                      className="w-full h-56 px-3 py-2 text-sm bg-muted rounded-md border font-mono resize-none focus:outline-none focus:ring-2 focus:ring-ring"
                      placeholder='{"name": "John", "age": "30",}'
                    />
                  </div>
                  <div>
                    <label className="text-sm font-medium mb-1.5 block">JSON Schema</label>
                    <textarea
                      value={testSchema}
                      onChange={(e) => setTestSchema(e.target.value)}
                      className="w-full h-56 px-3 py-2 text-sm bg-muted rounded-md border font-mono resize-none focus:outline-none focus:ring-2 focus:ring-ring"
                      placeholder='{"type": "object", "properties": {...}}'
                    />
                  </div>
                </div>

                {testResult && (
                  <div className="space-y-3">
                    <div>
                      <label className="text-sm font-medium mb-1.5 block">Output</label>
                      <pre className="w-full px-3 py-2 text-sm bg-muted rounded-md border font-mono whitespace-pre-wrap overflow-auto max-h-48">
                        {testResult.repaired}
                      </pre>
                    </div>

                    {formattedRepairs.length > 0 && (
                      <div>
                        <label className="text-sm font-medium mb-1.5 block">Repairs performed</label>
                        <ul className="text-xs text-muted-foreground space-y-1">
                          {formattedRepairs.map((label, i) => (
                            <li key={i} className="font-mono bg-muted px-2 py-1 rounded">
                              {label}
                            </li>
                          ))}
                        </ul>
                      </div>
                    )}
                  </div>
                )}
              </CardContent>
            </Card>
          </div>
        </TabsContent>

        {/* Settings Tab */}
        <TabsContent value="settings" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-2xl overflow-y-auto">
            {config && (
              <>
                <Card>
                  <CardHeader className="pb-3">
                    <CardTitle className="text-base">Syntax Repair</CardTitle>
                    <CardDescription>Fix JSON syntax errors in LLM responses</CardDescription>
                  </CardHeader>
                  <CardContent>
                    <div className="flex items-center justify-between">
                      <div>
                        <span className="text-sm font-medium">Fix malformed JSON</span>
                        <p className="text-xs text-muted-foreground mt-0.5">Trailing commas, missing brackets, unquoted keys, single quotes, markdown fences, missing commas, Python keywords</p>
                      </div>
                      <Switch
                        checked={config.syntax_repair}
                        onCheckedChange={(syntax_repair) => updateConfig({ syntax_repair })}
                        disabled={saving || !config.enabled}
                      />
                    </div>
                  </CardContent>
                </Card>

                <Card>
                  <CardHeader className="pb-3">
                    <CardTitle className="text-base">Schema Coercion</CardTitle>
                    <CardDescription>Fix JSON values to match the expected schema (requires <code className="text-xs bg-muted px-1 py-0.5 rounded">json_schema</code> response format)</CardDescription>
                  </CardHeader>
                  <CardContent className="space-y-4">
                    <div className="flex items-center justify-between">
                      <div>
                        <span className="text-sm font-medium">Type coercion</span>
                        <p className="text-xs text-muted-foreground mt-0.5">Convert values to match schema types: <code className="text-xs bg-muted px-0.5 rounded">"42"</code> &rarr; <code className="text-xs bg-muted px-0.5 rounded">42</code>, <code className="text-xs bg-muted px-0.5 rounded">"true"</code> &rarr; <code className="text-xs bg-muted px-0.5 rounded">true</code>, <code className="text-xs bg-muted px-0.5 rounded">42</code> &rarr; <code className="text-xs bg-muted px-0.5 rounded">"42"</code></p>
                      </div>
                      <Switch
                        checked={config.schema_coercion}
                        onCheckedChange={(schema_coercion) => updateConfig({ schema_coercion })}
                        disabled={saving || !config.enabled}
                      />
                    </div>
                    <div className="flex items-center justify-between">
                      <div>
                        <span className="text-sm font-medium">Normalize enum values</span>
                        <p className="text-xs text-muted-foreground mt-0.5">Case-insensitive matching against schema enum: <code className="text-xs bg-muted px-0.5 rounded">"active"</code> &rarr; <code className="text-xs bg-muted px-0.5 rounded">"Active"</code></p>
                      </div>
                      <Switch
                        checked={config.normalize_enums}
                        onCheckedChange={(normalize_enums) => updateConfig({ normalize_enums })}
                        disabled={saving || !config.enabled}
                      />
                    </div>
                    <div className="flex items-center justify-between">
                      <div>
                        <span className="text-sm font-medium">Add default values</span>
                        <p className="text-xs text-muted-foreground mt-0.5">Insert <code className="text-xs bg-muted px-0.5 rounded">default</code> values for missing required fields defined in schema</p>
                      </div>
                      <Switch
                        checked={config.add_defaults}
                        onCheckedChange={(add_defaults) => updateConfig({ add_defaults })}
                        disabled={saving || !config.enabled}
                      />
                    </div>
                    <div className="flex items-center justify-between">
                      <div>
                        <span className="text-sm font-medium">Strip extra fields</span>
                        <p className="text-xs text-muted-foreground mt-0.5">Remove fields not in schema when <code className="text-xs bg-muted px-0.5 rounded">additionalProperties: false</code></p>
                      </div>
                      <Switch
                        checked={config.strip_extra_fields}
                        onCheckedChange={(strip_extra_fields) => updateConfig({ strip_extra_fields })}
                        disabled={saving || !config.enabled}
                      />
                    </div>
                  </CardContent>
                </Card>
              </>
            )}
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}
