import { execSync } from 'child_process';
import { homedir } from 'os';
import { join } from 'path';
import fs from 'fs-extra';

const TEST_CONFIG_DIR = join(homedir(), '.localrouter-test');

async function globalSetup(): Promise<void> {
  console.log('\n=== E2E Test Global Setup ===\n');

  // Set environment variables for test isolation
  process.env.LOCALROUTER_ENV = 'test';
  process.env.LOCALROUTER_KEYCHAIN = 'file';

  // Clean and create test config directory
  console.log(`Creating clean test config directory: ${TEST_CONFIG_DIR}`);
  await fs.remove(TEST_CONFIG_DIR);
  await fs.ensureDir(TEST_CONFIG_DIR);

  // Create minimal initial config
  const initialConfig = `# LocalRouter Test Config
server:
  host: "127.0.0.1"
  port: 3625
providers: []
clients: []
mcp_servers: []
strategies: []
`;
  await fs.writeFile(join(TEST_CONFIG_DIR, 'settings.yaml'), initialConfig);

  // Build the Tauri app in debug mode (if not already built)
  console.log('Building Tauri app (debug mode)...');
  try {
    execSync('cargo build --manifest-path=src-tauri/Cargo.toml', {
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
