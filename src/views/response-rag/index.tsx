import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Database, Info, Settings } from "lucide-react"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { GatewayIndexingTree } from "@/components/permissions/GatewayIndexingTree"
import { IndexingStateButton } from "@/components/permissions/IndexingStateButton"
import type { ContextManagementConfig, IndexingState } from "@/types/tauri-commands"

// Must match defaults in crates/lr-config/src/types.rs
const DEFAULT_RESPONSE_THRESHOLD_BYTES = 200


interface ResponseRagViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function ResponseRagView({ activeSubTab, onTabChange }: ResponseRagViewProps) {
  const [config, setConfig] = useState<ContextManagementConfig | null>(null)
  const [, setSaving] = useState(false)

  const tab = activeSubTab || "info"

  const handleTabChange = (newTab: string) => {
    onTabChange?.("response-rag", newTab)
  }

  useEffect(() => {
    let ignore = false

    invoke<ContextManagementConfig>("get_context_management_config")
      .then((cfg) => { if (!ignore) setConfig(cfg) })
      .catch((err) => console.error("Failed to load context management config:", err))

    return () => {
      ignore = true
    }
  }, [])

  const updateField = async (field: string, value: unknown) => {
    try {
      setSaving(true)
      await invoke("update_context_management_config", { [field]: value })
      const updated = await invoke<ContextManagementConfig>("get_context_management_config")
      setConfig(updated)
    } catch (error) {
      toast.error(`Failed to update: ${error}`)
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="flex flex-col h-full min-h-0 max-w-5xl">
      <div className="flex-shrink-0 pb-4">
        <div className="flex items-center gap-2">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <Database className="h-6 w-6" />
            MCP Response RAG
          </h1>
          <Badge variant="outline" className="bg-purple-500/10 text-purple-900 dark:text-purple-400">EXPERIMENTAL</Badge>
        </div>
        <p className="text-sm text-muted-foreground">
          Index and compress tool responses using FTS5 search indexing
        </p>
      </div>

      <Tabs
        value={tab}
        onValueChange={handleTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="info">
            <Info className="h-3.5 w-3.5 mr-1" />
            Info
          </TabsTrigger>
          <TabsTrigger value="settings">
            <Settings className="h-3.5 w-3.5 mr-1" />
            Settings
          </TabsTrigger>
        </TabsList>

        {/* Info Tab */}
        <TabsContent value="info" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-2xl">
            {/* Tool Indexing */}
            {config && (
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">Tool Indexing</CardTitle>
                  <CardDescription>
                    Control which tool responses get indexed into FTS5 for search.
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-0">
                  <div className="border rounded-lg overflow-hidden">
                    {/* Gateway tools tree (strip its own border) */}
                    <div className="[&>div]:border-0 [&>div]:rounded-none">
                      <GatewayIndexingTree
                        permissions={config.gateway_indexing}
                        onUpdate={async () => {
                          const updated = await invoke<ContextManagementConfig>("get_context_management_config")
                          setConfig(updated)
                        }}
                      />
                    </div>

                    {/* Client Tools - same level as "All Gateway Tools" */}
                    <div className="flex items-center gap-2 px-3 py-3 border-t bg-background">
                      <span className="font-semibold text-sm flex-1 min-w-0 truncate">Client Tools</span>
                      <div className="shrink-0">
                        <IndexingStateButton
                          value={config.client_tools_indexing_default}
                          onChange={(state: IndexingState) => updateField("clientToolsIndexingDefault", state)}
                          size="sm"
                        />
                      </div>
                    </div>
                    <div className="flex items-center gap-2 py-2 border-t border-border/50 hover:bg-muted/30 transition-colors text-sm" style={{ paddingLeft: "28px", paddingRight: "12px" }}>
                      <div className="w-5" />
                      <span className="text-xs text-muted-foreground">
                        Per-client overrides are configured in each client&apos;s settings.
                      </span>
                    </div>
                  </div>
                </CardContent>
              </Card>
            )}
          </div>
        </TabsContent>

        {/* Settings Tab */}
        <TabsContent value="settings" className="flex-1 min-h-0 mt-4">
          {config && (
            <div className="space-y-4 max-w-2xl">
              {/* Response Threshold */}
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">Response Compression Threshold</CardTitle>
                  <CardDescription>
                    Tool responses larger than this threshold are indexed and replaced with a
                    truncated preview and a search hint.
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <div className="flex gap-2 items-center">
                    <Input
                      type="number"
                      defaultValue={config.response_threshold_bytes}
                      key={`response-${config.response_threshold_bytes}`}
                      onBlur={(e) => {
                        const v = parseInt(e.target.value)
                        if (!isNaN(v) && v >= 0 && v !== config.response_threshold_bytes) {
                          updateField("responseThresholdBytes", v)
                        }
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          (e.target as HTMLInputElement).blur()
                        }
                      }}
                      className="w-40"
                      min={0}
                    />
                    <span className="text-sm text-muted-foreground">bytes</span>
                    {config.response_threshold_bytes !== DEFAULT_RESPONSE_THRESHOLD_BYTES && (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => updateField("responseThresholdBytes", DEFAULT_RESPONSE_THRESHOLD_BYTES)}
                      >
                        Reset to default
                      </Button>
                    )}
                  </div>
                  <p className="text-xs text-muted-foreground mt-2">
                    Default: {DEFAULT_RESPONSE_THRESHOLD_BYTES.toLocaleString()} bytes. Set higher to compress fewer responses. Set to 0 to always compress.
                  </p>
                </CardContent>
              </Card>
            </div>
          )}
        </TabsContent>
      </Tabs>
    </div>
  )
}
