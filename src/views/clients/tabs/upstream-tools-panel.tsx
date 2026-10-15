import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/Card'
import { Checkbox } from '@/components/ui/checkbox'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import type {
  UpstreamToolsPolicy,
  GetClientUpstreamToolsParams,
  SetClientUpstreamToolsParams,
} from '@/types/tauri-commands'

export function UpstreamToolsPanel({
  clientId,
  onUpdate,
}: {
  clientId: string
  onUpdate: () => void
}) {
  const [policy, setPolicy] = useState<UpstreamToolsPolicy | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    let live = true
    setPolicy(null)
    setError(null)
    void invoke<UpstreamToolsPolicy>('get_client_upstream_tools', {
      clientId,
    } satisfies GetClientUpstreamToolsParams)
      .then((value) => {
        if (live) setPolicy(value)
      })
      .catch((error) => {
        if (live) setError(String(error))
      })
    return () => {
      live = false
    }
  }, [clientId])
  async function save() {
    if (!policy) return
    setBusy(true)
    setError(null)
    try {
      const value = await invoke<UpstreamToolsPolicy>(
        'set_client_upstream_tools',
        {
          clientId,
          allowedTools: policy.allowed_tools,
          maxToolCalls: policy.max_tool_calls,
        } satisfies SetClientUpstreamToolsParams,
      )
      setPolicy(value)
      onUpdate()
    } catch (error) {
      setError(String(error))
    } finally {
      setBusy(false)
    }
  }
  const valid =
    policy &&
    (policy.max_tool_calls === null ||
      (Number.isInteger(policy.max_tool_calls) &&
        policy.max_tool_calls >= 1 &&
        policy.max_tool_calls <= 30))
  return (
    <Card>
      <CardHeader>
        <CardTitle>Provider search and web fetch</CardTitle>
        <CardDescription>
          Allow retrieval executed by an upstream provider. Existing model
          permissions still apply.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {(['web_search', 'web_fetch'] as const).map((tool) => (
          <label key={tool} className="flex items-center gap-3 text-sm">
            <Checkbox
              checked={policy?.allowed_tools.includes(tool) ?? false}
              disabled={busy || !policy}
              onCheckedChange={(checked) =>
                setPolicy(
                  (current) =>
                    current && {
                      ...current,
                      allowed_tools:
                        checked === true
                          ? [...new Set([...current.allowed_tools, tool])]
                          : current.allowed_tools.filter(
                              (value) => value !== tool,
                            ),
                    },
                )
              }
            />
            {tool === 'web_search' ? 'Allow web search' : 'Allow web fetch'}
          </label>
        ))}
        <label className="flex items-start gap-3 text-sm">
          <Checkbox
            checked={policy?.max_tool_calls === null}
            disabled={busy || !policy}
            onCheckedChange={(checked) =>
              setPolicy(
                (current) =>
                  current && {
                    ...current,
                    max_tool_calls: checked === true ? null : 8,
                  },
              )
            }
          />
          <span>
            Use provider-managed call limits
            <span className="block text-xs text-muted-foreground">
              Required for LLM Gateway search, whose global call cap has no
              verified mapping. This setting provides no LocalRouter step
              guarantee.
            </span>
          </span>
        </label>
        {policy?.max_tool_calls !== null && (
          <label className="block space-y-1 text-sm">
            Maximum tool calls per upstream request
            <Input
              aria-label="Maximum upstream tool calls"
              type="number"
              min={1}
              max={30}
              value={policy?.max_tool_calls ?? 8}
              disabled={busy || !policy}
              onChange={(event) =>
                setPolicy(
                  (current) =>
                    current && {
                      ...current,
                      max_tool_calls: Number(event.target.value),
                    },
                )
              }
            />
          </label>
        )}
        <p className="text-xs text-muted-foreground">
          OpenRouter supports search and fetch with a call cap. LLM Gateway
          supports basic search. Tools may incur separate fees; requests with
          cost ceilings, free-only routing, strict privacy, automatic MCP or
          bound session continuity are rejected. Dynamic tool output bypasses
          response caching and session reuse. Failed calls are not retried
          automatically.
        </p>
        <p className="text-xs text-muted-foreground">
          For tools you want LocalRouter to mediate and approve, configure a
          search or fetch MCP server under MCPs and grant it through the
          client's MCP settings.
        </p>
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        <Button onClick={() => void save()} disabled={busy || !valid}>
          {busy ? 'Saving…' : 'Save tool permissions'}
        </Button>
      </CardContent>
    </Card>
  )
}
