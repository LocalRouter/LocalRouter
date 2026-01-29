import { execSync } from 'child_process';
import { homedir } from 'os';
import { join } from 'path';
import { createServer } from 'net';
import fs from 'fs-extra';

const TEST_CONFIG_DIR = join(homedir(), '.localrouter-test');
const PORT_FILE = join(TEST_CONFIG_DIR, '.test-port');

/**
 * Find an available port by binding to port 0 and reading the assigned port
 */
async function findAvailablePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      if (address && typeof address === 'object') {
        const port = address.port;
        server.close(() => resolve(port));
      } else {
        server.close(() => reject(new Error('Could not determine port')));
      }
    });
    server.on('error', reject);
  });
}

async function globalSetup(): Promise<void> {
  console.log('\n=== E2E Test Global Setup ===\n');

  // Set environment variables for test isolation
  process.env.LOCALROUTER_ENV = 'test';
  process.env.LOCALROUTER_KEYCHAIN = 'file';

  // Clean and create test config directory
  console.log(`Creating clean test config directory: ${TEST_CONFIG_DIR}`);
  await fs.remove(TEST_CONFIG_DIR);
  await fs.ensureDir(TEST_CONFIG_DIR);

  // Find an available port for testing
  const testPort = await findAvailablePort();
  console.log(`Using test port: ${testPort}`);

  // Save port to file so tests can read it
  await fs.writeFile(PORT_FILE, testPort.toString());

  // Create minimal initial config with the random port using proper YAML structure
  // Note: AppConfig requires `version` field and at least one router
  const initialConfig = {
    version: 2, // Current config version
    server: {
      host: '127.0.0.1',
      port: testPort,
      enable_cors: true,
    },
    // At least one provider is required - Ollama doesn't need an API key
    providers: [
      {
        name: 'Ollama',
        provider_type: 'ollama',
        enabled: true,
      },
    ],
    clients: [],
    mcp_servers: [],
    strategies: [],
    // At least one router is required with at least one strategy
    routers: [
      {
        name: 'Default',
        model_selection: {
          type: 'automatic',
          providers: [],
        },
        strategies: ['lowest_cost'],
        fallback_enabled: true,
        rate_limiters: [],
      },
    ],
    logging: {
      level: 'info',
      enable_access_log: true,
    },
    oauth_clients: [],
  };

  // Use YAML library to ensure proper formatting
  const YAML = await import('yaml');
  const yamlContent = YAML.stringify(initialConfig);
  console.log('Writing config file with content:\n', yamlContent);
  await fs.writeFile(join(TEST_CONFIG_DIR, 'settings.yaml'), yamlContent);

  // Build the Tauri app in debug mode (if not already built)
  console.log('Building Tauri app (debug mode)...');
  try {
    // Use cargo build from the workspace root - the binary ends up in target/debug/
    execSync('cargo build -p localrouter', {
      cwd: process.cwd(),
      stdio: 'inherit',
      env: {
        ...process.env,
        LOCALROUTER_ENV: 'test',
        LOCALROUTER_KEYCHAIN: 'file',
      },
    });
    console.log('✓ Build complete');
  } catch (error) {
    console.error('Failed to build Tauri app:', error);
    throw error;
  }

  console.log('\n=== Setup Complete ===\n');
}

export default globalSetup;
