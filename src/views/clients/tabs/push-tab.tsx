import { useCallback, useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Info, Trash2 } from "lucide-react"
import { FEATURES } from "@/constants/features"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card"
import { listenSafe } from "@/hooks/useTauriListener"
import { PushPermissionTree } from "@/components/permissions/PushPermissionTree"
import { SamplePopupButton } from "@/components/shared/SamplePopupButton"
import { CopyableCode } from "@/components/client/CopyableCode"
import {
  CLAUDE_CHANNELS_LAUNCH_COMMAND,
  ClaudeChannelsExplanation,
  ClaudeCodeHttpNotice,
} from "@/components/client/ClaudeCodeBridge"
import type {
  ClientPushSettings,
  GetClientPushSettingsParams,
  PushSourceInfo,
  RemoveClientPushWebhookUrlParams,
} from "@/types/tauri-commands"

interface Client {
  id: string
  client_id: string
  name: string
  template_id?: string | null
  push_permissions?: unknown
  push_webhook_urls?: string[]
}

interface ClientPushCardProps {
  client: Client
  onUpdate: () => void
  onViewChange?: (view: string, subTab?: string | null) => void
}

/** Push Relay settings for one client: permissions, webhooks, and how it receives events. */
export function ClientPushCard({ client, onUpdate, onViewChange }: ClientPushCardProps) {
  const [settings, setSettings] = useState<ClientPushSettings | null>(null)
  const [sources, setSources] = useState<PushSourceInfo[]>([])
  const [saving, setSaving] = useState(false)
  const isClaudeCode = client.template_id === "claude-code"

  const load = useCallback(async () => {
    try {
      const [s, src] = await Promise.all([
        invoke<ClientPushSettings>("get_client_push_settings", {
          clientId: client.client_id,
        } satisfies GetClientPushSettingsParams),
        invoke<PushSourceInfo[]>("list_push_sources"),
      ])
      setSettings(s)
      setSources(src)
    } catch (error) {
      console.error("Failed to load push settings:", error)
    }
  }, [client.client_id])

  useEffect(() => {
    load()
  }, [load, client.push_permissions, client.push_webhook_urls])

  useEffect(() => {
    const listeners = [listenSafe("clients-changed", load), listenSafe("config-changed", load)]
    return () => listeners.forEach((l) => l.cleanup())
  }, [load])

  const run = async (fn: () => Promise<unknown>, success?: string) => {
    setSaving(true)
    try {
      await fn()
      if (success) toast.success(success)
      await load()
      onUpdate()
    } catch (error) {
      toast.error(`Failed to update: ${error}`)
    } finally {
      setSaving(false)
    }
  }

  const removeWebhook = (url: string) =>
    run(
      () =>
        invoke("remove_client_push_webhook_url", {
          clientId: client.client_id,
          url,
        } satisfies RemoveClientPushWebhookUrlParams),
      "Webhook URL removed; the next delivery to it will ask again",
    )

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center gap-2">
          <FEATURES.pushRelay.icon className={`h-5 w-5 ${FEATURES.pushRelay.color}`} />
          <CardTitle className="text-base">{FEATURES.pushRelay.name}</CardTitle>
          {settings && !settings.relay_enabled && (
            <Badge variant="outline" className="text-[10px] px-1.5 py-0">Off globally</Badge>
          )}
        </div>
        <CardDescription>
          Events pushed by upstream MCP servers, delivered to this client as MCP Events, Claude Channels or
          the EventPoll tools. Push never widens the client&rsquo;s MCP access.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        {settings && !settings.relay_enabled && (
          <div className="flex items-start gap-2 rounded-lg bg-muted/50 p-3 text-xs text-muted-foreground">
            <Info className="h-4 w-4 shrink-0 mt-0.5" />
            <span>
              Push Relay is turned off, so nothing is relayed to any client.{" "}
              {onViewChange && (
                <button className="text-primary hover:underline" onClick={() => onViewChange("push-relay")}>
                  Global settings
                </button>
              )}
            </span>
          </div>
        )}

        {/* Permission tree */}
        <div className="space-y-1.5">
          <div className="flex items-start justify-between gap-4">
            <div className="space-y-1.5">
              <p className="text-sm font-medium">Permissions</p>
              <p className="text-xs text-muted-foreground">
                Allow relays each event; Ask holds each one for your approval; Block hides the event type. The most
                specific setting wins.
              </p>
            </div>
            <SamplePopupButton popupType="push_event" />
          </div>
          {settings ? (
            <PushPermissionTree
              clientId={client.client_id}
              sources={sources}
              settings={settings}
              onUpdate={() => {
                load()
                onUpdate()
              }}
              disabled={!settings.relay_enabled}
            />
          ) : (
            <p className="text-sm text-muted-foreground py-4 text-center">Loading...</p>
          )}
        </div>

        {/* Fallback tools */}
        <div className="space-y-1.5">
          <p className="text-sm font-medium">Fallback tools</p>
          <p className="text-xs text-muted-foreground">
            <code className="bg-muted px-1 rounded">EventList</code>,{" "}
            <code className="bg-muted px-1 rounded">EventPoll</code> and{" "}
            <code className="bg-muted px-1 rounded">EventWait</code> are offered automatically whenever a permitted
            server pushes events, unless the client declares native MCP Events support. Claude Code sessions through the
            STDIO bridge keep them too: LocalRouter cannot tell whether Claude was launched with channels.
          </p>
        </div>

        {/* Webhook URLs */}
        {settings && (
          <div className="space-y-1.5">
            <p className="text-sm font-medium">Approved webhook URLs</p>
            <p className="text-xs text-muted-foreground">
              Callback URLs this client subscribed with and you chose to always allow. Removing one makes the next
              delivery to it ask again.
            </p>
            {settings.webhook_urls.length === 0 ? (
              <p className="text-xs text-muted-foreground italic">None</p>
            ) : (
              <div className="space-y-1">
                {settings.webhook_urls.map((url) => (
                  <div key={url} className="flex items-center justify-between gap-2 rounded-md bg-muted/40 px-2 py-1">
                    <code className="text-xs break-all">{url}</code>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-7 w-7 shrink-0"
                      onClick={() => removeWebhook(url)}
                      disabled={saving}
                      title="Remove"
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {/* Claude Channels */}
        {settings && isClaudeCode && (
          <div className="space-y-2">
            <ClaudeCodeHttpNotice clientId={client.client_id} onSwitched={() => load()} />
            <div>
              <p className="text-sm font-medium">Claude Channels</p>
              <p className="text-xs text-muted-foreground">
                Launch Claude Code with the channels flag so pushed events wake it as channel messages:
              </p>
            </div>
            <CopyableCode value={CLAUDE_CHANNELS_LAUNCH_COMMAND} />
            <ClaudeChannelsExplanation />
          </div>
        )}
      </CardContent>
    </Card>
  )
}
