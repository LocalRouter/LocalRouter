import type { ModelPermissions, RouteSnapshot } from '@/types/tauri-commands'

export function profileModelRefs(snapshot: RouteSnapshot): [string, string][] {
  const refs = [...snapshot.prioritized_models]
  const policy = snapshot.routing_policy
  if (policy?.enabled) {
    refs.push(...policy.options.flatMap(o => o.models))
    if (policy.mode === 'semantic' && policy.decision_model) refs.push(policy.decision_model)
  }
  return refs.filter((ref, i) => refs.findIndex(other => other[0] === ref[0] && other[1] === ref[1]) === i)
}
export function profilePermissionIssues(snapshot: RouteSnapshot, permissions: ModelPermissions): string[] {
  return profileModelRefs(snapshot).flatMap(([provider, model]) => {
    const permission = permissions.models[`${provider}__${model}`] ?? permissions.providers[provider] ?? permissions.global
    return permission === 'off' ? [`Model not permitted: ${provider}/${model}`] : permission === 'ask' ? [`Model requires approval: ${provider}/${model}`] : []
  })
}
