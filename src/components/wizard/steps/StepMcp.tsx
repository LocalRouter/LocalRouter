/**
 * Step 3: Select MCP Servers
 *
 * MCP server selection.
 * Shows empty state with option to add servers if none configured.
 */

import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Loader2, Info, Plus } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/Modal"
import LegacySelect from "@/components/ui/Select"
import KeyValueInput from "@/components/ui/KeyValueInput"
import { McpServerSelector } from "@/components/mcp/McpServerSelector"
import { McpServerTemplates, McpServerTemplate } from "@/components/mcp/McpServerTemplates"

interface McpServer {
  id: string
  name: string
  enabled: boolean
  proxy_url: string
}

type McpAccessMode = "none" | "all" | "specific"

interface StepMcpProps {
  accessMode: McpAccessMode
  selectedServers: string[]
  onChange: (mode: McpAccessMode, servers: string[]) => void
}

export function StepMcp({ accessMode, selectedServers, onChange }: StepMcpProps) {
  const [servers, setServers] = useState<McpServer[]>([])
  const [loading, setLoading] = useState(true)

  // MCP server creation state
  const [showAddServer, setShowAddServer] = useState(false)
  const [createTab, setCreateTab] = useState<"templates" | "manual">("templates")
  const [selectedTemplate, setSelectedTemplate] = useState<McpServerTemplate | null>(null)
  const [isCreating, setIsCreating] = useState(false)

  // Form state
  const [serverName, setServerName] = useState("")
  const [transportType, setTransportType] = useState<"Stdio" | "Sse">("Stdio")
  const [command, setCommand] = useState("")
  const [envVars, setEnvVars] = useState<Record<string, string>>({})
  const [url, setUrl] = useState("")
  const [headers, setHeaders] = useState<Record<string, string>>({})

  useEffect(() => {
    loadServers()
  }, [])

  const loadServers = async () => {
    try {
      setLoading(true)
      const serverList = await invoke<McpServer[]>("list_mcp_servers")
      setServers(serverList)
    } catch (error) {
      console.error("Failed to load MCP servers:", error)
      setServers([])
    } finally {
      setLoading(false)
    }
  }

  const resetForm = () => {
    setServerName("")
    setTransportType("Stdio")
    setCommand("")
    setEnvVars({})
    setUrl("")
    setHeaders({})
    setSelectedTemplate(null)
    setCreateTab("templates")
  }

  const handleSelectTemplate = (template: McpServerTemplate) => {
    setSelectedTemplate(template)
    setServerName(template.name)
    setTransportType(template.transport)

    if (template.transport === "Stdio" && template.command) {
      const fullCommand = template.args
        ? [template.command, ...template.args].join(" ")
        : template.command
      setCommand(fullCommand)
    } else if (template.transport === "Sse" && template.url) {
      setUrl(template.url)
    }

    setCreateTab("manual")
  }

  const handleCreateServer = async (e: React.FormEvent) => {
    e.preventDefault()
    setIsCreating(true)

    try {
      let transportConfig
      if (transportType === "Stdio") {
        transportConfig = {
          type: "stdio",
          command,
          env: envVars,
        }
      } else {
        transportConfig = {
          type: "http_sse",
          url,
          headers: headers,
        }
      }

      await invoke("create_mcp_server", {
        name: serverName || null,
        transport: transportType,
        transportConfig,
        authConfig: null,
      })

      toast.success("MCP server created")
      await loadServers()
      setShowAddServer(false)
      resetForm()
    } catch (error) {
      console.error("Failed to create MCP server:", error)
      toast.error(`Error creating MCP server: ${error}`)
    } finally {
      setIsCreating(false)
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (servers.length === 0) {
    return (
      <div className="space-y-4">
        <div className="rounded-lg border border-blue-500/30 bg-blue-500/10 p-4">
          <div className="flex items-start gap-3">
            <Info className="h-5 w-5 text-blue-600 dark:text-blue-400 mt-0.5 shrink-0" />
            <div className="space-y-1">
              <p className="text-sm font-medium text-blue-700 dark:text-blue-300">
                No MCP servers configured
              </p>
              <p className="text-sm text-blue-600/90 dark:text-blue-400/90">
                MCP servers provide tools and resources to LLM applications.
                You can add servers now or configure access later.
              </p>
            </div>
          </div>
        </div>

        <Button onClick={() => setShowAddServer(true)} className="w-full">
          <Plus className="h-4 w-4 mr-2" />
          Add MCP
        </Button>

        <p className="text-xs text-muted-foreground text-center">
          You can skip this step and add MCP access later.
        </p>

        {/* Add MCP Dialog */}
        <Dialog
          open={showAddServer}
          onOpenChange={(open) => {
            if (!open) {
              setShowAddServer(false)
              resetForm()
            }
          }}
        >
          <DialogContent className="max-w-2xl max-h-[90vh] overflow-y-auto">
            <DialogHeader>
              <DialogTitle>Add MCP</DialogTitle>
            </DialogHeader>

            <Tabs value={createTab} onValueChange={(v) => setCreateTab(v as "templates" | "manual")}>
              <TabsList className="grid w-full grid-cols-2">
                <TabsTrigger value="templates">Templates</TabsTrigger>
                <TabsTrigger value="manual">Manual</TabsTrigger>
              </TabsList>

              {/* Templates Tab */}
              <TabsContent value="templates" className="mt-4">
                <McpServerTemplates
                  onSelectTemplate={(template) => {
                    handleSelectTemplate(template)
                  }}
                />
              </TabsContent>

              {/* Manual Tab */}
              <TabsContent value="manual" className="mt-4">
                <form onSubmit={handleCreateServer} className="space-y-4">
                  {/* Show selected template info */}
                  {selectedTemplate && (
                    <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded p-3">
                      <div className="flex items-center justify-between">
                        <div className="flex items-center gap-2">
                          <span className="text-2xl">{selectedTemplate.icon}</span>
                          <div>
                            <p className="text-sm font-medium text-blue-900 dark:text-blue-100">
                              Using template: {selectedTemplate.name}
                            </p>
                            <p className="text-xs text-blue-700 dark:text-blue-300">
                              Customize the settings below
                            </p>
                          </div>
                        </div>
                        <Button
                          type="button"
                          variant="secondary"
                          size="sm"
                          onClick={() => {
                            setSelectedTemplate(null)
                            setServerName("")
                            setCommand("")
                            setUrl("")
                          }}
                        >
                          Clear
                        </Button>
                      </div>
                      {selectedTemplate.setupInstructions && (
                        <p className="text-xs text-blue-700 dark:text-blue-300 mt-2">
                          {selectedTemplate.setupInstructions}
                        </p>
                      )}
                    </div>
                  )}

                  <div>
                    <Label className="mb-2 block">Server Name</Label>
                    <Input
                      value={serverName}
                      onChange={(e) => setServerName(e.target.value)}
                      placeholder="My MCP Server"
                      required
                    />
                  </div>

                  <div>
                    <Label className="mb-2 block">Transport Type</Label>
                    <LegacySelect
                      value={transportType}
                      onChange={(e) => setTransportType(e.target.value as "Stdio" | "Sse")}
                    >
                      <option value="Stdio">STDIO (Subprocess)</option>
                      <option value="Sse">HTTP-SSE (Server-Sent Events)</option>
                    </LegacySelect>
                  </div>

                  {/* STDIO Config */}
                  {transportType === "Stdio" && (
                    <>
                      <div>
                        <Label className="mb-2 block">Command</Label>
                        <Input
                          value={command}
                          onChange={(e) => setCommand(e.target.value)}
                          placeholder="npx -y @modelcontextprotocol/server-everything"
                          required
                        />
                        <p className="text-xs text-muted-foreground mt-1">
                          Full command with arguments
                        </p>
                      </div>

                      <div>
                        <Label className="mb-2 block">Environment Variables</Label>
                        <KeyValueInput
                          value={envVars}
                          onChange={setEnvVars}
                          keyPlaceholder="KEY"
                          valuePlaceholder="VALUE"
                        />
                      </div>
                    </>
                  )}

                  {/* HTTP-SSE Config */}
                  {transportType === "Sse" && (
                    <>
                      <div>
                        <Label className="mb-2 block">URL</Label>
                        <Input
                          value={url}
                          onChange={(e) => setUrl(e.target.value)}
                          placeholder="https://api.example.com/mcp"
                          required
                        />
                      </div>

                      <div>
                        <Label className="mb-2 block">Headers (Optional)</Label>
                        <KeyValueInput
                          value={headers}
                          onChange={setHeaders}
                          keyPlaceholder="Header Name"
                          valuePlaceholder="Header Value"
                        />
                      </div>
                    </>
                  )}

                  <div className="flex justify-end gap-2 pt-4">
                    <Button
                      type="button"
                      variant="secondary"
                      onClick={() => {
                        setShowAddServer(false)
                        resetForm()
                      }}
                      disabled={isCreating}
                    >
                      Cancel
                    </Button>
                    <Button type="submit" disabled={isCreating}>
                      {isCreating ? "Creating..." : "Create Server"}
                    </Button>
                  </div>
                </form>
              </TabsContent>
            </Tabs>
          </DialogContent>
        </Dialog>
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        Select which MCP servers this client can access.
      </p>

      <McpServerSelector
        servers={servers}
        accessMode={accessMode}
        selectedServers={selectedServers}
        onChange={onChange}
      />

      <div className="flex items-center justify-between pt-2">
        <p className="text-xs text-muted-foreground">
          MCP servers provide tools and resources like filesystem access, database queries, and more.
        </p>
        <Button
          variant="outline"
          size="sm"
          onClick={() => setShowAddServer(true)}
        >
          <Plus className="h-3 w-3 mr-1" />
          Add Server
        </Button>
      </div>

      {/* Add MCP Dialog (also available when servers exist) */}
      <Dialog
        open={showAddServer}
        onOpenChange={(open) => {
          if (!open) {
            setShowAddServer(false)
            resetForm()
          }
        }}
      >
        <DialogContent className="max-w-2xl max-h-[90vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Add MCP</DialogTitle>
          </DialogHeader>

          <Tabs value={createTab} onValueChange={(v) => setCreateTab(v as "templates" | "manual")}>
            <TabsList className="grid w-full grid-cols-2">
              <TabsTrigger value="templates">Templates</TabsTrigger>
              <TabsTrigger value="manual">Manual</TabsTrigger>
            </TabsList>

            {/* Templates Tab */}
            <TabsContent value="templates" className="mt-4">
              <McpServerTemplates
                onSelectTemplate={(template) => {
                  handleSelectTemplate(template)
                }}
              />
            </TabsContent>

            {/* Manual Tab */}
            <TabsContent value="manual" className="mt-4">
              <form onSubmit={handleCreateServer} className="space-y-4">
                {/* Show selected template info */}
                {selectedTemplate && (
                  <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded p-3">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-2">
                        <span className="text-2xl">{selectedTemplate.icon}</span>
                        <div>
                          <p className="text-sm font-medium text-blue-900 dark:text-blue-100">
                            Using template: {selectedTemplate.name}
                          </p>
                          <p className="text-xs text-blue-700 dark:text-blue-300">
                            Customize the settings below
                          </p>
                        </div>
                      </div>
                      <Button
                        type="button"
                        variant="secondary"
                        size="sm"
                        onClick={() => {
                          setSelectedTemplate(null)
                          setServerName("")
                          setCommand("")
                          setUrl("")
                        }}
                      >
                        Clear
                      </Button>
                    </div>
                    {selectedTemplate.setupInstructions && (
                      <p className="text-xs text-blue-700 dark:text-blue-300 mt-2">
                        {selectedTemplate.setupInstructions}
                      </p>
                    )}
                  </div>
                )}

                <div>
                  <Label className="mb-2 block">Server Name</Label>
                  <Input
                    value={serverName}
                    onChange={(e) => setServerName(e.target.value)}
                    placeholder="My MCP Server"
                    required
                  />
                </div>

                <div>
                  <Label className="mb-2 block">Transport Type</Label>
                  <LegacySelect
                    value={transportType}
                    onChange={(e) => setTransportType(e.target.value as "Stdio" | "Sse")}
                  >
                    <option value="Stdio">STDIO (Subprocess)</option>
                    <option value="Sse">HTTP-SSE (Server-Sent Events)</option>
                  </LegacySelect>
                </div>

                {/* STDIO Config */}
                {transportType === "Stdio" && (
                  <>
                    <div>
                      <Label className="mb-2 block">Command</Label>
                      <Input
                        value={command}
                        onChange={(e) => setCommand(e.target.value)}
                        placeholder="npx -y @modelcontextprotocol/server-everything"
                        required
                      />
                      <p className="text-xs text-muted-foreground mt-1">
                        Full command with arguments
                      </p>
                    </div>

                    <div>
                      <Label className="mb-2 block">Environment Variables</Label>
                      <KeyValueInput
                        value={envVars}
                        onChange={setEnvVars}
                        keyPlaceholder="KEY"
                        valuePlaceholder="VALUE"
                      />
                    </div>
                  </>
                )}

                {/* HTTP-SSE Config */}
                {transportType === "Sse" && (
                  <>
                    <div>
                      <Label className="mb-2 block">URL</Label>
                      <Input
                        value={url}
                        onChange={(e) => setUrl(e.target.value)}
                        placeholder="https://api.example.com/mcp"
                        required
                      />
                    </div>

                    <div>
                      <Label className="mb-2 block">Headers (Optional)</Label>
                      <KeyValueInput
                        value={headers}
                        onChange={setHeaders}
                        keyPlaceholder="Header Name"
                        valuePlaceholder="Header Value"
                      />
                    </div>
                  </>
                )}

                <div className="flex justify-end gap-2 pt-4">
                  <Button
                    type="button"
                    variant="secondary"
                    onClick={() => {
                      setShowAddServer(false)
                      resetForm()
                    }}
                    disabled={isCreating}
                  >
                    Cancel
                  </Button>
                  <Button type="submit" disabled={isCreating}>
                    {isCreating ? "Creating..." : "Create Server"}
                  </Button>
                </div>
              </form>
            </TabsContent>
          </Tabs>
        </DialogContent>
      </Dialog>
    </div>
  )
}
