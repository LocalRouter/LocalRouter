import { homedir } from 'os';
import { join } from 'path';
import fs from 'fs-extra';

const TEST_CONFIG_DIR = join(homedir(), '.localrouter-test');

async function globalTeardown(): Promise<void> {
  console.log('\n=== E2E Test Global Teardown ===\n');

  // Clean up test config directory
  if (await fs.pathExists(TEST_CONFIG_DIR)) {
    console.log(`Removing test config directory: ${TEST_CONFIG_DIR}`);
    await fs.remove(TEST_CONFIG_DIR);
    console.log('✓ Test config directory removed');
  }

  console.log('\n=== Teardown Complete ===\n');
}

export default globalTeardown;
