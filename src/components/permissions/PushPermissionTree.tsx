import { useMemo, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { PermissionTreeSelector } from "./PermissionTreeSelector"
import {
  PushPermissionButton,
  PushPermissionWithDefaultButton,
  type PushPermissionChoice,
} from "./PushPermissionButton"
import type { TreeNode } from "./types"
import type {
  ClientPushSettings,
  PushPermission,
  PushSourceInfo,
  SetClientPushPermissionParams,
} from "@/types/tauri-commands"

const SERVER_PREFIX = "s:"
const EVENT_PREFIX = "e:"

/** Key of an event type in `PushPermissions.events`. Mirrors Rust `PushPermissions::event_key`. */
export function pushEventKey(serverId: string, eventName: string): string {
  return `${serverId}__${eventName}`
}

/** The built-in push test source (Try it out → MCP → Push). Not an MCP server. */
export const PUSH_TEST_SOURCE_ID = "localrouter"
export const PUSH_TEST_EVENT = "test.event"

/** The test source is listed only once a permission is configured for it. */
export function showPushTestSource(settings: ClientPushSettings): boolean {
  return (
    settings.permissions.servers?.[PUSH_TEST_SOURCE_ID] !== undefined ||
    settings.permissions.events?.[pushEventKey(PUSH_TEST_SOURCE_ID, PUSH_TEST_EVENT)] !== undefined
  ) && !settings.servers.some((s) => s.server_id === PUSH_TEST_SOURCE_ID)
}

/** Servers worth showing in a push tree: they offer consumable push. */
export function pushTreeSources(sources: PushSourceInfo[]): PushSourceInfo[] {
  return sources.filter((s) => s.has_consumable_push)
}

function sourceSummary(source: PushSourceInfo): string {
  const parts: string[] = []
  const supported = source.events.filter((e) => e.supported).length
  if (supported > 0) parts.push(`${supported} event type${supported === 1 ? "" : "s"}`)
  if (source.channels) parts.push("Claude Channels")
  return parts.join(" · ")
}

interface PushPermissionTreeProps {
  clientId: string
  sources: PushSourceInfo[]
  settings: ClientPushSettings
  onUpdate: () => void
  disabled?: boolean
}

/**
 * Client → server → event type push permissions with Inherit / Allow / Ask / Block.
 * Servers the client has no MCP access to are listed but locked: push never
 * widens MCP access.
 */
export function PushPermissionTree({ clientId, sources, settings, onUpdate, disabled }: PushPermissionTreeProps) {
  const [saving, setSaving] = useState(false)

  const access = useMemo(
    () => new Map(settings.servers.map((s) => [s.server_id, s.mcp_access])),
    [settings.servers],
  )

  const showTest = showPushTestSource(settings)
  const nodes: TreeNode[] = useMemo(
    () => [
      ...pushTreeSources(sources).map((source) => {
        const hasAccess = access.get(source.server_id) ?? false
        return {
          id: SERVER_PREFIX + source.server_id,
          label: source.server_name,
          description: hasAccess
            ? sourceSummary(source)
            : "No MCP access for this client — no events are delivered",
          disabled: !hasAccess,
          searchText: source.events.map((e) => e.name).join(" "),
          children: source.events.map((event) => ({
            id: EVENT_PREFIX + pushEventKey(source.server_id, event.name),
            label: event.name,
            description: event.supported
              ? event.description ?? undefined
              : "Webhook only — not supported (LocalRouter does not receive webhooks)",
            disabled: !hasAccess || !event.supported,
          })),
        }
      }),
      ...(showTest
        ? [{
            id: SERVER_PREFIX + PUSH_TEST_SOURCE_ID,
            label: "LocalRouter test source",
            description: "Built-in test events from Try it out → MCP → Push; only sessions that opted into push testing receive them",
            children: [{
              id: EVENT_PREFIX + pushEventKey(PUSH_TEST_SOURCE_ID, PUSH_TEST_EVENT),
              label: PUSH_TEST_EVENT,
              description: "Test event",
            }],
          }]
        : []),
    ],
    [sources, access, showTest],
  )

  const flatPermissions = useMemo(() => {
    const flat: Record<string, PushPermissionChoice> = {}
    for (const [id, p] of Object.entries(settings.permissions.servers ?? {})) flat[SERVER_PREFIX + id] = p
    for (const [key, p] of Object.entries(settings.permissions.events ?? {})) flat[EVENT_PREFIX + key] = p
    return flat
  }, [settings.permissions])

  const save = async (params: SetClientPushPermissionParams) => {
    setSaving(true)
    try {
      await invoke("set_client_push_permission", { ...params } satisfies SetClientPushPermissionParams)
      onUpdate()
    } catch (error) {
      toast.error(`Failed to update push permission: ${error}`)
    } finally {
      setSaving(false)
    }
  }

  const handleNodeChange = (nodeId: string, choice: PushPermissionChoice) => {
    const state = choice === "inherit" ? null : choice
    if (nodeId.startsWith(SERVER_PREFIX)) {
      return save({ clientId, level: "server", serverId: nodeId.slice(SERVER_PREFIX.length), state })
    }
    const key = nodeId.slice(EVENT_PREFIX.length)
    if (showTest && key === pushEventKey(PUSH_TEST_SOURCE_ID, PUSH_TEST_EVENT)) {
      return save({ clientId, level: "event", serverId: PUSH_TEST_SOURCE_ID, eventName: PUSH_TEST_EVENT, state })
    }
    const source = sources.find((s) => key.startsWith(`${s.server_id}__`))
    if (!source) return
    return save({
      clientId,
      level: "event",
      serverId: source.server_id,
      eventName: key.slice(source.server_id.length + 2),
      state,
    })
  }

  const handleClientChange = (state: PushPermission | null) =>
    save({ clientId, level: "global", state })

  return (
    <PermissionTreeSelector<PushPermissionChoice>
      nodes={nodes}
      permissions={flatPermissions}
      globalPermission={settings.effective_client}
      onPermissionChange={(key, state) => handleNodeChange(key, state)}
      onGlobalChange={(state) => handleClientChange(state === "inherit" ? null : state)}
      renderButton={(props) => (
        <PushPermissionButton
          {...props}
          value={props.value as PushPermission}
        />
      )}
      renderGlobalButton={(childRollupStates) => (
        <PushPermissionWithDefaultButton
          value={settings.permissions.global ?? null}
          globalDefault={settings.global_default}
          onChange={handleClientChange}
          disabled={saving || disabled}
          childRollupStates={childRollupStates}
        />
      )}
      disabled={saving || disabled}
      globalLabel="All push sources"
      emptyMessage="No upstream MCP server offers push yet. Servers appear here once LocalRouter has seen them offer events or Claude Channels."
      defaultExpanded
      searchable={nodes.length > 5}
      searchPlaceholder="Search servers and event types..."
    />
  )
}
