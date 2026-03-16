/**
 * Website demo wrapper for the Secret Scanning approval popup.
 * Renders the shared FirewallApprovalCard with isSecretScanRequest=true
 * and hardcoded demo findings in a dark-themed container.
 */
import {
  FirewallApprovalCard,
} from "@app/components/shared/FirewallApprovalCard"
import type { SecretFindingSummary } from "@app/types/tauri-commands"

const DEMO_FINDINGS: SecretFindingSummary[] = [
  {
    rule_id: "openai-api-key",
    rule_description: "OpenAI API Key",
    category: "ai_service",
    matched_text: "sk-proj...4f3K",
    entropy: 4.82,
  },
]

export function SecretScanApprovalDemo() {
  const noop = () => {}

  return (
    <div className="dark rounded-lg border-2 border-border shadow-2xl max-w-sm bg-background">
      <FirewallApprovalCard
        className="flex flex-col p-4"
        clientName="Cursor"
        toolName="gpt-4o"
        serverName="openai"
        isSecretScanRequest
        secretScanFindings={DEMO_FINDINGS}
        secretScanDurationMs={12}
        onAction={noop}
      />
    </div>
  )
}
