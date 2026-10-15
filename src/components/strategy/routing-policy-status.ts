import type { RoutingPolicy, RoutingPolicyIssue } from '@/types/tauri-commands'

/**
 * Why a policy cannot send any request somewhere other than its default route.
 * Mirrors `RoutingPolicy::setup_issue` in crates/lr-config/src/routing_policy.rs.
 */
export function routingPolicyIssue(policy: RoutingPolicy | null | undefined): RoutingPolicyIssue | null {
  if (!policy) return null
  const canChoose = (policy.rules?.length ?? 0) > 0 || policy.mode_rules.length > 0 || (policy.mode === 'semantic' && policy.decision_model !== null)
  if (!canChoose) return policy.mode === 'semantic' ? 'needs_decision_model' : 'needs_mode_rule'
  if (policy.options.every(o => o.models.length === 0)) return 'needs_destination_models'
  return null
}

/** Short status, e.g. for a pill or list row. */
export const ROUTING_POLICY_ISSUE_LABEL: Record<RoutingPolicyIssue, string> = {
  needs_decision_model: 'Needs a decision model',
  needs_mode_rule: 'Needs a mode rule',
  needs_destination_models: 'Needs route models',
}

/** What the user must do, stated with the consequence until they do. */
export function routingPolicyIssueMessage(issue: RoutingPolicyIssue, defaultRoute: string): string {
  const until = `Until then every request uses the default route (${defaultRoute}).`
  switch (issue) {
    case 'needs_decision_model':
      return `This policy is on but cannot choose a route: select a decision model, or add an exact mode rule. ${until}`
    case 'needs_mode_rule':
      return `This policy is on but cannot choose a route: add an exact mode rule that maps a client mode to a route. ${until}`
    case 'needs_destination_models':
      return 'This policy is on but every route uses the ordinary priority list, so routing changes nothing: add destination models to at least one route.'
  }
}

export function isRoutingPolicyIssue(value: string | null | undefined): value is RoutingPolicyIssue {
  return value === 'needs_decision_model' || value === 'needs_mode_rule' || value === 'needs_destination_models'
}
