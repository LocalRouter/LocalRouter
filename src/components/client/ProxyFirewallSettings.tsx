/**
 * Editor for a client's HTTPS-proxy firewall policy: default action, model
 * enforcement, forced model rewrites, and ordered match rules. Shown for a
 * client in the HTTPS Proxy LLM mode.
 */

import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import type {
  LlmProxyPolicy,
  FirewallAction,
  FirewallRule,
  ModelRewrite,
  GetClientProxyPolicyParams,
  SetClientProxyPolicyParams,
} from "@/types/tauri-commands"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Plus, Trash2, ShieldCheck } from "lucide-react"

const ACTIONS: FirewallAction[] = ["allow", "ask", "deny"]

function ActionSelect({ value, onChange }: { value: FirewallAction; onChange: (a: FirewallAction) => void }) {
  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value as FirewallAction)}
      className="h-7 rounded-md border border-border bg-background px-2 text-xs"
    >
      {ACTIONS.map((a) => (
        <option key={a} value={a}>
          {a}
        </option>
      ))}
    </select>
  )
}

export function ProxyFirewallSettings({ clientId }: { clientId: string }) {
  const [policy, setPolicy] = useState<LlmProxyPolicy | null>(null)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    let cancelled = false
    invoke<LlmProxyPolicy>("get_client_proxy_policy", { clientId } satisfies GetClientProxyPolicyParams)
      .then((p) => { if (!cancelled) setPolicy(p) })
      .catch((e) => console.error("Failed to load firewall policy:", e))
    return () => { cancelled = true }
  }, [clientId])

  const save = async (next: LlmProxyPolicy) => {
    setPolicy(next)
    setSaving(true)
    try {
      await invoke("set_client_proxy_policy", { clientId, policy: next } satisfies SetClientProxyPolicyParams)
    } catch (e) {
      toast.error(`Failed to save firewall policy: ${e}`)
    } finally {
      setSaving(false)
    }
  }

  if (!policy) return null

  const patch = (p: Partial<LlmProxyPolicy>) => save({ ...policy, ...p })

  const updateRule = (i: number, r: Partial<FirewallRule>) =>
    patch({ rules: policy.rules.map((rule, idx) => (idx === i ? { ...rule, ...r } : rule)) })
  const updateMatcher = (i: number, m: Partial<FirewallRule["matcher"]>) =>
    updateRule(i, { matcher: { ...policy.rules[i].matcher, ...m } })

  const addRule = () =>
    patch({ rules: [...policy.rules, { name: "New rule", matcher: {}, action: "ask", enabled: true }] })
  const removeRule = (i: number) => patch({ rules: policy.rules.filter((_, idx) => idx !== i) })

  const addRewrite = () => patch({ model_rewrites: [...policy.model_rewrites, { from: "", to: "" }] })
  const updateRewrite = (i: number, r: Partial<ModelRewrite>) =>
    patch({ model_rewrites: policy.model_rewrites.map((rw, idx) => (idx === i ? { ...rw, ...r } : rw)) })
  const removeRewrite = (i: number) =>
    patch({ model_rewrites: policy.model_rewrites.filter((_, idx) => idx !== i) })

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <ShieldCheck className="h-4 w-4" /> Firewall
        </CardTitle>
        <CardDescription>
          Control proxied requests — allow, ask for approval, or deny — and rewrite models.
          {saving && <span className="ml-2 text-xs">saving…</span>}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-5">
        {/* Default action + model enforcement */}
        <div className="flex flex-wrap items-center gap-4">
          <div className="flex items-center gap-2">
            <Label className="text-xs text-muted-foreground">Default action</Label>
            <ActionSelect value={policy.default_action} onChange={(a) => patch({ default_action: a })} />
          </div>
          <label className="flex items-center gap-2 text-xs">
            <Switch
              checked={policy.enforce_model_permissions}
              onCheckedChange={(v) => patch({ enforce_model_permissions: v })}
            />
            Enforce strategy model allow-list
          </label>
        </div>

        {/* Rules */}
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <Label className="text-xs font-medium">Rules (first match wins)</Label>
            <Button size="sm" variant="outline" className="h-6 gap-1 text-xs" onClick={addRule}>
              <Plus className="h-3 w-3" /> Add rule
            </Button>
          </div>
          {policy.rules.length === 0 && (
            <p className="text-xs text-muted-foreground">No rules — the default action applies to all requests.</p>
          )}
          {policy.rules.map((rule, i) => (
            <div key={i} className="rounded-md border p-2 space-y-2">
              <div className="flex items-center gap-2">
                <Switch checked={rule.enabled} onCheckedChange={(v) => updateRule(i, { enabled: v })} />
                <Input
                  value={rule.name}
                  onChange={(e) => updateRule(i, { name: e.target.value })}
                  className="h-7 text-xs"
                  placeholder="Rule name"
                />
                <ActionSelect value={rule.action} onChange={(a) => updateRule(i, { action: a })} />
                <Button size="icon" variant="ghost" className="h-7 w-7 shrink-0" onClick={() => removeRule(i)}>
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </div>
              <div className="grid grid-cols-3 gap-2">
                <Input
                  value={rule.matcher.model_contains ?? ""}
                  onChange={(e) => updateMatcher(i, { model_contains: e.target.value || null })}
                  className="h-7 text-xs"
                  placeholder="model contains…"
                />
                <Input
                  value={rule.matcher.content_contains ?? ""}
                  onChange={(e) => updateMatcher(i, { content_contains: e.target.value || null })}
                  className="h-7 text-xs"
                  placeholder="content contains…"
                />
                <select
                  value={rule.matcher.has_tools == null ? "" : String(rule.matcher.has_tools)}
                  onChange={(e) =>
                    updateMatcher(i, { has_tools: e.target.value === "" ? null : e.target.value === "true" })
                  }
                  className="h-7 rounded-md border border-border bg-background px-2 text-xs"
                >
                  <option value="">tools: any</option>
                  <option value="true">tools: yes</option>
                  <option value="false">tools: no</option>
                </select>
              </div>
            </div>
          ))}
        </div>

        {/* Model rewrites */}
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <Label className="text-xs font-medium">Model rewrites</Label>
            <Button size="sm" variant="outline" className="h-6 gap-1 text-xs" onClick={addRewrite}>
              <Plus className="h-3 w-3" /> Add rewrite
            </Button>
          </div>
          {policy.model_rewrites.map((rw, i) => (
            <div key={i} className="flex items-center gap-2">
              <Input
                value={rw.from}
                onChange={(e) => updateRewrite(i, { from: e.target.value })}
                className="h-7 text-xs"
                placeholder="requested model"
              />
              <span className="text-xs text-muted-foreground">→</span>
              <Input
                value={rw.to}
                onChange={(e) => updateRewrite(i, { to: e.target.value })}
                className="h-7 text-xs"
                placeholder="sent model"
              />
              <Button size="icon" variant="ghost" className="h-7 w-7 shrink-0" onClick={() => removeRewrite(i)}>
                <Trash2 className="h-3.5 w-3.5" />
              </Button>
            </div>
          ))}
        </div>
      </CardContent>
    </Card>
  )
}
