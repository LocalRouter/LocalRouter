import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { ChevronRight, ChevronDown, Plus, X, Server, Wrench, Sparkles } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { FirewallPolicySelector, type FirewallPolicy } from "@/components/firewall/FirewallPolicySelector"

interface Client {
  id: string
  name: string
  client_id: string
}

interface FirewallRules {
  default_policy: FirewallPolicy
  server_rules: Record<string, FirewallPolicy>
  tool_rules: Record<string, FirewallPolicy>
  skill_rules: Record<string, FirewallPolicy>
  skill_tool_rules: Record<string, FirewallPolicy>
}

interface FirewallTabProps {
  client: Client
  onUpdate: () => void
}

type RuleType = "server" | "tool" | "skill" | "skill_tool"

interface RuleSection {
  type: RuleType
  label: string
  description: string
  icon: React.ReactNode
  rules: Record<string, FirewallPolicy>
}

export function ClientFirewallTab({ client, onUpdate }: FirewallTabProps) {
  const [rules, setRules] = useState<FirewallRules | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [expandedSections, setExpandedSections] = useState<Set<RuleType>>(new Set())
  const [addingRule, setAddingRule] = useState<RuleType | null>(null)
  const [newRuleName, setNewRuleName] = useState("")

  const loadRules = useCallback(async () => {
    try {
      const result = await invoke<FirewallRules>("get_client_firewall_rules", {
        clientId: client.id,
      })
      setRules(result)
    } catch (error) {
      console.error("Failed to load firewall rules:", error)
      toast.error("Failed to load firewall rules")
    } finally {
      setLoading(false)
    }
  }, [client.id])

  useEffect(() => {
    loadRules()

    const unsubscribe = listen("clients-changed", () => {
      loadRules()
    })
    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [loadRules])

  const handleDefaultPolicyChange = async (policy: FirewallPolicy) => {
    setSaving(true)
    try {
      await invoke("set_client_default_firewall_policy", {
        clientId: client.id,
        policy,
      })
      onUpdate()
    } catch (error) {
      console.error("Failed to set default policy:", error)
      toast.error("Failed to update default policy")
    } finally {
      setSaving(false)
    }
  }

  const handleRuleChange = async (ruleType: RuleType, key: string, policy: FirewallPolicy) => {
    setSaving(true)
    try {
      await invoke("set_client_firewall_rule", {
        clientId: client.id,
        ruleType,
        key,
        policy,
      })
      onUpdate()
    } catch (error) {
      console.error("Failed to set firewall rule:", error)
      toast.error("Failed to update rule")
    } finally {
      setSaving(false)
    }
  }

  const handleRemoveRule = async (ruleType: RuleType, key: string) => {
    setSaving(true)
    try {
      // Setting to the default policy effectively removes the override
      // We pass null/remove via setting it to the default
      await invoke("set_client_firewall_rule", {
        clientId: client.id,
        ruleType,
        key,
        policy: null,
      })
      onUpdate()
    } catch (error) {
      console.error("Failed to remove rule:", error)
      toast.error("Failed to remove rule")
    } finally {
      setSaving(false)
    }
  }

  const handleAddRule = async (ruleType: RuleType) => {
    if (!newRuleName.trim()) return
    setSaving(true)
    try {
      await invoke("set_client_firewall_rule", {
        clientId: client.id,
        ruleType,
        key: newRuleName.trim(),
        policy: "ask" as FirewallPolicy,
      })
      setNewRuleName("")
      setAddingRule(null)
      onUpdate()
    } catch (error) {
      console.error("Failed to add rule:", error)
      toast.error("Failed to add rule")
    } finally {
      setSaving(false)
    }
  }

  const toggleSection = (type: RuleType) => {
    setExpandedSections((prev) => {
      const next = new Set(prev)
      if (next.has(type)) {
        next.delete(type)
      } else {
        next.add(type)
      }
      return next
    })
  }

  if (loading || !rules) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="text-muted-foreground text-sm">Loading firewall rules...</div>
      </div>
    )
  }

  const sections: RuleSection[] = [
    {
      type: "server",
      label: "MCP Server Rules",
      description: "Rules applied to all tools from a specific MCP server",
      icon: <Server className="h-4 w-4" />,
      rules: rules.server_rules,
    },
    {
      type: "tool",
      label: "Tool Rules",
      description: "Rules for specific namespaced MCP tools (e.g. filesystem__write_file)",
      icon: <Wrench className="h-4 w-4" />,
      rules: rules.tool_rules,
    },
    {
      type: "skill",
      label: "Skill Rules",
      description: "Rules applied to all tools from a specific skill",
      icon: <Sparkles className="h-4 w-4" />,
      rules: rules.skill_rules,
    },
    {
      type: "skill_tool",
      label: "Skill Tool Rules",
      description: "Rules for specific skill tools",
      icon: <Sparkles className="h-4 w-4" />,
      rules: rules.skill_tool_rules,
    },
  ]

  const totalRules = Object.keys(rules.server_rules).length +
    Object.keys(rules.tool_rules).length +
    Object.keys(rules.skill_rules).length +
    Object.keys(rules.skill_tool_rules).length

  return (
    <div className="space-y-6">
      {/* Default Policy */}
      <Card>
        <CardHeader>
          <CardTitle>Default Policy</CardTitle>
          <CardDescription>
            When no specific rule matches, tool calls will use this policy.
            Individual rules below take precedence over the default.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <FirewallPolicySelector
            value={rules.default_policy}
            onChange={handleDefaultPolicyChange}
            disabled={saving}
          />
        </CardContent>
      </Card>

      {/* Rule Sections */}
      <Card>
        <CardHeader>
          <CardTitle>
            Rules
            {totalRules > 0 && (
              <span className="ml-2 text-sm font-normal text-muted-foreground">
                ({totalRules} rule{totalRules !== 1 ? "s" : ""})
              </span>
            )}
          </CardTitle>
          <CardDescription>
            Specific rules override the default policy. Most specific rule wins:
            Tool → Server → Default.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-2">
          {sections.map((section) => {
            const ruleEntries = Object.entries(section.rules)
            const isExpanded = expandedSections.has(section.type)
            const isAdding = addingRule === section.type

            return (
              <div key={section.type} className="border rounded-lg">
                {/* Section Header */}
                <button
                  type="button"
                  onClick={() => toggleSection(section.type)}
                  className="w-full flex items-center gap-2 px-3 py-2 text-sm hover:bg-muted/50 transition-colors"
                >
                  {isExpanded ? (
                    <ChevronDown className="h-4 w-4 text-muted-foreground" />
                  ) : (
                    <ChevronRight className="h-4 w-4 text-muted-foreground" />
                  )}
                  <span className="text-muted-foreground">{section.icon}</span>
                  <span className="font-medium">{section.label}</span>
                  <span className="text-xs text-muted-foreground ml-auto">
                    {ruleEntries.length} rule{ruleEntries.length !== 1 ? "s" : ""}
                  </span>
                </button>

                {/* Expanded Content */}
                {isExpanded && (
                  <div className="border-t px-3 py-2 space-y-2">
                    <p className="text-xs text-muted-foreground mb-2">{section.description}</p>

                    {/* Existing Rules */}
                    {ruleEntries.map(([key, policy]) => (
                      <div
                        key={key}
                        className="flex items-center gap-2 py-1"
                      >
                        <code className="text-xs bg-muted px-1.5 py-0.5 rounded flex-1 truncate">
                          {key}
                        </code>
                        <FirewallPolicySelector
                          value={policy}
                          onChange={(p) => handleRuleChange(section.type, key, p)}
                          disabled={saving}
                          size="sm"
                        />
                        <button
                          type="button"
                          onClick={() => handleRemoveRule(section.type, key)}
                          disabled={saving}
                          className="text-muted-foreground hover:text-destructive transition-colors p-0.5"
                        >
                          <X className="h-3.5 w-3.5" />
                        </button>
                      </div>
                    ))}

                    {/* Add Rule */}
                    {isAdding ? (
                      <div className="flex items-center gap-2 pt-1">
                        <Input
                          value={newRuleName}
                          onChange={(e) => setNewRuleName(e.target.value)}
                          placeholder={
                            section.type === "server"
                              ? "server_id"
                              : section.type === "tool"
                              ? "server__tool_name"
                              : section.type === "skill"
                              ? "skill_name"
                              : "skill_tool_name"
                          }
                          className="h-7 text-xs flex-1"
                          onKeyDown={(e) => {
                            if (e.key === "Enter") handleAddRule(section.type)
                            if (e.key === "Escape") {
                              setAddingRule(null)
                              setNewRuleName("")
                            }
                          }}
                          autoFocus
                        />
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-7 text-xs"
                          onClick={() => handleAddRule(section.type)}
                          disabled={!newRuleName.trim() || saving}
                        >
                          Add
                        </Button>
                        <Button
                          size="sm"
                          variant="ghost"
                          className="h-7 text-xs"
                          onClick={() => {
                            setAddingRule(null)
                            setNewRuleName("")
                          }}
                        >
                          Cancel
                        </Button>
                      </div>
                    ) : (
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-7 text-xs text-muted-foreground"
                        onClick={() => {
                          setAddingRule(section.type)
                          setNewRuleName("")
                        }}
                      >
                        <Plus className="h-3 w-3 mr-1" />
                        Add rule
                      </Button>
                    )}
                  </div>
                )}
              </div>
            )
          })}
        </CardContent>
      </Card>
    </div>
  )
}
