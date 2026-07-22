/**
 * Allowed-models control for an HTTPS-proxy client.
 *
 * A proxy client talks straight to its own upstream (e.g. api.anthropic.com),
 * so there is no LocalRouter provider/model catalog to pick from — the model is
 * whatever the tool sends. Instead of the gateway's catalog selector we offer a
 * simple optional whitelist: by default every model is allowed (pure
 * inspection); turn off "Allow all models" to restrict the proxy to a typed
 * list of exact model ids.
 *
 * This maps onto the strategy's existing `model_permissions` (which the proxy
 * firewall already enforces): allow-all is `global: "allow"`; a whitelist is
 * `global: "off"` with each listed model marked `allow` under the anthropic
 * provider key. No backend change required.
 */

import { useEffect, useState } from "react"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card"
import { Switch } from "@/components/ui/Toggle"
import { ListChecks } from "lucide-react"
import type { PermissionState } from "@/types/tauri-commands"

interface ModelPerms {
  global: PermissionState
  providers: Record<string, PermissionState>
  models: Record<string, PermissionState>
}

const PROVIDER_KEY = "anthropic__"

/** Model ids currently whitelisted (allow-state entries, provider prefix stripped). */
function toList(perms: ModelPerms): string[] {
  return Object.entries(perms.models)
    .filter(([, state]) => state === "allow")
    .map(([key]) => (key.startsWith(PROVIDER_KEY) ? key.slice(PROVIDER_KEY.length) : key))
}

/** Turn a newline-separated textarea into a `model_permissions` whitelist. */
function fromText(text: string): ModelPerms {
  const models: Record<string, PermissionState> = {}
  for (const raw of text.split("\n")) {
    const name = raw.trim()
    if (name) models[`${PROVIDER_KEY}${name}`] = "allow"
  }
  return { global: "off", providers: {}, models }
}

export function ProxyAllowedModels({
  value,
  onChange,
  disabled,
}: {
  value: ModelPerms
  onChange: (next: ModelPerms) => void
  disabled?: boolean
}) {
  const allowAll = value.global === "allow"
  const [text, setText] = useState(() => toList(value).join("\n"))

  // Re-sync the textarea when the strategy loads/changes underneath us, but not
  // on every keystroke (we own the draft while editing in whitelist mode).
  useEffect(() => {
    if (allowAll) setText(toList(value).join("\n"))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [allowAll])

  const setAllowAll = (on: boolean) => {
    if (on) {
      onChange({ global: "allow", providers: {}, models: {} })
    } else {
      // Entering whitelist mode: start from whatever is typed (usually empty).
      onChange(fromText(text))
    }
  }

  const onText = (next: string) => {
    setText(next)
    onChange(fromText(next))
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="p-2 rounded-lg bg-primary/10">
              <ListChecks className="h-4 w-4 text-primary" />
            </div>
            <div>
              <CardTitle className="text-base">Allowed Models</CardTitle>
              <CardDescription>
                Restrict which models may pass through the proxy. By default every model is allowed.
              </CardDescription>
            </div>
          </div>
          <label className="flex items-center gap-2 text-xs shrink-0">
            <span className="text-muted-foreground">Allow all models</span>
            <Switch checked={allowAll} onCheckedChange={setAllowAll} disabled={disabled} />
          </label>
        </div>
      </CardHeader>
      {!allowAll && (
        <CardContent className="space-y-2">
          <textarea
            value={text}
            onChange={(e) => onText(e.target.value)}
            disabled={disabled}
            rows={5}
            spellCheck={false}
            placeholder={"One model id per line, e.g.\nclaude-sonnet-4-5-20250929\nclaude-opus-4-1-20250805"}
            className="w-full rounded-md border border-border bg-background p-2 font-mono text-xs resize-y focus:outline-none focus:ring-1 focus:ring-ring"
          />
          <p className="text-xs text-muted-foreground">
            Exact model ids, one per line. A request whose model isn&apos;t listed is denied &mdash; an
            empty list blocks everything. Re-enable &ldquo;Allow all models&rdquo; to permit everything.
          </p>
        </CardContent>
      )}
    </Card>
  )
}
