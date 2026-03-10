import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Wrench, Play, Loader2 } from "lucide-react"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/Toggle"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import type { JsonRepairConfig, JsonRepairTestResult } from "@/types/tauri-commands"

interface JsonRepairViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function JsonRepairView({ activeSubTab, onTabChange }: JsonRepairViewProps) {
  const [config, setConfig] = useState<JsonRepairConfig | null>(null)
  const [saving, setSaving] = useState(false)
  const [testInput, setTestInput] = useState(`\`\`\`json
{name: 'Alice' "age": "28" "score": "95.5",
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
  const [useSchema, setUseSchema] = useState(true)

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

  const runTest = async () => {
    if (!testInput.trim()) return
    setTestLoading(true)
    setTestResult(null)
    try {
      const result = await invoke<JsonRepairTestResult>("test_json_repair", {
        content: testInput,
        schema: useSchema ? testSchema : null,
      })
      setTestResult(result)
    } catch (err) {
      toast.error(`JSON repair test failed: ${err}`)
    } finally {
      setTestLoading(false)
    }
  }

  return (
    <div className="flex flex-col h-full p-6 gap-4">
      <div className="flex-shrink-0">
        <div className="flex items-center gap-3 mb-1">
          <h1 className="text-xl font-semibold flex items-center gap-2">
            <Wrench className="h-6 w-6" />
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
          <TabsTrigger value="info">Info</TabsTrigger>
          <TabsTrigger value="try-it-out">Try it out</TabsTrigger>
          <TabsTrigger value="settings">Settings</TabsTrigger>
        </TabsList>

        {/* Info Tab */}
        <TabsContent value="info" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-2xl overflow-y-auto">
            {config && (
              <Card>
                <CardHeader className="pb-3">
                  <div className="flex items-center justify-between">
                    <div>
                      <CardTitle className="text-base">Global Enable JSON Repair</CardTitle>
                      <CardDescription>Automatically repair JSON responses for requests with JSON response format. Individual clients can override this in their settings.</CardDescription>
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
                <CardTitle className="text-base">Syntax Repair</CardTitle>
                <CardDescription>
                  Fixes malformed JSON syntax so the response is valid and parseable. Runs character-by-character with near-zero latency.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <div className="space-y-3 text-sm">
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
                </div>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-base">Schema Coercion</CardTitle>
                <CardDescription>
                  When a JSON Schema is provided via <code className="text-xs bg-muted px-1 py-0.5 rounded">response_format: json_schema</code>, values are coerced to match the schema.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <div className="space-y-3 text-sm">
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

            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-base">Streaming Support</CardTitle>
                <CardDescription>
                  Unlike other services that only repair non-streaming responses, LocalRouter repairs JSON inline as chunks arrive.
                  The character-at-a-time state machine buffers only the current token (string, number, keyword) — never the full response.
                </CardDescription>
              </CardHeader>
            </Card>

            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-base">How it activates</CardTitle>
                <CardDescription>
                  JSON Repair automatically activates for requests with <code className="text-xs bg-muted px-1 py-0.5 rounded">response_format: json_object</code> or <code className="text-xs bg-muted px-1 py-0.5 rounded">json_schema</code>.
                  It can be disabled globally or overridden per-client. No code changes needed on the caller side.
                </CardDescription>
              </CardHeader>
            </Card>
          </div>
        </TabsContent>

        {/* Try it out Tab */}
        <TabsContent value="try-it-out" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-2xl overflow-y-auto">
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-base">Test JSON Repair</CardTitle>
                <CardDescription>
                  Paste malformed JSON and optionally provide a schema to test repair
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div>
                  <label className="text-sm font-medium mb-1.5 block">Input (malformed JSON)</label>
                  <textarea
                    value={testInput}
                    onChange={(e) => setTestInput(e.target.value)}
                    className="w-full h-48 px-3 py-2 text-sm bg-muted rounded-md border font-mono resize-none focus:outline-none focus:ring-2 focus:ring-ring"
                    placeholder='{"name": "John", "age": "30",}'
                  />
                </div>

                <div className="flex items-center gap-2">
                  <Switch
                    checked={useSchema}
                    onCheckedChange={setUseSchema}
                  />
                  <span className="text-sm">Include JSON Schema for coercion</span>
                </div>

                {useSchema && (
                  <div>
                    <label className="text-sm font-medium mb-1.5 block">JSON Schema</label>
                    <textarea
                      value={testSchema}
                      onChange={(e) => setTestSchema(e.target.value)}
                      className="w-full h-48 px-3 py-2 text-sm bg-muted rounded-md border font-mono resize-none focus:outline-none focus:ring-2 focus:ring-ring"
                      placeholder='{"type": "object", "properties": {...}}'
                    />
                  </div>
                )}

                <Button onClick={runTest} disabled={testLoading || !testInput.trim()}>
                  {testLoading ? (
                    <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                  ) : (
                    <Play className="h-4 w-4 mr-2" />
                  )}
                  Repair
                </Button>

                {testResult && (
                  <div className="space-y-3">
                    <div className="flex items-center gap-2">
                      {testResult.was_modified ? (
                        <Badge variant="default" className="bg-green-600">Repaired</Badge>
                      ) : (
                        <Badge variant="secondary">No changes needed</Badge>
                      )}
                      {testResult.repairs.length > 0 && (
                        <span className="text-xs text-muted-foreground">
                          {testResult.repairs.length} fix(es) applied
                        </span>
                      )}
                    </div>

                    <div>
                      <label className="text-sm font-medium mb-1.5 block">Output</label>
                      <pre className="w-full px-3 py-2 text-sm bg-muted rounded-md border font-mono whitespace-pre-wrap overflow-auto max-h-48">
                        {testResult.repaired}
                      </pre>
                    </div>

                    {testResult.repairs.length > 0 && (
                      <div>
                        <label className="text-sm font-medium mb-1.5 block">Repairs performed</label>
                        <ul className="text-xs text-muted-foreground space-y-1">
                          {testResult.repairs.map((repair, i) => (
                            <li key={i} className="font-mono bg-muted px-2 py-1 rounded">
                              {typeof repair === "string"
                                ? repair.replace(/_/g, " ")
                                : JSON.stringify(repair)}
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
