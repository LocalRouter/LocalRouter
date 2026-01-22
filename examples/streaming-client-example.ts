/**
 * MCP Streaming Client - Complete Usage Example
 *
 * This example demonstrates:
 * 1. Session initialization with multiple servers
 * 2. Event handling (responses, notifications, errors)
 * 3. Request sending and response correlation
 * 4. Session cleanup
 */

import {
  MCPStreamingClient,
  MCPStreamingSession,
  createNamespacedMethod,
  isBroadcastMethod,
} from '../src/lib/mcp-streaming-client';

/**
 * Example 1: Basic Session Initialization and File Reading
 */
async function example1_BasicUsage() {
  console.log('=== Example 1: Basic Usage ===\n');

  const client = new MCPStreamingClient('http://localhost:3625', 'your-bearer-token');

  try {
    // Initialize session with allowed servers
    const session = await client.initialize(['filesystem', 'github']);

    console.log('✓ Session initialized');
    console.log(`  Stream URL: /gateway/stream/${session.sessionId}`);

    // Set up event handlers
    session.on('response', (event) => {
      console.log(`✓ Response from ${event.server_id}:`);
      console.log(JSON.stringify(event.response, null, 2));
    });

    session.on('error', (event) => {
      console.error(`✗ Error from ${event.server_id}: ${event.error}`);
    });

    session.on('heartbeat', () => {
      console.log('♥ Heartbeat');
    });

    // Send request to read a file
    const readFileRequestId = await session.sendRequest({
      jsonrpc: '2.0',
      id: 'read-file-1',
      method: createNamespacedMethod('filesystem', 'tools/call'),
      params: {
        name: 'read_file',
        arguments: {
          path: '/etc/hosts',
        },
      },
    });

    console.log(`✓ Request sent: ${readFileRequestId}`);

    // Keep session open for a bit to receive response
    await new Promise((resolve) => setTimeout(resolve, 2000));

    // Close session
    await session.close();
    console.log('✓ Session closed\n');
  } catch (error) {
    console.error('Error:', error);
  }
}

/**
 * Example 2: Broadcast Request to Multiple Servers
 */
async function example2_BroadcastRequest() {
  console.log('=== Example 2: Broadcast Request ===\n');

  const client = new MCPStreamingClient('http://localhost:3625', 'your-bearer-token');

  try {
    const session = await client.initialize(['filesystem', 'github', 'database']);

    // Track responses from each server
    const responses = new Map<string, unknown>();

    session.on('response', (event) => {
      responses.set(event.server_id, event.response);
      console.log(`✓ Got tools list from ${event.server_id}`);
    });

    // Send broadcast request (goes to all servers)
    const toolsListRequestId = await session.sendRequest({
      jsonrpc: '2.0',
      id: 'tools-list-broadcast',
      method: 'tools/list',
      params: {},
    });

    console.log(`✓ Broadcast request sent: ${toolsListRequestId}`);

    // Wait for all responses
    await new Promise((resolve) => setTimeout(resolve, 3000));

    console.log(`\n✓ Received responses from ${responses.size} servers:`);
    responses.forEach((_, serverId) => {
      console.log(`  - ${serverId}`);
    });

    await session.close();
    console.log('✓ Session closed\n');
  } catch (error) {
    console.error('Error:', error);
  }
}

/**
 * Example 3: Multiple Concurrent Requests
 */
async function example3_ConcurrentRequests() {
  console.log('=== Example 3: Concurrent Requests ===\n');

  const client = new MCPStreamingClient('http://localhost:3625', 'your-bearer-token');

  try {
    const session = await client.initialize(['filesystem']);

    const requestIds: string[] = [];
    let responseCount = 0;

    session.on('response', (event) => {
      responseCount++;
      console.log(`✓ Response #${responseCount} for request: ${event.request_id}`);
    });

    // Send multiple requests concurrently
    const requests = [
      {
        id: 1,
        method: createNamespacedMethod('filesystem', 'tools/call'),
        params: {
          name: 'read_file',
          arguments: { path: '/etc/hosts' },
        },
      },
      {
        id: 2,
        method: createNamespacedMethod('filesystem', 'tools/call'),
        params: {
          name: 'read_file',
          arguments: { path: '/etc/passwd' },
        },
      },
      {
        id: 3,
        method: createNamespacedMethod('filesystem', 'tools/call'),
        params: {
          name: 'read_file',
          arguments: { path: '/etc/group' },
        },
      },
    ];

    for (const req of requests) {
      const reqId = await session.sendRequest({
        jsonrpc: '2.0',
        id: req.id.toString(),
        method: req.method,
        params: req.params,
      });
      requestIds.push(reqId);
      console.log(`✓ Request ${req.id} sent: ${reqId}`);
    }

    // Wait for all responses
    await new Promise((resolve) => setTimeout(resolve, 3000));

    console.log(`\n✓ Sent ${requestIds.length} requests, received ${responseCount} responses`);

    await session.close();
    console.log('✓ Session closed\n');
  } catch (error) {
    console.error('Error:', error);
  }
}

/**
 * Example 4: Error Handling
 */
async function example4_ErrorHandling() {
  console.log('=== Example 4: Error Handling ===\n');

  const client = new MCPStreamingClient('http://localhost:3625', 'your-bearer-token');

  try {
    const session = await client.initialize(['filesystem']);

    const errorLog: string[] = [];

    session.on('error', (event) => {
      const msg = `Error from ${event.server_id}: ${event.error}`;
      errorLog.push(msg);
      console.log(`✗ ${msg}`);
    });

    session.on('request-timeout', (event) => {
      console.log(`⏱ Request timeout: ${event.request_id}`);
    });

    // Try to read a file that doesn't exist
    const reqId = await session.sendRequest({
      jsonrpc: '2.0',
      id: 'read-nonexistent',
      method: createNamespacedMethod('filesystem', 'tools/call'),
      params: {
        name: 'read_file',
        arguments: {
          path: '/nonexistent/file.txt',
        },
      },
    });

    console.log(`✓ Request sent: ${reqId}`);

    // Wait for error response
    await new Promise((resolve) => setTimeout(resolve, 2000));

    console.log(`\n✓ Captured ${errorLog.length} errors`);

    await session.close();
    console.log('✓ Session closed\n');
  } catch (error) {
    console.error('Error:', error);
  }
}

/**
 * Example 5: Notification Handling
 */
async function example5_Notifications() {
  console.log('=== Example 5: Notification Handling ===\n');

  const client = new MCPStreamingClient('http://localhost:3625', 'your-bearer-token');

  try {
    const session = await client.initialize(['filesystem']);

    const notifications: string[] = [];

    session.on('notification', (event) => {
      const msg = `${event.server_id}: ${event.notification.method}`;
      notifications.push(msg);
      console.log(`📬 Notification: ${msg}`);
    });

    console.log('Listening for notifications...');

    // Keep session open to receive notifications
    await new Promise((resolve) => setTimeout(resolve, 5000));

    console.log(`\n✓ Received ${notifications.length} notifications`);

    await session.close();
    console.log('✓ Session closed\n');
  } catch (error) {
    console.error('Error:', error);
  }
}

/**
 * Example 6: Helper Functions
 */
function example6_HelperFunctions() {
  console.log('=== Example 6: Helper Functions ===\n');

  // Create namespaced method
  const method1 = createNamespacedMethod('filesystem', 'tools/call');
  console.log(`Namespaced method: ${method1}`);
  // Output: filesystem__tools/call

  // Check if broadcast
  console.log(`Is "tools/list" broadcast? ${isBroadcastMethod('tools/list')}`);
  // Output: true

  console.log(`Is "filesystem__tools/call" broadcast? ${isBroadcastMethod('filesystem__tools/call')}`);
  // Output: false

  console.log();
}

/**
 * Run examples
 */
async function main() {
  console.log('\n╔════════════════════════════════════════════════════╗');
  console.log('║  MCP Streaming Client - Usage Examples             ║');
  console.log('╚════════════════════════════════════════════════════╝\n');

  example6_HelperFunctions();

  // Run examples in sequence
  // Uncomment to run individual examples:
  // await example1_BasicUsage();
  // await example2_BroadcastRequest();
  // await example3_ConcurrentRequests();
  // await example4_ErrorHandling();
  // await example5_Notifications();

  console.log('✓ All examples completed\n');
}

// Run if executed directly
if (require.main === module) {
  main().catch(console.error);
}

export {
  example1_BasicUsage,
  example2_BroadcastRequest,
  example3_ConcurrentRequests,
  example4_ErrorHandling,
  example5_Notifications,
  example6_HelperFunctions,
};
