# MCP Streaming Client Documentation

## Overview

The MCP Streaming Client is a comprehensive TypeScript/JavaScript library for interacting with the LocalRouter AI SSE streaming gateway. It enables real-time multiplexing of multiple MCP backend servers into a single client-facing stream with proper request/response correlation, notifications, and error handling.

**Key Features:**
- 🚀 Real-time event streaming via SSE
- 🔄 Automatic request/response correlation
- 📡 Multi-server multiplexing
- 🎯 Broadcast request support
- ⏱️ Automatic request timeout tracking
- 🔐 Bearer token authentication
- 📝 TypeScript type definitions
- 🛠️ Production-ready error handling

## Installation

### TypeScript/Node.js

```bash
npm install mcp-streaming-client
```

Or with Yarn:

```bash
yarn add mcp-streaming-client
```

### Browser

Include the library directly in your HTML:

```html
<script src="https://cdn.example.com/mcp-streaming-client.js"></script>
<script>
  const client = new MCPStreamingClient('http://localhost:3625', token);
</script>
```

## Quick Start

### Basic Usage

```typescript
import { MCPStreamingClient } from 'mcp-streaming-client';

// Create client
const client = new MCPStreamingClient('http://localhost:3625', 'your-bearer-token');

// Initialize session with allowed servers
const session = await client.initialize(['filesystem', 'github']);

// Listen for events
session.on('response', (event) => {
  console.log(`Response from ${event.server_id}:`, event.response);
});

session.on('error', (event) => {
  console.error(`Error from ${event.server_id}: ${event.error}`);
});

// Send request
const requestId = await session.sendRequest({
  jsonrpc: '2.0',
  id: 'read-file-1',
  method: 'filesystem__tools/call',
  params: {
    name: 'read_file',
    arguments: { path: '/etc/hosts' }
  }
});

// Close when done
await session.close();
```

## API Reference

### MCPStreamingClient

Main client for initializing streaming sessions.

#### Constructor

```typescript
new MCPStreamingClient(baseUrl: string, bearerToken: string)
```

**Parameters:**
- `baseUrl` - Server URL (e.g., `http://localhost:3625`)
- `bearerToken` - Authentication token

**Example:**
```typescript
const client = new MCPStreamingClient('http://localhost:3625', 'lr-abc123...');
```

#### Methods

##### `initialize(allowedServers: string[], clientInfo?: ClientInfo): Promise<MCPStreamingSession>`

Initialize a new streaming session.

**Parameters:**
- `allowedServers` - List of MCP server IDs to allow access to
- `clientInfo` - Optional client identification (default: `{name: 'mcp-streaming-client', version: '1.0.0'}`)

**Returns:** Active streaming session

**Example:**
```typescript
const session = await client.initialize(['filesystem', 'github'], {
  name: 'my-app',
  version: '1.0.0'
});
```

**Throws:**
- `Error` if initialization fails (401 Unauthorized, 500 Server Error, etc.)

### MCPStreamingSession

Active streaming session with event handling and request submission.

#### Properties

- `connected: boolean` - Whether session is connected to SSE stream
- `sessionId: string` - Unique session identifier

#### Methods

##### `connect(): Promise<void>`

Connect to the SSE event stream. Called automatically by `client.initialize()`.

```typescript
await session.connect();
```

##### `sendRequest(request: JsonRpcRequest): Promise<string>`

Send a JSON-RPC request through the streaming session.

**Parameters:**
- `request` - JSON-RPC 2.0 request object

**Returns:** Internal request ID for tracking

**Example:**
```typescript
const requestId = await session.sendRequest({
  jsonrpc: '2.0',
  id: 'tool-call-1',
  method: 'filesystem__tools/call',
  params: {
    name: 'write_file',
    arguments: {
      path: '/tmp/test.txt',
      content: 'Hello, World!'
    }
  }
});
```

**Request Routing:**
- **Direct routing:** Method name starts with `serverId__`
  - Example: `filesystem__tools/call` → routes only to `filesystem`
- **Broadcast routing:** Method is broadcast method
  - Example: `tools/list` → routes to all allowed servers
  - Broadcast methods: `tools/list`, `resources/list`, `prompts/list`

**Throws:**
- `Error` if request fails to submit

##### `close(): Promise<void>`

Close the streaming session and release resources.

```typescript
await session.close();
```

#### Events

Sessions extend `EventTarget` and emit the following events:

##### Response Event

```typescript
session.on('response', (event: StreamingEventResponse) => {
  console.log(event.request_id);   // Internal request ID
  console.log(event.server_id);    // Which server sent response
  console.log(event.response);     // JSON-RPC response object
});
```

**Event Object:**
```typescript
interface StreamingEventResponse {
  type: 'response';
  request_id: string;
  server_id: string;
  response: JsonRpcResponse;
}
```

##### Notification Event

```typescript
session.on('notification', (event: StreamingEventNotification) => {
  console.log(event.server_id);
  console.log(event.notification.method);  // e.g., "notifications/tools/list_changed"
});
```

**Event Object:**
```typescript
interface StreamingEventNotification {
  type: 'notification';
  server_id: string;
  notification: JsonRpcNotification;
}
```

##### Chunk Event (Streaming)

```typescript
session.on('chunk', (event: StreamingEventChunk) => {
  if (event.chunk.is_final) {
    console.log('Stream complete:', event.chunk.data);
  } else {
    console.log('Partial chunk:', event.chunk.data);
  }
});
```

**Event Object:**
```typescript
interface StreamingEventChunk {
  type: 'chunk';
  request_id: string;
  server_id: string;
  chunk: {
    is_final: boolean;
    data: unknown;
  };
}
```

##### Error Event

```typescript
session.on('error', (event: StreamingEventError) => {
  console.error(`Server ${event.server_id}: ${event.error}`);
  if (event.request_id) {
    console.error(`Request ${event.request_id} failed`);
  }
});
```

**Event Object:**
```typescript
interface StreamingEventError {
  type: 'error';
  request_id?: string;
  server_id?: string;
  error: string;
}
```

##### Heartbeat Event

```typescript
session.on('heartbeat', (event: StreamingEventHeartbeat) => {
  console.log('Keep-alive signal received');
});
```

**Event Object:**
```typescript
interface StreamingEventHeartbeat {
  type: 'heartbeat';
}
```

##### Request Timeout Event

```typescript
session.on('request-timeout', (event) => {
  console.error(`Request ${event.request_id} timed out`);
  console.error(`Target servers: ${event.target_servers.join(', ')}`);
});
```

##### Stream Error Event

```typescript
session.on('stream-error', (event) => {
  console.error('SSE stream error:', event);
  // May need to reconnect
});
```

##### Closed Event

```typescript
session.on('closed', () => {
  console.log('Session closed');
});
```

## Helper Functions

### createNamespacedMethod(serverId: string, method: string): string

Create a namespaced method name for routing to a specific server.

```typescript
const method = createNamespacedMethod('filesystem', 'tools/call');
// Result: "filesystem__tools/call"
```

### parseNamespacedMethod(method: string): {serverId: string, method: string} | null

Parse a namespaced method to extract server ID and method name.

```typescript
const parsed = parseNamespacedMethod('filesystem__tools/call');
// Result: { serverId: 'filesystem', method: 'tools/call' }
```

### isBroadcastMethod(method: string): boolean

Check if a method is a broadcast method (routed to all servers).

```typescript
isBroadcastMethod('tools/list');      // true
isBroadcastMethod('filesystem__tools/call'); // false
```

## Type Definitions

### JsonRpcRequest

```typescript
interface JsonRpcRequest {
  jsonrpc: '2.0';
  id?: string | number;
  method: string;
  params?: unknown;
}
```

### JsonRpcResponse

```typescript
interface JsonRpcResponse {
  jsonrpc: '2.0';
  id?: string | number;
  result?: unknown;
  error?: JsonRpcError;
}
```

### JsonRpcNotification

```typescript
interface JsonRpcNotification {
  jsonrpc: '2.0';
  method: string;
  params?: unknown;
}
```

### JsonRpcError

```typescript
interface JsonRpcError {
  code: number;
  message: string;
  data?: unknown;
}
```

### StreamingSessionInfo

```typescript
interface StreamingSessionInfo {
  session_id: string;
  stream_url: string;
  request_url: string;
  initialized_servers: string[];
  failed_servers: string[];
}
```

## Advanced Usage

### Handling Broadcast Requests

```typescript
// Broadcast tools/list to all servers
const requestId = await session.sendRequest({
  jsonrpc: '2.0',
  id: 'broadcast-1',
  method: 'tools/list',
  params: {}
});

// Collect responses from multiple servers
const responses = new Map();
let expectedCount = 3; // Number of allowed servers

const handleResponse = (event) => {
  responses.set(event.server_id, event.response);

  if (responses.size === expectedCount) {
    console.log('All servers responded:', Array.from(responses.values()));
  }
};

session.on('response', handleResponse);
```

### Handling Streaming Responses

```typescript
const chunks = new Map();

session.on('chunk', (event) => {
  if (!chunks.has(event.request_id)) {
    chunks.set(event.request_id, []);
  }

  chunks.get(event.request_id).push(event.chunk.data);

  if (event.chunk.is_final) {
    const fullData = chunks.get(event.request_id);
    console.log('Complete stream:', fullData);
    chunks.delete(event.request_id);
  }
});
```

### Request Correlation with Custom IDs

```typescript
const requestMap = new Map();

// Send request with custom ID
const requestId = await session.sendRequest({
  jsonrpc: '2.0',
  id: 'my-custom-id-123',
  method: 'filesystem__tools/call',
  params: {...}
});

// Store mapping
requestMap.set(requestId, {
  customId: 'my-custom-id-123',
  timestamp: Date.now(),
  originalRequest: {...}
});

// Handle response
session.on('response', (event) => {
  const metadata = requestMap.get(event.request_id);
  console.log('Response to request:', metadata.customId);
  requestMap.delete(event.request_id);
});
```

### Error Recovery

```typescript
let reconnectAttempts = 0;
const maxAttempts = 5;

async function ensureConnected() {
  if (!session.connected) {
    try {
      await session.connect();
      reconnectAttempts = 0;
      console.log('Reconnected successfully');
    } catch (error) {
      reconnectAttempts++;
      if (reconnectAttempts < maxAttempts) {
        const delay = Math.pow(2, reconnectAttempts) * 1000; // Exponential backoff
        console.log(`Reconnecting in ${delay}ms...`);
        await new Promise(resolve => setTimeout(resolve, delay));
        await ensureConnected();
      } else {
        throw new Error('Max reconnection attempts exceeded');
      }
    }
  }
}

session.on('stream-error', async () => {
  console.error('Stream error, attempting to reconnect...');
  await ensureConnected();
});
```

### Timeout Handling

```typescript
const pendingRequests = new Map();

session.on('request-timeout', (event) => {
  const request = pendingRequests.get(event.request_id);
  console.error(`Request ${event.request_id} timed out after 60 seconds`);
  console.error(`Target servers: ${event.target_servers.join(', ')}`);

  // Retry logic
  if (!request.retried) {
    console.log('Retrying request...');
    sendRequestWithRetry(request.originalRequest, true);
  }

  pendingRequests.delete(event.request_id);
});

async function sendRequestWithRetry(request, retried = false) {
  const requestId = await session.sendRequest(request);
  pendingRequests.set(requestId, { originalRequest: request, retried });
}
```

## Examples

### Example 1: Read File from Multiple Servers

```typescript
async function readFileFromAllServers(filePath) {
  const client = new MCPStreamingClient('http://localhost:3625', token);
  const session = await client.initialize(['filesystem', 'github', 'database']);

  const results = new Map();

  session.on('response', (event) => {
    results.set(event.server_id, event.response);
  });

  // Send to each server
  for (const serverId of ['filesystem', 'github', 'database']) {
    await session.sendRequest({
      jsonrpc: '2.0',
      id: `read-${serverId}`,
      method: createNamespacedMethod(serverId, 'tools/call'),
      params: {
        name: 'read_file',
        arguments: { path: filePath }
      }
    });
  }

  // Wait for all responses
  await new Promise(resolve => setTimeout(resolve, 2000));

  await session.close();
  return results;
}
```

### Example 2: Monitor Tool Changes

```typescript
async function monitorToolChanges() {
  const client = new MCPStreamingClient('http://localhost:3625', token);
  const session = await client.initialize(['filesystem', 'github']);

  const toolsCache = new Map();

  // Fetch initial tools list
  await session.sendRequest({
    jsonrpc: '2.0',
    id: 'initial-tools',
    method: 'tools/list',
    params: {}
  });

  // Cache initial responses
  session.once('response', (event) => {
    if (event.response.result?.tools) {
      toolsCache.set(event.server_id, event.response.result.tools);
    }
  });

  // Listen for changes
  session.on('notification', (event) => {
    if (event.notification.method === 'notifications/tools/list_changed') {
      console.log(`Tools changed on ${event.server_id}, refetching...`);

      // Refetch tools
      session.sendRequest({
        jsonrpc: '2.0',
        id: `refresh-${Date.now()}`,
        method: createNamespacedMethod(event.server_id, 'tools/list'),
        params: {}
      });
    }
  });

  // Keep session alive
  return session;
}
```

## Browser Usage

See `examples/streaming-client-browser.html` for a complete interactive browser demo.

To run locally:

1. Start the server: `cargo run`
2. Open `examples/streaming-client-browser.html` in your browser
3. Enter credentials and click "Initialize Session"
4. Send requests and view real-time responses

## Troubleshooting

### Connection Issues

**Problem:** "Failed to connect to SSE stream"

**Solutions:**
1. Check base URL is correct and server is running
2. Verify CORS is enabled if using cross-origin requests
3. Check network tab for SSE request status

### Authentication Errors

**Problem:** "401 Unauthorized"

**Solutions:**
1. Verify bearer token is valid
2. Token should start with `lr-` or be an OAuth access token
3. Check token expiration time

### Request Timeout

**Problem:** "Request timed out"

**Solutions:**
1. Check server logs for processing delays
2. Increase request timeout if needed (currently 60 seconds)
3. Ensure backend servers are healthy

### Missing Responses

**Problem:** "Not receiving responses to broadcast requests"

**Solutions:**
1. Verify all servers in `initialized_servers` list are healthy
2. Check server logs for errors processing request
3. Ensure request parameters are valid for all servers

## Performance Tips

1. **Reuse sessions** - Don't create new session for each request
2. **Batch broadcasts** - Send multiple requests together, then wait for responses
3. **Monitor memory** - Close sessions when done to release resources
4. **Use event filtering** - Only listen to events you need
5. **Correlation** - Store request metadata to avoid lookup on response

## Security Considerations

1. **Bearer tokens** should be kept secret (use environment variables)
2. **HTTPS** should be used in production
3. **CORS** should be restricted to known domains
4. **Rate limits** should be configured on the server
5. **Validate** all responses before using them

## Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create a feature branch
3. Add tests for new features
4. Ensure all tests pass
5. Submit a pull request

## License

MIT License - See LICENSE file for details

## Support

For issues, questions, or suggestions:

- GitHub Issues: https://github.com/anthropics/claude-code/issues
- Documentation: https://localrouterai.com/docs
- Community Discord: https://discord.gg/localrouterai
