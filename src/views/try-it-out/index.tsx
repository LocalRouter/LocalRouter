import { useState, useEffect } from "react"
import { FlaskConical } from "lucide-react"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { LlmTab } from "./llm-tab"
import { McpTab } from "./mcp-tab"
import { RouteLLMTryItOutTab } from "./routellm-tab"
import { GuardrailsTab } from "./guardrails-tab"

interface TryItOutViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

// Parse init path: "init/<mode>/<target>" -> { mode, target }
function parseInitPath(innerPath: string | null): {
  initMode?: string
  initTarget?: string
} {
  if (!innerPath || !innerPath.startsWith("init/")) return {}
  const parts = innerPath.slice(5).split("/") // remove "init/"
  return { initMode: parts[0] || undefined, initTarget: parts.slice(1).join("/") || undefined }
}

export function TryItOutView({ activeSubTab, onTabChange }: TryItOutViewProps) {
  // Parse subTab to determine which main tab is selected
  // Format: "llm" or "mcp" or "llm/..." or "mcp/..."
  const parseSubTab = (subTab: string | null) => {
    if (!subTab) return { mainTab: "llm", innerPath: null }
    const parts = subTab.split("/")
    const mainTab = parts[0] || "llm"
    const innerPath = parts.slice(1).join("/") || null
    return { mainTab, innerPath }
  }

  const { mainTab, innerPath } = parseSubTab(activeSubTab)
  const { initMode, initTarget } = parseInitPath(innerPath)

  // Store init params so they survive the URL cleanup
  const [llmInitial, setLlmInitial] = useState<{
    mode?: "client" | /* "strategy" | */ "direct"
    provider?: string
    clientId?: string
  }>({})
  const [mcpInitial, setMcpInitial] = useState<{
    mode?: "client" | "all" | "direct"
    directTarget?: string
    clientId?: string
  }>({})
  const [guardrailsInitial, setGuardrailsInitial] = useState<{
    clientId?: string
  }>({})

  // Capture init params and clear the URL
  useEffect(() => {
    if (!initMode) return

    if (mainTab === "llm") {
      if (initMode === "direct" && initTarget) {
        setLlmInitial({ mode: "direct", provider: initTarget })
      } else if (initMode === "client" && initTarget) {
        setLlmInitial({ mode: "client", clientId: initTarget })
      }
    } else if (mainTab === "mcp") {
      if (initMode === "direct" && initTarget) {
        setMcpInitial({ mode: "direct", directTarget: initTarget })
      } else if (initMode === "client" && initTarget) {
        setMcpInitial({ mode: "client", clientId: initTarget })
      }
    } else if (mainTab === "guardrails") {
      if (initMode === "client" && initTarget) {
        setGuardrailsInitial({ clientId: initTarget })
      }
    }

    // Clear init from URL
    onTabChange("try-it-out", mainTab)
  }, []) // Only on mount

  const handleTabChange = (tab: string) => {
    onTabChange("try-it-out", tab)
  }

  const handleInnerPathChange = (tab: string, path: string | null) => {
    onTabChange("try-it-out", path ? `${tab}/${path}` : tab)
  }

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="flex-shrink-0 pb-4">
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2"><FlaskConical className="h-6 w-6" />Try It Out</h1>
        <p className="text-sm text-muted-foreground">
          Test LLM completions, MCP server capabilities, and Strong/Weak routing
        </p>
      </div>

      <Tabs
        value={mainTab}
        onValueChange={handleTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="llm">LLM</TabsTrigger>
          <TabsTrigger value="mcp">MCP & Skill</TabsTrigger>
          <TabsTrigger value="routellm">Strong/Weak</TabsTrigger>
          <TabsTrigger value="guardrails">GuardRails</TabsTrigger>
        </TabsList>

        <TabsContent value="llm" className="flex-1 min-h-0 mt-4">
          <LlmTab
            initialMode={llmInitial.mode}
            initialProvider={llmInitial.provider}
            initialClientId={llmInitial.clientId}
          />
        </TabsContent>

        <TabsContent value="mcp" className="flex-1 min-h-0 mt-4">
          <McpTab
            innerPath={mainTab === "mcp" ? innerPath : null}
            onPathChange={(path) => handleInnerPathChange("mcp", path)}
            initialMode={mcpInitial.mode}
            initialDirectTarget={mcpInitial.directTarget}
            initialClientId={mcpInitial.clientId}
          />
        </TabsContent>

        <TabsContent value="routellm" className="flex-1 min-h-0 mt-4">
          <RouteLLMTryItOutTab />
        </TabsContent>

        <TabsContent value="guardrails" className="flex-1 min-h-0 mt-4">
          <GuardrailsTab initialClientId={guardrailsInitial.clientId} />
        </TabsContent>
      </Tabs>
    </div>
  )
}
