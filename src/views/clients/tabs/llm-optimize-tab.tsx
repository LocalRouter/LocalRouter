import { ClientGuardrailsTab } from "./guardrails-tab"
import { ClientCompressionTab } from "./compression-tab"
import { ClientSecretScanningTab } from "./secret-scanning-tab"

interface Client {
  id: string
  name: string
  client_id: string
}

interface LlmOptimizeTabProps {
  client: Client
  onUpdate: () => void
  onViewChange?: (view: string, subTab?: string | null) => void
}

export function ClientLlmOptimizeTab({ client, onUpdate, onViewChange }: LlmOptimizeTabProps) {
  return (
    <div className="space-y-4">
      <ClientCompressionTab client={client} onUpdate={onUpdate} onViewChange={onViewChange} />
      <ClientGuardrailsTab client={client} onUpdate={onUpdate} onViewChange={onViewChange} />
      <ClientSecretScanningTab client={client} onUpdate={onUpdate} onViewChange={onViewChange} />
    </div>
  )
}
