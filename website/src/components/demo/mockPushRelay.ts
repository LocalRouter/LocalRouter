/**
 * Push Relay demo state: global settings, the upstream push sources the
 * handlers in TauriMockSetup.ts report, and each client's push settings.
 *
 * Sources: VocalCord (MCP Events, push + poll, one webhook-only type),
 * NetGet (Claude Channels and events), a billing server that offers only
 * webhooks (shown as not supported), and the existing demo servers, which
 * offer no push.
 */
import type {
  ClientPushSettings,
  PushFallbackTools,
  PushPermissions,
  PushRelayConfig,
  PushRelayLimits,
  PushSourceInfo,
} from '@app/types/tauri-commands'

const MIB = 1024 * 1024

export const defaultPushRelayLimits: PushRelayLimits = {
  journal_max_events: 10_000,
  journal_max_bytes: 64 * MIB,
  journal_retention_secs: 24 * 3600,
  max_event_bytes: MIB,
  ask_timeout_secs: 30 * 60,
  ask_hold_cap: 1_000,
  first_discovery_wait_ms: 10_000,
  upstream_poll_min_interval_ms: 1_000,
  upstream_poll_max_interval_ms: 5 * 60 * 1_000,
  upstream_reconnect_backoff_min_ms: 1_000,
  upstream_reconnect_backoff_max_ms: 60_000,
  upstream_heartbeat_timeout_secs: 90,
  downstream_heartbeat_secs: 30,
  max_streams_per_session: 64,
  poll_max_events: 500,
  poll_default_events: 50,
  poll_max_wait_secs: 50,
  watch_max_lifetime_secs: 24 * 3600,
  channel_max_content_bytes: 16 * 1024,
  channel_rate_per_minute: 60,
  webhook_timeout_secs: 10,
  webhook_retry_delays_secs: [5, 30, 120, 600, 1_800],
  webhook_max_subscriptions: 100,
  webhook_lease_default_secs: 3600,
  webhook_lease_min_secs: 60,
  webhook_lease_max_secs: 24 * 3600,
}

export const mockPushRelayConfig: PushRelayConfig = {
  enabled: true,
  default_permission: 'allow',
  webhook_egress_enabled: true,
  allow_loopback_http: true,
  limits: { ...defaultPushRelayLimits, ask_timeout_secs: 15 * 60 },
  default_limits: defaultPushRelayLimits,
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
    events: [
      { name: 'invoice.paid', description: 'An invoice was paid', modes: ['webhook'], supported: false },
      { name: 'subscription.cancelled', description: 'A subscription was cancelled', modes: ['webhook'], supported: false },
    ],
  },
  { server_id: 'mcp-github', server_name: 'GitHub', enabled: true, snapshot_status: 'current', protocol_version: '2025-06-18', observed_at: minutesAgo(600), channels: false, resource_subscribe: false, has_consumable_push: false, events: [] },
  { server_id: 'mcp-filesystem', server_name: 'Filesystem', enabled: true, snapshot_status: 'current', protocol_version: '2025-03-26', observed_at: minutesAgo(900), channels: false, resource_subscribe: true, has_consumable_push: false, events: [] },
  { server_id: 'mcp-slack', server_name: 'Slack', enabled: true, snapshot_status: 'none', protocol_version: null, observed_at: null, channels: false, resource_subscribe: false, has_consumable_push: false, events: [] },
]

export interface MockClientPush {
  permissions: PushPermissions
  fallback_tools: PushFallbackTools
  webhook_urls: string[]
  claude_channels_launched: boolean
}

/** Keyed by client id (`id` in mockData.clients). */
export const mockClientPush: Record<string, MockClientPush> = {
  'client-2': {
    permissions: {
      servers: { 'mcp-netget': 'ask' },
      events: { 'mcp-vocalcord__message.edited': 'block' },
    },
    fallback_tools: 'auto',
    webhook_urls: [],
    claude_channels_launched: false,
  },
  'client-4': {
    permissions: { global: 'ask' },
    fallback_tools: 'always',
    webhook_urls: ['http://127.0.0.1:8787/hooks/localrouter', 'https://agents.example.com/webhooks/mcp-events'],
    claude_channels_launched: false,
  },
}

export function clientPush(clientId: string): MockClientPush {
  if (!mockClientPush[clientId]) {
    mockClientPush[clientId] = { permissions: {}, fallback_tools: 'auto', webhook_urls: [], claude_channels_launched: false }
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
    fallback_tools: p.fallback_tools,
    webhook_urls: p.webhook_urls,
    claude_channels_launched: p.claude_channels_launched,
    global_default: def,
    effective_client: client,
    relay_enabled: mockPushRelayConfig.enabled,
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
