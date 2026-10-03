import { defineConfig } from '@playwright/test'

// Browser-only Monitor regressions use the demo's IPC bridge, without a native app.
export default defineConfig({
  testDir: './monitor',
  workers: 1,
  timeout: 30_000,
  use: { channel: process.env.PLAYWRIGHT_CHANNEL || 'chromium', baseURL: 'http://127.0.0.1:1432', viewport: { width: 1440, height: 1000 } },
  webServer: {
    command: 'npm --prefix website run dev -- --host 127.0.0.1 --port 1432 --strictPort',
    cwd: '../..',
    url: 'http://127.0.0.1:1432/demo',
    reuseExistingServer: !process.env.CI,
  },
  outputDir: '../../test-results/monitor',
  reporter: 'list',
})
