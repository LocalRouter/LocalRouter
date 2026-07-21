/**
 * StepNameAndMode - Combined name input and client mode selection.
 *
 * Merges the old StepName and StepMode into a single wizard step.
 * - Name input at the top
 * - Shared ClientModeSelector below (with custom arrow icons)
 */

import type { LlmMode, McpMode } from "@/types/tauri-commands"
import type { ClientTemplate } from "@/components/client/ClientTemplates"
import { ClientModeSelector } from "@/components/client/ClientModeSelector"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"

interface StepNameAndModeProps {
  name: string
  onNameChange: (name: string) => void
  llmMode: LlmMode
  mcpMode: McpMode
  onLlmModeChange: (mode: LlmMode) => void
  onMcpModeChange: (mode: McpMode) => void
  template: ClientTemplate | null
}

export function StepNameAndMode({
  name,
  onNameChange,
  llmMode,
  mcpMode,
  onLlmModeChange,
  onMcpModeChange,
  template,
}: StepNameAndModeProps) {
  return (
    <div className="space-y-6">
      {/* Name input */}
      <div className="space-y-2">
        <Label htmlFor="client-name">Client Name</Label>
        <Input
          id="client-name"
          placeholder="e.g., OpenCode, Development, All MCPs"
          value={name}
          onChange={(e) => onNameChange(e.target.value)}
          autoFocus
        />
      </div>

      {/* Mode selection */}
      <div className="space-y-2">
        <Label>Access Mode</Label>
        <ClientModeSelector
          llmMode={llmMode}
          mcpMode={mcpMode}
          onLlmModeChange={onLlmModeChange}
          onMcpModeChange={onMcpModeChange}
          template={template}
        />
      </div>
    </div>
  )
}
