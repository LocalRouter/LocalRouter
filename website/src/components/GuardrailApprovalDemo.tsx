/**
 * Website demo wrapper for the GuardRail approval popup.
 * Renders the shared FirewallApprovalCard with isGuardrailRequest=true
 * and hardcoded demo matches in a dark-themed container.
 */
import {
  FirewallApprovalCard,
} from "@app/components/shared/FirewallApprovalCard"
import type { GuardrailMatchInfo, SourceCheckSummary } from "@app/types/tauri-commands"

const DEMO_MATCHES: GuardrailMatchInfo[] = [
  {
    rule_id: "builtin_ignore_previous",
    rule_name: "Ignore Previous Instructions",
    source_id: "builtin",
    source_label: "Built-in Rules",
    category: "prompt_injection",
    severity: "high",
    direction: "input",
    matched_text: "Ignore all previous instructions and...",
    message_index: 0,
    description: "Detected attempt to override system prompt",
  },
  {
    rule_id: "builtin_dan_mode",
    rule_name: "DAN Mode Jailbreak",
    source_id: "builtin",
    source_label: "Built-in Rules",
    category: "jailbreak",
    severity: "critical",
    direction: "input",
    matched_text: "You are now DAN, which stands for...",
    message_index: 0,
    description: "Known jailbreak pattern attempting to bypass safety",
  },
]

const DEMO_SOURCES: SourceCheckSummary[] = [
  { source_id: "builtin", source_label: "Built-in Rules", rules_checked: 38, match_count: 2 },
  { source_id: "llm_guard", source_label: "LLM Guard", rules_checked: 24, match_count: 0 },
]

export function GuardrailApprovalDemo() {
  const noop = () => {}

  return (
    <div className="dark rounded-lg border-2 border-border shadow-2xl max-w-sm bg-background">
      <FirewallApprovalCard
        className="flex flex-col p-4"
        clientName="Claude Code"
        toolName="claude-3-5-sonnet-20241022"
        serverName="anthropic"
        isGuardrailRequest
        guardrailMatches={DEMO_MATCHES}
        guardrailDirection="request"
        guardrailSourcesSummary={DEMO_SOURCES}
        onAction={noop}
      />
    </div>
  )
}
