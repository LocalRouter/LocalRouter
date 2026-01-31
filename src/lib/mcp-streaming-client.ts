/**
 * MCP Streaming Client Library
 *
 * A comprehensive TypeScript client for the LocalRouter SSE streaming gateway.
 * Multiplexes multiple MCP servers into a single client-facing stream with
 * request/response correlation, notifications, and deferred loading support.
 *
 * Usage:
 * ```typescript
 * const client = new MCPStreamingClient('http://localhost:3625', bearerToken);
 * const session = await client.initialize(['filesystem', 'github']);
 * const events = await session.getEventStream();
 *
 * // Handle events
 * events.on('response', (event) => {
 *   console.log(`Response from ${event.server_id}:`, event.response);
 * });
 *
 * // Send request
 * const reqId = await session.sendRequest({
 *   jsonrpc: '2.0',
 *   id: 'req-1',
 *   method: 'filesystem__tools/call',
 *   params: { name: 'read_file', arguments: { path: '/etc/hosts' } }
 * });
 * ```
 */

import { EventEmitter } from 'events';

/**
 * Streaming event types
 */
export interface StreamingEventResponse {
  type: 'response';
  request_id: string;
  server_id: string;
  response: JsonRpcResponse;
}

export interface StreamingEventNotification {
  type: 'notification';
  server_id: string;
  notification: JsonRpcNotification;
}

export interface StreamingEventChunk {
  type: 'chunk';
  request_id: string;
  server_id: string;
  chunk: {
    is_final: boolean;
    data: unknown;
  };
}

export interface StreamingEventError {
  type: 'error';
  request_id?: string;
  server_id?: string;
  error: string;
}

export interface StreamingEventHeartbeat {
  type: 'heartbeat';
}

/**
 * JSON-RPC types
 */
export interface JsonRpcRequest {
  jsonrpc: '2.0';
  id?: string | number;
  method: string;
  params?: unknown;
}

export interface JsonRpcResponse {
  jsonrpc: '2.0';
  id?: string | number;
  result?: unknown;
  error?: JsonRpcError;
}

export interface JsonRpcNotification {
  jsonrpc: '2.0';
  method: string;
  params?: unknown;
}

export interface JsonRpcError {
  code: number;
  message: string;
  data?: unknown;
}

/**
 * Session initialization response
 */
export interface StreamingSessionInfo {
  session_id: string;
  stream_url: string;
  request_url: string;
  initialized_servers: string[];
  failed_servers: string[];
}

/**
 * Request accepted response
 */
export interface RequestAccepted {
  request_id: string;
  target_servers: string[];
  broadcast: boolean;
}

/**
 * Pending request tracking
 */
interface PendingRequest {
  id: string | number;
  server_id: string;
  timeout: NodeJS.Timeout;
}

/**
 * MCPStreamingSession - represents an active streaming session
 */
export class MCPStreamingSession extends EventEmitter {
  private sessionId: string;
  private streamUrl: string;
  private requestUrl: string;
  private baseUrl: string;
  private bearerToken: string;
  private eventSource: EventSource | null = null;
  private pendingRequests = new Map<string, PendingRequest>();
  private requestTimeout = 60000; // 60 seconds default
  private isConnected = false;

  constructor(
    sessionId: string,
    streamUrl: string,
    requestUrl: string,
    baseUrl: string,
    bearerToken: string
  ) {
    super();
    this.sessionId = sessionId;
    this.streamUrl = streamUrl;
    this.requestUrl = requestUrl;
    this.baseUrl = baseUrl;
    this.bearerToken = bearerToken;
  }

  /**
   * Connect to the SSE event stream
   */
  async connect(): Promise<void> {
    return new Promise((resolve, reject) => {
      try {
        const url = `${this.baseUrl}${this.streamUrl}`;
        this.eventSource = new EventSource(url, {
          headers: {
            Authorization: `Bearer ${this.bearerToken}`,
          },
        } as any);

        this.eventSource.addEventListener('response', (event: Event) => {
          const e = event as MessageEvent;
          const data = JSON.parse(e.data);
          this.emit('response', {
            type: 'response' as const,
            request_id: data.request_id,
            server_id: data.server_id,
            response: data.response,
          } as StreamingEventResponse);

          // Clear pending request timeout
          this.clearPendingRequest(data.request_id);
        });

        this.eventSource.addEventListener('notification', (event: Event) => {
          const e = event as MessageEvent;
          const data = JSON.parse(e.data);
          this.emit('notification', {
            type: 'notification' as const,
            server_id: data.server_id,
            notification: data.notification,
          } as StreamingEventNotification);
        });

        this.eventSource.addEventListener('chunk', (event: Event) => {
          const e = event as MessageEvent;
          const data = JSON.parse(e.data);
          this.emit('chunk', {
            type: 'chunk' as const,
            request_id: data.request_id,
            server_id: data.server_id,
            chunk: data.chunk,
          } as StreamingEventChunk);

          if (data.chunk.is_final) {
            this.clearPendingRequest(data.request_id);
          }
        });

        this.eventSource.addEventListener('error', (event: Event) => {
          const e = event as MessageEvent;
          const data = JSON.parse(e.data);
          this.emit('error', {
            type: 'error' as const,
            request_id: data.request_id,
            server_id: data.server_id,
            error: data.error,
          } as StreamingEventError);

          if (data.request_id) {
            this.clearPendingRequest(data.request_id);
          }
        });

        this.eventSource.addEventListener('heartbeat', () => {
          this.emit('heartbeat', { type: 'heartbeat' as const } as StreamingEventHeartbeat);
        });

        this.eventSource.addEventListener('open', () => {
          this.isConnected = true;
          resolve();
        });

        this.eventSource.addEventListener('error', (e) => {
          if (!this.isConnected) {
            reject(new Error('Failed to connect to SSE stream'));
          } else {
            this.emit('stream-error', e);
          }
        });
      } catch (e) {
        reject(e);
      }
    });
  }

  /**
   * Send a JSON-RPC request through the streaming session
   */
  async sendRequest(request: JsonRpcRequest): Promise<string> {
    const response = await fetch(`${this.baseUrl}${this.requestUrl}`, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${this.bearerToken}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(request),
    });

    if (!response.ok) {
      const error = await response.text();
      throw new Error(`Failed to send request: ${response.status} ${error}`);
    }

    const data: RequestAccepted = await response.json();

    // Track this request for timeout
    const requestId = data.request_id;
    const timeout = setTimeout(() => {
      const pending = this.pendingRequests.get(requestId);
      if (pending) {
        this.emit('request-timeout', {
          request_id: requestId,
          target_servers: data.target_servers,
        });
        this.pendingRequests.delete(requestId);
      }
    }, this.requestTimeout);

    this.pendingRequests.set(requestId, {
      id: request.id || requestId,
      server_id: data.target_servers[0] || 'unknown',
      timeout,
    });

    return requestId;
  }

  /**
   * Close the streaming session
   */
  async close(): Promise<void> {
    if (this.eventSource) {
      this.eventSource.close();
      this.eventSource = null;
    }

    const url = `${this.baseUrl}/gateway/stream/${this.sessionId}`;
    await fetch(url, {
      method: 'DELETE',
      headers: {
        Authorization: `Bearer ${this.bearerToken}`,
      },
    });

    this.isConnected = false;
    this.emit('closed');
  }

  /**
   * Check if session is connected
   */
  get connected(): boolean {
    return this.isConnected;
  }

  /**
   * Clear timeout for a pending request
   */
  private clearPendingRequest(requestId: string): void {
    const pending = this.pendingRequests.get(requestId);
    if (pending) {
      clearTimeout(pending.timeout);
      this.pendingRequests.delete(requestId);
    }
  }
}

/**
 * MCPStreamingClient - main client for initializing streaming sessions
 */
export class MCPStreamingClient {
  private baseUrl: string;
  private bearerToken: string;

  constructor(baseUrl: string, bearerToken: string) {
    this.baseUrl = baseUrl.replace(/\/$/, ''); // Remove trailing slash
    this.bearerToken = bearerToken;
  }

  /**
   * Initialize a new streaming session
   */
  async initialize(
    allowedServers: string[],
    clientInfo: { name: string; version: string } = {
      name: 'mcp-streaming-client',
      version: '1.0.0',
    }
  ): Promise<MCPStreamingSession> {
    const response = await fetch(`${this.baseUrl}/gateway/stream`, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${this.bearerToken}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        protocolVersion: '2024-11-05',
        capabilities: {},
        clientInfo,
      }),
    });

    if (!response.ok) {
      const error = await response.text();
      throw new Error(`Failed to initialize session: ${response.status} ${error}`);
    }

    const sessionInfo: StreamingSessionInfo = await response.json();

    if (sessionInfo.failed_servers.length > 0) {
      console.warn(
        `Failed to initialize servers: ${sessionInfo.failed_servers.join(', ')}`
      );
    }

    const session = new MCPStreamingSession(
      sessionInfo.session_id,
      sessionInfo.stream_url,
      sessionInfo.request_url,
      this.baseUrl,
      this.bearerToken
    );

    await session.connect();

    return session;
  }
}

/**
 * Helper function to create a namespaced method name
 *
 * @param serverId - MCP server ID
 * @param method - Method name
 * @returns Namespaced method name (e.g., "filesystem__tools/call")
 */
export function createNamespacedMethod(serverId: string, method: string): string {
  return `${serverId}__${method}`;
}

/**
 * Parse a namespaced method to extract server ID and method name
 *
 * @param namespacedMethod - Namespaced method name
 * @returns {serverId, method} or null if not namespaced
 */
export function parseNamespacedMethod(
  namespacedMethod: string
): { serverId: string; method: string } | null {
  const parts = namespacedMethod.split('__');
  if (parts.length === 2) {
    return { serverId: parts[0], method: parts[1] };
  }
  return null;
}

/**
 * Broadcast methods that are routed to all servers
 */
export const BROADCAST_METHODS = ['tools/list', 'resources/list', 'prompts/list'];

/**
 * Check if a method is a broadcast method
 */
export function isBroadcastMethod(method: string): boolean {
  return BROADCAST_METHODS.includes(method);
}

// Export types for users
export type StreamingEvent =
  | StreamingEventResponse
  | StreamingEventNotification
  | StreamingEventChunk
  | StreamingEventError
  | StreamingEventHeartbeat;
