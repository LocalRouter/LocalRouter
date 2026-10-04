import { defineConfig } from '@playwright/test'
export default defineConfig({
  testDir: '.', testMatch: 'mcp-remote.spec.ts', workers: 1, timeout: 30_000,
  reporter: 'list', outputDir: '../../test-results/mcp-remote',
  use: { baseURL: 'http://127.0.0.1:1431', viewport: { width: 1440, height: 1080 },
    launchOptions: process.env.PLAYWRIGHT_CHROME_EXECUTABLE ? { executablePath: process.env.PLAYWRIGHT_CHROME_EXECUTABLE } : {},
  },
  webServer: { command: 'npm run dev --prefix ../../website -- --host 127.0.0.1 --port 1431 --strictPort',
    url: 'http://127.0.0.1:1431/demo', reuseExistingServer: true },
})
