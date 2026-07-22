import { ClientGuardrailsTab } from "./guardrails-tab"
import { ClientCompressionTab } from "./compression-tab"
import { ClientJsonRepairTab } from "./json-repair-tab"
import { ClientSecretScanningTab } from "./secret-scanning-tab"
import { ClientMemoryTab } from "./memory-tab"
import type { McpMode } from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
  mcp_mode?: McpMode
}

interface LlmOptimizeTabProps {
  client: Client
  onUpdate: () => void
  onViewChange?: (view: string, subTab?: string | null) => void
}

export function ClientLlmOptimizeTab({ client, onUpdate, onViewChange }: LlmOptimizeTabProps) {
  // Conversation memory only captures (and can therefore recall) in the
  // MCP-via-LLM path — that orchestrator is the sole place transcripts are
  // indexed. In every other mode the toggle would be inert, so hide it.
  const showMemory = client.mcp_mode === "via_llm"
  return (
    <div className="space-y-4">
      <ClientCompressionTab client={client} onUpdate={onUpdate} onViewChange={onViewChange} />
      <ClientJsonRepairTab client={client} onUpdate={onUpdate} onViewChange={onViewChange} />
      <ClientGuardrailsTab client={client} onUpdate={onUpdate} onViewChange={onViewChange} />
      <ClientSecretScanningTab client={client} onUpdate={onUpdate} onViewChange={onViewChange} />
      {showMemory && (
        <ClientMemoryTab client={client} onUpdate={onUpdate} onViewChange={onViewChange} />
      )}
    </div>
  )
}
