import { defineConfig } from '@playwright/test';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

export default defineConfig({
  testDir: './specs',
  fullyParallel: false,
  workers: 1,
  timeout: 60000,
  expect: { timeout: 10000 },
  globalSetup: path.join(__dirname, 'global-setup.ts'),
  globalTeardown: path.join(__dirname, 'global-teardown.ts'),
  use: {
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },
  reporter: [
    ['list'],
    ['html', { outputFolder: 'playwright-report' }],
  ],
});
