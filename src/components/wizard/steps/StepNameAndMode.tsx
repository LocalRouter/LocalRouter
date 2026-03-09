/**
 * StepNameAndMode - Combined name input and client mode selection.
 *
 * Merges the old StepName and StepMode into a single wizard step.
 * - Name input at the top
 * - Shared ClientModeSelector below (with custom arrow icons)
 */

import type { ClientMode } from "@/types/tauri-commands"
import type { ClientTemplate } from "@/components/client/ClientTemplates"
import { ClientModeSelector } from "@/components/client/ClientModeSelector"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"

interface StepNameAndModeProps {
  name: string
  onNameChange: (name: string) => void
  mode: ClientMode
  onModeChange: (mode: ClientMode) => void
  template: ClientTemplate | null
}

export function StepNameAndMode({
  name,
  onNameChange,
  mode,
  onModeChange,
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
        <ClientModeSelector mode={mode} onModeChange={onModeChange} template={template} />
      </div>
    </div>
  )
}
