/**
 * Push Relay demo state: the global default permission, the upstream push sources the
 * handlers in TauriMockSetup.ts report, and each client's push settings.
 *
 * Sources: VocalCord (MCP Events, push + poll, one webhook-only type),
 * NetGet (Claude Channels and events), a billing server that offers only
 * webhooks (shown as not supported), Home Assistant (poll only), Build Alerts
 * (push), and the existing demo servers, which offer no push or are not known yet.
 *
 * Live status (`get_mcp_server_push_status`, `mockPushStatusFor`): VocalCord is
 * listening via push stream with events received, Home Assistant is polling with
 * no events yet, NetGet's stream is reconnecting after an error, and Build Alerts
 * is not listened to (no client is consuming it).
 */
import type {
  ClientPushSettings,
  McpServerPushStatus,
  PendingPushApproval,
  PushPermissions,
  PushRelayConfig,
  PushSourceInfo,
} from '@app/types/tauri-commands'

export const mockPushRelayConfig: PushRelayConfig = {
  default_permission: 'allow',
  channel_messages_per_minute: 10,
  history_retention_hours: 24,
  history_max_events: 10_000,
}

const minutesAgo = (m: number) => new Date(Date.now() - m * 60_000).toISOString()

export const mockPushSources: PushSourceInfo[] = [
  {
    server_id: 'mcp-vocalcord',
    server_name: 'VocalCord',
    enabled: true,
    snapshot_status: 'current',
    protocol_version: '2025-11-25',
    observed_at: minutesAgo(42),
    channels: false,
    resource_subscribe: false,
    has_consumable_push: true,
    receiving_clients: 0,
    events: [
      { name: 'message.received', description: 'A message arrived on any connected channel (Telegram, Discord, Slack)', modes: ['push', 'poll', 'webhook'], supported: true },
      { name: 'message.edited', description: 'A previously delivered message was edited', modes: ['push', 'poll'], supported: true },
      { name: 'call.missed', description: 'A voice call went unanswered', modes: ['poll'], supported: true },
      { name: 'attachment.ready', description: 'An uploaded attachment finished processing', modes: ['webhook'], supported: false },
    ],
  },
  {
    server_id: 'mcp-netget',
    server_name: 'NetGet',
    enabled: true,
    snapshot_status: 'current',
    protocol_version: '2025-06-18',
    observed_at: minutesAgo(180),
    channels: true,
    resource_subscribe: true,
    has_consumable_push: true,
    receiving_clients: 0,
    events: [
      { name: 'intercept.parked', description: 'A protocol event is waiting for a manual answer', modes: ['push', 'poll'], supported: true },
      { name: 'server.stopped', description: 'A NetGet server stopped or failed', modes: ['push'], supported: true },
    ],
  },
  {
    server_id: 'mcp-billing',
    server_name: 'Billing Webhooks',
    enabled: true,
    snapshot_status: 'current',
    protocol_version: '2026-07-28',
    observed_at: minutesAgo(60 * 26),
    channels: false,
    resource_subscribe: false,
    has_consumable_push: false,
    receiving_clients: 0,
    events: [
      { name: 'invoice.paid', description: 'An invoice was paid', modes: ['webhook'], supported: false },
      { name: 'subscription.cancelled', description: 'A subscription was cancelled', modes: ['webhook'], supported: false },
    ],
  },
  {
    server_id: 'mcp-home',
    server_name: 'Home Assistant',
    enabled: true,
    snapshot_status: 'current',
    protocol_version: '2025-11-25',
    observed_at: minutesAgo(8),
    channels: false,
    resource_subscribe: false,
    has_consumable_push: true,
    receiving_clients: 0,
    events: [
      { name: 'state.changed', description: 'A device or entity changed state', modes: ['poll'], supported: true },
    ],
  },
  {
    server_id: 'mcp-builds',
    server_name: 'Build Alerts',
    enabled: true,
    snapshot_status: 'current',
    protocol_version: '2025-11-25',
    observed_at: minutesAgo(130),
    channels: false,
    resource_subscribe: false,
    has_consumable_push: true,
    receiving_clients: 0,
    events: [
      { name: 'build.failed', description: 'A pipeline run failed', modes: ['push', 'poll'], supported: true },
      { name: 'build.finished', description: 'A pipeline run finished successfully', modes: ['push', 'poll', 'webhook'], supported: true },
    ],
  },
  { server_id: 'mcp-github', server_name: 'GitHub', enabled: true, snapshot_status: 'current', protocol_version: '2025-06-18', observed_at: minutesAgo(600), channels: false, resource_subscribe: false, has_consumable_push: false, receiving_clients: 0, events: [] },
  { server_id: 'mcp-filesystem', server_name: 'Filesystem', enabled: true, snapshot_status: 'current', protocol_version: '2025-03-26', observed_at: minutesAgo(900), channels: false, resource_subscribe: true, has_consumable_push: false, receiving_clients: 0, events: [] },
  { server_id: 'mcp-slack', server_name: 'Slack', enabled: true, snapshot_status: 'none', protocol_version: null, observed_at: null, channels: false, resource_subscribe: false, has_consumable_push: false, receiving_clients: 0, events: [] },
  { server_id: 'mcp-postgres', server_name: 'PostgreSQL', enabled: false, snapshot_status: 'none', protocol_version: null, observed_at: null, channels: false, resource_subscribe: false, has_consumable_push: false, receiving_clients: 0, events: [] },
  { server_id: 'mcp-browser', server_name: 'Browser Control', enabled: true, snapshot_status: 'current', protocol_version: '2025-06-18', observed_at: minutesAgo(75), channels: false, resource_subscribe: false, has_consumable_push: false, receiving_clients: 0, events: [] },
]

export interface MockClientPush {
  permissions: PushPermissions
  webhook_urls: string[]
}

/** Keyed by client id (`id` in mockData.clients). */
export const mockClientPush: Record<string, MockClientPush> = {
  'client-2': {
    permissions: {
      servers: { 'mcp-netget': 'ask' },
      events: { 'mcp-vocalcord__message.edited': 'off' },
    },
    webhook_urls: [],
  },
  'client-4': {
    permissions: { global: 'ask' },
    webhook_urls: ['http://127.0.0.1:8787/hooks/localrouter', 'https://agents.example.com/webhooks/mcp-events'],
  },
}

/** Whether any push can reach a client with these settings (Client::may_receive_push). */
export function mockReceivesPush(permissions: PushPermissions): boolean {
  if (mockPushRelayConfig.default_permission !== 'off') return true
  return [permissions.global, ...Object.values(permissions.servers ?? {}), ...Object.values(permissions.events ?? {})]
    .some((p) => p != null && p !== 'off')
}

/** A client's effective permission for one event type (event → server → client → default). */
export function mockEffectivePush(permissions: PushPermissions, serverId: string, event: string) {
  return permissions.events?.[`${serverId}__${event}`]
    ?? permissions.servers?.[serverId]
    ?? permissions.global
    ?? mockPushRelayConfig.default_permission
}

/**
 * The sources with `receiving_clients` counted the way the Rust command does: clients with
 * MCP access and a permission other than Off for a supported event type or channel messages.
 */
export function mockPushSourcesFor(clients: { id: string; mcpAccess: (serverId: string) => boolean }[]): PushSourceInfo[] {
  return mockPushSources.map((s) => {
    const names = [...s.events.filter((e) => e.supported).map((e) => e.name), ...(s.channels ? ['channel.message'] : [])]
    const receiving_clients = !s.enabled || names.length === 0 ? 0 : clients.filter((c) =>
      c.mcpAccess(s.server_id)
      && names.some((n) => mockEffectivePush(clientPush(c.id).permissions, s.server_id, n) !== 'off'),
    ).length
    return { ...s, receiving_clients }
  })
}

export function clientPush(clientId: string): MockClientPush {
  if (!mockClientPush[clientId]) {
    mockClientPush[clientId] = { permissions: {}, webhook_urls: [] }
  }
  return mockClientPush[clientId]
}

/** Resolve a client's settings the way the Rust command does. */
export function mockClientPushSettings(
  clientId: string,
  mcpAccess: (serverId: string) => boolean,
): ClientPushSettings {
  const p = clientPush(clientId)
  const def = mockPushRelayConfig.default_permission
  const client = p.permissions.global ?? def
  return {
    client_id: clientId,
    permissions: p.permissions,
    webhook_urls: p.webhook_urls,
    global_default: def,
    effective_client: client,
    receives_push: mockReceivesPush(p.permissions),
    servers: mockPushSources.map((s) => ({
      server_id: s.server_id,
      mcp_access: s.enabled && mcpAccess(s.server_id),
      effective: p.permissions.servers?.[s.server_id] ?? client,
    })),
  }
}

/** What ~/.claude.json holds per Claude Code client (demo). */
export const mockClaudeCodeTransport: Record<string, 'none' | 'http' | 'stdio_bridge' | 'other'> = {
  // An existing setup from before the STDIO bridge default, so the demo shows the switch notice.
  'client-2': 'http',
}

/** Held (Ask) events: NetGet is Ask for Claude Code, everything is Ask for Cline. */
export const mockPendingPushApprovals: PendingPushApproval[] = [
  {
    request_id: 'hold-1', client_id: 'client-2', client_name: 'Claude Code',
    server_id: 'mcp-netget', server_name: 'netget', event_name: 'intercept.parked',
    qualified_name: 'netget__intercept.parked', event_id: 'netget__evt-7781', ingress: 'push',
    received_at: minutesAgo(2),
    data: { server_id: 3, protocol: 'redis', peer: '127.0.0.1:52811', event: 'redis_command', command: 'GET session:42' },
    position: 0, pending_of_type: 1, decided: false, firewall_request_id: null,
  },
  {
    request_id: 'hold-2', client_id: 'client-2', client_name: 'Claude Code',
    server_id: 'mcp-netget', server_name: 'netget', event_name: 'intercept.parked',
    qualified_name: 'netget__intercept.parked', event_id: 'netget__evt-7782', ingress: 'push',
    received_at: minutesAgo(1),
    data: { server_id: 3, protocol: 'redis', peer: '127.0.0.1:52811', event: 'redis_command', command: 'SET session:42 active' },
    position: 1, pending_of_type: 1, decided: false, firewall_request_id: null,
  },
  {
    request_id: 'hold-3', client_id: 'client-4', client_name: 'Cline',
    server_id: 'mcp-vocalcord', server_name: 'vocalcord', event_name: 'message.received',
    qualified_name: 'vocalcord__message.received', event_id: 'vocalcord__msg-551', ingress: 'poll',
    received_at: minutesAgo(4),
    data: { channel: 'telegram', from: 'Dana', text: 'Can you look at the failing deploy?' },
    position: 0, pending_of_type: 0, decided: false, firewall_request_id: null,
  },
]

const secondsAgo = (s: number) => new Date(Date.now() - s * 1000).toISOString()

/**
 * Live push status of one demo server. A server no client may receive from (for
 * instance with the default permission switched to Off) is not listened to.
 */
export function mockPushStatusFor(serverId: string, receivingClients: number): McpServerPushStatus {
  const base: McpServerPushStatus = {
    server_id: serverId, relay_running: true, listening: false, event_types: [],
    events_received: 0, last_event_at: null, consuming_clients: 0, consuming_sessions: 0,
  }
  const live = receivingClients > 0
  switch (serverId) {
    case 'mcp-vocalcord': {
      const last = secondsAgo(130)
      return live
        ? {
            ...base, listening: true, events_received: 3, last_event_at: last, consuming_clients: 2, consuming_sessions: 2,
            event_types: [
              { event: 'message.received', ingress_mode: 'push', state: 'listening', last_error: null, subscriptions: 1, events_received: 3, last_event_at: last },
              { event: 'message.edited', ingress_mode: 'push', state: 'listening', last_error: null, subscriptions: 1, events_received: 0, last_event_at: null },
            ],
          }
        : {
            ...base, events_received: 3, last_event_at: last,
            event_types: [{ event: 'message.received', ingress_mode: null, state: 'idle', last_error: null, subscriptions: 0, events_received: 3, last_event_at: last }],
          }
    }
    case 'mcp-home':
      return live
        ? {
            ...base, listening: true, consuming_clients: 1, consuming_sessions: 1,
            event_types: [{ event: 'state.changed', ingress_mode: 'poll', state: 'listening', last_error: null, subscriptions: 1, events_received: 0, last_event_at: null }],
          }
        : base
    case 'mcp-netget':
      return live
        ? {
            ...base, consuming_clients: 1, consuming_sessions: 1,
            event_types: [{ event: 'intercept.parked', ingress_mode: 'push', state: 'reconnecting', last_error: 'upstream stream ended', subscriptions: 1, events_received: 0, last_event_at: null }],
          }
        : base
    default:
      return base
  }
}
