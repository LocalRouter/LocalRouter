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

      {/* Wrapping a provider changes something outside LocalRouter — the
          provider's own port — so say so before the client is created rather
          than only in the setup screen afterwards. */}
      {template?.supportsReverseProxy && template.reverseProxy && (
        <div className="rounded-md border bg-muted/40 p-3 text-xs text-muted-foreground space-y-1">
          <p className="font-medium text-foreground">
            LocalRouter will take over port {template.reverseProxy.listenPort}
          </p>
          <p>
            {template.name} moves to port {template.reverseProxy.upstreamPort}, and LocalRouter
            answers on {template.reverseProxy.listenPort} in its place — so every app already
            pointed there keeps working, and its traffic shows up in the Monitor.
          </p>
          <p>You&apos;ll run the actual switch (and can undo it) on the next screen.</p>
        </div>
      )}

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
