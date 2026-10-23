/**
 * Website demo wrapper for the Secret Scanning approval popup.
 * Renders the shared FirewallApprovalCard with isSecretScanRequest=true
 * and hardcoded demo findings in a dark-themed container.
 */
import {
  FirewallApprovalCard,
} from "@app/components/shared/FirewallApprovalCard"
import type { SecretFindingSummary } from "@app/types/tauri-commands"

const REQUEST_JSON = JSON.stringify({ model: 'gpt-4o', messages: [{ role: 'user', content: 'OPENAI_API_KEY=[REDACTED: secret removed by LocalRouter]' }] }, null, 2)
const MATCH_START = REQUEST_JSON.indexOf('[REDACTED: secret removed by LocalRouter]')
const DEMO_FINDINGS: SecretFindingSummary[] = [
  {
    rule_id: "openai-api-key",
    rule_description: "OpenAI API Key",
    category: "ai_service",
    matched_text: "sk-proj...4f3K",
    entropy: 4.82,
    request_match: { start: MATCH_START, end: MATCH_START + "[REDACTED: secret removed by LocalRouter]".length },
    context: { source: 'Message 2 · user message', line: 2, before: 'Check my configuration:\nOPENAI_API_KEY=', after: '\nWhy does authentication fail?' },
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
        secretScanLlmSource="OpenAI"
        secretScanFindings={DEMO_FINDINGS}
        secretScanRequestJson={REQUEST_JSON}
        onAction={noop}
        onRedactPermanently={async () => {}}
        onAllowSecretPermanently={async () => {}}
      />
    </div>
  )
}
